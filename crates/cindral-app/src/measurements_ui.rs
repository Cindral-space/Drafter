//! Shared status-field input routing and cached readout.
use super::*;
use cindral_tools::measurements::{MeasurementTool, MeasurementsBox, Readout};
impl DrafterApp {
    pub(crate) fn capture_measurement_keys(&mut self, response: &egui::Response) -> bool {
        if !response.hovered() || response.ctx.wants_keyboard_input() {
            return false;
        }
        let accepts = !self.modifying
            || !matches!(
                self.modify.kind(),
                ModifyKind::Select | ModifyKind::Eraser | ModifyKind::PaintBucket
            );
        if !accepts && self.measurements.text.is_empty() {
            return false;
        }
        if response.ctx.input(|i| shortcuts::pressed(i, shortcuts::Action::Cancel)) && !self.measurements.text.is_empty()
        {
            self.measurements.escape();
            self.error = None;
            return true;
        }
        response.ctx.input(|i| {
            if i.modifiers.command || i.modifiers.ctrl || i.modifiers.alt || i.modifiers.mac_cmd {
                return;
            }
            for event in &i.events {
                if let egui::Event::Text(text) = event {
                    if !self.measurements.text.is_empty() || MeasurementsBox::starts_input(text) {
                        self.measurements.type_text(text);
                    }
                }
            }
            if shortcuts::pressed(i, shortcuts::Action::Backspace) {
                self.measurements.backspace();
            }
        });
        if response.ctx.input(|i| shortcuts::pressed(i, shortcuts::Action::Apply)) && !self.measurements.text.is_empty() {
            self.apply_measurement();
            self.measurements.invalid = self.error.is_some();
            return true;
        }
        !self.measurements.text.is_empty()
    }
    pub(crate) fn measurement_readout(&self) -> Readout {
        if let Some(readout) = self.instance_readout() {
            return readout;
        }
        if !self.modifying {
            return self.tool.measurement_readout();
        }
        self.modify.readout()
    }
}

#[derive(Clone)]
pub(crate) enum TapeTarget {
    Context,
    Instance(Box<TapeInstance>),
}
#[derive(Clone)]
pub(crate) struct TapeInstance {
    path: cindral_core::InstancePath,
    before: glam::DMat4,
    owner_world: glam::DMat4,
    context_world: glam::DMat4,
}
pub(crate) struct TapeResize {
    candidate: ModifySession,
    target: TapeTarget,
    matrix: glam::DMat4,
    replacing: bool,
    revision: u64,
}
impl DrafterApp {
    pub(crate) fn request_tape_resize(&mut self) {
        let mut candidate = self.modify.clone();
        match candidate
            .measure_with_settings(&self.measurements.text, self.presentation.measurements)
        {
            Ok(_) => {
                let Some(factor) = candidate.resize_factor() else {
                    return;
                };
                let Some(anchor) = candidate.measurement_anchor() else {
                    return;
                };
                let matrix = glam::DMat4::from_translation(anchor)
                    * glam::DMat4::from_scale(DVec3::splat(factor))
                    * glam::DMat4::from_translation(-anchor);
                let replacing = self.tape_resized && self.modify.editing_completed();
                let target = if replacing {
                    self.tape_target.clone().unwrap_or(TapeTarget::Context)
                } else if let Some(path) = self.organization.selected.clone() {
                    let Some(instance) = self.history.instance(&path) else {
                        self.error = Some("Selected container no longer exists".into());
                        return;
                    };
                    let Some(world) = self
                        .scene
                        .occurrences()
                        .iter()
                        .find(|o| o.path == path)
                        .map(|o| o.world)
                    else {
                        self.error = Some("Selected container has no visible occurrence".into());
                        return;
                    };
                    TapeTarget::Instance(Box::new(TapeInstance {
                        path,
                        before: instance.transform,
                        owner_world: world * instance.transform.inverse(),
                        context_world: self.history.world_transform(),
                    }))
                } else {
                    TapeTarget::Context
                };
                self.tape_resize = Some(TapeResize {
                    candidate,
                    target,
                    matrix,
                    replacing,
                    revision: self.history.revision(),
                });
                self.error = None;
            }
            Err(error) => {
                self.error = Some(error.to_string());
                self.measurements.invalid = true;
            }
        }
    }
    pub(crate) fn confirm_tape_resize(&mut self, accept: bool) {
        let Some(pending) = self.tape_resize.take() else {
            return;
        };
        if !accept {
            return;
        }
        if self.history.revision() != pending.revision {
            self.error = Some("Model changed; measure again before resizing".into());
            return;
        }
        let result = match &pending.target {
            TapeTarget::Context => self
                .history
                .resize_active(pending.matrix, pending.replacing),
            TapeTarget::Instance(instance) => {
                let TapeInstance {
                    path,
                    before,
                    owner_world,
                    context_world,
                } = instance.as_ref();
                let transform = owner_world.inverse()
                    * *context_world
                    * pending.matrix
                    * context_world.inverse()
                    * *owner_world
                    * *before;
                let command = cindral_core::OrganizationCommand::Transform {
                    path: path.clone(),
                    transform,
                };
                if pending.replacing {
                    self.history.replace_organization(command)
                } else {
                    self.history.organization(command)
                }
            }
        };
        match result {
            Ok(true) => {
                self.modify = pending.candidate;
                self.last_tape = self.modify.measurement();
                self.modify.finish();
                self.tape_target = Some(pending.target);
                self.tape_resized = true;
                self.measurements.completed();
                self.error = None;
                self.refresh_mesh();
            }
            result => {
                self.error = Some(
                    result
                        .err()
                        .map(|e| e.to_string())
                        .unwrap_or_else(|| "No resize change".into()),
                );
                self.measurements.invalid = true;
            }
        }
    }
    pub(crate) fn tape_resize_window(&mut self, ctx: &egui::Context) {
        let Some(pending) = &self.tape_resize else {
            return;
        };
        let selected = matches!(pending.target, TapeTarget::Instance(_));
        let mut answer = None;
        egui::Modal::new(egui::Id::new("confirm-tape-resize")).show(ctx,|ui|{
            ui.heading("Resize from measured distance?");
            ui.label(if selected {"Resize the selected container to the entered measurement?"}else{"Resize all geometry and containers in the current editing context to the entered measurement?"});
            ui.label("This creates one undo step. Retyping replaces that same resize.");
            ui.horizontal(|ui|{if ui.button("Resize").clicked(){answer=Some(true);}
                if ui.button("Cancel").clicked(){answer=Some(false);}});
        });
        if ctx.input(|i| shortcuts::pressed(i, shortcuts::Action::Cancel)) {
            answer = Some(false);
        }
        if let Some(answer) = answer {
            self.confirm_tape_resize(answer);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn key(key: Key) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }
    }
    fn send(app: &mut DrafterApp, ctx: &egui::Context, events: Vec<egui::Event>) {
        crate::input_tests::frame(
            app,
            ctx,
            vec![egui::Event::PointerMoved(egui::pos2(300., 200.))],
            egui::Modifiers::NONE,
        );
        crate::input_tests::frame(app, ctx, events, egui::Modifiers::NONE);
    }
    fn square() -> DrafterApp {
        let mut app = DrafterApp::default();
        app.execute(Command::AddPolyline {
            points: vec![
                DVec3::ZERO,
                DVec3::X * 2.,
                (DVec3::X + DVec3::Y) * 2.,
                DVec3::Y * 2.,
            ],
            closed: true,
        })
        .unwrap();
        app
    }
    #[test]
    fn all_drawing_keyboard_reentry_escape_and_history() {
        for kind in [
            ToolKind::Line,
            ToolKind::Rectangle,
            ToolKind::Circle,
            ToolKind::Polygon,
            ToolKind::Arc,
        ] {
            let mut app = DrafterApp::default();
            app.set_tool(kind);
            app.tool.click(DVec3::ZERO).unwrap();
            if kind == ToolKind::Arc {
                app.tool.click(DVec3::X).unwrap();
            }
            let ctx = egui::Context::default();
            let values = if kind == ToolKind::Rectangle {
                ["2m,3m", "4m,5m", "6m,7m"]
            } else if kind == ToolKind::Arc {
                ["90deg", "120deg", "45deg"]
            } else {
                ["2m", "3m", "4m"]
            };
            for text in values {
                send(
                    &mut app,
                    &ctx,
                    vec![egui::Event::Text(text.into()), key(Key::Enter)],
                );
                assert!(app.error.is_none(), "{kind:?}: {:?}", app.error);
                assert_eq!(app.history.undo_depth(), 1);
            }
            let after = app.model().clone();
            send(&mut app, &ctx, vec![egui::Event::Text("5m".into())]);
            send(&mut app, &ctx, vec![key(Key::Escape)]);
            assert!(app.measurements.text.is_empty());
            assert!(app.tool.editing_completed());
            send(&mut app, &ctx, vec![key(Key::Escape)]);
            assert!(!app.tool.editing_completed());
            assert_eq!(app.model(), &after);
            app.history_step(false);
            assert!(app.model().edges().is_empty());
            app.history_step(true);
            assert_eq!(app.model(), &after);
        }
    }
    #[test]
    fn tape_confirmation_cancel_retype_and_undo() {
        let mut app = square();
        app.choose_modify(ModifyKind::TapeMeasure);
        app.modify
            .begin(
                app.history.model(),
                &Selection::default(),
                None,
                DVec3::ZERO,
                DVec3::Z,
            )
            .unwrap();
        app.modify.update(DVec3::X * 2.).unwrap();
        app.modify.finish();
        let before = app.model().clone();
        let depth = app.history.undo_depth();
        app.measurements.text = "4m".into();
        app.apply_measurement();
        assert!(app.tape_resize.is_some());
        assert_eq!(app.model(), &before);
        app.confirm_tape_resize(false);
        assert_eq!(app.model(), &before);
        for (text, size) in [("4m", 4.), ("6m", 6.), ("8m", 8.)] {
            app.measurements.text = text.into();
            app.apply_measurement();
            app.confirm_tape_resize(true);
            assert!(app.error.is_none(), "{:?}", app.error);
            assert_eq!(app.model().bounds().unwrap().1.x, size);
            assert_eq!(app.history.undo_depth(), depth + 1);
        }
        let after = app.model().clone();
        app.history_step(false);
        assert_eq!(app.model(), &before);
        app.history_step(true);
        assert_eq!(app.model(), &after);
    }
    #[test]
    fn viewport_does_not_capture_another_text_fields_input() {
        let mut app = DrafterApp::default();
        app.set_tool(ToolKind::Line);
        app.tool.click(DVec3::ZERO).unwrap();
        let ctx = egui::Context::default();
        let id = egui::Id::new("outliner-rename-test");
        let mut name = String::new();
        for events in [
            vec![egui::Event::PointerMoved(egui::pos2(300., 200.))],
            vec![
                egui::Event::PointerMoved(egui::pos2(300., 200.)),
                egui::Event::Text("12m".into()),
            ],
        ] {
            let _ = ctx.run(
                egui::RawInput {
                    events,
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(800., 600.),
                    )),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        let field = ui.add(egui::TextEdit::singleline(&mut name).id(id));
                        field.request_focus();
                        let (_, response) = ui.allocate_exact_size(
                            egui::vec2(600., 400.),
                            egui::Sense::click_and_drag(),
                        );
                        app.viewport_input(&response);
                    });
                },
            );
        }
        assert_eq!(name, "12m");
        assert!(app.measurements.text.is_empty());
        assert_eq!(app.history.undo_depth(), 0);
    }
}

#[cfg(test)]
mod context_escape_regression {
    use super::*;
    #[test]
    fn completed_operation_escape_keeps_edit_context() {
        let mut app = DrafterApp::default();
        app.execute(Command::AddEdge {
            start: DVec3::ZERO,
            end: DVec3::X,
        })
        .unwrap();
        app.selection
            .vertices
            .extend(app.model().vertices().keys().copied().collect::<Vec<_>>());
        let path = app
            .create_container(cindral_core::DefinitionKind::Group)
            .unwrap();
        app.enter_context(path.clone()).unwrap();
        app.set_tool(ToolKind::Circle);
        app.tool.click(DVec3::ZERO).unwrap();
        app.measurements.text = "2m".into();
        app.apply_measurement();
        assert!(app.tool.editing_completed());
        let ctx = egui::Context::default();
        for events in [
            vec![egui::Event::PointerMoved(egui::pos2(300., 200.))],
            vec![egui::Event::Key {
                key: Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
        ] {
            crate::input_tests::frame(&mut app, &ctx, events, egui::Modifiers::NONE);
        }
        assert_eq!(app.history.context(), &path);
        assert!(!app.tool.editing_completed());
    }
}
