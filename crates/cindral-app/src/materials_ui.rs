//! In-model Materials panel: virtualized palette, live property editor,
//! builtin collection, procedural patterns and the user folder library.
//!
//! Also hosts the SketchUp-style Paint Bucket click routing, hover preview
//! and status hints; see [`DrafterApp::paint_bucket_click`].
use crate::DrafterApp;
use cindral_core::{FaceId, InstancePath, Material, MaterialId, OrganizationCommand as Org};
use cindral_tools::{
    faces_with_effective_material, faces_with_material, fill_adjacent_same_material, InstanceHit,
};
use eframe::egui::{self, Color32, RichText};
use std::collections::{BTreeMap, BTreeSet};

/// How the palette list is ordered.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum MaterialSort {
    /// Alphabetical by name.
    #[default]
    Name,
    /// Most-used first, name as tiebreaker.
    Usage,
}

/// A dedicated inspector view keeps browsing separate from shared editing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum MaterialsView {
    #[default]
    Browse,
    Edit,
    Library,
}

/// One paint plan drives both execution and the visible preview.
#[derive(Clone, Debug, Default, PartialEq)]
struct PaintPlan {
    faces: Vec<FaceId>,
    face_path: InstancePath,
    instances: Vec<InstancePath>,
}

#[derive(Clone, Debug, PartialEq)]
enum InspectedMaterial {
    Empty,
    Single(Option<MaterialId>, bool),
    Mixed,
}

/// Hover preview for the Paint Bucket: what a click would paint.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct PaintHover {
    /// Face ids in the active definition that would be painted.
    pub faces: Vec<FaceId>,
    /// Container that would be painted as a whole.
    pub instance: Option<InstancePath>,
    /// Hover target plus modifier state; equality gates recomputation.
    plan: PaintPlan,
    key: (Option<FaceId>, Option<InstancePath>, u8, u64),
    selected_faces: BTreeSet<FaceId>,
    selected_instances: BTreeSet<InstancePath>,
    selected_instance: Option<InstancePath>,
    context: InstancePath,
}

impl Default for PaintHover {
    fn default() -> Self {
        Self {
            faces: Vec::new(),
            instance: None,
            plan: PaintPlan::default(),
            key: (None, None, u8::MAX, u64::MAX),
            selected_faces: BTreeSet::new(),
            selected_instances: BTreeSet::new(),
            selected_instance: None,
            context: Vec::new(),
        }
    }
}

/// Live-editing state for the Materials panel.
pub(crate) struct MaterialsUi {
    pub(crate) view: MaterialsView,
    pub(crate) picker_open: bool,
    picker_search: String,
    picker_recent: bool,
    picker_focus: usize,
    picker_focus_search: bool,
    recent_materials: Vec<Option<MaterialId>>,
    pub(crate) material_tiles: BTreeMap<String, egui::TextureHandle>,
    mapping_invalid: bool,
    hex_material: Option<MaterialId>,
    hex_dirty: bool,
    delete: Option<MaterialId>,
    replace_target: Option<MaterialId>,
    notice: Option<(String, u64)>,
    pub(crate) search: String,
    pub(crate) sort: MaterialSort,
    counts: BTreeMap<MaterialId, usize>,
    counts_revision: Option<u64>,
    default_count: usize,
    /// Original values captured while a color/opacity drag is live.
    drag_original: Option<(MaterialId, Material)>,
    /// Original values captured while the name field is being edited.
    name_original: Option<(MaterialId, Material)>,
    name_buffer: String,
    /// Live texture-mapping edit session: pre-edit material plus whether the
    /// size text fields own it (text commits on Enter/focus loss; slider
    /// edits commit when a frame passes without a widget change).
    mapping_session: Option<(MaterialId, Material, bool)>,
    /// Width/height text buffers for the size editor (persists between
    /// keystrokes so partial input is not clobbered by the formatted value).
    size_buffers: Option<(MaterialId, String, String)>,
    editing_widgets: bool,
    hex: String,
    rename: Option<(MaterialId, String)>,
    replace: Option<MaterialId>,
    /// Rows built for the visible list window this frame (virtualization check).
    pub(crate) rows_built: usize,
    library_tiles: BTreeMap<String, egui::TextureHandle>,
    pub(crate) eyedropper: bool,
    /// Lock-aspect toggle for the texture size editor (on by default).
    pub(crate) lock_aspect: bool,
    /// Decoded pixel sizes per texture slot (decoded once, cached).
    texture_sizes: BTreeMap<String, (u32, u32)>,
    pub(crate) library: crate::materials_library::LibraryCache,
    library_restored: bool,
}

impl Default for MaterialsUi {
    fn default() -> Self {
        Self {
            view: MaterialsView::Browse,
            picker_open: false,
            picker_search: String::new(),
            picker_recent: false,
            picker_focus: 0,
            picker_focus_search: false,
            recent_materials: Vec::new(),
            material_tiles: BTreeMap::new(),
            mapping_invalid: false,
            hex_material: None,
            hex_dirty: false,
            delete: None,
            replace_target: None,
            notice: None,
            search: String::new(),
            sort: MaterialSort::default(),
            counts: BTreeMap::new(),
            counts_revision: None,
            default_count: 0,
            drag_original: None,
            name_original: None,
            name_buffer: String::new(),
            mapping_session: None,
            size_buffers: None,
            editing_widgets: false,
            hex: String::new(),
            rename: None,
            replace: None,
            rows_built: 0,
            library_tiles: BTreeMap::new(),
            eyedropper: false,
            lock_aspect: true,
            texture_sizes: BTreeMap::new(),
            library: crate::materials_library::LibraryCache::new(Vec::new()),
            library_restored: false,
        }
    }
}

const ROW_HEIGHT: f32 = 32.;

impl MaterialsUi {
    fn refresh_counts(&mut self, history: &cindral_core::DocumentHistory) {
        let revision = history.revision();
        if self.counts_revision == Some(revision) {
            return;
        }
        let mut counts: BTreeMap<MaterialId, usize> = BTreeMap::new();
        let mut default_count = 0usize;
        for definition in history.definitions() {
            if let Some(model) = history.definition_model(definition) {
                for face in model.faces().values() {
                    match face.material {
                        Some(id) => *counts.entry(id).or_default() += 1,
                        None => default_count += 1,
                    }
                }
            }
            if let Some(instances) = history.instances(definition) {
                for node in instances.values() {
                    match node.material {
                        Some(id) => *counts.entry(id).or_default() += 1,
                        None => default_count += 1,
                    }
                }
            }
        }
        self.counts = counts;
        self.default_count = default_count;
        self.counts_revision = Some(revision);
    }

    fn usage(&self, id: Option<MaterialId>) -> usize {
        id.map_or(self.default_count, |id| {
            self.counts.get(&id).copied().unwrap_or(0)
        })
    }

    fn visible_entries(&self, history: &cindral_core::DocumentHistory) -> Vec<(MaterialId, usize)> {
        let needle = self.search.trim().to_lowercase();
        let name_of = |id: MaterialId| {
            history
                .materials()
                .get(&id)
                .map_or(String::new(), |m| m.name.clone())
        };
        let mut entries: Vec<(MaterialId, usize)> = history
            .materials()
            .iter()
            .filter(|(_, m)| needle.is_empty() || m.name.to_lowercase().contains(&needle))
            .map(|(id, _)| (*id, self.counts.get(id).copied().unwrap_or(0)))
            .collect();
        match self.sort {
            MaterialSort::Name => entries.sort_by_key(|(id, _)| name_of(*id)),
            MaterialSort::Usage => {
                entries.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| name_of(a.0).cmp(&name_of(b.0))))
            }
        }
        entries
    }

    fn unused_materials(&self, history: &cindral_core::DocumentHistory) -> Vec<MaterialId> {
        history
            .materials()
            .keys()
            .filter(|id| !self.counts.contains_key(id))
            .copied()
            .collect()
    }
}

impl DrafterApp {
    /// The Materials section of the organization sidebar.
    pub(crate) fn materials_panel(&mut self, ui: &mut egui::Ui) {
        if !self.materials.library_restored {
            let folders = self
                .png
                .options
                .library_folders
                .iter()
                .map(std::path::PathBuf::from)
                .collect::<Vec<_>>();
            for folder in &folders {
                self.materials.library.add_folder(folder.clone());
                self.materials.library.refresh(folder);
            }
            self.materials.library_restored = true;
        }
        if self
            .organization
            .material
            .is_some_and(|id| !self.history.materials().contains_key(&id))
        {
            self.activate_material(None);
        }
        self.materials.refresh_counts(&self.history);
        self.materials.library.poll();
        self.materials.rows_built = 0;

        self.material_selection_readout(ui);
        let previous_view = self.materials.view;
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.materials.view, MaterialsView::Browse, "Browse");
            ui.selectable_value(&mut self.materials.view, MaterialsView::Edit, "Edit");
            ui.selectable_value(&mut self.materials.view, MaterialsView::Library, "Library");
        });
        if previous_view != self.materials.view {
            self.finish_material_edits();
        }
        ui.separator();
        match self.materials.view {
            MaterialsView::Browse => self.material_browser(ui),
            MaterialsView::Edit => {
                egui::ScrollArea::vertical()
                    .id_salt("material_edit_scroll")
                    .show(ui, |ui| {
                        self.material_editor(ui);
                    });
            }
            MaterialsView::Library => {
                egui::ScrollArea::vertical()
                    .id_salt("material_library_scroll")
                    .show(ui, |ui| {
                        self.builtin_section(ui);
                        self.library_section(ui);
                    });
            }
        }
        if self.materials.view != MaterialsView::Browse {
            self.material_action_feedback(ui);
        }
        self.material_modals(ui);
    }

    /// Set paint explicitly, finishing any shared edit before switching slots.
    pub(crate) fn activate_material(&mut self, id: Option<MaterialId>) {
        self.finish_material_edits();
        let id = id.filter(|id| self.history.materials().contains_key(id));
        self.organization.material = id;
        let material = id.and_then(|id| self.history.materials().get(&id));
        self.organization.material_name = material.map_or("Default", |m| m.name.as_str()).into();
        self.organization.texture = material.and_then(|m| m.texture.clone()).unwrap_or_default();
        if let Some(material) = material {
            self.organization.color = material.color;
            self.organization.opacity = material.opacity;
            self.materials.hex = cindral_core::format_hex_rgb(material.color);
        }
        self.materials.hex_material = id;
        self.materials.hex_dirty = false;
        self.materials.size_buffers = None;
        self.materials.mapping_invalid = false;
        self.materials.eyedropper = false;
        self.materials.recent_materials.retain(|entry| {
            *entry != id && entry.is_none_or(|id| self.history.materials().contains_key(&id))
        });
        self.materials.recent_materials.insert(0, id);
        self.materials.recent_materials.truncate(12);
        self.paint_hover = Default::default();
    }

    pub(crate) fn reset_material_document_state(&mut self) {
        self.materials.recent_materials.clear();
        self.materials.material_tiles.clear();
        self.materials.counts_revision = None;
        self.materials.drag_original = None;
        self.materials.name_original = None;
        self.materials.mapping_session = None;
        self.materials.size_buffers = None;
        self.materials.hex_material = None;
        self.materials.hex.clear();
        self.materials.hex_dirty = false;
        self.materials.search.clear();
        self.materials.picker_open = false;
        self.materials.eyedropper = false;
        self.materials.rename = None;
        self.materials.replace = None;
        self.materials.replace_target = None;
        self.materials.delete = None;
        self.materials.notice = None;
        self.materials.mapping_invalid = false;
        self.paint_hover = Default::default();
    }

    pub(crate) fn finish_material_edits(&mut self) {
        self.commit_name_edit();
        if let Some(id) = self.organization.material {
            self.commit_material_drag(id);
            self.mapping_session_commit(id);
        }
    }

    fn material_browser(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.materials.search)
                    .hint_text("Search materials")
                    .desired_width((ui.available_width() - 118.).max(80.)),
            );
            egui::ComboBox::from_id_salt("material_sort")
                .width(100.)
                .selected_text(match self.materials.sort {
                    MaterialSort::Name => "Sort: name",
                    MaterialSort::Usage => "Sort: usage",
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.materials.sort, MaterialSort::Name, "Name");
                    ui.selectable_value(&mut self.materials.sort, MaterialSort::Usage, "Usage");
                });
        });
        ui.horizontal_wrapped(|ui| {
            if ui.button("New color").clicked() {
                self.create_basic_material("New material", [200, 200, 200]);
                self.materials.view = MaterialsView::Edit;
            }
            if ui.button("Import image…").clicked() {
                self.import_material_image();
            }
            if ui.button("Collections…").clicked() {
                self.materials.view = MaterialsView::Library;
            }
        });
        let entries = self.materials.visible_entries(&self.history);
        if entries.is_empty() && !self.materials.search.trim().is_empty() {
            ui.label("No matching materials. Default clears an assignment.");
        }
        let ids: Vec<Option<MaterialId>> = std::iter::once(None)
            .chain(entries.iter().map(|e| Some(e.0)))
            .collect();
        let list_id = ui.make_persistent_id("materials_list_focus");
        ui.interact(
            egui::Rect::from_min_max(ui.cursor().min, ui.max_rect().max),
            list_id,
            egui::Sense::focusable_noninteractive(),
        );
        let mut scroll_index = None;
        if ui.memory(|m| m.has_focus(list_id)) {
            let index = ids
                .iter()
                .position(|id| *id == self.organization.material)
                .unwrap_or(0);
            let next = ui.input_mut(|i| {
                if i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown) {
                    Some((index + 1).min(ids.len() - 1))
                } else if i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp) {
                    Some(index.saturating_sub(1))
                } else {
                    None
                }
            });
            if let Some(next) = next {
                self.activate_material(ids[next]);
                scroll_index = Some(next);
            }
        }
        let keyboard_focus = ui.memory(|m| m.has_focus(list_id));
        let mut scroll = egui::ScrollArea::vertical()
            .id_salt("materials_browser_list")
            .max_height(
                (ui.available_height()
                    - if self.materials.notice.is_some() {
                        190.
                    } else {
                        92.
                    })
                .max(50.),
            )
            .auto_shrink([false, false]);
        if let Some(index) = scroll_index {
            scroll = scroll
                .vertical_scroll_offset(index as f32 * (ROW_HEIGHT + ui.spacing().item_spacing.y));
        }
        scroll.show_rows(ui, ROW_HEIGHT, ids.len(), |ui, rows| {
            ui.spacing_mut().item_spacing.y = 9.;
            for row in rows {
                self.materials.rows_built += 1;
                let id = ids[row];
                let name = id
                    .and_then(|id| self.history.materials().get(&id))
                    .map_or("Default", |m| m.name.as_str())
                    .to_owned();
                let (picked, response) =
                    self.material_choice_row(ui, id, &name, self.organization.material == id, true);
                if keyboard_focus && scroll_index.is_some() && self.organization.material == id {
                    response.scroll_to_me(None);
                }
                if picked {
                    self.activate_material(id);
                    ui.memory_mut(|m| m.request_focus(list_id));
                }
            }
        });
        ui.separator();
        self.material_actions(ui);
    }

    fn material_choice_row(
        &mut self,
        ui: &mut egui::Ui,
        id: Option<MaterialId>,
        name: &str,
        selected: bool,
        actions: bool,
    ) -> (bool, egui::Response) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.y = 0.;
            ui.spacing_mut().button_padding.y = 4.;
            let thumb = self.material_thumbnail(ui.ctx(), id);
            let color = id
                .and_then(|id| self.history.materials().get(&id))
                .map_or(Color32::from_rgb(148, 159, 172), |m| {
                    Color32::from_rgb(m.color[0], m.color[1], m.color[2])
                });
            let button = egui::Button::new("")
                .fill(color)
                .min_size(egui::vec2(28., 32.));
            let swatch = if let Some(tile) = thumb {
                ui.add(egui::Button::image(
                    egui::Image::from_texture(&tile).fit_to_exact_size(egui::vec2(24., 24.)),
                ))
            } else {
                ui.add(button)
            };
            let usage = self.materials.usage(id);
            let reserve = if actions && id.is_some() { 68. } else { 36. };
            let label = ui
                .add_sized(
                    [(ui.available_width() - reserve).max(30.), ROW_HEIGHT],
                    egui::Button::new(name).selected(selected).truncate(),
                )
                .on_hover_text(format!("{name} · {usage} uses · choose for painting"));
            ui.label(usage.to_string());
            if let Some(id) = id.filter(|_| actions) {
                if ui
                    .add_sized([28., 32.], egui::Button::new("..."))
                    .on_hover_text("Material actions")
                    .clicked()
                {
                    self.materials.rename = Some((id, name.to_owned()));
                }
            }
            (
                swatch.on_hover_text(name).clicked() || label.clicked(),
                label,
            )
        })
        .inner
    }

    pub(crate) fn material_thumbnail(
        &mut self,
        ctx: &egui::Context,
        id: Option<MaterialId>,
    ) -> Option<egui::TextureHandle> {
        let key = self.history.materials().get(&id?)?.texture.clone()?;
        if let Some(tile) = self.materials.material_tiles.get(&key) {
            return Some(tile.clone());
        }
        // Reuse an already prepared 64px mip: no source decode in a paint frame.
        let pixels = self.scene.texture_preview(&key)?;
        let image = egui::ColorImage::from_rgba_unmultiplied(
            [pixels.width as usize, pixels.height as usize],
            &pixels.rgba,
        );
        let tile = ctx.load_texture(format!("material-preview-{key}"), image, Default::default());
        self.materials.material_tiles.insert(key, tile.clone());
        Some(tile)
    }

    fn inherited_material(&self, path: &[cindral_core::InstanceId]) -> Option<MaterialId> {
        (1..=path.len())
            .rev()
            .find_map(|len| self.history.instance(&path[..len]).and_then(|n| n.material))
    }

    fn inspected_material(&self) -> InspectedMaterial {
        let mut values = BTreeSet::new();
        let mut inherited = false;
        for face in &self.selection.faces {
            if let Some(face) = self.history.model().faces().get(face) {
                let parent = self.inherited_material(self.history.context());
                inherited |= face.material.is_none() && parent.is_some();
                values.insert(face.material.or(parent));
            }
        }
        for path in self
            .organization
            .multiple
            .iter()
            .chain(self.organization.selected.iter())
        {
            if self.history.instance(path).is_some() {
                values.insert(self.inherited_material(path));
            }
        }
        match values.len() {
            0 => InspectedMaterial::Empty,
            1 => InspectedMaterial::Single(*values.first().unwrap(), inherited),
            _ => InspectedMaterial::Mixed,
        }
    }

    fn material_selection_readout(&mut self, ui: &mut egui::Ui) {
        match self.inspected_material() {
            InspectedMaterial::Empty => {
                ui.weak("Select a face to inspect its material.");
            }
            InspectedMaterial::Mixed => {
                ui.label("Selection: mixed materials");
            }
            InspectedMaterial::Single(id, inherited) => {
                let name = id
                    .and_then(|id| self.history.materials().get(&id))
                    .map_or("Default", |m| m.name.as_str());
                let kind = if self.organization.selected.is_some()
                    || !self.organization.multiple.is_empty()
                {
                    "Container assignment"
                } else {
                    "Selected face"
                };
                ui.add(
                    egui::Label::new(format!(
                        "{kind}: {name}{}",
                        if inherited { " (inherited)" } else { "" }
                    ))
                    .truncate(),
                );
                if ui.button("Use for painting").clicked() {
                    self.activate_material(id);
                }
            }
        }
    }

    fn import_material_image(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("Texture image", &["png", "jpg", "jpeg"])
            .pick_file()
        {
            self.import_library_material(&path);
        }
    }

    pub(crate) fn open_material_picker(&mut self) {
        self.materials.picker_open = true;
        self.materials.picker_focus_search = true;
        self.materials.picker_focus = 0;
        self.materials.picker_search.clear();
    }

    pub(crate) fn material_quick_picker(&mut self, ctx: &egui::Context) {
        if !self.materials.picker_open {
            return;
        }
        self.materials.refresh_counts(&self.history);
        let mut picked = None;
        let mut close = false;
        let modal = egui::Modal::new(egui::Id::new("material_quick_pick")).show(ctx, |ui| {
            ui.set_width(320.);
            ui.heading("Choose paint material");
            let search = ui.add(
                egui::TextEdit::singleline(&mut self.materials.picker_search)
                    .hint_text("Search materials")
                    .desired_width(f32::INFINITY),
            );
            if self.materials.picker_focus_search {
                search.request_focus();
                self.materials.picker_focus_search = false;
            }
            if search.changed() {
                self.materials.picker_focus = 0;
            }
            ui.horizontal(|ui| {
                if ui
                    .selectable_value(&mut self.materials.picker_recent, false, "In model")
                    .changed()
                {
                    self.materials.picker_focus = 0;
                }
                if ui
                    .selectable_value(&mut self.materials.picker_recent, true, "Recent materials")
                    .changed()
                {
                    self.materials.picker_focus = 0;
                }
            });
            let query = self.materials.picker_search.to_lowercase();
            let mut ids: Vec<Option<MaterialId>> = if self.materials.picker_recent {
                self.materials.recent_materials.clone()
            } else {
                std::iter::once(None)
                    .chain(
                        self.materials
                            .visible_entries(&self.history)
                            .into_iter()
                            .map(|e| Some(e.0)),
                    )
                    .collect()
            };
            ids.retain(|id| match id {
                None => query.is_empty() || "default".contains(&query),
                Some(id) => self
                    .history
                    .materials()
                    .get(id)
                    .is_some_and(|m| m.name.to_lowercase().contains(&query)),
            });
            if ids.is_empty() {
                ui.label("No matching materials.");
            }
            self.materials.picker_focus =
                self.materials.picker_focus.min(ids.len().saturating_sub(1));
            let moved = ui.input_mut(|i| {
                if i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown) {
                    self.materials.picker_focus =
                        (self.materials.picker_focus + 1).min(ids.len().saturating_sub(1));
                    true
                } else if i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp) {
                    self.materials.picker_focus = self.materials.picker_focus.saturating_sub(1);
                    true
                } else {
                    false
                }
            });
            if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter)) {
                picked = ids.get(self.materials.picker_focus).copied();
            }
            let mut scroll = egui::ScrollArea::vertical()
                .id_salt("quick_material_rows")
                .max_height(280.);
            if moved {
                scroll = scroll.vertical_scroll_offset(
                    self.materials.picker_focus as f32 * (ROW_HEIGHT + ui.spacing().item_spacing.y),
                );
            }
            scroll.show_rows(ui, ROW_HEIGHT, ids.len(), |ui, range| {
                for row in range {
                    let id = ids[row];
                    let name = id
                        .and_then(|id| self.history.materials().get(&id))
                        .map_or("Default", |m| m.name.as_str())
                        .to_owned();
                    let (clicked, response) = self.material_choice_row(
                        ui,
                        id,
                        &name,
                        self.materials.picker_focus == row,
                        false,
                    );
                    if moved && self.materials.picker_focus == row {
                        response.scroll_to_me(None);
                    }
                    if clicked {
                        picked = Some(id);
                    }
                }
            });
            ui.weak("Up/Down choose · Enter uses it · Esc closes");
            if ui.button("Open Materials inspector").clicked() {
                self.show_tray = true;
                self.inspector_tab = crate::workspace_ui::InspectorTab::Materials;
                close = true;
            }
        });
        if let Some(id) = picked {
            self.activate_material(id);
            close = true;
        }
        if close || modal.should_close() {
            self.materials.picker_open = false;
            ctx.memory_mut(|m| {
                if let Some(id) = m.focused() {
                    m.surrender_focus(id);
                }
            });
        }
    }

    fn material_editor(&mut self, ui: &mut egui::Ui) {
        let Some(id) = self.organization.material else {
            ui.label(
                RichText::new("Default material — painting with it clears materials")
                    .weak()
                    .small(),
            );
            ui.horizontal_wrapped(|ui| {
                if ui.button("New color").clicked() {
                    self.create_basic_material("New material", [200, 200, 200]);
                }
                if ui.button("Import image…").clicked() {
                    self.import_material_image();
                }
            });
            if ui.button("Eyedropper").clicked() {
                self.choose_modify(cindral_tools::ModifyKind::PaintBucket);
                self.materials.eyedropper = true;
            }
            return;
        };
        let Some(material) = self.history.materials().get(&id).cloned() else {
            return;
        };

        ui.strong("Edit shared material");
        ui.label(format!(
            "Changes update all {} uses of {}.",
            self.materials.usage(Some(id)),
            material.name
        ));
        if self.materials.hex_material != Some(id) {
            self.materials.hex_material = Some(id);
            self.materials.hex = cindral_core::format_hex_rgb(material.color);
        }

        // Name: live preview, one history entry on focus loss or Enter.
        if self
            .materials
            .name_original
            .as_ref()
            .is_some_and(|(n, _)| *n != id)
        {
            self.materials.name_original = None;
        }
        if self.materials.name_original.is_none() {
            self.materials.name_buffer = material.name.clone();
        }
        let name =
            ui.add(egui::TextEdit::singleline(&mut self.materials.name_buffer).hint_text("Name"));
        if name.changed() {
            if self.materials.name_original.is_none() {
                self.materials.name_original = Some((id, material.clone()));
            }
            let trimmed = self.materials.name_buffer.trim().to_owned();
            if trimmed.is_empty() {
                self.materials.name_buffer = material.name.clone();
            } else {
                let edited = Material {
                    name: trimmed,
                    ..material.clone()
                };
                let _ = self.history.material_preview(id, edited);
                self.scene_mark_dirty();
            }
        }
        if name.lost_focus() || ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            self.commit_name_edit();
        }

        // Color: egui's editor provides the HSV spectrum, RGB sliders and hex
        // input inside its popup; every change previews live. It takes the
        // display sRGB bytes directly, matching Material storage.
        let mut srgb = material.color;
        let color = ui.color_edit_button_srgb(&mut srgb);
        if color.changed() {
            self.materials.editing_widgets = true;
            if srgb != material.color {
                if self.materials.drag_original.is_none() {
                    self.materials.drag_original = Some((id, material.clone()));
                }
                let edited = Material {
                    color: srgb,
                    ..material.clone()
                };
                let _ = self.history.material_preview(id, edited);
                self.scene_mark_dirty();
            }
        }

        // Opacity slider (0-100%), previewed live, committed on release.
        let mut percent = material.opacity * 100.;
        let slider = ui.add(egui::Slider::new(&mut percent, 0.0..=100.).text("Opacity %"));
        if slider.drag_started() {
            self.materials.drag_original = Some((id, material.clone()));
        }
        if slider.changed() {
            let edited = Material {
                opacity: percent / 100.,
                ..material.clone()
            };
            let _ = self.history.material_preview(id, edited);
            self.scene_mark_dirty();
        }
        if slider.drag_stopped() {
            self.commit_material_drag(id);
        }

        // A frame with no widget changes ends a live edit session.
        if self.materials.editing_widgets && !color.changed() && !slider.changed() {
            self.materials.editing_widgets = false;
            self.commit_material_drag(id);
        }

        // Standalone hex field for exact #RRGGBB entry.
        ui.horizontal(|ui| {
            if !self.materials.hex_dirty {
                self.materials.hex =
                    cindral_core::format_hex_rgb(self.history.materials()[&id].color);
            }
            let hex = ui.add(
                egui::TextEdit::singleline(&mut self.materials.hex)
                    .hint_text("#RRGGBB")
                    .desired_width(84.)
                    .font(egui::TextStyle::Monospace),
            );
            if hex.changed() {
                self.materials.hex_dirty = true;
                self.materials.hex = self.materials.hex.trim().to_owned();
            }
            let apply = ui.small_button("Set");
            if (apply.clicked()
                || (hex.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter))))
                && !self.materials.hex.is_empty()
            {
                match cindral_core::parse_hex_rgb(&self.materials.hex) {
                    Some(color) => {
                        let edited = Material {
                            color,
                            ..self.history.materials()[&id].clone()
                        };
                        self.org_command(Org::UpdateMaterial {
                            id,
                            material: edited,
                        });
                        self.record_recent_color(color);
                        self.materials.hex_dirty = false;
                    }
                    None => {
                        self.error =
                            Some(format!("\"{}\" is not a #RRGGBB color", self.materials.hex))
                    }
                }
            }
        });

        if ui.button("Load PNG/JPEG…").clicked() {
            self.load_material_texture();
        }
        egui::CollapsingHeader::new("Advanced asset reference").show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                let slot = ui.add(
                    egui::TextEdit::singleline(&mut self.organization.texture)
                        .hint_text("Texture asset slot")
                        .desired_width(110.),
                );
                let apply = ui.small_button("Set");
                if apply.clicked()
                    || (slot.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)))
                {
                    let texture = (!self.organization.texture.trim().is_empty())
                        .then(|| self.organization.texture.trim().to_owned());
                    if texture != material.texture {
                        let edited = Material {
                            texture,
                            ..self.history.materials()[&id].clone()
                        };
                        self.org_command(Org::UpdateMaterial {
                            id,
                            material: edited,
                        });
                        self.request_texture_sync();
                    }
                }
            });
        });

        self.texture_mapping_editor(ui, id, &material);
        self.recent_colors_row(ui);
        let _ = material;
    }

    /// Real-world texture size, rotation and flip for the edited material.
    ///
    /// Sizes accept the shared measurements parser (`60cm`, `2'`, `1/2"`);
    /// typing previews valid values live and commits exactly one undo entry
    /// on Enter or focus loss (Escape reverts); sliders and buttons commit
    /// one entry per completed edit.
    fn texture_mapping_editor(&mut self, ui: &mut egui::Ui, id: MaterialId, material: &Material) {
        if material.texture.is_none() {
            return;
        }
        // A session for another material ends where it stands.
        if self
            .materials
            .mapping_session
            .as_ref()
            .is_some_and(|(n, _, _)| *n != id)
        {
            self.materials.mapping_session = None;
        }
        if self
            .materials
            .size_buffers
            .as_ref()
            .is_some_and(|(n, _, _)| *n != id)
        {
            self.materials.size_buffers = None;
        }
        if self.materials.size_buffers.is_none() {
            let settings = self.presentation.measurements;
            let mapping = material.texture_mapping.unwrap_or_default();
            let (width, height) = Self::size_field_texts(&mapping, settings);
            self.materials.size_buffers = Some((id, width, height));
        }
        ui.strong("Size of one image repeat");
        ui.label("Accepts m, cm, mm, ft and in. Images with several planks repeat as a whole.");
        let mapping = material.texture_mapping.unwrap_or_default();
        let lock_aspect = self.materials.lock_aspect;

        let mut buffers = self.materials.size_buffers.take().unwrap();
        ui.label("Width");
        let size = ui.add(
            egui::TextEdit::singleline(&mut buffers.1)
                .desired_width(64.)
                .font(egui::TextStyle::Monospace),
        );
        ui.label("Height");
        let height = ui.add(
            egui::TextEdit::singleline(&mut buffers.2)
                .desired_width(64.)
                .font(egui::TextStyle::Monospace),
        );
        let lock = ui
            .selectable_label(lock_aspect, "Lock aspect ratio")
            .on_hover_text("Lock aspect ratio: editing one side scales the other");
        if lock.clicked() {
            self.materials.lock_aspect = !lock_aspect;
        }
        if size.changed() || height.changed() {
            self.materials.mapping_invalid = !self.mapping_size_preview(
                id,
                &buffers.1,
                &buffers.2,
                size.changed() && !height.changed(),
            );
            if !self.materials.mapping_invalid && lock_aspect {
                let mapping = self.history.materials()[&id]
                    .texture_mapping
                    .unwrap_or_default();
                let (width_text, height_text) =
                    Self::size_field_texts(&mapping, self.presentation.measurements);
                if size.changed() && !height.changed() {
                    buffers.2 = height_text;
                } else if height.changed() && !size.changed() {
                    buffers.1 = width_text;
                }
            }
        }
        if self.materials.mapping_invalid {
            ui.colored_label(
                ui.visuals().error_fg_color,
                "Enter a positive width and height with units; the last valid size is unchanged.",
            );
        }
        let focused = size.has_focus() || height.has_focus();
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) && focused {
            self.mapping_session_revert(id);
            self.materials.mapping_invalid = false;
            self.materials.size_buffers = None;
        } else if (ui.input(|i| i.key_pressed(egui::Key::Enter)) && focused)
            || size.lost_focus()
            || height.lost_focus()
        {
            if self.materials.mapping_invalid {
                self.mapping_session_revert(id);
                self.materials.size_buffers = Some(buffers);
            } else {
                self.mapping_size_commit(id, &buffers.1, &buffers.2);
                self.materials.size_buffers = None;
            }
        } else {
            self.materials.size_buffers = Some(buffers);
        }

        let mut rotation = mapping.rotation_deg;
        let rot = ui.add(
            egui::Slider::new(&mut rotation, -360.0..=360.0)
                .text("Rotation °")
                .step_by(1.),
        );
        if rot.drag_started() && self.materials.mapping_session.is_none() {
            self.materials.mapping_session = Some((id, material.clone(), false));
        }
        if rot.changed() {
            if self.materials.mapping_session.is_none() {
                self.materials.mapping_session = Some((id, material.clone(), false));
            }
            let edited = Material {
                texture_mapping: Some(cindral_core::TextureMapping {
                    rotation_deg: rotation,
                    ..mapping
                }),
                ..self.history.materials()[&id].clone()
            };
            let _ = self.history.material_preview(id, edited);
            self.scene_mark_dirty();
        }
        if rot.drag_stopped() {
            self.mapping_session_commit(id);
        }
        // A frame with no slider changes ends a rotation edit session.
        if self
            .materials
            .mapping_session
            .as_ref()
            .is_some_and(|(_, _, text)| !*text)
            && !rot.changed()
        {
            self.mapping_session_commit(id);
        }
        let flip_h = ui
            .selectable_label(mapping.flip_h, "Flip H")
            .on_hover_text("Mirror the texture horizontally");
        if flip_h.clicked() {
            self.commit_mapping_value(
                id,
                cindral_core::TextureMapping {
                    flip_h: !mapping.flip_h,
                    ..mapping
                },
            );
        }
        let flip_v = ui
            .selectable_label(mapping.flip_v, "Flip V")
            .on_hover_text("Mirror the texture vertically");
        if flip_v.clicked() {
            self.commit_mapping_value(
                id,
                cindral_core::TextureMapping {
                    flip_v: !mapping.flip_v,
                    ..mapping
                },
            );
        }
        let (image_w, image_h) = self.material_texture_size(material);
        let reset = ui
            .button("Reset to image size")
            .on_hover_text("Physical size at 96 DPI");
        if reset.clicked() {
            self.commit_mapping_value(
                id,
                cindral_core::TextureMapping::from_pixels(image_w, image_h),
            );
            self.materials.size_buffers = None;
        }
    }

    /// Parse the size text buffers and preview the resulting mapping.
    ///
    /// Returns false when the text does not yet form a valid mapping (zero,
    /// negative or unparseable); the document keeps its last valid value.
    fn mapping_size_preview(
        &mut self,
        id: MaterialId,
        width_text: &str,
        height_text: &str,
        width_drove: bool,
    ) -> bool {
        let current = self.history.materials()[&id].clone();
        let base = self
            .materials
            .mapping_session
            .as_ref()
            .filter(|(n, _, _)| *n == id)
            .map_or(&current, |(_, m, _)| m)
            .texture_mapping
            .unwrap_or_default();
        let aspect = base.width_m / base.height_m.max(1e-9);
        let lock_aspect = self.materials.lock_aspect;
        let parsed = |text: &str| cindral_tools::parse_length(text);
        let width = parsed(width_text);
        let height_value = parsed(height_text);
        let (new_w, new_h) = match (width, height_value) {
            (Ok(w), Ok(h)) => {
                if lock_aspect && width_drove {
                    (w, w / aspect.max(1e-9))
                } else if lock_aspect {
                    (h * aspect.max(1e-9), h)
                } else {
                    (w, h)
                }
            }
            (Ok(w), Err(_)) if height_text.trim().is_empty() => (
                w,
                if lock_aspect {
                    w / aspect.max(1e-9)
                } else {
                    base.height_m
                },
            ),
            (Err(_), Ok(h)) if width_text.trim().is_empty() => (
                if lock_aspect {
                    h * aspect.max(1e-9)
                } else {
                    base.width_m
                },
                h,
            ),
            _ => return false,
        };
        if !new_w.is_finite() || !new_h.is_finite() || new_w <= 0. || new_h <= 0. {
            return false;
        }
        let updated = cindral_core::TextureMapping {
            width_m: new_w,
            height_m: new_h,
            ..base
        };
        if updated == base {
            return true;
        }
        if self.materials.mapping_session.is_none() {
            self.materials.mapping_session = Some((id, current.clone(), true));
        }
        let edited = Material {
            texture_mapping: Some(updated),
            ..current
        };
        let _ = self.history.material_preview(id, edited);
        self.scene_mark_dirty();
        true
    }

    /// Commit the size text session: one undo entry when the final text
    /// forms a mapping that differs from the pre-edit value; otherwise the
    /// preview is silently restored (no entry, no invalid value recorded).
    fn mapping_size_commit(&mut self, id: MaterialId, width_text: &str, height_text: &str) {
        if self.mapping_size_preview(id, width_text, height_text, true) {
            self.mapping_session_commit(id);
        } else {
            self.mapping_session_revert(id);
        }
    }

    /// Record one undo entry covering the live mapping session (if any).
    fn mapping_session_commit(&mut self, id: MaterialId) {
        if let Some((session_id, original, _)) = self.materials.mapping_session.take() {
            if session_id == id {
                if let Some(edited) = self.history.materials().get(&id).cloned() {
                    if edited != original {
                        if let Err(e) = self.history.finish_material_edit(id, original, edited) {
                            self.error = Some(e.to_string());
                        }
                        self.refresh_mesh();
                    }
                }
            }
        }
    }

    /// Discard the live mapping session and restore the pre-edit material.
    fn mapping_session_revert(&mut self, id: MaterialId) {
        if let Some((session_id, original, _)) = self.materials.mapping_session.take() {
            if session_id == id {
                let _ = self.history.material_preview(id, original);
                self.scene_mark_dirty();
            }
        }
    }

    /// Commit a discrete mapping change (flip toggle, size reset) as one
    /// undo entry.
    fn commit_mapping_value(&mut self, id: MaterialId, mapping: cindral_core::TextureMapping) {
        let edited = Material {
            texture_mapping: Some(mapping),
            ..self.history.materials()[&id].clone()
        };
        self.org_command(Org::UpdateMaterial {
            id,
            material: edited,
        });
    }

    /// Decoded pixel size of a material's texture (0,0 when unavailable).
    ///
    /// Results are cached per texture slot: decoding a 4096² image on every
    /// frame would stall the UI.
    fn material_texture_size(&mut self, material: &Material) -> (u32, u32) {
        let Some(key) = material.texture.clone() else {
            return (0, 0);
        };
        if let Some(size) = self.materials.texture_sizes.get(&key) {
            return *size;
        }
        let size = self
            .files
            .textures
            .get(&key)
            .and_then(|asset| {
                cindral_io::decode_texture(asset)
                    .ok()
                    .map(|t| (t.width, t.height))
            })
            .unwrap_or((0, 0));
        self.materials.texture_sizes.insert(key, size);
        size
    }

    /// Formatted width/height text for the size fields.
    fn size_field_texts(
        mapping: &cindral_core::TextureMapping,
        settings: cindral_core::MeasurementSettings,
    ) -> (String, String) {
        let digits = usize::from(settings.precision).clamp(1, 6);
        let text = format!(
            "{:.*} × {:.*}",
            digits, mapping.width_m, digits, mapping.height_m
        );
        let mut parts = text.split(" × ");
        (
            parts.next().unwrap_or("").to_string(),
            parts.next().unwrap_or("").to_string(),
        )
    }

    /// Duplicate a material with a unique name and select the copy; one
    /// undo entry.
    fn duplicate_material(&mut self, material_id: MaterialId) {
        if let Some(current) = self.history.materials().get(&material_id).cloned() {
            let copy = Material {
                name: unique_material_name(&self.history, &current.name),
                ..current
            };
            self.org_command(Org::CreateMaterial(copy));
            self.select_last_material();
        }
    }

    /// Select everything using a material, matching the panel's usage count:
    /// faces in the active context, instances carrying the material anywhere
    /// in the tree, and instances of definitions whose own faces use it
    /// (painted faces inside a component cannot be selected from outside).
    fn select_material_users(&mut self, material: MaterialId) {
        let mut selection = cindral_tools::Selection::default();
        for face in faces_with_material(self.history.model(), Some(material)) {
            selection.faces.insert(face);
        }
        self.selection = selection;
        self.selection_start = None;
        let active = self.history.active_definition();
        let carried: BTreeSet<cindral_core::DefinitionId> = self
            .history
            .definitions()
            .filter(|&d| d != active)
            .filter(|&d| {
                self.history
                    .definition_model(d)
                    .is_some_and(|m| !faces_with_material(m, Some(material)).is_empty())
            })
            .collect();
        let mut paths = Vec::new();
        visit_material_users(
            &self.history,
            cindral_core::DefinitionId(0),
            Vec::new(),
            material,
            &carried,
            &mut paths,
        );
        if paths.len() == 1 {
            self.organization.selected = paths.pop();
            self.organization.multiple.clear();
        } else {
            self.organization.selected = None;
            self.organization.multiple = paths.into_iter().collect();
        }
    }

    fn recent_colors_row(&mut self, ui: &mut egui::Ui) {
        let recents = self.png.options.recent_colors.clone();
        if recents.is_empty() {
            return;
        }
        let id = self.organization.material;
        let mut picked: Option<[u8; 3]> = None;
        ui.horizontal_wrapped(|ui| {
            ui.label("Recent color values (edits all uses)");
            for color in recents {
                let response = ui
                    .add_sized(
                        [28., 28.],
                        egui::Button::new("").fill(Color32::from_rgb(color[0], color[1], color[2])),
                    )
                    .on_hover_text(format!(
                        "Set shared color to {}",
                        cindral_core::format_hex_rgb(color)
                    ));
                response.widget_info(|| {
                    egui::WidgetInfo::labeled(
                        egui::WidgetType::Button,
                        ui.is_enabled(),
                        cindral_core::format_hex_rgb(color),
                    )
                });
                if response.clicked() {
                    picked = Some(color);
                }
            }
        });
        if let Some(color) = picked {
            if let Some(id) = id {
                if let Some(current) = self.history.materials().get(&id).cloned() {
                    let edited = Material { color, ..current };
                    self.org_command(Org::UpdateMaterial {
                        id,
                        material: edited,
                    });
                    self.record_recent_color(color);
                }
            } else {
                self.create_basic_material(&cindral_core::format_hex_rgb(color), color);
            }
        }
    }

    fn material_action_notice(&mut self, before: u64, message: String) {
        if self.error.is_none() && self.history.revision() != before {
            self.materials.notice = Some((message, self.history.revision()));
        }
    }

    fn delete_material(&mut self, id: MaterialId) {
        let before = self.history.revision();
        self.org_command(Org::DeleteMaterial(id));
        if self.organization.material == Some(id) {
            self.activate_material(None);
        }
        self.material_action_notice(
            before,
            "Material deleted; its users now use Default.".into(),
        );
    }

    fn material_actions(&mut self, ui: &mut egui::Ui) {
        let unused = self.materials.unused_materials(&self.history);
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(
                    !unused.is_empty(),
                    egui::Button::new(format!("Purge unused ({})", unused.len())),
                )
                .on_disabled_hover_text("Every material is used by a face or container")
                .on_hover_text(
                    "Delete unused materials, including unused active paint; undo in one step",
                )
                .clicked()
            {
                let count = unused.len();
                let before = self.history.revision();
                self.org_command(Org::DeleteMaterials(unused));
                if self
                    .organization
                    .material
                    .is_some_and(|id| !self.history.materials().contains_key(&id))
                {
                    self.activate_material(None);
                }
                self.material_action_notice(before, format!("Purged {count} unused materials."));
            }
            if ui
                .selectable_label(self.materials.eyedropper, "Eyedropper")
                .on_hover_text("Activate Paint and sample a face, including from Default")
                .clicked()
            {
                self.choose_modify(cindral_tools::ModifyKind::PaintBucket);
                self.materials.eyedropper = !self.materials.eyedropper;
            }
        });
        self.material_action_feedback(ui);
    }

    fn material_action_feedback(&mut self, ui: &mut egui::Ui) {
        let Some((message, revision)) = self.materials.notice.clone() else {
            return;
        };
        if revision != self.history.revision() {
            self.materials.notice = None;
            return;
        }
        ui.label(message);
        ui.horizontal(|ui| {
            if ui.button("Undo").clicked() {
                self.history_step(false);
                self.materials.notice = None;
            }
            if ui.button("Dismiss").clicked() {
                self.materials.notice = None;
            }
        });
    }

    fn material_modals(&mut self, ui: &mut egui::Ui) {
        if let Some((id, mut name)) = self.materials.rename.clone() {
            let mut close = false;
            let modal = egui::Modal::new(egui::Id::new("material-actions")).show(ui.ctx(), |ui| {
                ui.set_width(280.);
                ui.heading("Material actions");
                ui.label(format!(
                    "{name} · {} uses across the document",
                    self.materials.usage(Some(id))
                ));
                let field = ui.text_edit_singleline(&mut name);
                if (ui.button("Rename").clicked()
                    || (field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter))))
                    && !name.trim().is_empty()
                {
                    if let Some(current) = self.history.materials().get(&id).cloned() {
                        self.org_command(Org::UpdateMaterial {
                            id,
                            material: Material {
                                name: name.trim().into(),
                                ..current
                            },
                        });
                    }
                    close = true;
                }
                if ui.button("Edit shared properties").clicked() {
                    self.activate_material(Some(id));
                    self.materials.view = MaterialsView::Edit;
                    close = true;
                }
                if ui.button("Duplicate").clicked() {
                    self.duplicate_material(id);
                    close = true;
                }
                if ui.button("Select users").clicked() {
                    self.select_material_users(id);
                    close = true;
                }
                if ui.button("Replace throughout document…").clicked() {
                    self.materials.replace = Some(id);
                    self.materials.replace_target = None;
                    close = true;
                }
                ui.separator();
                if ui.button("Delete material…").clicked() {
                    if self.materials.usage(Some(id)) > 0 {
                        self.materials.delete = Some(id);
                    } else {
                        self.delete_material(id);
                    }
                    close = true;
                }
                if ui.button("Close").clicked() {
                    close = true;
                }
            });
            if close || modal.should_close() {
                self.materials.rename = None;
            } else {
                self.materials.rename = Some((id, name));
            }
        }
        if let Some(id) = self.materials.delete {
            let mut close = false;
            let modal=egui::Modal::new(egui::Id::new("material-delete")).show(ui.ctx(),|ui| {
                ui.set_width(300.);ui.heading("Delete used material?");
                let name=self.history.materials().get(&id).map_or("Deleted material",|m|m.name.as_str());
                ui.label(format!("Delete {name}? Its {} uses across the document will fall back to Default. You can Undo this change.",self.materials.usage(Some(id))));
                if ui.button("Delete and use Default").clicked() {self.delete_material(id);close=true;}
                if ui.button("Cancel").clicked() {close=true;}
            });
            if close || modal.should_close() {
                self.materials.delete = None;
            }
        }
        if let Some(from) = self.materials.replace {
            let mut close = false;
            let modal=egui::Modal::new(egui::Id::new("material-replace")).show(ui.ctx(),|ui| {
                ui.set_width(300.);ui.heading("Replace material");
                let source=self.history.materials().get(&from).map_or("Missing material",|m|m.name.as_str()).to_owned();
                let target=self.materials.replace_target.and_then(|id|self.history.materials().get(&id)).map_or("Choose a target",|m|m.name.as_str());
                ui.label(format!("{source} → {target}"));
                ui.label(format!("Updates all {} uses throughout the document, including definitions and containers.",self.materials.usage(Some(from))));
                let entries:Vec<_>=self.history.materials().iter().filter(|(id,_)|**id!=from).map(|(id,m)|(*id,m.name.clone())).collect();
                if entries.is_empty() {ui.label("Create another material before replacing this one.");}
                egui::ScrollArea::vertical().max_height(240.).show_rows(ui,ROW_HEIGHT,entries.len(),|ui,range| {
                    for row in range {let (id,name)=&entries[row];if ui.add_sized([ui.available_width(),ROW_HEIGHT],egui::Button::new(name).selected(self.materials.replace_target==Some(*id)).truncate()).clicked() {self.materials.replace_target=Some(*id);}}
                });
                if ui.add_enabled(self.materials.replace_target.is_some(),egui::Button::new("Replace all uses")).clicked() {
                    let before=self.history.revision();let to=self.materials.replace_target;
                    self.org_command(Org::ReplaceMaterial{from,to});
                    self.material_action_notice(before,format!("Replaced all uses of {source}."));close=true;
                }
                ui.weak("One undo step. Choosing a row only selects the replacement.");
                if ui.button("Cancel").clicked() {close=true;}
            });
            if close || modal.should_close() {
                self.materials.replace = None;
                self.materials.replace_target = None;
            }
        }
    }

    fn builtin_section(&mut self, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new("Builtin collection")
            .default_open(true)
            .show(ui, |ui| {
                let mut chosen = None;
                for group in [
                    crate::builtin_materials::BuiltinGroup::Neutral,
                    crate::builtin_materials::BuiltinGroup::Wood,
                    crate::builtin_materials::BuiltinGroup::Wall,
                    crate::builtin_materials::BuiltinGroup::Accent,
                ] {
                    ui.label(group.to_string());
                    ui.horizontal_wrapped(|ui| {
                        for color in crate::builtin_materials::BUILTIN_COLORS
                            .iter()
                            .filter(|c| c.group == group)
                        {
                            let response = ui
                                .add_sized(
                                    [28., 28.],
                                    egui::Button::new("").fill(Color32::from_rgb(
                                        color.srgb[0],
                                        color.srgb[1],
                                        color.srgb[2],
                                    )),
                                )
                                .on_hover_text(color.name);
                            response.widget_info(|| {
                                egui::WidgetInfo::labeled(
                                    egui::WidgetType::Button,
                                    ui.is_enabled(),
                                    color.name,
                                )
                            });
                            if response.clicked() {
                                chosen = Some((color.name, color.srgb));
                            }
                        }
                    });
                }
                ui.label("Patterns");
                let mut pattern = None;
                ui.horizontal_wrapped(|ui| {
                    for (index, p) in crate::builtin_materials::BUILTIN_PATTERNS
                        .iter()
                        .enumerate()
                    {
                        let key = format!("builtin-pattern-{index}");
                        let tile = self
                            .materials
                            .library_tiles
                            .entry(key.clone())
                            .or_insert_with(|| {
                                let thumb = (p.generator)(64);
                                ui.ctx().load_texture(
                                    key,
                                    egui::ColorImage::from_rgba_unmultiplied([64, 64], &thumb.rgba),
                                    Default::default(),
                                )
                            })
                            .clone();
                        let response = ui
                            .add(egui::Button::image(
                                egui::Image::from_texture(&tile)
                                    .fit_to_exact_size(egui::vec2(44., 44.)),
                            ))
                            .on_hover_text(p.name);
                        response.widget_info(|| {
                            egui::WidgetInfo::labeled(
                                egui::WidgetType::Button,
                                ui.is_enabled(),
                                p.name,
                            )
                        });
                        if response.clicked() {
                            pattern = Some(index);
                        }
                    }
                });
                if let Some((name, color)) = chosen {
                    self.create_basic_material(name, color);
                }
                if let Some(index) = pattern {
                    self.create_pattern_material(index);
                }
            });
    }

    fn library_section(&mut self, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new("Folder library")
            .default_open(true)
            .show(ui, |ui| {
                if ui.button("Add collection from folder…").clicked() {
                    if let Some(folder) = rfd::FileDialog::new().pick_folder() {
                        self.materials.library.add_folder(folder.clone());
                        self.materials.library.refresh(&folder);
                        let path = folder.display().to_string();
                        if !self.png.options.library_folders.contains(&path) {
                            self.png.options.library_folders.push(path);
                            self.save_preferences();
                        }
                    }
                }
                let folders = self.materials.library.folders().to_vec();
                if folders.is_empty() {
                    ui.label("Add a folder of PNG/JPEG images to build your library.");
                }
                let mut import = None;
                for folder in folders {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(folder.file_name().unwrap_or_default().to_string_lossy());
                        if ui.button("Rescan").clicked() {
                            self.materials.library.refresh(&folder);
                            self.materials
                                .library_tiles
                                .retain(|key, _| !std::path::Path::new(key).starts_with(&folder));
                        }
                        if ui
                            .button("Remove folder")
                            .on_hover_text(
                                "Remove the collection, keeping imported materials and files",
                            )
                            .clicked()
                        {
                            self.materials.library.remove_folder(&folder);
                            self.materials
                                .library_tiles
                                .retain(|key, _| !std::path::Path::new(key).starts_with(&folder));
                            self.png
                                .options
                                .library_folders
                                .retain(|f| f != &folder.display().to_string());
                            self.save_preferences();
                        }
                    });
                    // Borrow the scan rather than cloning every decoded thumbnail each frame.
                    let Some(scan) = self.materials.library.scan(&folder) else {
                        ui.label("Scanning…");
                        continue;
                    };
                    if scan.skipped > 0 {
                        ui.label(format!(
                            "{} files skipped (unsupported, unreadable or too large)",
                            scan.skipped
                        ));
                    }
                    if scan.images.is_empty() {
                        ui.label(
                            "No PNG/JPEG images found. Check the folder and access permissions.",
                        );
                        continue;
                    }
                    let columns = (ui.available_width() / 80.).floor().max(1.) as usize;
                    let rows = scan.images.len().div_ceil(columns);
                    let tiles = &mut self.materials.library_tiles;
                    egui::ScrollArea::vertical()
                        .id_salt(("folder_tiles", &folder))
                        .max_height(264.)
                        .show_rows(ui, 88., rows, |ui, range| {
                            for row in range {
                                ui.horizontal(|ui| {
                                    for image in
                                        scan.images.iter().skip(row * columns).take(columns)
                                    {
                                        let key = image.path.display().to_string();
                                        let tile = tiles
                                            .entry(key.clone())
                                            .or_insert_with(|| {
                                                ui.ctx().load_texture(
                                                    key,
                                                    image.thumb.clone(),
                                                    Default::default(),
                                                )
                                            })
                                            .clone();
                                        ui.allocate_ui_with_layout(
                                            egui::vec2(72., 88.),
                                            egui::Layout::top_down(egui::Align::Min),
                                            |ui| {
                                                ui.spacing_mut().item_spacing.y = 2.;
                                                let response = ui
                                                    .add(egui::Button::image(
                                                        egui::Image::from_texture(&tile)
                                                            .fit_to_exact_size(egui::vec2(
                                                                56., 56.,
                                                            )),
                                                    ))
                                                    .on_hover_text(&image.name);
                                                response.widget_info(|| {
                                                    egui::WidgetInfo::labeled(
                                                        egui::WidgetType::Button,
                                                        ui.is_enabled(),
                                                        &image.name,
                                                    )
                                                });
                                                if response.clicked() {
                                                    import = Some(image.path.clone());
                                                }
                                                ui.add(egui::Label::new(&image.name).truncate());
                                            },
                                        );
                                    }
                                });
                            }
                        });
                }
                if let Some(path) = import {
                    self.import_library_material(&path);
                }
            });
    }

    /// Create a material from a builtin color and make it the paint target.
    fn create_basic_material(&mut self, name: &str, srgb: [u8; 3]) {
        let material = Material {
            name: unique_material_name(&self.history, name),
            color: srgb,
            opacity: 1.,
            texture: None,
            texture_mapping: None,
        };
        self.org_command(Org::CreateMaterial(material));
        self.select_last_material();
        self.record_recent_color(srgb);
    }

    /// Create a textured material from a procedural builtin pattern.
    fn create_pattern_material(&mut self, index: usize) {
        if let Err(e) = self.ensure_builtin_pattern(index) {
            self.error = Some(e);
            return;
        }
        let pattern = crate::builtin_materials::BUILTIN_PATTERNS[index];
        let material = Material {
            name: unique_material_name(&self.history, pattern.name),
            color: pattern.srgb,
            opacity: 1.,
            texture: Some(format!("builtin:{}", pattern.name)),
            texture_mapping: None,
        };
        self.org_command(Org::CreateMaterial(material));
        self.select_last_material();
    }

    /// Import one library image as a textured material (click-to-import).
    fn import_library_material(&mut self, path: &std::path::Path) {
        let result = (|| -> cindral_io::Result<()> {
            let (key, asset) = cindral_io::load_texture(path)?;
            if self.files.textures.len() >= 127 && !self.files.textures.contains_key(&key) {
                return Err(cindral_io::IoError::Invalid(
                    "at most 127 texture assets can be loaded in one document session".into(),
                ));
            }
            self.files.textures.insert(key.clone(), asset);
            let name = path
                .file_stem()
                .map_or_else(|| "Image".into(), |s| s.to_string_lossy().into_owned());
            let material = Material {
                name: unique_material_name(&self.history, &name),
                color: [255, 255, 255],
                opacity: 1.,
                texture: Some(key),
                texture_mapping: None,
            };
            self.org_command(Org::CreateMaterial(material));
            if self.error.is_none() {
                self.select_last_material();
                self.request_texture_sync();
            }
            Ok(())
        })();
        if let Err(e) = result {
            self.error = Some(e.to_string());
        }
    }

    fn select_last_material(&mut self) {
        if self.error.is_none() {
            self.activate_material(self.history.materials().keys().next_back().copied());
        }
    }

    /// Encode a procedural pattern into the runtime asset table once, then
    /// refresh textures off-thread.
    fn ensure_builtin_pattern(&mut self, index: usize) -> Result<(), String> {
        if self.insert_builtin_pattern(index)? {
            self.request_texture_sync();
        }
        Ok(())
    }

    /// Encodes the procedural pattern into the runtime asset table once;
    /// returns true when bytes were newly inserted. No scene sync here —
    /// callers batch one `request_texture_sync` after several inserts.
    fn insert_builtin_pattern(&mut self, index: usize) -> Result<bool, String> {
        let name = crate::builtin_materials::BUILTIN_PATTERNS[index].name;
        let key = format!("builtin:{name}");
        if self.files.textures.contains_key(&key) {
            return Ok(false);
        }
        let pixels = crate::builtin_materials::pattern_texture(index).ok_or("unknown pattern")?;
        let image = image::RgbaImage::from_raw(pixels.width, pixels.height, pixels.rgba)
            .ok_or("pattern image is empty")?;
        let mut bytes = std::io::Cursor::new(Vec::new());
        image
            .write_to(&mut bytes, image::ImageFormat::Png)
            .map_err(|e| format!("pattern encoding failed: {e}"))?;
        self.files.textures.insert(
            key,
            cindral_io::TextureAsset {
                mime: "image/png".into(),
                bytes: bytes.into_inner(),
            },
        );
        Ok(true)
    }

    /// Regenerate procedural pattern slots referenced by a loaded document.
    /// Inserts only; the caller applies one texture sync for all assets.
    pub(crate) fn ensure_builtin_textures(&mut self) {
        let missing: Vec<String> = self
            .history
            .materials()
            .values()
            .filter_map(|m| m.texture.as_ref())
            .filter(|key| key.starts_with("builtin:"))
            .filter(|key| !self.files.textures.contains_key(*key))
            .cloned()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        for key in missing {
            let name = key.trim_start_matches("builtin:");
            let Some(index) = (0..crate::builtin_materials::BUILTIN_PATTERNS.len())
                .find(|i| crate::builtin_materials::BUILTIN_PATTERNS[*i].name == name)
            else {
                continue;
            };
            if let Err(e) = self.insert_builtin_pattern(index) {
                self.error = Some(e);
            }
        }
    }

    /// Mark scene caches dirty after a history-less material preview.
    fn scene_mark_dirty(&mut self) {
        self.refresh_mesh();
    }

    fn commit_material_drag(&mut self, id: MaterialId) {
        if let Some((original_id, original)) = self.materials.drag_original.take() {
            if original_id == id {
                if let Some(edited) = self.history.materials().get(&id).cloned() {
                    if edited != original {
                        self.record_recent_color(edited.color);
                        if let Err(e) = self.history.finish_material_edit(id, original, edited) {
                            self.error = Some(e.to_string());
                        }
                        self.refresh_mesh();
                    }
                }
            }
        }
    }

    fn commit_name_edit(&mut self) {
        if let Some((id, original)) = self.materials.name_original.take() {
            if let Some(edited) = self.history.materials().get(&id).cloned() {
                if edited != original {
                    if let Err(e) = self.history.finish_material_edit(id, original, edited) {
                        self.error = Some(e.to_string());
                    }
                    self.refresh_mesh();
                }
            }
        }
    }

    fn record_recent_color(&mut self, color: [u8; 3]) {
        self.png.options.push_recent_color(color);
        self.save_preferences();
    }

    /// Persist chrome preferences (theme, recents, library folders).
    pub(crate) fn save_preferences(&mut self) {
        let path = crate::png_ui::preferences_path();
        if let Err(e) = self.png.options.save(&path) {
            self.error = Some(format!("Could not save preferences: {e}"));
        }
    }

    // ---- Paint Bucket routing -------------------------------------------

    /// SketchUp-style click routing. Returns true when the press is consumed.
    fn paint_plan(&self, hit: Option<&InstanceHit>, mods: &egui::Modifiers) -> PaintPlan {
        let mut plan = PaintPlan {
            face_path: self.history.context().to_vec(),
            ..Default::default()
        };
        if mods.alt || self.materials.eyedropper {
            return plan;
        }
        let Some(hit) = hit.filter(|h| h.path.starts_with(self.history.context())) else {
            return plan;
        };
        let ctrl = mods.ctrl || mods.command;
        if hit.path.len() > self.history.context().len() {
            let path = hit.path[..self.history.context().len() + 1].to_vec();
            let Some(node) = self.history.instance(&path) else {
                return plan;
            };
            if ctrl && mods.shift {
                plan.face_path = path;
                if let Some(model) = self.history.definition_model(node.definition) {
                    plan.faces = faces_with_effective_material(model, node.material);
                }
            } else if ctrl {
                plan.instances.push(path);
            } else if mods.shift {
                plan.faces = faces_with_effective_material(self.history.model(), node.material);
            } else if self.organization.selected.as_ref() == Some(&path)
                || self.organization.multiple.contains(&path)
            {
                plan.instances = self
                    .organization
                    .multiple
                    .iter()
                    .chain(self.organization.selected.iter())
                    .cloned()
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .collect();
                plan.faces = self.selection.faces.iter().copied().collect();
            } else {
                plan.instances.push(path);
            }
            return plan;
        }
        let cindral_tools::Hit::Face(face) = hit.hit else {
            return plan;
        };
        let model = self.history.model();
        let Some(current) = model.faces().get(&face) else {
            return plan;
        };
        plan.faces = if mods.shift {
            faces_with_material(model, current.material)
        } else if ctrl {
            fill_adjacent_same_material(model, face)
        } else if self.selection.faces.contains(&face) {
            self.selection.faces.iter().copied().collect()
        } else {
            vec![face]
        };
        plan
    }

    pub(crate) fn paint_bucket_click(
        &mut self,
        hit: Option<&InstanceHit>,
        _child: Option<InstancePath>,
        mods: &egui::Modifiers,
    ) -> bool {
        if mods.alt || self.materials.eyedropper {
            if let Some(hit) = hit.filter(|h| h.path.starts_with(self.history.context())) {
                let explicit = match hit.hit {
                    cindral_tools::Hit::Face(face) => self
                        .history
                        .definition_model(hit.definition)
                        .and_then(|m| m.faces().get(&face))
                        .and_then(|f| f.material),
                    _ => None,
                };
                self.activate_material(explicit.or_else(|| self.inherited_material(&hit.path)));
            }
            return true;
        }
        let plan = self.paint_plan(hit, mods);
        let changed = !plan.faces.is_empty() || !plan.instances.is_empty();
        let paint = self.organization.material;
        for path in plan.instances {
            self.org_command(Org::PaintInstance {
                path,
                material: paint,
            });
        }
        if !plan.faces.is_empty() {
            if plan.face_path == self.history.context() {
                self.org_command(Org::PaintFaces {
                    faces: plan.faces,
                    material: paint,
                });
            } else {
                self.paint_in_context(&plan.face_path, plan.faces, paint);
            }
        }
        self.paint_hover = Default::default();
        changed
    }

    /// Paint faces inside a specific definition (the clicked object scope) by
    /// entering that definition, painting once and leaving again.
    fn paint_in_context(
        &mut self,
        path: &InstancePath,
        faces: Vec<FaceId>,
        material: Option<MaterialId>,
    ) {
        let previous = self.history.context().to_vec();
        if self.history.enter(path.clone()).is_ok() {
            self.org_command(Org::PaintFaces { faces, material });
            while self.history.context().len() > previous.len() {
                if self.history.exit().unwrap_or(false) {
                    break;
                }
            }
            self.refresh_mesh();
        }
    }

    /// Update the hover preview for the current hit and modifiers.
    pub(crate) fn paint_hover_update(&mut self, hit: Option<&InstanceHit>, mods: &egui::Modifiers) {
        let face = hit.and_then(|h| match h.hit {
            cindral_tools::Hit::Face(face) => Some(face),
            _ => None,
        });
        let path = hit.map(|h| h.path.clone());
        let flags = u8::from(mods.ctrl || mods.command)
            | u8::from(mods.shift) << 1
            | u8::from(mods.alt || self.materials.eyedropper) << 2;
        let key = (face, path, flags, self.history.revision());
        if self.paint_hover.key == key
            && self.paint_hover.context == self.history.context()
            && self.paint_hover.selected_faces == self.selection.faces
            && self.paint_hover.selected_instances == self.organization.multiple
            && self.paint_hover.selected_instance == self.organization.selected
        {
            return;
        }
        let plan = self.paint_plan(hit, mods);
        self.paint_hover = PaintHover {
            faces: plan.faces.clone(),
            instance: plan.instances.first().cloned(),
            plan,
            key,
            selected_faces: self.selection.faces.clone(),
            selected_instances: self.organization.multiple.clone(),
            selected_instance: self.organization.selected.clone(),
            context: self.history.context().to_vec(),
        };
    }

    /// Modifier-aware Paint Bucket hint for the status bar.
    pub(crate) fn paint_hint(&self, mods: &egui::Modifiers) -> String {
        let ctrl = mods.ctrl || mods.command;
        let shift = mods.shift;
        let material = self
            .organization
            .material
            .and_then(|id| self.history.materials().get(&id))
            .map_or("Default", |m| m.name.as_str());
        let action = if mods.alt {
            "Click to sample the clicked material"
        } else if self.materials.eyedropper {
            "Eyedropper: click a face to sample its material"
        } else if ctrl && shift {
            "Replace matching faces inside the clicked object (shared copies also update)"
        } else if ctrl {
            "Fill adjacent faces sharing the clicked material"
        } else if shift {
            "Replace matching faces in the current edit context"
        } else {
            "Click to paint · Ctrl adjacent · Shift replace in context · Ctrl+Shift inside object · Alt sample"
        };
        let scope = if mods.alt || self.materials.eyedropper {
            String::new()
        } else {
            format!(
                " · {} faces, {} objects",
                self.paint_hover.plan.faces.len(),
                self.paint_hover.plan.instances.len()
            )
        };
        format!("{action}{scope} · painting: {material}")
    }

    /// Outline the region a Paint Bucket click would paint. Adjacency keeps
    /// the preview bounded; large regions draw their bounding box instead.
    pub(crate) fn paint_hover_overlay(
        &self,
        ui: &egui::Ui,
        rect: egui::Rect,
        view: cindral_inference::View,
    ) {
        if !(self.modifying && self.modify.kind() == cindral_tools::ModifyKind::PaintBucket) {
            return;
        }
        let world_view = cindral_inference::View {
            matrix: self
                .camera
                .view_projection(rect.width() / rect.height())
                .as_dmat4(),
            size: view.size,
        };
        let stroke = egui::Stroke::new(
            2_f32,
            self.viewport_overlay(Color32::from_rgb(120, 200, 255)),
        );
        for path in &self.paint_hover.plan.instances {
            if let Some((min, max)) = self.selected_world_bounds(path) {
                paint_scope_bounds(ui, rect, world_view, min, max, stroke);
            }
        }
        if self.paint_hover.faces.is_empty() {
            return;
        }
        let view = if self.paint_hover.plan.face_path == self.history.context() {
            view
        } else {
            let Some(item) = self
                .scene
                .occurrences()
                .iter()
                .find(|o| o.path == self.paint_hover.plan.face_path)
            else {
                return;
            };
            cindral_inference::View {
                matrix: world_view.matrix * item.world,
                size: view.size,
            }
        };
        let screen = |p: cindral_core::DVec3| {
            view.project(p)
                .map(|p| rect.min + egui::vec2(p.x as f32, p.y as f32))
        };
        let stroke = egui::Stroke::new(
            2_f32,
            self.viewport_overlay(Color32::from_rgb(120, 200, 255)),
        );
        let model = if self.paint_hover.plan.face_path == self.history.context() {
            self.history.model()
        } else {
            let Some(node) = self.history.instance(&self.paint_hover.plan.face_path) else {
                return;
            };
            let Some(model) = self.history.definition_model(node.definition) else {
                return;
            };
            model
        };
        if self.paint_hover.faces.len() <= 256 {
            for id in &self.paint_hover.faces {
                let Some(face) = model.faces().get(id) else {
                    continue;
                };
                for ring in std::iter::once(&face.outer).chain(&face.holes) {
                    for (a, b) in ring.iter().zip(ring.iter().cycle().skip(1)) {
                        if let (Some(a), Some(b)) = (
                            screen(model.vertices()[a].position),
                            screen(model.vertices()[b].position),
                        ) {
                            ui.painter().line_segment([a, b], stroke);
                        }
                    }
                }
            }
        } else {
            let mut min = cindral_core::DVec3::splat(f64::INFINITY);
            let mut max = cindral_core::DVec3::splat(f64::NEG_INFINITY);
            for id in &self.paint_hover.faces {
                let Some(face) = model.faces().get(id) else {
                    continue;
                };
                for &v in &face.outer {
                    let p = model.vertices()[&v].position;
                    min = min.min(p);
                    max = max.max(p);
                }
            }
            paint_scope_bounds(ui, rect, view, min, max, stroke);
        }
    }
}

fn paint_scope_bounds(
    ui: &egui::Ui,
    rect: egui::Rect,
    view: cindral_inference::View,
    min: cindral_core::DVec3,
    max: cindral_core::DVec3,
    stroke: egui::Stroke,
) {
    for i in 0..8 {
        let corner = |i| {
            cindral_core::DVec3::new(
                if i & 1 == 0 { min.x } else { max.x },
                if i & 2 == 0 { min.y } else { max.y },
                if i & 4 == 0 { min.z } else { max.z },
            )
        };
        for bit in [1, 2, 4] {
            let j = i ^ bit;
            if i < j {
                if let (Some(a), Some(b)) = (view.project(corner(i)), view.project(corner(j))) {
                    ui.painter().line_segment(
                        [
                            rect.min + egui::vec2(a.x as f32, a.y as f32),
                            rect.min + egui::vec2(b.x as f32, b.y as f32),
                        ],
                        stroke,
                    );
                }
            }
        }
    }
}

/// Names stay unique by appending an incrementing number.
fn unique_material_name(history: &cindral_core::DocumentHistory, base: &str) -> String {
    let mut candidate = base.to_owned();
    let mut n = 2;
    while history.materials().values().any(|m| m.name == candidate) {
        candidate = format!("{base} {n}");
        n += 1;
    }
    candidate
}

/// Collect instance paths that use a material: nodes carrying it directly
/// plus every instance of a definition whose faces use it.
fn visit_material_users(
    history: &cindral_core::DocumentHistory,
    definition: cindral_core::DefinitionId,
    parent: InstancePath,
    material: MaterialId,
    carried: &BTreeSet<cindral_core::DefinitionId>,
    out: &mut Vec<InstancePath>,
) {
    if parent.len() >= 64 || out.len() > 100_000 {
        return;
    }
    if let Some(nodes) = history.instances(definition) {
        for node in nodes.values() {
            let mut path = parent.clone();
            path.push(node.id);
            let uses = node.material == Some(material) || carried.contains(&node.definition);
            if uses {
                out.push(path.clone());
            }
            visit_material_users(history, node.definition, path, material, carried, out);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input_tests;
    use cindral_core::{Command, DVec3, OrganizationCommand as Org};
    use cindral_inference::View;
    use eframe::egui::PointerButton;

    fn position(app: &DrafterApp, p: DVec3) -> egui::Pos2 {
        let view = View {
            matrix: app.camera.view_projection(1.5).as_dmat4(),
            size: glam::DVec2::new(600., 400.),
        };
        let p = view.project(p).unwrap();
        egui::pos2(p.x as f32 + 8., p.y as f32 + 8.)
    }
    fn send(
        app: &mut DrafterApp,
        ctx: &egui::Context,
        events: Vec<egui::Event>,
        mods: egui::Modifiers,
    ) {
        input_tests::frame(app, ctx, events, mods);
    }
    fn button(p: egui::Pos2, pressed: bool, modifiers: egui::Modifiers) -> egui::Event {
        egui::Event::PointerButton {
            pos: p,
            button: PointerButton::Primary,
            pressed,
            modifiers,
        }
    }
    fn click(app: &mut DrafterApp, ctx: &egui::Context, p: egui::Pos2, mods: egui::Modifiers) {
        send(
            app,
            ctx,
            vec![egui::Event::PointerMoved(p), button(p, true, mods)],
            mods,
        );
        send(app, ctx, vec![button(p, false, mods)], mods);
    }
    fn click_at(app: &mut DrafterApp, ctx: &egui::Context, world: DVec3, mods: egui::Modifiers) {
        let p = position(app, world);
        click(app, ctx, p, mods);
    }
    /// 10-unit square: the framing the scripted picker reliably resolves.
    fn square(app: &mut DrafterApp, x: f64, y: f64) {
        app.execute(Command::AddPolyline {
            points: vec![
                DVec3::new(x, y, 0.),
                DVec3::new(x + 10., y, 0.),
                DVec3::new(x + 10., y + 10., 0.),
                DVec3::new(x, y + 10., 0.),
            ],
            closed: true,
        })
        .unwrap();
    }
    fn setup_paint() -> (DrafterApp, egui::Context) {
        let mut app = DrafterApp::default();
        // A adjacent to B; C separate.
        square(&mut app, 0., 0.);
        square(&mut app, 10., 0.);
        square(&mut app, 30., 30.);
        app.camera
            .set_view(cindral_render::camera::StandardView::Top);
        app.camera.projection = cindral_render::camera::Projection::Orthographic;
        app.camera.target = glam::Vec3::new(5., 5., 0.);
        app.camera.distance = 30.;
        for (name, color) in [("Red", [200, 40, 40]), ("Blue", [40, 80, 210])] {
            app.org_command(Org::CreateMaterial(Material {
                name: name.into(),
                color,
                opacity: 1.,
                texture: None,
                texture_mapping: None,
            }));
        }
        app.organization.material = app.history.materials().keys().next_back().copied();
        app.choose_modify(cindral_tools::ModifyKind::PaintBucket);
        let ctx = egui::Context::default();
        send(&mut app, &ctx, vec![], egui::Modifiers::NONE);
        (app, ctx)
    }
    fn face_at(app: &DrafterApp, p: DVec3) -> cindral_core::FaceId {
        *app.model()
            .faces()
            .iter()
            .find(|(_, f)| {
                let mut min = DVec3::splat(f64::INFINITY);
                let mut max = DVec3::splat(f64::NEG_INFINITY);
                for &v in &f.outer {
                    let q = app.model().vertices()[&v].position;
                    min = min.min(q);
                    max = max.max(q);
                }
                min.cmple(p).all() && max.cmpge(p).all()
            })
            .unwrap()
            .0
    }
    fn material_of(app: &DrafterApp, p: DVec3) -> Option<MaterialId> {
        app.model().faces().get(&face_at(app, p)).unwrap().material
    }

    #[test]
    fn paint_click_ctrl_and_shift_scopes_with_single_undo_entries() {
        let (mut app, ctx) = setup_paint();
        let materials: Vec<MaterialId> = app.history.materials().keys().copied().collect();
        let (red, blue) = (materials[0], materials[1]);

        // Plain click paints one face.
        app.organization.material = Some(blue);
        click_at(
            &mut app,
            &ctx,
            DVec3::new(5., 5., 0.),
            egui::Modifiers::NONE,
        );
        assert_eq!(material_of(&app, DVec3::new(5., 5., 0.)), Some(blue));
        assert_eq!(material_of(&app, DVec3::new(15., 5., 0.)), None);
        assert_eq!(app.history.undo_depth(), 6); // three squares, two materials, one paint

        // Ctrl fill-adjacent fills the connected same-material strip only.
        app.organization.material = Some(red);
        let ctrl = egui::Modifiers {
            ctrl: true,
            ..Default::default()
        };
        click_at(&mut app, &ctx, DVec3::new(5., 5., 0.), ctrl);
        assert_eq!(material_of(&app, DVec3::new(5., 5., 0.)), Some(red));
        // The neighbor keeps its (unpainted = default) material: not same-material.
        assert_eq!(material_of(&app, DVec3::new(15., 5., 0.)), None);
        assert_eq!(material_of(&app, DVec3::new(35., 35., 0.)), None);

        // Shift replace paints every face sharing the clicked material.
        app.organization.material = Some(blue);
        let shift = egui::Modifiers {
            shift: true,
            ..Default::default()
        };
        click_at(&mut app, &ctx, DVec3::new(5., 5., 0.), shift);
        assert_eq!(material_of(&app, DVec3::new(5., 5., 0.)), Some(blue));
        assert_eq!(material_of(&app, DVec3::new(15., 5., 0.)), None);

        // Painting the strip: plain, then shift-replace repaints both strip faces.
        app.organization.material = Some(red);
        click_at(
            &mut app,
            &ctx,
            DVec3::new(5., 5., 0.),
            egui::Modifiers::NONE,
        );
        click_at(
            &mut app,
            &ctx,
            DVec3::new(15., 5., 0.),
            egui::Modifiers::NONE,
        );
        assert_eq!(material_of(&app, DVec3::new(5., 5., 0.)), Some(red));
        assert_eq!(material_of(&app, DVec3::new(15., 5., 0.)), Some(red));
        app.organization.material = Some(blue);
        click_at(&mut app, &ctx, DVec3::new(5., 5., 0.), shift);
        assert_eq!(material_of(&app, DVec3::new(5., 5., 0.)), Some(blue));
        assert_eq!(material_of(&app, DVec3::new(15., 5., 0.)), Some(blue));
        assert_eq!(material_of(&app, DVec3::new(35., 35., 0.)), None);
        // One undo entry per paint click: 5 draws + 6 paints.
        assert_eq!(app.history.undo_depth(), 11);
        // Undoing the shift replace returns both strip faces to red; the
        // untouched square stays unpainted.
        app.history.undo().unwrap();
        assert_eq!(material_of(&app, DVec3::new(5., 5., 0.)), Some(red));
        assert_eq!(material_of(&app, DVec3::new(15., 5., 0.)), Some(red));
        assert_eq!(material_of(&app, DVec3::new(35., 35., 0.)), None);
    }

    #[test]
    fn paint_group_instance_and_ctrl_shift_object_scope() {
        let (mut app, ctx) = setup_paint();
        let materials: Vec<MaterialId> = app.history.materials().keys().copied().collect();
        let (red, blue) = (materials[0], materials[1]);
        // Wrap the two-strip in a component; keep the separate square loose.
        let vertices = app.model().vertices().keys().copied().collect();
        app.org_command(Org::Create {
            kind: cindral_core::DefinitionKind::Component,
            name: "Part".into(),
            vertices,
        });
        let path = vec![*app
            .history
            .instances(cindral_core::DefinitionId(0))
            .unwrap()
            .keys()
            .next()
            .unwrap()];
        app.selection = cindral_tools::Selection::default();
        app.organization.selected = None;
        app.organization.multiple.clear();

        // Plain click on the container paints the instance, not its faces.
        app.organization.material = Some(red);
        click_at(
            &mut app,
            &ctx,
            DVec3::new(5., 5., 0.),
            egui::Modifiers::NONE,
        );
        assert_eq!(app.history.instance(&path).unwrap().material, Some(red));
        assert!(app.model().faces().values().all(|f| f.material.is_none()));

        // Ctrl+Shift replaces matching faces inside the clicked object only:
        // the unpainted (red-inheriting) faces become explicitly blue.
        app.organization.material = Some(blue);
        let both = egui::Modifiers {
            ctrl: true,
            shift: true,
            ..Default::default()
        };
        click_at(&mut app, &ctx, DVec3::new(5., 5., 0.), both);
        assert_eq!(app.history.instance(&path).unwrap().material, Some(red));
        assert!(app
            .history
            .definition_model(app.history.instance(&path).unwrap().definition)
            .unwrap()
            .faces()
            .values()
            .all(|f| f.material == Some(blue)));
        // The loose square outside the object was untouched.
        assert_eq!(material_of_root(&app, DVec3::new(35., 35., 0.)), None);
    }
    fn material_of_root(app: &DrafterApp, p: DVec3) -> Option<MaterialId> {
        // After grouping, the root holds only the separate square.
        app.model()
            .faces()
            .values()
            .next()
            .unwrap()
            .material
            .filter(|_| {
                let face = app.model().faces().values().next().unwrap();
                let mut min = DVec3::splat(f64::INFINITY);
                let mut max = DVec3::splat(f64::NEG_INFINITY);
                for &v in &face.outer {
                    let q = app.model().vertices()[&v].position;
                    min = min.min(q);
                    max = max.max(q);
                }
                min.cmple(p).all() && max.cmpge(p).all()
            })
    }

    #[test]
    fn panel_virtualization_search_sort_and_purge() {
        let mut app = DrafterApp::default();
        square(&mut app, 0., 0.);
        // Load 5,000 materials directly through schema values (no history).
        let mut data = app.history.to_data();
        for i in 0..5000 {
            data.materials.insert(
                MaterialId(1_000_000 + i),
                Material {
                    name: format!("M {i:04}"),
                    color: [(i % 255) as u8, 60, 120],
                    opacity: 1.,
                    texture: None,
                    texture_mapping: None,
                },
            );
        }
        let used = MaterialId(1_000_000);
        data.next_id = data.next_id.max(1_000_000 + 5000);
        app.set_document_data(data).unwrap();
        let face = *app.model().faces().keys().next().unwrap();
        app.org_command(Org::PaintFaces {
            faces: vec![face],
            material: Some(used),
        });

        let ctx = egui::Context::default();
        // Render the dedicated Materials inspector so its list builds visible rows.
        app.inspector_tab = crate::workspace_ui::InspectorTab::Materials;
        let inspector = |app: &mut DrafterApp, ctx: &egui::Context| {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1200., 800.),
                )),
                ..Default::default()
            };
            let _ = ctx.run(input, |ctx| app.model_inspector(ctx));
        };
        for _ in 0..2 {
            inspector(&mut app, &ctx);
        }
        assert!(
            app.materials.rows_built > 0 && app.materials.rows_built < 100,
            "built {} rows for 5001 materials",
            app.materials.rows_built
        );
        // Search narrows the list; usage sorts the painted material first.
        app.materials.search = "M 0499".into();
        inspector(&mut app, &ctx);
        assert!(app.materials.rows_built <= 4);
        app.materials.search.clear();
        app.materials.sort = MaterialSort::Usage;
        inspector(&mut app, &ctx);
        // Usage sorts most-used first (the painted material leads) with the
        // rest grouped by descending usage; name is the tiebreaker.
        let entries = app.materials.visible_entries(&app.history);
        assert_eq!(entries[0], (used, 1));
        assert!(entries[1..].windows(2).all(|w| w[0].1 >= w[1].1));
        assert!(entries[1..].windows(2).all(|w| w[0].1 != w[1].1
            || app.history.materials()[&w[0].0].name <= app.history.materials()[&w[1].0].name));
        // Name sort orders alphabetically.
        app.materials.sort = MaterialSort::Name;
        let entries = app.materials.visible_entries(&app.history);
        let names: Vec<&str> = entries
            .iter()
            .map(|(id, _)| app.history.materials()[id].name.as_str())
            .collect();
        let mut sorted = names.clone();
        sorted.sort_unstable();
        assert_eq!(names, sorted);
        let unused = app.materials.unused_materials(&app.history);
        assert_eq!(unused.len(), 4999);
        assert!(!unused.contains(&used));
        // Purge is one undo step and keeps the used material.
        let depth = app.history.undo_depth();
        app.org_command(Org::DeleteMaterials(unused));
        assert_eq!(app.history.undo_depth(), depth + 1);
        assert_eq!(app.history.materials().len(), 1);
        assert!(app
            .model()
            .faces()
            .values()
            .all(|f| f.material == Some(used)));
        app.history.undo().unwrap();
        assert_eq!(app.history.materials().len(), 5000);
    }

    #[test]
    fn live_slider_edit_previews_and_commits_one_entry() {
        let (mut app, _ctx) = setup_paint();
        let id = app.organization.material.unwrap();
        let depth = app.history.undo_depth();
        for opacity in [0.9, 0.7, 0.4] {
            let edited = Material {
                opacity,
                ..app.history.materials()[&id].clone()
            };
            app.history.material_preview(id, edited).unwrap();
        }
        assert_eq!(app.history.undo_depth(), depth);
        assert_eq!(app.history.materials()[&id].opacity, 0.4);
        let original = Material {
            opacity: 1.,
            ..app.history.materials()[&id].clone()
        };
        let edited = app.history.materials()[&id].clone();
        app.history
            .finish_material_edit(id, original, edited)
            .unwrap();
        assert_eq!(app.history.undo_depth(), depth + 1);
        app.history.undo().unwrap();
        assert_eq!(app.history.materials()[&id].opacity, 1.);
    }

    #[test]
    fn recent_colors_dedupe_cap_and_corrupt_preferences_fallback() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("preferences.json");
        let mut options = crate::png_export::Options::default();
        for i in 0..15 {
            options.push_recent_color([i as u8, 0, 0]);
        }
        assert_eq!(options.recent_colors.len(), 12);
        assert_eq!(options.recent_colors[0], [14, 0, 0]);
        options.push_recent_color([3, 0, 0]);
        assert_eq!(options.recent_colors.len(), 12);
        assert_eq!(options.recent_colors[0], [3, 0, 0]);
        assert_eq!(
            options
                .recent_colors
                .iter()
                .filter(|c| **c == [3, 0, 0])
                .count(),
            1
        );
        options.save(&path).unwrap();
        assert_eq!(
            crate::png_export::Options::load(&path).recent_colors,
            options.recent_colors
        );
        std::fs::write(&path, "{\"recent_colors\": \"interrupted").unwrap();
        let recovered = crate::png_export::Options::load(&path);
        assert!(recovered.recent_colors.is_empty());
        assert_eq!(recovered.theme, crate::theme::Preference::System);
    }

    #[test]
    fn builtin_pattern_materials_regenerate_after_load_without_embedding() {
        let mut app = DrafterApp::default();
        square(&mut app, 0., 0.);
        app.create_pattern_material(0);
        let key = format!(
            "builtin:{}",
            crate::builtin_materials::BUILTIN_PATTERNS[0].name
        );
        assert!(app.files.textures.contains_key(&key));
        assert!(app
            .history
            .materials()
            .values()
            .any(|m| m.texture.as_deref() == Some(key.as_str())));
        // Saving excludes builtin slots.
        let bundle = app.file_values();
        assert!(!bundle.textures.contains_key(&key));
        // Loading regenerates the runtime asset without an error.
        let mut restored = DrafterApp::default();
        restored.apply_file(bundle, None).unwrap();
        assert!(restored.files.textures.contains_key(&key));
        assert!(restored.error.is_none());
    }

    #[test]
    fn mapping_size_typing_previews_and_commits_one_entry() {
        let mut app = DrafterApp::default();
        square(&mut app, 0., 0.);
        app.create_pattern_material(0);
        let id = app.organization.material.unwrap();
        let depth = app.history.undo_depth();
        let width = |app: &DrafterApp| {
            app.history.materials()[&id]
                .texture_mapping
                .unwrap_or_default()
                .width_m
        };
        // Valid keystrokes preview without recording history; incomplete or
        // invalid ones never reach the document.
        for text in ["0", "0.", "0.6"] {
            assert_eq!(
                app.mapping_size_preview(id, text, "1.00", true),
                text == "0.6"
            );
            assert_eq!(app.history.undo_depth(), depth);
        }
        assert!((width(&app) - 0.6).abs() < 1e-9);
        assert!(!app.mapping_size_preview(id, "0.6.", "1.00", true));
        assert!(!app.mapping_size_preview(id, "-2", "1.00", true));
        assert!(!app.mapping_size_preview(id, "abc", "1.00", true));
        assert!((width(&app) - 0.6).abs() < 1e-9);

        // Enter (or focus loss) commits exactly one entry for the whole edit.
        app.mapping_size_commit(id, "0.6", "1.00");
        assert_eq!(app.history.undo_depth(), depth + 1);
        assert!((width(&app) - 0.6).abs() < 1e-9);
        app.history.undo().unwrap();
        assert_eq!(app.history.materials()[&id].texture_mapping, None);

        // Escape reverts a live preview without an entry.
        assert!(app.mapping_size_preview(id, "2", "1.00", true));
        assert!((width(&app) - 2.).abs() < 1e-9);
        app.mapping_session_revert(id);
        assert_eq!(app.history.undo_depth(), depth);
        assert_eq!(app.history.materials()[&id].texture_mapping, None);
    }

    #[test]
    fn select_users_covers_component_interiors_and_instances() {
        let mut app = DrafterApp::default();
        square(&mut app, 0., 0.);
        let before = app.model().vertices().len();
        square(&mut app, 30., 30.);
        let part_vertices: Vec<cindral_core::VertexId> = app
            .model()
            .vertices()
            .keys()
            .skip(before)
            .copied()
            .collect();
        app.org_command(Org::Create {
            kind: cindral_core::DefinitionKind::Component,
            name: "Part".into(),
            vertices: part_vertices,
        });
        let path = vec![*app
            .history
            .instances(cindral_core::DefinitionId(0))
            .unwrap()
            .keys()
            .next()
            .unwrap()];
        app.org_command(Org::CreateMaterial(Material {
            name: "Red".into(),
            color: [200, 40, 40],
            opacity: 1.,
            texture: None,
            texture_mapping: None,
        }));
        let id = app.history.materials().keys().next_back().copied().unwrap();

        // The material is used only inside the component definition.
        app.history.enter(path.clone()).unwrap();
        let faces: Vec<FaceId> = app.history.model().faces().keys().copied().collect();
        app.org_command(Org::PaintFaces {
            faces,
            material: Some(id),
        });
        app.history.exit().unwrap();
        app.materials.refresh_counts(&app.history);
        assert_eq!(app.materials.usage(Some(id)), 1);
        assert!(app.selection.faces.is_empty());
        app.select_material_users(id);
        assert!(app.selection.faces.is_empty());
        assert_eq!(app.organization.selected.as_ref(), Some(&path));

        // A material-carrying instance counts and is selected as well.
        app.org_command(Org::PaintInstance {
            path: path.clone(),
            material: Some(id),
        });
        app.materials.refresh_counts(&app.history);
        assert_eq!(app.materials.usage(Some(id)), 2);
        app.select_material_users(id);
        assert_eq!(app.organization.selected.as_ref(), Some(&path));
    }

    #[test]
    fn duplicate_material_copies_appearance_with_unique_name_and_one_undo() {
        let (mut app, _ctx) = setup_paint();
        let red = app.history.materials().keys().next().copied().unwrap();
        let original = app.history.materials()[&red].clone();
        let depth = app.history.undo_depth();
        app.duplicate_material(red);
        assert_eq!(app.history.undo_depth(), depth + 1);
        let copy_id = app.history.materials().keys().next_back().copied().unwrap();
        let copy = app.history.materials()[&copy_id].clone();
        assert_eq!(copy.name, format!("{} 2", original.name));
        assert_eq!(copy.color, original.color);
        assert_eq!(copy.texture, original.texture);
        assert_eq!(app.organization.material, Some(copy_id));
        // Repeated duplicates keep names unique.
        app.duplicate_material(red);
        let third = app.history.materials().keys().next_back().copied().unwrap();
        assert_eq!(
            app.history.materials()[&third].name,
            format!("{} 3", original.name)
        );
        app.history.undo().unwrap();
        assert!(!app.history.materials().contains_key(&third));
        assert!(app.history.materials().contains_key(&copy_id));
    }

    #[test]
    fn paint_bucket_escape_resets_without_leaving_the_tool() {
        let (mut app, ctx) = setup_paint();
        let depth = app.history.undo_depth();
        let model = app.model().clone();
        let p = position(&app, DVec3::new(5., 5., 0.));
        send(
            &mut app,
            &ctx,
            vec![egui::Event::PointerMoved(p)],
            egui::Modifiers::NONE,
        );
        send(
            &mut app,
            &ctx,
            vec![egui::Event::Key {
                key: eframe::egui::Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
            egui::Modifiers::NONE,
        );
        assert_eq!(
            app.modify.kind(),
            cindral_tools::ModifyKind::PaintBucket,
            "Escape must keep the Paint Bucket selected"
        );
        assert_eq!(app.history.undo_depth(), depth);
        assert_eq!(app.model(), &model);
        // The tool still paints afterwards.
        click_at(
            &mut app,
            &ctx,
            DVec3::new(5., 5., 0.),
            egui::Modifiers::NONE,
        );
        assert_eq!(
            material_of(&app, DVec3::new(5., 5., 0.)).is_some(),
            app.organization.material.is_some()
        );
    }
    fn ui_review_frame(
        app: &mut DrafterApp,
        ctx: &egui::Context,
        events: Vec<egui::Event>,
        mode: u8,
    ) -> egui::FullOutput {
        ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(360., 1000.),
                )),
                events,
                ..Default::default()
            },
            |ctx| {
                if mode == 2 {
                    app.material_quick_picker(ctx);
                } else {
                    egui::CentralPanel::default().show(ctx, |ui| match mode {
                        0 => app.builtin_section(ui),
                        1 => app.library_section(ui),
                        4 => app.material_browser(ui),
                        _ => {
                            app.materials.refresh_counts(&app.history);
                            app.material_modals(ui);
                        }
                    });
                }
            },
        )
    }

    fn image_center(output: &egui::FullOutput, id: egui::TextureId) -> egui::Pos2 {
        fn find(shape: &egui::Shape, id: egui::TextureId) -> Option<egui::Pos2> {
            match shape {
                egui::Shape::Rect(rect) if rect.fill_texture_id() == id => Some(rect.rect.center()),
                egui::Shape::Mesh(mesh) if mesh.texture_id == id && !mesh.vertices.is_empty() => {
                    let mut min = egui::pos2(f32::INFINITY, f32::INFINITY);
                    let mut max = egui::pos2(f32::NEG_INFINITY, f32::NEG_INFINITY);
                    for vertex in &mesh.vertices {
                        min.x = min.x.min(vertex.pos.x);
                        min.y = min.y.min(vertex.pos.y);
                        max.x = max.x.max(vertex.pos.x);
                        max.y = max.y.max(vertex.pos.y);
                    }
                    Some(egui::Rect::from_min_max(min, max).center())
                }
                egui::Shape::Vec(shapes) => shapes.iter().find_map(|s| find(s, id)),
                _ => None,
            }
        }
        output
            .shapes
            .iter()
            .find_map(|s| find(&s.shape, id))
            .unwrap_or_else(|| panic!("image {id:?} was not rendered"))
    }

    fn text_center(output: &egui::FullOutput, text: &str) -> egui::Pos2 {
        output
            .shapes
            .iter()
            .find_map(|shape| {
                if let egui::Shape::Text(label) = &shape.shape {
                    (label.galley.text() == text).then(|| label.pos + label.galley.size() * 0.5)
                } else {
                    None
                }
            })
            .unwrap_or_else(|| panic!("missing UI text: {text}"))
    }

    fn ui_review_click(
        app: &mut DrafterApp,
        ctx: &egui::Context,
        pos: egui::Pos2,
        mode: u8,
    ) -> egui::FullOutput {
        ui_review_frame(
            app,
            ctx,
            vec![
                egui::Event::PointerMoved(pos),
                button(pos, true, egui::Modifiers::NONE),
            ],
            mode,
        );
        ui_review_frame(
            app,
            ctx,
            vec![button(pos, false, egui::Modifiers::NONE)],
            mode,
        )
    }

    #[test]
    fn pattern_image_button_activates_and_previews_are_cached() {
        let mut app = DrafterApp::default();
        let ctx = egui::Context::default();
        crate::workspace_ui::configure_style(&ctx);
        ui_review_frame(&mut app, &ctx, vec![], 0);
        let first = app.materials.library_tiles["builtin-pattern-4"].id();
        let output = ui_review_frame(&mut app, &ctx, vec![], 0);
        assert_eq!(first, app.materials.library_tiles["builtin-pattern-4"].id());
        assert!(!output.textures_delta.set.iter().any(|(id, _)| *id == first));
        let pos = image_center(&output, first);
        let depth = app.history.undo_depth();
        ui_review_click(&mut app, &ctx, pos, 0);
        let active = app
            .organization
            .material
            .expect("pattern click chooses a paint material");
        assert_eq!(app.history.materials()[&active].name, "Grain Horizontal");
        assert!(app.history.materials()[&active].texture.is_some());
        assert_eq!(app.history.undo_depth(), depth + 1);
    }

    #[test]
    fn folder_images_are_clickable_and_only_visible_tiles_are_uploaded() {
        let temp = tempfile::tempdir().unwrap();
        for i in 0..128 {
            image::save_buffer(
                temp.path().join(format!("tile-{i:03}.png")),
                &[120, 70, 30, 255],
                1,
                1,
                image::ColorType::Rgba8,
            )
            .unwrap();
        }
        let mut app = DrafterApp::default();
        app.materials.library.add_folder(temp.path().to_owned());
        app.materials.library.refresh(temp.path());
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while app.materials.library.scan(temp.path()).is_none() {
            app.materials.library.poll();
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let ctx = egui::Context::default();
        crate::workspace_ui::configure_style(&ctx);
        ui_review_frame(&mut app, &ctx, vec![], 1);
        let output = ui_review_frame(&mut app, &ctx, vec![], 1);
        assert!(
            app.materials.library_tiles.len() < 32,
            "only a few visible rows should allocate GPU thumbnails"
        );
        let key = temp.path().join("tile-000.png").display().to_string();
        let pos = image_center(&output, app.materials.library_tiles[&key].id());
        assert!(
            text_center(&output, "tile-000.png").y > pos.y + 28.,
            "folder names belong below their thumbnails"
        );
        ui_review_click(&mut app, &ctx, pos, 1);
        let material = &app.history.materials()[&app
            .organization
            .material
            .expect("folder image click chooses the imported material")];
        assert_eq!(material.name, "tile-000");
        assert!(material.texture.is_some());
        assert_eq!(app.files.textures.len(), 1);
    }

    #[test]
    fn preview_tracks_selection_modifiers_and_the_faces_actually_painted() {
        let (mut app, _) = setup_paint();
        let faces: Vec<_> = app.model().faces().keys().copied().collect();
        let hit = InstanceHit {
            path: vec![],
            definition: cindral_core::DefinitionId(0),
            hit: cindral_tools::Hit::Face(faces[0]),
            local_position: DVec3::ZERO,
            world_position: DVec3::ZERO,
        };
        app.selection.faces = faces[..2].iter().copied().collect();
        app.paint_hover_update(Some(&hit), &egui::Modifiers::NONE);
        assert_eq!(app.paint_hover.faces.len(), 2);
        app.selection.faces.insert(faces[2]);
        app.paint_hover_update(Some(&hit), &egui::Modifiers::NONE);
        assert_eq!(app.paint_hover.faces.len(), 3);
        let both = egui::Modifiers {
            ctrl: true,
            shift: true,
            ..Default::default()
        };
        app.paint_hover_update(Some(&hit), &both);
        let preview = app.paint_hover.faces.clone();
        assert_eq!(preview.len(), 3);
        let paint = app.organization.material;
        app.paint_bucket_click(Some(&hit), None, &both);
        let painted: Vec<_> = app
            .model()
            .faces()
            .iter()
            .filter(|(_, f)| f.material == paint)
            .map(|(id, _)| *id)
            .collect();
        assert_eq!(preview, painted);
        let before = app.history.to_data();
        let alt = egui::Modifiers {
            alt: true,
            ..Default::default()
        };
        app.paint_hover_update(Some(&hit), &alt);
        assert!(app.paint_hover.faces.is_empty());
        app.paint_bucket_click(Some(&hit), None, &alt);
        assert_eq!(before, app.history.to_data());
    }

    #[test]
    fn nested_sampling_prefers_face_then_inherited_material_without_changing_geometry() {
        let (mut app, _) = setup_paint();
        let ids: Vec<_> = app.history.materials().keys().copied().collect();
        let (red, blue) = (ids[0], ids[1]);
        app.org_command(Org::Create {
            kind: cindral_core::DefinitionKind::Component,
            name: "Part".into(),
            vertices: app.model().vertices().keys().copied().collect(),
        });
        let path = vec![*app
            .history
            .instances(cindral_core::DefinitionId(0))
            .unwrap()
            .keys()
            .next()
            .unwrap()];
        let definition = app.history.instance(&path).unwrap().definition;
        app.org_command(Org::PaintInstance {
            path: path.clone(),
            material: Some(red),
        });
        app.history.enter(path.clone()).unwrap();
        let faces: Vec<_> = app.model().faces().keys().copied().collect();
        app.org_command(Org::PaintFaces {
            faces: vec![faces[0]],
            material: Some(blue),
        });
        app.selection.faces.insert(faces[1]);
        app.activate_material(Some(blue));
        assert_eq!(
            app.inspected_material(),
            InspectedMaterial::Single(Some(red), true)
        );
        assert_eq!(
            app.organization.material,
            Some(blue),
            "inspection leaves paint unchanged"
        );
        app.selection.faces.insert(faces[0]);
        assert_eq!(app.inspected_material(), InspectedMaterial::Mixed);
        app.history.exit().unwrap();
        app.selection.clear();
        let alt = egui::Modifiers {
            alt: true,
            ..Default::default()
        };
        let mut hit = InstanceHit {
            path: path.clone(),
            definition,
            hit: cindral_tools::Hit::Face(faces[0]),
            local_position: DVec3::ZERO,
            world_position: DVec3::ZERO,
        };
        let before = app.history.to_data();
        app.paint_bucket_click(Some(&hit), Some(path.clone()), &alt);
        assert_eq!(app.organization.material, Some(blue));
        hit.hit = cindral_tools::Hit::Face(faces[1]);
        app.paint_bucket_click(Some(&hit), Some(path), &alt);
        assert_eq!(app.organization.material, Some(red));
        assert_eq!(app.history.to_data(), before);
    }

    #[test]
    fn quick_picker_keyboard_changes_only_paint_and_escape_restores_focus() {
        let (mut app, _) = setup_paint();
        let before = app.history.to_data();
        let depth = app.history.undo_depth();
        let ctx = egui::Context::default();
        app.open_material_picker();
        ui_review_frame(&mut app, &ctx, vec![], 2);
        ui_review_frame(&mut app, &ctx, vec![], 2);
        ui_review_frame(&mut app, &ctx, vec![egui::Event::Text("Red".into())], 2);
        let key = |key| egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        };
        ui_review_frame(&mut app, &ctx, vec![key(egui::Key::Enter)], 2);
        assert!(!app.materials.picker_open);
        assert_eq!(app.organization.material_name, "Red");
        assert_eq!(app.history.undo_depth(), depth);
        assert_eq!(app.history.to_data(), before);
        app.open_material_picker();
        ui_review_frame(&mut app, &ctx, vec![], 2);
        ui_review_frame(&mut app, &ctx, vec![key(egui::Key::ArrowDown)], 2);
        assert_eq!(app.materials.picker_focus, 1);
        ui_review_frame(&mut app, &ctx, vec![key(egui::Key::Escape)], 2);
        assert!(!app.materials.picker_open);
        assert!(ctx.memory(|m| m.focused()).is_none());
        assert_eq!(app.organization.material_name, "Red");
        app.set_document_data(cindral_core::DocumentHistory::new().to_data())
            .unwrap();
        assert!(app.materials.recent_materials.is_empty());
    }

    #[test]
    fn used_delete_requires_confirmation_and_replace_waits_for_apply() {
        let (mut app, _) = setup_paint();
        let ids: Vec<_> = app.history.materials().keys().copied().collect();
        let (red, blue) = (ids[0], ids[1]);
        let face = *app.model().faces().keys().next().unwrap();
        app.org_command(Org::PaintFaces {
            faces: vec![face],
            material: Some(red),
        });
        let ctx = egui::Context::default();
        app.materials.rename = Some((red, "Red".into()));
        ui_review_frame(&mut app, &ctx, vec![], 3);
        let out = ui_review_frame(&mut app, &ctx, vec![], 3);
        let pos = text_center(&out, "Delete material…");
        ui_review_click(&mut app, &ctx, pos, 3);
        assert_eq!(app.materials.delete, Some(red));
        assert!(app.history.materials().contains_key(&red));
        app.materials.delete = None;
        app.materials.replace = Some(red);
        ui_review_frame(&mut app, &ctx, vec![], 3);
        let out = ui_review_frame(&mut app, &ctx, vec![], 3);
        let pos = text_center(&out, "Blue");
        ui_review_click(&mut app, &ctx, pos, 3);
        assert_eq!(app.materials.replace_target, Some(blue));
        assert_eq!(app.model().faces()[&face].material, Some(red));
        let depth = app.history.undo_depth();
        let out = ui_review_frame(&mut app, &ctx, vec![], 3);
        let pos = text_center(&out, "Replace all uses");
        ui_review_click(&mut app, &ctx, pos, 3);
        assert_eq!(app.model().faces()[&face].material, Some(blue));
        assert_eq!(app.history.undo_depth(), depth + 1);
        assert!(app.materials.notice.is_some());
        app.history_step(false);
        assert_eq!(app.model().faces()[&face].material, Some(red));
    }

    #[test]
    fn prepared_texture_thumbnail_survives_scene_upload_without_redecoding() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("preview.png");
        image::save_buffer(&path, &[120, 70, 30, 255], 1, 1, image::ColorType::Rgba8).unwrap();
        let (key, asset) = cindral_io::load_texture(&path).unwrap();
        let mut app = DrafterApp::default();
        app.files.textures.insert(key.clone(), asset);
        app.org_command(Org::CreateMaterial(Material {
            name: "Preview".into(),
            color: [255; 3],
            opacity: 1.,
            texture: Some(key.clone()),
            texture_mapping: None,
        }));
        let id = *app.history.materials().keys().next().unwrap();
        app.sync_file_textures().unwrap();
        let ctx = egui::Context::default();
        let tile = app
            .material_thumbnail(&ctx, Some(id))
            .expect("scene upload retains a small mip for the paint chip");
        assert_eq!(tile.size(), [64, 64]);
        assert_eq!(
            app.material_thumbnail(&ctx, Some(id)).unwrap().id(),
            tile.id()
        );
    }

    #[test]
    fn quick_picker_arrows_reach_virtualized_rows_beyond_the_first_page() {
        let mut app = DrafterApp::default();
        let mut data = app.history.to_data();
        for i in 0..64 {
            data.materials.insert(
                MaterialId(1000 + i),
                Material {
                    name: format!("Material {i:03}"),
                    color: [100, 100, 100],
                    opacity: 1.,
                    texture: None,
                    texture_mapping: None,
                },
            );
        }
        data.next_id = 2000;
        app.set_document_data(data).unwrap();
        let expected = app.materials.visible_entries(&app.history)[29].0;
        let ctx = egui::Context::default();
        app.open_material_picker();
        ui_review_frame(&mut app, &ctx, vec![], 2);
        ui_review_frame(&mut app, &ctx, vec![], 2);
        let key = |key| egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        };
        for _ in 0..30 {
            ui_review_frame(&mut app, &ctx, vec![key(egui::Key::ArrowDown)], 2);
        }
        let out = ui_review_frame(&mut app, &ctx, vec![], 2);
        text_center(&out, &app.history.materials()[&expected].name);
        ui_review_frame(&mut app, &ctx, vec![key(egui::Key::Enter)], 2);
        assert_eq!(app.organization.material, Some(expected));
        assert_eq!(app.history.undo_depth(), 0);
    }

    #[test]
    fn materials_views_keep_inspector_width_in_both_sizes_and_themes() {
        for (width, height) in [(1366., 768.), (1920., 1080.)] {
            for theme in [egui::Theme::Light, egui::Theme::Dark] {
                let (mut app, _) = setup_paint();
                let ctx = egui::Context::default();
                crate::workspace_ui::configure_style(&ctx);
                ctx.set_theme(theme);
                app.materials.library_restored = true;
                let id = app.organization.material.unwrap();
                let mut material = app.history.materials()[&id].clone();
                material.texture = Some("review-texture".into());
                app.org_command(Org::UpdateMaterial { id, material });
                for view in [
                    MaterialsView::Browse,
                    MaterialsView::Edit,
                    MaterialsView::Library,
                ] {
                    app.materials.view = view;
                    for _ in 0..3 {
                        let output=ctx.run(egui::RawInput{screen_rect:Some(egui::Rect::from_min_size(egui::Pos2::ZERO,egui::vec2(width,height))),..Default::default()},|ctx| {
                            app.workspace_header(ctx);app.workspace_status(ctx);app.tool_palette(ctx);
                            let before=ctx.available_rect().width();app.model_inspector(ctx);
                            let inspector=before-ctx.available_rect().width();
                            assert!(inspector<=321.,"{view:?} grew to {inspector}px at {width}x{height} in {theme:?}");
                        });
                        let bounds = |name: &str| {
                            output
                                .shapes
                                .iter()
                                .find_map(|shape| {
                                    if let egui::Shape::Text(text) = &shape.shape {
                                        (text.galley.text() == name).then(|| {
                                            egui::Rect::from_min_size(text.pos, text.galley.size())
                                        })
                                    } else {
                                        None
                                    }
                                })
                                .unwrap_or_else(|| panic!("missing inspector label {name}"))
                        };
                        let close = bounds("×");
                        let recovery = if app.recovery_count() == 0 {
                            "Recovery".into()
                        } else {
                            format!("Recovery ({})", app.recovery_count())
                        };
                        for name in ["Model", "Appearance", "Materials", recovery.as_str()] {
                            assert!(!close.intersects(bounds(name)), "close overlaps {name}");
                        }
                    }
                }
            }
        }
    }
    #[test]
    fn browse_list_retains_keyboard_focus_after_a_pointer_pick() {
        let (mut app, _) = setup_paint();
        let before = app.history.to_data();
        let ctx = egui::Context::default();
        crate::workspace_ui::configure_style(&ctx);
        ui_review_frame(&mut app, &ctx, vec![], 4);
        let output = ui_review_frame(&mut app, &ctx, vec![], 4);
        let pos = text_center(&output, "Red");
        ui_review_click(&mut app, &ctx, pos, 4);
        assert_eq!(app.organization.material_name, "Red");
        ui_review_frame(&mut app, &ctx, vec![], 4);
        ui_review_frame(
            &mut app,
            &ctx,
            vec![egui::Event::Key {
                key: egui::Key::ArrowUp,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
            4,
        );
        assert_eq!(app.organization.material_name, "Blue");
        assert_eq!(app.history.to_data(), before);
    }
}
