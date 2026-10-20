//! Styles, named scenes and a short interruptible camera transition.
use super::*;
use cindral_core::{OrganizationCommand, SavedScene, SceneCamera, StyleMode};
const TRANSITION_SECONDS: f64 = 0.25;
struct Transition {
    from: SceneCamera,
    to: SceneCamera,
    started: f64,
}
pub(crate) struct PresentationUi {
    selected: Option<usize>,
    name: String,
    animate: bool,
    transition: Option<Transition>,
}
impl Default for PresentationUi {
    fn default() -> Self {
        Self {
            selected: None,
            name: String::new(),
            animate: true,
            transition: None,
        }
    }
}
fn camera_values(camera: &Camera) -> SceneCamera {
    SceneCamera {
        target: camera.target.to_array(),
        distance: camera.distance,
        yaw: camera.yaw,
        pitch: camera.pitch,
        orthographic: camera.projection == Projection::Orthographic,
    }
}
fn apply_camera(camera: &mut Camera, value: &SceneCamera) {
    camera.target = glam::Vec3::from_array(value.target);
    camera.distance = value.distance;
    camera.yaw = value.yaw;
    camera.pitch = value.pitch;
    camera.projection = if value.orthographic {
        Projection::Orthographic
    } else {
        Projection::Perspective
    };
}
fn valid_name(name: &str) -> bool {
    !name.trim().is_empty() && name.len() <= 512 && !name.chars().any(char::is_control)
}
impl DrafterApp {
    pub(crate) fn presentation_changed(&mut self) {
        self.presentation_serial = self.presentation_serial.wrapping_add(1);
        self.files.last_revision = None;
    }
    fn capture_scene(&self, name: String) -> SavedScene {
        self.history.capture_scene(
            name,
            camera_values(&self.camera),
            self.presentation.style.clone(),
        )
    }
    fn add_scene(&mut self, name: String) {
        if !valid_name(&name) || self.presentation.scenes.len() >= 1000 {
            return;
        }
        self.presentation
            .scenes
            .push(self.capture_scene(name.trim().into()));
        self.presentation_ui.selected = Some(self.presentation.scenes.len() - 1);
        self.presentation_ui.name = self.presentation.scenes.last().unwrap().name.clone();
        self.presentation_changed();
    }
    fn update_scene(&mut self, index: usize) {
        let Some(scene) = self.presentation.scenes.get(index) else {
            return;
        };
        let captured = self.capture_scene(scene.name.clone());
        self.presentation.scenes[index] = captured;
        self.presentation_changed();
    }
    fn rename_scene(&mut self, index: usize, name: String) {
        if !valid_name(&name) {
            return;
        }
        if let Some(scene) = self.presentation.scenes.get_mut(index) {
            scene.name = name.trim().into();
            self.presentation_ui.name = scene.name.clone();
            self.presentation_changed();
        }
    }
    fn delete_scene(&mut self, index: usize) {
        if index >= self.presentation.scenes.len() {
            return;
        }
        self.presentation.scenes.remove(index);
        self.presentation_ui.selected = None;
        self.presentation_ui.name.clear();
        self.cancel_scene_transition();
        self.presentation_changed();
    }
    fn restore_scene(&mut self, index: usize, now: f64, animate: bool) {
        let Some(scene) = self.presentation.scenes.get(index).cloned() else {
            return;
        };
        self.cancel_modify();
        self.tool.cancel();
        self.files.fit = false;
        match self
            .history
            .organization(OrganizationCommand::RestoreVisibility {
                tags: scene.tags,
                objects: scene.objects,
            }) {
            Ok(true) => self.refresh_mesh(),
            Ok(false) => {}
            Err(error) => {
                self.error = Some(error.to_string());
                return;
            }
        }
        self.selection.clear();
        self.presentation_ui.transition = if animate {
            Some(Transition {
                from: camera_values(&self.camera),
                to: scene.camera.clone(),
                started: now,
            })
        } else {
            apply_camera(&mut self.camera, &scene.camera);
            None
        };
        self.presentation.style = scene.style;
        self.presentation_ui.selected = Some(index);
        self.presentation_ui.name = scene.name;
        self.presentation_changed();
    }
    pub(crate) fn cancel_scene_transition(&mut self) {
        self.presentation_ui.transition = None;
    }
    fn skip_scene_transition(&mut self) {
        if let Some(transition) = self.presentation_ui.transition.take() {
            apply_camera(&mut self.camera, &transition.to);
        }
    }
    fn advance_transition(&mut self, now: f64) {
        let Some(transition) = &self.presentation_ui.transition else {
            return;
        };
        let t = ((now - transition.started) / TRANSITION_SECONDS).clamp(0., 1.) as f32;
        if t >= 1. {
            self.skip_scene_transition();
            return;
        }
        let t = t * t * (3. - 2. * t);
        let from = &transition.from;
        let to = &transition.to;
        let yaw_delta = (to.yaw - from.yaw + std::f32::consts::PI)
            .rem_euclid(std::f32::consts::TAU)
            - std::f32::consts::PI;
        let value = SceneCamera {
            target: glam::Vec3::from_array(from.target)
                .lerp(glam::Vec3::from_array(to.target), t)
                .to_array(),
            distance: (from.distance.ln() * (1. - t) + to.distance.ln() * t).exp(),
            yaw: from.yaw + yaw_delta * t,
            pitch: from.pitch + (to.pitch - from.pitch) * t,
            orthographic: from.orthographic,
        };
        apply_camera(&mut self.camera, &value);
    }
    pub(crate) fn transition_tick(&mut self, ctx: &egui::Context) {
        if ctx.input(|i| i.key_pressed(Key::Escape)) {
            self.skip_scene_transition();
        }
        self.advance_transition(ctx.input(|i| i.time));
        if self.presentation_ui.transition.is_some() {
            ctx.request_repaint();
        }
    }
    pub(crate) fn presentation_panel(&mut self, ctx: &egui::Context) {
        let mut switch = None;
        egui::TopBottomPanel::top("scene_tabs").show(ctx, |ui| {
            egui::ScrollArea::horizontal().show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Scenes");
                    for (index, scene) in self.presentation.scenes.iter().enumerate() {
                        if ui
                            .selectable_label(
                                self.presentation_ui.selected == Some(index),
                                &scene.name,
                            )
                            .clicked()
                        {
                            switch = Some(index);
                        }
                    }
                    if ui
                        .small_button("+")
                        .on_hover_text("Capture camera, visibility and style")
                        .clicked()
                    {
                        self.add_scene(format!("Scene {}", self.presentation.scenes.len() + 1));
                    }
                });
            });
        });
        if let Some(index) = switch {
            self.restore_scene(index, ctx.input(|i| i.time), self.presentation_ui.animate);
            ctx.request_repaint();
        }
        egui::SidePanel::left("styles_scenes")
            .default_width(205.)
            .width_range(180. ..=320.)
            .show(ctx, |ui| {
                ui.heading("Styles & Scenes");
                let before = self.presentation.style.clone();
                let style = &mut self.presentation.style;
                egui::ComboBox::from_id_salt("style_mode")
                    .selected_text(style_label(style.mode))
                    .show_ui(ui, |ui| {
                        for mode in [
                            StyleMode::Shaded,
                            StyleMode::Textured,
                            StyleMode::Wireframe,
                            StyleMode::HiddenLine,
                            StyleMode::XRay,
                        ] {
                            ui.selectable_value(&mut style.mode, mode, style_label(mode));
                        }
                    });
                ui.checkbox(&mut style.profiles, "Profiles");
                ui.checkbox(&mut style.soft_edges, "Show soft edges");
                ui.checkbox(&mut style.smooth_edges, "Show smooth edges");
                ui.horizontal(|ui| {
                    ui.label("Background");
                    ui.color_edit_button_srgb(&mut style.background);
                });
                optional_color(ui, "Sky", &mut style.sky, [90, 130, 185]);
                optional_color(ui, "Ground", &mut style.ground, [80, 90, 65]);
                if self.presentation.style != before {
                    self.presentation_changed();
                }
                ui.separator();
                ui.checkbox(&mut self.presentation_ui.animate, "Animate scene camera");
                if self.presentation_ui.transition.is_some()
                    && ui.button("Skip transition (Esc)").clicked()
                {
                    self.skip_scene_transition();
                }
                ui.add(
                    egui::TextEdit::singleline(&mut self.presentation_ui.name)
                        .hint_text("Scene name"),
                );
                let name = self.presentation_ui.name.clone();
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(
                            valid_name(&name) && self.presentation.scenes.len() < 1000,
                            egui::Button::new("Add"),
                        )
                        .clicked()
                    {
                        self.add_scene(name.clone());
                    }
                    if let Some(index) = self.presentation_ui.selected {
                        if ui
                            .button("Update")
                            .on_hover_text(
                                "Replace the scene with the current camera, visibility and style",
                            )
                            .clicked()
                        {
                            self.update_scene(index);
                        }
                    }
                });
                if let Some(index) = self.presentation_ui.selected {
                    ui.horizontal(|ui| {
                        if ui
                            .add_enabled(valid_name(&name), egui::Button::new("Rename"))
                            .clicked()
                        {
                            self.rename_scene(index, name.clone());
                        }
                        if ui.button("Delete").clicked() {
                            self.delete_scene(index);
                        }
                    });
                }
            });
    }
}
fn style_label(mode: StyleMode) -> &'static str {
    match mode {
        StyleMode::Shaded => "Shaded",
        StyleMode::Textured => "Shaded with textures",
        StyleMode::Wireframe => "Wireframe",
        StyleMode::HiddenLine => "Hidden line",
        StyleMode::XRay => "X-ray",
    }
}
fn optional_color(ui: &mut egui::Ui, label: &str, color: &mut Option<[u8; 3]>, default: [u8; 3]) {
    ui.horizontal(|ui| {
        let mut enabled = color.is_some();
        if ui.checkbox(&mut enabled, label).changed() {
            *color = enabled.then_some(default);
        }
        if let Some(color) = color {
            ui.color_edit_button_srgb(color);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use cindral_core::{DefinitionId, DefinitionKind};
    fn fixture() -> DrafterApp {
        let mut app = DrafterApp::default();
        app.execute(Command::AddEdge {
            start: DVec3::ZERO,
            end: DVec3::X,
        })
        .unwrap();
        let vertices = app.history.model().vertices().keys().copied().collect();
        app.organize(OrganizationCommand::Create {
            kind: DefinitionKind::Group,
            name: "Part".into(),
            vertices,
        })
        .unwrap();
        app.organize(OrganizationCommand::CreateTag("Layer".into()))
            .unwrap();
        app
    }
    #[test]
    fn scripted_scene_capture_switch_restore_and_native_roundtrip() {
        let mut app = fixture();
        let id = *app
            .history
            .instances(DefinitionId(0))
            .unwrap()
            .keys()
            .next()
            .unwrap();
        let tag = *app.history.tags().keys().next().unwrap();
        app.add_scene("Visible".into());
        let first = app.presentation.scenes[0].clone();
        app.camera.target = glam::vec3(3., 4., 5.);
        app.camera.distance = 25.;
        app.camera.projection = Projection::Orthographic;
        app.set_view_style(cindral_core::ViewStyle {
            mode: StyleMode::Wireframe,
            profiles: true,
            background: [200, 220, 240],
            ..Default::default()
        });
        app.organize(OrganizationCommand::Hide {
            path: vec![id],
            hidden: true,
        })
        .unwrap();
        app.organize(OrganizationCommand::TagVisibility {
            id: tag,
            visible: false,
        })
        .unwrap();
        app.add_scene("Hidden".into());
        let second = app.presentation.scenes[1].clone();
        for (index, expected) in [(0, first), (1, second)] {
            app.restore_scene(index, 1., false);
            assert_eq!(camera_values(&app.camera), expected.camera);
            assert_eq!(app.presentation.style, expected.style);
            assert_eq!(app.history.tags()[&tag].visible, expected.tags[&tag]);
            assert_eq!(
                app.history.instances(DefinitionId(0)).unwrap()[&id].hidden,
                expected.objects[&(DefinitionId(0), id)]
            );
            assert!(app.history.document_changes().geometry.is_empty());
        }
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("scenes.cdraft");
        app.save_path(&path).unwrap();
        let expected = app.file_values();
        let mut loaded = DrafterApp::default();
        loaded.open_path(&path).unwrap();
        assert_eq!(loaded.file_values(), expected);
        loaded.restore_scene(0, 2., false);
        assert_eq!(
            camera_values(&loaded.camera),
            expected.presentation.scenes[0].camera
        );
    }
    #[test]
    fn scene_update_rename_delete_and_transition_skip() {
        let mut app = fixture();
        app.add_scene("Start".into());
        app.camera.yaw = -3.1;
        app.add_scene("End".into());
        app.camera.yaw = 3.1;
        app.restore_scene(1, 10., true);
        app.advance_transition(10.125);
        assert!(
            (app.camera.yaw.abs() - std::f32::consts::PI).abs() < 0.1,
            "yaw uses shortest arc"
        );
        app.skip_scene_transition();
        assert_eq!(app.camera.yaw, -3.1);
        app.camera.distance = 45.;
        app.update_scene(1);
        app.rename_scene(1, "Updated".into());
        assert_eq!(app.presentation.scenes[1].camera.distance, 45.);
        assert_eq!(app.presentation.scenes[1].name, "Updated");
        app.restore_scene(0, 20., true);
        app.advance_transition(21.);
        assert_eq!(
            camera_values(&app.camera),
            app.presentation.scenes[0].camera
        );
        app.restore_scene(1, 30., true);
        app.cancel_scene_transition();
        let before = camera_values(&app.camera);
        app.advance_transition(31.);
        assert_eq!(camera_values(&app.camera), before);
        app.delete_scene(0);
        assert_eq!(app.presentation.scenes.len(), 1);
        assert!(app.presentation_ui.selected.is_none());
    }
}
