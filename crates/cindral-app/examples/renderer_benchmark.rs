//! Static large-mesh renderer baseline (frozen contract in `docs/ARCHITECTURE.md`).
//!
//! Run `WGPU_BACKEND=opengl cargo run --release -p cindral-app --example renderer_benchmark`.
//! See `docs/RENDERER_BENCHMARK.md` for methodology and limitations.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use eframe::egui;
use eframe::egui_wgpu::{self, wgpu};
use glam::Vec3;
use wgpu::util::DeviceExt;

/// Quad cells per side of the heightfield.
const CELLS: usize = 500;
/// Offscreen target width in pixels.
const WIDTH: u32 = 1280;
/// Offscreen target height in pixels.
const HEIGHT: u32 = 800;
/// Unmeasured submissions executed first.
const WARMUP: usize = 20;
/// Measured submissions.
const MEASURED: usize = 100;
/// Frame budget used only as a reference line (60 Hz).
const BUDGET_MS: f64 = 1000.0 / 60.0;
/// Colour format of the offscreen target.
const COLOR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
/// Depth format of the offscreen target.
const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
/// Clear colour as bytes (RGBA8) used to detect covered pixels.
const CLEAR_RGBA8: [u8; 4] = [13, 15, 20, 255];
/// Clear colour in linear floats matching [`CLEAR_RGBA8`].
const CLEAR: wgpu::Color = wgpu::Color {
    r: 13.0 / 255.0,
    g: 15.0 / 255.0,
    b: 20.0 / 255.0,
    a: 1.0,
};

/// CPU-side indexed mesh.
struct Mesh {
    /// Interleaved position (xyz) and normal (xyz) per vertex.
    vertices: Vec<[f32; 6]>,
    /// Triangle list indices.
    indices: Vec<u32>,
}

impl Mesh {
    fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }
}

/// Analytic terrain height at (x, y).
fn height(x: f32, y: f32) -> f32 {
    12.0 * (x * 0.021).sin() * (y * 0.017).cos() + 5.0 * (x * 0.07 + y * 0.05).sin()
}

/// Analytic partial derivatives (dz/dx, dz/dy) of [`height`].
fn gradient(x: f32, y: f32) -> (f32, f32) {
    let dx = 12.0 * 0.021 * (x * 0.021).cos() * (y * 0.017).cos()
        + 5.0 * 0.07 * (x * 0.07 + y * 0.05).cos();
    let dy = -12.0 * 0.017 * (x * 0.021).sin() * (y * 0.017).sin()
        + 5.0 * 0.05 * (x * 0.07 + y * 0.05).cos();
    (dx, dy)
}

/// Builds a `cells`×`cells` heightfield, 1 unit per cell, centred on the origin.
/// Triangles are counter-clockwise when viewed from +Z.
fn build_heightfield(cells: usize) -> Mesh {
    let side = cells + 1;
    let half = cells as f32 * 0.5;
    let mut vertices = Vec::with_capacity(side * side);
    for j in 0..side {
        for i in 0..side {
            let x = i as f32 - half;
            let y = j as f32 - half;
            let (dx, dy) = gradient(x, y);
            let n = Vec3::new(-dx, -dy, 1.0).normalize();
            vertices.push([x, y, height(x, y), n.x, n.y, n.z]);
        }
    }
    let mut indices = Vec::with_capacity(cells * cells * 6);
    for j in 0..cells {
        for i in 0..cells {
            let a = (j * side + i) as u32;
            let b = a + 1;
            let d = a + side as u32;
            let c = d + 1;
            indices.extend_from_slice(&[a, b, c, a, c, d]);
        }
    }
    Mesh { vertices, indices }
}

/// Axis-aligned bounds of the mesh positions.
fn bounds(mesh: &Mesh) -> (Vec3, Vec3) {
    mesh.vertices.iter().fold(
        (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)),
        |(lo, hi), v| {
            let p = Vec3::new(v[0], v[1], v[2]);
            (lo.min(p), hi.max(p))
        },
    )
}

fn f32_bytes(values: impl Iterator<Item = f32>) -> Vec<u8> {
    values.flat_map(f32::to_ne_bytes).collect()
}

/// Summary statistics of measured durations, in milliseconds.
#[derive(Debug, PartialEq)]
struct Stats {
    mean: f64,
    p50: f64,
    p95: f64,
    max: f64,
}

/// Computes mean and nearest-rank percentiles. Returns `None` for no samples.
fn summarize(samples_ms: &[f64]) -> Option<Stats> {
    if samples_ms.is_empty() {
        return None;
    }
    let mut sorted = samples_ms.to_vec();
    sorted.sort_by(f64::total_cmp);
    let rank = |p: f64| {
        let idx = ((p * sorted.len() as f64).ceil() as usize).clamp(1, sorted.len()) - 1;
        sorted[idx]
    };
    Some(Stats {
        mean: sorted.iter().sum::<f64>() / sorted.len() as f64,
        p50: rank(0.50),
        p95: rank(0.95),
        max: sorted[sorted.len() - 1],
    })
}

/// Minimal executor for wgpu futures that complete after `device.poll`.
fn block_on<F: std::future::Future>(future: F) -> F::Output {
    struct Waker(std::thread::Thread);
    impl std::task::Wake for Waker {
        fn wake(self: Arc<Self>) {
            self.0.unpark();
        }
    }
    let mut future = std::pin::pin!(future);
    let waker = Arc::new(Waker(std::thread::current())).into();
    let mut cx = std::task::Context::from_waker(&waker);
    loop {
        if let std::task::Poll::Ready(value) = future.as_mut().poll(&mut cx) {
            return value;
        }
        std::thread::park_timeout(Duration::from_millis(10));
    }
}

/// GPU resources that are created once, outside timing.
struct Scene {
    pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    index_count: u32,
    color_view: wgpu::TextureView,
    depth_view: wgpu::TextureView,
    color: wgpu::Texture,
}

fn create_scene(device: &wgpu::Device, queue: &wgpu::Queue, mesh: &Mesh) -> Scene {
    let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("benchmark vertices"),
        contents: &f32_bytes(mesh.vertices.iter().flatten().copied()),
        usage: wgpu::BufferUsages::VERTEX,
    });
    let index_bytes: Vec<u8> = mesh.indices.iter().flat_map(|i| i.to_ne_bytes()).collect();
    let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("benchmark indices"),
        contents: &index_bytes,
        usage: wgpu::BufferUsages::INDEX,
    });

    let mut camera = cindral_render::camera::Camera::default();
    let (min, max) = bounds(mesh);
    let aspect = WIDTH as f32 / HEIGHT as f32;
    camera.zoom_extents(min, max, aspect);
    let mut uniform = camera.view_projection(aspect).to_cols_array().to_vec();
    uniform.extend_from_slice(&[0.4, 0.3, 0.85, 0.0]);
    let uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("benchmark uniforms"),
        contents: &f32_bytes(uniform.into_iter()),
        usage: wgpu::BufferUsages::UNIFORM,
    });
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("benchmark bind layout"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        }],
    });
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("benchmark bind group"),
        layout: &layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: uniform_buffer.as_entire_binding(),
        }],
    });
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("benchmark shader"),
        source: wgpu::ShaderSource::Wgsl(include_str!("renderer_benchmark.wgsl").into()),
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("benchmark pipeline layout"),
        bind_group_layouts: &[&layout],
        push_constant_ranges: &[],
    });
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("benchmark pipeline"),
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_main"),
            compilation_options: Default::default(),
            buffers: &[wgpu::VertexBufferLayout {
                array_stride: 24,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3],
            }],
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_main"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: COLOR_FORMAT,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            front_face: wgpu::FrontFace::Ccw,
            cull_mode: Some(wgpu::Face::Back),
            ..Default::default()
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: DEPTH_FORMAT,
            depth_write_enabled: true,
            depth_compare: wgpu::CompareFunction::Less,
            stencil: Default::default(),
            bias: Default::default(),
        }),
        multisample: wgpu::MultisampleState::default(),
        multiview: None,
        cache: None,
    });
    let size = wgpu::Extent3d {
        width: WIDTH,
        height: HEIGHT,
        depth_or_array_layers: 1,
    };
    let make = |label, format, usage| {
        device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage,
            view_formats: &[],
        })
    };
    let color = make(
        "benchmark color",
        COLOR_FORMAT,
        wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
    );
    let depth = make(
        "benchmark depth",
        DEPTH_FORMAT,
        wgpu::TextureUsages::RENDER_ATTACHMENT,
    );
    // Make sure uploads are finished before any timing starts.
    queue.submit(std::iter::empty());
    Scene {
        pipeline,
        bind_group,
        vertex_buffer,
        index_buffer,
        index_count: mesh.indices.len() as u32,
        color_view: color.create_view(&Default::default()),
        depth_view: depth.create_view(&Default::default()),
        color,
    }
}

/// Encodes one render pass into `encoder`.
fn encode_draw(encoder: &mut wgpu::CommandEncoder, scene: &Scene) {
    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("benchmark pass"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: &scene.color_view,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(CLEAR),
                store: wgpu::StoreOp::Store,
            },
        })],
        depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
            view: &scene.depth_view,
            depth_ops: Some(wgpu::Operations {
                load: wgpu::LoadOp::Clear(1.0),
                store: wgpu::StoreOp::Discard,
            }),
            stencil_ops: None,
        }),
        timestamp_writes: None,
        occlusion_query_set: None,
    });
    pass.set_pipeline(&scene.pipeline);
    pass.set_bind_group(0, &scene.bind_group, &[]);
    pass.set_vertex_buffer(0, scene.vertex_buffer.slice(..));
    pass.set_index_buffer(scene.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
    pass.draw_indexed(0..scene.index_count, 0, 0..1);
}

/// One timed unit: encode + submit + wait for GPU completion.
fn render_once(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    scene: &Scene,
) -> Result<Duration, String> {
    let start = Instant::now();
    let mut encoder = device.create_command_encoder(&Default::default());
    encode_draw(&mut encoder, scene);
    let index = queue.submit([encoder.finish()]);
    device
        .poll(wgpu::PollType::Wait {
            submission_index: Some(index),
            timeout: None,
        })
        .map_err(|e| format!("device.poll failed: {e}"))?;
    Ok(start.elapsed())
}

/// Renders once more, reads the colour target back and returns the fraction of
/// pixels that differ from the clear colour. Untimed.
fn verify_coverage(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    scene: &Scene,
) -> Result<f64, String> {
    let row_bytes = WIDTH * 4;
    debug_assert_eq!(row_bytes % wgpu::COPY_BYTES_PER_ROW_ALIGNMENT, 0);
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("benchmark readback"),
        size: u64::from(row_bytes * HEIGHT),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    encode_draw(&mut encoder, scene);
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: &scene.color,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(row_bytes),
                rows_per_image: Some(HEIGHT),
            },
        },
        wgpu::Extent3d {
            width: WIDTH,
            height: HEIGHT,
            depth_or_array_layers: 1,
        },
    );
    queue.submit([encoder.finish()]);
    let (tx, rx) = std::sync::mpsc::channel();
    buffer.slice(..).map_async(wgpu::MapMode::Read, move |r| {
        let _ = tx.send(r);
    });
    device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        })
        .map_err(|e| format!("device.poll failed during readback: {e}"))?;
    rx.recv()
        .map_err(|e| format!("readback callback lost: {e}"))?
        .map_err(|e| format!("readback map failed: {e}"))?;
    let data = buffer.slice(..).get_mapped_range();
    let (pixels, _) = data.as_chunks::<4>();
    let covered = pixels
        .iter()
        .filter(|px| px[..3] != CLEAR_RGBA8[..3])
        .count();
    drop(data);
    buffer.unmap();
    Ok(covered as f64 / f64::from(WIDTH * HEIGHT))
}

/// Runs the whole benchmark and prints the report.
fn run(render: &egui_wgpu::RenderState) -> Result<(), String> {
    let device = &render.device;
    let queue = &render.queue;
    let info = render.adapter.get_info();

    let mesh = build_heightfield(CELLS);
    device.push_error_scope(wgpu::ErrorFilter::Validation);
    let scene = create_scene(device, queue, &mesh);
    let coverage = verify_coverage(device, queue, &scene)?;
    if let Some(error) = block_on(device.pop_error_scope()) {
        return Err(format!("wgpu validation error during setup: {error}"));
    }
    if coverage < 0.01 {
        return Err(format!(
            "raster verification failed: only {:.2}% of pixels covered",
            coverage * 100.0
        ));
    }

    device.push_error_scope(wgpu::ErrorFilter::Validation);
    for _ in 0..WARMUP {
        render_once(device, queue, &scene)?;
    }
    let mut samples = Vec::with_capacity(MEASURED);
    for _ in 0..MEASURED {
        samples.push(render_once(device, queue, &scene)?.as_secs_f64() * 1000.0);
    }
    if let Some(error) = block_on(device.pop_error_scope()) {
        return Err(format!("wgpu validation error while rendering: {error}"));
    }
    let stats = summarize(&samples).ok_or("no samples collected")?;

    let triangles = mesh.triangle_count();
    let index_bytes = mesh.indices.len() * std::mem::size_of::<u32>();
    let profile = if cfg!(debug_assertions) {
        "debug (debug assertions on)"
    } else {
        "release (debug assertions off)"
    };
    println!("Cindral renderer benchmark: simple one-draw static baseline");
    println!(
        "adapter:      {} ({:?}, {:?})",
        info.name, info.backend, info.device_type
    );
    println!("target:       {WIDTH}x{HEIGHT} {COLOR_FORMAT:?} + {DEPTH_FORMAT:?}");
    println!("grid:         {CELLS}x{CELLS} cells");
    println!("triangles:    {triangles}");
    println!("vertices:     {}", mesh.vertices.len());
    println!(
        "index memory: {index_bytes} bytes ({:.2} MiB)",
        index_bytes as f64 / (1024.0 * 1024.0)
    );
    println!("build:        {profile}");
    println!("raster check: {:.1}% of pixels covered", coverage * 100.0);
    println!("warmup/measured: {WARMUP}/{MEASURED} submissions");
    println!(
        "frame ms:     mean {:.3}  p50 {:.3}  p95 {:.3}  max {:.3}",
        stats.mean, stats.p50, stats.p95, stats.max
    );
    println!(
        "throughput:   {:.1} frames/s, {:.1} Mtriangles/s (at mean)",
        1000.0 / stats.mean,
        triangles as f64 / (stats.mean / 1000.0) / 1.0e6
    );
    println!(
        "vs {BUDGET_MS:.2} ms: mean {:.2}x, p95 {:.2}x of budget (reference only; not full editor performance)",
        stats.mean / BUDGET_MS,
        stats.p95 / BUDGET_MS
    );
    Ok(())
}

struct Benchmark {
    render: egui_wgpu::RenderState,
    done: bool,
    failed: Arc<AtomicBool>,
}

impl eframe::App for Benchmark {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if !self.done {
            self.done = true;
            if let Err(error) = run(&self.render) {
                eprintln!("Benchmark failed: {error}");
                self.failed.store(true, Ordering::SeqCst);
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        ctx.request_repaint();
    }
}

fn main() -> eframe::Result {
    let failed = Arc::new(AtomicBool::new(false));
    let app_failed = failed.clone();
    let options = eframe::NativeOptions {
        renderer: eframe::Renderer::Wgpu,
        viewport: egui::ViewportBuilder::default().with_inner_size([640.0, 400.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Cindral Drafter · renderer benchmark",
        options,
        Box::new(move |cc| {
            let render = cc
                .wgpu_render_state
                .clone()
                .ok_or("renderer_benchmark requires the wgpu backend")?;
            Ok(Box::new(Benchmark {
                render,
                done: false,
                failed: app_failed,
            }))
        }),
    )?;
    if failed.load(Ordering::SeqCst) {
        std::process::exit(1);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pos(mesh: &Mesh, i: u32) -> Vec3 {
        let v = mesh.vertices[i as usize];
        Vec3::new(v[0], v[1], v[2])
    }

    #[test]
    fn counts_match_contract() {
        let mesh = build_heightfield(CELLS);
        assert_eq!(mesh.triangle_count(), 500_000);
        assert_eq!(mesh.vertices.len(), 501 * 501);
        assert_eq!(mesh.indices.len(), 1_500_000);
    }

    #[test]
    fn indices_are_valid() {
        let mesh = build_heightfield(CELLS);
        let count = mesh.vertices.len() as u32;
        assert!(mesh.indices.iter().all(|&i| i < count));
        let mut used = vec![false; count as usize];
        for &i in &mesh.indices {
            used[i as usize] = true;
        }
        assert!(used.iter().all(|&u| u));
    }

    #[test]
    fn normals_are_unit_upward_and_match_surface() {
        let mesh = build_heightfield(CELLS);
        for v in &mesh.vertices {
            let n = Vec3::new(v[3], v[4], v[5]);
            assert!(n.is_finite());
            assert!((n.length() - 1.0).abs() < 1e-4);
            assert!(n.z > 0.0);
        }
        let small = build_heightfield(4);
        for tri in small.indices.as_chunks::<3>().0 {
            let (a, b, c) = (
                pos(&small, tri[0]),
                pos(&small, tri[1]),
                pos(&small, tri[2]),
            );
            let face = (b - a).cross(c - a).normalize();
            for &i in tri {
                let v = small.vertices[i as usize];
                assert!(face.dot(Vec3::new(v[3], v[4], v[5])) > 0.9);
            }
        }
    }

    #[test]
    fn triangles_are_nondegenerate_and_counter_clockwise() {
        let mesh = build_heightfield(CELLS);
        for tri in mesh.indices.as_chunks::<3>().0 {
            let (a, b, c) = (pos(&mesh, tri[0]), pos(&mesh, tri[1]), pos(&mesh, tri[2]));
            let cross = (b - a).cross(c - a);
            assert!(cross.length() > 1e-3, "degenerate triangle {tri:?}");
            assert!(cross.z > 0.0, "triangle {tri:?} faces away from +Z");
        }
    }

    #[test]
    fn statistics_use_nearest_rank() {
        let samples: Vec<f64> = (1..=100).map(f64::from).collect();
        let s = summarize(&samples).unwrap();
        assert_eq!(s.p50, 50.0);
        assert_eq!(s.p95, 95.0);
        assert_eq!(s.max, 100.0);
        assert!((s.mean - 50.5).abs() < 1e-12);
        assert!(summarize(&[]).is_none());
    }
}
