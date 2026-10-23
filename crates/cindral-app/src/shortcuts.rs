//! Logical-key action registry. No physical scancodes or desktop-reserved bindings.
use super::*;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Action {
    Draw(ToolKind),
    Modify(ModifyKind),
    Orbit,
    Pan,
    Zoom,
    Fit,
    View(StandardView),
    Projection,
    Undo,
    Redo,
    SelectAll,
    Delete,
    Group,
    Save,
    Open,
    SaveAs,
    Cancel,
    Help,
    AxisX,
    AxisY,
    AxisZ,
    AxisClear,
    Apply,
    Backspace,
    Section,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Context {
    Canvas,
    Text,
}
#[derive(Clone, Copy, Debug)]
pub(crate) struct Binding {
    pub key: Key,
    pub ctrl: bool,
    pub shift: bool,
    pub label: &'static str,
}
#[derive(Clone, Debug)]
pub(crate) struct Entry {
    pub action: Action,
    pub label: &'static str,
    pub category: &'static str,
    pub bindings: &'static [Binding],
    pub during_operation: bool,
    pub context: Context,
    pub unbound: Option<&'static str>,
}
macro_rules! entry { ($a:expr,$l:literal,$c:literal,$active:expr,[$(($k:ident,$ctrl:expr,$shift:expr,$hint:literal)),*]) => { Entry { action:$a,label:$l,category:$c,bindings:&[$(Binding {key:Key::$k,ctrl:$ctrl,shift:$shift,label:$hint}),*],during_operation:$active,context:Context::Canvas,unbound:None } }; }
use Action::*;
pub(crate) static MAP: &[Entry] = &[
    entry!(
        Modify(ModifyKind::Select),
        "Select",
        "Tools",
        true,
        [(Space, false, false, "Space")]
    ),
    entry!(
        Draw(ToolKind::Line),
        "Line",
        "Tools",
        true,
        [(L, false, false, "L")]
    ),
    entry!(
        Draw(ToolKind::Rectangle),
        "Rectangle",
        "Tools",
        true,
        [(R, false, false, "R")]
    ),
    entry!(
        Draw(ToolKind::Circle),
        "Circle",
        "Tools",
        true,
        [(C, false, false, "C")]
    ),
    entry!(
        Draw(ToolKind::Arc),
        "Arc",
        "Tools",
        true,
        [(A, false, false, "A")]
    ),
    entry!(
        Draw(ToolKind::Polygon),
        "Polygon",
        "Tools",
        true,
        [(N, false, false, "N")]
    ),
    entry!(
        Modify(ModifyKind::PushPull),
        "Push/Pull",
        "Tools",
        true,
        [(P, false, false, "P")]
    ),
    entry!(
        Modify(ModifyKind::Move),
        "Move",
        "Tools",
        true,
        [(M, false, false, "M")]
    ),
    entry!(
        Modify(ModifyKind::Rotate),
        "Rotate",
        "Tools",
        true,
        [(Q, false, false, "Q")]
    ),
    entry!(
        Modify(ModifyKind::Scale),
        "Scale",
        "Tools",
        true,
        [(S, false, false, "S")]
    ),
    entry!(
        Modify(ModifyKind::Offset),
        "Offset",
        "Tools",
        true,
        [(F, false, false, "F")]
    ),
    entry!(
        Modify(ModifyKind::Eraser),
        "Eraser",
        "Tools",
        true,
        [(E, false, false, "E")]
    ),
    entry!(
        Modify(ModifyKind::TapeMeasure),
        "Tape Measure",
        "Tools",
        true,
        [(T, false, false, "T")]
    ),
    entry!(
        Modify(ModifyKind::PaintBucket),
        "Paint Bucket",
        "Tools",
        true,
        [(B, false, false, "B")]
    ),
    entry!(Orbit, "Orbit", "Navigation", true, [(O, false, false, "O")]),
    entry!(Pan, "Pan", "Navigation", true, [(H, false, false, "H")]),
    entry!(Zoom, "Zoom", "Navigation", true, [(Z, false, false, "Z")]),
    entry!(
        Fit,
        "Fit model",
        "Navigation",
        true,
        [(Z, false, true, "Shift+Z")]
    ),
    entry!(
        View(StandardView::Iso),
        "Iso",
        "Navigation",
        false,
        [(Num0, false, false, "0")]
    ),
    entry!(
        View(StandardView::Front),
        "Front",
        "Navigation",
        false,
        [(Num1, false, false, "1")]
    ),
    entry!(
        View(StandardView::Right),
        "Right",
        "Navigation",
        false,
        [(Num3, false, false, "3")]
    ),
    entry!(
        View(StandardView::Top),
        "Top",
        "Navigation",
        false,
        [(Num7, false, false, "7")]
    ),
    entry!(
        Projection,
        "Toggle projection",
        "Navigation",
        false,
        [(Num5, false, false, "5")]
    ),
    entry!(Undo, "Undo", "Actions", true, [(Z, true, false, "Ctrl+Z")]),
    entry!(
        Redo,
        "Redo",
        "Actions",
        true,
        [(Y, true, false, "Ctrl+Y"), (Z, true, true, "Ctrl+Shift+Z")]
    ),
    entry!(
        SelectAll,
        "Select all",
        "Actions",
        false,
        [(A, true, false, "Ctrl+A")]
    ),
    entry!(
        Delete,
        "Erase selection",
        "Actions",
        false,
        [(Delete, false, false, "Delete")]
    ),
    entry!(
        Group,
        "Make Group",
        "Actions",
        false,
        [(G, false, false, "G")]
    ),
    entry!(Save, "Save", "Files", true, [(S, true, false, "Ctrl+S")]),
    entry!(Open, "Open", "Files", true, [(O, true, false, "Ctrl+O")]),
    entry!(
        SaveAs,
        "Save As…",
        "Files",
        true,
        [(S, true, true, "Ctrl+Shift+S")]
    ),
    entry!(
        Cancel,
        "Cancel",
        "Actions",
        true,
        [(Escape, false, false, "Escape")]
    ),
    entry!(
        Help,
        "Shortcut help",
        "Help",
        true,
        [(F1, false, false, "F1")]
    ),
    entry!(
        AxisX,
        "Lock X",
        "Constraints",
        true,
        [(ArrowRight, false, false, "ArrowRight")]
    ),
    entry!(
        AxisY,
        "Lock Y",
        "Constraints",
        true,
        [(ArrowLeft, false, false, "ArrowLeft")]
    ),
    entry!(
        AxisZ,
        "Lock Z",
        "Constraints",
        true,
        [(ArrowUp, false, false, "ArrowUp")]
    ),
    entry!(
        AxisClear,
        "Clear axis lock",
        "Constraints",
        true,
        [(ArrowDown, false, false, "ArrowDown")]
    ),
    Entry {
        action: Modify(ModifyKind::Copy),
        label: "Copy",
        category: "Tools",
        bindings: &[],
        during_operation: true,
        context: Context::Canvas,
        unbound: Some("Use the existing Copy button; M activates Move"),
    },
    Entry {
        action: Modify(ModifyKind::FollowMe),
        label: "Follow Me",
        category: "Tools",
        bindings: &[],
        during_operation: true,
        context: Context::Canvas,
        unbound: Some("No conventional default"),
    },
    Entry {
        action: Section,
        label: "Section Plane",
        category: "Tools",
        bindings: &[],
        during_operation: false,
        context: Context::Canvas,
        unbound: Some("Use the existing section panel; no conventional default"),
    },
    Entry {
        context: Context::Text,
        ..entry!(
            Apply,
            "Apply measurement",
            "Measurements",
            true,
            [(Enter, false, false, "Enter")]
        )
    },
    Entry {
        context: Context::Text,
        ..entry!(
            Backspace,
            "Edit measurement",
            "Measurements",
            true,
            [(Backspace, false, false, "Backspace")]
        )
    },
];
pub(crate) fn hint(action: Action) -> &'static str {
    MAP.iter()
        .find(|e| e.action == action)
        .and_then(|e| e.bindings.first())
        .map_or("", |b| b.label)
}
pub(crate) fn tooltip(entry: &Entry) -> String {
    match entry.bindings.first() {
        Some(b) => format!("{} ({})", entry.label, b.label),
        None => entry.label.into(),
    }
}
pub(crate) fn pressed(i: &egui::InputState, action: Action) -> bool {
    MAP.iter().filter(|e|e.action==action).any(|e|e.bindings.iter().any(|b|i.events.iter().any(|event| matches!(event,egui::Event::Key{key,pressed:true,repeat:false,modifiers,..} if *key==b.key && !modifiers.alt && (!modifiers.mac_cmd || b.ctrl) && (modifiers.ctrl || modifiers.command)==b.ctrl && modifiers.shift==b.shift))))
}
#[derive(Default)]
pub(crate) struct Ui {
    pub open: bool,
    pub search: String,
    pub navigation: Option<Action>,
    pub notice: Option<String>,
}
pub(crate) const CHANGED: &str = "Changed shortcuts: Polygon P → N; Fit F → Shift+Z. P now Push/Pull; F now Offset. F1: all shortcuts.";
impl DrafterApp {
    pub(crate) fn shortcut_overlay(&mut self, ctx: &egui::Context) {
        if !self.shortcuts.open {
            return;
        }
        if ctx.input(|i| pressed(i, Cancel)) {
            consume(ctx, Cancel);
            self.shortcuts.open = false;
            ctx.memory_mut(|m| {
                if let Some(id) = m.focused() {
                    m.surrender_focus(id);
                }
            });
            return;
        }
        let mut open = true;
        egui::Window::new("Keyboard shortcuts")
            .open(&mut open)
            .show(ctx, |ui| {
                ui.label("Search by action or category");
                ui.text_edit_singleline(&mut self.shortcuts.search);
                let query = self.shortcuts.search.to_lowercase();
                egui::ScrollArea::vertical()
                    .max_height(450.)
                    .show(ui, |ui| {
                        for e in MAP.iter().filter(|e| {
                            format!("{} {}", e.category, e.label)
                                .to_lowercase()
                                .contains(&query)
                        }) {
                            ui.label(format!(
                                "{} · {}{}",
                                e.category,
                                tooltip(e),
                                e.unbound
                                    .map_or(String::new(), |r| format!(" — unbound: {r}"))
                            ));
                        }
                    });
            });
        if !open {
            self.shortcuts.open = false;
            ctx.memory_mut(|m| {
                if let Some(id) = m.focused() {
                    m.surrender_focus(id);
                }
            });
        }
    }
    pub(crate) fn route_shortcuts(&mut self, response: &egui::Response) -> bool {
        let ctx = &response.ctx;
        if self.shortcuts.open
            || ctx.wants_keyboard_input()
            || !self.measurements.text.is_empty()
            || !response.hovered()
        {
            return false;
        }
        let active = self.modify.active()
            || self.tool.anchor().is_some()
            || self.organization.drag.is_some();
        let actions = ctx.input(|i| {
            MAP.iter()
                .filter(|e| {
                    e.context == Context::Canvas
                        && (e.during_operation || !active)
                        && pressed(i, e.action)
                })
                .map(|e| e.action)
                .collect::<Vec<_>>()
        });
        for action in actions {
            ctx.request_repaint();
            match action {
                Draw(k) => self.set_tool(k),
                Modify(k) => self.choose_modify(k),
                Orbit | Pan | Zoom => {
                    self.choose_modify(ModifyKind::Select);
                    self.shortcuts.navigation = Some(action);
                }
                Fit => self.extents(response.rect.aspect_ratio()),
                View(v) => self.camera.set_view(v),
                Projection => toggle_projection(&mut self.camera),
                SelectAll => {
                    self.selection = Selection {
                        vertices: self.history.model().vertices().keys().copied().collect(),
                        edges: self.history.model().edges().keys().copied().collect(),
                        faces: self.history.model().faces().keys().copied().collect(),
                    };
                    self.organization.multiple = self
                        .scene
                        .occurrences()
                        .iter()
                        .filter(|o| {
                            o.path.len() == self.history.context().len() + 1
                                && o.path.starts_with(self.history.context())
                        })
                        .map(|o| o.path.clone())
                        .collect();
                    self.organization.selected = self.organization.multiple.first().cloned();
                }
                Delete => {
                    let mut paths = self.organization.multiple.clone();
                    if let Some(path) = &self.organization.selected {
                        paths.insert(path.clone());
                    }
                    let edges = self
                        .history
                        .model()
                        .edges()
                        .iter()
                        .filter(|(id, e)| {
                            self.selection.edges.contains(id)
                                || e.vertices
                                    .iter()
                                    .any(|id| self.selection.vertices.contains(id))
                        })
                        .map(|(id, _)| *id)
                        .collect::<Vec<_>>();
                    let faces = self.selection.faces.iter().copied().collect::<Vec<_>>();
                    if !edges.is_empty() || !faces.is_empty() {
                        if let Err(e) = self.execute(Command::Batch(vec![
                            Command::EraseFaces(faces),
                            Command::EraseEdges(edges),
                        ])) {
                            self.error = Some(e.to_string());
                            return true;
                        }
                    }
                    for path in paths {
                        if let Err(e) =
                            self.organize(cindral_core::OrganizationCommand::Delete(path))
                        {
                            self.error = Some(e.to_string());
                            return true;
                        }
                    }
                    self.selection.clear();
                    self.organization.selected = None;
                    self.organization.multiple.clear();
                }
                Group => {
                    if !self.selection.vertex_ids(self.history.model()).is_empty() {
                        if let Err(e) = self.create_container(cindral_core::DefinitionKind::Group) {
                            self.error = Some(e.to_string());
                        }
                    }
                }
                Undo | Redo => {
                    self.history_step(action == Redo);
                    return true;
                }
                Help => {
                    self.shortcuts.open = true;
                    return true;
                }
                _ => {}
            }
        }
        false
    }
}

/// A separate versioned marker cannot damage theme/document settings on interruption.
fn notice_at(path: &std::path::Path) -> std::io::Result<Option<String>> {
    if std::fs::read(path).ok().as_deref() == Some(b"shortcuts-v1\n") {
        return Ok(None);
    }
    std::fs::create_dir_all(path.parent().expect("marker has a parent"))?;
    cindral_io::atomic_write(path, |file| {
        use std::io::Write;
        file.write_all(b"shortcuts-v1\n")?;
        Ok(())
    })
    .map_err(std::io::Error::other)?;
    Ok(Some(CHANGED.into()))
}
pub(crate) fn first_launch_notice() -> Option<String> {
    let root = std::env::var_os("XDG_STATE_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::env::var_os("HOME")
                .map(std::path::PathBuf::from)
                .unwrap_or_else(std::env::temp_dir)
                .join(".local/state")
        });
    match notice_at(&root.join("cindral-drafter/shortcuts-v1")) {
        Ok(notice) => notice,
        Err(e) => Some(format!("{CHANGED} (Could not store notice marker: {e})")),
    }
}

pub(crate) fn consume(ctx: &egui::Context, action: Action) {
    if let Some(e) = MAP.iter().find(|e| e.action == action) {
        ctx.input_mut(|i| {
            for b in e.bindings {
                let modifiers = egui::Modifiers {
                    ctrl: b.ctrl,
                    command: b.ctrl,
                    shift: b.shift,
                    ..Default::default()
                };
                i.consume_key(modifiers, b.key);
            }
        });
    }
}
#[cfg(test)]
mod persistence_tests {
    use super::*;
    #[test]
    fn notice_is_once_and_separate_from_settings() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("state/shortcuts-v1");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let settings = path.with_file_name("settings.json");
        std::fs::write(&settings, b"existing preferences").unwrap();
        assert_eq!(notice_at(&path).unwrap().as_deref(), Some(CHANGED));
        assert!(notice_at(&path).unwrap().is_none());
        std::fs::write(&path, b"interrupted/invalid").unwrap();
        assert!(notice_at(&path).unwrap().is_some());
        assert_eq!(std::fs::read(&settings).unwrap(), b"existing preferences");
    }
}
