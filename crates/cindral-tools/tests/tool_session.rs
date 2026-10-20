//! Integration and CPU tests for Cindral Drafter ToolSession.

use cindral_core::{Command, DVec3, History, TOLERANCE};
use cindral_inference::DrawingPlane;
use cindral_tools::{parse_length, ToolError, ToolKind, ToolSession};

#[test]
fn test_closed_face_creation() {
    let mut history = History::new();

    // 1. Line chaining forming a closed square
    let mut session = ToolSession::new(ToolKind::Line);
    let p0 = DVec3::new(0.0, 0.0, 0.0);
    let p1 = DVec3::new(2.0, 0.0, 0.0);
    let p2 = DVec3::new(2.0, 2.0, 0.0);
    let p3 = DVec3::new(0.0, 2.0, 0.0);

    assert_eq!(session.click(p0).unwrap(), None);
    let cmd1 = session.click(p1).unwrap().expect("cmd1");
    let cmd2 = session.click(p2).unwrap().expect("cmd2");
    let cmd3 = session.click(p3).unwrap().expect("cmd3");
    let cmd4 = session.click(p0).unwrap().expect("cmd4");

    history.execute(cmd1).unwrap();
    assert_eq!(history.model().faces().len(), 0);
    history.execute(cmd2).unwrap();
    assert_eq!(history.model().faces().len(), 0);
    history.execute(cmd3).unwrap();
    assert_eq!(history.model().faces().len(), 0);
    history.execute(cmd4).unwrap();
    assert_eq!(history.model().faces().len(), 1);

    // Verify undo and redo
    history.undo().unwrap();
    assert_eq!(history.model().faces().len(), 0);
    history.redo().unwrap();
    assert_eq!(history.model().faces().len(), 1);

    // 2. Rectangle tool creating a closed face
    let mut rect_history = History::new();
    let mut rect_session = ToolSession::new(ToolKind::Rectangle);
    assert_eq!(rect_session.click(DVec3::ZERO).unwrap(), None);
    let rect_cmd = rect_session
        .click(DVec3::new(4.0, 3.0, 0.0))
        .unwrap()
        .expect("rectangle command");
    rect_history.execute(rect_cmd).unwrap();
    assert_eq!(rect_history.model().faces().len(), 1);

    // 3. Circle tool creating a closed face
    let mut circle_history = History::new();
    let mut circle_session = ToolSession::new(ToolKind::Circle);
    assert_eq!(circle_session.click(DVec3::ZERO).unwrap(), None);
    let circle_cmd = circle_session
        .click(DVec3::new(2.0, 0.0, 0.0))
        .unwrap()
        .expect("circle command");
    circle_history.execute(circle_cmd).unwrap();
    assert_eq!(circle_history.model().faces().len(), 1);
    assert_eq!(circle_history.model().vertices().len(), 48);

    // 4. Polygon tool creating a closed face
    let mut poly_history = History::new();
    let mut poly_session = ToolSession::new(ToolKind::Polygon);
    assert_eq!(poly_session.click(DVec3::ZERO).unwrap(), None);
    let poly_cmd = poly_session
        .click(DVec3::new(3.0, 0.0, 0.0))
        .unwrap()
        .expect("polygon command");
    poly_history.execute(poly_cmd).unwrap();
    assert_eq!(poly_history.model().faces().len(), 1);
    assert_eq!(poly_history.model().vertices().len(), 6);

    // 5. Arc + chord forming a closed face
    let mut arc_history = History::new();
    let mut arc_session = ToolSession::new(ToolKind::Arc);
    let center = DVec3::ZERO;
    let start = DVec3::new(2.0, 0.0, 0.0);
    let end_dir = DVec3::new(0.0, 2.0, 0.0);

    assert_eq!(arc_session.click(center).unwrap(), None);
    assert_eq!(arc_session.click(start).unwrap(), None);
    let arc_cmd = arc_session.click(end_dir).unwrap().expect("arc command");
    arc_history.execute(arc_cmd).unwrap();
    assert_eq!(arc_history.model().faces().len(), 0);

    // Close arc with straight line from end (0,2,0) to start (2,0,0)
    let chord_cmd = Command::AddEdge {
        start: DVec3::new(0.0, 2.0, 0.0),
        end: start,
    };
    arc_history.execute(chord_cmd).unwrap();
    assert_eq!(arc_history.model().faces().len(), 1);
}

#[test]
fn test_each_tool_click_order() {
    // Line: click 1 sets anchor, click 2 commits and chains
    let mut session = ToolSession::new(ToolKind::Line);
    assert_eq!(session.anchor(), None);
    assert_eq!(session.anchors(), &[]);

    let p0 = DVec3::new(1.0, 2.0, 3.0);
    let res1 = session.click(p0).unwrap();
    assert_eq!(res1, None);
    assert_eq!(session.anchor(), Some(p0));
    assert_eq!(session.anchors(), &[p0]);

    let p1 = DVec3::new(5.0, 2.0, 3.0);
    let res2 = session.click(p1).unwrap();
    assert_eq!(res2, Some(Command::AddEdge { start: p0, end: p1 }));
    assert_eq!(session.anchor(), Some(p1));
    assert_eq!(session.anchors(), &[p1]);
    assert_eq!(session.previous_direction(), Some(DVec3::X));

    // Rectangle: click 1 sets anchor, click 2 commits and resets
    session.set_kind(ToolKind::Rectangle);
    assert_eq!(session.anchor(), None);
    let r0 = DVec3::new(1.0, 1.0, 0.0);
    assert_eq!(session.click(r0).unwrap(), None);
    assert_eq!(session.anchor(), Some(r0));
    assert_eq!(session.plane().origin, r0);

    let r1 = DVec3::new(4.0, 5.0, 0.0);
    let res_rect = session.click(r1).unwrap().expect("rect command");
    assert!(matches!(
        res_rect,
        Command::AddPolyline { closed: true, .. }
    ));
    assert_eq!(session.anchor(), None);
    assert_eq!(session.anchors(), &[]);

    // Circle: click 1 center, click 2 radius point, then resets
    session.set_kind(ToolKind::Circle);
    let c0 = DVec3::new(2.0, 2.0, 0.0);
    assert_eq!(session.click(c0).unwrap(), None);
    assert_eq!(session.anchor(), Some(c0));
    assert_eq!(session.plane().origin, c0);

    let c1 = DVec3::new(5.0, 2.0, 0.0);
    let res_circle = session.click(c1).unwrap().expect("circle command");
    assert!(matches!(
        res_circle,
        Command::AddPolyline { closed: true, .. }
    ));
    assert_eq!(session.anchor(), None);

    // Polygon: click 1 center, click 2 first vertex, then resets
    session.set_kind(ToolKind::Polygon);
    let poly_c = DVec3::new(0.0, 0.0, 0.0);
    assert_eq!(session.click(poly_c).unwrap(), None);
    assert_eq!(session.anchor(), Some(poly_c));

    let poly_v = DVec3::new(0.0, 3.0, 0.0);
    let res_poly = session.click(poly_v).unwrap().expect("poly command");
    assert!(matches!(
        res_poly,
        Command::AddPolyline { closed: true, .. }
    ));
    assert_eq!(session.anchor(), None);

    // Arc: click 1 center, click 2 start, click 3 end direction
    session.set_kind(ToolKind::Arc);
    assert!(session.tangent_hint().is_none());

    let arc_c = DVec3::new(0.0, 0.0, 0.0);
    assert_eq!(session.click(arc_c).unwrap(), None);
    assert_eq!(session.anchor(), Some(arc_c));
    assert_eq!(session.anchors(), &[arc_c]);
    assert!(session.tangent_hint().is_none());

    let arc_s = DVec3::new(3.0, 0.0, 0.0);
    assert_eq!(session.click(arc_s).unwrap(), None);
    assert_eq!(session.anchor(), Some(arc_s));
    assert_eq!(session.anchors(), &[arc_c, arc_s]);
    let hint = session.tangent_hint().expect("tangent hint active");
    assert_eq!(hint.center, arc_c);
    assert_eq!(hint.reference, arc_s);

    let arc_e = DVec3::new(0.0, 3.0, 0.0);
    let res_arc = session.click(arc_e).unwrap().expect("arc command");
    assert!(matches!(
        res_arc,
        Command::AddPolyline { closed: false, .. }
    ));
    assert_eq!(session.anchor(), None);
    assert_eq!(session.anchors(), &[]);
    assert!(session.tangent_hint().is_none());
}

#[test]
fn test_arbitrary_tilted_planes() {
    let mut session = ToolSession::new(ToolKind::Rectangle);
    let tilted_plane = DrawingPlane {
        origin: DVec3::new(10.0, 20.0, 30.0),
        normal: DVec3::new(1.0, 2.0, 2.0), // length = 3
    };
    session.set_plane(tilted_plane).unwrap();
    assert_eq!(session.plane().origin, tilted_plane.origin);
    assert_eq!(session.plane().normal, tilted_plane.normal);

    let unit_n = tilted_plane.normal.normalize();

    // 1. Draw rectangle on tilted plane
    let p0 = DVec3::new(10.0, 20.0, 30.0);
    assert_eq!(session.click(p0).unwrap(), None);
    session.pointer_move(DVec3::new(12.0, 25.0, 31.0)).unwrap();
    let rect_cmd = session
        .click(DVec3::new(12.0, 25.0, 31.0))
        .unwrap()
        .expect("rectangle on tilted plane");

    if let Command::AddPolyline { points, closed } = &rect_cmd {
        assert!(closed);
        assert_eq!(points.len(), 4);
        for pt in points {
            let dist = (pt - session.plane().origin).dot(unit_n).abs();
            assert!(
                dist <= TOLERANCE * 10.0,
                "Point {:?} not on tilted plane",
                pt
            );
        }
    } else {
        panic!("Expected AddPolyline");
    }

    let mut history = History::new();
    history.execute(rect_cmd).unwrap();
    assert_eq!(history.model().faces().len(), 1);

    // 2. Draw circle on tilted plane
    session.set_kind(ToolKind::Circle);
    session.set_plane(tilted_plane).unwrap();
    assert_eq!(session.click(p0).unwrap(), None);
    let circle_cmd = session
        .click(DVec3::new(15.0, 20.0, 30.0))
        .unwrap()
        .expect("circle on tilted plane");
    if let Command::AddPolyline { points, closed } = &circle_cmd {
        assert!(closed);
        for pt in points {
            let dist = (pt - session.plane().origin).dot(unit_n).abs();
            assert!(dist <= TOLERANCE * 10.0, "Circle point not on tilted plane");
        }
    } else {
        panic!("Expected AddPolyline");
    }

    // 3. Draw arc on tilted plane and check tangent hint normal
    session.set_kind(ToolKind::Arc);
    session.set_plane(tilted_plane).unwrap();
    session.click(p0).unwrap();
    let p_start = DVec3::new(12.0, 20.0, 29.0);
    session.click(p_start).unwrap();
    let hint = session.tangent_hint().expect("hint on tilted plane");
    assert!((hint.normal - unit_n).length() <= TOLERANCE);
}

#[test]
fn test_preview_command_consistency() {
    // 1. Line
    let mut session = ToolSession::new(ToolKind::Line);
    let start = DVec3::new(1.0, 2.0, 3.0);
    let end = DVec3::new(4.0, 6.0, 3.0);
    session.click(start).unwrap();
    session.pointer_move(end).unwrap();

    let preview = session.preview();
    assert_eq!(preview, vec![start, end]);

    let cmd = session.click(end).unwrap().expect("line cmd");
    assert_eq!(cmd, Command::AddEdge { start, end });

    // 2. Rectangle
    session.set_kind(ToolKind::Rectangle);
    let r0 = DVec3::new(0.0, 0.0, 0.0);
    let r1 = DVec3::new(5.0, 3.0, 0.0);
    session.click(r0).unwrap();
    session.pointer_move(r1).unwrap();

    let rect_preview = session.preview();
    assert_eq!(rect_preview.len(), 5);
    assert_eq!(rect_preview[0], rect_preview[4]); // repeats first point

    let rect_cmd = session.click(r1).unwrap().expect("rect cmd");
    if let Command::AddPolyline { points, closed } = rect_cmd {
        assert!(closed);
        assert_eq!(points.len(), 4);
        assert_eq!(&rect_preview[0..4], &points[..]);
    } else {
        panic!("Expected AddPolyline");
    }

    // 3. Circle
    session.set_kind(ToolKind::Circle);
    let c0 = DVec3::ZERO;
    let c1 = DVec3::new(2.5, 0.0, 0.0);
    session.click(c0).unwrap();
    session.pointer_move(c1).unwrap();

    let circle_preview = session.preview();
    assert_eq!(circle_preview.len(), 49); // 48 segments + 1 repeated
    assert_eq!(circle_preview[0], circle_preview[48]);

    let circle_cmd = session.click(c1).unwrap().expect("circle cmd");
    if let Command::AddPolyline { points, closed } = circle_cmd {
        assert!(closed);
        assert_eq!(points.len(), 48);
        assert_eq!(&circle_preview[0..48], &points[..]);
    } else {
        panic!("Expected AddPolyline");
    }

    // 4. Polygon
    session.set_kind(ToolKind::Polygon);
    session.click(c0).unwrap();
    session.pointer_move(c1).unwrap();

    let poly_preview = session.preview();
    assert_eq!(poly_preview.len(), 7); // 6 sides + 1 repeated
    assert_eq!(poly_preview[0], poly_preview[6]);

    let poly_cmd = session.click(c1).unwrap().expect("poly cmd");
    if let Command::AddPolyline { points, closed } = poly_cmd {
        assert!(closed);
        assert_eq!(points.len(), 6);
        assert_eq!(&poly_preview[0..6], &points[..]);
    } else {
        panic!("Expected AddPolyline");
    }

    // 5. Arc
    session.set_kind(ToolKind::Arc);
    let center = DVec3::ZERO;
    let arc_start = DVec3::new(2.0, 0.0, 0.0);
    let arc_end_dir = DVec3::new(0.0, 2.0, 0.0);
    session.click(center).unwrap();
    session.click(arc_start).unwrap();
    session.pointer_move(arc_end_dir).unwrap();

    let arc_preview = session.preview();
    assert_eq!(arc_preview.len(), 25); // 24 segments + 1 end point (open)

    let arc_cmd = session.click(arc_end_dir).unwrap().expect("arc cmd");
    if let Command::AddPolyline { points, closed } = arc_cmd {
        assert!(!closed);
        assert_eq!(points.len(), 25);
        assert_eq!(arc_preview, points);
    } else {
        panic!("Expected AddPolyline");
    }
}

#[test]
fn test_typed_unit_measurements_and_invalid_errors_and_cancel() {
    // 1. Length parser
    assert_eq!(parse_length("10m").unwrap(), 10.0);
    assert_eq!(parse_length("2500mm").unwrap(), 2.5);
    assert_eq!(parse_length("50cm").unwrap(), 0.5);
    assert!((parse_length("12in").unwrap() - 0.3048).abs() < 1e-9);
    assert!((parse_length("3ft").unwrap() - 0.9144).abs() < 1e-9);
    assert_eq!(parse_length("-5.2m").unwrap(), -5.2);

    assert!(matches!(
        parse_length("10yd"),
        Err(ToolError::InvalidMeasurement(_))
    ));
    assert!(matches!(
        parse_length("10km"),
        Err(ToolError::InvalidMeasurement(_))
    ));
    assert!(matches!(
        parse_length("abc"),
        Err(ToolError::InvalidMeasurement(_))
    ));

    // 2. Measure on Line
    let mut session = ToolSession::new(ToolKind::Line);
    assert!(matches!(session.measure("10m"), Err(ToolError::NotReady)));

    session.click(DVec3::ZERO).unwrap();
    // Falls back to plane +U (+X on default XY plane)
    let line_cmd1 = session.measure("10m").unwrap().expect("line cmd1");
    assert_eq!(
        line_cmd1,
        Command::AddEdge {
            start: DVec3::ZERO,
            end: DVec3::new(10.0, 0.0, 0.0)
        }
    );
    // Next measure falls back to previous_direction (+X)
    let line_cmd2 = session.measure("5000mm").unwrap().expect("line cmd2");
    assert_eq!(
        line_cmd2,
        Command::AddEdge {
            start: DVec3::new(10.0, 0.0, 0.0),
            end: DVec3::new(15.0, 0.0, 0.0)
        }
    );
    // Measure along preview direction (+Y)
    session.pointer_move(DVec3::new(15.0, 5.0, 0.0)).unwrap();
    let line_cmd3 = session.measure("2m").unwrap().expect("line cmd3");
    assert_eq!(
        line_cmd3,
        Command::AddEdge {
            start: DVec3::new(15.0, 0.0, 0.0),
            end: DVec3::new(15.0, 2.0, 0.0)
        }
    );

    // Negative / zero measurement rejected
    assert!(matches!(
        session.measure("0m"),
        Err(ToolError::InvalidMeasurement(_))
    ));
    assert!(matches!(
        session.measure("-5m"),
        Err(ToolError::InvalidMeasurement(_))
    ));

    // 3. Measure on Rectangle
    session.set_kind(ToolKind::Rectangle);
    session.click(DVec3::ZERO).unwrap();
    assert!(matches!(
        session.measure("10m"), // Missing comma
        Err(ToolError::InvalidMeasurement(_))
    ));
    // State preserved after bad measurement!
    assert_eq!(session.anchor(), Some(DVec3::ZERO));
    let rect_cmd = session.measure("4m, 3m").unwrap().expect("rect cmd");
    if let Command::AddPolyline { points, closed } = rect_cmd {
        assert!(closed);
        assert_eq!(points[0], DVec3::ZERO);
        assert_eq!(points[1], DVec3::new(4.0, 0.0, 0.0));
        assert_eq!(points[2], DVec3::new(4.0, 3.0, 0.0));
        assert_eq!(points[3], DVec3::new(0.0, 3.0, 0.0));
    } else {
        panic!("Expected AddPolyline");
    }

    // 4. Measure on Arc (signed degrees, optional deg suffix)
    session.set_kind(ToolKind::Arc);
    session.click(DVec3::ZERO).unwrap();
    assert!(matches!(session.measure("90deg"), Err(ToolError::NotReady)));

    session.click(DVec3::new(2.0, 0.0, 0.0)).unwrap();
    // Test invalid angles
    assert!(matches!(
        session.measure("0deg"),
        Err(ToolError::InvalidMeasurement(_))
    ));
    assert!(matches!(
        session.measure("360deg"),
        Err(ToolError::InvalidMeasurement(_))
    ));
    assert!(matches!(
        session.measure("400deg"),
        Err(ToolError::InvalidMeasurement(_))
    ));
    // State preserved!
    assert_eq!(session.anchors().len(), 2);

    let arc_cmd = session.measure("90deg").unwrap().expect("arc 90 cmd");
    if let Command::AddPolyline { points, closed } = arc_cmd {
        assert!(!closed);
        assert_eq!(points[0], DVec3::new(2.0, 0.0, 0.0));
        let last = points.last().unwrap();
        assert!((last - DVec3::new(0.0, 2.0, 0.0)).length() <= TOLERANCE);
    } else {
        panic!("Expected AddPolyline");
    }

    // Test negative degrees on Arc
    session.click(DVec3::ZERO).unwrap();
    session.click(DVec3::new(2.0, 0.0, 0.0)).unwrap();
    let arc_cw = session.measure("-90").unwrap().expect("arc -90 cmd");
    if let Command::AddPolyline { points, closed } = arc_cw {
        assert!(!closed);
        let last = points.last().unwrap();
        assert!((last - DVec3::new(0.0, -2.0, 0.0)).length() <= TOLERANCE);
    }

    // 5. Cancellation
    session.set_kind(ToolKind::Rectangle);
    session.click(DVec3::ZERO).unwrap();
    assert_eq!(session.anchor(), Some(DVec3::ZERO));
    session.cancel();
    assert_eq!(session.anchor(), None);
    assert_eq!(session.anchors(), &[]);
}

#[test]
fn test_chained_line_behavior() {
    let mut session = ToolSession::new(ToolKind::Line);
    let mut history = History::new();

    // Start at (0,0,0)
    session.click(DVec3::ZERO).unwrap();

    // Click 1 -> (5,0,0)
    let c1 = session.click(DVec3::new(5.0, 0.0, 0.0)).unwrap().unwrap();
    history.execute(c1).unwrap();
    assert_eq!(session.anchor(), Some(DVec3::new(5.0, 0.0, 0.0)));
    assert_eq!(session.previous_direction(), Some(DVec3::X));

    // Try clicking same point (degenerate) -> error, state preserved
    assert!(matches!(
        session.click(DVec3::new(5.0, 0.0, 0.0)),
        Err(ToolError::Degenerate)
    ));
    assert_eq!(session.anchor(), Some(DVec3::new(5.0, 0.0, 0.0)));

    // Measure next segment along previous_direction (+X)
    let c2 = session.measure("5m").unwrap().unwrap();
    history.execute(c2).unwrap();
    assert_eq!(session.anchor(), Some(DVec3::new(10.0, 0.0, 0.0)));
    assert_eq!(session.previous_direction(), Some(DVec3::X));

    // Click to (10, 10, 0)
    let c3 = session.click(DVec3::new(10.0, 10.0, 0.0)).unwrap().unwrap();
    history.execute(c3).unwrap();
    assert_eq!(session.previous_direction(), Some(DVec3::Y));

    // Click to (0, 10, 0)
    let c4 = session.click(DVec3::new(0.0, 10.0, 0.0)).unwrap().unwrap();
    history.execute(c4).unwrap();
    assert_eq!(session.previous_direction(), Some(-DVec3::X));

    // Click back to origin to close loop
    let c5 = session.click(DVec3::ZERO).unwrap().unwrap();
    history.execute(c5).unwrap();

    // Face created!
    assert_eq!(history.model().faces().len(), 1);

    // Cancel uncommitted interaction
    session.cancel();
    assert_eq!(session.anchor(), None);
    // Cancellation does not erase committed history
    assert_eq!(history.model().faces().len(), 1);
}

#[test]
fn test_arc_radius_sweep_and_segment_settings() {
    let mut session = ToolSession::new(ToolKind::Arc);
    assert_eq!(session.segments(), 24);

    // Segment limits 3..=256
    assert!(matches!(
        session.set_segments(2),
        Err(ToolError::InvalidSegments)
    ));
    assert!(matches!(
        session.set_segments(257),
        Err(ToolError::InvalidSegments)
    ));
    assert_eq!(session.segments(), 24); // preserved

    session.set_segments(3).unwrap();
    assert_eq!(session.segments(), 3);
    session.set_segments(256).unwrap();
    assert_eq!(session.segments(), 256);
    session.set_segments(12).unwrap();
    assert_eq!(session.segments(), 12);

    // Arc geometry test: CCW sweep, exact radius preserved
    let center = DVec3::new(1.0, 1.0, 0.0);
    let start = DVec3::new(4.0, 1.0, 0.0); // radius = 3
    session.click(center).unwrap();
    session.click(start).unwrap();

    let end_dir = DVec3::new(1.0, 4.0, 0.0); // 90 deg CCW
    let cmd = session.click(end_dir).unwrap().expect("arc cmd");
    if let Command::AddPolyline { points, closed } = cmd {
        assert!(!closed);
        assert_eq!(points.len(), 13); // 12 segments + 1
        for pt in &points {
            let r = pt.distance(center);
            assert!(
                (r - 3.0).abs() <= TOLERANCE,
                "Arc point {:?} radius is {}, expected 3.0",
                pt,
                r
            );
        }
        // CCW check: intermediate points must have positive Y relative to center
        for pt in &points[1..12] {
            assert!(pt.y > center.y);
            assert!(pt.x > center.x);
        }
    } else {
        panic!("Expected AddPolyline");
    }

    // Degenerate arc clicks
    session.click(center).unwrap();
    // Start equal to center
    assert!(matches!(session.click(center), Err(ToolError::Degenerate)));
    assert_eq!(session.anchors().len(), 1); // preserved

    session.click(start).unwrap();
    // End direction equal to center
    assert!(matches!(session.click(center), Err(ToolError::Degenerate)));
    // End direction collinear with start (angle 0)
    assert!(matches!(session.click(start), Err(ToolError::Degenerate)));
    assert_eq!(session.anchors().len(), 2); // preserved
}

#[test]
fn test_default_tool_segment_counts() {
    assert_eq!(ToolSession::new(ToolKind::Circle).segments(), 48);
    assert_eq!(ToolSession::new(ToolKind::Polygon).segments(), 6);
    assert_eq!(ToolSession::new(ToolKind::Arc).segments(), 24);
    assert_eq!(ToolSession::new(ToolKind::Line).segments(), 48);
    assert_eq!(ToolSession::new(ToolKind::Rectangle).segments(), 48);

    let mut session = ToolSession::new(ToolKind::Circle);
    session.set_kind(ToolKind::Polygon);
    assert_eq!(session.segments(), 6);
    session.set_kind(ToolKind::Arc);
    assert_eq!(session.segments(), 24);
}

#[test]
fn line_work_plane_follows_elevated_anchors_and_tiny_curve_errors_preserve_state() {
    let mut line = ToolSession::new(ToolKind::Line);
    line.click(DVec3::new(0., 0., 5.)).unwrap();
    assert_eq!(line.plane().origin, DVec3::new(0., 0., 5.));
    line.click(DVec3::new(1., 0., 6.)).unwrap();
    assert_eq!(line.plane().origin, DVec3::new(1., 0., 6.));
    let mut circle = ToolSession::new(ToolKind::Circle);
    circle.click(DVec3::ZERO).unwrap();
    assert_eq!(
        circle.click(DVec3::X * TOLERANCE * 2.),
        Err(ToolError::Degenerate)
    );
    assert_eq!(circle.anchor(), Some(DVec3::ZERO));
    assert_eq!(circle.measure("0.0000002"), Err(ToolError::Degenerate));
    assert_eq!(circle.anchor(), Some(DVec3::ZERO));
}
