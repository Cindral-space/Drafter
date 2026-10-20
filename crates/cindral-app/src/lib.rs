//! Native editor, pure drawing/inference routing, and cached GPU geometry.
use cindral_core::{Command, DVec3, History};
use cindral_inference::{Axis, DrawingPlane, Engine, Query, Ray, Snap, SnapEntity, SnapKind, View};
use cindral_render::{
    camera::{Camera, Projection, StandardView},
    grid::{GridCallback, GridRenderer},
    mesh::{MeshCache, MeshUpdate, ModelCallback, ModelRenderer},
};
use cindral_tools::{ModifyKind, ModifySession, Selection, ToolKind, ToolSession};
mod modify_ui;
use eframe::egui::{self, Color32, Key, PointerButton, RichText};
use glam::Vec2;
use std::sync::Arc;

/// Editor state; geometry changes are committed only through core commands.
pub struct DrafterApp {
    camera: Camera,
    modifying: bool,
    modify: ModifySession,
    selection: Selection,
    selection_start: Option<glam::DVec2>,
    modify_press: Option<glam::DVec2>,
    drag_axis: DVec3,
    paint_color: [u8; 4],
    last_tape: Option<f64>,
    history: History,
    engine: Engine,
    tool: ToolSession,
    axis_lock: Option<Axis>,
    snap: Option<Snap>,
    measurement: String,
    error: Option<String>,
    mesh: Option<MeshCache>,
    pending: Option<Arc<MeshUpdate>>,
    work_plane: DrawingPlane,
}
impl Default for DrafterApp {
    fn default() -> Self {
        Self {
            camera: Camera::default(),
            modifying: false,
            modify: ModifySession::new(ModifyKind::Select),
            selection: Selection::default(),
            selection_start: None,
            modify_press: None,
            drag_axis: DVec3::Z,
            paint_color: [210, 130, 75, 255],
            last_tape: None,
            history: History::new(),
            engine: Engine::default(),
            tool: ToolSession::new(ToolKind::Line),
            axis_lock: None,
            snap: None,
            measurement: String::new(),
            error: None,
            mesh: None,
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
        renderer.callback_resources.insert(ModelRenderer::new(
            &render.device,
            render.target_format,
            4,
        ));
        let mut style = (*cc.egui_ctx.style()).clone();
        style.visuals = egui::Visuals::dark();
        style.visuals.panel_fill = Color32::from_rgb(24, 29, 36);
        style.visuals.selection.bg_fill = Color32::from_rgb(42, 105, 103);
        style.visuals.widgets.active.bg_fill = Color32::from_rgb(42, 105, 103);
        cc.egui_ctx.set_style(style);
        Ok(Self::default())
    }
    fn extents(&mut self, aspect: f32) {
        let (min, max) = self
            .history
            .model()
            .bounds()
            .unwrap_or((DVec3::splat(-10.0), DVec3::splat(10.0)));
        self.camera
            .zoom_extents(min.as_vec3(), max.as_vec3(), aspect);
    }
    fn set_tool(&mut self, kind: ToolKind) {
        if let Ok(true) = self.history.cancel_preview() {
            self.refresh_mesh();
        }
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
        let result = if let Some(cache) = &mut self.mesh {
            let changes = self.history.changes();
            cache.update(
                self.history.model(),
                &changes.faces.iter().copied().collect::<Vec<_>>(),
                &changes.edges.iter().copied().collect::<Vec<_>>(),
            )
        } else {
            MeshCache::rebuild(self.history.model()).map(|(cache, update)| {
                self.mesh = Some(cache);
                update
            })
        };
        match result {
            Ok(update) => {
                if update.reset {
                    self.pending = Some(Arc::new(update));
                } else if let Some(pending) = &mut self.pending {
                    let pending = Arc::make_mut(pending);
                    pending.face_writes.extend(update.face_writes);
                    pending.edge_writes.extend(update.edge_writes);
                    pending.face_vertices = update.face_vertices;
                    pending.edge_vertices = update.edge_vertices;
                } else {
                    self.pending = Some(Arc::new(update));
                }
            }
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
        if !response.hovered() {
            if self.modifying && (self.modify.active() || self.selection_start.is_some()) {
                self.modify_viewport_input(response);
            }
            return;
        }
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
                if self.measurement.is_empty() {
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
                    if i.key_pressed(Key::F) {
                        self.extents(aspect);
                    }
                    if self.tool.anchor().is_none() && i.key_pressed(Key::Num0) {
                        self.camera.set_view(StandardView::Iso);
                    }
                    if self.tool.anchor().is_none() && i.key_pressed(Key::Num1) {
                        self.camera.set_view(StandardView::Front);
                    }
                    if self.tool.anchor().is_none() && i.key_pressed(Key::Num3) {
                        self.camera.set_view(StandardView::Right);
                    }
                    if self.tool.anchor().is_none() && i.key_pressed(Key::Num7) {
                        self.camera.set_view(StandardView::Top);
                    }
                    if self.tool.anchor().is_none() && i.key_pressed(Key::Num5) {
                        toggle_projection(&mut self.camera);
                    }
                }
            }
            if let Some(position) = i.pointer.hover_pos() {
                let cursor = Vec2::new(
                    2.0 * (position.x - rect.left()) / rect.width() - 1.0,
                    1.0 - 2.0 * (position.y - rect.top()) / rect.height(),
                );
                self.camera.zoom(i.smooth_scroll_delta.y, cursor, aspect);
                let (origin, direction) = self.camera.ray(cursor, aspect);
                let view = View {
                    matrix: self.camera.view_projection(aspect).as_dmat4(),
                    size: glam::DVec2::new(rect.width() as f64, rect.height() as f64),
                };
                let mut query = Query::new(
                    glam::DVec2::new(
                        (position.x - rect.left()) as f64,
                        (position.y - rect.top()) as f64,
                    ),
                    Ray {
                        origin: origin.as_dvec3(),
                        direction: direction.as_dvec3(),
                    },
                    view,
                );
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
fn tool_label(kind: ToolKind) -> &'static str {
    match kind {
        ToolKind::Line => "Line",
        ToolKind::Rectangle => "Rectangle",
        ToolKind::Circle => "Circle",
        ToolKind::Arc => "Arc",
        ToolKind::Polygon => "Polygon",
    }
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
        egui::TopBottomPanel::top("header").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("CINDRAL DRAFTER")
                        .strong()
                        .size(20.)
                        .color(Color32::from_rgb(112, 217, 195)),
                );
                ui.separator();
                for (kind, shortcut) in [
                    (ToolKind::Line, "L"),
                    (ToolKind::Rectangle, "R"),
                    (ToolKind::Circle, "C"),
                    (ToolKind::Arc, "A"),
                    (ToolKind::Polygon, "P"),
                ] {
                    if ui
                        .selectable_label(
                            !self.modifying && self.tool.kind() == kind,
                            format!("{}  {shortcut}", tool_label(kind)),
                        )
                        .clicked()
                    {
                        self.set_tool(kind);
                    }
                }
                if ui
                    .add_enabled(self.history.undo_depth() > 0, egui::Button::new("Undo"))
                    .clicked()
                {
                    self.history_step(false);
                }
                if ui
                    .add_enabled(self.history.redo_depth() > 0, egui::Button::new("Redo"))
                    .clicked()
                {
                    self.history_step(true);
                }
            });
        });
        egui::TopBottomPanel::bottom("status").show(ctx,|ui| {
            ui.horizontal(|ui| {
                ui.label(if self.modifying {"Modify: select geometry, drag; Escape cancels; type value + Enter"} else {match self.tool.kind() {ToolKind::Line=>"Line: start, end; Esc stops chain",ToolKind::Rectangle=>"Rectangle: first corner, opposite corner",ToolKind::Circle=>"Circle: center, radius",ToolKind::Arc=>"Arc: center, start, end direction",ToolKind::Polygon=>"Polygon: center, vertex"}});
                ui.separator();ui.label("Measurements");
                let field=ui.add(egui::TextEdit::singleline(&mut self.measurement).desired_width(145.).hint_text(if self.modifying {match self.modify.kind(){ModifyKind::Rotate=>"angle / deg",ModifyKind::Scale=>"factor / x / %",_=>"distance"}} else {match self.tool.kind(){ToolKind::Rectangle=>"width,height",ToolKind::Arc=>"degrees",_=>"length / radius"}}));
                if field.lost_focus() && ctx.input(|i|i.key_pressed(Key::Enter)) {self.apply_measurement();}
            });
            if let Some(error)=&self.error {ui.colored_label(Color32::LIGHT_RED,error);}
            else {ui.label("MMB orbit · Shift+MMB pan · Scroll zoom · Arrows lock X/Y/Z · Esc cancel · Type a value, Enter");}
        });
        egui::SidePanel::left("navigation")
            .exact_width(174.)
            .resizable(false)
            .show(ctx, |ui| {
                ui.heading("Modify");
                for kind in [
                    ModifyKind::Select,
                    ModifyKind::PushPull,
                    ModifyKind::Move,
                    ModifyKind::Copy,
                    ModifyKind::Rotate,
                    ModifyKind::Scale,
                    ModifyKind::Offset,
                    ModifyKind::FollowMe,
                    ModifyKind::Eraser,
                    ModifyKind::TapeMeasure,
                    ModifyKind::PaintBucket,
                ] {
                    if ui
                        .selectable_label(
                            self.modifying && self.modify.kind() == kind,
                            match kind {
                                ModifyKind::Select => "Select",
                                ModifyKind::PushPull => "Push / Pull",
                                ModifyKind::Move => "Move",
                                ModifyKind::Copy => "Copy",
                                ModifyKind::Rotate => "Rotate",
                                ModifyKind::Scale => "Scale",
                                ModifyKind::Offset => "Offset",
                                ModifyKind::FollowMe => "Follow Me",
                                ModifyKind::Eraser => "Eraser",
                                ModifyKind::TapeMeasure => "Tape Measure",
                                ModifyKind::PaintBucket => "Paint Bucket",
                            },
                        )
                        .clicked()
                    {
                        self.choose_modify(kind);
                    }
                }
                if self.modifying && self.modify.kind() == ModifyKind::PaintBucket {
                    let mut color = [
                        self.paint_color[0],
                        self.paint_color[1],
                        self.paint_color[2],
                    ];
                    if ui.color_edit_button_srgb(&mut color).changed() {
                        self.paint_color = [color[0], color[1], color[2], 255];
                        self.modify.set_color(self.paint_color);
                    }
                }
                ui.separator();
                ui.heading("View");
                ui.selectable_value(
                    &mut self.camera.projection,
                    Projection::Perspective,
                    "Perspective",
                );
                ui.selectable_value(
                    &mut self.camera.projection,
                    Projection::Orthographic,
                    "Orthographic",
                );
                for (label, view) in [
                    ("Isometric  0", StandardView::Iso),
                    ("Top  7", StandardView::Top),
                    ("Bottom", StandardView::Bottom),
                    ("Front  1", StandardView::Front),
                    ("Back", StandardView::Back),
                    ("Left", StandardView::Left),
                    ("Right  3", StandardView::Right),
                ] {
                    if ui
                        .add_sized([150., 25.], egui::Button::new(label))
                        .clicked()
                    {
                        self.camera.set_view(view);
                    }
                }
                if ui.button("Zoom extents  F").clicked() {
                    let size = ctx.available_rect().size();
                    self.extents(size.x / size.y.max(1.));
                }
                ui.separator();
                ui.label("Work plane");
                ui.horizontal(|ui| {
                    for (label, normal) in [("XY", DVec3::Z), ("XZ", -DVec3::Y), ("YZ", DVec3::X)] {
                        if ui.button(label).clicked() {
                            self.work_plane = DrawingPlane {
                                origin: DVec3::ZERO,
                                normal,
                            };
                            self.tool.cancel();
                            if let Err(error) = self.tool.set_plane(self.work_plane) {
                                self.error = Some(error.to_string());
                            }
                        }
                    }
                });
                if matches!(
                    self.tool.kind(),
                    ToolKind::Circle | ToolKind::Arc | ToolKind::Polygon
                ) {
                    let mut count = self.tool.segments();
                    ui.horizontal(|ui| {
                        ui.label("Segments");
                        if ui
                            .add(egui::DragValue::new(&mut count).range(3..=256))
                            .changed()
                        {
                            if let Err(error) = self.tool.set_segments(count) {
                                self.error = Some(error.to_string());
                            }
                        }
                    });
                }
                ui.separator();
                ui.label(format!("{} edges", self.history.model().edges().len()));
                ui.label(format!("{} faces", self.history.model().faces().len()));
                if let Some(snap) = &self.snap {
                    ui.label(snap_label(snap.kind));
                }
                if let Some(axis) = self.axis_lock {
                    ui.label(format!(
                        "{} axis locked",
                        match axis {
                            Axis::X => "Red",
                            Axis::Y => "Green",
                            Axis::Z => "Blue",
                        }
                    ));
                }
            });
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(Color32::from_rgb(17, 22, 29)))
            .show(ctx, |ui| {
                let (rect, response) =
                    ui.allocate_exact_size(ui.available_size(), egui::Sense::click_and_drag());
                if rect.width() <= 0. || rect.height() <= 0. {
                    return;
                }
                self.viewport_input(&response);
                let aspect = rect.width() / rect.height();
                let matrix = self.camera.view_projection(aspect);
                ui.painter()
                    .add(eframe::egui_wgpu::Callback::new_paint_callback(
                        rect,
                        GridCallback {
                            view_projection: matrix.to_cols_array_2d(),
                        },
                    ));
                ui.painter()
                    .add(eframe::egui_wgpu::Callback::new_paint_callback(
                        rect,
                        ModelCallback {
                            view_projection: matrix.to_cols_array_2d(),
                            size: [
                                (rect.width() * ctx.pixels_per_point()).round() as u32,
                                (rect.height() * ctx.pixels_per_point()).round() as u32,
                            ],
                            update: self.pending.take(),
                        },
                    ));
                let view = View {
                    matrix: matrix.as_dmat4(),
                    size: glam::DVec2::new(rect.width() as f64, rect.height() as f64),
                };
                let screen = |point| {
                    view.project(point)
                        .map(|p| rect.min + egui::vec2(p.x as f32, p.y as f32))
                };
                self.modify_overlay(ui, rect, view);
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
                            Color32::WHITE,
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
        assert!(app.pending.as_ref().is_some_and(|update| update.reset));
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
