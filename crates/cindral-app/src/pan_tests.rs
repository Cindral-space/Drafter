//! Production input routing tests, without a window or GPU.
use super::*;
use egui::{Event, Pos2};
fn key(key: Key) -> Event {
    Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE }
}
fn button(pos: Pos2, button: PointerButton, pressed: bool, shift: bool) -> Event {
    Event::PointerButton { pos, button, pressed, modifiers: egui::Modifiers { shift, ..Default::default() } }
}
fn frame(app: &mut DrafterApp, ctx: &egui::Context, events: Vec<Event>, shift: bool) {
    input_tests::frame(app, ctx, events, egui::Modifiers { shift, ..Default::default() });
}
fn setup() -> (DrafterApp, egui::Context) {
    let mut app = DrafterApp::default();
    app.execute(Command::AddEdge { start: DVec3::ZERO, end: DVec3::X }).unwrap();
    app.selection.edges = app.model().edges().keys().copied().collect();
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, vec![Event::PointerMoved(egui::pos2(300., 200.))], false);
    (app, ctx)
}
fn drag(app: &mut DrafterApp, ctx: &egui::Context, button_kind: PointerButton, shift: bool) {
    let p = egui::pos2(300., 200.);
    frame(app, ctx, vec![button(p, button_kind, true, shift)], shift);
    assert!(app.pan_drag.is_some());
    for i in 1..=5 {
        frame(app, ctx, vec![Event::PointerMoved(p + egui::vec2(i as f32 * 8., i as f32 * 3.))], shift);
    }
    frame(app, ctx, vec![button(p + egui::vec2(40., 15.), button_kind, false, shift)], shift);
    assert!(app.pan_drag.is_none());
}
#[test]
fn h_primary_and_temporary_pan_have_identical_camera_and_document_invariants() {
    let (mut primary, ctx) = setup();
    let before = primary.camera.clone();
    let model = primary.model().to_data();
    let selection = primary.selection.clone();
    let context = primary.history.context().to_vec();
    let depth = (primary.history.undo_depth(), primary.history.redo_depth());
    let revision = primary.history.revision();
    frame(&mut primary, &ctx, vec![key(Key::H)], false);
    assert_eq!(primary.shortcuts.navigation, Some(shortcuts::Action::Pan));
    drag(&mut primary, &ctx, PointerButton::Primary, false);
    assert_ne!(primary.camera.target, before.target);
    assert_eq!(primary.shortcuts.navigation, Some(shortcuts::Action::Pan));
    assert_eq!(primary.model().to_data(), model);
    assert_eq!(primary.selection, selection);
    assert_eq!(primary.history.context(), context);
    assert_eq!((primary.history.undo_depth(), primary.history.redo_depth()), depth);
    assert_eq!(primary.history.revision(), revision);
    for kind in [ToolKind::Line, ToolKind::Rectangle, ToolKind::Circle, ToolKind::Arc, ToolKind::Polygon] {
        let (mut temporary, ctx) = setup();
        temporary.set_tool(kind);
        drag(&mut temporary, &ctx, PointerButton::Middle, true);
        assert_eq!(temporary.camera, primary.camera);
        assert_eq!(temporary.tool.kind(), kind);
        assert_eq!(temporary.model().to_data(), model);
        assert_eq!(temporary.selection, selection);
        assert_eq!(temporary.history.revision(), revision);
    }
    for kind in [ModifyKind::Select, ModifyKind::Move, ModifyKind::Copy, ModifyKind::Rotate,
        ModifyKind::Scale, ModifyKind::PushPull, ModifyKind::Offset, ModifyKind::FollowMe,
        ModifyKind::Eraser, ModifyKind::TapeMeasure, ModifyKind::PaintBucket] {
        let (mut temporary, ctx) = setup();
        temporary.choose_modify(kind);
        drag(&mut temporary, &ctx, PointerButton::Middle, true);
        assert_eq!(temporary.camera, primary.camera);
        assert_eq!(temporary.modify.kind(), kind);
        assert_eq!(temporary.model().to_data(), model);
        assert_eq!(temporary.selection, selection);
    }
}
#[test]
fn temporary_pan_retains_unfinished_line_but_h_cancels_preview() {
    let (mut app, ctx) = setup();
    let p = egui::pos2(300., 200.);
    frame(&mut app, &ctx, vec![button(p, PointerButton::Primary, true, false)], false);
    frame(&mut app, &ctx, vec![button(p, PointerButton::Primary, false, false)], false);
    let anchor = app.tool.anchor();
    assert!(anchor.is_some());
    drag(&mut app, &ctx, PointerButton::Middle, true);
    assert_eq!(app.tool.anchor(), anchor);
    let model = app.model().to_data();
    let depth = app.history.undo_depth();
    app.history.preview(Command::AddEdge { start: DVec3::Y, end: DVec3::Y + DVec3::X }).unwrap();
    frame(&mut app, &ctx, vec![key(Key::H)], false);
    assert!(app.tool.anchor().is_none());
    assert_eq!(app.model().to_data(), model);
    assert_eq!(app.history.undo_depth(), depth);
}
#[test]
fn escape_switch_resize_and_focus_loss_end_drag_without_resuming_held_press() {
    for abort in 0..5 {
        let (mut app, ctx) = setup();
        app.activate_pan();
        let p = egui::pos2(300., 200.);
        frame(&mut app, &ctx, vec![button(p, PointerButton::Primary, true, false)], false);
        frame(&mut app, &ctx, vec![Event::PointerMoved(p + egui::vec2(10., 5.))], false);
        assert!(app.pan_drag.is_some());
        match abort {
            0 => frame(&mut app, &ctx, vec![key(Key::Escape)], false),
            1 => frame(&mut app, &ctx, vec![key(Key::L)], false),
            _ => {
                let _ = ctx.run(egui::RawInput {
                    focused: abort != 3,
                    screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(800., 600.))),
                    ..Default::default()
                }, |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        let size = egui::vec2(if abort == 2 { 500. } else if abort == 4 { 0. } else { 600. }, 400.);
                        let (_, response) = ui.allocate_exact_size(size, egui::Sense::click_and_drag());
                        app.viewport_input(&response);
                    });
                });
            }
        }
        assert!(app.pan_drag.is_none(), "abort {abort}");
        if abort == 0 { assert_eq!(app.shortcuts.navigation, Some(shortcuts::Action::Pan)); }
        let target = app.camera.target;
        frame(&mut app, &ctx, vec![Event::PointerMoved(p + egui::vec2(40., 15.))], false);
        assert!(app.pan_drag.is_none());
        assert_eq!(app.camera.target, target);
    }
}
#[test]
fn panel_and_floating_dialog_presses_never_start_pan() {
    for floating in [false, true] {
        let mut app = DrafterApp::default();
        app.activate_pan();
        let ctx = egui::Context::default();
        let before = app.camera.clone();
        let p = egui::pos2(60., 60.);
        for events in [vec![Event::PointerMoved(p)], vec![button(p, PointerButton::Primary, true, false)], vec![Event::PointerMoved(egui::pos2(400., 200.))]] {
            let _ = ctx.run(egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(800., 600.))), events, ..Default::default()
            }, |ctx| {
                if floating {
                    egui::Window::new("Dialog").fixed_pos(egui::pos2(20., 20.)).fixed_size(egui::vec2(160., 140.)).show(ctx, |ui| { ui.button("Dialog control").clicked(); });
                } else {
                    egui::SidePanel::left("panel").exact_width(180.).show(ctx, |ui| { ui.button("Panel control").clicked(); });
                }
                egui::CentralPanel::default().show(ctx, |ui| {
                    let (_, response) = ui.allocate_exact_size(ui.available_size(), egui::Sense::click_and_drag());
                    app.viewport_input(&response);
                });
            });
        }
        assert!(app.pan_drag.is_none());
        assert_eq!(app.camera, before);
    }
}
#[test]
fn h_in_text_editor_is_text_and_pan_scroll_preserves_distance() {
    let (mut app, ctx) = setup();
    let mut text = String::new();
    for events in [vec![Event::PointerMoved(egui::pos2(300., 200.))], vec![key(Key::H), Event::Text("h".into())]] {
        let _ = ctx.run(egui::RawInput { events, ..Default::default() }, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let edit = ui.text_edit_singleline(&mut text);
                edit.request_focus();
                let (_, response) = ui.allocate_exact_size(egui::vec2(600., 400.), egui::Sense::click_and_drag());
                app.viewport_input(&response);
            });
        });
    }
    assert_eq!(text, "h");
    assert!(app.shortcuts.navigation.is_none());
    let (mut app, ctx) = setup();
    app.activate_pan();
    let before = app.camera.clone();
    frame(&mut app, &ctx, vec![Event::PointerMoved(egui::pos2(300., 200.)), Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point, delta: egui::vec2(20., 40.), modifiers: egui::Modifiers::NONE
    }], false);
    assert_eq!(app.camera.distance, before.distance);
    assert_ne!(app.camera.target, before.target);
}
#[test]
fn saved_scenes_are_unchanged_and_restoring_still_animates() {
    let (mut app, ctx) = setup();
    let scene = app.history.capture_scene("Before pan".into(), cindral_core::SceneCamera {
        target: app.camera.target.to_array(), distance: app.camera.distance,
        yaw: app.camera.yaw, pitch: app.camera.pitch, orthographic: false,
    }, app.presentation.style.clone());
    app.presentation.scenes.push(scene);
    let scenes = app.presentation.scenes.clone();
    let serial = app.presentation_serial;
    app.activate_pan();
    drag(&mut app, &ctx, PointerButton::Primary, false);
    assert_eq!(app.presentation.scenes, scenes);
    assert_eq!(app.presentation_serial, serial);
    let panned = app.camera.clone();
    app.restore_scene(0, 10., true);
    assert_eq!(app.camera, panned);
    let _ = ctx.run(egui::RawInput { time: Some(10.1), ..Default::default() }, |ctx| app.transition_tick(ctx));
    assert_ne!(app.camera, panned);
    assert_ne!(app.camera.target.to_array(), scenes[0].camera.target);
}
#[test]
fn temporary_drag_suspends_inference_then_hover_resumes() {
    let (mut app, ctx) = setup();
    let snap = app.snap.as_ref().map(|s| s.position);
    assert!(snap.is_some());
    let p = egui::pos2(300., 200.);
    frame(&mut app, &ctx, vec![button(p, PointerButton::Middle, true, true)], true);
    frame(&mut app, &ctx, vec![Event::PointerMoved(p + egui::vec2(40., 15.))], true);
    assert_eq!(app.snap.as_ref().map(|s| s.position), snap);
    // Even when measurement text exists, Escape belongs to an owned pan first.
    app.measurements.text = "3".into();
    frame(&mut app, &ctx, vec![key(Key::Escape)], true);
    assert!(app.pan_drag.is_none());
    assert_eq!(app.measurements.text, "3");
    app.measurements.reset();
    frame(&mut app, &ctx, vec![button(p + egui::vec2(40., 15.), PointerButton::Middle, false, false)], false);
    frame(&mut app, &ctx, vec![Event::PointerMoved(p + egui::vec2(80., 45.))], false);
    assert_ne!(app.snap.as_ref().map(|s| s.position), snap);
}
