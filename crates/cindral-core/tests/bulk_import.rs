//! Bulk construction invariants and import transaction behavior.
use cindral_core::*;
fn square() -> BulkGeometry {
    BulkGeometry {
        vertices: vec![
            DVec3::ZERO,
            DVec3::X,
            DVec3::X + DVec3::Y,
            DVec3::Y,
            DVec3::ZERO + DVec3::splat(TOLERANCE / 4.),
        ],
        faces: vec![
            BulkFace {
                outer: vec![0, 1, 2],
                ..Default::default()
            },
            BulkFace {
                outer: vec![4, 2, 3],
                ..Default::default()
            },
        ],
        ..Default::default()
    }
}
#[test]
fn weld_merge_style_and_validate() {
    let m = square().build(true, true).unwrap();
    assert_eq!(m.vertices().len(), 4);
    assert_eq!(m.faces().len(), 1);
    assert_eq!(m.edges().len(), 4);
    m.validate().unwrap();
    let m = square().build(false, true).unwrap();
    assert_eq!(m.faces().len(), 2);
    assert_eq!(m.edges().values().filter(|e| e.soft && e.smooth).count(), 1);
}
#[test]
fn import_undo_redo_then_local_edit() {
    let mut data = DocumentHistory::new().to_data();
    data.definitions.get_mut(&DefinitionId(0)).unwrap().geometry =
        square().build(true, true).unwrap().to_data();
    let mut h = DocumentHistory::from_import_data(data).unwrap();
    let original = h.to_data();
    assert_eq!(h.undo_depth(), 1);
    h.execute(Command::AddEdge {
        start: DVec3::new(10., 0., 0.),
        end: DVec3::new(11., 0., 0.),
    })
    .unwrap();
    assert_eq!(h.undo_depth(), 2);
    h.undo().unwrap();
    assert_eq!(h.to_data().definitions, original.definitions);
    h.undo().unwrap();
    assert!(h.model().faces().is_empty());
    assert!(!h.changes().faces.is_empty());
    h.redo().unwrap();
    assert_eq!(h.to_data(), original);
    h.redo().unwrap();
    assert_eq!(h.model().edges().len(), 5);
}
#[test]
fn bad_indices_nonfinite_cancel() {
    let mut g = square();
    g.faces[0].outer[1] = usize::MAX;
    assert!(g.build(false, false).is_err());
    let mut g = square();
    g.vertices[0].x = f64::NAN;
    assert!(g.build(false, false).is_err());
    assert!(square().build_cancellable(true, true, || true).is_err());
}
#[test]
fn append_retains_existing_history_and_imports_one_step() {
    let mut h = DocumentHistory::new();
    h.execute(Command::AddEdge {
        start: DVec3::new(10., 0., 0.),
        end: DVec3::new(11., 0., 0.),
    })
    .unwrap();
    let before = h.to_data();
    let mut data = DocumentHistory::new().to_data();
    data.definitions.get_mut(&DefinitionId(0)).unwrap().geometry =
        square().build(true, true).unwrap().to_data();
    h.append_import(data).unwrap();
    assert_eq!(h.undo_depth(), 2);
    assert_eq!(h.model().faces().len(), 1);
    assert_eq!(h.model().edges().len(), 5);
    h.undo().unwrap();
    assert_eq!(h.to_data().definitions, before.definitions);
    h.undo().unwrap();
    assert!(h.model().edges().is_empty());
    h.redo().unwrap();
    h.redo().unwrap();
    assert_eq!(h.model().edges().len(), 5);
}
#[test]
fn holes_and_material_boundaries_survive_cleanup() {
    let mut g = BulkGeometry {
        vertices: vec![
            DVec3::ZERO,
            DVec3::new(4., 0., 0.),
            DVec3::new(4., 4., 0.),
            DVec3::new(0., 4., 0.),
            DVec3::new(1., 1., 0.),
            DVec3::new(3., 1., 0.),
            DVec3::new(3., 3., 0.),
            DVec3::new(1., 3., 0.),
        ],
        faces: vec![BulkFace {
            outer: vec![0, 1, 2, 3],
            holes: vec![vec![4, 5, 6, 7]],
            ..Default::default()
        }],
        ..Default::default()
    };
    let model = g.build(true, true).unwrap();
    model.validate().unwrap();
    assert!(
        (model
            .face_area(*model.faces().keys().next().unwrap())
            .unwrap()
            - 12.)
            .abs()
            < 1e-10
    );
    g = square();
    g.faces[0].material = Some(MaterialId(1));
    g.faces[1].material = Some(MaterialId(2));
    let model = g.build(true, true).unwrap();
    assert_eq!(model.faces().len(), 2);
    assert!(model.edges().values().all(|e| !e.soft));
}
#[test]
fn append_component_sharing_and_tags_restore() {
    let mut source = DocumentHistory::new();
    source
        .execute(Command::AddPolyline {
            points: vec![DVec3::ZERO, DVec3::X, DVec3::X + DVec3::Y, DVec3::Y],
            closed: true,
        })
        .unwrap();
    source
        .organization(OrganizationCommand::Create {
            kind: DefinitionKind::Component,
            name: "Shared".into(),
            vertices: source.model().vertices().keys().copied().collect(),
        })
        .unwrap();
    let definition = source
        .instances(DefinitionId(0))
        .unwrap()
        .values()
        .next()
        .unwrap()
        .definition;
    source
        .organization(OrganizationCommand::AddInstance {
            definition,
            name: "Second".into(),
            transform: glam::DMat4::from_translation(DVec3::new(2., 0., 0.)),
        })
        .unwrap();
    source
        .organization(OrganizationCommand::CreateTag("Imported tag".into()))
        .unwrap();
    let mut destination = DocumentHistory::new();
    destination.append_import(source.to_data()).unwrap();
    let imported = destination.to_data();
    assert_eq!(destination.undo_depth(), 1);
    assert_eq!(destination.instances(DefinitionId(0)).unwrap().len(), 2);
    let defs: std::collections::BTreeSet<_> = destination
        .instances(DefinitionId(0))
        .unwrap()
        .values()
        .map(|i| i.definition)
        .collect();
    assert_eq!(defs.len(), 1);
    assert_eq!(destination.tags().len(), 1);
    destination.undo().unwrap();
    assert!(destination.instances(DefinitionId(0)).unwrap().is_empty());
    assert!(destination.tags().is_empty());
    destination.redo().unwrap();
    assert_eq!(destination.to_data().definitions, imported.definitions);
    assert_eq!(destination.tags().len(), 1);
}
#[test]
fn explicit_mesh_edges_do_not_restore_cleaned_diagonals() {
    let mut g = square();
    g.edges.push(BulkEdge {
        vertices: [0, 2],
        ..Default::default()
    });
    let m = g.clone().build(true, true).unwrap();
    assert_eq!(m.faces().len(), 1);
    assert_eq!(m.edges().len(), 4);
    let m = g.build(false, true).unwrap();
    assert_eq!(m.edges().values().filter(|e| e.soft && e.smooth).count(), 1);
}
