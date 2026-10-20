//! Scene visibility capture preserves geometry and stable organization identities.
use cindral_core::*;
fn camera() -> SceneCamera {
    SceneCamera { target: [0.; 3], distance: 10., yaw: 0.5, pitch: 0.5, orthographic: false }
}
#[test]
fn scene_visibility_restore_is_one_metadata_transaction() {
    let mut doc = DocumentHistory::new();
    doc.execute(Command::AddEdge { start: DVec3::ZERO, end: DVec3::X }).unwrap();
    let vertices = doc.model().vertices().keys().copied().collect();
    doc.organization(OrganizationCommand::Create { kind: DefinitionKind::Group, name: "Part".into(), vertices }).unwrap();
    doc.organization(OrganizationCommand::CreateTag("Layer".into())).unwrap();
    let owner = DefinitionId(0);
    let id = *doc.instances(owner).unwrap().keys().next().unwrap();
    let tag = *doc.tags().keys().next().unwrap();
    let mut scene = doc.capture_scene("Visible".into(), camera(), ViewStyle::default());
    doc.organization(OrganizationCommand::Hide { path: vec![id], hidden: true }).unwrap();
    doc.organization(OrganizationCommand::TagVisibility { id: tag, visible: false }).unwrap();
    let geometry = doc.to_data().definitions;
    let depth = doc.undo_depth();
    // Stale identities remain harmless after objects/tags are deleted between capture and recall.
    scene.tags.insert(TagId(u64::MAX), false);
    scene.objects.insert((owner, InstanceId(u64::MAX)), true);
    doc.organization(OrganizationCommand::RestoreVisibility { tags: scene.tags.clone(), objects: scene.objects.clone() }).unwrap();
    assert_eq!(doc.undo_depth(), depth + 1);
    assert!(!doc.instances(owner).unwrap()[&id].hidden);
    assert!(doc.tags()[&tag].visible);
    assert!(doc.document_changes().geometry.is_empty());
    for (id, before) in geometry {
        assert_eq!(doc.to_data().definitions[&id].geometry, before.geometry);
    }
    doc.undo().unwrap();
    assert!(doc.instances(owner).unwrap()[&id].hidden);
    assert!(!doc.tags()[&tag].visible);
    doc.redo().unwrap();
    assert!(!doc.instances(owner).unwrap()[&id].hidden);
}
#[test]
fn captured_style_is_independent_and_new_tags_retain_visibility() {
    let mut doc = DocumentHistory::new();
    let mut style = ViewStyle::default();
    let scene = doc.capture_scene("First".into(), camera(), style.clone());
    style.mode = StyleMode::XRay;
    style.soft_edges = true;
    style.background = [255, 255, 255];
    assert_eq!(scene.style, ViewStyle::default());
    assert_ne!(scene.style, style);
    doc.organization(OrganizationCommand::CreateTag("New".into())).unwrap();
    doc.organization(OrganizationCommand::RestoreVisibility { tags: scene.tags, objects: scene.objects }).unwrap();
    assert!(doc.tags().values().next().unwrap().visible);
}
