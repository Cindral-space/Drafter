//! Push/Pull audit contracts: tool commands plus real transactional geometry, no GPU.
use cindral_core::{Command, DVec3, FaceId, History};
use cindral_tools::{Hit, ModifyKind, ModifySession, Selection};

fn square() -> (History, FaceId, ModifySession) {
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
    let mut session = ModifySession::new(ModifyKind::PushPull);
    // Passed axis deliberately differs from face normal.
    session
        .begin(
            history.model(),
            &Selection::default(),
            Some(Hit::Face(face)),
            DVec3::ZERO,
            DVec3::X,
        )
        .unwrap();
    (history, face, session)
}

#[test]
fn explicit_units_produce_valid_precise_push_pull_geometry() {
    for (input, meters) in [
        ("2m", 2.),
        ("150cm", 1.5),
        ("250mm", 0.25),
        ("42in", 1.0668),
        ("3.5ft", 1.0668),
    ] {
        let (mut history, _, mut session) = square();
        let command = session.measure(input).unwrap().unwrap();
        history.preview(command).unwrap();
        assert!(history.commit_preview().unwrap());
        assert!(
            (history.model().bounds().unwrap().1.z - meters).abs() < 1e-7,
            "{input}"
        );
        assert_eq!(history.model().faces().len(), 6);
        history.model().validate().unwrap();
    }
}

#[test]
fn preview_cancel_restores_exact_model_without_history_entry() {
    let (mut history, _, mut session) = square();
    let before = history.model().clone();
    let depth = history.undo_depth();
    for text in ["1m", "2m", "150cm"] {
        history
            .preview(session.measure(text).unwrap().unwrap())
            .unwrap();
        history.model().validate().unwrap();
        assert_eq!(history.undo_depth(), depth);
    }
    assert!(history.cancel_preview().unwrap());
    session.cancel();
    assert_eq!(history.model(), &before);
    assert_eq!(history.undo_depth(), depth);
    assert!(!session.active());
    history.model().validate().unwrap();
}

#[test]
fn repeated_previews_commit_once_and_exactly_undo_redo() {
    let (mut history, _, mut session) = square();
    let before = history.model().clone();
    let depth = history.undo_depth();
    for i in 1..=10 {
        history
            .preview(
                session
                    .update(DVec3::new(5., 7., i as f64 * 0.1))
                    .unwrap()
                    .unwrap(),
            )
            .unwrap();
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

#[test]
fn invalid_input_preserves_last_valid_measurement_and_command() {
    let (_, _, mut session) = square();
    let valid = session.measure("150cm").unwrap().unwrap();
    for invalid in ["garbage", "NaN", "inf", "2m trailing"] {
        assert!(session.measure(invalid).is_err());
        assert_eq!(session.measurement(), Some(1.5));
    }
    assert_eq!(session.finish(), Some(valid));
}

#[test]
fn normal_constrains_preview_and_empty_click_reports_failure() {
    let (history, face, mut session) = square();
    assert_eq!(
        session.update(DVec3::new(50., 70., 2.)).unwrap(),
        Some(Command::PushPull { face, distance: 2. })
    );
    session.cancel();
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
    history.model().validate().unwrap();
}

#[test]
#[ignore = "Push/Pull parity: compound feet/inches notation is not implemented by parse_length"]
fn compound_imperial_distance() {
    let (mut history, _, mut session) = square();
    history
        .execute(session.measure("3'6\"").unwrap().unwrap())
        .unwrap();
    assert!((history.model().bounds().unwrap().1.z - 1.0668).abs() < 1e-7);
    history.model().validate().unwrap();
}
