//! Background COLLADA export and options dialog.
use super::*;
use cindral_io::{DaeControl, DaeOptions, DaeSelection, DaeUnits, ExportSummary};
use std::{path::PathBuf, sync::mpsc};
struct Job {
    control: DaeControl,
    receiver: mpsc::Receiver<cindral_io::Result<ExportSummary>>,
}
impl Drop for Job {
    fn drop(&mut self) {
        self.control.cancel();
    }
}
#[derive(Default)]
pub(crate) struct DaeUi {
    pub(crate) open: bool,
    options: DaeOptions,
    selection_only: bool,
    job: Option<Job>,
    status: String,
}
impl DrafterApp {
    fn dae_selection(&self) -> DaeSelection {
        let mut selection = DaeSelection {
            context: self.history.context().to_vec(),
            faces: self.selection.faces.clone(),
            edges: self.selection.edges.clone(),
            ..Default::default()
        };
        if let Some(path) = &self.organization.selected {
            if let Some((&id, parent)) = path.split_last() {
                if parent != selection.context {
                    selection.context = parent.to_vec();
                    selection.faces.clear();
                    selection.edges.clear();
                }
                selection.instances.insert(id);
            }
        }
        selection
    }
    fn start_dae_export(&mut self, path: PathBuf, options: DaeOptions) {
        self.cancel_modify();
        let bundle = self.file_values();
        let control = DaeControl::default();
        let worker_control = control.clone();
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let result = cindral_io::export_dae(&path, &bundle, &options, &worker_control);
            let _ = sender.send(result);
        });
        self.files.dae.job = Some(Job { control, receiver });
        self.files.dae.status = "Exporting COLLADA…".into();
    }
    pub(crate) fn dae_window(&mut self, ctx: &egui::Context) {
        if let Some(job) = &self.files.dae.job {
            match job.receiver.try_recv() {
                Ok(result) => {
                    self.files.dae.job = None;
                    self.files.dae.status = match result {
                        Ok(s) => format!(
                            "Exported {} triangles / {} occurrences",
                            s.triangles, s.instances
                        ),
                        Err(e) => e.to_string(),
                    };
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.files.dae.job = None;
                    self.files.dae.status = "COLLADA export worker stopped".into();
                }
                Err(mpsc::TryRecvError::Empty) => {
                    ctx.request_repaint_after(std::time::Duration::from_millis(50));
                }
            }
        }
        if !self.files.dae.open {
            return;
        }
        let mut export = false;
        egui::Window::new("Export COLLADA (.dae)")
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                let state = &mut self.files.dae;
                ui.add_enabled_ui(state.job.is_none(), |ui| {
                    ui.horizontal(|ui| {
                        ui.label("Units:");
                        ui.radio_value(&mut state.options.units, DaeUnits::Meters, "Meters");
                        ui.radio_value(&mut state.options.units, DaeUnits::Inches, "Inches");
                    });
                    ui.checkbox(&mut state.options.triangulate_all, "Triangulate all faces");
                    ui.checkbox(&mut state.options.include_hidden, "Include hidden geometry");
                    ui.checkbox(&mut state.selection_only, "Selection only");
                    ui.checkbox(&mut state.options.include_textures, "Include textures");
                    if ui.button("Choose file and export…").clicked() {
                        export = true;
                    }
                });
                if let Some(job) = &state.job {
                    ui.add(egui::ProgressBar::new(job.control.progress()).show_percentage());
                    if ui.button("Cancel export").clicked() {
                        if job.control.cancel() {
                            state.status = "Cancelling export…".into();
                        } else {
                            state.status = "Export already completed".into();
                        }
                    }
                } else if ui.button("Close").clicked() {
                    state.open = false;
                }
                if !state.status.is_empty() {
                    ui.label(&state.status);
                }
            });
        if export {
            if let Some(mut path) = rfd::FileDialog::new()
                .add_filter("COLLADA 1.4.1", &["dae"])
                .set_file_name("model.dae")
                .save_file()
            {
                path.set_extension("dae");
                let mut options = self.files.dae.options.clone();
                options.force = false;
                if path.exists() {
                    let confirm = rfd::MessageDialog::new()
                        .set_title("Replace COLLADA file?")
                        .set_description(format!("Replace {}?", path.display()))
                        .set_buttons(rfd::MessageButtons::YesNo)
                        .show();
                    if confirm != rfd::MessageDialogResult::Yes {
                        return;
                    }
                    options.force = true;
                }
                options.selection = if self.files.dae.selection_only {
                    Some(self.dae_selection())
                } else {
                    None
                };
                self.start_dae_export(path, options);
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn export_worker_snapshot_finishes_and_reports_cancellation() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("empty.dae");
        let mut app = DrafterApp::default();
        app.start_dae_export(path.clone(), DaeOptions::default());
        let job = app.files.dae.job.take().unwrap();
        assert!(job
            .receiver
            .recv_timeout(std::time::Duration::from_secs(10))
            .unwrap()
            .is_ok());
        assert!(path.exists());
        assert!(!job.control.cancel());
        let path = dir.path().join("selected.dae");
        let options = DaeOptions {
            selection: Some(app.dae_selection()),
            ..Default::default()
        };
        app.start_dae_export(path.clone(), options);
        let job = app.files.dae.job.take().unwrap();
        let s = job
            .receiver
            .recv_timeout(std::time::Duration::from_secs(10))
            .unwrap()
            .unwrap();
        assert_eq!(s.instances, 0);
        assert!(path.exists());
    }
    #[test]
    fn outliner_selection_captures_parent_context_and_current_faces() {
        let mut app = DrafterApp::default();
        app.selection
            .faces
            .insert(cindral_core::FaceId::from_raw(4));
        app.organization.selected = Some(vec![cindral_core::InstanceId(7)]);
        let s = app.dae_selection();
        assert!(s.context.is_empty());
        assert!(s.faces.contains(&cindral_core::FaceId::from_raw(4)));
        assert!(s.instances.contains(&cindral_core::InstanceId(7)));
        app.organization.selected = Some(vec![
            cindral_core::InstanceId(7),
            cindral_core::InstanceId(8),
        ]);
        let s = app.dae_selection();
        assert_eq!(s.context, vec![cindral_core::InstanceId(7)]);
        assert!(s.faces.is_empty());
        assert!(s.instances.contains(&cindral_core::InstanceId(8)));
    }
}
