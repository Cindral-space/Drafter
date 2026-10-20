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
    let face =
        cindral_core::benchmark::add_test_solid(&mut fixture.history).map_err(|e| e.to_string())?;
    let (seed_cache, setup_update) =
        MeshCache::rebuild(fixture.history.model()).map_err(|e| e.to_string())?;
    cache = seed_cache;
    buffers.apply(&render.device, &render.queue, &setup_update);
    wait(&render.device, &render.queue)?;
    println!(
        "m4_fixture: {} edges / {} faces; isolated closed box",
        fixture.history.model().edges().len(),
        fixture.history.model().faces().len()
    );
    let undo_depth = fixture.history.undo_depth();
    let mut frames = Vec::new();
    let mut cpu = Vec::new();
    let mut mesh = Vec::new();
    let mut gpu = Vec::new();
    for i in 1..=30 {
        let total = Instant::now();
        let start = Instant::now();
        fixture
            .history
            .preview(cindral_core::Command::PushPull {
                face,
                distance: 0.1 + i as f64 * 0.01,
            })
            .map_err(|e| e.to_string())?;
        cpu.push(start.elapsed().as_secs_f64() * 1000.);
        let dirty = fixture.history.changes();
        let faces = dirty.faces.iter().copied().collect::<Vec<_>>();
        let edges = dirty.edges.iter().copied().collect::<Vec<_>>();
        let start = Instant::now();
        let partial = cache
            .update(fixture.history.model(), &faces, &edges)
            .map_err(|e| e.to_string())?;
        mesh.push(start.elapsed().as_secs_f64() * 1000.);
        let start = Instant::now();
        buffers.apply(&render.device, &render.queue, &partial);
        wait(&render.device, &render.queue)?;
        gpu.push(start.elapsed().as_secs_f64() * 1000.);
        frames.push(total.elapsed().as_secs_f64() * 1000.);
    }
    fixture
        .history
        .commit_preview()
        .map_err(|e| e.to_string())?;
    for (name, samples) in [
        ("m4_kernel", cpu),
        ("m4_mesh", mesh),
        ("m4_gpu_completed", gpu),
        ("m4_total_update", frames),
    ] {
        println!(
            "{name}_ms: mean {:.3}; max {:.3}; samples {}",
            samples.iter().sum::<f64>() / samples.len() as f64,
            samples.iter().copied().fold(0., f64::max),
            samples.len()
        );
    }
    println!(
        "m4_preview_undo_entries: {}",
        fixture.history.undo_depth() - undo_depth
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
