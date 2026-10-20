//! Native presentation round trips and migrations from saved pre-scene archives.
use cindral_core::*;
use cindral_io::*;
#[test]
fn scenes_and_every_style_roundtrip_with_visibility_and_cameras() {
    let mut doc = DocumentHistory::new();
    doc.execute(Command::AddEdge { start: DVec3::ZERO, end: DVec3::X }).unwrap();
    doc.organization(OrganizationCommand::CreateTag("Structure".into())).unwrap();
    let mut bundle = DraftBundle { document: doc.to_data(), ..Default::default() };
    for (i, mode) in [StyleMode::Shaded, StyleMode::Textured, StyleMode::Wireframe, StyleMode::HiddenLine, StyleMode::XRay].into_iter().enumerate() {
        let style = ViewStyle { mode, profiles: true, soft_edges: true, smooth_edges: true, background: [20, 25, 30], sky: Some([90, 120, 190]), ground: Some([70, 90, 60]) };
        let scene = doc.capture_scene(format!("Scene {}", i + 1), SceneCamera { target: [i as f32, 2., 3.], distance: 10., yaw: 0.3, pitch: 0.6, orthographic: i % 2 == 0 }, style.clone());
        bundle.presentation.scenes.push(scene);
        bundle.presentation.style = style;
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("scenes.cdraft");
    save_draft(&path, &bundle).unwrap();
    let loaded = load_draft_report(&path).unwrap();
    assert_eq!(loaded.source_version, 3);
    assert_eq!(loaded.bundle, bundle);
    let mut invalid = bundle.clone();
    invalid.presentation.scenes[0].camera.distance = f32::NAN;
    assert!(save_draft(&path, &invalid).is_err());
    assert_eq!(load_draft(&path).unwrap(), bundle, "invalid scene cannot replace a saved file");
    invalid.presentation.scenes[0].camera.distance = 10.;
    invalid.presentation.scenes[0].name = "\n".into();
    assert!(validate_bundle(&invalid).is_err());
}
#[test]
fn actual_saved_v1_and_v2_files_initialize_default_presentation() {
    for (name, version) in [("v1-square.cdraft", 1), ("v2-textured.cdraft", 2)] {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name);
        let report = load_draft_report(&path).unwrap();
        assert_eq!(report.source_version, version);
        assert_eq!(report.bundle.presentation, Presentation::default());
        assert_eq!(report.bundle.document.schema_version, 1);
        validate_bundle(&report.bundle).unwrap();
        assert!(report.bundle.document.definitions.values().any(|d| !d.geometry.faces.is_empty()));
    }
}
