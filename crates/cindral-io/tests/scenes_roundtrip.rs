//! Native presentation round trips and migrations from saved pre-scene archives.
use cindral_core::*;
use cindral_io::*;
#[test]
fn scenes_and_every_style_roundtrip_with_visibility_and_cameras() {
    let mut doc = DocumentHistory::new();
    doc.execute(Command::AddEdge {
        start: DVec3::ZERO,
        end: DVec3::X,
    })
    .unwrap();
    doc.organization(OrganizationCommand::CreateTag("Structure".into()))
        .unwrap();
    let mut bundle = DraftBundle {
        document: doc.to_data(),
        ..Default::default()
    };
    for (i, mode) in [
        StyleMode::Shaded,
        StyleMode::Textured,
        StyleMode::Wireframe,
        StyleMode::HiddenLine,
        StyleMode::XRay,
    ]
    .into_iter()
    .enumerate()
    {
        let style = ViewStyle {
            mode,
            profiles: true,
            soft_edges: true,
            smooth_edges: true,
            background: [20, 25, 30],
            sky: Some([90, 120, 190]),
            ground: Some([70, 90, 60]),
        };
        let scene = doc.capture_scene(
            format!("Scene {}", i + 1),
            SceneCamera {
                target: [i as f32, 2., 3.],
                distance: 10.,
                yaw: 0.3,
                pitch: 0.6,
                orthographic: i % 2 == 0,
            },
            style.clone(),
        );
        bundle.presentation.scenes.push(scene);
        bundle.presentation.style = style;
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("scenes.cdraft");
    save_draft(&path, &bundle).unwrap();
    let loaded = load_draft_report(&path).unwrap();
    assert_eq!(loaded.source_version, 4);
    assert_eq!(loaded.bundle, bundle);
    let mut invalid = bundle.clone();
    invalid.presentation.scenes[0].camera.distance = f32::NAN;
    assert!(save_draft(&path, &invalid).is_err());
    assert_eq!(
        load_draft(&path).unwrap(),
        bundle,
        "invalid scene cannot replace a saved file"
    );
    invalid.presentation.scenes[0].camera.distance = 10.;
    invalid.presentation.scenes[0].name = "\n".into();
    assert!(validate_bundle(&invalid).is_err());
}
#[test]
fn actual_saved_v1_and_v2_files_initialize_default_presentation() {
    for (name, version) in [("v1-square.cdraft", 1), ("v2-textured.cdraft", 2)] {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name);
        let report = load_draft_report(&path).unwrap();
        assert_eq!(report.source_version, version);
        assert_eq!(report.bundle.presentation, Presentation::default());
        assert_eq!(report.bundle.document.schema_version, SCHEMA_VERSION);
        validate_bundle(&report.bundle).unwrap();
        assert!(report
            .bundle
            .document
            .definitions
            .values()
            .any(|d| !d.geometry.faces.is_empty()));
    }
}

#[test]
fn saved_v3_scene_preserves_appearance_and_defaults_environment() {
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/v3-scenes.cdraft");
    let report = load_draft_report(&path).unwrap();
    assert_eq!(report.source_version, 3);
    let p = report.bundle.presentation;
    assert_eq!(p.style.mode, StyleMode::HiddenLine);
    assert_eq!(p.sections, SectionState::default());
    assert_eq!(p.shadows, ShadowSettings::default());
    assert_eq!(p.scenes.len(), 1);
    assert_eq!(p.scenes[0].name, "Legacy section-free scene");
    assert_eq!(p.scenes[0].sections, SectionState::default());
    assert_eq!(p.scenes[0].shadows, ShadowSettings::default());
}
#[test]
fn section_shadow_roundtrip_and_invalid_values_preserve_file() {
    let mut b = DraftBundle::default();
    b.presentation.sections = SectionState {
        planes: vec![
            SectionPlane::through("East".into(), DVec3::X, DVec3::X).unwrap(),
            SectionPlane::through("Top".into(), DVec3::Z, DVec3::Z).unwrap(),
        ],
        active: Some(1),
        enabled: true,
    };
    b.presentation.shadows = ShadowSettings {
        enabled: true,
        year: 2024,
        month: 2,
        day: 29,
        minutes: 420.,
        latitude: -33.86,
        longitude: 151.2,
        utc_offset: 11.,
        light: 0.9,
        dark: 0.2,
        on_ground: false,
        quality: ShadowQuality::High,
        ..Default::default()
    };
    let doc = DocumentHistory::new();
    let mut scene = doc.capture_scene(
        "Section & sun".into(),
        SceneCamera {
            target: [0.; 3],
            distance: 10.,
            yaw: 0.,
            pitch: 0.5,
            orthographic: true,
        },
        ViewStyle::default(),
    );
    scene.sections = b.presentation.sections.clone();
    scene.shadows = b.presentation.shadows.clone();
    scene.sections.planes[1].flip();
    scene.shadows.enabled = false;
    b.presentation.scenes.push(scene);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("environment.cdraft");
    save_draft(&path, &b).unwrap();
    let report = load_draft_report(&path).unwrap();
    assert_eq!(report.source_version, 4);
    assert_eq!(report.bundle, b);
    for kind in 0..8 {
        let mut invalid = b.clone();
        match kind {
            0 => invalid.presentation.sections.planes[0].normal = [0.; 3],
            1 => invalid.presentation.sections.active = Some(99),
            2 => invalid.presentation.sections.planes[0].distance = f64::NAN,
            3 => invalid.presentation.shadows.day = 30,
            4 => invalid.presentation.shadows.latitude = 91.,
            5 => invalid.presentation.shadows.minutes = 1440.,
            6 => invalid.presentation.scenes[0].shadows.light = f32::INFINITY,
            _ => invalid.presentation.scenes[0].sections.planes[0].name = "\n".into(),
        }
        assert!(save_draft(&path, &invalid).is_err(), "kind {kind}");
        assert_eq!(load_draft(&path).unwrap(), b);
    }
}
