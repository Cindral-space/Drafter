//! Offset/Follow Me audit contracts. No window or GPU; ignored tests preserve known gaps.
use cindral_core::{Command, DVec3 as V, FaceId, History};
use cindral_tools::{Hit, ModifyKind, ModifySession, Selection};

fn fixture(kind: ModifyKind) -> (History, FaceId, ModifySession) {
    let mut h = History::new();
    h.execute(Command::AddPolyline {
        points: vec![V::ZERO, V::X * 10., (V::X + V::Y) * 10., V::Y * 10.],
        closed: true,
    })
    .unwrap();
    let face = *h.model().faces().keys().next().unwrap();
    let mut s = ModifySession::new(kind);
    s.begin(
        h.model(),
        &Selection::default(),
        Some(Hit::Face(face)),
        V::ZERO,
        V::Z,
    )
    .unwrap();
    (h, face, s)
}

fn command(s: &mut ModifySession, value: f64) -> Command {
    if s.kind() == ModifyKind::Offset {
        s.measure(&format!("{value}m")).unwrap().unwrap()
    } else {
        s.update(V::Z * value).unwrap().unwrap()
    }
}

#[test]
fn offset_explicit_units_create_precise_parallel_valid_regions() {
    for (text, distance) in [
        ("2m", 2.),
        ("150cm", 1.5),
        ("250mm", 0.25),
        ("42in", 1.0668),
        ("3.5ft", 1.0668),
    ] {
        let (mut h, _, mut s) = fixture(ModifyKind::Offset);
        h.execute(s.measure(text).unwrap().unwrap()).unwrap();
        h.model().validate().unwrap();
        assert_eq!(h.model().faces().len(), 2);
        let areas: Vec<_> = h
            .model()
            .faces()
            .keys()
            .map(|id| h.model().face_area(*id).unwrap())
            .collect();
        let inner = (10_f64 - 2. * distance).powi(2);
        assert!(
            areas.iter().any(|a| (a - inner).abs() < 1e-7),
            "{text}: {areas:?}"
        );
        assert!((areas.iter().sum::<f64>() - 100.).abs() < 1e-7);
        for point in [
            V::new(distance, distance, 0.),
            V::new(10. - distance, 10. - distance, 0.),
        ] {
            assert!(h
                .model()
                .vertices()
                .values()
                .any(|v| v.position.distance(point) < 1e-7));
        }
        assert_eq!(h.model().bounds().unwrap(), (V::ZERO, V::new(10., 10., 0.)));
    }
}

#[test]
fn follow_me_straight_sweep_has_valid_caps_sides_and_volume() {
    let (mut h, _, mut s) = fixture(ModifyKind::FollowMe);
    h.execute(command(&mut s, 3.)).unwrap();
    h.model().validate().unwrap();
    assert_eq!(h.model().faces().len(), 6);
    assert!((h.model().closed_solid_volume().unwrap().unwrap() - 300.).abs() < 1e-7);
    assert_eq!(h.model().bounds().unwrap(), (V::ZERO, V::new(10., 10., 3.)));
}

#[test]
fn offset_follow_me_cancel_exactly_restores_model_and_preserves_redo() {
    for kind in [ModifyKind::Offset, ModifyKind::FollowMe] {
        let (mut h, _, mut s) = fixture(kind);
        h.execute(Command::AddEdge {
            start: V::new(20., 0., 0.),
            end: V::new(21., 0., 0.),
        })
        .unwrap();
        h.undo().unwrap();
        let before = h.model().clone();
        let depths = (h.undo_depth(), h.redo_depth());
        for distance in [0.5, 1., 1.5] {
            h.preview(command(&mut s, distance)).unwrap();
            h.model().validate().unwrap();
            assert_ne!(h.model(), &before);
            assert_eq!((h.undo_depth(), h.redo_depth()), depths);
        }
        assert!(h.cancel_preview().unwrap());
        s.cancel();
        assert_eq!(h.model(), &before);
        assert_eq!((h.undo_depth(), h.redo_depth()), depths);
        assert!(!s.active());
        assert!(h.redo().unwrap());
    }
}

#[test]
fn offset_follow_me_multiple_previews_commit_one_exact_undo_redo() {
    for kind in [ModifyKind::Offset, ModifyKind::FollowMe] {
        let (mut h, _, mut s) = fixture(kind);
        let before = h.model().clone();
        let depth = h.undo_depth();
        for distance in [0.5, 1., 1.5] {
            h.preview(command(&mut s, distance)).unwrap();
        }
        assert!(h.commit_preview().unwrap());
        s.finish();
        let after = h.model().clone();
        assert_eq!(h.undo_depth(), depth + 1);
        assert!(!s.active());
        h.undo().unwrap();
        assert_eq!(h.model(), &before);
        h.redo().unwrap();
        assert_eq!(h.model(), &after);
        h.model().validate().unwrap();
    }
}

#[test]
fn offset_follow_me_invalid_input_and_geometry_are_atomic() {
    for kind in [ModifyKind::Offset, ModifyKind::FollowMe] {
        let (mut h, face, mut s) = fixture(kind);
        let valid = command(&mut s, 1.);
        let measurement = s.measurement();
        for text in ["garbage", "NaN", "inf", "2m trailing"] {
            assert!(s.measure(text).is_err());
            assert_eq!(s.measurement(), measurement);
        }
        assert_eq!(s.finish(), Some(valid));
        let before = h.model().clone();
        let depth = h.undo_depth();
        let invalid = if kind == ModifyKind::Offset {
            Command::Offset {
                face,
                distance: f64::NAN,
            }
        } else {
            Command::FollowMe {
                face,
                path: vec![V::ZERO, V::X],
            }
        };
        assert!(h.preview(invalid).is_err());
        assert_eq!(h.model(), &before);
        assert_eq!(h.undo_depth(), depth);
        let mut empty = ModifySession::new(kind);
        assert!(empty
            .begin(h.model(), &Selection::default(), None, V::ZERO, V::Z)
            .is_err());
        assert!(!empty.active());
    }
}

#[test]
fn follow_me_preselected_edges_resolve_full_path_independent_of_pointer() {
    let (mut h, face, _) = fixture(ModifyKind::FollowMe);
    h.execute(Command::AddPolyline {
        points: vec![V::ZERO, V::Z * 3., V::new(3., 0., 3.)],
        closed: false,
    })
    .unwrap();
    let selection = Selection {
        edges: h
            .model()
            .edges()
            .iter()
            .filter(|(_, e)| {
                e.vertices
                    .iter()
                    .any(|id| h.model().vertices()[id].position.z > 0.)
            })
            .map(|(id, _)| *id)
            .collect(),
        ..Default::default()
    };
    assert_eq!(selection.edges.len(), 2);
    let mut s = ModifySession::new(ModifyKind::FollowMe);
    s.begin(h.model(), &selection, Some(Hit::Face(face)), V::ZERO, V::Z)
        .unwrap();
    match s.update(V::Z * 0.1).unwrap().unwrap() {
        Command::FollowMe { path, .. } => {
            assert_eq!(path, vec![V::ZERO, V::Z * 3., V::new(3., 0., 3.)])
        }
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
#[ignore = "gap: Offset parser rejects compound feet/inches"]
fn offset_compound_imperial_length() {
    let (_, _, mut s) = fixture(ModifyKind::Offset);
    assert!(
        matches!(s.measure("3'6\"").unwrap(), Some(Command::Offset { distance, .. }) if (distance - 1.0668).abs() < 1e-8)
    );
}

#[test]
#[ignore = "gap: Offset measures Euclidean pointer displacement instead of perpendicular edge distance"]
fn offset_diagonal_pointer_uses_perpendicular_distance() {
    let (h, face, _) = fixture(ModifyKind::Offset);
    let mut s = ModifySession::new(ModifyKind::Offset);
    s.begin(
        h.model(),
        &Selection::default(),
        Some(Hit::Face(face)),
        V::new(0., 2., 0.),
        V::Z,
    )
    .unwrap();
    assert!(
        matches!(s.update(V::new(0.5, 3., 0.)).unwrap(), Some(Command::Offset { distance, .. }) if (distance - 0.5).abs() < 1e-8)
    );
}

#[test]
#[ignore = "gap: Offset finish clears target; completed operation cannot be remeasured"]
fn offset_remeasure_after_completion() {
    let (mut h, _, mut s) = fixture(ModifyKind::Offset);
    h.preview(command(&mut s, 2.)).unwrap();
    h.commit_preview().unwrap();
    s.finish();
    assert!(s.measure("1m").unwrap().is_some());
}

#[test]
#[ignore = "gap: Offset cannot target a selected open coplanar chain without a face"]
fn offset_selected_open_chain() {
    let mut h = History::new();
    h.execute(Command::AddPolyline {
        points: vec![V::ZERO, V::X * 10., V::new(10., 10., 0.)],
        closed: false,
    })
    .unwrap();
    let selection = Selection {
        edges: h.model().edges().keys().copied().collect(),
        ..Default::default()
    };
    let mut s = ModifySession::new(ModifyKind::Offset);
    s.begin(h.model(), &selection, None, V::X * 5., V::Z)
        .unwrap();
    h.execute(s.measure("1m").unwrap().unwrap()).unwrap();
    h.model().validate().unwrap();
}

#[test]
#[ignore = "gap: manual Follow Me replaces its two-point path rather than accumulating traced edges"]
fn follow_me_manual_trace_retains_corner() {
    let (mut h, face, _) = fixture(ModifyKind::FollowMe);
    h.execute(Command::AddPolyline {
        points: vec![V::ZERO, V::Z * 3., V::new(3., 0., 3.)],
        closed: false,
    })
    .unwrap();
    let mut s = ModifySession::new(ModifyKind::FollowMe);
    s.begin(
        h.model(),
        &Selection::default(),
        Some(Hit::Face(face)),
        V::ZERO,
        V::Z,
    )
    .unwrap();
    s.update(V::Z * 3.).unwrap();
    match s.update(V::new(3., 0., 3.)).unwrap().unwrap() {
        Command::FollowMe { path, .. } => {
            assert_eq!(path, vec![V::ZERO, V::Z * 3., V::new(3., 0., 3.)])
        }
        other => panic!("unexpected {other:?}"),
    }
}
