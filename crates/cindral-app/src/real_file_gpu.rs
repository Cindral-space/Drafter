//! Offscreen egui/wgpu frame submission using the production callbacks.
use eframe::{
    egui,
    egui_wgpu::{self, wgpu},
};
use std::{
    future::Future,
    sync::Arc,
    task::{Context, Poll, Wake, Waker},
};
struct ThreadWake(std::thread::Thread);
impl Wake for ThreadWake {
    fn wake(self: Arc<Self>) {
        self.0.unpark();
    }
}
fn block_on<T>(future: impl Future<Output = T>) -> T {
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
pub(super) struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    renderer: egui_wgpu::Renderer,
    target: wgpu::TextureView,
    allow_shadows: bool,
}
impl Gpu {
    pub(super) fn new() -> Result<Self, String> {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::from_env_or_default());
        let adapter =
            block_on(instance.request_adapter(&Default::default())).map_err(|e| e.to_string())?;
        println!("Adapter: {:?}", adapter.get_info());
        if adapter.get_info().device_type == wgpu::DeviceType::Cpu {
            return Err("software adapter is not a hardware measurement".into());
        }
        let (device, queue) =
            block_on(adapter.request_device(&Default::default())).map_err(|e| e.to_string())?;
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let mut renderer =
            egui_wgpu::Renderer::new(&device, format, egui_wgpu::RendererOptions::default());
        renderer
            .callback_resources
            .insert(cindral_render::grid::GridRenderer::new(&device, format, 1));
        renderer
            .callback_resources
            .insert(cindral_render::scene::SceneRenderer::new(
                &device, format, 1,
            ));
        let target = device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("real-file replay"),
                size: wgpu::Extent3d {
                    width: 1280,
                    height: 800,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            })
            .create_view(&Default::default());
        Ok(Self {
            device,
            queue,
            renderer,
            target,
            allow_shadows: false,
        })
    }
    pub(super) fn allow_shadows(&mut self) {
        self.allow_shadows = true;
    }
    pub(super) fn pan_metrics(&self) -> (u64, u64) {
        let renderer = self.renderer.callback_resources.get::<cindral_render::scene::SceneRenderer>().unwrap();
        (renderer.shadow_renders(), renderer.allocated_bytes())
    }
    pub(super) fn draw(
        &mut self,
        jobs: &[egui::ClippedPrimitive],
        textures: egui::TexturesDelta,
        ppp: f32,
    ) {
        for (id, delta) in textures.set {
            self.renderer
                .update_texture(&self.device, &self.queue, id, &delta);
        }
        let screen = egui_wgpu::ScreenDescriptor {
            size_in_pixels: [1280, 800],
            pixels_per_point: ppp,
        };
        let mut encoder = self.device.create_command_encoder(&Default::default());
        let mut commands =
            self.renderer
                .update_buffers(&self.device, &self.queue, &mut encoder, jobs, &screen);
        {
            let mut pass = encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("full egui replay"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &self.target,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                            store: wgpu::StoreOp::Store,
                        },
                        depth_slice: None,
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                })
                .forget_lifetime();
            self.renderer.render(&mut pass, jobs, &screen);
        }
        commands.push(encoder.finish());
        let submission = self.queue.submit(commands);
        self.device
            .poll(wgpu::PollType::Wait {
                submission_index: Some(submission),
                timeout: Some(std::time::Duration::from_secs(5)),
            })
            .expect("GPU frame completion");
        for id in textures.free {
            self.renderer.free_texture(&id);
        }
        if !self.allow_shadows {
        assert_eq!(
            self.renderer
                .callback_resources
                .get::<cindral_render::scene::SceneRenderer>()
                .unwrap()
                .shadow_renders(),
            0,
            "shadows are disabled throughout this replay"
        );
        }
    }
}
