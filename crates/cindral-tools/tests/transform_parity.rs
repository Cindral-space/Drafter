//! GPU-free transform audit contracts. Ignored tests describe confirmed gaps.
use cindral_core::{Command, DVec3, FaceId, History};
use cindral_tools::{Hit, ModifyKind, ModifySession, Selection};

const KINDS: [ModifyKind; 4] = [
    ModifyKind::Move,
    ModifyKind::Copy,
    ModifyKind::Rotate,
    ModifyKind::Scale,
];

fn square() -> (History, FaceId) {
    let mut history = History::new();
    history
        .execute(Command::AddPolyline {
            points: vec![
                DVec3::ZERO,
                DVec3::X * 2.,
                (DVec3::X + DVec3::Y) * 2.,
                DVec3::Y * 2.,
            ],
            closed: true,
        })
        .unwrap();
    let face = *history.model().faces().keys().next().unwrap();
    (history, face)
}

fn prepared(history: &History, face: FaceId, kind: ModifyKind) -> ModifySession {
    let mut session = ModifySession::new(kind);
    session
        .begin(
            history.model(),
            &Selection::default(),
            Some(Hit::Face(face)),
            DVec3::ZERO,
            DVec3::Z,
        )
        .unwrap();
    session.set_reference(DVec3::X).unwrap();
    if kind == ModifyKind::Copy {
        session.update(DVec3::Z).unwrap();
    }
    session
}

fn value(kind: ModifyKind) -> &'static str {
    match kind {
        ModifyKind::Rotate => "90",
        ModifyKind::Scale => "2",
        _ => "3m",
    }
}

#[test]
fn transform_typed_values_produce_precise_valid_geometry() {
    for kind in KINDS {
        let values: &[(&str, f64)] = match kind {
            ModifyKind::Rotate => &[("90", 90.), ("-90deg", -90.)],
            ModifyKind::Scale => &[("2", 2.), ("0.5", 0.5)],
            _ => &[
                ("2m", 2.),
                ("150cm", 1.5),
                ("-250mm", -0.25),
                ("42in", 1.0668),
            ],
        };
        for &(input, number) in values {
            let (mut history, face) = square();
            let before = history.model().clone();
            let mut session = prepared(&history, face, kind);
            session
                .update(if kind == ModifyKind::Copy {
                    DVec3::Z
                } else {
                    DVec3::X
                })
                .unwrap();
            history
                .execute(session.measure(input).unwrap().unwrap())
                .unwrap();
            history.model().validate().unwrap();
            for (id, vertex) in before.vertices() {
                let expected = match kind {
                    ModifyKind::Rotate => glam::DMat4::from_rotation_z(number.to_radians())
                        .transform_point3(vertex.position),
                    ModifyKind::Scale => vertex.position * number,
                    _ => {
                        vertex.position
                            + (if kind == ModifyKind::Copy {
                                DVec3::Z
                            } else {
                                DVec3::X
                            }) * number
                    }
                };
                if kind == ModifyKind::Copy {
                    assert_eq!(history.model().vertices()[id], *vertex);
                    assert!(history
                        .model()
                        .vertices()
                        .values()
                        .any(|v| v.position.distance(expected) < 1e-8));
                } else {
                    assert!(
                        history.model().vertices()[id].position.distance(expected) < 1e-8,
                        "{kind:?} {input}"
                    );
                }
            }
            assert_eq!(
                history.model().faces().len(),
                if kind == ModifyKind::Copy { 2 } else { 1 }
            );
        }
    }
}

#[test]
fn transform_cancel_restores_exact_model_and_history_for_all_kinds() {
    for kind in KINDS {
        let (mut history, face) = square();
        let before = history.model().clone();
        let depth = history.undo_depth();
        let mut session = prepared(&history, face, kind);
        history
            .preview(session.measure(value(kind)).unwrap().unwrap())
            .unwrap();
        assert_ne!(history.model(), &before);
        assert!(history.cancel_preview().unwrap());
        session.cancel();
        assert_eq!(history.model(), &before);
        assert_eq!(history.undo_depth(), depth);
        assert_eq!(history.redo_depth(), 0);
        assert!(!session.active());
        assert!(session.finish().is_none());
        history.model().validate().unwrap();
    }
}

#[test]
fn transform_absolute_previews_commit_one_entry_and_undo_redo_exactly() {
    for kind in KINDS {
        let (mut history, face) = square();
        let before = history.model().clone();
        let depth = history.undo_depth();
        let mut session = prepared(&history, face, kind);
        for input in ["1.1", "1.2", "1.3"] {
            history
                .preview(session.measure(input).unwrap().unwrap())
                .unwrap();
            assert_eq!(history.undo_depth(), depth);
        }
        assert!(history.commit_preview().unwrap());
        session.finish();
        let after = history.model().clone();
        assert_eq!(history.undo_depth(), depth + 1);
        assert!(history.undo().unwrap());
        assert_eq!(history.model(), &before);
        assert!(history.redo().unwrap());
        assert_eq!(history.model(), &after);
        assert!(!session.active());
        history.model().validate().unwrap();
    }
}

#[test]
fn transform_invalid_input_preserves_valid_session_and_empty_hit_reports_error() {
    for kind in KINDS {
        let (history, face) = square();
        let mut session = prepared(&history, face, kind);
        let valid = session.measure(value(kind)).unwrap().unwrap();
        let measurement = session.measurement();
        for text in ["NaN", "inf", "2m trailing", "nonsense"] {
            assert!(session.measure(text).is_err(), "{kind:?} {text}");
            assert_eq!(session.measurement(), measurement);
        }
        assert_eq!(session.finish(), Some(valid));
        assert!(session
            .begin(
                history.model(),
                &Selection::default(),
                None,
                DVec3::ZERO,
                DVec3::Z
            )
            .is_err());
        assert!(!session.active());
    }
}

#[test]
fn transform_rigid_and_positive_scale_preserve_hole_area() {
    for kind in KINDS {
        let (mut history, _) = square();
        history
            .execute(Command::AddPolyline {
                points: vec![
                    DVec3::new(0.5, 0.5, 0.),
                    DVec3::new(1.5, 0.5, 0.),
                    DVec3::new(1.5, 1.5, 0.),
                    DVec3::new(0.5, 1.5, 0.),
                ],
                closed: true,
            })
            .unwrap();
        let face = *history
            .model()
            .faces()
            .iter()
            .find(|(_, f)| !f.holes.is_empty())
            .unwrap()
            .0;
        let mut session = prepared(&history, face, kind);
        history
            .execute(session.measure(value(kind)).unwrap().unwrap())
            .unwrap();
        history.model().validate().unwrap();
        let factor = if kind == ModifyKind::Scale { 4. } else { 1. };
        let holed: Vec<_> = history
            .model()
            .faces()
            .iter()
            .filter(|(_, f)| !f.holes.is_empty())
            .collect();
        assert_eq!(holed.len(), if kind == ModifyKind::Copy { 2 } else { 1 });
        for (id, f) in holed {
            assert_eq!(f.holes.len(), 1);
            assert!((history.model().face_area(*id).unwrap() - 3. * factor).abs() < 1e-8);
        }
    }
}

#[test]
#[ignore = "gap: scalar length parser rejects compound feet/inches"]
fn transform_move_compound_feet_inches() {
    let (mut history, face) = square();
    let mut session = prepared(&history, face, ModifyKind::Move);
    history
        .execute(session.measure("3'6\"").unwrap().unwrap())
        .unwrap();
    assert!((history.model().bounds().unwrap().0.x - 1.0668).abs() < 1e-8);
}

#[test]
#[ignore = "gap: positive-only loose geometry Scale rejects reflection factor"]
fn transform_scale_negative_factor() {
    let (mut history, face) = square();
    let mut session = prepared(&history, face, ModifyKind::Scale);
    history
        .execute(session.measure("-1").unwrap().unwrap())
        .unwrap();
    assert_eq!(
        history.model().bounds().unwrap(),
        (DVec3::new(-2., -2., 0.), DVec3::ZERO)
    );
}

#[test]
#[ignore = "gap: Scale has no directional target-dimension entry or grips"]
fn transform_scale_target_dimension() {
    let (mut history, face) = square();
    let mut session = prepared(&history, face, ModifyKind::Scale);
    history
        .execute(session.measure("4m").unwrap().unwrap())
        .unwrap();
    assert!((history.model().bounds().unwrap().1.x - 4.).abs() < 1e-8);
}

#[test]
#[ignore = "gap: finish clears anchor so last Move cannot be remeasured"]
fn transform_move_remeasure_completed_operation() {
    let (mut history, face) = square();
    let mut session = prepared(&history, face, ModifyKind::Move);
    history
        .preview(session.measure("2m").unwrap().unwrap())
        .unwrap();
    history.commit_preview().unwrap();
    session.finish();
    let command = session.measure("150cm").unwrap().unwrap();
    history.preview(command).unwrap();
    history.commit_preview().unwrap();
    assert!((history.model().bounds().unwrap().0.x - 1.5).abs() < 1e-8);
}

#[test]
#[ignore = "gap: transform target resolution unions unrelated hit with preselection"]
fn transform_preselection_does_not_include_reference_geometry() {
    let (mut history, face) = square();
    history
        .execute(Command::AddPolyline {
            points: vec![
                DVec3::X * 6.,
                DVec3::X * 8.,
                DVec3::new(8., 2., 0.),
                DVec3::new(6., 2., 0.),
            ],
            closed: true,
        })
        .unwrap();
    let other = *history
        .model()
        .faces()
        .keys()
        .find(|&&id| id != face)
        .unwrap();
    let before: Vec<_> = history.model().faces()[&other]
        .outer
        .iter()
        .map(|id| (*id, history.model().vertices()[id].position))
        .collect();
    let selection = Selection {
        faces: [face].into_iter().collect(),
        ..Selection::default()
    };
    let mut session = ModifySession::new(ModifyKind::Move);
    session
        .begin(
            history.model(),
            &selection,
            Some(Hit::Face(other)),
            DVec3::X * 6.,
            DVec3::Z,
        )
        .unwrap();
    history
        .execute(session.measure("2m").unwrap().unwrap())
        .unwrap();
    for (id, position) in before {
        assert_eq!(history.model().vertices()[&id].position, position);
    }
}

#[test]
#[ignore = "gap: overlapping coplanar Copy is rejected with unsplit intersection instead of merging/splitting"]
fn transform_overlapping_copy_splits_intersections() {
    let (mut history, face) = square();
    let mut session = prepared(&history, face, ModifyKind::Copy);
    session.update(DVec3::X).unwrap();
    history
        .execute(session.measure("150cm").unwrap().unwrap())
        .unwrap();
    history.model().validate().unwrap();
}

#[test]
#[ignore = "gap: Move scalar parser does not accept relative coordinate tuples"]
fn transform_move_relative_coordinates() {
    let (mut history, face) = square();
    let mut session = prepared(&history, face, ModifyKind::Move);
    history
        .execute(session.measure("<2m,3m,0m>").unwrap().unwrap())
        .unwrap();
    assert_eq!(history.model().bounds().unwrap().0, DVec3::new(2., 3., 0.));
}
