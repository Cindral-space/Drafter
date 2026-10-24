//! Organization, shared-definition history, persistence boundary and transform regressions.
use cindral_core::*;
use glam::DMat4;
fn square(doc: &mut DocumentHistory, size: f64) {
    doc.execute(Command::AddPolyline {
        points: vec![
            DVec3::ZERO,
            DVec3::X * size,
            (DVec3::X + DVec3::Y) * size,
            DVec3::Y * size,
        ],
        closed: true,
    })
    .unwrap();
}
fn container(doc: &mut DocumentHistory, kind: DefinitionKind) -> InstancePath {
    let vertices = doc.model().vertices().keys().copied().collect();
    doc.organization(OrganizationCommand::Create {
        kind,
        name: "Part".into(),
        vertices,
    })
    .unwrap();
    let mut path = doc.context().to_vec();
    let id = *doc
        .instances(doc.active_definition())
        .unwrap()
        .keys()
        .next_back()
        .unwrap();
    path.push(id);
    path
}
#[test]
fn scripted_make_component_three_instances_edit_exit_undo_and_redo() {
    let mut doc = DocumentHistory::new();
    square(&mut doc, 2.);
    let path = container(&mut doc, DefinitionKind::Component);
    let definition = doc.instance(&path).unwrap().definition;
    for x in [5., 10.] {
        doc.organization(OrganizationCommand::AddInstance {
            definition,
            name: "Copy".into(),
            transform: DMat4::from_translation(DVec3::X * x),
        })
        .unwrap();
    }
    assert_eq!(
        doc.occurrences()
            .unwrap()
            .iter()
            .filter(|x| x.definition == definition)
            .count(),
        3
    );
    doc.enter(path.clone()).unwrap();
    let before = doc.model().clone();
    let face = *doc.model().faces().keys().next().unwrap();
    let depth = doc.undo_depth();
    doc.execute(Command::PushPull { face, distance: 3. })
        .unwrap();
    assert_eq!(doc.undo_depth(), depth + 1);
    for item in doc
        .occurrences()
        .unwrap()
        .iter()
        .filter(|x| x.definition == definition)
    {
        assert_eq!(
            doc.definition_model(item.definition).unwrap().faces().len(),
            6
        );
    }
    doc.exit().unwrap();
    doc.undo().unwrap();
    assert_eq!(doc.definition_model(definition), Some(&before));
    doc.redo().unwrap();
    assert_eq!(doc.definition_model(definition).unwrap().faces().len(), 6);
    doc.enter(path).unwrap();
    assert_eq!(doc.model().faces().len(), 6);
}
#[test]
fn make_unique_and_nested_private_groups_break_only_the_intended_link() {
    let mut doc = DocumentHistory::new();
    square(&mut doc, 2.);
    let group = container(&mut doc, DefinitionKind::Group);
    doc.enter(group.clone()).unwrap();
    square(&mut doc, 0.5);
    let child = container(&mut doc, DefinitionKind::Component);
    doc.exit().unwrap();
    doc.organization(OrganizationCommand::Duplicate {
        path: group.clone(),
        transform: DMat4::from_translation(DVec3::X * 5.),
    })
    .unwrap();
    let copied = *doc
        .instances(DefinitionId(0))
        .unwrap()
        .keys()
        .next_back()
        .unwrap();
    let copy = vec![copied];
    assert_ne!(
        doc.instance(&group).unwrap().definition,
        doc.instance(&copy).unwrap().definition
    );
    let mut nested_copy = copy.clone();
    nested_copy.push(
        *doc.instances(doc.instance(&copy).unwrap().definition)
            .unwrap()
            .keys()
            .next()
            .unwrap(),
    );
    assert_eq!(
        doc.instance(&child).unwrap().definition,
        doc.instance(&nested_copy).unwrap().definition
    );
    doc.organization(OrganizationCommand::MakeUnique(nested_copy.clone()))
        .unwrap();
    assert_ne!(
        doc.instance(&child).unwrap().definition,
        doc.instance(&nested_copy).unwrap().definition
    );
    doc.undo().unwrap();
    assert_eq!(
        doc.instance(&child).unwrap().definition,
        doc.instance(&nested_copy).unwrap().definition
    );
}
#[test]
fn navigation_and_tombstone_history_survive_undoing_container_creation() {
    let mut doc = DocumentHistory::new();
    square(&mut doc, 2.);
    let path = container(&mut doc, DefinitionKind::Component);
    let definition = doc.instance(&path).unwrap().definition;
    doc.enter(path).unwrap();
    let face = *doc.model().faces().keys().next().unwrap();
    doc.execute(Command::PushPull { face, distance: 1. })
        .unwrap();
    doc.undo().unwrap();
    doc.undo().unwrap();
    assert!(doc.context().is_empty());
    assert_eq!(doc.model().faces().len(), 1);
    doc.redo().unwrap();
    doc.redo().unwrap();
    assert_eq!(doc.definition_model(definition).unwrap().faces().len(), 6);
}
#[test]
fn reparent_preserves_world_under_mirrored_scaled_parent_and_rejects_cycles() {
    let mut doc = DocumentHistory::new();
    square(&mut doc, 2.);
    let a = container(&mut doc, DefinitionKind::Group);
    square(&mut doc, 1.);
    let b = container(&mut doc, DefinitionKind::Group);
    doc.organization(OrganizationCommand::Transform {
        path: b.clone(),
        transform: DMat4::from_scale_rotation_translation(
            DVec3::new(-2., 3., 1.),
            glam::DQuat::from_rotation_z(0.5),
            DVec3::new(4., 5., 2.),
        ),
    })
    .unwrap();
    let original = doc
        .occurrences()
        .unwrap()
        .into_iter()
        .find(|x| x.path == a)
        .unwrap()
        .world;
    doc.organization(OrganizationCommand::Reparent {
        path: a.clone(),
        parent: b.clone(),
    })
    .unwrap();
    let mut nested = b.clone();
    nested.push(a[0]);
    let world = doc
        .occurrences()
        .unwrap()
        .into_iter()
        .find(|x| x.path == nested)
        .unwrap()
        .world;
    assert!(world.abs_diff_eq(original, 1e-10));
    assert!(doc
        .organization(OrganizationCommand::Reparent {
            path: b.clone(),
            parent: nested
        })
        .is_err());
    doc.undo().unwrap();
    assert!(doc.instance(&a).is_some());
}
#[test]
fn tags_materials_and_versioned_data_are_separate_from_runtime_state() {
    let mut doc = DocumentHistory::new();
    square(&mut doc, 2.);
    let path = container(&mut doc, DefinitionKind::Component);
    doc.organization(OrganizationCommand::CreateTag("Roof".into()))
        .unwrap();
    let tag = *doc.tags().keys().next().unwrap();
    doc.organization(OrganizationCommand::AssignTag {
        path: path.clone(),
        tag: Some(tag),
    })
    .unwrap();
    doc.organization(OrganizationCommand::TagVisibility {
        id: tag,
        visible: false,
    })
    .unwrap();
    assert_eq!(doc.occurrences().unwrap().len(), 1);
    doc.undo().unwrap();
    assert_eq!(doc.occurrences().unwrap().len(), 2);
    doc.organization(OrganizationCommand::CreateMaterial(Material {
        name: "Glass".into(),
        color: [51, 128, 229],
        opacity: 0.4,
        texture: Some("textures/glass.png".into()),
    }))
    .unwrap();
    let material = *doc.materials().keys().next().unwrap();
    doc.organization(OrganizationCommand::PaintInstance {
        path: path.clone(),
        material: Some(material),
    })
    .unwrap();
    doc.enter(path.clone()).unwrap();
    let face = *doc.model().faces().keys().next().unwrap();
    doc.organization(OrganizationCommand::PaintFaces {
        faces: vec![face],
        material: Some(material),
    })
    .unwrap();
    doc.execute(Command::PaintFace {
        faces: vec![face],
        color: [12, 34, 56, 255],
    })
    .unwrap();
    assert_eq!(doc.model().faces()[&face].material, None);
    doc.undo().unwrap();
    assert_eq!(doc.model().faces()[&face].material, Some(material));
    let data = doc.to_data();
    assert_eq!(data.schema_version, SCHEMA_VERSION);
    let mut restored = DocumentHistory::from_data(data.clone()).unwrap();
    assert!(restored.context().is_empty());
    assert_eq!(restored.undo_depth(), 0);
    assert_eq!(restored.to_data(), data);
    restored
        .organization(OrganizationCommand::DeleteMaterial(material))
        .unwrap();
    restored.enter(path).unwrap();
    assert_eq!(restored.model().faces()[&face].material, None);
    restored.undo().unwrap();
    assert_eq!(restored.model().faces()[&face].material, Some(material));
    fn serializable<T: serde::Serialize + serde::de::DeserializeOwned>() {}
    serializable::<DocumentData>();
}
proptest::proptest! {
    #![proptest_config(proptest::test_runner::Config::with_cases(64))]
    #[test]
    fn component_sharing_and_global_undo_invariants(size in 0.2f64..5., distance in 0.1f64..3., scale in 0.2f64..4., mirror in proptest::bool::ANY) {
        let mut doc=DocumentHistory::new();square(&mut doc,size);let path=container(&mut doc,DefinitionKind::Component);let definition=doc.instance(&path).unwrap().definition;
        for x in [10.,20.] {doc.organization(OrganizationCommand::AddInstance{definition,name:"Instance".into(),transform:DMat4::from_scale_rotation_translation(DVec3::new(if mirror{-scale}else{scale},scale,1.),glam::DQuat::IDENTITY,DVec3::X*x)}).unwrap();}
        doc.enter(path).unwrap();let before=doc.model().clone();let face=*doc.model().faces().keys().next().unwrap();doc.execute(Command::PushPull{face,distance}).unwrap();doc.exit().unwrap();
        for o in doc.occurrences().unwrap().iter().filter(|o|o.definition==definition){proptest::prop_assert_eq!(doc.definition_model(o.definition).unwrap().faces().len(),6);}
        doc.undo().unwrap();proptest::prop_assert_eq!(doc.definition_model(definition),Some(&before));
    }
}

#[test]
fn cancelled_duplicate_preserves_redo_and_preview_commit_is_one_step() {
    let mut doc = DocumentHistory::new();
    square(&mut doc, 2.);
    let path = container(&mut doc, DefinitionKind::Component);
    doc.organization(OrganizationCommand::Rename {
        path: path.clone(),
        name: "Renamed".into(),
    })
    .unwrap();
    doc.undo().unwrap();
    let redo = doc.redo_depth();
    let depth = doc.undo_depth();
    let before = doc.to_data();
    for x in [3., 5., 7.] {
        doc.preview_duplicate(&path, DMat4::from_translation(DVec3::X * x))
            .unwrap();
        assert_eq!(doc.undo_depth(), depth);
        assert_eq!(doc.instances(DefinitionId(0)).unwrap().len(), 2);
    }
    doc.cancel_preview().unwrap();
    assert_eq!(
        doc.instances(DefinitionId(0)).unwrap(),
        &before.definitions[&DefinitionId(0)].instances
    );
    assert_eq!(doc.redo_depth(), redo);
    doc.redo().unwrap();
    assert_eq!(doc.instance(&path).unwrap().name, "Renamed");
    let depth = doc.undo_depth();
    doc.preview_duplicate(&path, DMat4::IDENTITY).unwrap();
    doc.commit_preview().unwrap();
    assert_eq!(doc.undo_depth(), depth + 1);
    doc.undo().unwrap();
    assert_eq!(doc.instances(DefinitionId(0)).unwrap().len(), 1);
}

#[test]
fn splitting_and_healing_painted_faces_preserves_material_and_history() {
    let mut doc = DocumentHistory::new();
    square(&mut doc, 2.);
    doc.organization(OrganizationCommand::CreateMaterial(Material {
        name: "Paint".into(),
        color: [51, 102, 153],
        opacity: 0.7,
        texture: None,
    }))
    .unwrap();
    let material = *doc.materials().keys().next().unwrap();
    let face = *doc.model().faces().keys().next().unwrap();
    doc.organization(OrganizationCommand::PaintFaces {
        faces: vec![face],
        material: Some(material),
    })
    .unwrap();
    let painted = doc.model().clone();
    doc.execute(Command::AddEdge {
        start: DVec3::X,
        end: DVec3::new(1., 2., 0.),
    })
    .unwrap();
    assert_eq!(doc.model().faces().len(), 2);
    assert!(doc
        .model()
        .faces()
        .values()
        .all(|f| f.material == Some(material)));
    let seam = *doc
        .model()
        .edges()
        .iter()
        .find(|(_, e)| {
            e.vertices
                .iter()
                .all(|v| (doc.model().vertices()[v].position.x - 1.).abs() < TOLERANCE)
        })
        .unwrap()
        .0;
    doc.execute(Command::EraseEdges(vec![seam])).unwrap();
    assert_eq!(doc.model().faces().len(), 1);
    assert!(doc
        .model()
        .faces()
        .values()
        .all(|f| f.material == Some(material)));
    doc.undo().unwrap();
    doc.undo().unwrap();
    assert_eq!(doc.model(), &painted);
}

#[test]
fn live_material_preview_records_one_undo_entry() {
    let mut doc = DocumentHistory::new();
    square(&mut doc, 2.);
    doc.organization(OrganizationCommand::CreateMaterial(Material {
        name: "Paint".into(),
        color: [255, 0, 0],
        opacity: 1.,
        texture: None,
    }))
    .unwrap();
    let id = *doc.materials().keys().next().unwrap();
    let original = doc.materials()[&id].clone();
    let depth = doc.undo_depth();
    for opacity in [0.9, 0.7, 0.5] {
        doc.material_preview(
            id,
            Material {
                opacity,
                ..original.clone()
            },
        )
        .unwrap();
    }
    assert_eq!(doc.undo_depth(), depth);
    assert_eq!(doc.materials()[&id].opacity, 0.5);
    let edited = doc.materials()[&id].clone();
    doc.finish_material_edit(id, original.clone(), edited.clone())
        .unwrap();
    assert_eq!(doc.undo_depth(), depth + 1);
    assert_eq!(doc.materials()[&id], edited);
    doc.undo().unwrap();
    assert_eq!(doc.materials()[&id], original);
    doc.redo().unwrap();
    assert_eq!(doc.materials()[&id], edited);
}

#[test]
fn replace_material_repaints_faces_and_instances_atomically() {
    let mut doc = DocumentHistory::new();
    square(&mut doc, 2.);
    let mut make = |name: &str| {
        doc.organization(OrganizationCommand::CreateMaterial(Material {
            name: name.into(),
            color: [10, 20, 30],
            opacity: 1.,
            texture: None,
        }))
        .unwrap();
        *doc.materials().keys().next_back().unwrap()
    };
    let red = make("Red");
    let blue = make("Blue");
    doc.organization(OrganizationCommand::Create {
        kind: DefinitionKind::Component,
        name: "Part".into(),
        vertices: doc.model().vertices().keys().copied().collect(),
    })
    .unwrap();
    let instance = *doc
        .instances(DefinitionId(0))
        .unwrap()
        .keys()
        .next()
        .unwrap();
    doc.organization(OrganizationCommand::PaintInstance {
        path: vec![instance],
        material: Some(red),
    })
    .unwrap();
    doc.organization(OrganizationCommand::PaintFaces {
        faces: doc.model().faces().keys().copied().collect(),
        material: Some(red),
    })
    .unwrap();
    let depth = doc.undo_depth();
    doc.organization(OrganizationCommand::ReplaceMaterial {
        from: red,
        to: Some(blue),
    })
    .unwrap();
    assert_eq!(doc.undo_depth(), depth + 1);
    assert!(doc
        .model()
        .faces()
        .values()
        .all(|f| f.material == Some(blue)));
    assert_eq!(
        doc.instances(DefinitionId(0)).unwrap()[&instance].material,
        Some(blue)
    );
    doc.undo().unwrap();
    assert!(doc
        .model()
        .faces()
        .values()
        .all(|f| f.material == Some(red)));
    assert_eq!(
        doc.instances(DefinitionId(0)).unwrap()[&instance].material,
        Some(red)
    );
    doc.redo().unwrap();
    doc.organization(OrganizationCommand::ReplaceMaterial {
        from: blue,
        to: None,
    })
    .unwrap();
    assert!(doc.model().faces().values().all(|f| f.material.is_none()));
    assert_eq!(
        doc.instances(DefinitionId(0)).unwrap()[&instance].material,
        None
    );
    assert!(doc.materials().contains_key(&blue));
}
