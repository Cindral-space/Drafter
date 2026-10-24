//! Release-only pan workload using full production UI/GPU frames.
use super::*;
use egui::{Event, Pos2};
use std::{path::Path, time::Instant};
fn frame(app: &mut DrafterApp, ctx: &egui::Context, gpu: &mut real_file_gpu::Gpu, events: Vec<Event>) -> f64 {
    let start = Instant::now();
    let out = ctx.run(egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(1280., 800.))),
        events, ..Default::default()
    }, |ctx| app.draw_frame(ctx));
    let jobs = ctx.tessellate(out.shapes, out.pixels_per_point);
    gpu.draw(&jobs, out.textures_delta, out.pixels_per_point);
    start.elapsed().as_secs_f64() * 1000.
}
fn button(pos: Pos2, pressed: bool) -> Event {
    Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE }
}
/// An external watchdog in the example enforces a hard timeout including GPU startup/import.
/// No adapter is an explicit skip, never a CPU-only timing presented as GPU timing.
pub fn run(path: &Path) -> Result<(), String> {
    if cfg!(debug_assertions) { return Err("pan benchmark requires --release".into()); }
    let mut gpu = match real_file_gpu::Gpu::new() {
        Ok(gpu) => gpu,
        Err(e) => { println!("SKIP GPU pan benchmark: {e}"); return Ok(()); }
    };
    gpu.allow_shadows();
    let mut app = DrafterApp::default();
    let source = match app.load_replay(path) {
        Ok(()) => "real file",
        Err(e) => {
            println!("Real file unavailable headlessly ({e}); using synthetic scene");
            app = DrafterApp::default();
            app.execute(Command::AddPolyline { points: vec![DVec3::ZERO, DVec3::X * 10., DVec3::new(10., 10., 0.), DVec3::Y * 10.], closed: true }).map_err(|e| e.to_string())?;
            "synthetic"
        }
    };
    app.isolate_pan_replay();
    let ctx = egui::Context::default();
    workspace_ui::configure_style(&ctx);
    app.activate_pan();
    frame(&mut app, &ctx, &mut gpu, vec![]);
    app.extents(app.replay_rect.aspect_ratio());
    let original = app.camera.clone();
    let revision = app.history.revision();
    let history = (app.history.undo_depth(), app.history.redo_depth());
    let mut worst: f64 = 0.;
    for shadows in [false, true] {
        app.presentation.shadows.enabled = shadows;
        app.camera = original.clone();
        let p = app.replay_rect.center();
        for _ in 0..10 { frame(&mut app, &ctx, &mut gpu, vec![Event::PointerMoved(p)]); }
        let start = Instant::now();
        let press_ms = frame(&mut app, &ctx, &mut gpu, vec![button(p, true)]);
        if app.pan_drag.is_none() { return Err("pan press was not captured".into()); }
        let metrics = gpu.pan_metrics();
        if shadows { assert!(metrics.0 > 0, "enabled shadows must render during warmup"); }
        let mut max: f64 = 0.;
        let mut total = 0.;
        for i in 1..=200 {
            let pos = p + egui::vec2((i as f32 * 0.1).sin() * 80., (i as f32 * 0.13).sin() * 40.);
            let dt = frame(&mut app, &ctx, &mut gpu, vec![Event::PointerMoved(pos)]);
            max = max.max(dt);
            total += dt;
            assert!(app.pending.is_none(), "pan must not queue geometry updates");
            assert_eq!(app.history.revision(), revision);
            assert_eq!(gpu.pan_metrics(), metrics, "pan rebuilt shadows or resized GPU buffers");
        }
        frame(&mut app, &ctx, &mut gpu, vec![button(p, false)]);
        assert_ne!(app.camera.target, original.target, "scripted pan must move the camera");
        assert_eq!((app.history.undo_depth(), app.history.redo_depth()), history);
        worst = worst.max(max);
        println!("{source}, shadows={shadows}, 1280x800 full UI + GPU wait: press={press_ms:.3}ms, 200 moves mean={:.3}ms worst={max:.3}ms total={:.3}s; no shadow rerenders/GPU allocation growth/mesh updates", total / 200., start.elapsed().as_secs_f64());
    }
    if worst >= 33. { return Err(format!("minimum 33ms pan budget missed: {worst:.3}ms")); }
    Ok(())
}
