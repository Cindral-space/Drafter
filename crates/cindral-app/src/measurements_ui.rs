//! Shared status-field input routing and cached readout.
use super::*;
use cindral_tools::measurements::{MeasurementsBox, Readout, ValueKind};
impl DrafterApp {
    pub(crate) fn capture_measurement_keys(&mut self, response: &egui::Response) -> bool {
        if !response.hovered() || response.ctx.wants_keyboard_input() { return false; }
        let accepts = !self.modifying || !matches!(self.modify.kind(), ModifyKind::Select | ModifyKind::Eraser | ModifyKind::PaintBucket);
        if !accepts { return false; }
        if response.ctx.input(|i| i.key_pressed(Key::Escape)) && !self.measurements.text.is_empty() {
            self.measurements.escape(); self.error = None; return true;
        }
        response.ctx.input(|i| {
            if i.modifiers.command || i.modifiers.alt { return; }
            for event in &i.events {
                if let egui::Event::Text(text) = event {
                    if !self.measurements.text.is_empty() || MeasurementsBox::starts_input(text) { self.measurements.type_text(text); }
                }
            }
            if i.key_pressed(Key::Backspace) { self.measurements.backspace(); }
        });
        if response.ctx.input(|i| i.key_pressed(Key::Enter)) && !self.measurements.text.is_empty() {
            self.apply_measurement();
            self.measurements.invalid = self.error.is_some();
            return true;
        }
        false
    }
    pub(crate) fn measurement_readout(&self) -> Readout {
        if !self.modifying { return self.tool.measurement_readout(); }
        let label = match self.modify.kind() {
            ModifyKind::Rotate => "Angle", ModifyKind::Scale => "Scale",
            ModifyKind::Select | ModifyKind::Eraser | ModifyKind::PaintBucket => "",
            _ => "Distance",
        };
        Readout { label, values: [self.modify.measurement().unwrap_or(0.),0.,0.], count: usize::from(!label.is_empty() && self.modify.measurement().is_some()), kind: match self.modify.kind() { ModifyKind::Rotate => ValueKind::Angle, ModifyKind::Scale => ValueKind::Factor, _ => ValueKind::Length } }
    }
}
