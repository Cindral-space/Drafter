//! CPU-only input regressions; no display server or GPU is required.
use super::*;
use shortcuts::{Action, Binding, Context, MAP};
fn event(key: Key, ctrl: bool, shift: bool, repeat: bool) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: Some(Key::V),
        pressed: true,
        repeat,
        modifiers: egui::Modifiers {
            ctrl,
            command: ctrl,
            shift,
            ..Default::default()
        },
    }
}
fn frame(app: &mut DrafterApp, ctx: &egui::Context, events: Vec<egui::Event>) {
    let mut released = Vec::new();
    for e in events {
        if let egui::Event::Key {
            key,
            physical_key,
            pressed: true,
            repeat: false,
            modifiers,
        } = e
        {
            released.push(egui::Event::Key {
                key,
                physical_key,
                pressed: false,
                repeat: false,
                modifiers,
            });
        }
        released.push(e);
    }
    input_tests::frame(app, ctx, released, egui::Modifiers::NONE);
}
fn setup() -> (DrafterApp, egui::Context) {
    let mut app = DrafterApp::default();
    app.choose_modify(ModifyKind::Select);
    let ctx = egui::Context::default();
    frame(
        &mut app,
        &ctx,
        vec![egui::Event::PointerMoved(egui::pos2(300., 200.))],
    );
    (app, ctx)
}
fn activate(app: &mut DrafterApp, ctx: &egui::Context, key: Key) {
    frame(app, ctx, vec![event(key, false, false, false)]);
}
#[test]
fn map_is_complete_unique_and_explicit() {
    for (idx, e) in MAP.iter().enumerate() {
        assert!(!e.bindings.is_empty() || e.unbound.is_some(), "{}", e.label);
        for b in e.bindings {
            for other in &MAP[idx + 1..] {
                if other.context == e.context {
                    for c in other.bindings {
                        assert_ne!(
                            (b.key, b.ctrl, b.shift),
                            (c.key, c.ctrl, c.shift),
                            "{} / {}",
                            e.label,
                            other.label
                        );
                    }
                }
            }
        }
    }
    for k in [
        ToolKind::Line,
        ToolKind::Rectangle,
        ToolKind::Circle,
        ToolKind::Arc,
        ToolKind::Polygon,
    ] {
        assert!(MAP.iter().any(|e| e.action == Action::Draw(k)));
    }
    for k in [
        ModifyKind::Select,
        ModifyKind::Eraser,
        ModifyKind::PushPull,
        ModifyKind::Move,
        ModifyKind::Copy,
        ModifyKind::Rotate,
        ModifyKind::Scale,
        ModifyKind::Offset,
        ModifyKind::FollowMe,
        ModifyKind::TapeMeasure,
        ModifyKind::PaintBucket,
    ] {
        assert!(MAP.iter().any(|e| e.action == Action::Modify(k)));
    }
}
#[test]
fn all_tool_bindings_activate_from_select_and_preserve_selection() {
    for e in MAP
        .iter()
        .filter(|e| e.category == "Tools" && !e.bindings.is_empty())
    {
        let (mut app, ctx) = setup();
        app.execute(Command::AddEdge {
            start: DVec3::ZERO,
            end: DVec3::X,
        })
        .unwrap();
        app.selection.edges = app.model().edges().keys().copied().collect();
        let selected = app.selection.clone();
        activate(&mut app, &ctx, e.bindings[0].key);
        match e.action {
            Action::Draw(k) => {
                assert!(!app.modifying);
                assert_eq!(app.tool.kind(), k);
            }
            Action::Modify(k) => {
                assert!(app.modifying);
                assert_eq!(app.modify.kind(), k);
            }
            _ => unreachable!(),
        }
        assert_eq!(app.selection, selected);
    }
}
#[test]
fn text_focus_and_measurement_text_block_every_canvas_binding() {
    for e in MAP.iter().filter(|e| e.context == Context::Canvas) {
        for b in e.bindings {
            let (mut app, ctx) = setup();
            app.measurements.text = "3".into();
            frame(&mut app, &ctx, vec![event(b.key, b.ctrl, b.shift, false)]);
            assert!(app.modifying);
            assert_eq!(app.modify.kind(), ModifyKind::Select);
            assert!(app.shortcuts.navigation.is_none());
            assert!(!app.shortcuts.open);
            let id = egui::Id::new("focused editor");
            ctx.memory_mut(|m| m.request_focus(id));
            app.measurements.text.clear();
            frame(&mut app, &ctx, vec![event(b.key, b.ctrl, b.shift, false)]);
            assert!(app.modifying);
            assert_eq!(app.modify.kind(), ModifyKind::Select);
        }
    }
}
#[test]
fn measurements_keep_unit_letters_and_escape_releases_shortcuts() {
    let (mut app, ctx) = setup();
    activate(&mut app, &ctx, Key::L);
    frame(&mut app, &ctx, vec![egui::Event::Text("3".into())]);
    frame(
        &mut app,
        &ctx,
        vec![
            event(Key::M, false, false, false),
            egui::Event::Text("m".into()),
            event(Key::X, false, false, false),
            egui::Event::Text("x".into()),
        ],
    );
    assert_eq!(app.measurements.text, "3mx");
    assert!(!app.modifying);
    assert_eq!(app.tool.kind(), ToolKind::Line);
    activate(&mut app, &ctx, Key::Escape);
    activate(&mut app, &ctx, Key::R);
    assert_eq!(app.tool.kind(), ToolKind::Rectangle);
}
#[test]
fn modifiers_repeat_and_logical_keys() {
    let (mut app, ctx) = setup();
    for modifiers in [
        egui::Modifiers::CTRL,
        egui::Modifiers::ALT,
        egui::Modifiers::SHIFT,
        egui::Modifiers::MAC_CMD,
    ] {
        frame(
            &mut app,
            &ctx,
            vec![egui::Event::Key {
                key: Key::L,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers,
            }],
        );
        assert!(app.modifying);
    }
    frame(&mut app, &ctx, vec![event(Key::L, false, false, true)]);
    assert!(app.modifying);
    frame(&mut app, &ctx, vec![event(Key::Z, true, false, false)]);
    assert!(app.shortcuts.navigation.is_none());
    activate(&mut app, &ctx, Key::L);
    assert_eq!(app.tool.kind(), ToolKind::Line);
    assert!(!app.modifying);
    activate(&mut app, &ctx, Key::Num5);
    let projection = app.camera.projection;
    frame(&mut app, &ctx, vec![event(Key::Num5, false, false, true)]);
    assert_eq!(app.camera.projection, projection);
}
#[test]
fn mid_operation_switch_rolls_back_preview_and_keeps_context_selection() {
    let (mut app, ctx) = setup();
    app.execute(Command::AddEdge {
        start: DVec3::ZERO,
        end: DVec3::X,
    })
    .unwrap();
    let before = app.model().to_data();
    let depth = app.history.undo_depth();
    app.selection.edges = app.model().edges().keys().copied().collect();
    let selection = app.selection.clone();
    app.choose_modify(ModifyKind::Move);
    app.history
        .preview(Command::AddEdge {
            start: DVec3::Y,
            end: DVec3::Y + DVec3::X,
        })
        .unwrap();
    activate(&mut app, &ctx, Key::R);
    assert_eq!(app.model().to_data(), before);
    assert_eq!(app.history.undo_depth(), depth);
    assert_eq!(app.selection, selection);
    assert!(app.history.context().is_empty());
    assert_eq!(app.tool.kind(), ToolKind::Rectangle);
}
#[test]
fn changed_bindings_navigation_history_and_help() {
    let (mut app, ctx) = setup();
    activate(&mut app, &ctx, Key::N);
    assert_eq!(app.tool.kind(), ToolKind::Polygon);
    activate(&mut app, &ctx, Key::P);
    assert!(app.modifying);
    assert_eq!(app.modify.kind(), ModifyKind::PushPull);
    app.camera.distance = 500.;
    activate(&mut app, &ctx, Key::F);
    assert_eq!(app.camera.distance, 500.);
    assert_eq!(app.modify.kind(), ModifyKind::Offset);
    frame(&mut app, &ctx, vec![event(Key::Z, false, true, false)]);
    assert_ne!(app.camera.distance, 500.);
    for (k, a) in [
        (Key::O, Action::Orbit),
        (Key::H, Action::Pan),
        (Key::Z, Action::Zoom),
    ] {
        activate(&mut app, &ctx, k);
        assert_eq!(app.shortcuts.navigation, Some(a));
    }
    activate(&mut app, &ctx, Key::Escape);
    assert!(app.shortcuts.navigation.is_none());
    activate(&mut app, &ctx, Key::F1);
    assert!(app.shortcuts.open);
    ctx.memory_mut(|m| m.request_focus(egui::Id::new("help search")));
    let _ = ctx.run(
        egui::RawInput {
            events: vec![
                egui::Event::Key {
                    key: Key::Escape,
                    physical_key: None,
                    pressed: false,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                },
                event(Key::Escape, false, false, false),
            ],
            ..Default::default()
        },
        |ctx| app.shortcut_overlay(ctx),
    );
    assert!(!app.shortcuts.open);
    assert!(ctx.memory(|m| m.focused().is_none()));
    frame(
        &mut app,
        &ctx,
        vec![egui::Event::PointerMoved(egui::pos2(300., 200.))],
    );
    activate(&mut app, &ctx, Key::L);
    assert!(!app.modifying);
}
#[test]
fn tooltip_tracks_a_changed_test_binding() {
    let mut e = MAP
        .iter()
        .find(|e| e.action == Action::Modify(ModifyKind::PushPull))
        .unwrap()
        .clone();
    assert_eq!(shortcuts::tooltip(&e), "Push/Pull (P)");
    static TEST: &[Binding] = &[Binding {
        key: Key::V,
        ctrl: false,
        shift: false,
        label: "V",
    }];
    e.bindings = TEST;
    assert_eq!(shortcuts::tooltip(&e), "Push/Pull (V)");
}
#[test]
fn documentation_matches_map() {
    let mut table = String::from(
        "| Category | Action | Binding | During operation |\n| --- | --- | --- | --- |\n",
    );
    for e in MAP {
        let bindings = if let Some(reason) = e.unbound {
            format!("Unbound: {reason}")
        } else {
            e.bindings
                .iter()
                .map(|b| b.label)
                .collect::<Vec<_>>()
                .join(" / ")
        };
        table.push_str(&format!(
            "| {} | {} | {} | {} |\n",
            e.category,
            e.label,
            bindings,
            if e.during_operation {
                "Yes"
            } else {
                "Idle only"
            }
        ));
    }
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    for file in ["docs/SHORTCUTS.md", "README.md"] {
        let path = root.join(file);
        let text = std::fs::read_to_string(&path).unwrap();
        let (before, tail) = text.split_once("<!-- shortcuts:start -->").unwrap();
        let (_, after) = tail.split_once("<!-- shortcuts:end -->").unwrap();
        let expected =
            format!("{before}<!-- shortcuts:start -->\n{table}<!-- shortcuts:end -->{after}");
        if std::env::var_os("UPDATE_SHORTCUT_DOCS").is_some() {
            std::fs::write(path, &expected).unwrap();
        } else {
            assert_eq!(text,expected,"{file}: run UPDATE_SHORTCUT_DOCS=1 cargo test -p cindral-app documentation_matches_map");
        }
    }
}
#[test]
fn selection_group_delete_and_redo_aliases() {
    let (mut app, ctx) = setup();
    app.execute(Command::AddEdge {
        start: DVec3::ZERO,
        end: DVec3::X,
    })
    .unwrap();
    frame(&mut app, &ctx, vec![event(Key::A, true, false, false)]);
    assert_eq!(app.selection.edges.len(), 1);
    activate(&mut app, &ctx, Key::Delete);
    assert!(app.model().edges().is_empty());
    frame(&mut app, &ctx, vec![event(Key::Z, true, false, false)]);
    assert_eq!(app.model().edges().len(), 1);
    assert!(app.shortcuts.navigation.is_none());
    for (key, shift) in [(Key::Y, false), (Key::Z, true)] {
        frame(&mut app, &ctx, vec![event(key, true, shift, false)]);
        assert!(app.model().edges().is_empty());
        frame(&mut app, &ctx, vec![event(Key::Z, true, false, false)]);
        assert_eq!(app.model().edges().len(), 1);
    }
    frame(&mut app, &ctx, vec![event(Key::A, true, false, false)]);
    activate(&mut app, &ctx, Key::G);
    assert!(app.model().edges().is_empty());
    assert!(app.organization.selected.is_some());
    let context = app.history.context().to_vec();
    activate(&mut app, &ctx, Key::R);
    assert_eq!(app.history.context(), context);
}
#[test]
fn focused_file_actions_do_not_save() {
    let (mut app, ctx) = setup();
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("focused.cdraft");
    app.files.path = Some(path.clone());
    for text in [false, true] {
        app.measurements.text = if text { "3".into() } else { String::new() };
        if !text {
            ctx.memory_mut(|m| m.request_focus(egui::Id::new("scene rename")));
        }
        let _ = ctx.run(
            egui::RawInput {
                events: vec![event(Key::S, true, false, false)],
                ..Default::default()
            },
            |ctx| app.file_tick(ctx),
        );
        assert!(!path.exists());
    }
}

#[test]
fn material_quick_pick_shortcut_opens_without_editing_the_document() {
    let (mut app, ctx) = setup();
    let before = app.history.to_data();
    frame(&mut app, &ctx, vec![event(Key::B, false, true, false)]);
    assert!(app.materials.picker_open);
    assert_eq!(app.modify.kind(), ModifyKind::PaintBucket);
    assert!(app.inspector_tab == workspace_ui::InspectorTab::Materials);
    assert_eq!(app.history.to_data(), before);
}
