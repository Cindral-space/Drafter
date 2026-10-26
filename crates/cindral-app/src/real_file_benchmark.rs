//! Private-file replay through production importer, pointer routing and full egui layout.
use super::*;
use cindral_core::InstanceId;
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

fn face_pick_position(app: &DrafterApp, target: cindral_core::FaceId) -> Result<Pos2, String> {
    let rect = app.replay_rect;
    let aspect = rect.width() / rect.height();
    let view = View {
        matrix: app.camera.view_projection(aspect).as_dmat4() * app.history.world_transform(),
        size: glam::DVec2::new(rect.width() as f64, rect.height() as f64),
    };
    for y in 2..78 {
        for x in 2..118 {
            let cursor = glam::DVec2::new(
                rect.width() as f64 * x as f64 / 120.,
                rect.height() as f64 * y as f64 / 80.,
            );
            let ndc = Vec2::new(
                2. * cursor.x as f32 / rect.width() - 1.,
                1. - 2. * cursor.y as f32 / rect.height(),
            );
            let (origin, direction) = app.camera.ray(ndc, aspect);
            let ray = Ray {
                origin: app
                    .history
                    .world_transform()
                    .inverse()
                    .transform_point3(origin.as_dvec3()),
                direction: app
                    .history
                    .world_transform()
                    .inverse()
                    .transform_vector3(direction.as_dvec3()),
            };
            let hit = cindral_tools::pick_section(
                app.history.model(),
                ray,
                view,
                cursor,
                10.,
                app.local_section().as_ref(),
            );
            if hit == Some(cindral_tools::Hit::Face(target)) {
                return Ok(rect.min + egui::vec2(cursor.x as f32, cursor.y as f32));
            }
        }
    }
    Err(format!(
        "no direct face pick position found for {:?}",
        target
    ))
}
fn report(label: &str, samples: &[(f64, cindral_core::profile::Sample)]) {
    let mut frame_times: Vec<f64> = samples.iter().map(|s| s.0).collect();
    frame_times.sort_by(f64::total_cmp);
    let p50 = frame_times[frame_times.len() / 2];
    let p95 = frame_times[(frame_times.len() as f64 * 0.95) as usize];
    let max = frame_times[frame_times.len() - 1];
    let avg = frame_times.iter().sum::<f64>() / frame_times.len() as f64;
    println!(
        "{label}: mean={avg:.3}ms p50={p50:.3}ms p95={p95:.3}ms max={max:.3}ms n={}",
        samples.len()
    );
    if std::env::var_os("CINDRAL_ENFORCE_BUDGET").is_some() {
        let pick_max = samples.iter().map(|s| s.1.phases[0]).fold(0., f64::max);
        assert!(pick_max < 5., "{label}: pick exceeded 5ms: {pick_max:.3}");
        if label.contains("preview") {
            assert!(max < 33., "{label}: preview exceeded 33ms: {max:.3}");
        }
    }
    for (i, name) in cindral_core::profile::NAMES.iter().enumerate() {
        let mut phase_vals: Vec<f64> = samples.iter().map(|s| s.1.phases[i]).collect();
        phase_vals.sort_by(f64::total_cmp);
        let mean = phase_vals.iter().sum::<f64>() / phase_vals.len() as f64;
        let p50 = phase_vals[phase_vals.len() / 2];
        let p95 = phase_vals[(phase_vals.len() as f64 * 0.95) as usize];
        let max = phase_vals[phase_vals.len() - 1];
        println!("  {name}: mean={mean:.3}ms p50={p50:.3}ms p95={p95:.3}ms max={max:.3}ms");
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
    app.selection.clear();
    app.organization.selected = None;

    // Case (a): planar face with no hole (FaceId(900))
    let start_a = face_pick_position(&app, cindral_core::FaceId::from_raw(900))?;
    run_pushpull_drag(&mut app, &ctx, start_a, "PushPull (a) planar face no hole")?;

    // Case (b): face with an inner ring or cavity (FaceId(905))
    let start_b = face_pick_position(&app, cindral_core::FaceId::from_raw(905))?;
    run_pushpull_drag(
        &mut app,
        &ctx,
        start_b,
        "PushPull (b) face with inner ring/cavity",
    )?;

    // Case (c): face in a large group/component (FaceId(1790) in ThomThom)
    app.enter_context(vec![InstanceId(25)])
        .map_err(|e| e.to_string())?;
    let start_c = face_pick_position(&app, cindral_core::FaceId::from_raw(1790))?;
    run_pushpull_drag(
        &mut app,
        &ctx,
        start_c,
        "PushPull (c) face in large component",
    )?;
    app.exit_context().map_err(|e| e.to_string())?;

    Ok(())
}

fn run_pushpull_drag(
    app: &mut DrafterApp,
    ctx: &egui::Context,
    start: Pos2,
    label: &str,
) -> Result<Vec<(f64, cindral_core::profile::Sample)>, String> {
    app.choose_modify(ModifyKind::PushPull);
    frame(app, ctx, vec![Event::PointerMoved(start)]);
    let begin_dt = frame(
        app,
        ctx,
        vec![Event::PointerMoved(start), button(start, true)],
    );
    println!("{label} begin={begin_dt:.3}ms");
    if !app.modify.active() {
        return Err(format!("{label} did not begin: {:?}", app.error));
    }
    let mut samples = Vec::new();
    for i in 1..=60 {
        let p = start + egui::vec2(0., -i as f32 * 0.5);
        let dt = frame(app, ctx, vec![Event::PointerMoved(p)]);
        if app.error.is_some() {
            return Err(format!("{label} preview: {:?}", app.error));
        }
        samples.push((dt, cindral_core::profile::last()));
    }
    let mut frame_times: Vec<f64> = samples.iter().map(|s| s.0).collect();
    frame_times.sort_by(f64::total_cmp);
    let p50 = frame_times[29];
    let p95 = frame_times[56];
    let max = frame_times[59];
    let avg = frame_times.iter().sum::<f64>() / frame_times.len() as f64;
    println!(
        "{label}: mean={avg:.3}ms p50={p50:.3}ms p95={p95:.3}ms max={max:.3}ms (n={})",
        frame_times.len()
    );
    for (i, name) in cindral_core::profile::NAMES.iter().enumerate() {
        let mut phase_vals: Vec<f64> = samples.iter().map(|s| s.1.phases[i]).collect();
        phase_vals.sort_by(f64::total_cmp);
        let mean = phase_vals.iter().sum::<f64>() / phase_vals.len() as f64;
        println!(
            "  {name}: mean={mean:.3}ms p50={:.3}ms p95={:.3}ms max={:.3}ms",
            phase_vals[29], phase_vals[56], phase_vals[59]
        );
    }
    let commit_dt = frame(app, ctx, vec![button(start + egui::vec2(0., -30.), false)]);
    println!("{label} release/commit={commit_dt:.3}ms");
    app.history_step(false);
    Ok(samples)
}

/// Release Rotate replay on a private imported model, including inference and egui tessellation.
pub fn run_rotate(path: &Path) -> Result<(), String> {
    if cfg!(debug_assertions) {
        return Err("use --release".into());
    }
    if std::env::var("CINDRAL_BENCH_GPU").as_deref() == Ok("1") {
        match super::real_file_gpu::Gpu::new() {
            Ok(gpu) => GPU.with_borrow_mut(|g| *g = Some(gpu)),
            Err(e) => println!("GPU skipped: {e}"),
        }
    }
    let mut app = DrafterApp::default();
    let ctx = egui::Context::default();
    workspace_ui::configure_style(&ctx);
    app.load_replay(path).map_err(|e| e.to_string())?;
    frame(&mut app, &ctx, vec![]);
    let path = app
        .scene
        .occurrences()
        .iter()
        .find(|o| {
            !o.path.is_empty()
                && app
                    .history
                    .definition_model(o.definition)
                    .is_some_and(|m| !m.faces().is_empty())
        })
        .map(|o| o.path[..1].to_vec())
        .ok_or("no visible occurrence")?;
    let (lo, hi) = app
        .selected_world_bounds(&path)
        .ok_or("missing occurrence bounds")?;
    app.organization.selected = Some(path);
    app.selection.clear();
    app.choose_modify(ModifyKind::Rotate);
    app.camera.set_view(StandardView::Top);
    let rect = app.replay_rect;
    app.camera
        .zoom_extents(lo.as_vec3(), hi.as_vec3(), rect.width() / rect.height());
    let view = View {
        matrix: app
            .camera
            .view_projection(rect.width() / rect.height())
            .as_dmat4(),
        size: glam::DVec2::new(rect.width() as f64, rect.height() as f64),
    };
    let p = view
        .project((lo + hi) * 0.5)
        .ok_or("selected object offscreen")?;
    let center = rect.min + egui::vec2(p.x as f32, p.y as f32);
    if !rect.shrink(160.).contains(center) {
        return Err("selected object lacks 160px sweep margin".into());
    }
    let click = |app: &mut DrafterApp, p| {
        frame(app, &ctx, vec![Event::PointerMoved(p)]);
        for down in [true, false] {
            frame(app, &ctx, vec![button(p, down)]);
        }
    };
    click(&mut app, center);
    click(&mut app, center + egui::vec2(80., 0.));
    if !app.modify.active() || !app.rotate.reference_set {
        return Err(format!("Rotate failed to start: {:?}", app.error));
    }
    let depth = app.history.undo_depth();
    let before = app.history.to_data();
    let mut samples = Vec::new();
    for i in 1..=60 {
        let a = i as f32 * 0.02;
        let p = center + egui::vec2(a.cos(), -a.sin()) * 140.;
        let dt = frame(&mut app, &ctx, vec![Event::PointerMoved(p)]);
        samples.push(dt);
        if app.error.is_some() {
            return Err(format!("preview failed: {:?}", app.error));
        }
    }
    assert_eq!(app.history.undo_depth(), depth);
    assert_ne!(
        app.history.to_data(),
        before,
        "sweep must change an actual occurrence placement"
    );
    app.cancel_modify();
    assert_eq!(app.history.to_data(), before);
    samples.sort_by(f64::total_cmp);
    let mean = samples.iter().sum::<f64>() / samples.len() as f64;
    println!("REAL private mybedroom.skp: selected occurrence Rotate, 60 full CPU frames incl inference/mesh/egui/tessellation; mean={mean:.3}ms p95={:.3}ms max={:.3}ms; GPU={}",samples[56],samples[59],GPU.with_borrow(|g|g.is_some()));
    if samples[59] > 33. {
        return Err("Rotate frame exceeds minimum 33ms budget".into());
    }
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
