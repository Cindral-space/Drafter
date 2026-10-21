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
