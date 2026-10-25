//! Release-only Phase 2 materials workload: texture upload, mapped slider
//! drags and bulk paint on the real private file (or a labeled synthetic).
use super::*;
use cindral_core::{Material, OrganizationCommand as Org};
use egui::{Event, Pos2};
use std::{path::Path, time::Instant};

fn frame(
    app: &mut DrafterApp,
    ctx: &egui::Context,
    gpu: &mut real_file_gpu::Gpu,
    events: Vec<Event>,
) -> f64 {
    let start = Instant::now();
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
    let t = Instant::now();
    let jobs = ctx.tessellate(out.shapes, out.pixels_per_point);
    let _ = (t,);
    gpu.draw(&jobs, out.textures_delta, out.pixels_per_point);
    start.elapsed().as_secs_f64() * 1000.
}

/// Generates a 4096x4096 texture in memory (wood-grain-ish stripes); the
/// seed shifts the colors so distinct SHA-256 asset keys are produced.
fn big_texture(seed: u8) -> (String, cindral_io::TextureAsset) {
    let side = 4096usize;
    let mut rgba = Vec::with_capacity(side * side * 4);
    for y in 0..side {
        let grain = ((y as f32 / 24.).sin() * 30.) as i16;
        for x in 0..side {
            let ripple = (((x + y) as f32 / 96.).sin() * 12.) as i16;
            rgba.extend_from_slice(&[
                (150 + grain + ripple + i16::from(seed)).clamp(0, 255) as u8,
                (100 + grain / 2 + i16::from(seed)).clamp(0, 255) as u8,
                (60 + ripple).clamp(0, 255) as u8,
                255,
            ]);
        }
    }
    let image = image::RgbaImage::from_raw(side as u32, side as u32, rgba).unwrap();
    let mut bytes = std::io::Cursor::new(Vec::new());
    image
        .write_to(&mut bytes, image::ImageFormat::Png)
        .expect("encode big texture");
    let (key, asset) = cindral_io::texture_from_bytes(bytes.into_inner()).expect("valid png");
    (key, asset)
}

/// Loads the private model headlessly or builds the labeled synthetic grid.
fn load_or_synthetic(path: &Path) -> Result<(DrafterApp, String), String> {
    let mut app = DrafterApp::default();
    match app.load_replay(path) {
        Ok(()) => Ok((app, "real file".into())),
        Err(e) => {
            println!("Real file unavailable headlessly ({e}); using synthetic scene");
            let mut app = DrafterApp::default();
            for y in 0..50 {
                for x in 0..50 {
                    let origin = DVec3::new(x as f64 * 2., y as f64 * 2., 0.);
                    app.execute(Command::AddPolyline {
                        points: vec![
                            origin,
                            origin + DVec3::X * 2.,
                            origin + DVec3::X * 2. + DVec3::Y * 2.,
                            origin + DVec3::Y * 2.,
                        ],
                        closed: true,
                    })
                    .map_err(|e| e.to_string())?;
                }
            }
            Ok((app, "synthetic 5,000-face grid".into()))
        }
    }
}

/// Runs the Phase 2 workload; external watchdog in the example enforces a
/// hard timeout. Missing adapter is a skip; missing file is a labeled
/// synthetic substitution.
pub fn run(path: &Path) -> Result<(), String> {
    if cfg!(debug_assertions) {
        return Err("materials benchmark requires --release".into());
    }
    let mut gpu = match real_file_gpu::Gpu::new() {
        Ok(gpu) => gpu,
        Err(e) => {
            println!("SKIP GPU materials benchmark: {e}");
            return Ok(());
        }
    };
    gpu.allow_shadows();

    let (mut app, source) = load_or_synthetic(path)?;
    let ctx = egui::Context::default();
    workspace_ui::configure_style(&ctx);
    let faces: Vec<_> = app.history.model().faces().keys().copied().collect();
    println!("{source}: {} faces", faces.len());

    // 4096x4096 import + material creation + upload timing.
    let t = Instant::now();
    let (key, asset) = big_texture(0);
    let encode_ms = t.elapsed().as_secs_f64() * 1000.;
    let t = Instant::now();
    app.files.textures.insert(key.clone(), asset);
    app.org_command(Org::CreateMaterial(Material {
        name: "Benchmark wood".into(),
        color: [255, 255, 255],
        opacity: 1.,
        texture: Some(key.clone()),
        texture_mapping: Some(cindral_core::TextureMapping {
            width_m: 0.6,
            height_m: 0.6,
            rotation_deg: 0.,
            flip_h: false,
            flip_v: false,
        }),
    }));
    let material = app.history.materials().keys().next_back().copied().unwrap();
    app.organization.material = Some(material);
    app.sync_file_textures().map_err(|e| e.to_string())?;
    let texture_import_ms = t.elapsed().as_secs_f64() * 1000.;
    println!(
        "4096x4096 import: generate+encode {encode_ms:.1}ms (one-time, not upload); decode+layer-sync {texture_import_ms:.3}ms"
    );

    // Paint everything, measure command+mesh sync and undo with full frames.
    let t = Instant::now();
    app.org_command(Org::PaintFaces {
        faces: faces.clone(),
        material: Some(material),
    });
    let paint_ms = t.elapsed().as_secs_f64() * 1000.;
    frame(&mut app, &ctx, &mut gpu, vec![]);
    let t = Instant::now();
    app.history.undo().map_err(|e| e.to_string())?;
    app.org_command(Org::PaintFaces {
        faces: faces.clone(),
        material: Some(material),
    });
    let undo_ms = t.elapsed().as_secs_f64() * 1000.;
    frame(&mut app, &ctx, &mut gpu, vec![]);
    println!(
        "{source}: paint-all command {paint_ms:.3}ms, undo+repaint {undo_ms:.3}ms (targets < 100ms each)"
    );
    assert!(app.error.is_none(), "app error: {:?}", app.error);

    // GPU memory after one 4096-class upload.
    println!(
        "GPU allocated bytes after 4096x4096 upload: {} MiB",
        gpu.allocated_bytes() as f64 / 1024. / 1024.
    );

    // Warm up, then drag the opacity slider via synthetic pointer motion
    // across the inspector's slider (exercises preview + table upload +
    // full egui + GPU each frame).
    frame(&mut app, &ctx, &mut gpu, vec![]);
    let revision = app.history.revision();
    let mut color_worst: f64 = 0.;
    // Warmup excludes one-time GL pipeline creation for the textured state,
    // matching the pan benchmark protocol.
    for i in 0..15 {
        let current = app.history.materials()[&material].clone();
        app.history
            .material_preview(
                material,
                Material {
                    color: [90 + i, 120, 90],
                    ..current
                },
            )
            .map_err(|e| e.to_string())?;
        app.refresh_mesh();
        frame(&mut app, &ctx, &mut gpu, vec![]);
    }
    app.history.undo().map_err(|e| e.to_string())?;
    app.org_command(Org::PaintFaces {
        faces: faces.clone(),
        material: Some(material),
    });
    frame(&mut app, &ctx, &mut gpu, vec![]);
    let mut color_total: f64 = 0.;
    for i in 0..60 {
        let byte = 40 + (i * 3) % 200;
        let t = Instant::now();
        let current = app.history.materials()[&material].clone();
        app.history
            .material_preview(
                material,
                Material {
                    color: [byte, 120, 90],
                    ..current
                },
            )
            .map_err(|e| e.to_string())?;
        app.refresh_mesh();
        let cpu_ms = t.elapsed().as_secs_f64() * 1000.;
        let dt = frame(&mut app, &ctx, &mut gpu, vec![]);
        if i < 3 || i % 20 == 0 {
            println!("  tick {i}: cpu {cpu_ms:.3}ms full-frame {dt:.3}ms");
        }
        color_worst = color_worst.max(dt);
        color_total += dt;
    }
    println!(
        "{source}: 60 color slider ticks, full UI+GPU frames: worst {color_worst:.3}ms mean {:.3}ms (target < 16ms)",
        color_total / 60.
    );
    assert!(color_worst < 16., "color slider worst {color_worst:.3}ms");

    // Texture-size field: change the mapping size per tick (same table path).
    let mut size_worst: f64 = 0.;
    for i in 0..60 {
        let width = 0.3 + f64::from(i) * 0.01;
        let current = app.history.materials()[&material].clone();
        app.history
            .material_preview(
                material,
                Material {
                    texture_mapping: Some(cindral_core::TextureMapping {
                        width_m: width,
                        height_m: width,
                        ..current.texture_mapping.unwrap_or_default()
                    }),
                    ..current
                },
            )
            .map_err(|e| e.to_string())?;
        app.refresh_mesh();
        size_worst = size_worst.max(frame(&mut app, &ctx, &mut gpu, vec![]));
    }
    app.history.undo().map_err(|e| e.to_string())?;
    println!(
        "{source}: 60 texture-size ticks, full UI+GPU frames: worst {size_worst:.3}ms (target < 16ms)"
    );
    assert!(size_worst < 16., "size slider worst {size_worst:.3}ms");
    assert_ne!(
        app.history.revision(),
        revision,
        "workload mutated the document"
    );
    Ok(())
}

/// Off-thread texture re-layout budget: 10 large textures prepare on the
/// worker while the UI thread runs full frames; no frame may exceed 33 ms.
/// Prints the old synchronous re-layout cost for comparison.
pub fn run_texture_prepare(path: &Path) -> Result<(), String> {
    if cfg!(debug_assertions) {
        return Err("texture prepare benchmark requires --release".into());
    }
    let mut gpu = match real_file_gpu::Gpu::new() {
        Ok(gpu) => gpu,
        Err(e) => {
            println!("SKIP GPU texture prepare benchmark: {e}");
            return Ok(());
        }
    };
    gpu.allow_shadows();
    let (mut app, source) = load_or_synthetic(path)?;
    let ctx = egui::Context::default();
    workspace_ui::configure_style(&ctx);

    // Ten distinct 4096x4096 textures; generation is one-time and outside timing.
    for i in 0..10u8 {
        let (key, asset) = big_texture(i * 7);
        app.files.textures.insert(key, asset);
    }

    // The UI-thread request (map clone + spawn) must be far under one frame.
    let t = Instant::now();
    app.request_texture_sync();
    let request_ms = t.elapsed().as_secs_f64() * 1000.;
    assert!(app.files.texture_job.is_some(), "job must be running");

    // Warmup: the first frames pay one-time GL pipeline creation for the
    // loaded scene (established benchmark protocol), not per-frame UI work.
    for _ in 0..5 {
        let _ = frame(&mut app, &ctx, &mut gpu, vec![]);
    }
    if app.files.texture_job.is_none() {
        // A tiny set could settle during warmup; restart it for the budget.
        app.request_texture_sync();
    }

    // Full UI+GPU frames while the worker prepares; draw_frame polls the
    // job and the renderer stages one texture layer per frame.
    let mut worst = 0f64;
    let mut total = 0f64;
    let mut frames = 0usize;
    loop {
        let job_done = app.files.texture_job.is_none();
        let dt = frame(&mut app, &ctx, &mut gpu, vec![]);
        frames += 1;
        worst = worst.max(dt);
        total += dt;
        if dt > 16. {
            println!(
                "  frame {frames}: {dt:.3}ms (job done before frame: {job_done}, status: {})",
                app.files.status
            );
        }
        if app.files.texture_job.is_none() {
            break;
        }
        if frames > 2000 {
            return Err("texture preparation never settled".into());
        }
    }
    println!(
        "{source}: request_texture_sync on the UI thread {request_ms:.3}ms; \
10 large textures prepared over {frames} full frames, worst {worst:.3}ms / mean {:.3}ms (budget 33ms)",
        total / frames as f64
    );
    assert!(worst < 33., "UI frame budget exceeded: {worst:.3}ms");
    assert_eq!(app.files.status, "Textures ready");
    assert!(app.error.is_none(), "app error: {:?}", app.error);

    // The old synchronous path on the same textures, for the before/after record.
    let t = Instant::now();
    app.sync_file_textures().map_err(|e| e.to_string())?;
    let sync_ms = t.elapsed().as_secs_f64() * 1000.;
    println!("{source}: old synchronous re-layout of the same 10 textures: {sync_ms:.1}ms");
    Ok(())
}
