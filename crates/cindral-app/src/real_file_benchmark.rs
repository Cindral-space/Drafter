//! Private-file replay through production importer, pointer routing and full egui layout.
use super::*;
use egui::{Event, Pos2};
use std::{path::Path, time::Instant};
thread_local! { static GPU: std::cell::RefCell<Option<super::real_file_gpu::Gpu>> = const { std::cell::RefCell::new(None) }; }
fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.
}
fn button(pos: Pos2, pressed: bool) -> Event {
    Event::PointerButton {
        pos,
        button: PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    }
}
fn frame(app: &mut DrafterApp, ctx: &egui::Context, events: Vec<Event>) -> f64 {
    let t = Instant::now();
    let out = ctx.run(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                Pos2::ZERO,
                egui::vec2(1280., 800.),
            )),
            events,
            ..Default::default()
        },
        |ctx| app.draw_frame(ctx),
    );
    let jobs = ctx.tessellate(out.shapes, out.pixels_per_point);
    GPU.with_borrow_mut(|gpu| {
        if let Some(gpu) = gpu {
            let _phase = cindral_core::profile::Scope::new(9);
            gpu.draw(&jobs, out.textures_delta, out.pixels_per_point);
        }
    });
    let elapsed = ms(t);
    if std::env::var_os("CINDRAL_ENFORCE_BUDGET").is_some() {
        assert!(
            elapsed < 100.,
            "interactive frame exceeded 100ms: {elapsed:.3}"
        );
    }
    elapsed
}
fn face_position(app: &DrafterApp, nested: bool) -> Result<Pos2, String> {
    let rect = app.replay_rect;
    let aspect = rect.width() / rect.height();
    let view = View {
        matrix: app.camera.view_projection(aspect).as_dmat4(),
        size: glam::DVec2::new(rect.width() as f64, rect.height() as f64),
    };
    // Search setup is outside measured interaction, with exactly the production scene picker.
    for y in 2..38 {
        for x in 2..58 {
            let cursor = glam::DVec2::new(
                rect.width() as f64 * x as f64 / 60.,
                rect.height() as f64 * y as f64 / 40.,
            );
            let (o, d) = app.camera.ray(
                Vec2::new(
                    2. * cursor.x as f32 / rect.width() - 1.,
                    1. - 2. * cursor.y as f32 / rect.height(),
                ),
                aspect,
            );
            if let Some(hit) = app.scene_hit(
                Ray {
                    origin: o.as_dvec3(),
                    direction: d.as_dvec3(),
                },
                view,
                cursor,
            ) {
                if (nested || matches!(hit.hit, cindral_tools::Hit::Face(_)))
                    && (hit.path.len() > app.history.context().len()) == nested
                {
                    return Ok(rect.min + egui::vec2(cursor.x as f32, cursor.y as f32));
                }
            }
        }
    }
    Err(format!(
        "no visible {} face found",
        if nested {
            "component"
        } else {
            "active-context"
        }
    ))
}
fn report(label: &str, samples: &[(f64, cindral_core::profile::Sample)]) {
    let max = samples.iter().map(|s| s.0).fold(0., f64::max);
    let avg = samples.iter().map(|s| s.0).sum::<f64>() / samples.len() as f64;
    println!("{label}: mean={avg:.3} max={max:.3}ms n={}", samples.len());
    if std::env::var_os("CINDRAL_ENFORCE_BUDGET").is_some() {
        let pick_max = samples.iter().map(|s| s.1.phases[0]).fold(0., f64::max);
        assert!(pick_max < 5., "{label}: pick exceeded 5ms: {pick_max:.3}");
        if label.contains("preview") {
            assert!(max < 33., "{label}: preview exceeded 33ms: {max:.3}");
        }
    }
    for (i, name) in cindral_core::profile::NAMES.iter().enumerate() {
        println!(
            "  {name}: mean={:.3} max={:.3}ms",
            samples.iter().map(|s| s.1.phases[i]).sum::<f64>() / samples.len() as f64,
            samples.iter().map(|s| s.1.phases[i]).fold(0., f64::max)
        );
    }
}
/// Returns an error rather than substituting authored geometry when import/replay fails.
pub fn run(path: &Path) -> Result<(), String> {
    if cfg!(debug_assertions) {
        return Err("use --release; all timings must be release".into());
    }
    if std::env::var("CINDRAL_BENCH_GPU").as_deref() == Ok("1") {
        let gpu = super::real_file_gpu::Gpu::new()?;
        GPU.with_borrow_mut(|slot| *slot = Some(gpu));
    }
    let mut app = DrafterApp::default();
    let ctx = egui::Context::default();
    workspace_ui::configure_style(&ctx);
    let t = Instant::now();
    app.load_replay(path).map_err(|e| e.to_string())?;
    println!("import + initial CPU mesh + textures={:.3}ms", ms(t));
    app.choose_modify(ModifyKind::Select);
    println!(
        "first full UI frame incl tessellation={:.3}ms",
        frame(&mut app, &ctx, vec![])
    );
    let rect = app.replay_rect;
    let mut hover = Vec::new();
    for i in 0..200 {
        let p = rect.min
            + egui::vec2(
                rect.width() * ((i * 37) % 199) as f32 / 200.,
                rect.height() * ((i * 61) % 197) as f32 / 200.,
            );
        let dt = frame(&mut app, &ctx, vec![Event::PointerMoved(p)]);
        hover.push((dt, cindral_core::profile::last()));
    }
    report("hover full frame", &hover);
    // Move one actual imported component via press/update/release. No direct kernel commands.
    let start = face_position(&app, true)?;
    app.choose_modify(ModifyKind::Move);
    frame(&mut app, &ctx, vec![Event::PointerMoved(start)]);
    println!(
        "component Move begin={:.3}ms",
        frame(
            &mut app,
            &ctx,
            vec![Event::PointerMoved(start), button(start, true)]
        )
    );
    if app.organization.drag.is_none() {
        return Err(format!("Move did not begin: {:?}", app.error));
    }
    let mut samples = Vec::new();
    for i in 1..=60 {
        let p = start + egui::vec2(i as f32 * 0.8, i as f32 * 0.3);
        let dt = frame(&mut app, &ctx, vec![Event::PointerMoved(p)]);
        if app.error.is_some() {
            return Err(format!("Move preview: {:?}", app.error));
        }
        samples.push((dt, cindral_core::profile::last()));
    }
    report("component Move preview full frame", &samples);
    let end = start + egui::vec2(48., 18.);
    println!(
        "component Move release/commit frame={:.3}ms",
        frame(&mut app, &ctx, vec![button(end, false)])
    );
    if app.organization.drag.is_some() || app.error.is_some() {
        return Err(format!("Move commit failed: {:?}", app.error));
    }
    app.history_step(false);
    app.organization.selected = None;
    // Loose root faces use the same path as a user selecting geometry then Move.
    let start = face_position(&app, false)?;
    app.choose_modify(ModifyKind::Select);
    frame(
        &mut app,
        &ctx,
        vec![Event::PointerMoved(start), button(start, true)],
    );
    frame(&mut app, &ctx, vec![button(start, false)]);
    let rect = app.replay_rect;
    let aspect = rect.width() / rect.height();
    let cursor = glam::DVec2::new(
        (start.x - rect.left()) as f64,
        (start.y - rect.top()) as f64,
    );
    let (o, d) = app.camera.ray(
        Vec2::new(
            2. * cursor.x as f32 / rect.width() - 1.,
            1. - 2. * cursor.y as f32 / rect.height(),
        ),
        aspect,
    );
    let hit = app
        .scene_hit(
            Ray {
                origin: o.as_dvec3(),
                direction: d.as_dvec3(),
            },
            View {
                matrix: app.camera.view_projection(aspect).as_dmat4(),
                size: glam::DVec2::new(rect.width() as f64, rect.height() as f64),
            },
            cursor,
        )
        .ok_or("lost root hit")?;
    app.select_connected(hit.hit);
    app.choose_modify(ModifyKind::Move);
    frame(&mut app, &ctx, vec![Event::PointerMoved(start)]);
    println!(
        "loose Move begin={:.3}ms",
        frame(&mut app, &ctx, vec![button(start, true)])
    );
    if !app.modify.active() {
        return Err(format!("loose Move did not begin: {:?}", app.error));
    }
    let mut samples = Vec::new();
    for i in 1..=60 {
        let p = start + egui::vec2(i as f32 * 0.4, i as f32 * 0.2);
        let dt = frame(&mut app, &ctx, vec![Event::PointerMoved(p)]);
        if app.error.is_some() {
            return Err(format!("loose Move preview: {:?}", app.error));
        }
        samples.push((dt, cindral_core::profile::last()));
    }
    report("loose Move preview full frame", &samples);
    println!(
        "loose Move release/commit frame={:.3}ms",
        frame(
            &mut app,
            &ctx,
            vec![button(start + egui::vec2(24., 12.), false)]
        )
    );
    app.history_step(false);
    let start = face_position(&app, false)?;
    app.choose_modify(ModifyKind::PushPull);
    frame(&mut app, &ctx, vec![Event::PointerMoved(start)]);
    println!(
        "PushPull begin={:.3}ms",
        frame(
            &mut app,
            &ctx,
            vec![Event::PointerMoved(start), button(start, true)]
        )
    );
    if !app.modify.active() {
        return Err(format!("PushPull did not begin: {:?}", app.error));
    }
    let mut samples = Vec::new();
    for i in 1..=60 {
        let p = start + egui::vec2(0., -i as f32 * 0.5);
        let dt = frame(&mut app, &ctx, vec![Event::PointerMoved(p)]);
        if app.error.is_some() {
            return Err(format!("PushPull preview: {:?}", app.error));
        }
        samples.push((dt, cindral_core::profile::last()));
    }
    report("PushPull preview full frame", &samples);
    println!(
        "PushPull release/commit frame={:.3}ms",
        frame(
            &mut app,
            &ctx,
            vec![button(start + egui::vec2(0., -30.), false)]
        )
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn private_file_replay_when_present() {
        let Some(path) = std::env::var_os("CINDRAL_REAL_FILE") else {
            eprintln!("SKIP: set CINDRAL_REAL_FILE and run this regression with --release");
            return;
        };
        let path = std::path::Path::new(&path);
        if !path.is_file() {
            eprintln!("SKIP: private real file is absent");
            return;
        }
        if cfg!(debug_assertions) {
            eprintln!("SKIP: real-file timings require --release");
            return;
        }
        super::run(path).unwrap();
    }
}
