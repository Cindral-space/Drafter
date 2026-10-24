//! File dialogs, atomic saves, asynchronous autosave and explicit recovery.
use super::*;
use cindral_io::{
    DraftBundle, ImportControl, ImportOptions, ImportReport, ImportResult, TextureAsset,
};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::mpsc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
fn sorted_recovery_files(directory: &Path) -> cindral_io::Result<Vec<PathBuf>> {
    let mut files = cindral_io::recovery_files(directory)?;
    files.sort_by_cached_key(|path| {
        (
            std::cmp::Reverse(std::fs::metadata(path).and_then(|m| m.modified()).ok()),
            path.clone(),
        )
    });
    Ok(files)
}

fn snapshot_age(path: &Path) -> String {
    let Some(age) = std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|modified| SystemTime::now().duration_since(modified).ok())
    else {
        return "Snapshot time unavailable".into();
    };
    let seconds = age.as_secs();
    if seconds < 60 {
        "Saved less than a minute ago".into()
    } else if seconds < 3600 {
        format!("Saved {} min ago", seconds / 60)
    } else if seconds < 86400 {
        format!("Saved {} h ago", seconds / 3600)
    } else {
        format!("Saved {} days ago", seconds / 86400)
    }
}

struct AutosaveJob {
    path: PathBuf,
    generation: u64,
    revision: (u64, u64),
    superseded: bool,
    receiver: mpsc::Receiver<Result<PathBuf, String>>,
}
pub(crate) struct PreparedImport {
    result: ImportResult,
    scene: scene_cache::SceneCache,
    update: SceneUpdate,
}
fn prepare_import(
    result: ImportResult,
    control: &ImportControl,
) -> cindral_io::Result<PreparedImport> {
    if result.textures.len() > 127 {
        return Err(cindral_io::IoError::Invalid(
            "combined texture count exceeds 127".into(),
        ));
    }
    let mut scene = scene_cache::SceneCache::default();
    let mut layers = BTreeMap::new();
    let mut pixels = Vec::new();
    for (key, asset) in &result.textures {
        control.check()?;
        let image = cindral_io::decode_texture(asset)?;
        layers.insert(key.clone(), pixels.len() as u32 + 1);
        pixels.push(cindral_render::texture::TexturePixels {
            width: image.width,
            height: image.height,
            rgba: image.rgba,
        });
    }
    scene.set_textures(layers, pixels);
    control.check()?;
    let update = scene.sync(&result.history)?;
    control.check()?;
    Ok(PreparedImport {
        result,
        scene,
        update,
    })
}
pub(crate) struct ImportJob {
    save_epoch: u64,
    pub(crate) replace_document: bool,
    pub(crate) generation: u64,
    pub(crate) revision: u64,
    pub(crate) path: PathBuf,
    pub(crate) control: ImportControl,
    pub(crate) cancelled: bool,
    pub(crate) receiver: mpsc::Receiver<cindral_io::Result<PreparedImport>>,
}
pub(crate) struct FileState {
    pub(crate) dae: crate::dae_ui::DaeUi,
    pub(crate) path: Option<PathBuf>,
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
    close_error: Option<String>,
    close_approved: bool,
    recovered_from: Option<PathBuf>,
    status: String,
    import_save_epoch: u64,
    pending_open: Option<PathBuf>,
    import_dialog: Option<PathBuf>,
    import_new_document: bool,
    import_unit: Option<f64>,
    import_axis: Option<cindral_io::UpAxis>,
    ready_import: Option<PreparedImport>,
    import_assets: BTreeMap<String, TextureAsset>,
    pub(crate) import_options: ImportOptions,
    pub(crate) import_job: Option<ImportJob>,
    pub(crate) import_summary: Option<ImportReport>,
    pub(crate) import_report_open: bool,
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
const RETAIN_SNAPSHOTS: usize = 20;
const RECOVERY_GRACE: Duration = Duration::from_secs(7 * 24 * 60 * 60);

// Retain recent files even when there are many; recovery cleanup must be conservative.
fn prune_recovery(
    directory: &Path,
    protected: &[PathBuf],
    now: SystemTime,
) -> cindral_io::Result<Vec<PathBuf>> {
    let mut files: Vec<_> = cindral_io::recovery_files(directory)?
        .into_iter()
        .filter_map(|path| {
            let name = path.file_name()?.to_str()?;
            let values = name
                .strip_prefix("draft-")?
                .strip_suffix(".cdraft.autosave")?;
            if values.split('-').count() != 3
                || values.split('-').any(|part| part.parse::<u128>().is_err())
            {
                return None;
            }
            let modified = std::fs::metadata(&path).ok()?.modified().ok()?;
            Some((modified, path))
        })
        .collect();
    files.sort_by(|a, b| b.cmp(a));
    let mut removed = Vec::new();
    for (modified, path) in files.into_iter().skip(RETAIN_SNAPSHOTS) {
        if protected.contains(&path)
            || now.duration_since(modified).unwrap_or_default() < RECOVERY_GRACE
        {
            continue;
        }
        match std::fs::remove_file(&path) {
            Ok(()) => removed.push(path),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => removed.push(path),
            Err(e) => tracing::warn!("could not retire recovery snapshot {}: {e}", path.display()),
        }
    }
    Ok(removed)
}

enum CloseAction {
    Retry,
    SaveAs,
    CloseAnyway,
    Cancel,
}
impl Default for FileState {
    fn default() -> Self {
        let directory = state_directory();
        let snapshot = snapshot_path(&directory, 0);
        Self {
            dae: Default::default(),
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
            close_error: None,
            close_approved: false,
            recovered_from: None,
            status: "Untitled draft".into(),
            import_save_epoch: 0,
            pending_open: None,
            import_dialog: None,
            import_new_document: false,
            import_unit: None,
            import_axis: None,
            ready_import: None,
            import_assets: BTreeMap::new(),
            import_options: ImportOptions::default(),
            import_job: None,
            import_summary: None,
            import_report_open: false,
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
    Dae,
    Png,
    Recover(PathBuf),
    Discard(PathBuf),
}
impl DrafterApp {
    pub(crate) fn sync_file_textures(&mut self) -> cindral_io::Result<()> {
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
    fn has_unsaved_values(&self) -> bool {
        self.meaningful() && (self.files.saved_values.is_none() || self.files.modified)
    }
    fn backup_current(&mut self) -> cindral_io::Result<()> {
        self.refresh_save_state();
        if self.has_unsaved_values()
            && !(self.files.last_revision == Some(self.file_revision())
                && self.files.snapshot.is_file())
        {
            std::fs::create_dir_all(&self.files.directory)?;
            // An older background job may still own the previous snapshot path.
            let path = snapshot_path(&self.files.directory, self.files.generation);
            cindral_io::save_autosave(&path, &self.file_values())?;
            self.files.snapshot = path;
            self.files.last_revision = Some(self.file_revision());
            self.retire_recovery();
        }
        Ok(())
    }
    fn reset_file_target(&mut self, path: Option<PathBuf>) {
        self.files.import_assets.clear();
        self.files.generation = self.files.generation.wrapping_add(1);
        self.files.path = path;
        self.files.snapshot = snapshot_path(&self.files.directory, self.files.generation);
        self.files.last_time = Instant::now();
        self.files.last_revision = None;
        self.files.saved_values = None;
        self.files.checked_save_revision = None;
        self.files.modified = false;
        self.files.recovered_from = None;
        self.files.close_error = None;
        self.files.close_approved = false;
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
        self.refresh_save_state();
        if self.has_unsaved_values() {
            self.files.pending_open = Some(path.to_owned());
            return Ok(());
        }
        self.open_path_confirmed(path)
    }
    fn open_path_confirmed(&mut self, path: &Path) -> cindral_io::Result<()> {
        let discarded_import = self.files.import_job.is_some();
        self.cancel_import();
        if path
            .extension()
            .is_some_and(|ext| !ext.eq_ignore_ascii_case("cdraft"))
        {
            self.start_import_mode(
                path.to_owned(),
                ImportOptions {
                    as_component: false,
                    ..self.files.import_options.clone()
                },
                true,
            );
            return Ok(());
        }
        let bundle = cindral_io::load_draft(path)?;
        self.cancel_modify();
        self.tool.cancel();
        self.backup_current()?;
        self.apply_file(bundle, Some(path.to_owned()))?;
        if discarded_import {
            self.files.status.push_str(" · pending import discarded");
        }
        Ok(())
    }
    /// Atomically save committed values; any active drag is cancelled before writing.
    pub fn save_path(&mut self, path: &Path) -> cindral_io::Result<()> {
        if !path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("cdraft"))
        {
            return Err(cindral_io::IoError::Invalid(
                "Save As .cdraft is required".into(),
            ));
        }
        self.cancel_modify();
        self.tool.cancel();
        let values = self.file_values();
        cindral_io::save_draft(path, &values)?;
        self.files.saved_values = Some(values);
        self.files.checked_save_revision = Some(self.file_revision());
        self.files.modified = false;
        if self.files.path.as_deref() != Some(path) {
            self.files.import_save_epoch = self.files.import_save_epoch.wrapping_add(1);
        }
        self.files.path = Some(path.to_owned());
        self.files.last_revision = Some(self.file_revision());
        self.files.status = format!("Saved {}", path.display());
        if let Some(job) = &mut self.files.job {
            if job.generation == self.files.generation {
                job.superseded = true;
            }
        }
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
                let discarded_import = self.files.import_job.is_some();
                self.cancel_import();
                self.cancel_modify();
                self.tool.cancel();
                self.backup_current()?;
                self.files.textures.clear();
                self.set_document_data(History::new().to_data())?;
                self.reset_file_target(None);
                self.sync_file_textures()?;
                self.files.fit = true;
                self.files.status = if discarded_import {
                    "New draft · pending import discarded"
                } else {
                    "New draft"
                }
                .into();
            }
            Action::Open => {
                self.cancel_import();
                if let Some(path) = rfd::FileDialog::new()
                    .add_filter(
                        "Supported documents",
                        &["cdraft", "skp", "dae", "gltf", "glb", "obj"],
                    )
                    .add_filter("SketchUp (*.skp)", &["skp"])
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
                    .add_filter(
                        "Supported documents (*.cdraft, *.skp, *.gltf, *.glb, *.dae, *.obj)",
                        &["cdraft", "skp", "gltf", "glb", "dae", "obj"],
                    )
                    .add_filter("SketchUp (*.skp)", &["skp"])
                    .add_filter("glTF 2.0 (*.gltf, *.glb)", &["gltf", "glb"])
                    .add_filter("COLLADA (*.dae)", &["dae"])
                    .add_filter("Wavefront OBJ (*.obj)", &["obj"])
                    .pick_file()
                {
                    self.files.import_dialog = Some(path);
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
            Action::Png => {
                self.png.open = true;
            }
            Action::Dae => {
                self.files.dae.open = true;
            }
            Action::Recover(path) => {
                self.cancel_import();
                let bundle = cindral_io::load_draft(&path)?;
                self.cancel_modify();
                let previous_source = self.files.recovered_from.replace(path.clone());
                if let Err(e) = self.backup_current() {
                    self.files.recovered_from = previous_source;
                    return Err(e);
                }
                self.apply_file(bundle, None)?;
                self.files.recovered_from = Some(path);
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
                (
                    "Open…",
                    shortcuts::hint(shortcuts::Action::Open),
                    Action::Open,
                ),
                (
                    "Save",
                    shortcuts::hint(shortcuts::Action::Save),
                    Action::Save,
                ),
                (
                    "Save As…",
                    shortcuts::hint(shortcuts::Action::SaveAs),
                    Action::SaveAs,
                ),
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
            if ui.button("Import…").clicked() {
                action = Some(Action::Import);
                ui.close();
            }
            ui.menu_button("Export", |ui| {
                for (text, a) in [
                    ("OBJ…", Action::Obj),
                    ("STL…", Action::Stl),
                    ("glTF 2.0…", Action::Gltf),
                    ("COLLADA…", Action::Dae),
                    ("Image (PNG)…", Action::Png),
                ] {
                    if ui.button(text).clicked() {
                        action = Some(a);
                        ui.close();
                    }
                }
            });
            ui.separator();
            if ui.button("Recovery snapshots").clicked() {
                self.open_recovery_panel();
                ui.close();
            }
        });
        if let Some(action) = action {
            if let Err(e) = self.do_file_action(action) {
                self.error = Some(e.to_string());
            }
        }
    }
    pub(crate) fn file_tick(&mut self, ctx: &egui::Context) {
        self.dae_window(ctx);
        self.png_window(ctx);
        self.import_tick(ctx);
        self.import_window(ctx);
        if !self.files.checked {
            self.files.checked = true;
            match sorted_recovery_files(&self.files.directory) {
                Ok(files) => {
                    self.files.offer = !files.is_empty();
                    self.files.candidates = files;
                }
                Err(e) => self.error = Some(e.to_string()),
            }
        }
        let shortcut = if ctx.wants_keyboard_input()
            || !self.measurements.text.is_empty()
            || self.shortcuts.open
        {
            None
        } else {
            ctx.input(|i| {
                if shortcuts::pressed(i, shortcuts::Action::SaveAs) {
                    Some(Action::SaveAs)
                } else if shortcuts::pressed(i, shortcuts::Action::Save) {
                    Some(Action::Save)
                } else if shortcuts::pressed(i, shortcuts::Action::Open) {
                    Some(Action::Open)
                } else {
                    None
                }
            })
        };
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
    pub(crate) fn recovery_count(&self) -> usize {
        self.files.candidates.len()
    }
    pub(crate) fn recovery_offer(&mut self) {
        if self.files.offer {
            self.files.offer = false;
            self.show_tray = true;
            self.inspector_tab = workspace_ui::InspectorTab::Recovery;
        }
    }
    fn open_recovery_panel(&mut self) {
        match sorted_recovery_files(&self.files.directory) {
            Ok(files) => self.files.candidates = files,
            Err(e) => self.error = Some(e.to_string()),
        }
        self.show_tray = true;
        self.inspector_tab = workspace_ui::InspectorTab::Recovery;
    }
    pub(crate) fn recovery_panel(&mut self, ui: &mut egui::Ui) {
        ui.strong("Recovery snapshots");
        ui.label("Recover opens an unsaved draft. Closing this panel keeps your snapshots.");
        if ui.button("Refresh").clicked() {
            self.open_recovery_panel();
        }
        if self.files.candidates.is_empty() {
            ui.weak("No recovery snapshots found.");
        }
        let mut action = None;
        for (index, path) in self.files.candidates.iter().enumerate() {
            ui.separator();
            let filename = path.file_name().unwrap_or_default().to_string_lossy();
            let name = if filename
                .strip_prefix("draft-")
                .and_then(|name| name.strip_suffix(".cdraft.autosave"))
                .is_some_and(|parts| {
                    parts.split('-').count() == 3
                        && parts.split('-').all(|part| part.parse::<u128>().is_ok())
                }) {
                format!("Recovery snapshot {}", index + 1)
            } else {
                filename.trim_end_matches(".cdraft.autosave").to_owned()
            };
            ui.add(egui::Label::new(egui::RichText::new(name).strong()).wrap())
                .on_hover_text(path.display().to_string());
            ui.weak(snapshot_age(path));
            ui.horizontal(|ui| {
                if ui.button("Recover").clicked() {
                    action = Some(Action::Recover(path.clone()));
                }
                if ui
                    .add(
                        egui::Button::new("×")
                            .frame(false)
                            .min_size(egui::vec2(28., 28.)),
                    )
                    .on_hover_text("Permanently delete this recovery snapshot")
                    .clicked()
                {
                    action = Some(Action::Discard(path.clone()));
                }
            });
        }
        if let Some(action) = action {
            let recovering = matches!(action, Action::Recover(_));
            match self.do_file_action(action) {
                Ok(()) if recovering => self.inspector_tab = workspace_ui::InspectorTab::Model,
                Ok(()) => {}
                Err(e) => self.error = Some(e.to_string()),
            }
        }
    }
    pub(crate) fn autosave_tick(&mut self, ctx: &egui::Context) {
        if let Some(job) = self.files.job.take() {
            match job.receiver.try_recv() {
                Ok(Ok(path)) => {
                    if job.superseded {
                        if let Err(e) = std::fs::remove_file(&path) {
                            if e.kind() != std::io::ErrorKind::NotFound {
                                tracing::warn!(
                                    "could not remove superseded recovery snapshot: {e}"
                                );
                            }
                        }
                        self.files.candidates.retain(|p| p != &path);
                    } else if job.generation == self.files.generation {
                        self.files.last_revision = Some(job.revision);
                        self.files.status = format!(
                            "Autosaved recovery snapshot {}",
                            path.file_name().unwrap_or_default().to_string_lossy()
                        );
                    }
                    self.retire_recovery();
                }
                Ok(Err(e)) => self.error = Some(format!("Autosave failed: {e}")),
                Err(mpsc::TryRecvError::Empty) => self.files.job = Some(job),
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.error = Some("Autosave worker stopped before completion".into())
                }
            }
        }
        let busy = self.modify.active()
            || self.organization.drag.is_some()
            || self.files.import_job.is_some()
            || self.files.close_error.is_some()
            || self.files.close_approved;
        if !busy {
            self.refresh_save_state();
        }
        if self.files.job.is_none()
            && !busy
            && self.has_unsaved_values()
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
            let job_path = path.clone();
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
                        path: job_path,
                        generation,
                        revision,
                        superseded: false,
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
    /// Preserve committed values before a normal exit. A fresh path isolates any in-flight writer.
    pub(crate) fn preserve_on_close(&mut self) -> cindral_io::Result<()> {
        if self.files.close_approved {
            return Ok(());
        }
        self.cancel_import();
        self.cancel_section();
        self.cancel_modify();
        self.tool.cancel();
        self.refresh_save_state();
        if !self.has_unsaved_values()
            || (self.files.last_revision == Some(self.file_revision())
                && self.files.snapshot.is_file())
        {
            return Ok(());
        }
        self.backup_current()?;
        self.files.last_revision = Some(self.file_revision());
        Ok(())
    }
    fn retire_recovery(&mut self) {
        let mut protected = vec![self.files.snapshot.clone()];
        if let Some(job) = &self.files.job {
            protected.push(job.path.clone());
        }
        if let Some(path) = &self.files.recovered_from {
            protected.push(path.clone());
        }
        match prune_recovery(&self.files.directory, &protected, SystemTime::now()) {
            Ok(removed) => self.files.candidates.retain(|p| !removed.contains(p)),
            Err(e) => tracing::warn!("could not retire recovery snapshots: {e}"),
        }
    }
    fn approve_close(&mut self, ctx: &egui::Context) {
        self.files.close_approved = true;
        self.files.close_error = None;
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }
    fn save_before_close(&mut self, path: Option<PathBuf>, ctx: &egui::Context) {
        let Some(mut path) = path else { return };
        path.set_extension("cdraft");
        match self.save_path(&path) {
            Ok(()) => self.approve_close(ctx),
            Err(e) => self.files.close_error = Some(format!("Could not save the draft: {e}")),
        }
    }
    fn close_action(&mut self, action: CloseAction, ctx: &egui::Context) {
        match action {
            CloseAction::Retry => match self.preserve_on_close() {
                Ok(()) => self.approve_close(ctx),
                Err(e) => self.files.close_error = Some(e.to_string()),
            },
            CloseAction::SaveAs => {
                let path = rfd::FileDialog::new()
                    .add_filter("Cindral Drafter", &["cdraft"])
                    .set_file_name("model.cdraft")
                    .save_file();
                self.save_before_close(path, ctx);
            }
            CloseAction::CloseAnyway => self.approve_close(ctx),
            CloseAction::Cancel => self.files.close_error = None,
        }
    }
    /// Handle native close and let the user choose how to proceed after a failed backup.
    pub(crate) fn closing(&mut self, ctx: &egui::Context) -> bool {
        if ctx.input(|i| i.viewport().close_requested()) {
            if self.files.close_approved {
                return true;
            }
            if self.files.close_error.is_none() {
                match self.preserve_on_close() {
                    Ok(()) => {
                        self.files.close_approved = true;
                        return true;
                    }
                    Err(e) => self.files.close_error = Some(e.to_string()),
                }
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
        }
        if let Some(error) = self.files.close_error.clone() {
            let mut action = None;
            egui::Modal::new(egui::Id::new("close-recovery-failed")).show(ctx, |ui| {
                ui.set_max_width(460.);
                ui.heading("Couldn't create a recovery copy");
                ui.label(
                    "Save your draft elsewhere, retry, or close without saving the latest changes.",
                );
                ui.label(error);
                ui.horizontal(|ui| {
                    for (label, choice) in [
                        ("Save As…", CloseAction::SaveAs),
                        ("Retry", CloseAction::Retry),
                        ("Close anyway", CloseAction::CloseAnyway),
                        ("Keep editing", CloseAction::Cancel),
                    ] {
                        if ui.button(label).clicked() {
                            action = Some(choice);
                        }
                    }
                });
            });
            if let Some(action) = action {
                self.close_action(action, ctx);
            }
        }
        self.files.close_approved
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
    /// Cancel any active asynchronous import worker and discard its result.
    pub fn cancel_import(&mut self) {
        self.files.ready_import = None;
        if let Some(mut job) = self.files.import_job.take() {
            job.control.cancel();
            job.cancelled = true;
            self.files.status = "Import cancelled".into();
        }
    }
    /// Spawn a worker thread to import a 3D model file asynchronously.
    pub fn start_import(&mut self, path: PathBuf, options: ImportOptions) {
        self.start_import_mode(path, options, false);
    }
    fn start_import_mode(&mut self, path: PathBuf, options: ImportOptions, replace_document: bool) {
        self.cancel_import();
        self.files.import_summary = None;
        let control = ImportControl::default();
        let thread_control = control.clone();
        let thread_path = path.clone();
        let revision = self.history.revision();
        let thread_options = options;
        let unit = self.files.import_unit;
        let axis = self.files.import_axis;
        let (sender, receiver) = mpsc::channel();
        let spawned = std::thread::Builder::new()
            .name("cindral-import".into())
            .spawn(move || {
                let res = cindral_io::import_file_with_basis(
                    &thread_path,
                    &thread_options,
                    &thread_control,
                    unit,
                    axis,
                )
                .and_then(|result| prepare_import(result, &thread_control));
                let _ = sender.send(res);
            });
        match spawned {
            Ok(_) => {
                self.files.import_job = Some(ImportJob {
                    replace_document,
                    save_epoch: self.files.import_save_epoch,
                    generation: self.files.generation,
                    revision,
                    path,
                    control,
                    cancelled: false,
                    receiver,
                });
                self.files.status = "Importing model…".into();
            }
            Err(e) => {
                self.error = Some(format!("Could not spawn import thread: {e}"));
            }
        }
    }
    /// Install the result of a completed import, keeping its single undo step.
    fn apply_import_result(
        &mut self,
        prepared: PreparedImport,
        path: &Path,
    ) -> cindral_io::Result<()> {
        self.install_import_result(prepared, path, true)
    }
    fn install_import_result(
        &mut self,
        prepared: PreparedImport,
        path: &Path,
        replace: bool,
    ) -> cindral_io::Result<()> {
        let PreparedImport {
            result,
            scene,
            mut update,
        } = prepared;
        let ImportResult {
            history,
            textures,
            report,
        } = result;
        self.history = history;
        self.files.textures = textures;
        let new_definitions = scene.definitions();
        update.removed.extend(
            self.scene
                .definitions()
                .into_iter()
                .filter(|id| !new_definitions.contains(id)),
        );
        self.scene = scene;
        scene_cache::merge(&mut self.pending, update);
        self.organization = Default::default();
        self.selection.clear();
        if replace {
            self.guide_cache.clear();
            self.guide_definition = cindral_core::DefinitionId(0);
            self.modify.replace_guides(Vec::new());
        }
        self.modify.cancel();
        self.rotate = Default::default();
        self.measurements.reset();
        self.tool.cancel();
        self.refresh_mesh();
        self.last_tape = None;
        self.files.fit = true;
        self.files.offer = false;
        self.error = None;
        self.files.status = format!(
            "Opened as new unsaved document: {} · Save As .cdraft",
            path.file_name().unwrap_or_default().to_string_lossy()
        );
        if path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("skp"))
        {
            self.files
                .status
                .push_str(" · imported from .skp · Save As .cdraft");
        }
        self.files.import_summary = Some(report);
        self.files.import_report_open = true;
        Ok(())
    }
    fn import_operation_active(&self) -> bool {
        self.modify.active()
            || self.tool.anchor().is_some()
            || self.organization.drag.is_some()
            || self.draw_press.is_some()
            || self.modify_press.is_some()
            || self.history.has_active_preview()
            || self.section_cursor_active()
    }
    pub(crate) fn import_tick(&mut self, ctx: &egui::Context) {
        let Some(job) = self.files.import_job.take() else {
            return;
        };
        if job.control.is_cancelled()
            || job.cancelled
            || job.generation != self.files.generation
            || job.save_epoch != self.files.import_save_epoch
            || (job.replace_document && job.revision != self.history.revision())
        {
            job.control.cancel();
            self.files.ready_import = None;
            self.files.status = "Import discarded: document closed, replaced or saved under another name (or edited during Open)".into();
            return;
        }
        let result = self
            .files
            .ready_import
            .take()
            .map(|p| Ok(Ok(p)))
            .unwrap_or_else(|| job.receiver.try_recv());
        match result {
            Ok(Ok(prepared)) if self.import_operation_active() => {
                self.files.ready_import = Some(prepared);
                self.files.status = "Import ready, applying...".into();
                self.files.import_job = Some(job);
                ctx.request_repaint();
            }
            Ok(Ok(mut prepared)) => {
                let result = if job.replace_document {
                    History::from_data(prepared.result.history.to_data())
                        .map_err(cindral_io::IoError::from)
                        .and_then(|history| {
                            prepared.result.history = history;
                            self.backup_current()?;
                            self.reset_file_target(None);
                            self.apply_import_result(prepared, &job.path)
                        })
                } else {
                    self.append_import_result(prepared, &job.path)
                };
                if let Err(e) = result {
                    self.error = Some(format!("Import failed: {e}"));
                    self.files.status = "Import failed".into();
                }
            }
            Ok(Err(e)) => {
                self.error = Some(format!("Import failed: {e}"));
                self.files.status = "Import failed".into();
            }
            Err(mpsc::TryRecvError::Empty) => {
                self.files.import_job = Some(job);
                ctx.request_repaint();
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                self.error = Some("Import worker stopped unexpectedly".into());
                self.files.status = "Import failed".into();
            }
        }
    }
    fn append_import_result(
        &mut self,
        prepared: PreparedImport,
        path: &Path,
    ) -> cindral_io::Result<()> {
        let mut result = prepared.result;
        // Resolve texture identity by encoded content before comparing materials.
        let mut incoming = result.history.to_data();
        let mut assets = self.files.textures.clone();
        let mut remap = BTreeMap::new();
        for (key, asset) in &result.textures {
            let target = if let Some((key, _)) = assets.iter().find(|(_, old)| *old == asset) {
                key.clone()
            } else {
                let mut target = key.clone();
                let mut suffix = 2;
                while assets.contains_key(&target) {
                    target = format!("{key} ({suffix})");
                    suffix += 1;
                }
                assets.insert(target.clone(), asset.clone());
                target
            };
            remap.insert(key.clone(), target);
        }
        for material in incoming.materials.values_mut() {
            if let Some(key) = &material.texture {
                material.texture = remap.get(key).cloned().or_else(|| Some(key.clone()));
            }
        }
        let used: std::collections::BTreeSet<_> = incoming
            .materials
            .values()
            .filter_map(|m| m.texture.as_ref())
            .collect();
        result.textures = assets
            .into_iter()
            .filter(|(key, _)| used.contains(key) && !self.files.textures.contains_key(key))
            .collect();
        let mut history = self.history.clone();
        history.enter(Vec::new())?;
        let active = history.active_definition();
        let before: std::collections::BTreeSet<_> = history
            .instances(active)
            .ok_or(cindral_core::KernelError::MissingEntity)?
            .keys()
            .copied()
            .collect();
        let counts = history.append_import_report(incoming)?;
        let selected = history
            .instances(active)
            .ok_or(cindral_core::KernelError::MissingEntity)?
            .keys()
            .find(|id| !before.contains(id))
            .copied();
        let mut textures = self.files.textures.clone();
        for (key, asset) in &result.textures {
            if textures.get(key).is_some_and(|old| old != asset) {
                return Err(cindral_io::IoError::Invalid(
                    "texture slot collision".into(),
                ));
            }
            textures.insert(key.clone(), asset.clone());
        }
        let assets: BTreeMap<_, _> = result
            .textures
            .iter()
            .filter(|(key, _)| !self.files.textures.contains_key(*key))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        result.history = history;
        result.textures = textures;
        result.report.notes.push(format!(
            "Current model: merged {} tags / {} materials; renamed {} materials / {} definitions",
            counts[0], counts[1], counts[2], counts[3]
        ));
        let prepared = prepare_import(result, &ImportControl::default())?;
        let count = prepared
            .result
            .history
            .instances(active)
            .ok_or(cindral_core::KernelError::MissingEntity)?
            .keys()
            .filter(|id| !before.contains(id))
            .count();
        let count = if count == 0 {
            prepared.result.report.faces + prepared.result.report.edges
        } else {
            count
        };
        self.install_import_result(prepared, path, false)?;
        self.files.import_assets.extend(assets);
        if let Some(id) = selected {
            let mut path = self.history.context().to_vec();
            path.push(id);
            self.organization.selected = Some(path.clone());
            self.frame_import_if_outside(&path);
        }
        self.files.fit = false;
        self.files.status = format!(
            "Added {count} objects from {} into current model",
            path.display()
        );
        Ok(())
    }
    fn frame_import_if_outside(&mut self, path: &[cindral_core::InstanceId]) {
        let Some((min, max)) = self.selected_world_bounds(path) else {
            return;
        };
        let aspect = self.png.viewport[0].max(1) as f32 / self.png.viewport[1].max(1) as f32;
        let vp = self.camera.view_projection(aspect);
        let mut outside = false;
        for x in [min.x, max.x] {
            for y in [min.y, max.y] {
                for z in [min.z, max.z] {
                    let p = vp * DVec3::new(x, y, z).as_vec3().extend(1.);
                    outside |=
                        p.w <= 0. || p.x.abs() > p.w || p.y.abs() > p.w || p.z < 0. || p.z > p.w;
                }
            }
        }
        if outside {
            self.camera
                .zoom_extents(min.as_vec3(), max.as_vec3(), aspect);
        }
    }
    pub(crate) fn reconcile_import_textures(&mut self) -> cindral_io::Result<()> {
        let keys: std::collections::BTreeSet<_> = self
            .history
            .materials()
            .values()
            .filter_map(|m| m.texture.as_ref())
            .collect();
        for (key, asset) in &self.files.import_assets {
            if keys.contains(key) {
                self.files.textures.insert(key.clone(), asset.clone());
            } else {
                self.files.textures.remove(key);
            }
        }
        self.sync_file_textures()
    }
    fn resolve_open_choice(&mut self, choice: u8) {
        let Some(path) = self.files.pending_open.clone() else {
            return;
        };
        if choice == 2 {
            self.files.pending_open = None;
            return;
        }
        let result = if choice == 0 {
            self.do_file_action(Action::Save)
        } else {
            Ok(())
        };
        self.refresh_save_state();
        match result {
            Err(e) => self.error = Some(e.to_string()),
            Ok(()) if choice == 1 || !self.has_unsaved_values() => {
                self.files.pending_open = None;
                if let Err(e) = self.open_path_confirmed(&path) {
                    self.error = Some(e.to_string());
                }
            }
            _ => {}
        }
    }
    fn import_options_window(&mut self, ctx: &egui::Context) {
        if let Some(path) = self.files.pending_open.clone() {
            let mut choice = None;
            egui::Modal::new(egui::Id::new("open-unsaved")).show(ctx, |ui| {
                ui.heading("Save changes before opening?");
                ui.label(format!("Open {} as its own document", path.display()));
                ui.horizontal(|ui| {
                    if ui.button("Save").clicked() {
                        choice = Some(0);
                    }
                    if ui.button("Discard").clicked() {
                        choice = Some(1);
                    }
                    if ui.button("Cancel").clicked() {
                        choice = Some(2);
                    }
                });
            });
            if let Some(choice) = choice {
                self.resolve_open_choice(choice);
            }
        }
        if let Some(path) = self.files.import_dialog.clone() {
            let mut start = false;
            let mut cancel = false;
            egui::Modal::new(egui::Id::new("import-options")).show(ctx, |ui| {
                ui.heading("Import options");
                ui.label(path.display().to_string());
                ui.radio_value(
                    &mut self.files.import_new_document,
                    false,
                    "Add into current model",
                );
                ui.radio_value(
                    &mut self.files.import_new_document,
                    true,
                    "Open as new document",
                );
                ui.checkbox(
                    &mut self.files.import_options.as_component,
                    "As one component (otherwise loose geometry)",
                );
                ui.checkbox(
                    &mut self.files.import_options.merge_coplanar,
                    "Merge coplanar faces",
                );
                ui.checkbox(
                    &mut self.files.import_options.soften_coplanar,
                    "Soften coplanar edges",
                );
                egui::ComboBox::from_label("Source units")
                    .selected_text(
                        self.files
                            .import_unit
                            .map_or("From file".into(), |v| format!("{v} meters/unit")),
                    )
                    .show_ui(ui, |ui| {
                        for (label, value) in [
                            ("From file", None),
                            ("Meters", Some(1.)),
                            ("Centimeters", Some(0.01)),
                            ("Millimeters", Some(0.001)),
                            ("Inches", Some(0.0254)),
                            ("Feet", Some(0.3048)),
                        ] {
                            ui.selectable_value(&mut self.files.import_unit, value, label);
                        }
                    });
                egui::ComboBox::from_label("Up axis")
                    .selected_text(format!("{:?}", self.files.import_axis))
                    .show_ui(ui, |ui| {
                        for (label, axis) in [
                            ("From file", None),
                            ("X", Some(cindral_io::UpAxis::X)),
                            ("Y", Some(cindral_io::UpAxis::Y)),
                            ("Z", Some(cindral_io::UpAxis::Z)),
                        ] {
                            ui.selectable_value(&mut self.files.import_axis, axis, label);
                        }
                    });
                ui.horizontal(|ui| {
                    start = ui.button("Continue").clicked();
                    cancel = ui.button("Cancel").clicked();
                });
            });
            if cancel {
                self.files.import_dialog = None;
            }
            if start {
                self.files.import_dialog = None;
                if self.files.import_new_document {
                    if let Err(e) = self.open_path(&path) {
                        self.error = Some(e.to_string());
                    }
                } else {
                    self.start_import(path, self.files.import_options.clone());
                }
            }
        }
    }
    pub(crate) fn import_window(&mut self, ctx: &egui::Context) {
        self.import_options_window(ctx);
        if let Some(job) = &self.files.import_job {
            let mut cancel = false;
            let file_name = job
                .path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            let progress = job.control.progress().clamp(0.0, 1.0);
            egui::Window::new("Importing Model")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.set_min_width(280.0);
                    if self.files.ready_import.is_some() {
                        ui.label("Import ready, applying...");
                    } else {
                        ui.label(format!("Importing {file_name}…"));
                    }
                    ui.add_space(4.0);
                    ui.add(egui::ProgressBar::new(progress).show_percentage());
                    ui.add_space(8.0);
                    if ui.button("Cancel").clicked() {
                        cancel = true;
                    }
                });
            if cancel {
                self.cancel_import();
            }
        } else if let Some(summary) = self
            .files
            .import_summary
            .as_ref()
            .filter(|_| self.files.import_report_open)
        {
            let mut open = true;
            let mut dismiss = false;
            egui::Window::new("Import Summary")
                .collapsible(false)
                .open(&mut open)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.set_min_width(280.0);
                    ui.label("Import completed:");
                    ui.add_space(4.0);
                    egui::Grid::new("import_summary_grid")
                        .num_columns(2)
                        .spacing([16.0, 4.0])
                        .show(ui, |ui| {
                            ui.label("Vertices:");
                            ui.label(summary.vertices.to_string());
                            ui.end_row();
                            ui.label("Edges:");
                            ui.label(summary.edges.to_string());
                            ui.end_row();
                            ui.label("Faces:");
                            ui.label(summary.faces.to_string());
                            ui.end_row();
                            ui.label("Instances:");
                            ui.label(summary.instances.to_string());
                            ui.end_row();
                            if summary.skipped > 0 {
                                ui.label("Skipped:");
                                ui.label(summary.skipped.to_string());
                                ui.end_row();
                            }
                        });
                    ui.small("Geometry counted once per stored definition; instances include import wrappers.");
                    for note in &summary.notes { ui.label(note); }
                    if !summary.warnings.is_empty() {
                        ui.add_space(6.0);
                        ui.label(format!("Repairs and warnings ({}):", summary.warnings.len()));
                        egui::ScrollArea::vertical()
                            .max_height(100.0)
                            .show(ui, |ui| {
                                for w in &summary.warnings {
                                    ui.label(format!("• {w}"));
                                }
                            });
                    }
                    if !summary.not_imported.is_empty() {
                        ui.add_space(6.0);
                        ui.label("Not imported");
                        for feature in &summary.not_imported { ui.label(feature); }
                    }
                    ui.add_space(8.0);
                    if ui.button("OK").clicked() {
                        dismiss = true;
                    }
                });
            if !open || dismiss {
                self.files.import_report_open = false;
            }
        }
    }
}

#[cfg(feature = "kernel-benchmark")]
impl DrafterApp {
    pub(super) fn isolate_pan_replay(&mut self) {
        self.files.import_summary = None;
        self.files.checked = true;
        self.files.offer = false;
        self.files.interval = Duration::from_secs(86400);
    }
    pub(super) fn load_replay(&mut self, path: &Path) -> cindral_io::Result<()> {
        let control = ImportControl::default();
        let result = cindral_io::import_file(
            path,
            &ImportOptions {
                as_component: false,
                ..Default::default()
            },
            &control,
        )?;
        let prepared = prepare_import(result, &control)?;
        self.apply_import_result(prepared, path)?;
        // Replay starts after dismissing the summary, with recovery/autosave isolated.
        self.files.import_summary = None;
        self.files.checked = true;
        self.files.offer = false;
        self.files.interval = Duration::from_secs(86400);
        Ok(())
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
    fn recovery_inspector_discovers_once_preserves_dismissed_files_and_refreshes() {
        let directory = tempfile::tempdir().unwrap();
        let mut editor = app(directory.path());
        let first = directory.path().join("z-old.cdraft.autosave");
        let second = directory.path().join("a-new.cdraft.autosave");
        for (path, seconds) in [(&first, 100), (&second, 200)] {
            let file = std::fs::File::create(path).unwrap();
            file.set_modified(UNIX_EPOCH + Duration::from_secs(seconds))
                .unwrap();
        }
        editor.files.checked = false;
        editor.show_tray = false;
        editor.file_tick(&egui::Context::default());
        editor.recovery_offer();
        assert!(editor.show_tray);
        assert!(editor.inspector_tab == workspace_ui::InspectorTab::Recovery);
        assert_eq!(editor.files.candidates, vec![second.clone(), first.clone()]);
        editor.show_tray = false;
        editor.file_tick(&egui::Context::default());
        editor.recovery_offer();
        assert!(!editor.show_tray, "dismissal must survive later frames");
        assert!(first.exists() && second.exists());
        editor.open_recovery_panel();
        assert!(editor.show_tray);
        editor.do_file_action(Action::Discard(second)).unwrap();
        editor.do_file_action(Action::Discard(first)).unwrap();
        editor.open_recovery_panel();
        assert_eq!(editor.recovery_count(), 0);
        assert!(editor.inspector_tab == workspace_ui::InspectorTab::Recovery);
    }

    #[test]
    fn recovery_inspector_button_recovers_unsaved_values_and_retains_source() {
        let directory = tempfile::tempdir().unwrap();
        let source = app(directory.path());
        let path = directory.path().join("named.cdraft.autosave");
        let expected = source.file_values();
        cindral_io::save_autosave(&path, &expected).unwrap();
        let mut editor = app(directory.path());
        editor
            .history
            .execute(Command::AddEdge {
                start: DVec3::ZERO,
                end: DVec3::Z,
            })
            .unwrap();
        editor.open_recovery_panel();
        let ctx = egui::Context::default();
        workspace_ui::configure_style(&ctx);
        let frame =
            |editor: &mut DrafterApp, input| ctx.run(input, |ctx| editor.model_inspector(ctx));
        // The first layout pass initializes fonts and panel sizing.
        let _ = frame(&mut editor, egui::RawInput::default());
        let output = frame(&mut editor, egui::RawInput::default());
        let position = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.text() == "Recover" => {
                    Some(text.visual_bounding_rect().center())
                }
                _ => None,
            })
            .expect("Recovery tab must expose its Recover button");
        for pressed in [true, false] {
            let input = egui::RawInput {
                events: vec![
                    egui::Event::PointerMoved(position),
                    egui::Event::PointerButton {
                        pos: position,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
                ..Default::default()
            };
            let _ = frame(&mut editor, input);
        }
        assert_eq!(editor.file_values(), expected);
        assert!(editor.files.path.is_none());
        assert!(path.exists());
        assert_eq!(editor.files.recovered_from, Some(path));
        assert_eq!(editor.history.undo_depth(), 0);
        assert!(editor.inspector_tab == workspace_ui::InspectorTab::Model);
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
            path: editor.files.snapshot.clone(),
            generation: editor.files.generation,
            revision: editor.file_revision(),
            superseded: false,
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
        editor.open_path_confirmed(&path).unwrap();
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
        loaded.open_path_confirmed(&path).unwrap();
        assert_eq!(loaded.file_values(), expected);
        assert_eq!(loaded.history.undo_depth(), 0);
        assert!(loaded.history.context().is_empty());
        let ctx = egui::Context::default();
        source.set_view_style(cindral_core::ViewStyle {
            mode: cindral_core::StyleMode::XRay,
            ..Default::default()
        });
        let expected_recovery = source.file_values();
        source.autosave_tick(&ctx);
        let deadline = Instant::now() + Duration::from_secs(10);
        while source.files.job.is_some() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
            source.autosave_tick(&ctx);
        }
        assert!(source.files.job.is_none(), "autosave worker timed out");
        assert_eq!(
            cindral_io::load_draft(&source.files.snapshot).unwrap(),
            expected_recovery
        );
        let mut recovered = app(directory.path());
        recovered
            .do_file_action(Action::Recover(source.files.snapshot.clone()))
            .unwrap();
        assert_eq!(recovered.file_values(), expected_recovery);
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
        assert!(app.open_path_confirmed(&path).is_err());
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
            path: editor.files.snapshot.clone(),
            generation: editor.files.generation,
            revision: old_revision,
            superseded: false,
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
        loaded.open_path_confirmed(&path).unwrap();
        assert_eq!(loaded.view_style().mode, cindral_core::StyleMode::XRay);
    }

    #[test]
    fn outgoing_backup_uses_a_different_path_from_an_inflight_autosave() {
        let directory = tempfile::tempdir().unwrap();
        let mut editor = app(directory.path());
        let old_path = editor.files.snapshot.clone();
        let (_sender, receiver) = mpsc::channel();
        editor.files.job = Some(AutosaveJob {
            path: editor.files.snapshot.clone(),
            generation: 0,
            revision: (0, 0),
            superseded: false,
            receiver,
        });
        editor.backup_current().unwrap();
        assert_ne!(editor.files.snapshot, old_path);
        assert_eq!(
            cindral_io::load_draft(&editor.files.snapshot).unwrap(),
            editor.file_values()
        );
    }

    #[test]
    fn manual_save_supersedes_pending_autosave_without_regressing_saved_revision() {
        let directory = tempfile::tempdir().unwrap();
        let mut editor = app(directory.path());
        let old_values = editor.file_values();
        let old_revision = editor.file_revision();
        let snapshot = editor.files.snapshot.clone();
        let (sender, receiver) = mpsc::channel();
        editor.files.job = Some(AutosaveJob {
            path: editor.files.snapshot.clone(),
            generation: editor.files.generation,
            revision: old_revision,
            superseded: false,
            receiver,
        });
        editor.set_view_style(cindral_core::ViewStyle {
            mode: cindral_core::StyleMode::XRay,
            ..Default::default()
        });
        let saved_revision = editor.file_revision();
        let path = directory.path().join("saved.cdraft");
        editor.save_path(&path).unwrap();
        let saved_values = editor.file_values();
        // The old worker publishes only after the manual save completes.
        cindral_io::save_autosave(&snapshot, &old_values).unwrap();
        editor.files.candidates.push(snapshot.clone());
        sender.send(Ok(snapshot.clone())).unwrap();
        editor.autosave_tick(&egui::Context::default());
        assert_eq!(editor.files.last_revision, Some(saved_revision));
        assert!(editor.files.job.is_none());
        assert!(!snapshot.exists());
        assert!(editor.files.candidates.is_empty());
        assert_eq!(cindral_io::load_draft(&path).unwrap(), saved_values);

        // A later edit must still get its own recovery checkpoint.
        editor.set_view_style(Default::default());
        editor.preserve_on_close().unwrap();
        assert_eq!(
            cindral_io::load_draft(&editor.files.snapshot).unwrap(),
            editor.file_values()
        );
    }

    #[test]
    fn closing_preserves_latest_committed_values_despite_a_late_worker() {
        let directory = tempfile::tempdir().unwrap();
        let mut editor = app(directory.path());
        let old_values = editor.file_values();
        let old_path = editor.files.snapshot.clone();
        let (_sender, receiver) = mpsc::channel();
        editor.files.job = Some(AutosaveJob {
            path: editor.files.snapshot.clone(),
            generation: editor.files.generation,
            revision: editor.file_revision(),
            superseded: false,
            receiver,
        });
        editor
            .history
            .execute(Command::AddEdge {
                start: DVec3::ZERO,
                end: DVec3::Z,
            })
            .unwrap();
        let expected = editor.file_values();
        editor
            .history
            .preview(Command::AddEdge {
                start: DVec3::Z,
                end: DVec3::Z * 2.,
            })
            .unwrap();
        editor.preserve_on_close().unwrap();
        let final_path = editor.files.snapshot.clone();
        assert_ne!(final_path, old_path);
        assert_eq!(cindral_io::load_draft(&final_path).unwrap(), expected);
        cindral_io::save_autosave(&old_path, &old_values).unwrap();
        assert_eq!(cindral_io::load_draft(&final_path).unwrap(), expected);
        // on_exit repeats the close hook; it must not create another backup.
        editor.preserve_on_close().unwrap();
        assert_eq!(editor.files.snapshot, final_path);
    }

    #[test]
    fn closing_empty_or_explicitly_saved_documents_creates_no_recovery() {
        let directory = tempfile::tempdir().unwrap();
        let mut editor = app(directory.path());
        let path = directory.path().join("saved.cdraft");
        editor.save_path(&path).unwrap();
        editor.preserve_on_close().unwrap();
        assert!(cindral_io::recovery_files(directory.path())
            .unwrap()
            .is_empty());
        editor.do_file_action(Action::New).unwrap();
        let before = cindral_io::recovery_files(directory.path()).unwrap();
        editor.preserve_on_close().unwrap();
        assert_eq!(
            cindral_io::recovery_files(directory.path()).unwrap(),
            before
        );
    }

    #[test]
    fn recovery_failure_cancels_native_close_and_keeps_document() {
        let directory = tempfile::tempdir().unwrap();
        let mut editor = app(directory.path());
        let blocked = directory.path().join("not-a-directory");
        std::fs::write(&blocked, b"blocked").unwrap();
        editor.files.directory = blocked;
        editor.files.interval = Duration::MAX;
        let expected = editor.file_values();
        let ctx = egui::Context::default();
        let mut input = egui::RawInput::default();
        input
            .viewports
            .get_mut(&egui::ViewportId::ROOT)
            .unwrap()
            .events
            .push(egui::ViewportEvent::Close);
        let output = ctx.run(input.clone(), |ctx| editor.draw_frame(ctx));
        assert!(output.viewport_output[&egui::ViewportId::ROOT]
            .commands
            .contains(&egui::ViewportCommand::CancelClose));
        assert!(editor.files.close_error.is_some());
        assert_eq!(editor.file_values(), expected);
        // Fixing the destination permits a later close attempt to preserve the draft.
        editor.files.directory = directory.path().to_owned();
        let repeated = ctx.run(input, |ctx| editor.draw_frame(ctx));
        assert!(repeated.viewport_output[&egui::ViewportId::ROOT]
            .commands
            .contains(&egui::ViewportCommand::CancelClose));
        assert!(!editor.files.close_approved);
        assert!(!editor.files.snapshot.exists());
        editor.close_action(CloseAction::Retry, &ctx);
        assert!(editor.files.close_approved);
        assert_eq!(
            cindral_io::load_draft(&editor.files.snapshot).unwrap(),
            expected
        );
    }

    #[test]
    fn close_anyway_bypasses_exit_backup_and_keep_editing_cancels_close() {
        let directory = tempfile::tempdir().unwrap();
        let mut editor = app(directory.path());
        let blocked = directory.path().join("blocked");
        std::fs::write(&blocked, b"blocked").unwrap();
        editor.files.directory = blocked;
        assert!(editor.preserve_on_close().is_err());
        editor.files.close_error = Some("Recovery unavailable".into());
        let ctx = egui::Context::default();
        editor.close_action(CloseAction::Cancel, &ctx);
        assert!(editor.files.close_error.is_none());
        assert!(!editor.files.close_approved);
        let expected = editor.file_values();
        editor.files.close_error = Some("Recovery unavailable".into());
        let output = ctx.run(egui::RawInput::default(), |ctx| {
            editor.close_action(CloseAction::CloseAnyway, ctx);
        });
        assert!(output.viewport_output[&egui::ViewportId::ROOT]
            .commands
            .contains(&egui::ViewportCommand::Close));
        assert!(editor.files.close_approved);
        assert!(editor.files.close_error.is_none());
        // The exit fallback must not retry a write that the user chose to bypass.
        editor.preserve_on_close().unwrap();
        eframe::App::on_exit(&mut editor);
        assert_eq!(editor.file_values(), expected);
        assert!(cindral_io::recovery_files(directory.path())
            .unwrap()
            .is_empty());
    }

    #[test]
    fn close_save_as_handles_cancel_failure_and_success() {
        let directory = tempfile::tempdir().unwrap();
        let mut editor = app(directory.path());
        editor.files.close_error = Some("Recovery unavailable".into());
        let ctx = egui::Context::default();
        editor.save_before_close(None, &ctx);
        assert!(!editor.files.close_approved);
        assert_eq!(
            editor.files.close_error.as_deref(),
            Some("Recovery unavailable")
        );
        let blocked = directory.path().join("blocked");
        std::fs::write(&blocked, b"blocked").unwrap();
        editor.save_before_close(Some(blocked.join("failed.cdraft")), &ctx);
        assert!(!editor.files.close_approved);
        assert!(editor
            .files
            .close_error
            .as_deref()
            .unwrap()
            .contains("Could not save"));
        let expected = editor.file_values();
        let path = directory.path().join("elsewhere.cdraft");
        let output = ctx.run(egui::RawInput::default(), |ctx| {
            editor.save_before_close(Some(path.clone()), ctx);
        });
        assert!(output.viewport_output[&egui::ViewportId::ROOT]
            .commands
            .contains(&egui::ViewportCommand::Close));
        assert!(editor.files.close_approved);
        assert!(editor.files.close_error.is_none());
        assert_eq!(cindral_io::load_draft(&path).unwrap(), expected);
    }

    #[test]
    fn unchanged_saved_values_skip_periodic_and_outgoing_backups() {
        let directory = tempfile::tempdir().unwrap();
        let mut editor = app(directory.path());
        let path = directory.path().join("saved.cdraft");
        editor.save_path(&path).unwrap();
        editor
            .history
            .execute(Command::AddEdge {
                start: DVec3::ZERO,
                end: DVec3::Z,
            })
            .unwrap();
        editor.history.undo().unwrap();
        editor.autosave_tick(&egui::Context::default());
        assert!(editor.files.job.is_none());
        editor.open_path_confirmed(&path).unwrap();
        editor.do_file_action(Action::New).unwrap();
        assert!(cindral_io::recovery_files(directory.path())
            .unwrap()
            .is_empty());
    }

    fn dated_snapshot(directory: &Path, index: u64, modified: SystemTime) -> PathBuf {
        let path = directory.join(format!("draft-1-{index}-0.cdraft.autosave"));
        let file = std::fs::File::create(&path).unwrap();
        file.set_times(std::fs::FileTimes::new().set_modified(modified))
            .unwrap();
        path
    }

    #[test]
    fn retention_keeps_newest_twenty_and_protected_older_snapshots() {
        let directory = tempfile::tempdir().unwrap();
        let now = SystemTime::now();
        let paths: Vec<_> = (0..25)
            .map(|index| {
                dated_snapshot(
                    directory.path(),
                    index,
                    now - RECOVERY_GRACE - Duration::from_secs(100 - index),
                )
            })
            .collect();
        let mut editor = app(directory.path());
        editor.files.snapshot = paths[0].clone();
        editor.files.recovered_from = Some(paths[1].clone());
        let (_sender, receiver) = mpsc::channel();
        editor.files.job = Some(AutosaveJob {
            path: paths[2].clone(),
            generation: 0,
            revision: (0, 0),
            superseded: false,
            receiver,
        });
        editor.files.candidates = paths.clone();
        editor.retire_recovery();
        for (index, path) in paths.iter().enumerate() {
            assert_eq!(path.exists(), !(3..5).contains(&index));
        }
        assert!(!editor.files.candidates.contains(&paths[3]));
        assert!(!editor.files.candidates.contains(&paths[4]));
        assert_eq!(editor.files.candidates.len(), 23);
        assert_eq!(
            cindral_io::recovery_files(directory.path()).unwrap().len(),
            23
        );
    }

    #[test]
    fn retention_preserves_recent_future_and_unrelated_files() {
        let directory = tempfile::tempdir().unwrap();
        let now = SystemTime::now();
        for index in 0..30 {
            dated_snapshot(directory.path(), index, now - Duration::from_secs(60));
        }
        let future = dated_snapshot(directory.path(), 30, now + Duration::from_secs(60));
        let unrelated = directory.path().join("saved.cdraft");
        std::fs::write(&unrelated, b"saved draft").unwrap();
        let named_recovery = directory.path().join("important.cdraft.autosave");
        std::fs::File::create(&named_recovery)
            .unwrap()
            .set_times(std::fs::FileTimes::new().set_modified(UNIX_EPOCH))
            .unwrap();
        assert!(prune_recovery(directory.path(), &[], now)
            .unwrap()
            .is_empty());
        assert!(future.exists());
        assert!(named_recovery.exists());
        assert_eq!(std::fs::read(unrelated).unwrap(), b"saved draft");
        assert_eq!(
            cindral_io::recovery_files(directory.path()).unwrap().len(),
            32
        );
    }

    fn test_import_result() -> PreparedImport {
        let mut data = cindral_core::DocumentHistory::new().to_data();
        let geometry = cindral_core::BulkGeometry {
            vertices: vec![DVec3::ZERO, DVec3::X, DVec3::X + DVec3::Y, DVec3::Y],
            faces: vec![cindral_core::BulkFace {
                outer: vec![0, 1, 2],
                ..Default::default()
            }],
            edges: vec![],
        }
        .build(false, false)
        .unwrap();
        data.definitions
            .get_mut(&cindral_core::DefinitionId(0))
            .unwrap()
            .geometry = geometry.to_data();
        let history = cindral_core::DocumentHistory::from_import_data(data).unwrap();
        prepare_import(
            ImportResult {
                history,
                textures: BTreeMap::new(),
                report: ImportReport {
                    vertices: 4,
                    edges: 3,
                    faces: 1,
                    instances: 0,
                    skipped: 0,
                    warnings: vec!["test warning".into()],
                    ..Default::default()
                },
            },
            &ImportControl::default(),
        )
        .unwrap()
    }

    fn queue_import(
        editor: &mut DrafterApp,
        path: &str,
        result: cindral_io::Result<PreparedImport>,
    ) {
        let (sender, receiver) = mpsc::channel();
        sender.send(result).unwrap();
        editor.files.import_job = Some(ImportJob {
            save_epoch: editor.files.import_save_epoch,
            replace_document: false,
            generation: editor.files.generation,
            revision: editor.history.revision(),
            path: path.into(),
            control: ImportControl::default(),
            cancelled: false,
            receiver,
        });
    }
    fn resource_import() -> PreparedImport {
        use cindral_io::{ImportedDefinition, ImportedInstance, ImportedScene};
        let mut encoded = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut encoded, 1, 1);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            encoder
                .write_header()
                .unwrap()
                .write_image_data(&[255, 0, 0, 255])
                .unwrap();
        }
        let (key, asset) = cindral_io::texture_from_bytes(encoded).unwrap();
        let scene = ImportedScene {
            definitions: vec![ImportedDefinition {
                name: "Part".into(),
                geometry: cindral_core::BulkGeometry {
                    vertices: vec![DVec3::ZERO, DVec3::X, DVec3::Y],
                    faces: vec![cindral_core::BulkFace {
                        outer: vec![0, 1, 2],
                        material: Some(cindral_core::MaterialId(1)),
                        ..Default::default()
                    }],
                    ..Default::default()
                },
                ..Default::default()
            }],
            roots: vec![ImportedInstance {
                tag: Some(0),
                ..Default::default()
            }],
            tags: vec![cindral_core::Tag {
                name: "Layer".into(),
                visible: true,
            }],
            materials: vec![cindral_core::Material {
                name: "Paint".into(),
                color: [1., 0., 0.],
                opacity: 1.,
                texture: Some(key.clone()),
            }],
            textures: BTreeMap::from([(key, asset)]),
            ..Default::default()
        };
        prepare_import(
            cindral_io::convert_scene(scene, &ImportOptions::default(), &ImportControl::default())
                .unwrap(),
            &ImportControl::default(),
        )
        .unwrap()
    }
    #[test]
    fn import_waits_for_tool_then_keeps_intervening_edits_identity_and_one_undo() {
        let directory = tempfile::tempdir().unwrap();
        let mut editor = app(directory.path());
        let path = directory.path().join("working.cdraft");
        editor.save_path(&path).unwrap();
        queue_import(&mut editor, "source.obj", Ok(resource_import()));
        editor
            .execute(Command::AddEdge {
                start: DVec3::splat(10.),
                end: DVec3::splat(10.) + DVec3::X,
            })
            .unwrap();
        editor.tool.click(DVec3::ZERO).unwrap();
        let before = editor.file_values();
        let depth = editor.history.undo_depth();
        editor.import_tick(&egui::Context::default());
        assert!(editor.files.import_job.is_some());
        assert_eq!(editor.files.status, "Import ready, applying...");
        assert_eq!(editor.file_values(), before);
        editor.tool.cancel();
        editor.import_tick(&egui::Context::default());
        assert_eq!(editor.files.path.as_ref(), Some(&path));
        assert_eq!(editor.history.undo_depth(), depth + 1);
        assert!(editor.organization.selected.is_some());
        assert_eq!(
            editor
                .history
                .instance(editor.organization.selected.as_ref().unwrap())
                .unwrap()
                .transform,
            glam::DMat4::IDENTITY
        );
        let after = editor.file_values();
        assert!(after.document.definitions.len() > before.document.definitions.len());
        assert_eq!(after.document.tags.len(), before.document.tags.len() + 1);
        assert_eq!(
            after.document.materials.len(),
            before.document.materials.len() + 1
        );
        assert_eq!(after.textures.len(), before.textures.len() + 1);
        editor.history_step(false);
        let undone = editor.file_values();
        assert_eq!(undone.document.definitions, before.document.definitions);
        assert_eq!(undone.document.tags, before.document.tags);
        assert_eq!(undone.document.materials, before.document.materials);
        assert_eq!(undone.textures, before.textures);
        editor.history_step(true);
        assert_eq!(editor.file_values(), after);
        assert!(editor.files.status.contains("into current model"));
    }
    #[test]
    fn import_waits_for_geometry_preview_cancellation() {
        let directory = tempfile::tempdir().unwrap();
        let mut editor = app(directory.path());
        let before = editor.file_values();
        let depth = editor.history.undo_depth();
        queue_import(&mut editor, "source.obj", Ok(resource_import()));
        editor
            .preview(Command::AddEdge {
                start: DVec3::splat(10.),
                end: DVec3::splat(10.) + DVec3::X,
            })
            .unwrap();
        editor.import_tick(&egui::Context::default());
        assert!(editor.files.ready_import.is_some());
        assert_eq!(editor.history.undo_depth(), depth);
        editor.cancel_preview().unwrap();
        assert_eq!(editor.file_values(), before);
        editor.import_tick(&egui::Context::default());
        assert!(editor.files.import_job.is_none());
        assert_eq!(editor.history.undo_depth(), depth + 1);
    }
    #[test]
    fn app_import_merges_texture_content_materials_and_tags_without_changing_existing() {
        let directory = tempfile::tempdir().unwrap();
        let mut editor = app(directory.path());
        editor
            .append_import_result(resource_import(), Path::new("first.obj"))
            .unwrap();
        let before = editor.file_values();
        let mut second = resource_import();
        let asset = second.result.textures.pop_first().unwrap().1;
        second
            .result
            .textures
            .insert("different-source-slot".into(), asset);
        let mut data = second.result.history.to_data();
        for material in data.materials.values_mut() {
            material.texture = Some("different-source-slot".into());
        }
        second.result.history = History::from_import_data(data).unwrap();
        editor
            .append_import_result(second, Path::new("second.obj"))
            .unwrap();
        assert_eq!(editor.history.materials(), &before.document.materials);
        assert_eq!(editor.history.tags(), &before.document.tags);
        assert_eq!(editor.files.textures, before.textures);
        assert!(editor
            .files
            .import_summary
            .as_ref()
            .unwrap()
            .notes
            .iter()
            .any(|n| n.contains("merged 1 tags / 1 materials")));
        editor.history_step(false);
        assert_eq!(
            editor.file_values().document.definitions,
            before.document.definitions
        );
        assert_eq!(editor.files.textures, before.textures);
    }
    #[test]
    fn import_failure_cancellation_and_changed_target_leave_values_and_history_unchanged() {
        let directory = tempfile::tempdir().unwrap();
        let mut editor = app(directory.path());
        for mode in 0..6 {
            queue_import(
                &mut editor,
                "source.obj",
                if mode == 0 {
                    Err(cindral_io::IoError::Invalid("bad source".into()))
                } else {
                    Ok(resource_import())
                },
            );
            if mode == 1 {
                editor.cancel_import();
            }
            if mode == 2 {
                editor
                    .save_path(&directory.path().join("different.cdraft"))
                    .unwrap();
            }
            if mode == 3 {
                editor.do_file_action(Action::New).unwrap();
            }
            if mode == 4 {
                editor.apply_file(DraftBundle::default(), None).unwrap();
            }
            if mode == 5 {
                editor.files.import_job.as_mut().unwrap().replace_document = true;
                editor
                    .execute(Command::AddEdge {
                        start: DVec3::ZERO,
                        end: DVec3::X,
                    })
                    .unwrap();
            }
            let before = editor.file_values();
            let depth = editor.history.undo_depth();
            let redo = editor.history.redo_depth();
            editor.import_tick(&egui::Context::default());
            assert_eq!(editor.file_values(), before);
            assert_eq!(editor.history.undo_depth(), depth);
            assert_eq!(editor.history.redo_depth(), redo);
            assert!(editor.files.import_job.is_none());
        }
    }
    #[test]
    fn open_unsaved_requires_save_discard_or_cancel() {
        let directory = tempfile::tempdir().unwrap();
        let mut editor = app(directory.path());
        let target = directory.path().join("target.cdraft");
        cindral_io::save_draft(&target, &DraftBundle::default()).unwrap();
        let before = editor.file_values();
        editor.open_path(&target).unwrap();
        assert!(editor.files.pending_open.is_some());
        assert_eq!(editor.file_values(), before);
        editor.resolve_open_choice(2);
        assert!(editor.files.pending_open.is_none());
        assert_eq!(editor.file_values(), before);
        editor.open_path(&target).unwrap();
        editor.resolve_open_choice(1);
        assert_eq!(editor.files.path.as_ref(), Some(&target));
        assert_eq!(editor.history.undo_depth(), 0);
        let current = directory.path().join("current.cdraft");
        editor.save_path(&current).unwrap();
        editor
            .execute(Command::AddEdge {
                start: DVec3::ZERO,
                end: DVec3::X,
            })
            .unwrap();
        let expected = editor.file_values();
        editor.open_path(&target).unwrap();
        editor.resolve_open_choice(0);
        assert_eq!(cindral_io::load_draft(&current).unwrap(), expected);
        assert_eq!(editor.files.path.as_ref(), Some(&target));
    }
    #[test]
    fn open_routes_every_foreign_extension_to_replacement_and_native_to_fresh_history() {
        let directory = tempfile::tempdir().unwrap();
        let mut editor = app(directory.path());
        for extension in ["skp", "dae", "glb", "gltf", "obj"] {
            let path = directory.path().join(format!("source.{extension}"));
            editor.open_path_confirmed(&path).unwrap();
            assert!(editor.files.import_job.as_ref().unwrap().replace_document);
            editor.cancel_import();
        }
        for extension in ["cdraft", "skp", "dae", "glb", "gltf", "obj"] {
            let identity = editor.files.path.clone();
            queue_import(
                &mut editor,
                &format!("source.{extension}"),
                Ok(resource_import()),
            );
            editor.import_tick(&egui::Context::default());
            assert_eq!(editor.files.path, identity);
        }
    }

    #[test]
    fn import_cancellation_discards_completed_result() {
        let directory = tempfile::tempdir().unwrap();
        let mut editor = app(directory.path());
        let (sender, receiver) = mpsc::channel();
        editor.files.import_job = Some(ImportJob {
            save_epoch: editor.files.import_save_epoch,
            replace_document: false,
            generation: editor.files.generation,
            revision: editor.history.revision(),
            path: PathBuf::from("model.gltf"),
            control: ImportControl::default(),
            cancelled: false,
            receiver,
        });

        editor.cancel_import();
        assert!(editor.files.import_job.is_none());
        assert_eq!(editor.files.status, "Import cancelled");

        let result = test_import_result();
        assert!(sender.send(Ok(result)).is_err());

        editor.import_tick(&egui::Context::default());
        assert!(editor.files.import_summary.is_none());
        assert_eq!(editor.history.undo_depth(), 1);
    }

    #[test]
    fn import_accepts_result_and_preserves_single_undo_step() {
        let directory = tempfile::tempdir().unwrap();
        let mut editor = app(directory.path());
        let (sender, receiver) = mpsc::channel();
        let path = PathBuf::from("model.dae");
        editor.files.import_job = Some(ImportJob {
            save_epoch: editor.files.import_save_epoch,
            replace_document: false,
            generation: editor.files.generation,
            revision: editor.history.revision(),
            path: path.clone(),
            control: ImportControl::default(),
            cancelled: false,
            receiver,
        });

        let result = test_import_result();
        sender.send(Ok(result)).unwrap();

        editor.import_tick(&egui::Context::default());
        assert!(editor.files.import_job.is_none());
        let summary = editor.files.import_summary.as_ref().unwrap();
        assert_eq!(summary.vertices, 4);
        assert_eq!(summary.faces, 1);
        assert_eq!(summary.warnings.len(), 1);
        assert_eq!(editor.history.undo_depth(), 2);

        editor.history.undo().unwrap();
        assert_eq!(editor.history.undo_depth(), 1);
    }

    #[test]
    fn skp_open_result_replaces_previous_geometry_and_native_save_target() {
        let directory = tempfile::tempdir().unwrap();
        let mut editor = app(directory.path());
        editor
            .execute(Command::AddPolyline {
                points: vec![
                    DVec3::Z * 10.,
                    DVec3::Z * 10. + DVec3::X,
                    DVec3::Z * 10. + DVec3::X + DVec3::Y,
                    DVec3::Z * 10. + DVec3::Y,
                ],
                closed: true,
            })
            .unwrap();
        editor.files.path = Some(directory.path().join("previous.cdraft"));
        let old = editor.history.model().to_data();
        let (sender, receiver) = mpsc::channel();
        sender.send(Ok(test_import_result())).unwrap();
        editor.files.import_job = Some(ImportJob {
            save_epoch: editor.files.import_save_epoch,
            replace_document: true,
            generation: editor.files.generation,
            revision: editor.history.revision(),
            path: directory.path().join("source.skp"),
            control: ImportControl::default(),
            cancelled: false,
            receiver,
        });
        editor.import_tick(&egui::Context::default());
        assert!(editor.files.path.is_none());
        assert!(editor.history.context().is_empty());
        assert_eq!(editor.model().faces().len(), 1);
        assert_eq!(editor.model().bounds().unwrap().1.z, 0.);
        let backups = cindral_io::recovery_files(directory.path()).unwrap();
        assert_eq!(backups.len(), 1);
        let backup_path = backups[0].clone();
        assert_ne!(editor.files.snapshot, backup_path);
        let backup = cindral_io::load_draft(&backup_path).unwrap();
        assert_eq!(
            backup.document.definitions[&cindral_core::DefinitionId(0)].geometry,
            old
        );
        let face = *editor.model().faces().keys().next().unwrap();
        assert!(editor
            .execute(Command::PushPull {
                face,
                distance: 0.2
            })
            .unwrap());
        let imported = editor.file_values();
        let ctx = egui::Context::default();
        editor.autosave_tick(&ctx);
        let deadline = Instant::now() + Duration::from_secs(10);
        while editor.files.job.is_some() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
            editor.autosave_tick(&ctx);
        }
        assert!(editor.files.job.is_none(), "autosave worker timed out");
        assert_eq!(
            cindral_io::load_draft(&editor.files.snapshot).unwrap(),
            imported
        );
        assert_eq!(cindral_io::load_draft(&backup_path).unwrap(), backup);
    }

    #[test]
    fn skp_open_uses_import_worker_and_never_saves_over_source() {
        let directory = tempfile::tempdir().unwrap();
        let mut editor = app(directory.path());
        let path = directory.path().join("source.skp");
        std::fs::write(&path, b"source stays unchanged").unwrap();
        editor.open_path_confirmed(&path).unwrap();
        assert!(editor.files.import_job.is_some());
        editor.cancel_import();
        assert!(editor.save_path(&path).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"source stays unchanged");
        editor
            .apply_import_result(test_import_result(), &path)
            .unwrap();
        assert!(editor.files.status.contains("imported from .skp"));
        assert!(editor.files.status.contains("Save As .cdraft"));
        assert!(editor.files.path.is_none());
    }

    #[test]
    fn stale_import_job_is_guarded_after_new_and_open() {
        let directory = tempfile::tempdir().unwrap();
        let mut editor = app(directory.path());
        let (sender, receiver) = mpsc::channel();
        let old_gen = editor.files.generation;
        let job = ImportJob {
            save_epoch: editor.files.import_save_epoch,
            replace_document: false,
            generation: old_gen,
            revision: editor.history.revision(),
            path: PathBuf::from("stale.obj"),
            control: ImportControl::default(),
            cancelled: false,
            receiver,
        };
        editor.files.import_job = Some(job);

        editor.do_file_action(Action::New).unwrap();
        assert!(editor.files.import_job.is_none());
        assert_ne!(editor.files.generation, old_gen);

        let (_s2, r2) = mpsc::channel();
        let stale_job = ImportJob {
            save_epoch: editor.files.import_save_epoch,
            replace_document: false,
            generation: old_gen,
            revision: editor.history.revision(),
            path: PathBuf::from("stale.obj"),
            control: ImportControl::default(),
            cancelled: false,
            receiver: r2,
        };
        assert!(sender.send(Ok(test_import_result())).is_err());
        editor.files.import_job = Some(stale_job);
        editor.import_tick(&egui::Context::default());
        assert!(editor.files.import_summary.is_none());
    }

    #[test]
    fn import_options_defaults_and_toggle() {
        let directory = tempfile::tempdir().unwrap();
        let mut editor = app(directory.path());
        assert!(editor.files.import_options.as_component);
        assert!(editor.files.import_options.merge_coplanar);
        assert!(editor.files.import_options.soften_coplanar);

        editor.files.import_options.as_component = false;
        editor.files.import_options.merge_coplanar = false;
        editor.files.import_options.soften_coplanar = false;

        assert!(!editor.files.import_options.as_component);
        assert!(!editor.files.import_options.merge_coplanar);
        assert!(!editor.files.import_options.soften_coplanar);
    }
    #[test]
    #[ignore = "run in release mode for 100k-triangle kernel and CPU mesh latency"]
    #[allow(clippy::assertions_on_constants)]
    fn imported_100k_interactive_mesh_update() {
        assert!(!cfg!(debug_assertions), "benchmark requires --release");
        let mut geometry = cindral_core::BulkGeometry::default();
        for y in 0..200 {
            for x in 0..250 {
                let base = geometry.vertices.len();
                let p = DVec3::new(x as f64, y as f64, 0.);
                geometry
                    .vertices
                    .extend([p, p + DVec3::X, p + DVec3::X + DVec3::Y, p + DVec3::Y]);
                geometry.faces.push(cindral_core::BulkFace {
                    outer: vec![base, base + 1, base + 2],
                    ..Default::default()
                });
                geometry.faces.push(cindral_core::BulkFace {
                    outer: vec![base, base + 2, base + 3],
                    ..Default::default()
                });
            }
        }
        let scene = cindral_io::ImportedScene {
            definitions: vec![cindral_io::ImportedDefinition {
                name: "Sheet".into(),
                geometry,
                ..Default::default()
            }],
            roots: vec![cindral_io::ImportedInstance::default()],
            ..Default::default()
        };
        let control = ImportControl::default();
        let result = cindral_io::convert_scene(scene, &ImportOptions::default(), &control).unwrap();
        let prepared = prepare_import(result, &control).unwrap();
        let mut editor = DrafterApp::default();
        editor
            .apply_import_result(prepared, Path::new("sheet.gltf"))
            .unwrap();
        let path = editor
            .history
            .occurrences()
            .unwrap()
            .into_iter()
            .find(|o| {
                !editor
                    .history
                    .definition_model(o.definition)
                    .unwrap()
                    .faces()
                    .is_empty()
            })
            .unwrap()
            .path;
        editor.history.enter(path).unwrap();
        editor.refresh_mesh();
        editor.pending = None;
        let start = Instant::now();
        editor
            .execute(Command::AddEdge {
                start: DVec3::new(0., 0.5, 0.),
                end: DVec3::new(0.5, 0., 0.),
            })
            .unwrap();
        let elapsed = start.elapsed();
        eprintln!(
            "Imported 100k triangles: connected edit plus dirty CPU mesh {:.3}ms",
            elapsed.as_secs_f64() * 1000.
        );
        assert!(
            elapsed < Duration::from_millis(16),
            "interactive budget exceeded: {elapsed:?}"
        );
        assert_eq!(editor.history.undo_depth(), 2);
    }
    #[test]
    #[ignore = "release-only SKP CPU edit/dirty mesh workload; CINDRAL_SKP_BENCHMARK_SEED supplies authored seed"]
    #[allow(clippy::assertions_on_constants)]
    fn imported_skp_interactive_mesh_update() {
        assert!(!cfg!(debug_assertions), "benchmark requires --release");
        let Some(path) = std::env::var_os("CINDRAL_SKP_BENCHMARK_SEED") else {
            eprintln!(
                "SKIP: set CINDRAL_SKP_BENCHMARK_SEED to authored connected benchmark fixture"
            );
            return;
        };
        let path = std::path::PathBuf::from(path);
        let control = ImportControl::default();
        let result = cindral_io::import_file(&path, &ImportOptions::default(), &control).unwrap();
        let prepared = prepare_import(result, &control).unwrap();
        let mut editor = DrafterApp::default();
        editor.apply_import_result(prepared, &path).unwrap();
        let occurrence = editor
            .history
            .occurrences()
            .unwrap()
            .into_iter()
            .find(|o| {
                !editor
                    .history
                    .definition_model(o.definition)
                    .unwrap()
                    .faces()
                    .is_empty()
            })
            .unwrap();
        editor.history.enter(occurrence.path).unwrap();
        editor.refresh_mesh();
        editor.pending = None;
        let start = Instant::now();
        editor
            .execute(Command::AddEdge {
                start: DVec3::new(0., 0.127, 0.),
                end: DVec3::new(0.127, 0., 0.),
            })
            .unwrap();
        let elapsed = start.elapsed();
        eprintln!(
            "SKP connected edit plus dirty CPU mesh {:.3}ms",
            elapsed.as_secs_f64() * 1000.
        );
        assert!(
            elapsed < Duration::from_millis(16),
            "interactive budget exceeded: {elapsed:?}"
        );
        assert_eq!(editor.history.undo_depth(), 2);
    }

    #[test]
    #[ignore = "release-only SKP CPU edit/dirty mesh workload; CINDRAL_SKP_BENCHMARK_SEED supplies authored seed"]
    #[allow(clippy::assertions_on_constants)]
    fn imported_skp_pushpull_interactive_mesh_update() {
        assert!(!cfg!(debug_assertions), "benchmark requires --release");
        let Some(path) = std::env::var_os("CINDRAL_SKP_BENCHMARK_SEED") else {
            eprintln!(
                "SKIP: set CINDRAL_SKP_BENCHMARK_SEED to authored connected benchmark fixture"
            );
            return;
        };
        let path = std::path::PathBuf::from(path);
        let control = ImportControl::default();
        let result = cindral_io::import_file(
            &path,
            &ImportOptions {
                as_component: false,
                ..Default::default()
            },
            &control,
        )
        .unwrap();
        let prepared = prepare_import(result, &control).unwrap();
        let mut editor = DrafterApp::default();
        editor.apply_import_result(prepared, &path).unwrap();
        let occurrence = editor
            .history
            .occurrences()
            .unwrap()
            .into_iter()
            .find(|o| {
                !editor
                    .history
                    .definition_model(o.definition)
                    .unwrap()
                    .faces()
                    .is_empty()
            })
            .unwrap();
        editor.history.enter(occurrence.path).unwrap();
        editor.refresh_mesh();
        editor.pending = None;
        assert!(editor.history.context().is_empty());
        let face = editor
            .model()
            .faces()
            .iter()
            .filter(|(_, f)| f.plane.normal.z > 0.9)
            .max_by(|(_, a), (_, b)| a.plane.distance.total_cmp(&b.plane.distance))
            .map(|(id, _)| *id)
            .unwrap();
        let count = editor.model().faces().len();
        eprintln!(
            "SKP import and CPU scene preparation complete; starting Push/Pull on {count} faces"
        );
        let start = Instant::now();
        assert!(editor
            .execute(Command::PushPull {
                face,
                distance: 0.0254
            })
            .unwrap());
        let elapsed = start.elapsed();
        eprintln!(
            "SKP {count} faces Push/Pull plus dirty CPU mesh {:.3}ms",
            elapsed.as_secs_f64() * 1000.
        );
        assert!(
            elapsed < Duration::from_millis(16),
            "interactive budget exceeded: {elapsed:?}"
        );
        assert_eq!(editor.history.undo_depth(), 2);
        editor.history.undo().unwrap();
        editor.refresh_mesh();
        editor.pending = None;
        // Repeated absolute previews use the original source, just as dragging does.
        let face = *editor.model().faces().keys().nth(count / 2).unwrap();
        let mut worst = Duration::ZERO;
        for i in 1..=20 {
            let start = Instant::now();
            assert!(editor
                .preview(Command::PushPull {
                    face,
                    distance: 0.001 * i as f64 * if i % 2 == 0 { 1. } else { -1. },
                })
                .unwrap());
            worst = worst.max(start.elapsed());
            assert_eq!(editor.history.undo_depth(), 1);
        }
        eprintln!(
            "SKP {count} faces: 20 drag previews plus dirty CPU mesh, worst {:.3}ms",
            worst.as_secs_f64() * 1000.
        );
        assert!(
            worst < Duration::from_millis(16),
            "drag preview budget exceeded: {worst:?}"
        );
        assert!(editor.commit_preview().unwrap());
        assert_eq!(editor.history.undo_depth(), 2);
        assert!(editor.history.undo().unwrap());
        assert!(editor.history.redo().unwrap());
    }
}
