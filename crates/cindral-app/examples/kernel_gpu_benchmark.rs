//! Full versus dirty-range CPU mesh/GPU buffer updates after a single-face edit.
use cindral_render::mesh::{GeometryBuffers, MeshCache, MeshUpdate};
use eframe::{
    egui,
    egui_wgpu::{self, wgpu},
};
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Instant,
};
fn wait(device: &wgpu::Device, queue: &wgpu::Queue) -> Result<(), String> {
    let submission = queue.submit(std::iter::empty());
    device
        .poll(wgpu::PollType::Wait {
            submission_index: Some(submission),
            timeout: None,
        })
        .map_err(|e| e.to_string())?;
    Ok(())
}
fn bytes(update: &MeshUpdate) -> usize {
    update
        .face_writes
        .iter()
        .chain(&update.edge_writes)
        .map(|write| write.bytes.len())
        .sum()
}
fn run(render: &egui_wgpu::RenderState) -> Result<(), String> {
    let mut fixture =
        cindral_core::benchmark::tiled_squares(100, 100).map_err(|e| e.to_string())?;
    let (mut cache, initial) =
        MeshCache::rebuild(fixture.history.model()).map_err(|e| e.to_string())?;
    let mut buffers = GeometryBuffers::new(&render.device);
    buffers.apply(&render.device, &render.queue, &initial);
    wait(&render.device, &render.queue)?;
    fixture
        .history
        .execute(fixture.edit)
        .map_err(|e| e.to_string())?;
    let info = render.adapter.get_info();
    println!(
        "adapter: {} {:?}; fixture 50000 edges / 20000 faces before edit",
        info.name, info.backend
    );
    let start = Instant::now();
    let (_full_cache, full) =
        MeshCache::rebuild(fixture.history.model()).map_err(|e| e.to_string())?;
    println!(
        "full_cpu_mesh_ms: {:.3}; bytes {}",
        start.elapsed().as_secs_f64() * 1000.,
        bytes(&full)
    );
    let start = Instant::now();
    let mut full_buffers = GeometryBuffers::new(&render.device);
    full_buffers.apply(&render.device, &render.queue, &full);
    wait(&render.device, &render.queue)?;
    println!(
        "full_gpu_buffer_ms: {:.3}; allocated {}",
        start.elapsed().as_secs_f64() * 1000.,
        full_buffers.allocated_bytes()
    );
    let change = fixture.history.changes();
    let faces: Vec<_> = change.faces.iter().copied().collect();
    let edges: Vec<_> = change.edges.iter().copied().collect();
    let start = Instant::now();
    let partial = cache
        .update(fixture.history.model(), &faces, &edges)
        .map_err(|e| e.to_string())?;
    println!(
        "partial_cpu_mesh_ms: {:.3}; bytes {}; dirty_faces {}; dirty_edges {}",
        start.elapsed().as_secs_f64() * 1000.,
        bytes(&partial),
        faces.len(),
        edges.len()
    );
    let start = Instant::now();
    buffers.apply(&render.device, &render.queue, &partial);
    wait(&render.device, &render.queue)?;
    println!(
        "partial_gpu_buffer_ms: {:.3}; allocated {}",
        start.elapsed().as_secs_f64() * 1000.,
        buffers.allocated_bytes()
    );
    Ok(())
}
struct Bench {
    render: egui_wgpu::RenderState,
    done: bool,
    failed: Arc<AtomicBool>,
}
impl eframe::App for Bench {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if !self.done {
            self.done = true;
            if let Err(e) = run(&self.render) {
                eprintln!("{e}");
                self.failed.store(true, Ordering::Relaxed);
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }
}
fn main() -> eframe::Result {
    let failed = Arc::new(AtomicBool::new(false));
    let app_failed = failed.clone();
    eframe::run_native(
        "Cindral kernel buffer benchmark",
        eframe::NativeOptions {
            renderer: eframe::Renderer::Wgpu,
            ..Default::default()
        },
        Box::new(move |cc| {
            Ok(Box::new(Bench {
                render: cc.wgpu_render_state.clone().ok_or("wgpu required")?,
                done: false,
                failed: app_failed,
            }))
        }),
    )?;
    if failed.load(Ordering::Relaxed) {
        std::process::exit(1);
    }
    Ok(())
}
