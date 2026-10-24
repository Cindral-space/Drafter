//! Shared persistent-tool and temporary Shift+MMB pan lifecycle.
use super::*;

#[derive(Clone, Copy)]
pub(crate) struct PanDrag {
    button: PointerButton,
    rect: egui::Rect,
    depth: f32,
}

impl DrafterApp {
    pub(crate) fn activate_pan(&mut self) {
        self.choose_modify(ModifyKind::Select);
        self.shortcuts.navigation = Some(shortcuts::Action::Pan);
        self.snap = None;
    }

    fn pan_press_depth(&self, rect: egui::Rect, pos: egui::Pos2) -> f32 {
        let ndc = Vec2::new(
            2. * (pos.x - rect.left()) / rect.width() - 1.,
            1. - 2. * (pos.y - rect.top()) / rect.height(),
        );
        let aspect = rect.aspect_ratio();
        let (origin, direction) = self.camera.ray(ndc, aspect);
        let hit = self.scene_hit(
            Ray { origin: origin.as_dvec3(), direction: direction.as_dvec3() },
            View {
                matrix: self.camera.view_projection(aspect).as_dmat4(),
                size: glam::DVec2::new(rect.width() as f64, rect.height() as f64),
            },
            glam::DVec2::new((pos.x - rect.left()) as f64, (pos.y - rect.top()) as f64),
        );
        self.camera.pan_depth(hit.map(|h| h.world_position.as_vec3()))
    }

    /// Returns true while owning navigation input, including release/abort frames.
    pub(crate) fn pan_input(&mut self, response: &egui::Response) -> bool {
        let ctx = &response.ctx;
        let rect = response.rect;
        let active = self.shortcuts.navigation == Some(shortcuts::Action::Pan);
        let (focused, escape, middle, shift) = ctx.input(|i| (
            i.focused,
            shortcuts::pressed(i, shortcuts::Action::Cancel),
            i.pointer.button_down(PointerButton::Middle),
            i.modifiers.shift,
        ));
        let valid = focused && rect.is_finite() && rect.width() > 0. && rect.height() > 0.;
        if !valid || escape || self.pan_drag.is_some_and(|d| d.rect != rect) {
            let owned = self.pan_drag.take().is_some();
            return owned || active;
        }
        if self.pan_drag.is_none() && response.hovered() && response.is_pointer_button_down_on() {
            let button = if middle && shift {
                Some(PointerButton::Middle)
            } else if active {
                Some(PointerButton::Primary)
            } else { None };
            if let Some(button) = button {
                let press = ctx.input(|i| {
                    i.pointer.button_pressed(button).then(|| i.pointer.press_origin()).flatten()
                });
                if let Some(pos) = press.filter(|p| rect.contains(*p)) {
                    self.pan_drag = Some(PanDrag { button, rect, depth: self.pan_press_depth(rect, pos) });
                    self.cancel_scene_transition();
                }
            }
        }
        if let Some(drag) = self.pan_drag {
            let (down, delta, pressed) = ctx.input(|i| (
                i.pointer.button_down(drag.button), i.pointer.delta(), i.pointer.button_pressed(drag.button)
            ));
            // Include motion delivered with release, but never the press-frame delta.
            if !pressed {
                    self.camera.pan_at_depth(Vec2::new(delta.x, delta.y), rect.height(), drag.depth);
            }
            if down {
                ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
            } else {
                self.pan_drag = None;
            }
            return true;
        }
        if active {
            if response.hovered() {
                ctx.set_cursor_icon(egui::CursorIcon::Grab);
                // egui already normalizes wheel/touchpad scrolling into points.
                let delta = ctx.input(|i| i.raw_scroll_delta);
                if delta != egui::Vec2::ZERO {
                    self.camera.pan_at_depth(Vec2::new(delta.x, delta.y), rect.height(), self.camera.pan_depth(None));
                }
            }
            return true;
        }
        // Never run inference or commit another tool while temporary navigation owns a press.
        middle && shift
    }
}
