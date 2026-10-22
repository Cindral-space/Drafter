//! Replacing a completed operation is atomic and preserves one history entry.
use cindral_core::{Command, DVec3, DocumentHistory};
#[test]
fn replacement_failure_and_one_entry_undo_redo() {
    let mut h = DocumentHistory::new();
    h.execute(Command::AddEdge {
        start: DVec3::ZERO,
        end: DVec3::X,
    })
    .unwrap();
    let depth = h.undo_depth();
    for n in [2., 3.] {
        assert!(h
            .replace_last(Command::AddEdge {
                start: DVec3::ZERO,
                end: DVec3::X * n
            })
            .unwrap());
        assert_eq!(h.undo_depth(), depth);
    }
    let model = h.model().clone();
    assert!(h
        .replace_last(Command::AddEdge {
            start: DVec3::ZERO,
            end: DVec3::splat(f64::NAN)
        })
        .is_err());
    assert_eq!(h.model(), &model);
    assert_eq!(h.undo_depth(), depth);
    h.undo().unwrap();
    assert!(h.model().edges().is_empty());
    h.redo().unwrap();
    assert_eq!(h.model(), &model);
}

#[test]
fn context_resize_includes_children_replaces_and_restores_exactly() {
    use cindral_core::{DefinitionId, DefinitionKind, OrganizationCommand};
    let mut h = DocumentHistory::new();
    h.execute(Command::AddPolyline {
        points: vec![DVec3::ZERO, DVec3::X, DVec3::X + DVec3::Y, DVec3::Y],
        closed: true,
    })
    .unwrap();
    h.organization(OrganizationCommand::Create {
        kind: DefinitionKind::Group,
        name: "Box".into(),
        vertices: h.model().vertices().keys().copied().collect(),
    })
    .unwrap();
    h.execute(Command::AddEdge {
        start: DVec3::X * 3.,
        end: DVec3::X * 4.,
    })
    .unwrap();
    let id = *h.instances(DefinitionId(0)).unwrap().keys().next().unwrap();
    let before = h.model().clone();
    let instance = h.instance(&[id]).unwrap().clone();
    let depth = h.undo_depth();
    for (i, n) in [2., 3.].into_iter().enumerate() {
        h.resize_active(glam::DMat4::from_scale(DVec3::splat(n)), i > 0)
            .unwrap();
        assert_eq!(h.undo_depth(), depth + 1);
        assert_eq!(h.model().bounds().unwrap().1.x, 4. * n);
    }
    let after = h.model().clone();
    let final_instance = h.instance(&[id]).unwrap().clone();
    assert!(h
        .resize_active(glam::DMat4::from_scale(DVec3::ZERO), true)
        .is_err());
    assert_eq!(h.model(), &after);
    h.undo().unwrap();
    assert_eq!(h.model(), &before);
    assert_eq!(h.instance(&[id]), Some(&instance));
    h.redo().unwrap();
    assert_eq!(h.model(), &after);
    assert_eq!(h.instance(&[id]), Some(&final_instance));
}

#[test]
fn invalid_typed_preview_preserves_the_visible_preview_and_redo() {
    let mut h = DocumentHistory::new();
    h.execute(Command::AddEdge {
        start: DVec3::ZERO,
        end: DVec3::X,
    })
    .unwrap();
    h.undo().unwrap();
    let redo = h.redo_depth();
    h.preview_atomic(Command::AddEdge {
        start: DVec3::ZERO,
        end: DVec3::Y,
    })
    .unwrap();
    let before = h.model().clone();
    assert!(h
        .preview_atomic(Command::AddEdge {
            start: DVec3::ZERO,
            end: DVec3::NAN
        })
        .is_err());
    assert_eq!(h.model(), &before);
    assert_eq!(h.undo_depth(), 0);
    assert_eq!(h.redo_depth(), redo);
    h.cancel_preview().unwrap();
    assert!(h.model().edges().is_empty());
    h.redo().unwrap();
    assert_eq!(h.model().bounds().unwrap().1, DVec3::X);
}
