//! Bounded readback of the production scene target (top-to-bottom premultiplied RGBA8).
use crate::SceneRenderer;
use egui_wgpu::wgpu;
use std::{
    future::Future,
    sync::Arc,
    task::{Context, Poll, Wake, Waker},
    time::Duration,
};

struct ThreadWake(std::thread::Thread);
impl Wake for ThreadWake {
    fn wake(self: Arc<Self>) {
        self.0.unpark();
    }
}
/// Drive wgpu adapter/device/error-scope futures without a runtime.
pub fn block_on<T>(future: impl Future<Output = T>) -> T {
    let waker = Waker::from(Arc::new(ThreadWake(std::thread::current())));
    let mut context = Context::from_waker(&waker);
    let mut future = std::pin::pin!(future);
    loop {
        match future.as_mut().poll(&mut context) {
            Poll::Ready(value) => return value,
            Poll::Pending => std::thread::park(),
        }
    }
}
/// Padded wgpu row stride. Call only after validating nonzero, bounded dimensions.
pub fn row_stride(width: u32) -> u32 {
    (width * 4).div_ceil(256) * 256
}
/// Remove copy padding without flipping the image; wgpu copies start at the top left.
pub fn unpack_rows(bytes: &[u8], width: u32, height: u32) -> Result<Vec<u8>, String> {
    if width == 0 || height == 0 || width > 8192 || height > 8192 {
        return Err("invalid readback size".into());
    }
    let stride = row_stride(width) as usize;
    if bytes.len() != stride * height as usize {
        return Err("incomplete GPU readback".into());
    }
    let mut rgba = Vec::with_capacity(width as usize * height as usize * 4);
    for row in bytes.chunks_exact(stride) {
        rgba.extend_from_slice(&row[..width as usize * 4]);
    }
    Ok(rgba)
}
impl SceneRenderer {
    /// Read the same offscreen pass used by the viewport. Caller installs GPU error scopes
    /// around renderer construction, uploads and this call, and checks them before encoding.
    pub fn capture(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        matrix: [[f32; 4]; 4],
        size: [u32; 2],
    ) -> Result<Vec<u8>, String> {
        if size.contains(&0)
            || size
                .iter()
                .any(|&v| v > 8192 || v > device.limits().max_texture_dimension_2d)
        {
            return Err("PNG size exceeds GPU texture limit (8192 maximum)".into());
        }
        // One-shot readback must see every staged texture layer; finish all
        // pending uploads instead of the render thread's one-layer-per-frame.
        while self.texture_staging_active() {
            self.pump_texture_upload(device, queue);
        }
        let mut encoder = device.create_command_encoder(&Default::default());
        self.render(device, queue, &mut encoder, matrix, size);
        let stride = row_stride(size[0]);
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("PNG readback"),
            size: u64::from(stride) * u64::from(size[1]),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        encoder.copy_texture_to_buffer(
            self.capture_texture().as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(stride),
                    rows_per_image: Some(size[1]),
                },
            },
            wgpu::Extent3d {
                width: size[0],
                height: size[1],
                depth_or_array_layers: 1,
            },
        );
        queue.submit([encoder.finish()]);
        let (sender, receiver) = std::sync::mpsc::sync_channel(1);
        buffer.slice(..).map_async(wgpu::MapMode::Read, move |r| {
            let _ = sender.send(r);
        });
        device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: Some(Duration::from_secs(30)),
            })
            .map_err(|e| format!("PNG GPU completion failed: {e}"))?;
        receiver
            .recv_timeout(Duration::from_secs(1))
            .map_err(|e| e.to_string())?
            .map_err(|e| format!("PNG readback failed: {e}"))?;
        let result = unpack_rows(&buffer.slice(..).get_mapped_range(), size[0], size[1]);
        buffer.unmap();
        result
    }
}
#[cfg(test)]
mod tests {
    #[test]
    fn padding_and_orientation() {
        let mut bytes = vec![99; 512];
        bytes[..4].copy_from_slice(&[1, 2, 3, 4]);
        bytes[256..260].copy_from_slice(&[5, 6, 7, 8]);
        assert_eq!(
            super::unpack_rows(&bytes, 1, 2).unwrap(),
            [1, 2, 3, 4, 5, 6, 7, 8]
        );
        assert_eq!(super::row_stride(64), 256);
        assert!(super::unpack_rows(&bytes, 2, 3).is_err());
    }
}
