//! Shared scene conversion and coordinate/organization regressions.
use cindral_core::*;
use cindral_io::*;
use glam::DMat4;
fn scene() -> ImportedScene {
    ImportedScene {
        definitions: vec![ImportedDefinition {
            name: "square".into(),
            geometry: BulkGeometry {
                vertices: vec![DVec3::ZERO, DVec3::X, DVec3::X + DVec3::Y, DVec3::Y],
                faces: vec![
                    BulkFace {
                        outer: vec![0, 1, 2],
                        ..Default::default()
                    },
                    BulkFace {
                        outer: vec![0, 2, 3],
                        ..Default::default()
                    },
                ],
                ..Default::default()
            },
            ..Default::default()
        }],
        roots: vec![
            ImportedInstance {
                name: "first".into(),
                ..Default::default()
            },
            ImportedInstance {
                name: "second".into(),
                transform: DMat4::from_translation(DVec3::new(2., 0., 0.)),
                ..Default::default()
            },
        ],
        ..Default::default()
    }
}
#[test]
fn shared_definition_units_basis_and_loose_geometry() {
    let mut s = scene();
    s.meters_per_unit = 0.01;
    s.up_axis = UpAxis::Y;
    let r = convert_scene(s, &ImportOptions::default(), &ImportControl::default()).unwrap();
    assert_eq!(r.report.faces, 1);
    assert_eq!(r.history.undo_depth(), 1);
    let geometry: Vec<_> = r
        .history
        .occurrences()
        .unwrap()
        .into_iter()
        .filter(|o| {
            !r.history
                .definition_model(o.definition)
                .unwrap()
                .faces()
                .is_empty()
        })
        .collect();
    assert_eq!(geometry.len(), 2);
    assert_eq!(geometry[0].definition, geometry[1].definition);
    let m = r.history.definition_model(geometry[0].definition).unwrap();
    let (min, max) = m.bounds().unwrap();
    assert!(min.distance(DVec3::ZERO) < 1e-12);
    assert!(max.distance(DVec3::new(0.01, 0., 0.01)) < 1e-12);
    let r = convert_scene(
        scene(),
        &ImportOptions {
            as_component: false,
            ..Default::default()
        },
        &ImportControl::default(),
    )
    .unwrap();
    assert_eq!(r.history.model().faces().len(), 2);
    assert!(r.history.instances(DefinitionId(0)).unwrap().is_empty());
}
#[test]
fn cancellation_and_recursive_graph_are_atomic() {
    let c = ImportControl::default();
    c.cancel();
    assert!(convert_scene(scene(), &ImportOptions::default(), &c).is_err());
    let mut s = scene();
    s.definitions[0].instances.push(ImportedInstance::default());
    assert!(convert_scene(s, &ImportOptions::default(), &ImportControl::default()).is_err());
    let mut s = scene();
    s.meters_per_unit = f64::NAN;
    assert!(convert_scene(s, &ImportOptions::default(), &ImportControl::default()).is_err());
}
#[test]
fn obj_pipeline_uses_cleanup_without_restoring_diagonal_edges() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mesh.obj");
    std::fs::write(
        &path,
        "v 0 0 0\nv 1 0 0\nv 1 1 0\nv 0 1 0\nf 1 2 3\nf 1 3 4\n",
    )
    .unwrap();
    let imported =
        import_file(&path, &ImportOptions::default(), &ImportControl::default()).unwrap();
    assert_eq!(imported.report.faces, 1);
    assert_eq!(imported.report.edges, 4);
}

#[test]
fn file_dispatch_native_and_interchange_and_source_basis_override() {
    let directory = tempfile::tempdir().unwrap();
    let native = directory.path().join("source.cdraft");
    let converted = convert_scene(
        scene(),
        &ImportOptions::default(),
        &ImportControl::default(),
    )
    .unwrap();
    save_draft(
        &native,
        &DraftBundle {
            document: converted.history.to_data(),
            textures: converted.textures,
            ..Default::default()
        },
    )
    .unwrap();
    let original = std::fs::read(&native).unwrap();
    let imported = import_file(
        &native,
        &ImportOptions::default(),
        &ImportControl::default(),
    )
    .unwrap();
    assert!(imported.report.faces > 0);
    assert_eq!(std::fs::read(&native).unwrap(), original);
    for name in ["i1-seed.gltf", "i1-seed.glb", "i1-sample.dae"] {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name);
        assert!(
            import_file(&path, &ImportOptions::default(), &ImportControl::default())
                .unwrap()
                .report
                .faces
                > 0
        );
    }
    let obj = directory.path().join("source.obj");
    std::fs::write(&obj, "v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n").unwrap();
    let result = import_file_with_basis(
        &obj,
        &ImportOptions::default(),
        &ImportControl::default(),
        Some(0.01),
        Some(UpAxis::Z),
    )
    .unwrap();
    let bounds = result
        .history
        .occurrences()
        .unwrap()
        .iter()
        .filter_map(|o| result.history.definition_model(o.definition)?.bounds())
        .find(|(_, max)| max.x > 0.)
        .unwrap();
    assert!((bounds.1.x - 0.01).abs() < 1e-9);
}
