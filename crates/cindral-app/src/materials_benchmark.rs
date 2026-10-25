//! Release-only Phase 1 materials benchmarks (Ryzen 5 2400G / Vega 11).
//!
//! Run with:
//! ```sh
//! cargo test --release -p cindral-app --features kernel-benchmark --lib materials_benchmark -- --ignored --nocapture
//! ```
//! All workloads are CPU-side editor costs (document, caches, egui panels);
//! the GPU material table upload is one 3-texel-per-material texture write.
#![allow(clippy::print_stdout)]
use crate::DrafterApp;
use cindral_core::{Material, MaterialId, OrganizationCommand as Org};

/// 20,000-face document painted with one material, plus a second material.
fn app_with_painted_20k_faces() -> (DrafterApp, MaterialId, MaterialId) {
    let fixture = cindral_core::benchmark::tiled_squares(100, 100).unwrap();
    let mut app = DrafterApp::default();
    let geometry = fixture.history.model().to_data();
    let painted = MaterialId(geometry.next_id);
    let other = MaterialId(geometry.next_id + 1);
    let mut data = cindral_core::DocumentData {
        schema_version: cindral_core::SCHEMA_VERSION,
        next_id: geometry.next_id + 2,
        root: cindral_core::DefinitionId(0),
        definitions: std::collections::BTreeMap::from([(
            cindral_core::DefinitionId(0),
            cindral_core::DefinitionData {
                name: "Model".into(),
                kind: cindral_core::DefinitionKind::Root,
                geometry,
                instances: std::collections::BTreeMap::new(),
            },
        )]),
        tags: std::collections::BTreeMap::new(),
        materials: std::collections::BTreeMap::new(),
    };
    data.materials.insert(
        painted,
        Material {
            name: "Paint".into(),
            color: [180, 60, 40],
            opacity: 1.,
            texture: None,
            texture_mapping: None,
        },
    );
    data.materials.insert(
        other,
        Material {
            name: "Target".into(),
            color: [40, 90, 200],
            opacity: 1.,
            texture: None,
            texture_mapping: None,
        },
    );
    let definition = data.root;
    let geometry = &mut data.definitions.get_mut(&definition).unwrap().geometry;
    for face in geometry.faces.values_mut() {
        face.material = Some(painted);
    }
    app.set_document_data(data).unwrap();
    // Prime caches so measured iterations see steady state.
    app.refresh_mesh();
    (app, painted, other)
}

fn app_with_5000_materials() -> DrafterApp {
    let mut app = DrafterApp::default();
    let mut data = app.history.to_data();
    for i in 0..5000 {
        data.materials.insert(
            MaterialId(1_000_000 + i),
            Material {
                texture_mapping: None,
                name: format!("Material {i:04}"),
                color: [(i % 255) as u8, 90, 140],
                opacity: 1.,
                texture: None,
            },
        );
    }
    data.next_id = data.next_id.max(1_000_000 + 5000);
    app.set_document_data(data).unwrap();
    app
}

fn worst_and_mean(samples: &[f64]) -> (f64, f64) {
    let worst = samples.iter().copied().fold(0., f64::max);
    let mean = samples.iter().sum::<f64>() / samples.len() as f64;
    (worst, mean)
}

fn egui_context() -> eframe::egui::Context {
    let ctx = eframe::egui::Context::default();
    crate::workspace_ui::configure_style(&ctx);
    ctx
}

/// The pre-table design rewrote every painted face's vertex bytes on each
/// palette edit; this measures that legacy cost for a 20,000-face material.
#[test]
#[ignore = "release-only legacy-path comparison for a 20,000-face material"]
fn legacy_palette_edit_rewrote_20k_face_vertices() {
    let (app, painted, _other) = app_with_painted_20k_faces();
    let model = app.history.model();
    let faces: Vec<_> = model.faces().keys().copied().collect();
    assert_eq!(faces.len(), 20_000);
    let style = cindral_render::FaceStyle {
        color: [0.7, 0.2, 0.1, 1.],
        explicit_material: true,
        material_index: 1,
    };
    let (mut cache, initial) =
        cindral_render::MeshCache::rebuild_styled(model, &|_, _| style).unwrap();
    assert_eq!(initial.face_vertices, 60_000);
    let mut samples = Vec::new();
    for i in 0..30 {
        let tint = 0.2 + (i % 7) as f32 * 0.1;
        let tick_style = cindral_render::FaceStyle {
            color: [tint, 0.3, 0.4, 1.],
            explicit_material: true,
            material_index: 1,
        };
        let start = std::time::Instant::now();
        let update = cache
            .update_styled(model, &faces, &[], &|_, _| tick_style)
            .unwrap();
        samples.push(start.elapsed().as_secs_f64() * 1000.);
        assert!(!update.face_writes.is_empty());
    }
    let (worst, mean) = worst_and_mean(&samples);
    println!("legacy palette edit (20,000 faces restyled): worst {worst:.3} ms, mean {mean:.3} ms");
    let _ = painted;
}

/// One slider tick = material preview + scene sync + material table rebuild.
#[test]
#[ignore = "release-only; 16 ms preferred budget per slider tick"]
fn material_edit_slider_drag_20k_faces() {
    let (mut app, painted, _other) = app_with_painted_20k_faces();
    let mut samples = Vec::new();
    for i in 0..120 {
        let edited = Material {
            color: [40 + (i % 11) * 18, 80, 130],
            opacity: 1.,
            ..app.history.materials()[&painted].clone()
        };
        let start = std::time::Instant::now();
        app.history.material_preview(painted, edited).unwrap();
        app.refresh_mesh();
        samples.push(start.elapsed().as_secs_f64() * 1000.);
    }
    let (worst, mean) = worst_and_mean(&samples);
    println!(
        "slider tick (preview + scene sync, 20,000-face material): worst {worst:.3} ms, mean {mean:.3} ms"
    );
    assert!(worst < 16., "worst tick {worst:.3} ms exceeds 16 ms");
}

/// Painting every face of the model and undoing it stay well under 100 ms.
#[test]
#[ignore = "release-only; 100 ms budget for a 20,000-face paint and its undo"]
fn paint_20k_faces_and_undo() {
    let (mut app, painted, other) = app_with_painted_20k_faces();
    app.organization.material = Some(other);
    let faces: Vec<_> = app.history.model().faces().keys().copied().collect();
    assert_eq!(faces.len(), 20_000);
    let start = std::time::Instant::now();
    app.history
        .organization(Org::PaintFaces {
            faces: faces.clone(),
            material: Some(other),
        })
        .unwrap();
    let kernel_ms = start.elapsed().as_secs_f64() * 1000.;
    println!("kernel PaintFaces: {kernel_ms:.3} ms");
    app.history.undo().unwrap();
    let start = std::time::Instant::now();
    app.org_command(Org::PaintFaces {
        faces: faces.clone(),
        material: Some(other),
    });
    let paint_ms = start.elapsed().as_secs_f64() * 1000.;
    println!(
        "app paint total: {paint_ms:.3} ms; pending geometry updates {} writes {}",
        app.pending
            .as_ref()
            .map(|p| p.geometry.len())
            .unwrap_or_default(),
        app.pending
            .as_ref()
            .and_then(|p| p.geometry.first())
            .map(|g| g.mesh.face_writes.len())
            .unwrap_or_default(),
    );
    let start = std::time::Instant::now();
    app.refresh_mesh();
    let sync_ms = start.elapsed().as_secs_f64() * 1000.;
    assert!(app
        .history
        .model()
        .faces()
        .values()
        .all(|f| f.material == Some(other)));
    let start = std::time::Instant::now();
    app.history.undo().unwrap();
    app.refresh_mesh();
    let undo_ms = start.elapsed().as_secs_f64() * 1000.;
    assert!(app
        .history
        .model()
        .faces()
        .values()
        .all(|f| f.material == Some(painted)));
    println!(
        "paint 20,000 faces: command {paint_ms:.3} ms + mesh sync {sync_ms:.3} ms; undo + sync {undo_ms:.3} ms"
    );
    assert!(paint_ms + sync_ms < 100.);
    assert!(undo_ms < 100.);
}

/// The virtualized Materials panel stays under 16 ms with 5,000 materials.
#[test]
#[ignore = "release-only; 16 ms preferred budget per inspector frame"]
fn materials_panel_5000_materials_frame_times() {
    let mut app = app_with_5000_materials();
    app.inspector_tab = crate::workspace_ui::InspectorTab::Materials;
    let ctx = egui_context();
    let mut samples = Vec::new();
    for _ in 0..100 {
        let start = std::time::Instant::now();
        let input = eframe::egui::RawInput {
            screen_rect: Some(eframe::egui::Rect::from_min_size(
                eframe::egui::Pos2::ZERO,
                eframe::egui::vec2(1200., 800.),
            )),
            ..Default::default()
        };
        let _ = ctx.run(input, |ctx| app.model_inspector(ctx));
        samples.push(start.elapsed().as_secs_f64() * 1000.);
    }
    let (worst, mean) = worst_and_mean(&samples);
    println!(
        "Materials panel frame (5,000 materials): worst {worst:.3} ms, mean {mean:.3} ms, visible rows {}",
        app.materials.rows_built
    );
    assert!(
        app.materials.rows_built > 0,
        "benchmark must render the Materials list"
    );
    assert!(worst < 16., "worst frame {worst:.3} ms exceeds 16 ms");
}
