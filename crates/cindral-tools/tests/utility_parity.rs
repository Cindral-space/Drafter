//! Audit-only CPU contracts; ignored tests retain confirmed parity gaps.
use cindral_core::{Command, DVec3 as V, FaceId, History};
use cindral_tools::{Hit, ModifyKind, ModifySession, Selection};

fn square() -> (History, FaceId) {
    let mut h = History::new();
    h.execute(Command::AddPolyline {
        points: vec![V::ZERO, V::X * 10., (V::X + V::Y) * 10., V::Y * 10.],
        closed: true,
    })
    .unwrap();
    let face = *h.model().faces().keys().next().unwrap();
    (h, face)
}
fn begin(h: &History, kind: ModifyKind, hit: Option<Hit>) -> ModifySession {
    let mut s = ModifySession::new(kind);
    s.begin(h.model(), &Selection::default(), hit, V::ZERO, V::Z)
        .unwrap();
    s
}

#[test]
fn utility_parity_erase_boundary_and_paint_exact_history_and_cancel() {
    for kind in [ModifyKind::Eraser, ModifyKind::PaintBucket] {
        let (mut h, face) = square();
        let edge = *h.model().edges().keys().next().unwrap();
        let before = h.model().clone();
        let depth = h.undo_depth();
        let hit = if kind == ModifyKind::Eraser {
            Hit::Edge(edge)
        } else {
            Hit::Face(face)
        };
        let mut s = begin(&h, kind, Some(hit));
        s.set_color([17, 32, 240, 255]);
        let c = s.update(V::X).unwrap().unwrap();
        // Preparing then cancelling a tool does not execute its command.
        s.cancel();
        assert!(!s.active());
        assert_eq!(s.anchor(), None);
        assert_eq!(h.model(), &before);
        assert_eq!(h.undo_depth(), depth);
        h.execute(c).unwrap();
        h.model().validate().unwrap();
        assert_eq!(h.undo_depth(), depth + 1);
        if kind == ModifyKind::Eraser {
            assert!(!h.model().edges().contains_key(&edge));
            assert!(h.model().faces().is_empty());
        } else {
            assert_eq!(h.model().vertices(), before.vertices());
            assert_eq!(h.model().edges(), before.edges());
            let painted = &h.model().faces()[&face];
            let old = &before.faces()[&face];
            assert_eq!(painted.color, [17, 32, 240, 255]);
            assert_eq!(painted.outer, old.outer);
            assert_eq!(painted.holes, old.holes);
            assert_eq!(painted.plane, old.plane);
        }
        let after = h.model().clone();
        assert!(h.undo().unwrap());
        assert_eq!(h.model(), &before);
        // Cancellation must also preserve the existing redo branch.
        let mut pending = begin(&h, kind, Some(hit));
        pending.update(V::Y).unwrap();
        pending.cancel();
        assert_eq!(h.redo_depth(), 1);
        assert!(h.redo().unwrap());
        assert_eq!(h.model(), &after);
    }
}

#[test]
fn utility_parity_eraser_heals_coplanar_chord() {
    let (mut h, _) = square();
    h.execute(Command::AddEdge {
        start: V::ZERO,
        end: (V::X + V::Y) * 10.,
    })
    .unwrap();
    assert_eq!(h.model().faces().len(), 2);
    let edge = *h
        .model()
        .edges()
        .iter()
        .find(|(_, e)| {
            let a = h.model().vertices()[&e.vertices[0]].position;
            let b = h.model().vertices()[&e.vertices[1]].position;
            (a.distance(b) - 200_f64.sqrt()).abs() < 1e-7
        })
        .unwrap()
        .0;
    let before = h.model().clone();
    let depth = h.undo_depth();
    let mut s = begin(&h, ModifyKind::Eraser, Some(Hit::Edge(edge)));
    s.update(V::X).unwrap();
    h.execute(s.finish().unwrap()).unwrap();
    assert!(!s.active());
    h.model().validate().unwrap();
    assert_eq!(h.model().faces().len(), 1);
    assert_eq!(h.model().edges().len(), 4);
    let face = *h.model().faces().keys().next().unwrap();
    assert!((h.model().face_area(face).unwrap() - 100.).abs() < 1e-7);
    assert_eq!(h.undo_depth(), depth + 1);
    let after = h.model().clone();
    h.undo().unwrap();
    assert_eq!(h.model(), &before);
    h.redo().unwrap();
    assert_eq!(h.model(), &after);
}

#[test]
fn utility_parity_tape_units_distance_cancel_and_invalid_input_are_atomic() {
    for (text, expected) in [
        ("2m", 2.),
        ("150cm", 1.5),
        ("42in", 1.0668),
        ("3.5ft", 1.0668),
    ] {
        let (h, _) = square();
        let before = h.model().clone();
        let mut s = begin(&h, ModifyKind::TapeMeasure, None);
        assert!(s.update(V::new(3., 4., 0.)).unwrap().is_none());
        assert_eq!(s.measurement(), Some(5.));
        assert!(s.measure(text).unwrap().is_some());
        assert!((s.measurement().unwrap() - expected).abs() < 1e-7);
        let measurement = s.measurement();
        assert!(s.measure("invalid").is_err());
        assert!(s.update(V::NAN).is_err());
        assert_eq!(s.measurement(), measurement);
        s.cancel();
        assert!(s.guides().is_empty());
        assert!(!s.active());
        assert_eq!(s.measurement(), None);
        assert_eq!(h.model(), &before);
        assert_eq!(h.undo_depth(), 1);
    }
}

#[test]
fn utility_parity_empty_eraser_and_paint_report_errors_without_changes() {
    for kind in [ModifyKind::Eraser, ModifyKind::PaintBucket] {
        let (h, _) = square();
        let before = h.model().clone();
        let mut s = ModifySession::new(kind);
        assert!(s
            .begin(h.model(), &Selection::default(), None, V::ZERO, V::Z)
            .is_err());
        assert!(!s.active());
        assert_eq!(h.model(), &before);
    }
}

#[test]
#[ignore = "Eraser face hits currently resolve and erase all boundary edges"]
fn utility_parity_eraser_face_interior_does_not_erase() {
    let (mut h, face) = square();
    let before = h.model().clone();
    let mut s = ModifySession::new(ModifyKind::Eraser);
    if s.begin(
        h.model(),
        &Selection::default(),
        Some(Hit::Face(face)),
        V::splat(5.),
        V::Z,
    )
    .is_ok()
    {
        if let Some(c) = s.update(V::splat(5.)).unwrap() {
            h.execute(c).unwrap();
        }
    }
    assert_eq!(h.model(), &before);
}

#[test]
fn utility_parity_tape_compound_imperial() {
    let (h, _) = square();
    let mut s = begin(&h, ModifyKind::TapeMeasure, None);
    s.update(V::X).unwrap();
    s.measure("3'6\"").unwrap();
    assert!((s.measurement().unwrap() - 1.0668).abs() < 1e-7);
}

#[test]
#[ignore = "Tape guide follows anchor-pointer direction instead of offset parallel to source edge"]
fn utility_parity_tape_parallel_offset_guide() {
    let (h, _) = square();
    let edge = *h
        .model()
        .edges()
        .iter()
        .find(|(_, e)| {
            e.vertices
                .iter()
                .all(|v| h.model().vertices()[v].position.y == 0.)
        })
        .unwrap()
        .0;
    let mut s = ModifySession::new(ModifyKind::TapeMeasure);
    s.begin(
        h.model(),
        &Selection::default(),
        Some(Hit::Edge(edge)),
        V::X * 5.,
        V::Z,
    )
    .unwrap();
    s.update(V::new(5., 2., 0.)).unwrap();
    s.measure("2m").unwrap();
    s.finish();
    let guide = s.guides()[0];
    assert!(guide.direction.cross(V::X).length() < 1e-7);
    assert!((guide.origin.y - 2.).abs() < 1e-7);
}
