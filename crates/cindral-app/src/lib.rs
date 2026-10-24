//! Native editor, pure drawing/inference routing, and cached GPU geometry.
use cindral_core::{Command, DVec3, DocumentHistory as History};
use cindral_inference::{Axis, DrawingPlane, Engine, Query, Ray, Snap, SnapEntity, SnapKind, View};
use cindral_render::{
    camera::{Camera, Projection, StandardView},
    grid::{GridCallback, GridRenderer},
    scene::{SceneCallback, SceneRenderer, SceneUpdate},
};
use cindral_tools::{ModifyKind, ModifySession, Selection, ToolKind, ToolSession};
mod builtin_materials;
mod dae_ui;
mod files_ui;
#[cfg(all(test, feature = "kernel-benchmark"))]
mod materials_benchmark;
mod materials_library;
#[cfg(feature = "kernel-benchmark")]
pub mod materials_real_file;
mod materials_ui;
mod measurements_ui;
mod modify_ui;
mod organization_ui;
#[cfg(feature = "kernel-benchmark")]
pub mod pan_benchmark;
mod pan_ui;
pub mod png_cli;
pub mod png_export;
pub mod png_job;
mod png_ui;
mod presentation_ui;
#[cfg(feature = "kernel-benchmark")]
pub mod real_file_benchmark;
#[cfg(feature = "kernel-benchmark")]
mod real_file_gpu;
mod rotate_ui;
mod scene_cache;
mod section_ui;
mod shadow_ui;
mod shortcuts;
pub mod theme;
mod workspace_ui;
use eframe::egui::{self, Color32, Key, PointerButton};
use glam::Vec2;
use std::sync::Arc;

/// Editor state; geometry changes are committed only through core commands.
pub struct DrafterApp {
    #[cfg(feature = "kernel-benchmark")]
    replay_rect: egui::Rect,
    files: files_ui::FileState,
    png: png_ui::PngUi,
    #[cfg(target_os = "linux")]
    system_theme: theme::SystemTheme,
    shortcuts: shortcuts::Ui,
    show_tray: bool,
    inspector_tab: workspace_ui::InspectorTab,
    show_grid: bool,
    presentation: cindral_core::Presentation,
    presentation_ui: presentation_ui::PresentationUi,
    presentation_serial: u64,
    section_ui: section_ui::SectionUi,
    camera: Camera,
    pan_drag: Option<pan_ui::PanDrag>,
    modifying: bool,
    modify: ModifySession,
    selection: Selection,
    selection_start: Option<glam::DVec2>,
    modify_press: Option<glam::DVec2>,
    drag_axis: DVec3,
    rotate: rotate_ui::RotateUi,
    last_tape: Option<f64>,
    tape_resize: Option<measurements_ui::TapeResize>,
    tape_target: Option<measurements_ui::TapeTarget>,
    tape_resized: bool,
    guide_definition: cindral_core::DefinitionId,
    guide_cache: std::collections::BTreeMap<cindral_core::DefinitionId, Vec<cindral_tools::Guide>>,
    history: History,
    engine: Engine,
    tool: ToolSession,
    axis_lock: Option<Axis>,
    materials: materials_ui::MaterialsUi,
    paint_hover: materials_ui::PaintHover,
    snap: Option<Snap>,
    draw_press: Option<Snap>,
    draw_drag: Option<()>,
    measurements: cindral_tools::measurements::MeasurementsBox,
    error: Option<String>,
    scene: scene_cache::SceneCache,
    organization: organization_ui::OrganizationUi,
    pending: Option<Arc<SceneUpdate>>,
    work_plane: DrawingPlane,
}
impl Default for DrafterApp {
    fn default() -> Self {
        Self {
            #[cfg(feature = "kernel-benchmark")]
            replay_rect: egui::Rect::NOTHING,
            files: files_ui::FileState::default(),
            png: Default::default(),
            #[cfg(target_os = "linux")]
            system_theme: Default::default(),
            shortcuts: Default::default(),
            show_tray: true,
            inspector_tab: workspace_ui::InspectorTab::Model,
            show_grid: false,
            presentation: Default::default(),
            presentation_ui: Default::default(),
            presentation_serial: 0,
            section_ui: Default::default(),
            camera: Camera::default(),
            pan_drag: None,
            modifying: false,
            modify: ModifySession::new(ModifyKind::Select),
            selection: Selection::default(),
            selection_start: None,
            modify_press: None,
            drag_axis: DVec3::Z,
            rotate: Default::default(),
            last_tape: None,
            tape_resize: None,
            tape_target: None,
            tape_resized: false,
            guide_definition: cindral_core::DefinitionId(0),
            guide_cache: Default::default(),
            history: History::new(),
            engine: Engine::default(),
            tool: ToolSession::new(ToolKind::Line),
            axis_lock: None,
            materials: Default::default(),
            paint_hover: Default::default(),
            snap: None,
            draw_press: None,
            draw_drag: None,
            measurements: Default::default(),
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
        let png = png_ui::PngUi::with_surface(render.target_format.is_srgb());
        png.theme().apply(&cc.egui_ctx);
        let notice = shortcuts::first_launch_notice();
        Ok(Self {
            shortcuts: shortcuts::Ui {
                notice,
                ..Default::default()
            },
            modifying: true,
            #[cfg(target_os = "linux")]
            system_theme: theme::SystemTheme::start(cc.egui_ctx.clone()),
            png,
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
        self.pan_drag = None;
        self.shortcuts.navigation = None;
        self.cancel_section();
        self.cancel_modify();
        if let Ok(true) = self.history.cancel_preview() {
            self.refresh_mesh();
        }
        self.organization.drag = None;
        self.organization.completed = None;
        self.modify.cancel();
        self.modifying = false;
        self.tool.set_kind(kind);
        if let Err(error) = self.tool.set_plane(self.work_plane) {
            self.error = Some(error.to_string());
        }
        self.axis_lock = None;
        self.measurements.reset();
        self.snap = None;
        self.draw_press = None;
        self.draw_drag = None;
    }
    fn refresh_mesh(&mut self) {
        let _phase = cindral_core::profile::Scope::new(4);
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
            Ok(true) => true,
            Ok(false) => {
                self.error = Some(
                    "No change: input is below the geometry tolerance or already matches the model"
                        .into(),
                );
                false
            }
            Err(error) => {
                self.error = Some(error.to_string());
                false
            }
        }
    }
    fn history_step(&mut self, redo: bool) {
        self.measurements.reset();
        self.tape_resize = None;
        self.tape_target = None;
        self.tape_resized = false;
        if let Ok(true) = self.history.cancel_preview() {
            self.refresh_mesh();
        }
        self.modify.cancel();
        self.rotate = Default::default();
        self.organization.drag = None;
        self.organization.completed = None;
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
                if let Err(e) = self.reconcile_import_textures() {
                    self.error = Some(e.to_string());
                }
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
        if self.modifying && self.modify.kind() == ModifyKind::Rotate {
            self.rotate_measurement();
            return;
        }
        if self.modifying {
            if self.modify.kind() == ModifyKind::TapeMeasure {
                self.request_tape_resize();
                return;
            }
            self.modify_measurement();
            return;
        }
        let before = self.tool.clone();
        let replacing = self.tool.editing_completed();
        match self
            .tool
            .measure_with_settings(&self.measurements.text, self.presentation.measurements)
        {
            Ok(Some(command)) => {
                let result = if replacing {
                    self.history.replace_last(command)
                } else {
                    self.history.execute(command)
                };
                if matches!(result, Ok(true)) {
                    self.refresh_mesh();
                    self.error = None;
                    self.measurements.completed();
                } else {
                    self.error = Some(
                        result
                            .err()
                            .map(|e| e.to_string())
                            .unwrap_or_else(|| "No change".into()),
                    );
                    self.tool = before;
                }
            }
            Ok(None) => {
                self.measurements.applied();
                self.error = None;
            }
            Err(error) => self.error = Some(error.to_string()),
        }
        self.measurements.invalid = self.error.is_some();
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

    fn viewport_overlay(&self, color: Color32) -> Color32 {
        theme::overlay_color(self.presentation.style.background, color)
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
        let horizon = rect.top() + png_export::horizon(&self.camera) * rect.height();
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

    fn paint_snap_overlay(&self, ui: &egui::Ui, response: &egui::Response, view: View) {
        // Placement feedback belongs to drawing, and only while the canvas is in use.
        let navigating = response.ctx.input(|i| {
            i.pointer.button_down(PointerButton::Middle)
                || i.smooth_scroll_delta != egui::Vec2::ZERO
        });
        if (self.modifying && self.modify.kind() != ModifyKind::Rotate)
            || !response.hovered()
            || navigating
            || self.pan_drag.is_some()
            || self.shortcuts.navigation.is_some()
        {
            return;
        }
        let Some(snap) = &self.snap else {
            return;
        };
        let screen = |point| {
            view.project(point)
                .map(|p| response.rect.min + egui::vec2(p.x as f32, p.y as f32))
        };
        let painter = ui.painter().with_clip_rect(response.rect);
        let Some(p) = screen(snap.position) else {
            return;
        };
        if snap.kind == SnapKind::Plane {
            // A quiet point shows the projected placement without implying a geometry snap.
            painter.circle_filled(p, 2., self.viewport_ink().gamma_multiply(0.55));
            return;
        }
        if let Some(guide) = snap.guide {
            if let (Some(a), Some(b)) = (screen(guide[0]), screen(guide[1])) {
                painter.line_segment(
                    [a, b],
                    egui::Stroke::new(
                        1.0_f32,
                        self.viewport_overlay(Color32::from_rgb(100, 164, 190)),
                    ),
                );
            }
        }
        painter.circle_stroke(
            p,
            5.,
            egui::Stroke::new(
                2.0_f32,
                self.viewport_overlay(Color32::from_rgb(250, 202, 114)),
            ),
        );
        painter.text(
            p + egui::vec2(10., -10.),
            egui::Align2::LEFT_BOTTOM,
            snap_label(snap.kind),
            egui::FontId::proportional(12.),
            self.viewport_ink(),
        );
    }

    fn viewport_input(&mut self, response: &egui::Response) {
        if self.shortcuts.open || self.tape_resize.is_some() {
            self.pan_drag = None;
            return;
        }
        let rect = response.rect;
        let aspect = rect.width() / rect.height().max(1.0);
        let drawing_drag =
            self.draw_drag.is_some() || response.drag_started_by(PointerButton::Primary);
        let temporary_pan = response
            .ctx
            .input(|i| i.pointer.button_down(PointerButton::Middle) && i.modifiers.shift);
        if self.pan_drag.is_none() && !temporary_pan && self.capture_measurement_keys(response) {
            return;
        }
        if self.route_shortcuts(response) {
            return;
        }
        if self.shortcuts.open {
            return;
        }
        if self.pan_input(response) {
            return;
        }
        if response.dragged_by(PointerButton::Middle) {
            let delta = response.ctx.input(|i| i.pointer.delta());
            self.camera.orbit(Vec2::new(delta.x, delta.y));
            return;
        }
        if let Some(mode) = self.shortcuts.navigation {
            if response.hovered() {
                response.clone().on_hover_cursor(egui::CursorIcon::Grab);
                let (delta, scroll) = response
                    .ctx
                    .input(|i| (i.pointer.delta(), i.smooth_scroll_delta.y));
                if response.dragged_by(PointerButton::Primary) {
                    match mode {
                        shortcuts::Action::Orbit => self.camera.orbit(Vec2::new(delta.x, delta.y)),
                        _ => self.camera.zoom(-delta.y, Vec2::ZERO, aspect),
                    }
                }
                self.camera.zoom(scroll, Vec2::ZERO, aspect);
            }
            if response
                .ctx
                .input(|i| shortcuts::pressed(i, shortcuts::Action::Cancel))
            {
                self.shortcuts.navigation = None;
            }
            return;
        }
        if self.section_input(response) {
            return;
        }
        if !(self.modifying && self.modify.kind() == ModifyKind::Rotate)
            && self.organization_pointer(response)
        {
            return;
        }
        if !response.hovered() {
            if self.modifying && (self.modify.active() || self.selection_start.is_some()) {
                if self.modify.kind() == ModifyKind::Rotate {
                    self.rotate_pointer(response);
                } else {
                    self.modify_viewport_input(response);
                }
                return;
            }
            if !drawing_drag {
                self.draw_press = None;
                return;
            }
        }
        response.ctx.input(|i| {
            if let Some(position) = i.pointer.hover_pos() {
                let cursor = Vec2::new(
                    2. * (position.x - rect.left()) / rect.width() - 1.,
                    1. - 2. * (position.y - rect.top()) / rect.height(),
                );
                self.camera.zoom(i.smooth_scroll_delta.y, cursor, aspect);
            }
        });
        if self.modifying {
            if self.modify.kind() == ModifyKind::Rotate {
                self.rotate_pointer(response);
            } else {
                self.modify_viewport_input(response);
            }
            return;
        }
        let keyboard_free =
            !response.ctx.wants_keyboard_input() && self.measurements.text.is_empty();
        response.ctx.input(|i| {
            if keyboard_free {
                if shortcuts::pressed(i, shortcuts::Action::Cancel) {
                    self.tool.cancel();
                    self.measurements.reset();
                    self.axis_lock = None;
                    self.error = None;
                    self.draw_press = None;
                    self.draw_drag = None;
                }
                if shortcuts::pressed(i, shortcuts::Action::AxisX) {
                    self.axis_lock = if self.axis_lock == Some(Axis::X) {
                        None
                    } else {
                        Some(Axis::X)
                    };
                }
                if shortcuts::pressed(i, shortcuts::Action::AxisY) {
                    self.axis_lock = if self.axis_lock == Some(Axis::Y) {
                        None
                    } else {
                        Some(Axis::Y)
                    };
                }
                if shortcuts::pressed(i, shortcuts::Action::AxisZ) {
                    self.axis_lock = if self.axis_lock == Some(Axis::Z) {
                        None
                    } else {
                        Some(Axis::Z)
                    };
                }
                if shortcuts::pressed(i, shortcuts::Action::AxisClear) {
                    self.axis_lock = None;
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
        });
        if self.modifying {
            self.draw_press = None;
            self.draw_drag = None;
            return;
        }

        if response
            .ctx
            .input(|i| i.pointer.button_pressed(PointerButton::Primary))
        {
            self.tool.end_remeasurement();
            self.measurements.reset();
            self.draw_press = self.snap.clone();
        }

        if response.drag_started_by(PointerButton::Primary)
            && matches!(self.tool.kind(), ToolKind::Line | ToolKind::Rectangle)
        {
            let starts_with_anchor = !self.tool.anchors().is_empty();
            self.draw_drag = Some(());
            if !starts_with_anchor {
                if let Some(start) = self.draw_press.clone().or_else(|| self.snap.clone()) {
                    self.click_tool_at(start);
                }
            }
        }

        if response.drag_stopped_by(PointerButton::Primary) {
            if self.draw_drag.take().is_some() {
                if let Some(end) = self.snap.clone() {
                    self.click_tool_at(end);
                }
                self.draw_press = None;
                return;
            }
            self.draw_press = None;
        }

        if response.clicked_by(PointerButton::Primary) {
            if let Some(snap) = self.snap.clone() {
                self.click_tool_at(snap);
            }
            self.draw_press = None;
        }
    }

    fn click_tool_at(&mut self, snap: Snap) {
        self.measurements.reset();
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
                } else {
                    self.measurements.completed();
                }
            }
            Ok(None) => self.error = None,
            Err(error) => self.error = Some(error.to_string()),
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
    #[cfg(target_os = "linux")]
    fn raw_input_hook(&mut self, _ctx: &egui::Context, input: &mut egui::RawInput) {
        self.system_theme.update(input);
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.draw_frame(ctx);
    }
    fn on_exit(&mut self) {
        if let Err(error) = self.preserve_on_close() {
            tracing::error!("could not preserve draft on exit: {error}");
        }
    }
}
impl DrafterApp {
    fn draw_frame(&mut self, ctx: &egui::Context) {
        if self.closing(ctx) {
            return;
        }
        let _frame_profile = cindral_core::profile::Frame::new();
        self.transition_tick(ctx);
        let camera_at_tick = (
            self.camera.target,
            self.camera.distance,
            self.camera.yaw,
            self.camera.pitch,
            self.camera.projection,
        );
        let ui_profile = cindral_core::profile::Scope::new(7);
        self.shortcut_overlay(ctx);
        self.workspace_header(ctx);
        self.workspace_status(ctx);
        self.tape_resize_window(ctx);
        self.recovery_offer();
        self.autosave_tick(ctx);
        self.scene_tabs(ctx);
        self.tool_palette(ctx);
        self.model_inspector(ctx);
        drop(ui_profile);
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
                    self.pan_drag = None;
                    return;
                }
                #[cfg(feature = "kernel-benchmark")]
                {
                    self.replay_rect = rect;
                }
                let aspect = rect.width() / rect.height();
                self.png.viewport = [
                    (rect.width() * ctx.pixels_per_point()).round() as u32,
                    (rect.height() * ctx.pixels_per_point()).round() as u32,
                ];
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
                            size: [rect.width(), rect.height()],
                            view_projection: matrix.to_cols_array_2d(),
                            show_grid: self.show_grid,
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
                self.rotate_overlay(ui, rect);
                self.organization_overlay(ui, rect);
                self.paint_hover_overlay(ui, rect, view);
                let preview = self.tool.preview();
                for segment in preview.windows(2) {
                    if let (Some(a), Some(b)) = (screen(segment[0]), screen(segment[1])) {
                        ui.painter().line_segment(
                            [a, b],
                            egui::Stroke::new(
                                2.0_f32,
                                self.viewport_overlay(Color32::from_rgb(113, 226, 203)),
                            ),
                        );
                    }
                }
                self.paint_snap_overlay(ui, &response, view);
                self.orientation_indicator(ui, rect);
                self.paint_tool_cursor(ui, &response);
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

#[cfg(feature = "kernel-benchmark")]
/// Imported versus drawn drag performance workload.
pub mod import_pushpull_benchmark;

#[cfg(test)]
mod draw_parity_tests;

#[cfg(test)]
mod shortcut_tests;

#[cfg(test)]
mod pan_tests;
