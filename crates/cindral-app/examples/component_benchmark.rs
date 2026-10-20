//! Native M5 shared-definition and 5000-component renderer benchmark.
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
fn run(render: &egui_wgpu::RenderState) -> Result<(), String> {
    if let Ok(segments) = std::env::var("CINDRAL_CONNECTED_SEGMENTS") {
        let segments = segments.parse::<usize>().map_err(|e| e.to_string())?;
        let mode = std::env::var("CINDRAL_CONNECTED_MODE").unwrap_or_else(|_| "pushpull".into());
        let mut renderer =
            cindral_render::scene::SceneRenderer::new(&render.device, render.target_format, 1);
        return cindral_app::organization_benchmark::run_connected(segments, &mode, |update| {
            renderer.apply_update(&render.device, &render.queue, update);
            wait(&render.device, &render.queue)
        });
    }
    let workload = cindral_app::organization_benchmark::run_cpu().map_err(|e| e.to_string())?;
    let info = render.adapter.get_info();
    println!(
        "Adapter: {} ({:?}/{:?})",
        info.name, info.backend, info.device_type
    );
    let mut renderer =
        cindral_render::scene::SceneRenderer::new(&render.device, render.target_format, 1);
    let start = Instant::now();
    renderer.apply_update(&render.device, &render.queue, &workload.initial);
    wait(&render.device, &render.queue)?;
    println!(
        "initial GPU geometry + 5000 matrices upload and wait: {:.3} ms; allocated bytes: {}",
        start.elapsed().as_secs_f64() * 1000.,
        renderer.allocated_bytes()
    );
    let vp = glam::Mat4::orthographic_rh(-110., 110., -65., 65., 0.1, 1000.)
        * glam::Mat4::look_at_rh(
            glam::Vec3::new(100., 50., 300.),
            glam::Vec3::new(100., 50., 0.),
            glam::Vec3::Y,
        );
    let mut times = Vec::new();
    for i in 0..120 {
        let start = Instant::now();
        let mut encoder = render.device.create_command_encoder(&Default::default());
        renderer.render(
            &render.device,
            &render.queue,
            &mut encoder,
            vp.to_cols_array_2d(),
            [1280, 800],
        );
        render.queue.submit([encoder.finish()]);
        wait(&render.device, &render.queue)?;
        if i >= 20 {
            times.push(start.elapsed().as_secs_f64() * 1000.);
        }
    }
    times.sort_by(f64::total_cmp);
    println!("5000 boxes / 60000 submitted triangles (opaque+transparent shader passes), 1280x800, 20 warmup/100 measured frames: mean {:.3} ms; p95 {:.3} ms; draw calls {}",times.iter().sum::<f64>()/times.len() as f64,times[94],renderer.draw_calls());
    for (label, update) in [
        ("shared edit", &workload.edit),
        ("one placement", &workload.placement),
    ] {
        let start = Instant::now();
        renderer.apply_update(&render.device, &render.queue, update);
        wait(&render.device, &render.queue)?;
        println!(
            "{label} GPU update + wait: {:.3} ms; geometry definitions: {}; matrix batches: {}",
            start.elapsed().as_secs_f64() * 1000.,
            update.geometry.len(),
            update.batches.as_ref().map_or(0, Vec::len)
        );
    }
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
        "Cindral component benchmark",
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
