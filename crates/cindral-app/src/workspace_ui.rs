//! Modeling workspace: menus, grouped tools, inspector and measurements.
use super::*;
use egui::{Align, Layout, RichText, Stroke};

const ACCENT: Color32 = Color32::from_rgb(88, 82, 214);
const INK: Color32 = Color32::from_rgb(39, 46, 61);
const RAIL: Color32 = Color32::from_rgb(247, 248, 251);

pub(crate) fn configure_style(ctx: &egui::Context) {
    let mut style = (*ctx.style()).clone();
    style.visuals = egui::Visuals::light();
    style.visuals.panel_fill = Color32::from_rgb(250, 251, 253);
    style.visuals.selection.bg_fill = Color32::from_rgb(235, 233, 253);
    style.visuals.selection.stroke = Stroke::new(1.0f32, ACCENT);
    style.visuals.override_text_color = Some(INK);
    style.visuals.widgets.noninteractive.bg_stroke =
        Stroke::new(1.0f32, Color32::from_rgb(231, 234, 240));
    style.visuals.widgets.noninteractive.corner_radius = egui::CornerRadius::same(8);
    style.visuals.widgets.inactive.weak_bg_fill = Color32::from_rgb(241, 243, 248);
    style.visuals.widgets.inactive.bg_fill = Color32::WHITE;
    style.visuals.widgets.inactive.bg_stroke =
        Stroke::new(1.0f32, Color32::from_rgb(228, 231, 239));
    style.visuals.widgets.hovered.weak_bg_fill = Color32::from_rgb(233, 232, 249);
    style.visuals.widgets.active.bg_fill = Color32::from_rgb(235, 233, 253);
    style.visuals.widgets.active.fg_stroke = Stroke::new(1.0f32, ACCENT);
    for widget in [
        &mut style.visuals.widgets.inactive,
        &mut style.visuals.widgets.hovered,
        &mut style.visuals.widgets.active,
    ] {
        widget.corner_radius = egui::CornerRadius::same(7);
    }
    style.spacing.item_spacing = egui::vec2(8., 9.);
    style.spacing.button_padding = egui::vec2(10., 7.);
    style.spacing.interact_size.y = 30.;
    style
        .text_styles
        .insert(egui::TextStyle::Body, egui::FontId::proportional(13.));
    style
        .text_styles
        .insert(egui::TextStyle::Button, egui::FontId::proportional(13.));
    ctx.set_style(style);
}

const DRAW: [(ToolKind, &str, &str); 5] = [
    (ToolKind::Line, "Line", "L"),
    (ToolKind::Rectangle, "Rectangle", "R"),
    (ToolKind::Circle, "Circle", "C"),
    (ToolKind::Arc, "Arc", "A"),
    (ToolKind::Polygon, "Polygon", "P"),
];
const MODIFY: [(ModifyKind, &str); 11] = [
    (ModifyKind::Select, "Select"),
    (ModifyKind::Eraser, "Eraser"),
    (ModifyKind::PushPull, "Push/Pull"),
    (ModifyKind::Move, "Move"),
    (ModifyKind::Copy, "Copy"),
    (ModifyKind::Rotate, "Rotate"),
    (ModifyKind::Scale, "Scale"),
    (ModifyKind::Offset, "Offset"),
    (ModifyKind::FollowMe, "Follow Me"),
    (ModifyKind::TapeMeasure, "Measure"),
    (ModifyKind::PaintBucket, "Paint"),
];

impl DrafterApp {
    pub(crate) fn workspace_header(&mut self, ctx: &egui::Context) {
        self.file_tick(ctx);
        egui::TopBottomPanel::top("workspace_header")
            .frame(
                egui::Frame::new()
                    .fill(Color32::WHITE)
                    .inner_margin(egui::Margin::symmetric(12, 6)),
            )
            .show(ctx, |ui| {
                egui::MenuBar::new().ui(ui, |ui| {
                    self.file_menu(ui);
                    ui.menu_button("Edit", |ui| {
                        if ui
                            .add_enabled(
                                self.history.undo_depth() > 0,
                                egui::Button::new("Undo   Ctrl+Z"),
                            )
                            .clicked()
                        {
                            self.history_step(false);
                            ui.close();
                        }
                        if ui
                            .add_enabled(
                                self.history.redo_depth() > 0,
                                egui::Button::new("Redo   Ctrl+Shift+Z"),
                            )
                            .clicked()
                        {
                            self.history_step(true);
                            ui.close();
                        }
                    });
                    ui.menu_button("Camera", |ui| {
                        ui.selectable_value(
                            &mut self.camera.projection,
                            Projection::Perspective,
                            "Perspective",
                        );
                        ui.selectable_value(
                            &mut self.camera.projection,
                            Projection::Orthographic,
                            "Parallel projection",
                        );
                        ui.checkbox(&mut self.show_grid, "Construction grid");
                        ui.separator();
                        for (label, view) in [
                            ("Isometric   0", StandardView::Iso),
                            ("Top   7", StandardView::Top),
                            ("Bottom", StandardView::Bottom),
                            ("Front   1", StandardView::Front),
                            ("Back", StandardView::Back),
                            ("Left", StandardView::Left),
                            ("Right   3", StandardView::Right),
                        ] {
                            if ui.button(label).clicked() {
                                self.camera.set_view(view);
                                ui.close();
                            }
                        }
                        if ui.button("Zoom extents   F").clicked() {
                            let size = ctx.available_rect().size();
                            self.extents(size.x / size.y.max(1.));
                            ui.close();
                        }
                    });
                    ui.menu_button("Draw", |ui| {
                        for (kind, label, shortcut) in DRAW {
                            if ui.button(format!("{label}   {shortcut}")).clicked() {
                                self.set_tool(kind);
                                ui.close();
                            }
                        }
                    });
                    ui.menu_button("Tools", |ui| {
                        for (kind, label) in MODIFY {
                            if ui.button(label).clicked() {
                                self.choose_modify(kind);
                                ui.close();
                            }
                        }
                    });
                    ui.menu_button("Window", |ui| {
                        ui.checkbox(&mut self.show_tray, "Model inspector");
                    });
                    ui.menu_button("Help", |ui| {
                        ui.label("Middle mouse: orbit");
                        ui.label("Shift + middle mouse: pan");
                        ui.label("Scroll: zoom around cursor");
                        ui.separator();
                        ui.label("Arrow keys: lock red / green / blue axis");
                        ui.label("Escape: cancel current operation");
                        ui.label("Type a dimension, then press Enter");
                    });

                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let state = self.document_save_label();
                        ui.label(
                            RichText::new(state)
                                .size(11.)
                                .color(if state == "Modified" {
                                    ACCENT
                                } else {
                                    Color32::from_rgb(105, 115, 134)
                                }),
                        )
                        .on_hover_text(self.file_status());
                        let title_width = if ctx.content_rect().width() < 900. {
                            100.
                        } else {
                            220.
                        };
                        ui.add_sized(
                            [title_width, 26.],
                            egui::Label::new(
                                RichText::new(self.document_title()).strong().color(INK),
                            )
                            .truncate(),
                        )
                        .on_hover_text(self.document_title());
                    });
                });
                ui.separator();
                ui.horizontal_wrapped(|ui| {
                    if ui
                        .add_enabled(
                            self.history.undo_depth() > 0,
                            egui::Button::new("Undo").frame(false),
                        )
                        .on_hover_text("Undo · Ctrl+Z")
                        .clicked()
                    {
                        self.history_step(false);
                    }
                    if ui
                        .add_enabled(
                            self.history.redo_depth() > 0,
                            egui::Button::new("Redo").frame(false),
                        )
                        .on_hover_text("Redo · Ctrl+Shift+Z")
                        .clicked()
                    {
                        self.history_step(true);
                    }
                    ui.separator();
                    if ui
                        .add(egui::Button::new("Fit model").frame(false))
                        .on_hover_text("Zoom extents · F")
                        .clicked()
                    {
                        let size = ctx.available_rect().size();
                        self.extents(size.x / size.y.max(1.));
                    }
                    ui.separator();
                    ui.label(
                        RichText::new("Work plane")
                            .size(11.)
                            .color(Color32::from_rgb(112, 120, 138)),
                    );
                    egui::Frame::new()
                        .fill(Color32::from_rgb(245, 245, 250))
                        .corner_radius(6)
                        .inner_margin(egui::Margin::symmetric(3, 0))
                        .show(ui, |ui| {
                            ui.spacing_mut().item_spacing.x = 2.;
                            ui.horizontal(|ui| {
                                for (label, normal) in
                                    [("XY", DVec3::Z), ("XZ", -DVec3::Y), ("YZ", DVec3::X)]
                                {
                                    if ui
                                        .selectable_label(self.work_plane.normal == normal, label)
                                        .on_hover_text(format!(
                                        "Draw on the {label} plane; cancels the current drawing"
                                    ))
                                        .clicked()
                                    {
                                        self.work_plane = DrawingPlane {
                                            origin: DVec3::ZERO,
                                            normal,
                                        };
                                        self.tool.cancel();
                                        if let Err(e) = self.tool.set_plane(self.work_plane) {
                                            self.error = Some(e.to_string());
                                        }
                                    }
                                }
                            });
                        });
                    if !self.modifying
                        && matches!(
                            self.tool.kind(),
                            ToolKind::Circle | ToolKind::Arc | ToolKind::Polygon
                        )
                    {
                        let mut count = self.tool.segments();
                        ui.label("Segments");
                        if ui
                            .add(egui::DragValue::new(&mut count).range(3..=256))
                            .changed()
                        {
                            if let Err(e) = self.tool.set_segments(count) {
                                self.error = Some(e.to_string());
                            }
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

                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui
                            .add(egui::Button::new("Save").frame(false))
                            .on_hover_text("Save document · Ctrl+S")
                            .clicked()
                        {
                            self.save_document();
                        }
                        if ui
                            .selectable_label(self.show_tray, "Inspector")
                            .on_hover_text("Show or hide model properties")
                            .clicked()
                        {
                            self.show_tray = !self.show_tray;
                        }
                    });
                });
            });
    }

    fn active_tool_label(&self) -> &'static str {
        if self.modifying {
            MODIFY
                .iter()
                .find(|(kind, _)| *kind == self.modify.kind())
                .map_or("Select", |(_, label)| *label)
        } else {
            DRAW.iter()
                .find(|(kind, _, _)| *kind == self.tool.kind())
                .map_or("Draw", |(_, label, _)| *label)
        }
    }

    pub(crate) fn tool_palette(&mut self, ctx: &egui::Context) {
        egui::SidePanel::left("tool_palette")
            .exact_width(140.)
            .resizable(false)
            .frame(
                egui::Frame::new()
                    .fill(RAIL)
                    .inner_margin(egui::Margin::symmetric(8, 12)),
            )
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    egui::Grid::new("full_tool_palette")
                        .num_columns(2)
                        .spacing([4., 4.])
                        .show(ui, |ui| {
                            let mut cell = 0;
                            for &(kind, label) in &MODIFY[..2] {
                                let shortcut = if kind == ModifyKind::Select {
                                    "Space"
                                } else {
                                    ""
                                };
                                if tool_button(
                                    ui,
                                    label,
                                    shortcut,
                                    self.modifying && self.modify.kind() == kind,
                                )
                                .clicked()
                                {
                                    self.choose_modify(kind);
                                }
                                cell += 1;
                                if cell % 2 == 0 {
                                    ui.end_row();
                                }
                            }
                            for (kind, label, shortcut) in DRAW {
                                if tool_button(
                                    ui,
                                    label,
                                    shortcut,
                                    !self.modifying && self.tool.kind() == kind,
                                )
                                .clicked()
                                {
                                    self.set_tool(kind);
                                }
                                cell += 1;
                                if cell % 2 == 0 {
                                    ui.end_row();
                                }
                            }
                            for &(kind, label) in &MODIFY[2..] {
                                if tool_button(
                                    ui,
                                    label,
                                    "",
                                    self.modifying && self.modify.kind() == kind,
                                )
                                .clicked()
                                {
                                    self.choose_modify(kind);
                                }
                                cell += 1;
                                if cell % 2 == 0 {
                                    ui.end_row();
                                }
                            }
                        });
                });
            });
    }

    pub(crate) fn model_inspector(&mut self, ctx: &egui::Context) {
        if !self.show_tray {
            return;
        }
        egui::SidePanel::right("default_tray")
            .default_width(264.)
            .width_range(250. ..=420.)
            .frame(
                egui::Frame::new()
                    .fill(Color32::from_rgb(250, 251, 253))
                    .inner_margin(20),
            )
            .show(ctx, |ui| {
                let tab_id = egui::Id::new("inspector_appearance");
                let mut appearance =
                    ctx.data(|data| data.get_temp::<bool>(tab_id).unwrap_or(false));
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut appearance, false, "Model");
                    ui.selectable_value(&mut appearance, true, "Appearance");
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui
                            .add(
                                egui::Button::new(RichText::new("×").size(18.))
                                    .frame(false)
                                    .min_size(egui::vec2(28., 28.)),
                            )
                            .on_hover_text(
                                "Close inspector · Reopen from the Inspector toolbar button",
                            )
                            .clicked()
                        {
                            self.show_tray = false;
                        }
                    });
                });
                ctx.data_mut(|data| data.insert_temp(tab_id, appearance));
                ui.add_space(12.);
                egui::ScrollArea::vertical()
                    .id_salt("tray_scroll")
                    .show(ui, |ui| {
                        if appearance {
                            self.presentation_panel(ui);
                        } else {
                            ui.label(RichText::new("Document").size(14.).strong().color(INK));
                            ui.weak(format!(
                                "{} edges · {} faces",
                                self.history.model().edges().len(),
                                self.history.model().faces().len()
                            ));
                            ui.add_space(8.);
                            if self.organization.selected.is_some() {
                                ui.strong("Group or component");
                                ui.weak("Properties in Outliner");
                                ui.add_space(12.);
                            } else if !self.selection.edges.is_empty()
                                || !self.selection.faces.is_empty()
                            {
                                ui.strong(format!(
                                    "{} edges · {} faces selected",
                                    self.selection.edges.len(),
                                    self.selection.faces.len()
                                ));
                                ui.add_space(12.);
                            }
                            if self.organization.selected.is_none()
                                && self.selection.edges.is_empty()
                                && self.selection.faces.is_empty()
                            {
                                ui.add(
                                    egui::Label::new("Select geometry to inspect or organize it.")
                                        .wrap(),
                                );
                                ui.add_space(8.);
                            }
                            self.organization_panel(ui);
                        }
                    });
            });
    }

    pub(crate) fn orientation_indicator(&self, ui: &egui::Ui, rect: egui::Rect) {
        let center = rect.right_top() + egui::vec2(-46., 46.);
        let matrix = self
            .camera
            .view_projection(rect.width() / rect.height().max(1.));
        let aspect = rect.width() / rect.height().max(1.);
        let p = ui.painter();
        let mut axes = [
            (glam::Vec3::X, "X", Color32::from_rgb(169, 91, 99)),
            (glam::Vec3::Y, "Y", Color32::from_rgb(73, 135, 105)),
            (glam::Vec3::Z, "Z", Color32::from_rgb(87, 117, 179)),
        ];
        axes.sort_by(|a, b| {
            matrix
                .transform_vector3(a.0)
                .z
                .total_cmp(&matrix.transform_vector3(b.0).z)
        });
        // Projection scales x by 1/aspect; compensate to keep the tripod square.
        let scale = matrix
            .transform_vector3(glam::Vec3::Z)
            .y
            .hypot(matrix.transform_vector3(glam::Vec3::X).y)
            .hypot(matrix.transform_vector3(glam::Vec3::Y).y)
            .max(0.001);
        for (axis, label, color) in axes {
            let v = matrix.transform_vector3(axis);
            let delta = egui::vec2(v.x * aspect, -v.y) / scale * 23.;
            p.line_segment([center, center + delta], Stroke::new(1.5f32, color));
            p.circle_filled(center + delta, 8., Color32::from_rgb(250, 251, 253));
            p.text(
                center + delta,
                egui::Align2::CENTER_CENTER,
                label,
                egui::FontId::proportional(11.),
                color,
            );
        }
        p.circle_filled(center, 2.5, Color32::from_rgb(119, 129, 147));
    }

    pub(crate) fn workspace_status(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::bottom("workspace_status")
            .frame(egui::Frame::new().fill(Color32::WHITE).inner_margin(egui::Margin::symmetric(16, 8)))
            .show(ctx, |ui| {
            ui.horizontal(|ui| {
                // Allocate dimensions first so contextual instructions cannot hide the input.
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let hint = if self.modifying {
                        match self.modify.kind() {
                            ModifyKind::Rotate => "e.g. 90deg",
                            ModifyKind::Scale => "e.g. 150%",
                            _ => "e.g. 2m",
                        }
                    } else {
                        match self.tool.kind() {
                            ToolKind::Rectangle => "e.g. 2m,3m",
                            ToolKind::Arc => "e.g. 90deg",
                            _ => "e.g. 2m",
                        }
                    };
                    let field = ui.add(egui::TextEdit::singleline(&mut self.measurement)
                        .desired_width(if ctx.content_rect().width() < 900. { 105. } else { 150. })
                        .hint_text(hint).frame(false).background_color(Color32::from_rgb(244, 245, 249)))
                        .on_hover_text("Type a precise dimension during an operation, then press Enter. Accepts m, cm, mm, ft and in.");
                    if field.lost_focus() && ctx.input(|i| i.key_pressed(Key::Enter)) {
                        self.apply_measurement();
                    }
                    ui.label(RichText::new("Dimensions").small());
                    ui.separator();
                    ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                        let prompt = if self.modifying {
                            match self.modify.kind() {
                                ModifyKind::Select => "Click to select · Shift to add · Drag a selection box",
                                ModifyKind::TapeMeasure => "Click two points to measure or create a guide",
                                _ => "Select geometry, then drag · Type a value + Enter · Esc cancels",
                            }
                        } else {
                            match self.tool.kind() {
                                ToolKind::Line => "Drag a line or click start, then end · Esc stops chain",
                                ToolKind::Rectangle => "Drag a rectangle or click opposite corners",
                                ToolKind::Circle => "Click center, then radius",
                                ToolKind::Arc => "Click center, start, then end direction",
                                ToolKind::Polygon => "Click center, then vertex",
                            }
                        };
                        ui.label(RichText::new(self.active_tool_label()).color(ACCENT));
                        ui.add(egui::Label::new(prompt).truncate()).on_hover_text(prompt);
                    });
                });
            });
            if self.error.is_some() || self.axis_lock.is_some() {
            ui.horizontal(|ui| {
                if let Some(error) = self.error.clone() {
                    if ui.small_button("Dismiss").clicked() { self.error = None; }
                    ui.add(egui::Label::new(RichText::new(&error).color(Color32::from_rgb(174, 43, 35))).truncate()).on_hover_text(error);
                } else {
                    ui.add(egui::Label::new(RichText::new(self.file_status()).small()).truncate()).on_hover_text(self.file_status());
                    if let Some(axis) = self.axis_lock {
                        ui.separator();
                        ui.colored_label(ACCENT, format!("{axis:?} axis locked"));
                    }
                }
            });
            }
        });
    }
}

/// Original vector tool symbols with accessible names and full hover labels.
fn tool_button(ui: &mut egui::Ui, label: &str, shortcut: &str, selected: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(60., 52.), egui::Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::SelectableLabel,
            ui.is_enabled(),
            selected,
            label,
        )
    });
    let fill = if selected {
        Color32::from_rgb(231, 228, 252)
    } else if response.hovered() {
        Color32::from_rgb(235, 237, 245)
    } else {
        Color32::TRANSPARENT
    };
    let p = ui.painter();
    p.rect_filled(rect, 8., fill);
    if response.has_focus() {
        p.rect_stroke(
            rect.shrink(0.5),
            4.,
            Stroke::new(1.0f32, ACCENT),
            egui::StrokeKind::Inside,
        );
    }
    let c = rect.center_top() + egui::vec2(0., 16.);
    let icon_color = if selected { ACCENT } else { INK };
    let stroke = Stroke::new(1.7f32, icon_color);
    let pos = |x, y| c + egui::vec2(x, y);
    match label {
        "Line" => {
            p.line_segment(
                [pos(-9., 8.), pos(9., -8.)],
                Stroke::new(2.5f32, icon_color),
            );
        }
        "Rectangle" | "Offset" | "Scale" | "Copy" => {
            p.rect_stroke(
                egui::Rect::from_center_size(c, egui::vec2(19., 15.)),
                0.,
                stroke,
                egui::StrokeKind::Inside,
            );
            if label == "Scale" {
                p.arrow(pos(-4., 4.), egui::vec2(13., -12.), stroke);
            } else if label != "Rectangle" {
                p.rect_stroke(
                    egui::Rect::from_center_size(
                        if label == "Offset" { c } else { pos(5., -5.) },
                        egui::vec2(11., 9.),
                    ),
                    0.,
                    stroke,
                    egui::StrokeKind::Inside,
                );
            }
        }
        "Follow Me" => {
            p.line_segment([pos(-9., 8.), pos(-9., -6.)], stroke);
            p.arrow(pos(-9., -6.), egui::vec2(18., 0.), stroke);
            p.circle_stroke(pos(-9., 8.), 4., stroke);
        }
        "Circle" | "Rotate" => {
            p.circle_stroke(c, 9., stroke);
            if label != "Circle" {
                p.arrow(pos(8., -4.), egui::vec2(-4., -7.), stroke);
            }
        }
        "Arc" => {
            let points = (0..=16)
                .map(|i| {
                    let t = i as f32 / 16. * std::f32::consts::PI;
                    pos(t.cos() * 10., -t.sin() * 10. + 5.)
                })
                .collect();
            p.add(egui::Shape::line(points, stroke));
        }
        "Polygon" => {
            let points = (0..=6)
                .map(|i| {
                    let t = i as f32 / 6. * std::f32::consts::TAU;
                    pos(t.cos() * 10., t.sin() * 10.)
                })
                .collect();
            p.add(egui::Shape::line(points, stroke));
        }
        "Select" => {
            p.add(egui::Shape::convex_polygon(
                vec![
                    pos(-5., -10.),
                    pos(-5., 9.),
                    pos(0., 4.),
                    pos(5., 11.),
                    pos(8., 9.),
                    pos(3., 2.),
                    pos(10., 1.),
                ],
                icon_color,
                stroke,
            ));
        }
        "Move" => {
            for (x, y) in [(0., -11.), (11., 0.), (0., 11.), (-11., 0.)] {
                p.arrow(c, egui::vec2(x, y), stroke);
            }
        }
        "Push/Pull" => {
            p.rect_stroke(
                egui::Rect::from_center_size(pos(0., 5.), egui::vec2(20., 10.)),
                0.,
                stroke,
                egui::StrokeKind::Inside,
            );
            p.arrow(
                pos(0., 5.),
                egui::vec2(0., -17.),
                Stroke::new(2.0f32, icon_color),
            );
        }
        "Eraser" => {
            p.add(egui::Shape::convex_polygon(
                vec![pos(-10., 3.), pos(1., -9.), pos(10., -1.), pos(-1., 11.)],
                Color32::from_rgb(172, 181, 201),
                stroke,
            ));
        }
        "Measure" => {
            p.rect_filled(
                egui::Rect::from_center_size(c, egui::vec2(21., 13.)),
                3.,
                Color32::from_rgb(172, 181, 201),
            );
            for x in [-6., 0., 6.] {
                p.line_segment([pos(x, -6.), pos(x, 0.)], stroke);
            }
        }
        "Paint" => {
            p.add(egui::Shape::convex_polygon(
                vec![pos(-9., -2.), pos(0., -10.), pos(10., 0.), pos(1., 9.)],
                Color32::from_rgb(172, 181, 201),
                stroke,
            ));
            p.line_segment(
                [pos(-11., 11.), pos(11., 11.)],
                Stroke::new(3.0f32, icon_color),
            );
        }
        _ => {}
    }
    p.text(
        rect.center_bottom() - egui::vec2(0., 9.),
        egui::Align2::CENTER_CENTER,
        label,
        egui::FontId::proportional(11.),
        icon_color,
    );
    response.on_hover_text(if shortcut.is_empty() {
        label.to_owned()
    } else {
        format!("{label} · {shortcut}")
    })
}
