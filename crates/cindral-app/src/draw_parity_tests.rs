//! Synthetic egui routing checks: no window or GPU.
use super::*;

const KINDS: [ToolKind; 5] = [
    ToolKind::Line,
    ToolKind::Rectangle,
    ToolKind::Circle,
    ToolKind::Arc,
    ToolKind::Polygon,
];

fn key(key: Key, modifiers: egui::Modifiers) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers,
    }
}
fn frame(
    app: &mut DrafterApp,
    ctx: &egui::Context,
    events: Vec<egui::Event>,
    modifiers: egui::Modifiers,
) {
    input_tests::frame(
        app,
        ctx,
        vec![egui::Event::PointerMoved(egui::pos2(300., 200.))],
        egui::Modifiers::NONE,
    );
    let mut hovered = vec![egui::Event::PointerMoved(egui::pos2(300., 200.))];
    hovered.extend(events);
    input_tests::frame(app, ctx, hovered, modifiers);
}

#[test]
fn draw_escape_and_undo_redo_reset_all_tool_anchors() {
    for kind in KINDS {
        let mut app = DrafterApp::default();
        app.set_tool(kind);
        let ctx = egui::Context::default();
        frame(&mut app, &ctx, vec![], egui::Modifiers::NONE);
        app.tool.click(DVec3::ZERO).unwrap();
        if kind == ToolKind::Arc {
            app.tool.click(DVec3::X).unwrap();
        }
        frame(
            &mut app,
            &ctx,
            vec![key(Key::Escape, egui::Modifiers::NONE)],
            egui::Modifiers::NONE,
        );
        assert!(app.tool.anchors().is_empty());
        assert_eq!(app.history.undo_depth(), 0);
        assert!(app.model().edges().is_empty());
        app.tool.click(DVec3::ZERO).unwrap();
        if kind == ToolKind::Arc {
            app.tool.click(DVec3::X).unwrap();
        }
        let command = app.tool.click(DVec3::new(2., 3., 0.)).unwrap().unwrap();
        assert!(app.commit(command));
        let after = app.model().clone();
        if kind != ToolKind::Line {
            app.tool.click(DVec3::splat(10.)).unwrap();
        }
        let ctrl = egui::Modifiers {
            ctrl: true,
            command: true,
            ..Default::default()
        };
        frame(&mut app, &ctx, vec![key(Key::Z, ctrl)], ctrl);
        assert!(app.model().edges().is_empty(), "{kind:?}");
        assert!(app.tool.anchors().is_empty());
        frame(&mut app, &ctx, vec![key(Key::Y, ctrl)], ctrl);
        assert_eq!(app.model(), &after);
        assert!(app.tool.anchors().is_empty());
    }
}

#[test]
fn draw_keyboard_metric_input_commits_all_tools_and_invalid_input_reports_error() {
    for kind in KINDS {
        let mut app = DrafterApp::default();
        app.set_tool(kind);
        let ctx = egui::Context::default();
        app.tool.click(DVec3::ZERO).unwrap();
        if kind == ToolKind::Arc {
            app.tool.click(DVec3::X).unwrap();
        }
        let text = match kind {
            ToolKind::Rectangle => "2m,150cm",
            ToolKind::Arc => "90deg",
            _ => "150cm",
        };
        frame(
            &mut app,
            &ctx,
            vec![
                egui::Event::Text(text.into()),
                key(Key::Enter, egui::Modifiers::NONE),
            ],
            egui::Modifiers::NONE,
        );
        assert_eq!(app.history.undo_depth(), 1, "{kind:?}");
        app.model().validate().unwrap();
        app.tool.cancel();
        app.tool.click(DVec3::splat(10.)).unwrap();
        let before = app.model().clone();
        app.measurements.text = "garbage".into();
        app.apply_measurement();
        assert!(app.error.is_some());
        assert_eq!(app.model(), &before);
        assert_eq!(app.history.undo_depth(), 1);
    }
}

#[test]
fn draw_line_and_rectangle_drag_release_commit_once() {
    for kind in [ToolKind::Line, ToolKind::Rectangle] {
        let mut app = DrafterApp::default();
        app.set_tool(kind);
        let ctx = egui::Context::default();
        let start = egui::pos2(260., 160.);
        let end = egui::pos2(390., 270.);
        let button = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        input_tests::frame(
            &mut app,
            &ctx,
            vec![egui::Event::PointerMoved(start)],
            egui::Modifiers::NONE,
        );
        input_tests::frame(
            &mut app,
            &ctx,
            vec![button(start, true)],
            egui::Modifiers::NONE,
        );
        input_tests::frame(
            &mut app,
            &ctx,
            vec![egui::Event::PointerMoved(end)],
            egui::Modifiers::NONE,
        );
        assert_eq!(app.history.undo_depth(), 0);
        input_tests::frame(
            &mut app,
            &ctx,
            vec![button(end, false)],
            egui::Modifiers::NONE,
        );
        assert_eq!(app.history.undo_depth(), 1, "{kind:?}");
        app.model().validate().unwrap();
    }
}

#[test]
#[ignore = "gap: arrow keys constrain a point axis and do not set shape drawing plane"]
fn draw_circle_right_arrow_sets_red_normal_plane() {
    let mut app = DrafterApp::default();
    app.set_tool(ToolKind::Circle);
    let ctx = egui::Context::default();
    frame(
        &mut app,
        &ctx,
        vec![key(Key::ArrowRight, egui::Modifiers::NONE)],
        egui::Modifiers::NONE,
    );
    assert!(app.tool.plane().normal.normalize().dot(DVec3::X).abs() > 0.99);
}

#[cfg(feature = "kernel-benchmark")]
#[test]
#[ignore = "manual release-only CPU probe, excludes inference/mesh/GPU and full frame latency"]
#[allow(clippy::assertions_on_constants)]
fn draw_20k_cpu_budget() {
    use std::time::{Duration, Instant};
    assert!(!cfg!(debug_assertions), "run with --release");
    for kind in KINDS {
        let mut fixture = cindral_core::benchmark::tiled_squares(100, 100).unwrap();
        assert_eq!(fixture.history.model().faces().len(), 20_000);
        let mut tool = ToolSession::new(kind);
        let origin = DVec3::new(-10., -10., 0.);
        tool.click(origin).unwrap();
        if kind == ToolKind::Arc {
            tool.click(origin + DVec3::X).unwrap();
        }
        let mut worst = Duration::ZERO;
        for i in 1..=30 {
            let start = Instant::now();
            tool.pointer_move(origin + DVec3::new(2., 1. + i as f64 * 0.01, 0.))
                .unwrap();
            std::hint::black_box(tool.preview());
            worst = worst.max(start.elapsed());
        }
        let command = tool
            .click(origin + DVec3::new(2., 3., 0.))
            .unwrap()
            .unwrap();
        let start = Instant::now();
        assert!(fixture.history.execute(command).unwrap());
        let commit = start.elapsed();
        println!("{kind:?}: CPU preview max {:.6} ms; kernel commit {:.3} ms; 20000-face fixture; 30 samples", worst.as_secs_f64()*1000., commit.as_secs_f64()*1000.);
        assert!(worst < Duration::from_millis(16));
        assert!(commit < Duration::from_millis(100));
    }
}
