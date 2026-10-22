//! GPU-free drawing contracts; ignored tests specify desired, currently absent behavior.
use cindral_core::{Command, DVec3, History};
use cindral_tools::{parse_length, ToolKind, ToolSession};

const KINDS: [ToolKind; 5] = [
    ToolKind::Line,
    ToolKind::Rectangle,
    ToolKind::Circle,
    ToolKind::Arc,
    ToolKind::Polygon,
];

fn prepared(kind: ToolKind) -> ToolSession {
    let mut tool = ToolSession::new(kind);
    tool.click(DVec3::ZERO).unwrap();
    if kind == ToolKind::Arc {
        tool.click(DVec3::X).unwrap();
    }
    tool.pointer_move(DVec3::new(2., 3., 0.)).unwrap();
    tool
}

#[test]
fn draw_cancel_preserves_populated_model_and_history_for_all_tools() {
    let mut history = History::new();
    history
        .execute(Command::AddEdge {
            start: DVec3::splat(10.),
            end: DVec3::splat(11.),
        })
        .unwrap();
    let before = history.model().clone();
    let revision = history.revision();
    for kind in KINDS {
        let mut tool = prepared(kind);
        assert!(!tool.preview().is_empty(), "{kind:?}");
        tool.cancel();
        assert!(tool.anchors().is_empty());
        assert!(tool.preview().is_empty());
        assert_eq!(history.model(), &before);
        assert_eq!(history.revision(), revision);
        assert_eq!(history.undo_depth(), 1);
    }
}

#[test]
fn draw_completed_operation_is_valid_and_exactly_reversible_for_all_tools() {
    for kind in KINDS {
        let mut history = History::new();
        let before = history.model().clone();
        let mut tool = prepared(kind);
        let command = tool.click(DVec3::new(2., 3., 0.)).unwrap().unwrap();
        assert!(history.execute(command).unwrap(), "{kind:?}");
        history.model().validate().unwrap();
        assert_eq!(history.undo_depth(), 1);
        let after = history.model().clone();
        let closed = matches!(
            kind,
            ToolKind::Rectangle | ToolKind::Circle | ToolKind::Polygon
        );
        assert_eq!(history.model().faces().len(), usize::from(closed));
        assert!(history.undo().unwrap());
        assert_eq!(history.model(), &before);
        assert!(history.redo().unwrap());
        assert_eq!(history.model(), &after);
        history.model().validate().unwrap();
    }
}

#[test]
fn draw_explicit_metric_values_produce_lengths_and_radii() {
    for kind in [ToolKind::Line, ToolKind::Circle, ToolKind::Polygon] {
        for (text, length) in [("2m", 2.), ("150cm", 1.5)] {
            let mut tool = ToolSession::new(kind);
            tool.click(DVec3::ZERO).unwrap();
            tool.pointer_move(DVec3::X).unwrap();
            let command = tool.measure(text).unwrap().unwrap();
            match command {
                Command::AddEdge { start, end } => {
                    assert!((end.distance(start) - length).abs() < 1e-9)
                }
                Command::AddPolyline {
                    points,
                    closed: true,
                } => {
                    assert!(points.iter().all(|p| (p.length() - length).abs() < 1e-9));
                }
                _ => panic!("unexpected command"),
            }
        }
    }
    let mut rectangle = prepared(ToolKind::Rectangle);
    let mut history = History::new();
    history
        .execute(rectangle.measure("2m,150cm").unwrap().unwrap())
        .unwrap();
    let face = *history.model().faces().keys().next().unwrap();
    assert!((history.model().face_area(face).unwrap() - 3.).abs() < 1e-9);
}

#[test]
fn draw_arc_typed_quarter_sweeps_have_valid_endpoints() {
    for (text, end) in [("90deg", DVec3::Y), ("-90", -DVec3::Y)] {
        let mut tool = prepared(ToolKind::Arc);
        let Command::AddPolyline { points, closed } = tool.measure(text).unwrap().unwrap() else {
            panic!("expected arc")
        };
        assert!(!closed);
        assert!(points.last().unwrap().distance(end) < 1e-9);
        assert!(points.iter().all(|p| (p.length() - 1.).abs() < 1e-9));
    }
}

#[test]
fn draw_invalid_measurements_preserve_state_and_recover() {
    for kind in KINDS {
        let mut tool = prepared(kind);
        let anchors = tool.anchors().to_vec();
        let preview = tool.preview();
        for invalid in ["nonsense", "NaN", "0", "3'bad\""] {
            assert!(tool.measure(invalid).is_err(), "{kind:?}: {invalid}");
            assert_eq!(tool.anchors(), anchors);
            assert_eq!(tool.preview(), preview);
        }
        let valid = match kind {
            ToolKind::Rectangle => "2m,150cm",
            ToolKind::Arc => "90",
            _ => "2m",
        };
        assert!(tool.measure(valid).unwrap().is_some());
    }
}

#[test]
fn draw_compound_imperial_length() {
    assert!((parse_length("3'6\"").unwrap() - 1.0668).abs() < 1e-9);
}

#[test]
fn draw_line_remeasurement_revises_last_segment() {
    let mut history = History::new();
    let mut tool = ToolSession::new(ToolKind::Line);
    tool.click(DVec3::ZERO).unwrap();
    history
        .execute(tool.click(DVec3::X).unwrap().unwrap())
        .unwrap();
    history
        .replace_last(tool.measure("2m").unwrap().unwrap())
        .unwrap();
    assert_eq!(history.undo_depth(), 1);
    let (min, max) = history.model().bounds().unwrap();
    assert_eq!(min, DVec3::ZERO);
    assert_eq!(max, DVec3::X * 2.);
}

#[test]
fn draw_circle_sides_before_center() {
    let mut tool = ToolSession::new(ToolKind::Circle);
    assert!(tool.measure("12").unwrap().is_none());
    assert_eq!(tool.segments(), 12);
    assert!(tool.anchors().is_empty());
}

#[test]
fn draw_rectangle_partial_dimension_preserves_other_dimension() {
    let mut tool = prepared(ToolKind::Rectangle);
    let command = tool.measure("4m,").unwrap().unwrap();
    let mut history = History::new();
    history.execute(command).unwrap();
    let face = *history.model().faces().keys().next().unwrap();
    assert!((history.model().face_area(face).unwrap() - 12.).abs() < 1e-9);
}

#[test]
fn draw_circle_remeasurement_after_completion() {
    let mut tool = prepared(ToolKind::Circle);
    tool.measure("2m").unwrap().unwrap();
    assert!(tool.measure("150cm").unwrap().is_some());
}
