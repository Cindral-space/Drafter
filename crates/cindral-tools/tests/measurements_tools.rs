//! Scripted shared-parser/retained-operation/history contracts for every adapter.
use cindral_core::{DVec3, DocumentHistory};
use cindral_tools::{ToolKind, ToolSession};
#[test]
fn drawing_repeated_measurement_and_invalid_input() {
    for kind in [
        ToolKind::Line,
        ToolKind::Rectangle,
        ToolKind::Circle,
        ToolKind::Arc,
        ToolKind::Polygon,
    ] {
        let mut h = DocumentHistory::new();
        let mut t = ToolSession::new(kind);
        t.click(DVec3::ZERO).unwrap();
        if kind == ToolKind::Arc {
            t.click(DVec3::X).unwrap();
        }
        t.pointer_move(DVec3::new(2., 3., 0.)).unwrap();
        let entries = if kind == ToolKind::Rectangle {
            ["2m,3m", "4m", "5m,6m"]
        } else if kind == ToolKind::Arc {
            ["90deg", "120deg", "45deg"]
        } else {
            ["2m", "3m", "4m"]
        };
        for (i, text) in entries.into_iter().enumerate() {
            let command = t.measure(text).unwrap().unwrap();
            if i == 0 {
                h.execute(command).unwrap();
            } else {
                h.replace_last(command).unwrap();
            }
            assert_eq!(h.undo_depth(), 1, "{kind:?}");
            let saved = h.model().clone();
            let readout = t.measurement_readout();
            assert!(t.measure("garbage").is_err());
            assert_eq!(h.model(), &saved);
            assert_eq!(t.measurement_readout(), readout);
        }
        let final_model = h.model().clone();
        h.undo().unwrap();
        assert!(h.model().edges().is_empty());
        h.redo().unwrap();
        assert_eq!(h.model(), &final_model);
        t.cancel();
        assert!(!t.editing_completed());
        assert!(t.measure("2m").is_err() || matches!(kind, ToolKind::Circle | ToolKind::Polygon));
    }
}
#[test]
fn coordinates_signed_line_sides_and_arc_radius() {
    let mut t = ToolSession::new(ToolKind::Line);
    t.click(DVec3::X).unwrap();
    let c = t.measure("[2m,3m,4m]").unwrap().unwrap();
    assert!(matches!(c,cindral_core::Command::AddEdge{end,..} if end==DVec3::new(2.,3.,4.)));
    t.cancel();
    t.click(DVec3::ZERO).unwrap();
    t.pointer_move(DVec3::X).unwrap();
    assert!(
        matches!(t.measure("-2m").unwrap().unwrap(),cindral_core::Command::AddEdge{end,..} if end==DVec3::X*-2.)
    );
    for kind in [ToolKind::Circle, ToolKind::Polygon, ToolKind::Arc] {
        t.set_kind(kind);
        assert!(t.measure("12s").unwrap().is_none());
        assert_eq!(t.segments(), 12);
        t.click(DVec3::ZERO).unwrap();
        t.pointer_move(DVec3::X).unwrap();
        if kind == ToolKind::Arc {
            assert!(t.measure("2m").unwrap().is_none());
            t.pointer_move(DVec3::Y).unwrap();
        }
        let c = t
            .measure(if kind == ToolKind::Arc { "90deg" } else { "2m" })
            .unwrap()
            .unwrap();
        let mut h = DocumentHistory::new();
        h.execute(c).unwrap();
        h.replace_last(t.measure("24s").unwrap().unwrap()).unwrap();
        assert_eq!(h.model().edges().len(), 24);
        assert_eq!(h.undo_depth(), 1);
    }
}

use cindral_tools::{Hit, ModifyKind, ModifySession, Selection};
fn modify_fixture(kind: ModifyKind) -> (DocumentHistory, ModifySession) {
    let mut h = DocumentHistory::new();
    h.execute(cindral_core::Command::AddPolyline {
        points: vec![
            DVec3::ZERO,
            DVec3::X * 10.,
            (DVec3::X + DVec3::Y) * 10.,
            DVec3::Y * 10.,
        ],
        closed: true,
    })
    .unwrap();
    let face = *h.model().faces().keys().next().unwrap();
    let mut s = ModifySession::new(kind);
    s.begin(
        h.model(),
        &Selection::default(),
        Some(Hit::Face(face)),
        DVec3::ZERO,
        DVec3::Z,
    )
    .unwrap();
    s.update(DVec3::X).unwrap();
    (h, s)
}
#[test]
fn modify_distances_remeasure_one_entry() {
    for kind in [
        ModifyKind::PushPull,
        ModifyKind::Move,
        ModifyKind::Copy,
        ModifyKind::Offset,
    ] {
        let (mut h, mut s) = modify_fixture(kind);
        let before = h.model().clone();
        let entries = if kind == ModifyKind::Copy {
            ["20m", "30m", "40m"]
        } else {
            ["1m", "2m", "3m"]
        };
        for (i, text) in entries.into_iter().enumerate() {
            let command = s.measure(text).unwrap().unwrap();
            if i == 0 {
                h.preview_atomic(command).unwrap();
                h.commit_preview().unwrap();
            } else {
                h.replace_last(command).unwrap();
            }
            s.finish();
            assert!(s.editing_completed());
            assert_eq!(h.undo_depth(), 2);
            let saved = h.model().clone();
            assert!(s.measure("nope").is_err());
            assert_eq!(h.model(), &saved);
        }
        let after = h.model().clone();
        h.undo().unwrap();
        assert_eq!(h.model(), &before);
        h.redo().unwrap();
        assert_eq!(h.model(), &after);
        s.cancel();
        assert!(s.measure("2m").is_err());
    }
}
#[test]
fn copy_arrays_replace_the_same_operation() {
    let (mut h, mut s) = modify_fixture(ModifyKind::Copy);
    h.execute(s.measure("60m").unwrap().unwrap()).unwrap();
    s.finish();
    for text in ["3x", "*2", "/3"] {
        h.replace_last(s.measure(text).unwrap().unwrap()).unwrap();
        s.finish();
        assert_eq!(h.undo_depth(), 2);
    }
    assert_eq!(h.model().faces().len(), 4);
    h.undo().unwrap();
    assert_eq!(h.model().faces().len(), 1);
    h.redo().unwrap();
    assert_eq!(h.model().faces().len(), 4);
}
#[test]
fn rectangle_extrude_copy_full_undo_redo() {
    let mut h = DocumentHistory::new();
    let mut t = ToolSession::new(ToolKind::Rectangle);
    t.click(DVec3::ZERO).unwrap();
    h.execute(t.measure("2m,3m").unwrap().unwrap()).unwrap();
    let face = *h.model().faces().keys().next().unwrap();
    let mut s = ModifySession::new(ModifyKind::PushPull);
    s.begin(
        h.model(),
        &Selection::default(),
        Some(Hit::Face(face)),
        DVec3::ZERO,
        DVec3::Z,
    )
    .unwrap();
    h.preview_atomic(s.measure("1.5").unwrap().unwrap())
        .unwrap();
    h.commit_preview().unwrap();
    s.finish();
    let selection = Selection {
        vertices: h.model().vertices().keys().copied().collect(),
        ..Default::default()
    };
    s.set_kind(ModifyKind::Copy);
    s.begin(h.model(), &selection, None, DVec3::ZERO, DVec3::Z)
        .unwrap();
    s.update(DVec3::X * 5.).unwrap();
    h.execute(s.measure("5m").unwrap().unwrap()).unwrap();
    s.finish();
    h.replace_last(s.measure("3x").unwrap().unwrap()).unwrap();
    s.finish();
    assert_eq!(h.undo_depth(), 3);
    let after = h.model().clone();
    for _ in 0..3 {
        h.undo().unwrap();
    }
    assert!(h.model().edges().is_empty());
    for _ in 0..3 {
        h.redo().unwrap();
    }
    assert_eq!(h.model(), &after);
}

#[test]
fn rotation_slope_and_scale_length_axes_replacements() {
    for (kind, entries) in [
        (ModifyKind::Rotate, ["8:12", "-90deg", "45"]),
        (ModifyKind::Scale, ["2", "20m", "1,2,3"]),
    ] {
        let (mut h, mut s) = modify_fixture(kind);
        let before = h.model().clone();
        for (i, text) in entries.into_iter().enumerate() {
            let c = s.measure(text).unwrap().unwrap();
            if i == 0 {
                h.execute(c).unwrap();
            } else {
                h.replace_last(c).unwrap();
            }
            s.finish();
            assert_eq!(h.undo_depth(), 2);
        }
        let after = h.model().clone();
        assert!(s.measure("0m,bad").is_err());
        assert_eq!(h.model(), &after);
        h.undo().unwrap();
        assert_eq!(h.model(), &before);
        h.redo().unwrap();
        assert_eq!(h.model(), &after);
    }
}
#[test]
fn tape_proposes_absolute_resize_and_keeps_original_reference() {
    let (mut h, mut s) = modify_fixture(ModifyKind::TapeMeasure);
    s.update(DVec3::X * 5.).unwrap();
    let before = h.model().clone();
    for (i, text) in ["10m", "15m", "20m"].into_iter().enumerate() {
        let c = s.measure(text).unwrap().unwrap();
        if i == 0 {
            h.execute(c).unwrap();
        } else {
            h.replace_last(c).unwrap();
        }
        s.finish();
        assert_eq!(h.undo_depth(), 2);
    }
    assert_eq!(h.model().bounds().unwrap().1, DVec3::new(40., 40., 0.));
    let after = h.model().clone();
    h.undo().unwrap();
    assert_eq!(h.model(), &before);
    h.redo().unwrap();
    assert_eq!(h.model(), &after);
}

#[test]
fn negative_arc_sweep_survives_radius_and_segments() {
    let mut t = ToolSession::new(ToolKind::Arc);
    t.click(DVec3::ZERO).unwrap();
    t.click(DVec3::X).unwrap();
    t.measure("-90").unwrap();
    t.measure("2m").unwrap();
    let c = t.measure("12s").unwrap().unwrap();
    let cindral_core::Command::AddPolyline { points, .. } = c else {
        panic!("arc command")
    };
    assert!(points.last().unwrap().distance(-DVec3::Y * 2.) < 1e-9);
    assert_eq!(points.len(), 13);
}
