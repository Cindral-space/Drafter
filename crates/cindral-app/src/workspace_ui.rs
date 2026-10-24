//! Modeling workspace: menus, grouped tools, inspector and measurements.
use super::*;
use egui::{Align, Layout, RichText, Stroke};

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum InspectorTab {
    Model,
    Appearance,
    Recovery,
}

pub(crate) fn configure_style(ctx: &egui::Context) {
    ctx.options_mut(|o| o.fallback_theme = egui::Theme::Dark);
    ctx.all_styles_mut(|style| {
        let dark = style.visuals.dark_mode;
        let v = &mut style.visuals;
        let (ink, muted, accent, surface, panel, field, selected, border, error) = if dark {
            (
                rgb(235, 237, 245),
                rgb(188, 194, 210),
                rgb(188, 180, 255),
                rgb(31, 34, 43),
                rgb(37, 40, 50),
                rgb(44, 48, 60),
                rgb(65, 57, 100),
                rgb(135, 142, 160),
                rgb(255, 165, 156),
            )
        } else {
            (
                rgb(39, 46, 61),
                rgb(82, 91, 110),
                rgb(88, 65, 184),
                Color32::WHITE,
                rgb(250, 251, 253),
                rgb(241, 243, 248),
                rgb(231, 228, 252),
                rgb(124, 132, 149),
                rgb(157, 35, 29),
            )
        };
        v.override_text_color = Some(ink);
        v.weak_text_color = Some(muted);
        v.panel_fill = panel;
        v.window_fill = surface;
        v.extreme_bg_color = field;
        v.faint_bg_color = field;
        v.code_bg_color = field;
        v.selection.bg_fill = selected;
        v.selection.stroke = Stroke::new(1_f32, accent);
        v.hyperlink_color = accent;
        v.error_fg_color = error;
        v.warn_fg_color = if dark {
            rgb(255, 210, 125)
        } else {
            rgb(120, 72, 0)
        };
        v.disabled_alpha = 0.85;
        for widget in [
            &mut v.widgets.noninteractive,
            &mut v.widgets.inactive,
            &mut v.widgets.hovered,
            &mut v.widgets.active,
            &mut v.widgets.open,
        ] {
            widget.bg_fill = surface;
            widget.weak_bg_fill = field;
            widget.fg_stroke = Stroke::new(1_f32, ink);
            widget.bg_stroke = Stroke::new(1_f32, border);
            widget.corner_radius = egui::CornerRadius::same(7);
        }
        v.widgets.hovered.weak_bg_fill = selected;
        v.widgets.active.weak_bg_fill = selected;
        v.widgets.active.bg_fill = selected;
        style.spacing.item_spacing = egui::vec2(8., 9.);
        style.spacing.button_padding = egui::vec2(10., 7.);
        style.spacing.interact_size.y = 30.;
        style
            .text_styles
            .insert(egui::TextStyle::Body, egui::FontId::proportional(13.));
        style
            .text_styles
            .insert(egui::TextStyle::Button, egui::FontId::proportional(13.));
    });
}
fn rgb(r: u8, g: u8, b: u8) -> Color32 {
    Color32::from_rgb(r, g, b)
}

// Shared by root dropdowns and nested menus, without changing the menu bar.
fn dropdown_style(style: &mut egui::Style) {
    egui::containers::menu::menu_style(style);
    style.spacing.button_padding = egui::vec2(10., 5.);
    style.spacing.interact_size.y = 30.;
    style.spacing.item_spacing = egui::vec2(8., 2.);
    style.spacing.menu_width = 280.;
    style.visuals.window_corner_radius = egui::CornerRadius::same(8);
}

pub(crate) fn menu_section(ui: &mut egui::Ui, title: &str) {
    ui.label(
        RichText::new(title)
            .size(11.)
            .color(ui.visuals().weak_text_color()),
    );
}

const DRAW: [(ToolKind, &str); 5] = [
    (ToolKind::Line, "Line"),
    (ToolKind::Rectangle, "Rectangle"),
    (ToolKind::Circle, "Circle"),
    (ToolKind::Arc, "Arc"),
    (ToolKind::Polygon, "Polygon"),
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
                    .fill(ctx.style().visuals.window_fill)
                    .inner_margin(egui::Margin::symmetric(12, 6)),
            )
            .show(ctx, |ui| {
                egui::MenuBar::new()
                    .config(egui::containers::menu::MenuConfig::new().style(dropdown_style))
                    .ui(ui, |ui| {
                        self.file_menu(ui);
                        ui.menu_button("Edit", |ui| {
                            ui.set_min_width(224.);
                            if ui
                                .add_enabled(
                                    self.history.undo_depth() > 0,
                                    egui::Button::new("Undo")
                                        .shortcut_text(shortcuts::hint(shortcuts::Action::Undo)),
                                )
                                .clicked()
                            {
                                self.history_step(false);
                                ui.close();
                            }
                            if ui
                                .add_enabled(
                                    self.history.redo_depth() > 0,
                                    egui::Button::new("Redo")
                                        .shortcut_text(shortcuts::hint(shortcuts::Action::Redo)),
                                )
                                .clicked()
                            {
                                self.history_step(true);
                                ui.close();
                            }
                        });
                        ui.menu_button("Camera", |ui| {
                            ui.set_min_width(224.);
                            menu_section(ui, "Projection");
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
                            menu_section(ui, "Standard views");
                            for (label, shortcut, view) in [
                                ("Isometric", "0", StandardView::Iso),
                                ("Top", "7", StandardView::Top),
                                ("Bottom", "", StandardView::Bottom),
                                ("Front", "1", StandardView::Front),
                                ("Back", "", StandardView::Back),
                                ("Left", "", StandardView::Left),
                                ("Right", "3", StandardView::Right),
                            ] {
                                if ui
                                    .add(egui::Button::new(label).shortcut_text(
                                        if shortcut.is_empty() {
                                            ""
                                        } else {
                                            shortcuts::hint(shortcuts::Action::View(view))
                                        },
                                    ))
                                    .clicked()
                                {
                                    self.camera.set_view(view);
                                    ui.close();
                                }
                            }
                            ui.separator();
                            ui.checkbox(&mut self.show_grid, "Construction grid");
                            if ui
                                .add(
                                    egui::Button::new("Fit model")
                                        .shortcut_text(shortcuts::hint(shortcuts::Action::Fit)),
                                )
                                .clicked()
                            {
                                let size = ctx.available_rect().size();
                                self.extents(size.x / size.y.max(1.));
                                ui.close();
                            }
                        });
                        ui.menu_button("Draw", |ui| {
                            ui.set_min_width(224.);
                            for (kind, label) in DRAW {
                                let shortcut = shortcuts::hint(shortcuts::Action::Draw(kind));
                                if ui
                                    .add(
                                        egui::Button::new(label)
                                            .shortcut_text(shortcut)
                                            .selected(!self.modifying && self.tool.kind() == kind),
                                    )
                                    .clicked()
                                {
                                    self.set_tool(kind);
                                    ui.close();
                                }
                            }
                        });
                        ui.menu_button("Tools", |ui| {
                            ui.set_min_width(224.);
                            for (index, (kind, label)) in MODIFY.into_iter().enumerate() {
                                if index == 0 {
                                    menu_section(ui, "Selection");
                                } else if index == 2 || index == 9 {
                                    ui.separator();
                                    menu_section(
                                        ui,
                                        if index == 2 { "Modeling" } else { "Utilities" },
                                    );
                                }
                                let shortcut = shortcuts::hint(shortcuts::Action::Modify(kind));
                                if ui
                                    .add(
                                        egui::Button::new(label)
                                            .shortcut_text(shortcut)
                                            .selected(self.modifying && self.modify.kind() == kind),
                                    )
                                    .clicked()
                                {
                                    self.choose_modify(kind);
                                    ui.close();
                                }
                            }
                        });
                        ui.menu_button("View", |ui| {
                            ui.menu_button("Preferences", |ui| {
                                let mut theme = self.png.theme();
                                for (value, label) in [
                                    (theme::Preference::Light, "Light"),
                                    (theme::Preference::Dark, "Dark"),
                                    (theme::Preference::System, "System"),
                                ] {
                                    if ui.selectable_value(&mut theme, value, label).changed() {
                                        theme.apply(ctx);
                                        if let Err(error) = self.png.save_theme(theme) {
                                            self.error = Some(format!(
                                                "Theme preference could not be saved: {error}"
                                            ));
                                        }
                                    }
                                }
                            });
                        });
                        ui.menu_button("Window", |ui| {
                            ui.set_min_width(224.);
                            ui.checkbox(&mut self.show_tray, "Model inspector");
                        });
                        ui.menu_button("Help", |ui| {
                            if ui
                                .add(
                                    egui::Button::new("Keyboard shortcuts")
                                        .shortcut_text(shortcuts::hint(shortcuts::Action::Help)),
                                )
                                .clicked()
                            {
                                self.shortcuts.open = true;
                                ui.close();
                            }
                            menu_section(ui, "Navigate");
                            ui.label("Middle mouse: orbit");
                            ui.label("Shift + middle mouse: pan");
                            ui.label("Scroll: zoom around cursor");
                            ui.separator();
                            menu_section(ui, "Model");
                            ui.label("Arrow keys: lock red / green / blue axis");
                            ui.label("Escape: cancel current operation");
                            ui.label("Type a dimension, then press Enter");
                        });

                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if self.files.import_summary.is_some()
                                && ui.link("Import report").clicked()
                            {
                                self.files.import_report_open = true;
                            }
                            let state = self.document_save_label();
                            ui.label(RichText::new(state).size(11.).color(
                                if state == "Modified" {
                                    ui.visuals().selection.stroke.color
                                } else {
                                    ui.visuals().weak_text_color()
                                },
                            ))
                            .on_hover_text(self.file_status());
                            // Reserve room for every menu; long filenames yield first.
                            let title_width = (ui.available_width() - 12.).clamp(0., 240.);
                            ui.add_sized(
                                [title_width, 26.],
                                egui::Label::new(
                                    RichText::new(self.document_title())
                                        .strong()
                                        .color(ui.visuals().text_color()),
                                )
                                .halign(Align::RIGHT)
                                .truncate(),
                            )
                            .on_hover_text(self.document_title());
                        });
                    });
                ui.separator();
                let compact = ui.available_width() < 900.;
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.;
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui
                            .add(egui::Button::new("Save").fill(ui.visuals().selection.bg_fill))
                            .on_hover_text(shortcuts::tooltip(
                                shortcuts::MAP
                                    .iter()
                                    .find(|e| e.action == shortcuts::Action::Save)
                                    .unwrap(),
                            ))
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
                        ui.separator();
                        ui.allocate_ui_with_layout(
                            ui.available_size(),
                            Layout::left_to_right(Align::Center),
                            |ui| {
                                if ui
                                    .add_enabled(
                                        self.history.undo_depth() > 0,
                                        egui::Button::new("Undo").frame(false),
                                    )
                                    .on_hover_text(shortcuts::tooltip(
                                        shortcuts::MAP
                                            .iter()
                                            .find(|e| e.action == shortcuts::Action::Undo)
                                            .unwrap(),
                                    ))
                                    .clicked()
                                {
                                    self.history_step(false);
                                }
                                if ui
                                    .add_enabled(
                                        self.history.redo_depth() > 0,
                                        egui::Button::new("Redo").frame(false),
                                    )
                                    .on_hover_text(shortcuts::tooltip(
                                        shortcuts::MAP
                                            .iter()
                                            .find(|e| e.action == shortcuts::Action::Redo)
                                            .unwrap(),
                                    ))
                                    .clicked()
                                {
                                    self.history_step(true);
                                }
                                ui.separator();
                                if ui
                                    .add(egui::Button::new("Fit model").frame(false))
                                    .on_hover_text(shortcuts::tooltip(
                                        shortcuts::MAP
                                            .iter()
                                            .find(|e| e.action == shortcuts::Action::Fit)
                                            .unwrap(),
                                    ))
                                    .clicked()
                                {
                                    let size = ctx.available_rect().size();
                                    self.extents(size.x / size.y.max(1.));
                                }
                                ui.separator();
                                ui.label(
                                    RichText::new(self.active_tool_label())
                                        .strong()
                                        .color(ui.visuals().selection.stroke.color),
                                );
                                if !compact {
                                    self.header_tool_options(ui);
                                }
                            },
                        );
                    });
                });
                if compact && (!self.modifying || self.modify.kind() == ModifyKind::PaintBucket) {
                    ui.separator();
                    ui.horizontal_wrapped(|ui| self.header_tool_options(ui));
                }
            });
    }

    fn header_tool_options(&mut self, ui: &mut egui::Ui) {
        if !self.modifying {
            ui.label(
                RichText::new("Work plane")
                    .size(11.)
                    .color(ui.visuals().weak_text_color()),
            );
            egui::Frame::new()
                .fill(ui.visuals().faint_bg_color)
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
                                && self.work_plane.normal != normal
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
            if matches!(
                self.tool.kind(),
                ToolKind::Circle | ToolKind::Arc | ToolKind::Polygon
            ) {
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
        }
        if self.modifying && self.modify.kind() == ModifyKind::PaintBucket {
            ui.label("Paint color");
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
    }

    fn active_tool_label(&self) -> &'static str {
        if let Some(mode) = self.shortcuts.navigation {
            return match mode {
                shortcuts::Action::Orbit => "Orbit",
                shortcuts::Action::Pan => "Pan",
                _ => "Zoom",
            };
        }
        if self.modifying {
            MODIFY
                .iter()
                .find(|(kind, _)| *kind == self.modify.kind())
                .map_or("Select", |(_, label)| *label)
        } else {
            DRAW.iter()
                .find(|(kind, _)| *kind == self.tool.kind())
                .map_or("Draw", |(_, label)| *label)
        }
    }

    pub(crate) fn paint_tool_cursor(&self, ui: &egui::Ui, response: &egui::Response) {
        // Keep the system pointer outside the canvas and over other UI.
        if !response.hovered() && !(response.contains_pointer() && response.dragged()) {
            return;
        }
        if self.shortcuts.navigation.is_some() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
            return;
        }
        let (position, orbiting, shift) = ui.input(|i| {
            (
                i.pointer.hover_pos(),
                i.pointer.button_down(egui::PointerButton::Middle),
                i.modifiers.shift,
            )
        });
        let Some(position) = position.filter(|p| response.rect.contains(*p)) else {
            return;
        };
        let label = if orbiting {
            if shift {
                ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
                return;
            }
            ui.ctx().set_cursor_icon(egui::CursorIcon::Move);
            "Rotate"
        } else if self.organization.drag.is_some() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
            return;
        } else if self.section_cursor_active() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
            return;
        } else if self.modifying && self.modify.kind() == ModifyKind::Select {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Default);
            return;
        } else {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
            self.active_tool_label()
        };
        let painter = ui.painter().with_clip_rect(response.rect);
        let center = position + egui::vec2(23., 23.);
        if self.modifying && self.modify.kind() == ModifyKind::Rotate && self.rotate.copy {
            painter.text(
                center + egui::vec2(12., -12.),
                egui::Align2::CENTER_CENTER,
                "+",
                egui::FontId::proportional(18.),
                self.viewport_ink(),
            );
        }
        // A light outline keeps the tool readable against both sky and geometry.
        paint_tool_icon(
            &painter,
            center,
            label,
            Stroke::new(4.5_f32, Color32::WHITE),
        );
        paint_tool_icon(
            &painter,
            center,
            label,
            Stroke::new(1.7_f32, rgb(39, 46, 61)),
        );
    }

    pub(crate) fn tool_palette(&mut self, ctx: &egui::Context) {
        egui::SidePanel::left("tool_palette")
            .exact_width(132.)
            .resizable(false)
            .frame(
                egui::Frame::new()
                    .fill(ctx.style().visuals.panel_fill)
                    .inner_margin(egui::Margin::symmetric(6, 10)),
            )
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    egui::Grid::new("full_tool_palette")
                        .num_columns(2)
                        .spacing([2., 3.])
                        .show(ui, |ui| {
                            let mut cell = 0;
                            for &(kind, label) in &MODIFY[..2] {
                                if tool_button(
                                    ui,
                                    label,
                                    shortcuts::Action::Modify(kind),
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
                            for (kind, label) in DRAW {
                                if tool_button(
                                    ui,
                                    label,
                                    shortcuts::Action::Draw(kind),
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
                                    shortcuts::Action::Modify(kind),
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
            .default_width(320.)
            .width_range(300. ..=460.)
            .frame(
                egui::Frame::new()
                    .fill(ctx.style().visuals.panel_fill)
                    .inner_margin(20),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.;
                    ui.spacing_mut().button_padding.x = 4.;
                    ui.selectable_value(&mut self.inspector_tab, InspectorTab::Model, "Model");
                    ui.selectable_value(
                        &mut self.inspector_tab,
                        InspectorTab::Appearance,
                        "Appearance",
                    );
                    let count = self.recovery_count();
                    let label = if count == 0 {
                        "Recovery".into()
                    } else {
                        format!("Recovery ({count})")
                    };
                    ui.selectable_value(&mut self.inspector_tab, InspectorTab::Recovery, label);
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
                ui.add_space(12.);
                egui::ScrollArea::vertical()
                    .id_salt(("tray_scroll", self.inspector_tab))
                    .show(ui, |ui| {
                        if self.inspector_tab == InspectorTab::Recovery {
                            self.recovery_panel(ui);
                        } else if self.inspector_tab == InspectorTab::Appearance {
                            self.presentation_panel(ui);
                        } else {
                            ui.label(
                                RichText::new("Document")
                                    .size(14.)
                                    .strong()
                                    .color(ui.visuals().text_color()),
                            );
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
            p.circle_filled(center + delta, 8., ui.visuals().panel_fill);
            p.text(
                center + delta,
                egui::Align2::CENTER_CENTER,
                label,
                egui::FontId::proportional(11.),
                ui.visuals().text_color(),
            );
        }
        p.circle_filled(center, 2.5, ui.visuals().weak_text_color());
    }

    pub(crate) fn workspace_status(&mut self, ctx: &egui::Context) {
        let readout = self.measurement_readout();
        self.measurements
            .update(readout, self.presentation.measurements);
        egui::TopBottomPanel::bottom("workspace_status")
            .frame(egui::Frame::new().fill(ctx.style().visuals.window_fill).inner_margin(egui::Margin::symmetric(16, 8)))
            .show(ctx, |ui| {
            if let Some(notice)=self.shortcuts.notice.clone() {ui.horizontal(|ui| {ui.label(&notice);if ui.small_button("Dismiss").clicked(){self.shortcuts.notice=None;}});}
            ui.horizontal(|ui| {
                // Allocate dimensions first so contextual instructions cannot hide the input.
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let field = ui.add(egui::TextEdit::singleline(&mut self.measurements.text)
                        .desired_width(if ctx.content_rect().width() < 900. { 105. } else { 150. })
                        .hint_text(&self.measurements.display).frame(true).background_color(if self.measurements.invalid { theme::error_background(ui.visuals()) } else { ui.visuals().extreme_bg_color }))
                        .on_hover_text("Type a precise dimension during an operation, then press Enter. Accepts m, cm, mm, ft and in.");
                    if field.changed() { self.measurements.phase = cindral_tools::measurements::BoxPhase::Typing; self.measurements.invalid = false; }
                    if (field.has_focus() || field.lost_focus()) && ctx.input(|i| shortcuts::pressed(i, shortcuts::Action::Cancel)) {
                        let cleared=self.measurements.escape();
                        if cleared {field.surrender_focus();shortcuts::consume(ctx,shortcuts::Action::Cancel);}
                        if !cleared && self.modifying && self.modify.kind()==ModifyKind::Rotate {self.cancel_modify();field.surrender_focus();}
                        self.error = None;
                    }
                    if field.lost_focus() && ctx.input(|i| shortcuts::pressed(i, shortcuts::Action::Apply)) {
                        self.apply_measurement();
                    }
                    ui.label(RichText::new(readout.label).small()).on_hover_text("Right-click for document units and precision").context_menu(|ui| {
                        let before = self.presentation.measurements;
                        for unit in [cindral_core::LengthUnit::Meter, cindral_core::LengthUnit::Centimeter, cindral_core::LengthUnit::Millimeter, cindral_core::LengthUnit::Inch, cindral_core::LengthUnit::Foot] {
                            ui.radio_value(&mut self.presentation.measurements.unit, unit, unit.suffix());
                        }
                        ui.add(egui::Slider::new(&mut self.presentation.measurements.precision, 0..=9).text("Decimals"));
                        ui.add(egui::Slider::new(&mut self.presentation.measurements.angle_snap_degrees, 1..=90).text("Angle snap (degrees)"));
                        ui.checkbox(&mut self.presentation.measurements.decimal_comma, "Decimal comma (; separates values)");
                        if before != self.presentation.measurements { self.presentation_changed(); }
                    });
                    ui.separator();
                    ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                        let prompt = if self.shortcuts.navigation.is_some() {"Drag to navigate · Escape returns to Select"} else if self.modifying {
                            match self.modify.kind() {
                                ModifyKind::Select => "Click to select · Shift to add · Drag a selection box",
                                ModifyKind::Rotate => self.rotate_hint(),
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
                        ui.label(RichText::new(self.active_tool_label()).color(ui.visuals().selection.stroke.color));
                        ui.add(egui::Label::new(prompt).truncate()).on_hover_text(prompt);
                    });
                });
            });
            if self.error.is_some() || self.axis_lock.is_some() || self.files.import_summary.is_some() {
            ui.horizontal(|ui| {
                if let Some(error) = self.error.clone() {
                    if ui.small_button("Dismiss").clicked() { self.error = None; }
                    ui.add(egui::Label::new(RichText::new(&error).color(ui.visuals().error_fg_color)).truncate()).on_hover_text(error);
                } else {
                    ui.add(egui::Label::new(RichText::new(self.file_status()).small()).truncate()).on_hover_text(self.file_status());
                    if let Some(axis) = self.axis_lock {
                        ui.separator();
                        ui.colored_label(ui.visuals().selection.stroke.color, format!("{axis:?} axis locked"));
                    }
                }
            });
            }
        });
    }
}

/// Original vector tool symbols with accessible names and full hover labels.
fn tool_button(
    ui: &mut egui::Ui,
    label: &str,
    action: shortcuts::Action,
    selected: bool,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(56., 46.), egui::Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::SelectableLabel,
            ui.is_enabled(),
            selected,
            label,
        )
    });
    let fill = if selected {
        ui.visuals().selection.bg_fill
    } else if response.hovered() {
        ui.visuals().widgets.hovered.weak_bg_fill
    } else {
        Color32::TRANSPARENT
    };
    let p = ui.painter();
    p.rect_filled(rect, 8., fill);
    if response.has_focus() {
        p.rect_stroke(
            rect.shrink(0.5),
            4.,
            Stroke::new(1.0f32, ui.visuals().selection.stroke.color),
            egui::StrokeKind::Inside,
        );
    }
    let c = rect.center_top() + egui::vec2(0., 14.);
    let icon_color = if selected {
        ui.visuals().selection.stroke.color
    } else {
        ui.visuals().text_color()
    };
    paint_tool_icon(p, c, label, Stroke::new(1.7f32, icon_color));
    p.text(
        rect.center_bottom() - egui::vec2(0., 7.),
        egui::Align2::CENTER_CENTER,
        label,
        egui::FontId::proportional(10.),
        icon_color,
    );
    response.on_hover_text(shortcuts::tooltip(
        shortcuts::MAP
            .iter()
            .find(|e| e.action == action)
            .expect("registered tool"),
    ))
}

// Shared geometry keeps the cursor symbol consistent with its toolbar button.
fn paint_tool_icon(p: &egui::Painter, c: egui::Pos2, label: &str, stroke: Stroke) {
    let icon_color = stroke.color;
    let pos = |x, y| c + egui::vec2(x, y);
    match label {
        "Line" => {
            p.line_segment(
                [pos(-9., 8.), pos(9., -8.)],
                Stroke::new(stroke.width + 0.8, icon_color),
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
                Stroke::new(stroke.width + 0.3, icon_color),
            );
        }
        "Eraser" => {
            p.add(egui::Shape::convex_polygon(
                vec![pos(-10., 3.), pos(1., -9.), pos(10., -1.), pos(-1., 11.)],
                p.ctx()
                    .style()
                    .visuals
                    .widgets
                    .noninteractive
                    .bg_stroke
                    .color,
                stroke,
            ));
        }
        "Measure" => {
            p.rect_filled(
                egui::Rect::from_center_size(c, egui::vec2(21., 13.)),
                3.,
                p.ctx()
                    .style()
                    .visuals
                    .widgets
                    .noninteractive
                    .bg_stroke
                    .color,
            );
            for x in [-6., 0., 6.] {
                p.line_segment([pos(x, -6.), pos(x, 0.)], stroke);
            }
        }
        "Paint" => {
            p.add(egui::Shape::convex_polygon(
                vec![pos(-9., -2.), pos(0., -10.), pos(10., 0.), pos(1., 9.)],
                p.ctx()
                    .style()
                    .visuals
                    .widgets
                    .noninteractive
                    .bg_stroke
                    .color,
                stroke,
            ));
            p.line_segment(
                [pos(-11., 11.), pos(11., 11.)],
                Stroke::new(stroke.width + 1.3, icon_color),
            );
        }
        _ => {}
    }
}

#[cfg(test)]
mod header_tests {
    use super::*;

    fn frame(
        app: &mut DrafterApp,
        ctx: &egui::Context,
        events: Vec<egui::Event>,
    ) -> egui::FullOutput {
        ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(720., 480.),
                )),
                events,
                ..Default::default()
            },
            |ctx| app.workspace_header(ctx),
        )
    }

    fn text_rect(output: &egui::FullOutput, label: &str) -> egui::Rect {
        output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.job.text == label => {
                    let rect = egui::Rect::from_min_size(text.pos, text.galley.size());
                    assert!(shape.clip_rect.contains_rect(rect), "{label} is clipped");
                    Some(rect)
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("{label} is missing"))
    }

    fn click(app: &mut DrafterApp, ctx: &egui::Context, pos: egui::Pos2) {
        for pressed in [true, false] {
            frame(
                app,
                ctx,
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
        }
    }

    #[test]
    fn dropdown_commands_fit_compact_window_and_select_tools() {
        for (menu, labels) in [
            (
                "File",
                vec!["Open…", "Save As…", "Export", "Recovery snapshots"],
            ),
            (
                "Camera",
                vec![
                    "Projection",
                    "Standard views",
                    "Construction grid",
                    "Fit model",
                ],
            ),
            (
                "Tools",
                vec!["Selection", "Modeling", "Utilities", "Follow Me", "Paint"],
            ),
            (
                "Draw",
                vec!["Line", "Rectangle", "Circle", "Arc", "Polygon"],
            ),
        ] {
            let ctx = egui::Context::default();
            configure_style(&ctx);
            ctx.set_theme(egui::Theme::Light);
            let mut app = DrafterApp::default();
            frame(&mut app, &ctx, vec![]);
            let output = frame(&mut app, &ctx, vec![]);
            click(&mut app, &ctx, text_rect(&output, menu).center());
            frame(&mut app, &ctx, vec![]);
            let output = frame(&mut app, &ctx, vec![]);
            for label in labels {
                let rect = text_rect(&output, label);
                assert!(rect.bottom() <= 480., "{label} is outside compact window");
            }
            if menu == "File" {
                click(&mut app, &ctx, text_rect(&output, "Export").center());
                frame(&mut app, &ctx, vec![]);
                let submenu = frame(&mut app, &ctx, vec![]);
                for label in ["OBJ…", "STL…", "glTF 2.0…"] {
                    let rect = text_rect(&submenu, label);
                    assert!(rect.right() <= 720.);
                }
                click(&mut app, &ctx, egui::pos2(680., 400.));
                let closed = frame(&mut app, &ctx, vec![]);
                assert!(!closed.shapes.iter().any(|shape| matches!(&shape.shape,
                    egui::Shape::Text(text) if text.galley.job.text == "OBJ…")));
            }
            if menu == "Tools" {
                frame(
                    &mut app,
                    &ctx,
                    vec![egui::Event::Key {
                        key: Key::Escape,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: egui::Modifiers::NONE,
                    }],
                );
                let closed = frame(&mut app, &ctx, vec![]);
                assert!(!closed.shapes.iter().any(|shape| matches!(&shape.shape,
                    egui::Shape::Text(text) if text.galley.job.text == "Utilities")));
            }
            if menu == "Draw" {
                click(&mut app, &ctx, text_rect(&output, "Circle").center());
                assert!(!app.modifying);
                assert_eq!(app.tool.kind(), ToolKind::Circle);
                let output = frame(&mut app, &ctx, vec![]);
                assert!(!output.shapes.iter().any(|shape| matches!(&shape.shape,
                    egui::Shape::Text(text) if text.galley.job.text == "Polygon")));
            }
        }
    }

    #[test]
    fn header_keeps_actions_visible_and_options_contextual() {
        for width in [720., 900., 960., 1280.] {
            for drawing in [false, true] {
                let ctx = egui::Context::default();
                configure_style(&ctx);
                ctx.set_theme(egui::Theme::Light);
                let mut app = DrafterApp::default();
                if drawing {
                    app.set_tool(ToolKind::Circle);
                } else {
                    app.choose_modify(ModifyKind::Select);
                }
                let mut output = None;
                for _ in 0..3 {
                    output = Some(ctx.run(
                        egui::RawInput {
                            screen_rect: Some(egui::Rect::from_min_size(
                                egui::Pos2::ZERO,
                                egui::vec2(width, 480.),
                            )),
                            ..Default::default()
                        },
                        |ctx| app.workspace_header(ctx),
                    ));
                }
                let output = output.unwrap();
                let text_rect = |label: &str| {
                    output.shapes.iter().find_map(|shape| {
                        if let egui::Shape::Text(text) = &shape.shape {
                            (text.galley.job.text == label).then(|| {
                                let rect = egui::Rect::from_min_size(text.pos, text.galley.size());
                                assert!(
                                    shape.clip_rect.contains_rect(rect),
                                    "{label} clipped at {width}"
                                );
                                rect
                            })
                        } else {
                            None
                        }
                    })
                };
                let save = text_rect("Save").expect("Save is always visible");
                let inspector = text_rect("Inspector").expect("Inspector is always visible");
                assert!(inspector.right() < save.left());
                assert!(save.right() <= width);
                for label in ["File", "Edit", "Camera", "Draw", "Tools", "Window", "Help"] {
                    text_rect(label).expect("all menus remain visible");
                }
                assert_eq!(text_rect("Work plane").is_some(), drawing);
                assert_eq!(text_rect("Segments").is_some(), drawing);
                if drawing {
                    let segments = text_rect("Segments").unwrap();
                    assert!(!segments.intersects(inspector));
                    for label in ["XY", "XZ", "YZ"] {
                        text_rect(label).expect("plane options remain visible");
                    }
                }
            }
        }
    }
}
