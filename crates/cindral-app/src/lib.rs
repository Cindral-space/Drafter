//! Native editor, pure drawing/inference routing, and cached GPU geometry.
use cindral_core::{Command, DVec3, DocumentHistory as History};
use cindral_inference::{Axis, DrawingPlane, Engine, Query, Ray, Snap, SnapEntity, SnapKind, View};
use cindral_render::{
    camera::{Camera, Projection, StandardView},
    grid::{GridCallback, GridRenderer},
    scene::{SceneCallback, SceneRenderer, SceneUpdate},
};
use cindral_tools::{ModifyKind, ModifySession, Selection, ToolKind, ToolSession};
mod files_ui;
mod modify_ui;
mod organization_ui;
mod presentation_ui;
mod scene_cache;
mod section_ui;
mod shadow_ui;
mod workspace_ui;
use eframe::egui::{self, Color32, Key, PointerButton};
use glam::Vec2;
use std::sync::Arc;

/// Editor state; geometry changes are committed only through core commands.
pub struct DrafterApp {
    files: files_ui::FileState,
    show_tray: bool,
    presentation: cindral_core::Presentation,
    presentation_ui: presentation_ui::PresentationUi,
    presentation_serial: u64,
    section_ui: section_ui::SectionUi,
    camera: Camera,
    modifying: bool,
    modify: ModifySession,
    selection: Selection,
    selection_start: Option<glam::DVec2>,
    modify_press: Option<glam::DVec2>,
    drag_axis: DVec3,
    paint_color: [u8; 4],
    last_tape: Option<f64>,
    guide_definition: cindral_core::DefinitionId,
    guide_cache: std::collections::BTreeMap<cindral_core::DefinitionId, Vec<cindral_tools::Guide>>,
    history: History,
    engine: Engine,
    tool: ToolSession,
    axis_lock: Option<Axis>,
    snap: Option<Snap>,
    measurement: String,
    error: Option<String>,
    scene: scene_cache::SceneCache,
    organization: organization_ui::OrganizationUi,
    pending: Option<Arc<SceneUpdate>>,
    work_plane: DrawingPlane,
}
impl Default for DrafterApp {
    fn default() -> Self {
        Self {
            files: files_ui::FileState::default(),
            show_tray: true,
            presentation: Default::default(),
            presentation_ui: Default::default(),
            presentation_serial: 0,
            section_ui: Default::default(),
            camera: Camera::default(),
            modifying: false,
            modify: ModifySession::new(ModifyKind::Select),
            selection: Selection::default(),
            selection_start: None,
            modify_press: None,
            drag_axis: DVec3::Z,
            paint_color: [210, 130, 75, 255],
            last_tape: None,
            guide_definition: cindral_core::DefinitionId(0),
            guide_cache: Default::default(),
            history: History::new(),
            engine: Engine::default(),
            tool: ToolSession::new(ToolKind::Line),
            axis_lock: None,
            snap: None,
            measurement: String::new(),
            error: None,
            scene: scene_cache::SceneCache::default(),
            organization: organization_ui::OrganizationUi::default(),
            pending: None,
            work_plane: DrawingPlane::default(),
        }
    }
}
impl DrafterApp {
    /// Initialize original editor styling and the wgpu callback resources.
    pub fn new(
        cc: &eframe::CreationContext<'_>,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let render = cc
            .wgpu_render_state
            .as_ref()
            .ok_or("Cindral Drafter requires wgpu")?;
        let mut renderer = render.renderer.write();
        renderer.callback_resources.insert(GridRenderer::new(
            &render.device,
            render.target_format,
            4,
        ));
        renderer.callback_resources.insert(SceneRenderer::new(
            &render.device,
            render.target_format,
            4,
        ));
        workspace_ui::configure_style(&cc.egui_ctx);
        Ok(Self {
            modifying: true,
            ..Self::default()
        })
    }
    fn extents(&mut self, aspect: f32) {
        let (min, max) = self
            .selected_world_bounds(&[])
            .or_else(|| self.history.model().bounds())
            .unwrap_or((DVec3::splat(-10.), DVec3::splat(10.)));
        self.camera
            .zoom_extents(min.as_vec3(), max.as_vec3(), aspect);
    }
    fn set_tool(&mut self, kind: ToolKind) {
        self.cancel_section();
        if let Ok(true) = self.history.cancel_preview() {
            self.refresh_mesh();
        }
        self.organization.drag = None;
        self.modify.cancel();
        self.modifying = false;
        self.tool.set_kind(kind);
        if let Err(error) = self.tool.set_plane(self.work_plane) {
            self.error = Some(error.to_string());
        }
        self.axis_lock = None;
        self.measurement.clear();
        self.snap = None;
    }
    fn refresh_mesh(&mut self) {
        let active = self.history.active_definition();
        if self.guide_definition != active {
            let guides = self.guide_cache.remove(&active).unwrap_or_default();
            let previous = self.modify.replace_guides(guides);
            self.guide_cache.insert(self.guide_definition, previous);
            self.guide_definition = active;
            self.last_tape = None;
        }
        match self.scene.sync(&self.history) {
            Ok(update) => scene_cache::merge(&mut self.pending, update),
            Err(error) => self.error = Some(error.to_string()),
        }
    }
    /// Apply a core command and prepare the affected CPU mesh ranges.
    pub fn execute(&mut self, command: Command) -> Result<bool, cindral_core::KernelError> {
        let changed = self.history.execute(command)?;
        if changed {
            self.error = None;
            self.refresh_mesh();
        }
        Ok(changed)
    }
    /// Replace provisional geometry and upload only dirty mesh ranges; no history entry is added.
    pub fn preview(&mut self, command: Command) -> Result<bool, cindral_core::KernelError> {
        let result = self.history.preview(command);
        self.refresh_mesh();
        result
    }
    /// Commit the current provisional edit as one undo step.
    pub fn commit_preview(&mut self) -> Result<bool, cindral_core::KernelError> {
        self.history.commit_preview()
    }
    /// Restore provisional geometry and prepare the reverted dirty ranges.
    pub fn cancel_preview(&mut self) -> Result<bool, cindral_core::KernelError> {
        let changed = self.history.cancel_preview()?;
        if changed {
            self.refresh_mesh();
        }
        Ok(changed)
    }
    /// Select the topologically connected component for a given hit.
    pub fn select_connected(&mut self, hit: cindral_tools::Hit) {
        self.selection.connected(self.history.model(), hit, false);
        self.choose_modify(ModifyKind::Select);
    }
    /// Current rendering-independent document state.
    pub fn model(&self) -> &cindral_core::Model {
        self.history.model()
    }
    fn commit(&mut self, command: Command) -> bool {
        match self.execute(command) {
            Ok(_) => true,
            Err(error) => {
                self.error = Some(error.to_string());
                false
            }
        }
    }
    fn history_step(&mut self, redo: bool) {
        if let Ok(true) = self.history.cancel_preview() {
            self.refresh_mesh();
        }
        self.modify.cancel();
        self.organization.drag = None;
        self.selection.clear();
        self.selection_start = None;
        let result = if redo {
            self.history.redo()
        } else {
            self.history.undo()
        };
        match result {
            Ok(true) => {
                self.tool.cancel();
                self.snap = None;
                self.refresh_mesh();
            }
            Ok(false) => {}
            Err(error) => self.error = Some(error.to_string()),
        }
    }
    fn apply_measurement(&mut self) {
        if self.instance_measurement() {
            return;
        }
        if self.modifying {
            self.modify_measurement();
            return;
        }
        let before = self.tool.clone();
        match self.tool.measure(&self.measurement) {
            Ok(Some(command)) => {
                if self.commit(command) {
                    self.measurement.clear();
                } else {
                    self.tool = before;
                }
            }
            Ok(None) => {}
            Err(error) => self.error = Some(error.to_string()),
        }
    }
    /// Current viewport appearance, independent of document geometry.
    pub fn view_style(&self) -> &cindral_core::ViewStyle {
        &self.presentation.style
    }

    /// Change viewport appearance without rebuilding geometry or instance buffers.
    pub fn set_view_style(&mut self, style: cindral_core::ViewStyle) {
        if self.presentation.style != style {
            self.presentation.style = style;
            self.presentation_changed();
        }
    }

    fn viewport_ink(&self) -> Color32 {
        let [r, g, b] = self.presentation.style.background;
        if u32::from(r) * 2126 + u32::from(g) * 7152 + u32::from(b) * 722 > 128 * 10000 {
            Color32::from_rgb(37, 48, 58)
        } else {
            Color32::WHITE
        }
    }
    fn paint_environment(&self, ui: &egui::Ui, rect: egui::Rect) {
        let style = &self.presentation.style;
        // With Z up and no camera roll, the infinite ground horizon is horizontal.
        let horizon = if self.camera.projection == Projection::Orthographic {
            if self.camera.pitch >= 0. {
                rect.top()
            } else {
                rect.bottom()
            }
        } else {
            rect.center().y
                - (self.camera.pitch.sin() / self.camera.pitch.cos().max(1e-6))
                    / (cindral_render::camera::FOV_Y * 0.5).tan()
                    * rect.height()
                    * 0.5
        }
        .clamp(rect.top(), rect.bottom());
        if let Some(rgb) = style.sky {
            ui.painter().rect_filled(
                egui::Rect::from_min_max(rect.min, egui::pos2(rect.right(), horizon)),
                0.,
                Color32::from_rgb(rgb[0], rgb[1], rgb[2]),
            );
        }
        if let Some(rgb) = style.ground {
            ui.painter().rect_filled(
                egui::Rect::from_min_max(egui::pos2(rect.left(), horizon), rect.max),
                0.,
                Color32::from_rgb(rgb[0], rgb[1], rgb[2]),
            );
        }
    }

    fn viewport_input(&mut self, response: &egui::Response) {
        let rect = response.rect;
        let aspect = rect.width() / rect.height().max(1.0);
        if response.dragged_by(PointerButton::Middle) {
            let (delta, shift) = response
                .ctx
                .input(|i| (i.pointer.delta(), i.modifiers.shift));
            if shift {
                self.camera.pan(Vec2::new(delta.x, delta.y), rect.height());
            } else {
                self.camera.orbit(Vec2::new(delta.x, delta.y));
            }
        }
        if self.section_input(response) {
            return;
        }
        if self.organization_pointer(response) {
            return;
        }
        if !response.hovered() {
            if self.modifying && (self.modify.active() || self.selection_start.is_some()) {
                self.modify_viewport_input(response);
            }
            return;
        }
        let keyboard_free = !response.ctx.wants_keyboard_input();
        response.ctx.input(|i| {
            if let Some(position) = i.pointer.hover_pos() {
                let cursor = Vec2::new(
                    2. * (position.x - rect.left()) / rect.width() - 1.,
                    1. - 2. * (position.y - rect.top()) / rect.height(),
                );
                self.camera.zoom(i.smooth_scroll_delta.y, cursor, aspect);
            }
            if keyboard_free
                && self.measurement.is_empty()
                && !i.modifiers.command
                && !i.modifiers.alt
            {
                for (key, kind) in [
                    (Key::L, ToolKind::Line),
                    (Key::R, ToolKind::Rectangle),
                    (Key::C, ToolKind::Circle),
                    (Key::A, ToolKind::Arc),
                    (Key::P, ToolKind::Polygon),
                ] {
                    if i.key_pressed(key) {
                        self.set_tool(kind);
                    }
                }
                if i.key_pressed(Key::Space) {
                    self.choose_modify(ModifyKind::Select);
                }
                if i.key_pressed(Key::F) {
                    self.extents(aspect);
                }
                if !self.modify.active() && self.tool.anchor().is_none() {
                    for (key, view) in [
                        (Key::Num0, StandardView::Iso),
                        (Key::Num1, StandardView::Front),
                        (Key::Num3, StandardView::Right),
                        (Key::Num7, StandardView::Top),
                    ] {
                        if i.key_pressed(key) {
                            self.camera.set_view(view);
                        }
                    }
                    if i.key_pressed(Key::Num5) {
                        toggle_projection(&mut self.camera);
                    }
                }
            }
        });
        if self.modifying {
            self.modify_viewport_input(response);
            return;
        }
        let keyboard_free = !response.ctx.wants_keyboard_input();
        response.ctx.input(|i| {
            if keyboard_free {
                if i.key_pressed(Key::Escape) {
                    self.tool.cancel();
                    self.measurement.clear();
                    self.axis_lock = None;
                    self.error = None;
                }
                if i.key_pressed(Key::ArrowRight) {
                    self.axis_lock = if self.axis_lock == Some(Axis::X) {
                        None
                    } else {
                        Some(Axis::X)
                    };
                }
                if i.key_pressed(Key::ArrowLeft) {
                    self.axis_lock = if self.axis_lock == Some(Axis::Y) {
                        None
                    } else {
                        Some(Axis::Y)
                    };
                }
                if i.key_pressed(Key::ArrowUp) {
                    self.axis_lock = if self.axis_lock == Some(Axis::Z) {
                        None
                    } else {
                        Some(Axis::Z)
                    };
                }
                if i.key_pressed(Key::ArrowDown) {
                    self.axis_lock = None;
                }
                if i.modifiers.command && i.key_pressed(Key::Z) {
                    self.history_step(i.modifiers.shift);
                    return;
                }
                if i.modifiers.command && i.key_pressed(Key::Y) {
                    self.history_step(true);
                    return;
                }
            }
            if let Some(position) = i.pointer.hover_pos() {
                let cursor = Vec2::new(
                    2.0 * (position.x - rect.left()) / rect.width() - 1.0,
                    1.0 - 2.0 * (position.y - rect.top()) / rect.height(),
                );
                let (origin, direction) = self.camera.ray(cursor, aspect);
                let view = View {
                    matrix: self.camera.view_projection(aspect).as_dmat4()
                        * self.history.world_transform(),
                    size: glam::DVec2::new(rect.width() as f64, rect.height() as f64),
                };
                let mut query = Query::new(
                    glam::DVec2::new(
                        (position.x - rect.left()) as f64,
                        (position.y - rect.top()) as f64,
                    ),
                    Ray {
                        origin: self
                            .history
                            .world_transform()
                            .inverse()
                            .transform_point3(origin.as_dvec3()),
                        direction: self
                            .history
                            .world_transform()
                            .inverse()
                            .transform_vector3(direction.as_dvec3()),
                    },
                    view,
                );
                query.section = self.local_section();
                query.plane = self.tool.plane();
                query.anchor = self.tool.anchor();
                query.axis_lock = self.axis_lock;
                query.previous_direction = self.tool.previous_direction();
                query.tangent = self.tool.tangent_hint();
                self.snap = self.engine.infer(self.history.model(), &query);
                if let Some(snap) = &self.snap {
                    if let Err(error) = self.tool.pointer_move(snap.position) {
                        self.error = Some(error.to_string());
                    }
                }
            }
            if !keyboard_free {
                return;
            }
            for event in &i.events {
                if let egui::Event::Text(text) = event {
                    let starts_measure = text
                        .chars()
                        .next()
                        .is_some_and(|c| c.is_ascii_digit() || "+-.".contains(c));
                    if !self.measurement.is_empty()
                        || (self.tool.anchor().is_some() && starts_measure)
                    {
                        self.measurement.push_str(text);
                    }
                }
            }
            if i.key_pressed(Key::Backspace) {
                self.measurement.pop();
            }
            if i.key_pressed(Key::Enter) && !self.measurement.is_empty() {
                self.apply_measurement();
            }
        });
        if response.clicked_by(PointerButton::Primary) {
            if let Some(snap) = self.snap.clone() {
                if self.tool.anchors().is_empty() {
                    if let Some(SnapEntity::Face(id)) = snap.entity {
                        if let Some(face) = self.history.model().faces().get(&id) {
                            if let Err(error) = self.tool.set_plane(DrawingPlane {
                                origin: snap.position,
                                normal: face.plane.normal,
                            }) {
                                self.error = Some(error.to_string());
                            }
                        }
                    }
                }
                let before = self.tool.clone();
                match self.tool.click(snap.position) {
                    Ok(Some(command)) => {
                        if !self.commit(command) {
                            self.tool = before;
                        }
                    }
                    Ok(None) => self.error = None,
                    Err(error) => self.error = Some(error.to_string()),
                }
            }
        }
    }
}
fn toggle_projection(camera: &mut Camera) {
    camera.projection = match camera.projection {
        Projection::Perspective => Projection::Orthographic,
        Projection::Orthographic => Projection::Perspective,
    };
}
fn snap_label(kind: SnapKind) -> &'static str {
    match kind {
        SnapKind::Endpoint => "Endpoint",
        SnapKind::Intersection => "Intersection",
        SnapKind::Midpoint => "Midpoint",
        SnapKind::OnEdge => "On edge",
        SnapKind::OnFace => "On face",
        SnapKind::Axis(Axis::X) => "Red axis",
        SnapKind::Axis(Axis::Y) => "Green axis",
        SnapKind::Axis(Axis::Z) => "Blue axis",
        SnapKind::Parallel => "Parallel",
        SnapKind::Perpendicular => "Perpendicular",
        SnapKind::Tangent => "Tangent guide",
        SnapKind::Plane => "Work plane",
    }
}
impl eframe::App for DrafterApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.transition_tick(ctx);
        let camera_at_tick = (
            self.camera.target,
            self.camera.distance,
            self.camera.yaw,
            self.camera.pitch,
            self.camera.projection,
        );
        self.workspace_header(ctx);
        self.workspace_status(ctx);
        self.recovery_window(ctx);
        self.autosave_tick(ctx);
        self.scene_tabs(ctx);
        self.tool_palette(ctx);
        if self.show_tray {
            egui::SidePanel::right("default_tray")
                .default_width(280.)
                .width_range(250. ..=420.)
                .show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        ui.strong("Default Tray");
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.small_button("Hide").clicked() {
                                self.show_tray = false;
                            }
                        });
                    });
                    ui.separator();
                    egui::ScrollArea::vertical()
                        .id_salt("tray_scroll")
                        .show(ui, |ui| {
                            egui::CollapsingHeader::new("Entity Info")
                                .default_open(true)
                                .show(ui, |ui| {
                                    ui.label(format!(
                                        "{} edges · {} faces selected",
                                        self.selection.edges.len(),
                                        self.selection.faces.len()
                                    ));
                                    ui.weak(format!(
                                        "Model: {} edges · {} faces",
                                        self.history.model().edges().len(),
                                        self.history.model().faces().len()
                                    ));
                                });
                            self.organization_panel(ui);
                            self.presentation_panel(ui);
                        });
                });
        }
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(Color32::from_rgb(
                self.presentation.style.background[0],
                self.presentation.style.background[1],
                self.presentation.style.background[2],
            )))
            .show(ctx, |ui| {
                let (rect, response) =
                    ui.allocate_exact_size(ui.available_size(), egui::Sense::click_and_drag());
                if rect.width() <= 0. || rect.height() <= 0. {
                    return;
                }
                let aspect = rect.width() / rect.height();
                if self.files.fit {
                    self.extents(aspect);
                    self.files.fit = false;
                }
                self.paint_environment(ui, rect);
                self.viewport_input(&response);
                if camera_at_tick
                    != (
                        self.camera.target,
                        self.camera.distance,
                        self.camera.yaw,
                        self.camera.pitch,
                        self.camera.projection,
                    )
                {
                    self.cancel_scene_transition();
                }
                let matrix = self.camera.view_projection(aspect);
                ui.painter()
                    .add(eframe::egui_wgpu::Callback::new_paint_callback(
                        rect,
                        GridCallback {
                            view_projection: matrix.to_cols_array_2d(),
                        },
                    ));
                self.prepare_sections();
                ui.painter()
                    .add(eframe::egui_wgpu::Callback::new_paint_callback(
                        rect,
                        SceneCallback {
                            view_projection: matrix.to_cols_array_2d(),
                            size: [
                                (rect.width() * ctx.pixels_per_point()).round() as u32,
                                (rect.height() * ctx.pixels_per_point()).round() as u32,
                            ],
                            // Layout may request another egui pass. Keep uploads queued
                            // until a pass that will actually reach the GPU renderer.
                            update: if ctx.will_discard() {
                                self.pending.clone()
                            } else {
                                self.pending.take()
                            },
                            style: self.presentation.style.clone(),
                            section: self.presentation.sections.cut().cloned(),
                            caps: self.section_ui.caps.clone(),
                            shadows: self.presentation.shadows.clone(),
                            bounds: self.section_ui.bounds,
                        },
                    ));
                self.section_widgets(ui, rect);
                let view = View {
                    matrix: matrix.as_dmat4() * self.history.world_transform(),
                    size: glam::DVec2::new(rect.width() as f64, rect.height() as f64),
                };
                let screen = |point| {
                    view.project(point)
                        .map(|p| rect.min + egui::vec2(p.x as f32, p.y as f32))
                };
                self.modify_overlay(ui, rect, view);
                self.organization_overlay(ui, rect);
                let preview = self.tool.preview();
                for segment in preview.windows(2) {
                    if let (Some(a), Some(b)) = (screen(segment[0]), screen(segment[1])) {
                        ui.painter().line_segment(
                            [a, b],
                            egui::Stroke::new(2.0_f32, Color32::from_rgb(113, 226, 203)),
                        );
                    }
                }
                if let Some(snap) = &self.snap {
                    if let Some(guide) = snap.guide {
                        if let (Some(a), Some(b)) = (screen(guide[0]), screen(guide[1])) {
                            ui.painter().line_segment(
                                [a, b],
                                egui::Stroke::new(1.0_f32, Color32::from_rgb(100, 164, 190)),
                            );
                        }
                    }
                    if let Some(p) = screen(snap.position) {
                        ui.painter().circle_stroke(
                            p,
                            5.,
                            egui::Stroke::new(2.0_f32, Color32::from_rgb(250, 202, 114)),
                        );
                        ui.painter().text(
                            p + egui::vec2(10., -10.),
                            egui::Align2::LEFT_BOTTOM,
                            snap_label(snap.kind),
                            egui::FontId::proportional(12.),
                            self.viewport_ink(),
                        );
                    }
                }
            });
    }
}
#[cfg(test)]
mod tests {
    #[test]
    fn projection_toggle_round_trips() {
        let mut camera = super::Camera::default();
        let original = camera.projection;
        super::toggle_projection(&mut camera);
        assert_ne!(camera.projection, original);
        super::toggle_projection(&mut camera);
        assert_eq!(camera.projection, original);
    }
}

#[cfg(test)]
mod input_tests {
    use super::*;

    pub(super) fn frame(
        app: &mut DrafterApp,
        ctx: &egui::Context,
        events: Vec<egui::Event>,
        modifiers: egui::Modifiers,
    ) {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(800.0, 600.0),
            )),
            events,
            modifiers,
            ..Default::default()
        };
        let _ = ctx.run(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let (_, response) =
                    ui.allocate_exact_size(egui::vec2(600.0, 400.0), egui::Sense::click_and_drag());
                app.viewport_input(&response);
            });
        });
    }

    fn key(key: Key) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }
    }

    #[test]
    fn shortcuts_are_scoped_to_viewport() {
        let mut app = DrafterApp::default();
        let ctx = egui::Context::default();
        let original = app.camera.projection;
        frame(
            &mut app,
            &ctx,
            vec![
                egui::Event::PointerMoved(egui::pos2(750.0, 550.0)),
                key(Key::Num5),
            ],
            egui::Modifiers::NONE,
        );
        assert_eq!(app.camera.projection, original);
        frame(
            &mut app,
            &ctx,
            vec![egui::Event::Key {
                key: Key::Num5,
                physical_key: None,
                pressed: false,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
            egui::Modifiers::NONE,
        );
        frame(
            &mut app,
            &ctx,
            vec![
                egui::Event::PointerMoved(egui::pos2(300.0, 200.0)),
                key(Key::Num5),
            ],
            egui::Modifiers::NONE,
        );
        assert_ne!(app.camera.projection, original);
    }

    #[test]
    fn selection_mode_keeps_drawing_shortcuts_and_scroll_zoom() {
        let mut app = DrafterApp::default();
        app.choose_modify(ModifyKind::Select);
        let ctx = egui::Context::default();
        frame(
            &mut app,
            &ctx,
            vec![egui::Event::PointerMoved(egui::pos2(300., 200.))],
            egui::Modifiers::NONE,
        );
        let distance = app.camera.distance;
        frame(
            &mut app,
            &ctx,
            vec![
                egui::Event::PointerMoved(egui::pos2(300., 200.)),
                egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta: egui::vec2(0., 80.),
                    modifiers: egui::Modifiers::NONE,
                },
            ],
            egui::Modifiers::NONE,
        );
        assert_ne!(app.camera.distance, distance);
        frame(&mut app, &ctx, vec![key(Key::R)], egui::Modifiers::NONE);
        assert!(!app.modifying);
        assert_eq!(app.tool.kind(), ToolKind::Rectangle);
        frame(&mut app, &ctx, vec![key(Key::Space)], egui::Modifiers::NONE);
        assert!(app.modifying);
        assert_eq!(app.modify.kind(), ModifyKind::Select);
    }

    #[test]
    fn middle_drag_routes_orbit_and_shift_routes_pan() {
        for shift in [false, true] {
            let mut app = DrafterApp::default();
            let original = app.camera.clone();
            let ctx = egui::Context::default();
            let modifiers = egui::Modifiers {
                shift,
                ..Default::default()
            };
            let position = egui::pos2(300.0, 200.0);
            frame(
                &mut app,
                &ctx,
                vec![egui::Event::PointerMoved(position)],
                modifiers,
            );
            frame(
                &mut app,
                &ctx,
                vec![egui::Event::PointerButton {
                    pos: position,
                    button: PointerButton::Middle,
                    pressed: true,
                    modifiers,
                }],
                modifiers,
            );
            frame(
                &mut app,
                &ctx,
                vec![egui::Event::PointerMoved(position + egui::vec2(30.0, 15.0))],
                modifiers,
            );
            if shift {
                assert_ne!(app.camera.target, original.target);
                assert_eq!(app.camera.yaw, original.yaw);
            } else {
                assert_ne!(app.camera.yaw, original.yaw);
                assert_eq!(app.camera.target, original.target);
            }
        }
    }
}

#[cfg(test)]
mod drawing_tests {
    use super::input_tests::frame;
    use super::*;
    fn key(key: Key, modifiers: egui::Modifiers) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        }
    }
    fn click(app: &mut DrafterApp, ctx: &egui::Context, world: DVec3) {
        let view = View {
            matrix: app.camera.view_projection(1.5).as_dmat4(),
            size: glam::DVec2::new(600., 400.),
        };
        let p = view.project(world).unwrap();
        let position = egui::pos2(p.x as f32 + 8., p.y as f32 + 8.);
        let modifiers = egui::Modifiers::NONE;
        frame(
            app,
            ctx,
            vec![egui::Event::PointerMoved(position)],
            modifiers,
        );
        for pressed in [true, false] {
            frame(
                app,
                ctx,
                vec![egui::Event::PointerButton {
                    pos: position,
                    button: PointerButton::Primary,
                    pressed,
                    modifiers,
                }],
                modifiers,
            );
        }
    }
    #[test]
    fn canvas_rectangle_typed_dimensions_and_undo_redo_prepare_dirty_mesh() {
        let mut app = DrafterApp::default();
        let ctx = egui::Context::default();
        app.set_tool(ToolKind::Rectangle);
        click(&mut app, &ctx, DVec3::ZERO);
        assert!(app.tool.anchor().is_some());
        frame(
            &mut app,
            &ctx,
            vec![
                egui::Event::Text("2m,3m".into()),
                key(Key::Enter, egui::Modifiers::NONE),
            ],
            egui::Modifiers::NONE,
        );
        assert_eq!(app.model().faces().len(), 1);
        assert_eq!(app.model().edges().len(), 4);
        let face = *app.model().faces().keys().next().unwrap();
        assert!((app.model().face_area(face).unwrap() - 6.).abs() < 1e-4);
        assert!(app
            .pending
            .as_ref()
            .is_some_and(|update| update.geometry.iter().any(|g| g.mesh.reset)));
        let modifiers = egui::Modifiers {
            command: true,
            ctrl: true,
            ..Default::default()
        };
        frame(&mut app, &ctx, vec![key(Key::Z, modifiers)], modifiers);
        assert!(app.model().faces().is_empty());
        frame(&mut app, &ctx, vec![key(Key::Y, modifiers)], modifiers);
        assert_eq!(app.model().faces().len(), 1);
    }
    #[test]
    fn typing_length_on_canvas_does_not_trigger_standard_view_shortcut() {
        let mut app = DrafterApp::default();
        let ctx = egui::Context::default();
        click(&mut app, &ctx, DVec3::ZERO);
        let projection = app.camera.projection;
        frame(
            &mut app,
            &ctx,
            vec![
                key(Key::Num5, egui::Modifiers::NONE),
                egui::Event::Text("5m".into()),
                key(Key::Enter, egui::Modifiers::NONE),
            ],
            egui::Modifiers::NONE,
        );
        assert_eq!(app.camera.projection, projection);
        assert_eq!(app.model().edges().len(), 1);
        let edge = app.model().edges().values().next().unwrap();
        let length = app.model().vertices()[&edge.vertices[0]]
            .position
            .distance(app.model().vertices()[&edge.vertices[1]].position);
        assert!((length - 5.).abs() < 1e-5);
    }
}

#[cfg(test)]
mod axis_input_test {
    use super::*;
    #[test]
    fn hard_axis_lock_is_applied_before_the_current_frames_pick() {
        let mut app = DrafterApp::default();
        let ctx = egui::Context::default();
        app.tool.click(DVec3::ZERO).unwrap();
        let view = View {
            matrix: app.camera.view_projection(1.5).as_dmat4(),
            size: glam::DVec2::new(600., 400.),
        };
        let position = view.project(DVec3::new(1., 1., 2.)).unwrap();
        super::input_tests::frame(&mut app, &ctx, vec![], egui::Modifiers::NONE);
        super::input_tests::frame(
            &mut app,
            &ctx,
            vec![
                egui::Event::PointerMoved(egui::pos2(
                    position.x as f32 + 8.,
                    position.y as f32 + 8.,
                )),
                egui::Event::Key {
                    key: Key::ArrowUp,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
            egui::Modifiers::NONE,
        );
        let snap = app.snap.as_ref().unwrap();
        assert_eq!(snap.kind, SnapKind::Axis(Axis::Z));
        assert!(snap.position.x.abs() < 1e-7 && snap.position.y.abs() < 1e-7);
    }
}

#[cfg(feature = "kernel-benchmark")]
pub mod organization_benchmark;

#[cfg(test)]
mod style_tests {
    use super::*;
    #[test]
    fn switching_every_style_keeps_geometry_and_instances_clean() {
        let mut app = DrafterApp::default();
        app.execute(Command::AddPolyline {
            points: vec![DVec3::ZERO, DVec3::X, DVec3::X + DVec3::Y, DVec3::Y],
            closed: true,
        })
        .unwrap();
        app.pending.take();
        let revision = app.history.revision();
        let before = app.history.to_data();
        for mode in [
            cindral_core::StyleMode::Shaded,
            cindral_core::StyleMode::Textured,
            cindral_core::StyleMode::Wireframe,
            cindral_core::StyleMode::HiddenLine,
            cindral_core::StyleMode::XRay,
        ] {
            app.set_view_style(cindral_core::ViewStyle {
                mode,
                profiles: true,
                soft_edges: true,
                smooth_edges: true,
                background: [240, 240, 240],
                sky: Some([100, 140, 200]),
                ground: Some([100, 110, 80]),
            });
            assert!(app.pending.is_none());
            let delta = app.scene.sync(&app.history).unwrap();
            assert!(delta.geometry.is_empty() && delta.batches.is_none());
            assert_eq!(app.history.revision(), revision);
        }
        assert_eq!(app.history.to_data(), before);
    }
}
