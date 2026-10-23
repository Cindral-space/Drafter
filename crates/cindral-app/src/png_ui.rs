//! Current-view image options and responsive worker status.
use super::*;
use crate::png_export::{Aa, Control, Options};
use std::{path::PathBuf, sync::mpsc};
struct Job {
    control: Control,
    receiver: mpsc::Receiver<Result<String, String>>,
}
impl Drop for Job {
    fn drop(&mut self) {
        self.control.cancel();
    }
}
pub(crate) struct PngUi {
    pub(crate) open: bool,
    options: Options,
    prefs: PathBuf,
    job: Option<Job>,
    status: String,
    preference_warning: Option<String>,
    lock_aspect: bool,
    custom_size: bool,
    pub(crate) viewport: [u32; 2],
    pub(crate) srgb_surface: bool,
}
impl Default for PngUi {
    fn default() -> Self {
        let prefs = preferences_path();
        Self {
            open: false,
            options: Options::load(&prefs),
            prefs,
            job: None,
            status: String::new(),
            preference_warning: None,
            lock_aspect: true,
            custom_size: false,
            viewport: [1280, 800],
            srgb_surface: false,
        }
    }
}
fn preferences_path() -> PathBuf {
    let root = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            #[cfg(target_os = "windows")]
            if let Some(p) = std::env::var_os("APPDATA") {
                return PathBuf::from(p);
            }
            let home = std::env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_else(std::env::temp_dir);
            #[cfg(target_os = "macos")]
            {
                home.join("Library/Application Support")
            }
            #[cfg(not(target_os = "macos"))]
            {
                home.join(".config")
            }
        });
    root.join("cindral-drafter/png-export.json")
}
impl PngUi {
    pub(crate) fn with_surface(srgb_surface: bool) -> Self {
        Self {
            srgb_surface,
            ..Default::default()
        }
    }
}
impl DrafterApp {
    fn start_png(&mut self, path: PathBuf, force: bool) {
        self.cancel_modify();
        self.tool.cancel();
        self.cancel_section();
        let mut options = self.png.options.clone();
        options.context = self.history.context().to_vec();
        if let Err(e) = options.estimate(0) {
            self.png.status = e;
            return;
        }
        let bundle = self.file_values();
        let camera = self.camera.clone();
        let srgb = self.png.srgb_surface;
        let name = self
            .files
            .path
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Untitled".into());
        let control = Control::default();
        let worker = control.clone();
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let result = crate::png_job::export(
                bundle,
                Some(camera),
                None,
                &path,
                &options,
                srgb,
                &name,
                force,
                &worker,
            )
            .map(|_| {
                format!(
                    "Exported {} ({}×{})",
                    path.display(),
                    options.width,
                    options.height
                )
            });
            let _ = sender.send(result);
        });
        self.png.job = Some(Job { control, receiver });
        self.png.status = "Preparing PNG…".into();
    }
    pub(crate) fn png_window(&mut self, ctx: &egui::Context) {
        if let Some(job) = &self.png.job {
            match job.receiver.try_recv() {
                Ok(result) => {
                    self.png.job = None;
                    self.png.status = result.unwrap_or_else(|e| e);
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.png.job = None;
                    self.png.status = "PNG worker stopped before completion".into();
                }
                Err(mpsc::TryRecvError::Empty) => {
                    ctx.request_repaint_after(std::time::Duration::from_millis(50))
                }
            }
        }
        if !self.png.open {
            return;
        }
        if self.png.options.match_window && self.png.job.is_none() {
            self.png.options.width = self.png.viewport[0];
            self.png.options.height = self.png.viewport[1];
        }
        let mut export = false;
        egui::Window::new("Export Image (PNG)")
            .collapsible(false)
            .resizable(false)
            .default_width(420.)
            .default_height(700.)
            .max_height((ctx.content_rect().height() - 90.).max(100.))
            .vscroll(true)
            .show(ctx, |ui| {
                let state = &mut self.png;
                ui.add_enabled_ui(state.job.is_none(), |ui| {
                    ui.strong("Image size");
                    let preset = if state.custom_size && !state.options.match_window { "Custom" } else { size_label(&state.options) };
                    egui::ComboBox::from_id_salt("png-size")
                        .selected_text(preset)
                        .show_ui(ui, |ui| {
                            if ui.selectable_label(state.options.match_window, "Match window").clicked() {
                                state.options.match_window = true;
                                state.custom_size = false;
                                state.options.width = state.viewport[0].max(1);
                                state.options.height = state.viewport[1].max(1);
                            }
                            for (label, w, h) in [("1920×1080", 1920, 1080), ("3840×2160", 3840, 2160)] {
                                if ui.selectable_label(preset == label, label).clicked() {
                                    state.options.match_window = false;
                                    state.custom_size = false;
                                    state.options.width = w;
                                    state.options.height = h;
                                }
                            }
                            if ui.selectable_label(preset == "Custom", "Custom").clicked() {
                                state.options.match_window = false;
                                state.custom_size = true;
                            }
                        });
                    ui.add_enabled_ui(!state.options.match_window, |ui| {
                        let before = [state.options.width, state.options.height];
                        let mut changed_width = false;
                        let mut changed_height = false;
                        ui.horizontal(|ui| {
                            changed_width = ui.add(egui::DragValue::new(&mut state.options.width).range(1..=8192).prefix("Width ").suffix(" px")).changed();
                            changed_height = ui.add(egui::DragValue::new(&mut state.options.height).range(1..=8192).prefix("Height ").suffix(" px")).changed();
                        });
                        if changed_width || changed_height {
                            state.custom_size = true;
                        }
                        if state.lock_aspect {
                            lock_dimensions(&mut state.options, before, changed_width, changed_height);
                        }
                        ui.checkbox(&mut state.lock_aspect, "Lock aspect ratio");
                    });
                    framing_preview(ui, state.viewport, [state.options.width, state.options.height]);
                    ui.separator();
                    ui.strong("Quality and background");
                    ui.horizontal(|ui| {
                        ui.label("Antialiasing");
                        for aa in [Aa::Off, Aa::Medium, Aa::High] {
                            let mut candidate = state.options.clone();
                            candidate.aa = aa;
                            let validity = candidate.estimate(0);
                            let response = ui.add_enabled(validity.is_ok(), egui::Button::new(format!("{aa:?}")).selected(state.options.aa == aa));
                            if response.clicked() {
                                state.options.aa = aa;
                            }
                            if let Err(error) = validity {
                                response.on_disabled_hover_text(error);
                            }
                        }
                    });
                    ui.checkbox(&mut state.options.transparent, "Transparent background");
                    ui.small("UI, selection, tools, axes, guides and grid are excluded.");
                    let validity = state.options.estimate(0);
                    if let Err(error) = &validity {
                        ui.colored_label(ui.visuals().error_fg_color, error);
                        if let Some(aa) = compatible_quality(&state.options) {
                            if ui.button(format!("Use {aa:?} antialiasing for this size")).clicked() {
                                state.options.aa = aa;
                            }
                        }
                    }
                    ui.collapsing("Export limits", |ui| {
                        if let Ok((_, bytes)) = state.options.estimate(0) {
                            ui.label(format!("Base estimate: {} MiB, plus scene data.", bytes.div_ceil(1024 * 1024)));
                        }
                        ui.label("512 MiB export budget. Maximum render dimension: 8192 pixels, including antialiasing. Scene data is checked before rendering.");
                    });
                    ui.separator();
                    if ui.add_enabled(state.options.estimate(0).is_ok(), egui::Button::new("Choose file and export…")).clicked() {
                        export = true;
                    }
                });
                if let Some(job) = &state.job {
                    ui.add(egui::ProgressBar::new(job.control.progress()).show_percentage());
                    if ui.button("Cancel export").clicked() {
                        state.status = if job.control.cancel() { "Cancelling (submitted GPU work must finish)…" } else { "Export already completed" }.into();
                    }
                } else if ui.button("Close").clicked() {
                    state.open = false;
                }
                if !state.status.is_empty() {
                    ui.label(&state.status);
                }
                if let Some(warning) = &state.preference_warning {
                    ui.colored_label(ui.visuals().warn_fg_color, warning);
                }
            });
        if export {
            let mut dialog = rfd::FileDialog::new()
                .add_filter("PNG image", &["png"])
                .set_file_name("view.png");
            if !self.png.options.folder.as_os_str().is_empty() {
                dialog = dialog.set_directory(&self.png.options.folder);
            }
            if let Some(mut path) = dialog.save_file() {
                path.set_extension("png");
                let mut force = false;
                if path.exists() {
                    if rfd::MessageDialog::new()
                        .set_title("Replace PNG?")
                        .set_description(format!("Replace {}?", path.display()))
                        .set_buttons(rfd::MessageButtons::YesNo)
                        .show()
                        != rfd::MessageDialogResult::Yes
                    {
                        return;
                    }
                    force = true;
                }
                self.png.options.folder = path
                    .parent()
                    .unwrap_or(std::path::Path::new("."))
                    .to_path_buf();
                self.png.preference_warning =
                    self.png.options.save(&self.png.prefs).err().map(|e| {
                        format!("Export preferences could not be saved: {e}. Export will continue.")
                    });
                self.start_png(path, force);
            }
        }
    }
}

fn size_label(options: &Options) -> &'static str {
    match (options.match_window, options.width, options.height) {
        (true, _, _) => "Match window",
        (false, 1920, 1080) => "1920×1080",
        (false, 3840, 2160) => "3840×2160",
        _ => "Custom",
    }
}

fn compatible_quality(options: &Options) -> Option<Aa> {
    [Aa::High, Aa::Medium, Aa::Off].into_iter().find(|&aa| {
        let mut candidate = options.clone();
        candidate.aa = aa;
        candidate.estimate(0).is_ok()
    })
}

fn lock_dimensions(options: &mut Options, before: [u32; 2], width: bool, height: bool) {
    let ratio = before[0].max(1) as f64 / before[1].max(1) as f64;
    if width {
        let target = options.width as f64 / ratio;
        options.height = target.round().clamp(1., 8192.) as u32;
        if !(1.0..=8192.0).contains(&target) {
            options.width = (options.height as f64 * ratio).round().clamp(1., 8192.) as u32;
        }
    } else if height {
        let target = options.height as f64 * ratio;
        options.width = target.round().clamp(1., 8192.) as u32;
        if !(1.0..=8192.0).contains(&target) {
            options.height = (options.width as f64 / ratio).round().clamp(1., 8192.) as u32;
        }
    }
}

// Schematic only: equal-height frames describe the camera's fixed vertical span.
fn framing_preview(ui: &mut egui::Ui, viewport: [u32; 2], output: [u32; 2]) {
    ui.label("Framing preview (diagram)");
    let view_aspect = viewport[0].max(1) as f32 / viewport[1].max(1) as f32;
    let export_aspect = output[0].max(1) as f32 / output[1].max(1) as f32;
    let (area, _) = ui.allocate_exact_size(egui::vec2(360., 110.), egui::Sense::hover());
    let height = 100_f32.min(350. / view_aspect.max(export_aspect));
    let frame =
        |aspect| egui::Rect::from_center_size(area.center(), egui::vec2(height * aspect, height));
    let painter = ui.painter();
    painter.rect_filled(
        frame(export_aspect),
        0.,
        ui.visuals().selection.bg_fill.gamma_multiply(0.3),
    );
    painter.rect_stroke(
        frame(view_aspect),
        0.,
        egui::Stroke::new(1_f32, ui.visuals().text_color()),
        egui::StrokeKind::Inside,
    );
    painter.rect_stroke(
        frame(export_aspect),
        0.,
        egui::Stroke::new(2_f32, ui.visuals().selection.stroke.color),
        egui::StrokeKind::Inside,
    );
    ui.small("Thin outline: current view · Colored outline: export");
    ui.small(if (view_aspect - export_aspect).abs() < 0.001 {
        "Framing matches the current view."
    } else if export_aspect > view_aspect {
        "The export includes more on both sides; vertical framing stays fixed."
    } else {
        "The export crops both sides; vertical framing stays fixed."
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn four_k_quality_has_an_actionable_fallback() {
        let mut options = Options {
            width: 3840,
            height: 2160,
            match_window: false,
            ..Default::default()
        };
        assert!(options.estimate(0).is_err());
        assert_eq!(compatible_quality(&options), Some(Aa::Off));
        options.aa = Aa::Off;
        assert!(options.estimate(0).is_ok());
        assert_eq!(size_label(&options), "3840×2160");
    }

    #[test]
    fn locked_dimensions_follow_the_edited_axis() {
        let mut options = Options {
            width: 1280,
            height: 1080,
            ..Default::default()
        };
        lock_dimensions(&mut options, [1920, 1080], true, false);
        assert_eq!([options.width, options.height], [1280, 720]);
        options.height = 360;
        lock_dimensions(&mut options, [1280, 720], false, true);
        assert_eq!([options.width, options.height], [640, 360]);
        options.height = 8192;
        lock_dimensions(&mut options, [640, 360], false, true);
        assert_eq!([options.width, options.height], [8192, 4608]);
    }
}
