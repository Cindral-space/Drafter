//! Geographic sunlight controls; all edits are presentation-only.
use super::*;
use cindral_core::ShadowQuality;
impl DrafterApp {
    pub(crate) fn shadow_panel(&mut self, ui: &mut egui::Ui) {
        ui.collapsing("Sun & shadows", |ui| {
            let before = self.presentation.shadows.clone();
            let s = &mut self.presentation.shadows;
            ui.checkbox(&mut s.enabled, "Enable shadows");
            ui.horizontal(|ui| {
                ui.add(
                    egui::DragValue::new(&mut s.year)
                        .range(1900..=2100)
                        .prefix("Year "),
                );
                ui.add(
                    egui::DragValue::new(&mut s.month)
                        .range(1..=12)
                        .prefix("Month "),
                );
                ui.add(
                    egui::DragValue::new(&mut s.day)
                        .range(1..=31)
                        .prefix("Day "),
                );
            });
            while !cindral_core::valid_date(s.year, s.month, s.day) && s.day > 1 {
                s.day -= 1;
            }
            ui.label(format!(
                "Local time {:02}:{:02}",
                s.minutes as u32 / 60,
                s.minutes as u32 % 60
            ));
            ui.add(
                egui::Slider::new(&mut s.minutes, 0. ..=1439.)
                    .step_by(1.)
                    .show_value(false),
            );
            ui.add(
                egui::DragValue::new(&mut s.utc_offset)
                    .range(-14. ..=14.)
                    .speed(0.25)
                    .prefix("UTC offset ")
                    .suffix(" h"),
            );
            ui.add(
                egui::DragValue::new(&mut s.latitude)
                    .range(-90. ..=90.)
                    .speed(0.1)
                    .prefix("Latitude ")
                    .suffix("° N"),
            );
            ui.add(
                egui::DragValue::new(&mut s.longitude)
                    .range(-180. ..=180.)
                    .speed(0.1)
                    .prefix("Longitude ")
                    .suffix("° E"),
            );
            ui.label("Model +X is east; +Y is north. Offset includes DST.");
            ui.add(egui::Slider::new(&mut s.light, 0. ..=1.).text("Light"));
            ui.add(egui::Slider::new(&mut s.dark, 0. ..=1.).text("Dark"));
            ui.checkbox(&mut s.on_faces, "Shadows on faces");
            ui.checkbox(&mut s.on_ground, "Shadows on ground (Z=0)");
            ui.checkbox(&mut s.from_edges, "Shadows from edges");
            egui::ComboBox::from_id_salt("shadow_quality")
                .selected_text(format!("{:?}", s.quality))
                .show_ui(ui, |ui| {
                    for quality in [
                        ShadowQuality::Low,
                        ShadowQuality::Medium,
                        ShadowQuality::High,
                    ] {
                        ui.selectable_value(&mut s.quality, quality, format!("{quality:?}"));
                    }
                });
            if *s != before {
                self.presentation_changed();
            }
        });
    }
}
