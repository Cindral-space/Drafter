//! Window, viewport controls, and UI composition for Cindral Drafter.

use cindral_render::{
    camera::{Camera, Projection, StandardView},
    grid::{GridCallback, GridRenderer},
};
use eframe::egui::{self, Color32, Key, PointerButton, RichText};
use glam::{Vec2, Vec3};

/// Desktop editor state. Camera changes never mutate document geometry.
#[derive(Default)]
pub struct DrafterApp {
    camera: Camera,
}

impl DrafterApp {
    /// Initialize the wgpu resources and original editor styling.
    pub fn new(
        cc: &eframe::CreationContext<'_>,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let render = cc
            .wgpu_render_state
            .as_ref()
            .ok_or("Cindral Drafter requires the wgpu backend")?;
        render
            .renderer
            .write()
            .callback_resources
            .insert(GridRenderer::new(&render.device, render.target_format, 4));
        let mut style = (*cc.egui_ctx.style()).clone();
        style.visuals = egui::Visuals::dark();
        style.visuals.panel_fill = Color32::from_rgb(24, 29, 36);
        style.visuals.widgets.active.bg_fill = Color32::from_rgb(42, 105, 103);
        style.visuals.selection.bg_fill = Color32::from_rgb(42, 105, 103);
        style.spacing.item_spacing = egui::vec2(10.0, 8.0);
        cc.egui_ctx.set_style(style);
        tracing::info!("Cindral Drafter viewport initialized");
        Ok(Self::default())
    }

    fn extents(&mut self, aspect: f32) {
        // Until M2 introduces document bounds, fit the central construction area.
        self.camera.zoom_extents(
            Vec3::new(-10.0, -10.0, 0.0),
            Vec3::new(10.0, 10.0, 0.0),
            aspect,
        );
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
        if response.hovered() {
            response.ctx.input(|i| {
                if let Some(position) = i.pointer.hover_pos() {
                    let cursor = Vec2::new(
                        2.0 * (position.x - rect.left()) / rect.width() - 1.0,
                        1.0 - 2.0 * (position.y - rect.top()) / rect.height(),
                    );
                    self.camera.zoom(i.smooth_scroll_delta.y, cursor, aspect);
                }
                if i.key_pressed(Key::F) {
                    self.extents(aspect);
                }
                if i.key_pressed(Key::Num0) {
                    self.camera.set_view(StandardView::Iso);
                }
                if i.key_pressed(Key::Num1) {
                    self.camera.set_view(StandardView::Front);
                }
                if i.key_pressed(Key::Num3) {
                    self.camera.set_view(StandardView::Right);
                }
                if i.key_pressed(Key::Num7) {
                    self.camera.set_view(StandardView::Top);
                }
                if i.key_pressed(Key::Num5) {
                    toggle_projection(&mut self.camera);
                }
            });
        }
    }
}

fn toggle_projection(camera: &mut Camera) {
    camera.projection = match camera.projection {
        Projection::Perspective => Projection::Orthographic,
        Projection::Orthographic => Projection::Perspective,
    };
}

impl eframe::App for DrafterApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::TopBottomPanel::top("header").show(ctx, |ui| {
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("CINDRAL")
                        .strong()
                        .size(20.0)
                        .color(Color32::from_rgb(112, 217, 195)),
                );
                ui.label(RichText::new("DRAFTER").size(20.0));
                ui.separator();
                ui.label("Untitled");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label("M1 · Viewport");
                });
            });
            ui.add_space(6.0);
        });
        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label("MMB Orbit   ·   Shift + MMB Pan   ·   Scroll Zoom   ·   F Fit");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(format!("Distance {:.2} m", self.camera.distance));
                });
            });
        });
        egui::SidePanel::left("navigation")
            .exact_width(174.0)
            .resizable(false)
            .show(ctx, |ui| {
                ui.add_space(14.0);
                ui.heading("View");
                ui.label(RichText::new("Explore your workspace").small().weak());
                ui.add_space(12.0);
                let mut projection = self.camera.projection;
                ui.selectable_value(&mut projection, Projection::Perspective, "Perspective");
                ui.selectable_value(&mut projection, Projection::Orthographic, "Orthographic");
                self.camera.projection = projection;
                ui.separator();
                for (label, view) in [
                    ("Isometric     0", StandardView::Iso),
                    ("Top              7", StandardView::Top),
                    ("Bottom", StandardView::Bottom),
                    ("Front           1", StandardView::Front),
                    ("Back", StandardView::Back),
                    ("Left", StandardView::Left),
                    ("Right           3", StandardView::Right),
                ] {
                    if ui
                        .add_sized([150.0, 29.0], egui::Button::new(label))
                        .clicked()
                    {
                        self.camera.set_view(view);
                    }
                }
                ui.add_space(10.0);
                if ui
                    .add_sized([150.0, 32.0], egui::Button::new("Zoom extents  F"))
                    .clicked()
                {
                    let size = ctx.available_rect().size();
                    self.extents(size.x / size.y.max(1.0));
                }
                ui.separator();
                ui.label(RichText::new("WORLD AXES").small().weak());
                ui.colored_label(Color32::from_rgb(232, 95, 104), "X  Red");
                ui.colored_label(Color32::from_rgb(93, 207, 142), "Y  Green");
                ui.colored_label(Color32::from_rgb(100, 158, 255), "Z  Blue / up");
                ui.add_space(12.0);
                ui.label(RichText::new("Grid spacing  1 m").small().weak());
            });
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(Color32::from_rgb(17, 22, 29)))
            .show(ctx, |ui| {
                let (rect, response) =
                    ui.allocate_exact_size(ui.available_size(), egui::Sense::click_and_drag());
                if rect.width() <= 0.0 || rect.height() <= 0.0 {
                    return;
                }
                self.viewport_input(&response);
                ui.painter()
                    .add(eframe::egui_wgpu::Callback::new_paint_callback(
                        rect,
                        GridCallback {
                            view_projection: self
                                .camera
                                .view_projection(rect.width() / rect.height())
                                .to_cols_array_2d(),
                        },
                    ));
                ui.painter().text(
                    rect.left_top() + egui::vec2(20.0, 20.0),
                    egui::Align2::LEFT_TOP,
                    "CONSTRUCTION PLANE  /  XY",
                    egui::FontId::proportional(11.0),
                    Color32::from_gray(140),
                );
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

    fn frame(
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
