//! Import resource collisions and transaction leak checks.
use cindral_core::*;

fn source() -> DocumentData {
    let mut data = DocumentHistory::new().to_data();
    data.tags.insert(
        TagId(10),
        Tag {
            name: "Layer".into(),
            visible: false,
        },
    );
    data.materials.insert(
        MaterialId(11),
        Material {
            name: "Paint".into(),
            color: [255, 0, 0],
            opacity: 1.,
            texture: None,
            texture_mapping: None,
        },
    );
    let mut definition = data.definitions[&DefinitionId(0)].clone();
    definition.name = "Part".into();
    definition.kind = DefinitionKind::Component;
    data.definitions.insert(DefinitionId(12), definition);
    data.next_id = 20;
    data
}

#[test]
fn collisions_merge_without_modifying_existing_and_undo_leaves_no_records() {
    let mut destination = DocumentHistory::from_data(source()).unwrap();
    let before = destination.to_data();
    let mut incoming = source();
    incoming.tags.get_mut(&TagId(10)).unwrap().visible = true;
    assert_eq!(
        destination.append_import_report(incoming).unwrap(),
        [1, 1, 0, 1]
    );
    let after = destination.to_data();
    assert_eq!(after.tags, before.tags);
    assert_eq!(after.materials, before.materials);
    assert_eq!(after.definitions.len(), before.definitions.len() + 1);
    assert!(after.definitions.values().any(|d| d.name == "Part (2)"));
    destination.undo().unwrap();
    let undone = destination.to_data();
    assert_eq!(undone.definitions, before.definitions);
    assert_eq!(undone.tags, before.tags);
    assert_eq!(undone.materials, before.materials);
    destination.redo().unwrap();
    assert_eq!(destination.to_data().definitions, after.definitions);
    let mut different = source();
    different.materials.get_mut(&MaterialId(11)).unwrap().color = [0, 255, 0];
    assert_eq!(
        destination.append_import_report(different).unwrap(),
        [1, 0, 1, 1]
    );
    assert!(destination
        .materials()
        .values()
        .any(|m| m.name == "Paint (2)"));
    assert_eq!(
        destination.materials()[&MaterialId(11)],
        before.materials[&MaterialId(11)]
    );
    destination.undo().unwrap();
    assert_eq!(destination.materials(), &before.materials);
}

#[test]
fn exhausted_import_allocator_returns_error_without_mutation() {
    let mut data = DocumentHistory::new().to_data();
    data.next_id = u64::MAX;
    let mut destination = DocumentHistory::from_data(data).unwrap();
    let before = destination.to_data();
    assert!(matches!(
        destination.append_import(source()),
        Err(KernelError::Capacity)
    ));
    assert_eq!(destination.to_data(), before);
    assert_eq!(destination.undo_depth(), 0);
}
