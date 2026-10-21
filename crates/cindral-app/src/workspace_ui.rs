//! Compact modeling workspace: menus, tool palette, tray and measurements.
use super::*;
use egui::{Align, Layout, RichText, Stroke};

pub(crate) fn configure_style(ctx: &egui::Context) {
    let mut style = (*ctx.style()).clone();
    style.visuals = egui::Visuals::light();
    style.visuals.panel_fill = Color32::from_rgb(245, 246, 247);
    style.visuals.selection.bg_fill = Color32::from_rgb(205, 226, 247);
    style.visuals.selection.stroke = Stroke::new(1.0f32, Color32::from_rgb(32, 92, 150));
    style.spacing.item_spacing = egui::vec2(7., 7.);
    style.spacing.button_padding = egui::vec2(8., 5.);
    style.spacing.interact_size.y = 26.;
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
        egui::TopBottomPanel::top("workspace_header").show(ctx, |ui| {
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
                    ui.checkbox(&mut self.show_tray, "Default Tray");
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
                    ui.label(
                        RichText::new("CINDRAL DRAFTER")
                            .strong()
                            .color(Color32::from_rgb(60, 77, 94)),
                    );
                });
            });
            ui.separator();
            ui.horizontal_wrapped(|ui| {
                if ui
                    .add_enabled(self.history.undo_depth() > 0, egui::Button::new("Undo"))
                    .on_hover_text("Undo · Ctrl+Z")
                    .clicked()
                {
                    self.history_step(false);
                }
                if ui
                    .add_enabled(self.history.redo_depth() > 0, egui::Button::new("Redo"))
                    .on_hover_text("Redo · Ctrl+Shift+Z")
                    .clicked()
                {
                    self.history_step(true);
                }
                ui.separator();
                if ui
                    .button("Fit model")
                    .on_hover_text("Zoom extents · F")
                    .clicked()
                {
                    let size = ctx.available_rect().size();
                    self.extents(size.x / size.y.max(1.));
                }
                ui.separator();
                ui.label("Plane");
                for (label, normal) in [("XY", DVec3::Z), ("XZ", -DVec3::Y), ("YZ", DVec3::X)] {
                    if ui
                        .selectable_label(self.work_plane.normal == normal, label)
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
                if !self.show_tray && ui.button("Show tray").clicked() {
                    self.show_tray = true;
                }
            });
        });
    }

    pub(crate) fn tool_palette(&mut self, ctx: &egui::Context) {
        egui::SidePanel::left("tool_palette")
            .exact_width(110.)
            .resizable(false)
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    ui.weak("TOOLS");
                    egui::Grid::new("palette")
                        .num_columns(2)
                        .spacing([4., 4.])
                        .show(ui, |ui| {
                            let mut cell = 0;
                            for (kind, label) in MODIFY[..2].iter().copied() {
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
                            for (kind, label) in MODIFY[2..].iter().copied() {
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
                    ui.separator();
                    ui.weak("NAVIGATE");
                    if ui
                        .button("Isometric")
                        .on_hover_text("Isometric view · 0")
                        .clicked()
                    {
                        self.camera.set_view(StandardView::Iso);
                    }
                    if ui
                        .button("Top view")
                        .on_hover_text("Top view · 7")
                        .clicked()
                    {
                        self.camera.set_view(StandardView::Top);
                    }
                    ui.small("MMB orbit");
                    ui.small("Shift + MMB pan");
                });
            });
    }

    pub(crate) fn workspace_status(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::bottom("workspace_status").show(ctx, |ui| {
            ui.horizontal(|ui| {
                // Reserve the measurement field first so long prompts cannot push it offscreen.
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let hint = if self.modifying { match self.modify.kind() { ModifyKind::Rotate => "angle / deg", ModifyKind::Scale => "factor / %", _ => "distance" } } else { match self.tool.kind() { ToolKind::Rectangle => "width,height", ToolKind::Arc => "degrees", _ => "length / radius" } };
                    let field = ui.add(egui::TextEdit::singleline(&mut self.measurement).desired_width(140.).hint_text(hint));
                    if field.lost_focus() && ctx.input(|i| i.key_pressed(Key::Enter)) { self.apply_measurement(); }
                    ui.label("Measurements"); ui.separator();
                    ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                        let prompt = if self.modifying { match self.modify.kind() { ModifyKind::Select => "Select geometry · Shift adds · Drag a selection box", ModifyKind::TapeMeasure => "Measure: click two points to measure or create a guide", _ => "Select geometry, then drag · Type a value + Enter · Esc cancels" } } else { match self.tool.kind() { ToolKind::Line => "Line: click start, then end · Esc stops chain", ToolKind::Rectangle => "Rectangle: click first corner, then opposite corner", ToolKind::Circle => "Circle: click center, then radius", ToolKind::Arc => "Arc: click center, start, then end direction", ToolKind::Polygon => "Polygon: click center, then vertex" } };
                        ui.add(egui::Label::new(prompt).truncate());
                    });
                });
            });
            if let Some(error) = &self.error { ui.colored_label(Color32::from_rgb(174, 43, 35), error); }
            else { ui.horizontal(|ui| {
                ui.add(egui::Label::new(self.file_status()).truncate());
                if let Some(axis) = self.axis_lock { ui.separator(); ui.label(format!("{axis:?} axis locked")); }
            }); }
        });
    }
}

/// Original vector tool symbols, with text captions and full hover labels.
fn tool_button(ui: &mut egui::Ui, label: &str, shortcut: &str, selected: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(47., 47.), egui::Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::SelectableLabel,
            ui.is_enabled(),
            selected,
            label,
        )
    });
    let fill = if selected {
        Color32::from_rgb(205, 226, 247)
    } else if response.hovered() {
        Color32::from_rgb(225, 230, 235)
    } else {
        Color32::TRANSPARENT
    };
    let p = ui.painter();
    p.rect_filled(rect, 4., fill);
    if selected || response.has_focus() {
        p.rect_stroke(
            rect.shrink(0.5),
            4.,
            Stroke::new(1.0f32, Color32::from_rgb(80, 136, 190)),
            egui::StrokeKind::Inside,
        );
    }
    let c = rect.center_top() + egui::vec2(0., 16.);
    let stroke = Stroke::new(1.7f32, Color32::from_rgb(62, 78, 91));
    let pos = |x, y| c + egui::vec2(x, y);
    match label {
        "Line" => {
            p.line_segment(
                [pos(-9., 8.), pos(9., -8.)],
                Stroke::new(2.5f32, Color32::from_rgb(188, 66, 51)),
            );
        }
        "Rectangle" | "Offset" | "Scale" | "Copy" => {
            p.rect_stroke(
                egui::Rect::from_center_size(c, egui::vec2(19., 15.)),
                0.,
                stroke,
                egui::StrokeKind::Inside,
            );
            if label != "Rectangle" {
                p.rect_stroke(
                    egui::Rect::from_center_size(pos(4., -4.), egui::vec2(11., 9.)),
                    0.,
                    stroke,
                    egui::StrokeKind::Inside,
                );
            }
        }
        "Circle" | "Rotate" | "Follow Me" => {
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
                Color32::from_rgb(70, 83, 95),
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
                Stroke::new(2.0f32, Color32::from_rgb(188, 66, 51)),
            );
        }
        "Eraser" => {
            p.add(egui::Shape::convex_polygon(
                vec![pos(-10., 3.), pos(1., -9.), pos(10., -1.), pos(-1., 11.)],
                Color32::from_rgb(232, 169, 165),
                stroke,
            ));
        }
        "Measure" => {
            p.rect_filled(
                egui::Rect::from_center_size(c, egui::vec2(21., 13.)),
                3.,
                Color32::from_rgb(219, 177, 58),
            );
            for x in [-6., 0., 6.] {
                p.line_segment([pos(x, -6.), pos(x, 0.)], stroke);
            }
        }
        "Paint" => {
            p.add(egui::Shape::convex_polygon(
                vec![pos(-9., -2.), pos(0., -10.), pos(10., 0.), pos(1., 9.)],
                Color32::from_rgb(133, 176, 201),
                stroke,
            ));
            p.line_segment(
                [pos(-11., 11.), pos(11., 11.)],
                Stroke::new(3.0f32, Color32::from_rgb(73, 134, 174)),
            );
        }
        _ => {}
    }
    p.text(
        rect.center_bottom() - egui::vec2(0., 8.),
        egui::Align2::CENTER_CENTER,
        label,
        egui::FontId::proportional(9.),
        Color32::from_rgb(59, 67, 76),
    );
    response.on_hover_text(if shortcut.is_empty() {
        label.to_owned()
    } else {
        format!("{label} · {shortcut}")
    })
}
