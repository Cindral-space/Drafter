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

/// Hover preview for the Paint Bucket: what a click would paint.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct PaintHover {
    /// Face ids in the active definition that would be painted.
    pub faces: Vec<FaceId>,
    /// Container that would be painted as a whole.
    pub instance: Option<InstancePath>,
    /// Hover target plus modifier state; equality gates recomputation.
    key: (Option<FaceId>, Option<InstancePath>, u8),
}

impl Default for PaintHover {
    fn default() -> Self {
        Self {
            faces: Vec::new(),
            instance: None,
            key: (None, None, u8::MAX),
        }
    }
}

/// Live-editing state for the Materials panel.
pub(crate) struct MaterialsUi {
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

const ROW_HEIGHT: f32 = 22.;

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
        self.materials.refresh_counts(&self.history);
        self.materials.library.poll();
        self.materials.rows_built = 0;

        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.materials.search)
                    .hint_text("Search materials")
                    .desired_width(120.),
            );
            egui::ComboBox::from_id_salt("material_sort")
                .selected_text(match self.materials.sort {
                    MaterialSort::Name => "Sort: name",
                    MaterialSort::Usage => "Sort: usage",
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.materials.sort, MaterialSort::Name, "Name");
                    ui.selectable_value(&mut self.materials.sort, MaterialSort::Usage, "Usage");
                });
        });

        let entries = self.materials.visible_entries(&self.history);
        let total = entries.len() + 1; // + Default row
        let selected = self.organization.material;
        egui::ScrollArea::vertical()
            .max_height(ui.available_height() * 0.4)
            .auto_shrink([false, false])
            .show_rows(ui, ROW_HEIGHT, total, |ui, rows| {
                for row in rows {
                    self.materials.rows_built += 1;
                    if row == 0 {
                        self.material_list_row(
                            ui,
                            None,
                            "Default",
                            self.materials.usage(None),
                            selected.is_none(),
                        );
                    } else {
                        let (id, usage) = entries[row - 1];
                        let name = self
                            .history
                            .materials()
                            .get(&id)
                            .map_or(String::new(), |m| m.name.clone());
                        let textured = self
                            .history
                            .materials()
                            .get(&id)
                            .and_then(|m| m.texture.clone());
                        self.material_list_row(ui, Some(id), &name, usage, selected == Some(id));
                        let _ = textured;
                    }
                }
            });

        ui.separator();
        self.material_editor(ui);
        ui.separator();
        self.material_actions(ui);
        self.material_modals(ui);
        ui.separator();
        self.builtin_section(ui);
        self.library_section(ui);
    }

    fn material_list_row(
        &mut self,
        ui: &mut egui::Ui,
        id: Option<MaterialId>,
        name: &str,
        usage: usize,
        is_selected: bool,
    ) {
        let material = id.and_then(|id| self.history.materials().get(&id));
        let color = material.map_or(Color32::from_rgb(148, 159, 172), |m| {
            Color32::from_rgb(m.color[0], m.color[1], m.color[2])
        });
        ui.horizontal(|ui| {
            let (rect, response) =
                ui.allocate_exact_size(egui::vec2(14., 14.), egui::Sense::click());
            ui.painter().rect_filled(rect, 2., color);
            let label = ui.selectable_label(is_selected, name);
            let clicked = response.clicked() || label.clicked();
            if clicked {
                self.organization.material = id;
                self.organization.material_name = name.to_owned();
                if let Some(m) = material {
                    self.organization.color = m.color;
                    self.organization.opacity = m.opacity;
                    self.organization.texture = m.texture.clone().unwrap_or_default();
                } else {
                    self.organization.texture.clear();
                }
                self.materials.eyedropper = false;
            }
            ui.label(RichText::new(usage.to_string()).small().weak());
            // Opens the rename modal; replace/delete/duplicate live there too.
            let opened = ui
                .small_button("⋯")
                .on_hover_text("Rename material")
                .clicked();
            if opened {
                if let Some(id) = id {
                    self.materials.rename = Some((id, name.to_owned()));
                }
            }
        });
    }

    fn material_editor(&mut self, ui: &mut egui::Ui) {
        let Some(id) = self.organization.material else {
            ui.label(
                RichText::new("Default material — painting with it clears materials")
                    .weak()
                    .small(),
            );
            return;
        };
        let Some(material) = self.history.materials().get(&id).cloned() else {
            return;
        };

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
            let hex = ui.add(
                egui::TextEdit::singleline(&mut self.materials.hex)
                    .hint_text("#RRGGBB")
                    .desired_width(84.)
                    .font(egui::TextStyle::Monospace),
            );
            if hex.changed() {
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
                    }
                    None => {
                        self.error =
                            Some(format!("\"{}\" is not a #RRGGBB color", self.materials.hex))
                    }
                }
            }
        });

        // Texture slot: manual entry plus the bounded file picker.
        ui.horizontal(|ui| {
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
                    if let Err(e) = self.sync_file_textures() {
                        self.error = Some(e.to_string());
                    }
                }
            }
            if ui.button("Load PNG/JPEG…").clicked() {
                self.load_material_texture();
            }
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
        ui.label(RichText::new("Texture mapping").small().weak());
        let mapping = material.texture_mapping.unwrap_or_default();
        let lock_aspect = self.materials.lock_aspect;

        let mut buffers = self.materials.size_buffers.take().unwrap();
        let size = ui.add(
            egui::TextEdit::singleline(&mut buffers.1)
                .desired_width(64.)
                .font(egui::TextStyle::Monospace),
        );
        ui.label("×");
        let height = ui.add(
            egui::TextEdit::singleline(&mut buffers.2)
                .desired_width(64.)
                .font(egui::TextStyle::Monospace),
        );
        let lock = ui
            .selectable_label(lock_aspect, "🔗")
            .on_hover_text("Lock aspect ratio: editing one side scales the other");
        if lock.clicked() {
            self.materials.lock_aspect = !lock_aspect;
        }
        if size.changed() || height.changed() {
            self.mapping_size_preview(
                id,
                &buffers.1,
                &buffers.2,
                size.changed() && !height.changed(),
            );
        }
        let focused = size.has_focus() || height.has_focus();
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) && focused {
            self.mapping_session_revert(id);
            self.materials.size_buffers = None;
        } else if (ui.input(|i| i.key_pressed(egui::Key::Enter)) && focused)
            || size.lost_focus()
            || height.lost_focus()
        {
            self.mapping_size_commit(id, &buffers.1, &buffers.2);
            self.materials.size_buffers = None;
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
            ui.label(RichText::new("Recent").small().weak());
            for color in recents {
                let (rect, response) =
                    ui.allocate_exact_size(egui::vec2(16., 16.), egui::Sense::click());
                ui.painter()
                    .rect_filled(rect, 2., Color32::from_rgb(color[0], color[1], color[2]));
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

    fn material_actions(&mut self, ui: &mut egui::Ui) {
        let unused = self.materials.unused_materials(&self.history);
        ui.horizontal(|ui| {
            let label = if unused.is_empty() {
                "Purge unused".to_owned()
            } else {
                format!("Purge unused ({})", unused.len())
            };
            if ui
                .add_enabled(!unused.is_empty(), egui::Button::new(label))
                .on_hover_text("Delete every material no face or container uses (one undo step)")
                .clicked()
            {
                self.org_command(Org::DeleteMaterials(unused));
            }
            let selected = self.organization.material.is_some();
            if ui
                .add_enabled(selected, egui::Button::new("Eyedropper"))
                .on_hover_text("Pick a material by clicking a face (same as Alt+click)")
                .clicked()
            {
                self.materials.eyedropper = !self.materials.eyedropper;
            }
        });
    }

    fn material_modals(&mut self, ui: &mut egui::Ui) {
        let rename = self.materials.rename.clone();
        if let Some((material_id, mut name)) = rename {
            let mut close = false;
            egui::Modal::new(egui::Id::new("material-rename")).show(ui.ctx(), |ui| {
                ui.set_width(280.);
                ui.heading("Material actions");
                ui.label(RichText::new(&name).small().weak());
                ui.add_space(4.);
                let field = ui.text_edit_singleline(&mut name);
                if ui.button("Rename").clicked()
                    || (field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)))
                {
                    if !name.trim().is_empty() {
                        if let Some(current) = self.history.materials().get(&material_id).cloned() {
                            let edited = Material {
                                name: name.trim().to_owned(),
                                ..current
                            };
                            self.org_command(Org::UpdateMaterial {
                                id: material_id,
                                material: edited,
                            });
                        }
                    }
                    close = true;
                }
                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button("Duplicate").clicked() {
                        self.duplicate_material(material_id);
                        close = true;
                    }
                    if ui.button("Delete").clicked() {
                        self.org_command(Org::DeleteMaterial(material_id));
                        if self.organization.material == Some(material_id) {
                            self.organization.material = None;
                        }
                        close = true;
                    }
                });
                if ui.button("Select faces using it").clicked() {
                    self.select_material_users(material_id);
                    close = true;
                }
                if ui.button("Replace with another material…").clicked() {
                    self.materials.replace = Some(material_id);
                    close = true;
                }
                if ui.button("Close").clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    close = true;
                }
            });
            if close {
                self.materials.rename = None;
            }
        }
        if let Some(from) = self.materials.replace {
            let mut close = false;
            egui::Modal::new(egui::Id::new("material-replace")).show(ui.ctx(), |ui| {
                ui.set_width(280.);
                ui.heading("Replace material with…");
                let mut target = None;
                egui::ScrollArea::vertical()
                    .max_height(240.)
                    .show(ui, |ui| {
                        for (id, material) in self.history.materials() {
                            if *id == from {
                                continue;
                            }
                            if ui.selectable_label(false, &material.name).clicked() {
                                target = Some(*id);
                            }
                        }
                    });
                if let Some(to) = target {
                    self.org_command(Org::ReplaceMaterial { from, to: Some(to) });
                    close = true;
                }
                if ui.button("Cancel").clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    close = true;
                }
            });
            if close {
                self.materials.replace = None;
            }
        }
    }

    fn builtin_section(&mut self, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new("Builtin collection")
            .default_open(false)
            .show(ui, |ui| {
                let mut chosen: Option<(String, [u8; 3])> = None;
                let mut pattern = None;
                let mut group: Option<crate::builtin_materials::BuiltinGroup> = None;
                for color in crate::builtin_materials::BUILTIN_COLORS {
                    if group != Some(color.group) {
                        group = Some(color.group);
                        ui.label(RichText::new(color.group.to_string()).small().weak());
                    }
                    let (rect, response) =
                        ui.allocate_exact_size(egui::vec2(18., 18.), egui::Sense::click());
                    ui.painter().rect_filled(
                        rect,
                        2.,
                        Color32::from_rgb(color.srgb[0], color.srgb[1], color.srgb[2]),
                    );
                    if response.clicked() {
                        chosen = Some((color.name.to_owned(), color.srgb));
                    }
                }
                ui.add_space(4.);
                ui.label(RichText::new("patterns").small().weak());
                ui.horizontal_wrapped(|ui| {
                    for (index, p) in crate::builtin_materials::BUILTIN_PATTERNS
                        .iter()
                        .enumerate()
                    {
                        let label = format!("builtin-pattern-{index}");
                        let thumb = (p.generator)(64).rgba;
                        let tile = ui.ctx().load_texture(
                            &label,
                            egui::ColorImage::from_rgba_unmultiplied([64, 64], &thumb),
                            Default::default(),
                        );
                        let image = egui::Image::from_texture(&tile)
                            .fit_to_exact_size(egui::vec2(44., 44.));
                        if ui.add(image).on_hover_text(p.name).clicked() {
                            pattern = Some(index);
                        }
                    }
                });
                if let Some((name, srgb)) = chosen {
                    self.create_basic_material(&name, srgb);
                }
                if let Some(index) = pattern {
                    self.create_pattern_material(index);
                }
            });
    }

    fn library_section(&mut self, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new("Folder library")
            .default_open(false)
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
                let mut import: Option<std::path::PathBuf> = None;
                for folder in folders {
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(folder.file_name().map_or_else(
                                || folder.display().to_string(),
                                |n| n.to_string_lossy().into_owned(),
                            ))
                            .strong(),
                        );
                        if ui.small_button("Rescan").clicked() {
                            self.materials.library.refresh(&folder);
                        }
                        if ui
                            .small_button("×")
                            .on_hover_text("Remove this collection from the library")
                            .clicked()
                        {
                            self.materials.library.remove_folder(&folder);
                            self.png
                                .options
                                .library_folders
                                .retain(|f| f != &folder.display().to_string());
                            self.save_preferences();
                        }
                    });
                    let Some(scan) = self.materials.library.scan(&folder).cloned() else {
                        ui.label(RichText::new("Scanning…").small().weak());
                        continue;
                    };
                    if scan.skipped > 0 {
                        ui.label(
                            RichText::new(format!(
                                "{} file{} skipped",
                                scan.skipped,
                                if scan.skipped == 1 { "" } else { "s" }
                            ))
                            .small()
                            .weak(),
                        );
                    }
                    egui::ScrollArea::horizontal().show(ui, |ui| {
                        ui.horizontal(|ui| {
                            for image in &scan.images {
                                let label = image.path.display().to_string();
                                let handle = self
                                    .materials
                                    .library_tiles
                                    .entry(label.clone())
                                    .or_insert_with(|| {
                                        ui.ctx().load_texture(
                                            &label,
                                            image.thumb.clone(),
                                            Default::default(),
                                        )
                                    })
                                    .clone();
                                let widget = egui::Image::from_texture(&handle)
                                    .fit_to_exact_size(egui::vec2(56., 56.));
                                if ui.add(widget).on_hover_text(&image.name).clicked() {
                                    import = Some(image.path.clone());
                                }
                            }
                        });
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
                self.sync_file_textures()?;
            }
            Ok(())
        })();
        if let Err(e) = result {
            self.error = Some(e.to_string());
        }
    }

    fn select_last_material(&mut self) {
        if self.error.is_none() {
            self.organization.material = self.history.materials().keys().next_back().copied();
        }
    }

    /// Encode a procedural pattern into the runtime asset table once.
    fn ensure_builtin_pattern(&mut self, index: usize) -> Result<(), String> {
        let name = crate::builtin_materials::BUILTIN_PATTERNS[index].name;
        let key = format!("builtin:{name}");
        if self.files.textures.contains_key(&key) {
            return Ok(());
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
        if let Err(e) = self.sync_file_textures() {
            return Err(e.to_string());
        }
        Ok(())
    }

    /// Regenerate procedural pattern slots referenced by a loaded document.
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
            if let Err(e) = self.ensure_builtin_pattern(index) {
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
    pub(crate) fn paint_bucket_click(
        &mut self,
        hit: Option<&InstanceHit>,
        child: Option<InstancePath>,
        mods: &egui::Modifiers,
    ) -> bool {
        let alt = mods.alt;
        let ctrl = mods.ctrl || mods.command;
        let shift = mods.shift;

        // Sampling never mutates the model. Alt+click is conventional; the
        // eyedropper toggle exists because Linux window managers grab Alt.
        if alt || self.materials.eyedropper {
            if let Some(sample) = hit {
                let material = if sample.path.len() > self.history.context().len() {
                    self.history
                        .instance(&sample.path)
                        .and_then(|node| node.material)
                } else {
                    match &sample.hit {
                        cindral_tools::Hit::Face(face) => self
                            .history
                            .model()
                            .faces()
                            .get(face)
                            .and_then(|f| f.material),
                        _ => None,
                    }
                };
                self.organization.material = material;
                if let Some(id) = material {
                    if let Some(m) = self.history.materials().get(&id) {
                        self.organization.material_name = m.name.clone();
                        self.organization.color = m.color;
                        self.organization.opacity = m.opacity;
                        self.organization.texture = m.texture.clone().unwrap_or_default();
                    }
                } else {
                    self.organization.material_name = "Default".into();
                }
                self.materials.eyedropper = false;
            }
            return true;
        }

        let paint = self.organization.material;
        if let Some(path) = child {
            if ctrl && shift {
                // Replace same-material faces inside the clicked object only.
                if let Some(node) = self.history.instance(&path) {
                    let definition = node.definition;
                    let material = node.material;
                    if let Some(model) = self.history.definition_model(definition) {
                        let faces = faces_with_effective_material(model, material);
                        if !faces.is_empty() {
                            self.paint_in_context(&path, faces, paint);
                        }
                    }
                }
                return true;
            }
            if ctrl {
                self.org_command(Org::PaintInstance {
                    path,
                    material: paint,
                });
                return true;
            }
            if shift {
                // Replace every face sharing the clicked container's material.
                let material = self.history.instance(&path).and_then(|n| n.material);
                let faces = faces_with_effective_material(self.history.model(), material);
                if !faces.is_empty() {
                    self.org_command(Org::PaintFaces {
                        faces,
                        material: paint,
                    });
                }
                return true;
            }
            // Plain click: painting a selected object paints the selection.
            let in_selection = self.organization.selected.as_ref() == Some(&path)
                || self.organization.multiple.contains(&path);
            if in_selection {
                let paths: Vec<InstancePath> = self
                    .organization
                    .multiple
                    .iter()
                    .chain(self.organization.selected.iter())
                    .cloned()
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .collect();
                for p in &paths {
                    self.org_command(Org::PaintInstance {
                        path: p.clone(),
                        material: paint,
                    });
                }
                if !self.selection.faces.is_empty() {
                    let faces: Vec<FaceId> = self.selection.faces.iter().copied().collect();
                    self.org_command(Org::PaintFaces {
                        faces,
                        material: paint,
                    });
                }
                return true;
            }
            self.org_command(Org::PaintInstance {
                path,
                material: paint,
            });
            return true;
        }

        let Some(sample) = hit else { return false };
        if sample.path != self.history.context() {
            return false;
        }
        let cindral_tools::Hit::Face(face) = &sample.hit else {
            return false;
        };
        if !self.history.model().faces().contains_key(face) {
            return false;
        }
        let face = *face;
        let model = self.history.model();
        let current = model.faces().get(&face).and_then(|f| f.material);
        let faces = if ctrl && shift {
            // Within the active context this is the same scope as replace.
            faces_with_material(model, current)
        } else if ctrl {
            fill_adjacent_same_material(model, face)
        } else if shift {
            faces_with_material(model, current)
        } else if self.selection.faces.contains(&face) {
            // Clicking part of the selection paints the whole selection.
            let mut faces: Vec<FaceId> = self.selection.faces.iter().copied().collect();
            faces.push(face);
            faces.sort();
            faces.dedup();
            faces
        } else {
            vec![face]
        };
        if !faces.is_empty() {
            self.org_command(Org::PaintFaces {
                faces,
                material: paint,
            });
        }
        true
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
        let ctrl = mods.ctrl || mods.command;
        let shift = mods.shift;
        let sample = hit.filter(|h| h.path.starts_with(self.history.context()));
        let face = sample
            .filter(|h| h.path == self.history.context())
            .and_then(|h| match &h.hit {
                cindral_tools::Hit::Face(f) => Some(*f),
                _ => None,
            });
        let child = sample
            .filter(|h| h.path.len() > self.history.context().len())
            .map(|h| h.path[..self.history.context().len() + 1].to_vec());
        let key = (face, child.clone(), u8::from(ctrl) << 1 | u8::from(shift));
        if self.paint_hover.key == key {
            return;
        }
        let hover = if ctrl || shift {
            let mut faces = Vec::new();
            let instance = None;
            if let (Some(f), false) = (face, shift && ctrl) {
                let model = self.history.model();
                let current = model.faces().get(&f).and_then(|m| m.material);
                faces = if ctrl && shift {
                    faces_with_material(model, current)
                } else if ctrl {
                    fill_adjacent_same_material(model, f)
                } else {
                    faces_with_material(model, current)
                };
            } else if let Some(path) = &child {
                if let Some(node) = self.history.instance(path) {
                    if let Some(model) = self.history.definition_model(node.definition) {
                        faces = faces_with_material(model, node.material);
                    }
                }
            }
            PaintHover {
                faces,
                instance,
                key,
            }
        } else {
            let instance = child;
            let faces = face.map(|f| vec![f]).unwrap_or_default();
            PaintHover {
                faces,
                instance,
                key,
            }
        };
        self.paint_hover = hover;
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
            "Replace matching material inside the clicked object"
        } else if ctrl {
            "Fill adjacent faces sharing the clicked material"
        } else if shift {
            "Replace all faces sharing the clicked material"
        } else {
            "Click to paint · Ctrl fill adjacent · Shift replace · Alt sample"
        };
        format!("{action} · painting: {material}")
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
        if self.paint_hover.faces.is_empty() {
            return;
        }
        let screen = |p: cindral_core::DVec3| {
            view.project(p)
                .map(|p| rect.min + egui::vec2(p.x as f32, p.y as f32))
        };
        let stroke = egui::Stroke::new(
            2_f32,
            self.viewport_overlay(Color32::from_rgb(120, 200, 255)),
        );
        let model = self.history.model();
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
            if let (Some(a), Some(b)) = (screen(min), screen(max)) {
                ui.painter().rect_stroke(
                    egui::Rect::from_two_pos(a, b),
                    2.,
                    stroke,
                    egui::StrokeKind::Middle,
                );
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
        // Render the real inspector so the Materials list builds visible rows.
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
            app.materials.rows_built < 100,
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
}
