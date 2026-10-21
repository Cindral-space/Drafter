//! File dialogs, atomic saves, asynchronous autosave and explicit recovery.
use super::*;
use cindral_io::{DraftBundle, TextureAsset};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::mpsc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
struct AutosaveJob {
    generation: u64,
    revision: (u64, u64),
    receiver: mpsc::Receiver<Result<PathBuf, String>>,
}
pub(crate) struct FileState {
    path: Option<PathBuf>,
    saved_values: Option<DraftBundle>,
    checked_save_revision: Option<(u64, u64)>,
    modified: bool,
    pub(crate) fit: bool,
    pub(crate) textures: BTreeMap<String, TextureAsset>,
    directory: PathBuf,
    snapshot: PathBuf,
    generation: u64,
    last_time: Instant,
    pub(crate) last_revision: Option<(u64, u64)>,
    interval: Duration,
    job: Option<AutosaveJob>,
    candidates: Vec<PathBuf>,
    checked: bool,
    offer: bool,
    status: String,
}
fn state_directory() -> PathBuf {
    if let Some(path) = std::env::var_os("XDG_STATE_HOME") {
        return PathBuf::from(path).join("cindral-drafter/recovery");
    }
    #[cfg(target_os = "windows")]
    if let Some(path) = std::env::var_os("LOCALAPPDATA") {
        return PathBuf::from(path).join("CindralDrafter/recovery");
    }
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    #[cfg(target_os = "macos")]
    {
        home.join("Library/Application Support/CindralDrafter/recovery")
    }
    #[cfg(not(target_os = "macos"))]
    {
        home.join(".local/state/cindral-drafter/recovery")
    }
}
fn snapshot_path(directory: &Path, generation: u64) -> PathBuf {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    directory.join(format!(
        "draft-{}-{now}-{generation}.cdraft.autosave",
        std::process::id()
    ))
}
impl Default for FileState {
    fn default() -> Self {
        let directory = state_directory();
        let snapshot = snapshot_path(&directory, 0);
        Self {
            path: None,
            saved_values: None,
            checked_save_revision: None,
            modified: false,
            fit: false,
            textures: BTreeMap::new(),
            directory,
            snapshot,
            generation: 0,
            last_time: Instant::now(),
            last_revision: None,
            interval: Duration::from_secs(30),
            job: None,
            candidates: Vec::new(),
            checked: false,
            offer: false,
            status: "Untitled draft".into(),
        }
    }
}
#[derive(Clone)]
enum Action {
    New,
    Open,
    Save,
    SaveAs,
    Import,
    Obj,
    Stl,
    Gltf,
    Recover(PathBuf),
    Discard(PathBuf),
}
impl DrafterApp {
    fn sync_file_textures(&mut self) -> cindral_io::Result<()> {
        let mut layers = BTreeMap::new();
        let mut pixels = Vec::new();
        for (key, asset) in &self.files.textures {
            let image = cindral_io::decode_texture(asset)?;
            layers.insert(key.clone(), pixels.len() as u32 + 1);
            pixels.push(cindral_render::texture::TexturePixels {
                width: image.width,
                height: image.height,
                rgba: image.rgba,
            });
        }
        self.scene.set_textures(layers, pixels);
        self.refresh_mesh();
        Ok(())
    }
    fn file_revision(&self) -> (u64, u64) {
        (self.history.revision(), self.presentation_serial)
    }
    /// Snapshot serializable file values, with encoded assets and no runtime history/caches.
    pub fn file_values(&self) -> DraftBundle {
        DraftBundle {
            document: self.history.to_data(),
            textures: self.files.textures.clone(),
            presentation: self.presentation.clone(),
        }
    }
    fn meaningful(&self) -> bool {
        self.presentation != cindral_core::Presentation::default()
            || self.history.undo_depth() != 0
            || self.history.redo_depth() != 0
            || !self.history.tags().is_empty()
            || !self.history.materials().is_empty()
            || !self.files.textures.is_empty()
            || self.files.path.is_some()
            || !self.history.model().vertices().is_empty()
            || self
                .history
                .instances(cindral_core::DefinitionId(0))
                .is_some_and(|m| !m.is_empty())
    }
    fn backup_current(&mut self) -> cindral_io::Result<()> {
        if self.meaningful() {
            std::fs::create_dir_all(&self.files.directory)?;
            // An older background job may still own the previous snapshot path.
            self.files.snapshot = snapshot_path(&self.files.directory, self.files.generation);
            cindral_io::save_autosave(&self.files.snapshot, &self.file_values())?;
        }
        Ok(())
    }
    fn reset_file_target(&mut self, path: Option<PathBuf>) {
        self.files.generation = self.files.generation.wrapping_add(1);
        self.files.path = path;
        self.files.snapshot = snapshot_path(&self.files.directory, self.files.generation);
        self.files.last_time = Instant::now();
        self.files.last_revision = None;
        self.files.saved_values = None;
        self.files.checked_save_revision = None;
        self.files.modified = false;
    }
    fn apply_file(&mut self, bundle: DraftBundle, path: Option<PathBuf>) -> cindral_io::Result<()> {
        self.set_document_data(bundle.document)?;
        self.files.textures = bundle.textures;
        self.presentation = bundle.presentation;
        self.presentation_ui = Default::default();
        self.section_ui = Default::default();
        self.presentation_serial = self.presentation_serial.wrapping_add(1);
        self.reset_file_target(path);
        self.sync_file_textures()?;
        self.files.fit = true;
        self.files.offer = false;
        self.files.status = self.files.path.as_ref().map_or_else(
            || "Recovered/imported draft · Save As to keep it".into(),
            |p| format!("Opened {}", p.display()),
        );
        self.files.last_revision = self.files.path.as_ref().map(|_| self.file_revision());
        if self.files.path.is_some() {
            self.files.saved_values = Some(self.file_values());
            self.files.checked_save_revision = Some(self.file_revision());
        }
        self.error = None;
        Ok(())
    }
    /// Open validated native values into fresh history/root context; retain the previous draft's recovery snapshot.
    pub fn open_path(&mut self, path: &Path) -> cindral_io::Result<()> {
        let bundle = cindral_io::load_draft(path)?;
        self.cancel_modify();
        self.tool.cancel();
        self.backup_current()?;
        self.apply_file(bundle, Some(path.to_owned()))
    }
    /// Atomically save committed values; any active drag is cancelled before writing.
    pub fn save_path(&mut self, path: &Path) -> cindral_io::Result<()> {
        self.cancel_modify();
        self.tool.cancel();
        let values = self.file_values();
        cindral_io::save_draft(path, &values)?;
        self.files.saved_values = Some(values);
        self.files.checked_save_revision = Some(self.file_revision());
        self.files.modified = false;
        self.files.path = Some(path.to_owned());
        self.files.last_revision = Some(self.file_revision());
        self.files.status = format!("Saved {}", path.display());
        if self.files.job.is_none() && self.files.snapshot.exists() {
            if let Err(e) = std::fs::remove_file(&self.files.snapshot) {
                tracing::warn!("could not remove superseded recovery snapshot: {e}");
            }
        }
        Ok(())
    }
    fn do_file_action(&mut self, action: Action) -> cindral_io::Result<()> {
        match action {
            Action::New => {
                self.cancel_modify();
                self.tool.cancel();
                self.backup_current()?;
                self.files.textures.clear();
                self.set_document_data(History::new().to_data())?;
                self.reset_file_target(None);
                self.sync_file_textures()?;
                self.files.fit = true;
                self.files.status = "New draft".into();
            }
            Action::Open => {
                if let Some(path) = rfd::FileDialog::new()
                    .add_filter("Cindral Drafter", &["cdraft"])
                    .pick_file()
                {
                    self.open_path(&path)?;
                }
            }
            Action::Save => {
                if let Some(path) = self.files.path.clone() {
                    self.save_path(&path)?;
                } else {
                    return self.do_file_action(Action::SaveAs);
                }
            }
            Action::SaveAs => {
                if let Some(mut path) = rfd::FileDialog::new()
                    .add_filter("Cindral Drafter", &["cdraft"])
                    .set_file_name("model.cdraft")
                    .save_file()
                {
                    path.set_extension("cdraft");
                    self.save_path(&path)?;
                }
            }
            Action::Import => {
                if let Some(path) = rfd::FileDialog::new()
                    .add_filter("Wavefront OBJ", &["obj"])
                    .pick_file()
                {
                    let bundle = cindral_io::import_obj(&path)?;
                    self.cancel_modify();
                    self.tool.cancel();
                    self.backup_current()?;
                    self.apply_file(bundle, None)?;
                }
            }
            Action::Obj | Action::Stl | Action::Gltf => {
                let (label, ext) = match action {
                    Action::Obj => ("Wavefront OBJ", "obj"),
                    Action::Stl => ("Binary STL", "stl"),
                    _ => ("glTF 2.0", "gltf"),
                };
                if let Some(mut path) = rfd::FileDialog::new()
                    .add_filter(label, &[ext])
                    .set_file_name(format!("model.{ext}"))
                    .save_file()
                {
                    path.set_extension(ext);
                    self.cancel_modify();
                    let bundle = self.file_values();
                    let summary = match action {
                        Action::Obj => cindral_io::export_obj(&path, &bundle)?,
                        Action::Stl => cindral_io::export_stl(&path, &bundle)?,
                        _ => cindral_io::export_gltf(&path, &bundle)?,
                    };
                    self.files.status = format!(
                        "Exported {} triangles / {} occurrences",
                        summary.triangles, summary.instances
                    );
                }
            }
            Action::Recover(path) => {
                let bundle = cindral_io::load_draft(&path)?;
                self.cancel_modify();
                self.backup_current()?;
                self.apply_file(bundle, None)?;
            }
            Action::Discard(path) => {
                std::fs::remove_file(&path)?;
                self.files.candidates.retain(|p| p != &path);
            }
        }
        Ok(())
    }
    pub(crate) fn file_menu(&mut self, ui: &mut egui::Ui) {
        let mut action = None;
        ui.menu_button("File", |ui| {
            ui.set_min_width(240.);
            for (text, shortcut, a) in [
                ("New", "", Action::New),
                ("Open…", "Ctrl+O", Action::Open),
                ("Save", "Ctrl+S", Action::Save),
                ("Save As…", "Ctrl+Shift+S", Action::SaveAs),
            ] {
                if ui
                    .add(egui::Button::new(text).shortcut_text(shortcut))
                    .clicked()
                {
                    action = Some(a);
                    ui.close();
                }
            }
            ui.separator();
            if ui.button("Import OBJ…").clicked() {
                action = Some(Action::Import);
                ui.close();
            }
            ui.menu_button("Export", |ui| {
                for (text, a) in [
                    ("OBJ…", Action::Obj),
                    ("STL…", Action::Stl),
                    ("glTF 2.0…", Action::Gltf),
                ] {
                    if ui.button(text).clicked() {
                        action = Some(a);
                        ui.close();
                    }
                }
            });
            ui.separator();
            // Keep the snapshot list open for repeated discard/refresh actions.
            egui::containers::menu::SubMenuButton::new("Recovery snapshots")
                .config(
                    egui::containers::menu::MenuConfig::new()
                        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside),
                )
                .ui(ui, |ui| {
                    if ui.button("Refresh recovery snapshots").clicked() {
                        match cindral_io::recovery_files(&self.files.directory) {
                            Ok(files) => self.files.candidates = files,
                            Err(e) => self.error = Some(e.to_string()),
                        }
                    }
                    if self.files.candidates.is_empty() {
                        ui.label("No recovery snapshots found.");
                    }
                    egui::ScrollArea::vertical()
                        .max_height(210.)
                        .show(ui, |ui| {
                            for path in self.files.candidates.clone() {
                                ui.horizontal(|ui| {
                                    ui.add_sized(
                                        [180., 30.],
                                        egui::Label::new(
                                            path.file_name().unwrap_or_default().to_string_lossy(),
                                        )
                                        .truncate(),
                                    )
                                    .on_hover_text(path.display().to_string());
                                    if ui.small_button("Recover").clicked() {
                                        action = Some(Action::Recover(path.clone()));
                                        ui.close();
                                    }
                                    if ui
                                        .add(egui::Button::new("×").min_size(egui::vec2(30., 30.)))
                                        .on_hover_text("Discard this recovery snapshot permanently")
                                        .clicked()
                                    {
                                        action = Some(Action::Discard(path));
                                    }
                                });
                            }
                        });
                });
        });
        if let Some(action) = action {
            if let Err(e) = self.do_file_action(action) {
                self.error = Some(e.to_string());
            }
        }
    }
    pub(crate) fn file_tick(&mut self, ctx: &egui::Context) {
        if !self.files.checked {
            self.files.checked = true;
            match cindral_io::recovery_files(&self.files.directory) {
                Ok(files) => {
                    self.files.offer = !files.is_empty();
                    self.files.candidates = files;
                }
                Err(e) => self.error = Some(e.to_string()),
            }
        }
        let shortcut = ctx.input(|i| {
            if i.modifiers.command && i.key_pressed(Key::S) {
                Some(if i.modifiers.shift {
                    Action::SaveAs
                } else {
                    Action::Save
                })
            } else if i.modifiers.command && i.key_pressed(Key::O) {
                Some(Action::Open)
            } else {
                None
            }
        });
        if let Some(action) = shortcut {
            if let Err(e) = self.do_file_action(action) {
                self.error = Some(e.to_string());
            }
        }
        self.refresh_save_state();
    }
    fn refresh_save_state(&mut self) {
        let revision = self.file_revision();
        if self.files.checked_save_revision == Some(revision)
            || self.modify.active()
            || self.organization.drag.is_some()
        {
            return;
        }
        if let Some(saved) = &self.files.saved_values {
            let mut current = self.history.to_data();
            // Undo keeps identity allocators monotonic; allocator growth alone is not an edit.
            current.next_id = saved.document.next_id;
            for (id, definition) in &mut current.definitions {
                if let Some(before) = saved.document.definitions.get(id) {
                    definition.geometry.next_id = before.geometry.next_id;
                }
            }
            self.files.modified = saved.presentation != self.presentation
                || saved.textures != self.files.textures
                || saved.document != current;
        }
        self.files.checked_save_revision = Some(revision);
    }
    pub(crate) fn document_save_label(&self) -> &'static str {
        if self.files.path.is_none() {
            "Unsaved"
        } else if self.files.modified {
            "Modified"
        } else {
            "Saved"
        }
    }
    pub(crate) fn save_document(&mut self) {
        if let Err(e) = self.do_file_action(Action::Save) {
            self.error = Some(e.to_string());
        }
    }
    pub(crate) fn recovery_window(&mut self, ctx: &egui::Context) {
        if !self.files.offer {
            return;
        }
        let mut action = None;
        let mut open = true;
        egui::Window::new("Recover a draft")
            .collapsible(false)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.label("A previous session left recovery snapshots. Recover opens a new unsaved draft.");
                for path in self.files.candidates.clone() {
                    ui.horizontal(|ui| {
                        ui.label(path.file_name().unwrap_or_default().to_string_lossy());
                        if ui.button("Recover").clicked() {
                            action = Some(Action::Recover(path.clone()));
                        }
                        if ui.add(egui::Button::new("×").frame(false).min_size(egui::vec2(28., 28.)))
                            .on_hover_text("Discard this recovery snapshot permanently")
                            .clicked() {
                            action = Some(Action::Discard(path));
                        }
                    });
                }
            });
        self.files.offer = open;
        if let Some(action) = action {
            if let Err(e) = self.do_file_action(action) {
                self.error = Some(e.to_string());
            }
            if self.files.candidates.is_empty() {
                self.files.offer = false;
            }
        }
    }
    pub(crate) fn autosave_tick(&mut self, ctx: &egui::Context) {
        if let Some(job) = self.files.job.take() {
            match job.receiver.try_recv() {
                Ok(Ok(path)) => {
                    if job.generation == self.files.generation {
                        self.files.last_revision = Some(job.revision);
                        self.files.status = format!(
                            "Autosaved recovery snapshot {}",
                            path.file_name().unwrap_or_default().to_string_lossy()
                        );
                    }
                }
                Ok(Err(e)) => self.error = Some(format!("Autosave failed: {e}")),
                Err(mpsc::TryRecvError::Empty) => self.files.job = Some(job),
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.error = Some("Autosave worker stopped before completion".into())
                }
            }
        }
        let busy = self.modify.active() || self.organization.drag.is_some();
        if self.files.job.is_none()
            && !busy
            && self.meaningful()
            && self.files.last_time.elapsed() >= self.files.interval
            && self.files.last_revision != Some(self.file_revision())
        {
            self.files.last_time = Instant::now();
            let bundle = self.file_values();
            let path = self.files.snapshot.clone();
            let directory = self.files.directory.clone();
            let generation = self.files.generation;
            let revision = self.file_revision();
            let (sender, receiver) = mpsc::channel();
            let spawned = std::thread::Builder::new()
                .name("cindral-autosave".into())
                .spawn(move || {
                    let result = std::fs::create_dir_all(directory)
                        .map_err(|e| e.to_string())
                        .and_then(|()| {
                            cindral_io::save_autosave(&path, &bundle)
                                .map(|()| path)
                                .map_err(|e| e.to_string())
                        });
                    let _ = sender.send(result);
                });
            match spawned {
                Ok(_) => {
                    self.files.job = Some(AutosaveJob {
                        generation,
                        revision,
                        receiver,
                    })
                }
                Err(e) => self.error = Some(e.to_string()),
            }
        }
        ctx.request_repaint_after(if self.files.job.is_some() {
            Duration::from_millis(100)
        } else {
            Duration::from_secs(1)
        });
    }
    pub(crate) fn document_title(&self) -> String {
        self.files
            .path
            .as_ref()
            .and_then(|path| path.file_name())
            .map_or_else(
                || "Untitled draft".to_owned(),
                |name| name.to_string_lossy().into_owned(),
            )
    }
    pub(crate) fn file_status(&self) -> &str {
        &self.files.status
    }
    pub(crate) fn load_material_texture(&mut self) {
        let Some(id) = self.organization.material else {
            self.error = Some("Choose a material first".into());
            return;
        };
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("Texture image", &["png", "jpg", "jpeg"])
            .pick_file()
        {
            let result = (|| -> cindral_io::Result<()> {
                let (key, asset) = cindral_io::load_texture(&path)?;
                if self.files.textures.len() >= 127 && !self.files.textures.contains_key(&key) {
                    return Err(cindral_io::IoError::Invalid(
                        "at most 127 texture assets can be loaded in one document session".into(),
                    ));
                }
                let mut material = self
                    .history
                    .materials()
                    .get(&id)
                    .cloned()
                    .ok_or(cindral_core::KernelError::MissingEntity)?;
                material.texture = Some(key.clone());
                self.organization.texture = key.clone();
                self.files.textures.insert(key, asset);
                self.organize(cindral_core::OrganizationCommand::UpdateMaterial { id, material })?;
                self.sync_file_textures()?;
                Ok(())
            })();
            if let Err(e) = result {
                self.error = Some(e.to_string());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn app(directory: &Path) -> DrafterApp {
        let mut app = DrafterApp::default();
        app.files.directory = directory.to_owned();
        app.files.snapshot = snapshot_path(directory, 0);
        app.files.interval = Duration::ZERO;
        app.files.checked = true;
        app.history
            .execute(cindral_core::Command::AddPolyline {
                points: vec![DVec3::ZERO, DVec3::X, DVec3::X + DVec3::Y, DVec3::Y],
                closed: true,
            })
            .unwrap();
        app
    }
    #[test]
    fn saved_state_tracks_edits_and_is_not_cleared_by_recovery_autosave() {
        let directory = tempfile::tempdir().unwrap();
        let mut editor = app(directory.path());
        assert_eq!(editor.document_save_label(), "Unsaved");
        let path = directory.path().join("state.cdraft");
        editor.save_path(&path).unwrap();
        assert_eq!(editor.document_save_label(), "Saved");
        let original = editor.view_style().clone();
        editor.set_view_style(cindral_core::ViewStyle {
            mode: cindral_core::StyleMode::XRay,
            ..original.clone()
        });
        editor.refresh_save_state();
        assert_eq!(editor.document_save_label(), "Modified");
        let (sender, receiver) = mpsc::channel();
        editor.files.interval = Duration::MAX;
        editor.files.job = Some(AutosaveJob {
            generation: editor.files.generation,
            revision: editor.file_revision(),
            receiver,
        });
        sender.send(Ok(editor.files.snapshot.clone())).unwrap();
        editor.autosave_tick(&egui::Context::default());
        editor.refresh_save_state();
        assert_eq!(editor.document_save_label(), "Modified");
        editor.set_view_style(original);
        editor.refresh_save_state();
        assert_eq!(editor.document_save_label(), "Saved");
        editor
            .history
            .execute(Command::AddEdge {
                start: DVec3::ZERO,
                end: DVec3::Z,
            })
            .unwrap();
        editor.refresh_save_state();
        assert_eq!(editor.document_save_label(), "Modified");
        editor.history.undo().unwrap();
        editor.refresh_save_state();
        assert_eq!(editor.document_save_label(), "Saved");
        editor.history.redo().unwrap();
        editor.refresh_save_state();
        assert_eq!(editor.document_save_label(), "Modified");
        editor.save_path(&path).unwrap();
        assert_eq!(editor.document_save_label(), "Saved");
        editor.open_path(&path).unwrap();
        assert_eq!(editor.document_save_label(), "Saved");
        let face = *editor.model().faces().keys().next().unwrap();
        editor.select_connected(cindral_tools::Hit::Face(face));
        let group = editor
            .create_container(cindral_core::DefinitionKind::Group)
            .unwrap();
        editor.save_path(&path).unwrap();
        editor.enter_context(group).unwrap();
        editor.refresh_save_state();
        assert_eq!(editor.document_save_label(), "Saved");
        editor.exit_context().unwrap();
        editor.refresh_save_state();
        assert_eq!(editor.document_save_label(), "Saved");
    }
    #[test]
    fn save_shortcut_works_with_application_menu_closed() {
        let directory = tempfile::tempdir().unwrap();
        let mut source = app(directory.path());
        let path = directory.path().join("shortcut.cdraft");
        source.save_path(&path).unwrap();
        source
            .history
            .execute(Command::AddEdge {
                start: DVec3::ZERO,
                end: DVec3::Z,
            })
            .unwrap();
        let ctx = egui::Context::default();
        let modifiers = egui::Modifiers {
            ctrl: true,
            command: true,
            ..Default::default()
        };
        let _ = ctx.run(
            egui::RawInput {
                modifiers,
                events: vec![egui::Event::Key {
                    key: Key::S,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers,
                }],
                ..Default::default()
            },
            |ctx| source.workspace_header(ctx),
        );
        assert!(source.error.is_none());
        let bundle = cindral_io::load_draft(&path).unwrap();
        assert_eq!(bundle, source.file_values());
    }
    #[test]
    fn save_open_and_crash_recovery_reset_history_without_losing_values() {
        let directory = tempfile::tempdir().unwrap();
        let mut source = app(directory.path());
        let expected = source.file_values();
        let path = directory.path().join("draft.cdraft");
        source.save_path(&path).unwrap();
        let mut loaded = app(directory.path());
        loaded.open_path(&path).unwrap();
        assert_eq!(loaded.file_values(), expected);
        assert_eq!(loaded.history.undo_depth(), 0);
        assert!(loaded.history.context().is_empty());
        let ctx = egui::Context::default();
        source.files.last_revision = None;
        source.autosave_tick(&ctx);
        let deadline = Instant::now() + Duration::from_secs(10);
        while source.files.job.is_some() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
            source.autosave_tick(&ctx);
        }
        assert!(source.files.job.is_none(), "autosave worker timed out");
        assert_eq!(
            cindral_io::load_draft(&source.files.snapshot).unwrap(),
            expected
        );
        let mut recovered = app(directory.path());
        recovered
            .do_file_action(Action::Recover(source.files.snapshot.clone()))
            .unwrap();
        assert_eq!(recovered.file_values(), expected);
        assert!(recovered.files.path.is_none());
        assert_eq!(recovered.history.undo_depth(), 0);
        assert!(
            source.files.snapshot.exists(),
            "recovery retains the crash snapshot"
        );
    }
    #[test]
    fn invalid_open_preserves_the_active_document() {
        let directory = tempfile::tempdir().unwrap();
        let mut app = app(directory.path());
        let expected = app.file_values();
        let path = directory.path().join("bad.cdraft");
        std::fs::write(&path, b"broken").unwrap();
        assert!(app.open_path(&path).is_err());
        assert_eq!(app.file_values(), expected);
    }
    #[test]
    fn old_autosave_completion_does_not_clean_new_style_edits() {
        let directory = tempfile::tempdir().unwrap();
        let mut editor = app(directory.path());
        editor.files.interval = Duration::MAX;
        let old_revision = editor.file_revision();
        let (sender, receiver) = mpsc::channel();
        editor.files.job = Some(AutosaveJob {
            generation: editor.files.generation,
            revision: old_revision,
            receiver,
        });
        editor.set_view_style(cindral_core::ViewStyle {
            mode: cindral_core::StyleMode::XRay,
            ..Default::default()
        });
        sender.send(Ok(editor.files.snapshot.clone())).unwrap();
        editor.autosave_tick(&egui::Context::default());
        assert_ne!(editor.files.last_revision, Some(editor.file_revision()));
        let path = directory.path().join("style.cdraft");
        editor.save_path(&path).unwrap();
        let mut loaded = app(directory.path());
        loaded.open_path(&path).unwrap();
        assert_eq!(loaded.view_style().mode, cindral_core::StyleMode::XRay);
    }

    #[test]
    fn outgoing_backup_uses_a_different_path_from_an_inflight_autosave() {
        let directory = tempfile::tempdir().unwrap();
        let mut editor = app(directory.path());
        let old_path = editor.files.snapshot.clone();
        let (_sender, receiver) = mpsc::channel();
        editor.files.job = Some(AutosaveJob {
            generation: 0,
            revision: (0, 0),
            receiver,
        });
        editor.backup_current().unwrap();
        assert_ne!(editor.files.snapshot, old_path);
        assert_eq!(
            cindral_io::load_draft(&editor.files.snapshot).unwrap(),
            editor.file_values()
        );
    }
}
