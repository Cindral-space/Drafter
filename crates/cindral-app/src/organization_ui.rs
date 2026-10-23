//! Context navigation, Outliner and document metadata controls.
use super::*;
use cindral_core::{
    DefinitionId, DefinitionKind, InstancePath, Material, MaterialId, OrganizationCommand as Org,
    TagId,
};
use cindral_tools::Hit;
use glam::{DMat4, DVec2};
use std::collections::BTreeSet;
#[derive(Clone)]
struct Row {
    path: InstancePath,
    name: String,
    kind: DefinitionKind,
    hidden: bool,
    children: bool,
}
#[derive(Clone)]
pub(crate) struct InstanceDrag {
    path: InstancePath,
    kind: ModifyKind,
    before: DMat4,
    owner_world: DMat4,
    context_world: DMat4,
    anchor: DVec3,
    pivot: DVec3,
    reference: DVec3,
    radius: f64,
    extents: DVec3,
    measure_direction: Option<DVec3>,
    measured: Option<cindral_tools::measurements::Readout>,
    last: DVec3,
    start: DVec2,
}
pub(crate) struct OrganizationUi {
    pub(crate) selected: Option<InstancePath>,
    pub(crate) multiple: BTreeSet<InstancePath>,
    pub(crate) drag: Option<InstanceDrag>,
    pub(crate) completed: Option<InstanceDrag>,
    expanded: BTreeSet<InstancePath>,
    rows: Vec<Row>,
    revision: Option<u64>,
    rename: String,
    tag_name: String,
    tag: Option<TagId>,
    pub(crate) material: Option<MaterialId>,
    material_name: String,
    color: [f32; 3],
    opacity: f32,
    pub(crate) texture: String,
}
impl Default for OrganizationUi {
    fn default() -> Self {
        Self {
            selected: None,
            multiple: BTreeSet::new(),
            drag: None,
            completed: None,
            expanded: BTreeSet::new(),
            rows: Vec::new(),
            revision: None,
            rename: String::new(),
            tag_name: "Tag".into(),
            tag: None,
            material: None,
            material_name: "Material".into(),
            color: [0.65, 0.7, 0.75],
            opacity: 1.,
            texture: String::new(),
        }
    }
}
impl DrafterApp {
    /// Read the full organization document, independent of the current edit context.
    pub fn document(&self) -> &History {
        &self.history
    }
    /// Replace the editor document from validated schema values (file encoding belongs to M6).
    pub fn set_document_data(
        &mut self,
        data: cindral_core::DocumentData,
    ) -> Result<(), cindral_core::KernelError> {
        let history = History::from_data(data)?;
        let removed = self.scene.definitions();
        self.history = history;
        self.presentation = Default::default();
        self.presentation_ui = Default::default();
        self.presentation_serial = self.presentation_serial.wrapping_add(1);
        self.scene = scene_cache::SceneCache::default();
        self.pending = Some(Arc::new(SceneUpdate {
            removed,
            ..Default::default()
        }));
        self.guide_cache.clear();
        self.guide_definition = DefinitionId(0);
        self.modify.replace_guides(Vec::new());
        self.last_tape = None;
        self.organization = OrganizationUi::default();
        self.selection.clear();
        self.tool.cancel();
        self.modify.cancel();
        self.rotate = Default::default();
        self.refresh_mesh();
        Ok(())
    }
    /// Execute one organization command and prepare definition/instance dirty updates.
    pub fn organize(&mut self, command: Org) -> Result<bool, cindral_core::KernelError> {
        let changed = self.history.organization(command)?;
        if changed {
            self.refresh_mesh();
            self.selection.clear();
            if self
                .organization
                .selected
                .as_ref()
                .is_some_and(|p| self.history.instance(p).is_none())
            {
                self.organization.selected = None;
                self.organization.multiple.clear();
            }
        }
        Ok(changed)
    }
    fn org_command(&mut self, command: Org) {
        match self.organize(command) {
            Ok(_) => self.error = None,
            Err(e) => self.error = Some(e.to_string()),
        }
    }
    /// Move the selected complete geometry into a private group or shared component.
    pub fn create_container(
        &mut self,
        kind: DefinitionKind,
    ) -> Result<InstancePath, cindral_core::KernelError> {
        let vertices = self.selection.vertex_ids(self.history.model());
        self.organize(Org::Create {
            kind,
            name: if kind == DefinitionKind::Group {
                "Group"
            } else {
                "Component"
            }
            .into(),
            vertices,
        })?;
        let id = *self
            .history
            .instances(self.history.active_definition())
            .and_then(|m| m.keys().next_back())
            .ok_or(cindral_core::KernelError::MissingEntity)?;
        let mut path = self.history.context().to_vec();
        path.push(id);
        self.select_instance(path.clone());
        Ok(path)
    }
    /// Enter a nested occurrence. Tools and inference immediately use its local geometry.
    pub fn enter_context(&mut self, path: InstancePath) -> Result<(), cindral_core::KernelError> {
        self.cancel_modify();
        self.tool.cancel();
        self.work_plane = DrawingPlane::default();
        let _ = self.tool.set_plane(self.work_plane);
        self.axis_lock = None;
        self.history.enter(path.clone())?;
        self.selection.clear();
        self.organization.selected = None;
        for n in 1..=path.len() {
            self.organization.expanded.insert(path[..n].to_vec());
        }
        self.measurements.reset();
        self.refresh_mesh();
        Ok(())
    }
    /// Exit one editing level and restore the parent's local tool frame.
    pub fn exit_context(&mut self) -> Result<bool, cindral_core::KernelError> {
        self.cancel_modify();
        self.tool.cancel();
        self.work_plane = DrawingPlane::default();
        let _ = self.tool.set_plane(self.work_plane);
        self.axis_lock = None;
        let changed = self.history.exit()?;
        self.selection.clear();
        self.organization.selected = None;
        self.measurements.reset();
        if changed {
            self.refresh_mesh();
        }
        Ok(changed)
    }
    fn select_instance(&mut self, path: InstancePath) {
        if let Some(node) = self.history.instance(&path) {
            self.organization.rename = node.name.clone();
            self.organization.tag = node.tag;
            self.organization.selected = Some(path);
            self.organization.multiple.clear();
            self.selection.clear();
        }
    }
    fn rebuild_rows(&mut self) {
        fn visit(
            doc: &History,
            id: DefinitionId,
            parent: InstancePath,
            expanded: &BTreeSet<InstancePath>,
            rows: &mut Vec<Row>,
        ) {
            if parent.len() >= 64 || rows.len() > 100_000 {
                return;
            }
            if let Some(nodes) = doc.instances(id) {
                for node in nodes.values() {
                    let mut path = parent.clone();
                    path.push(node.id);
                    let children = doc
                        .instances(node.definition)
                        .is_some_and(|m| !m.is_empty());
                    rows.push(Row {
                        path: path.clone(),
                        name: node.name.clone(),
                        kind: doc
                            .definition_kind(node.definition)
                            .unwrap_or(DefinitionKind::Group),
                        hidden: node.hidden,
                        children,
                    });
                    if expanded.contains(&path) {
                        visit(doc, node.definition, path, expanded, rows);
                    }
                }
            }
        }
        let mut rows = Vec::new();
        visit(
            &self.history,
            DefinitionId(0),
            Vec::new(),
            &self.organization.expanded,
            &mut rows,
        );
        self.organization.rows = rows;
        self.organization.revision = Some(self.history.revision());
    }
    pub(crate) fn organization_panel(&mut self, ui: &mut egui::Ui) {
        if self.organization.revision.is_none()
            || self.organization.revision != Some(self.history.revision())
                && self.history.document_changes().instances
        {
            self.rebuild_rows();
        }
        self.organization.revision = Some(self.history.revision());
        let mut commands = Vec::new();
        let mut selected = None;
        let mut enter = None;
        let mut expanded = None;
        let selection_id = ui.make_persistent_id("outliner_selection");
        let previous = ui.ctx().data(|data| {
            data.get_temp::<Option<InstancePath>>(selection_id)
                .flatten()
        });
        let reveal_selection =
            self.organization.selected.is_some() && previous != self.organization.selected;
        ui.ctx()
            .data_mut(|data| data.insert_temp(selection_id, self.organization.selected.clone()));
        egui::CollapsingHeader::new("Outliner")
            .default_open(!self.organization.rows.is_empty())
            .open(reveal_selection.then_some(true))
            .show(ui, |ui| {
                let label = if self.history.context().is_empty() {
                    "Document".to_owned()
                } else {
                    format!(
                        "Editing {}",
                        self.history
                            .instance(self.history.context())
                            .map_or("context", |n| n.name.as_str())
                    )
                };
                if !self.history.context().is_empty() {
                    ui.horizontal(|ui| {
                        ui.label(label);
                        if ui.small_button("Exit").clicked() {
                            enter = Some(
                                self.history.context()[..self.history.context().len() - 1].to_vec(),
                            );
                        }
                    });
                }
                let root = ui
                    .add(egui::Label::new("Document").sense(egui::Sense::hover()))
                    .on_hover_text(
                        "Drop a group or component here to move it to the document root",
                    );
                if self.organization.rows.is_empty() {
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new("Groups and components will appear here.").weak(),
                        )
                        .wrap(),
                    );
                }
                if let Some(payload) = root.dnd_release_payload::<InstancePath>() {
                    commands.push(Org::Reparent {
                        path: (*payload).clone(),
                        parent: Vec::new(),
                    });
                }
                egui::ScrollArea::vertical()
                    .id_salt("outline_tree")
                    .max_height((ui.available_height() * 0.4).max(90.))
                    .show_rows(ui, 23., self.organization.rows.len(), |ui, range| {
                        for row in &self.organization.rows[range] {
                            ui.horizontal(|ui| {
                                ui.add_space((row.path.len() - 1) as f32 * 11.);
                                if row.children {
                                    if ui
                                        .small_button(
                                            if self.organization.expanded.contains(&row.path) {
                                                "-"
                                            } else {
                                                "+"
                                            },
                                        )
                                        .clicked()
                                    {
                                        expanded = Some(row.path.clone());
                                    }
                                } else {
                                    ui.add_space(19.);
                                }
                                let marker = if row.kind == DefinitionKind::Component {
                                    "C"
                                } else {
                                    "G"
                                };
                                let text = format!(
                                    "{marker} {}{}",
                                    row.name,
                                    if row.hidden { " (hidden)" } else { "" }
                                );
                                let response = ui.add(
                                    egui::Button::new(text)
                                        .frame(false)
                                        .selected(
                                            self.organization.selected.as_ref() == Some(&row.path),
                                        )
                                        .sense(egui::Sense::click_and_drag()),
                                );
                                if response.clicked() {
                                    selected = Some(row.path.clone());
                                }
                                if response.double_clicked() {
                                    enter = Some(row.path.clone());
                                }
                                response.dnd_set_drag_payload(row.path.clone());
                                if let Some(payload) =
                                    response.dnd_release_payload::<InstancePath>()
                                {
                                    commands.push(Org::Reparent {
                                        path: (*payload).clone(),
                                        parent: row.path.clone(),
                                    });
                                }
                            });
                        }
                    });
                let can_group = !self.selection.vertex_ids(self.history.model()).is_empty();
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(can_group, egui::Button::new("Group selection"))
                        .on_disabled_hover_text("Select geometry to create a group")
                        .clicked()
                    {
                        commands.push(Org::Create {
                            kind: DefinitionKind::Group,
                            name: "Group".into(),
                            vertices: self.selection.vertex_ids(self.history.model()),
                        });
                    }
                    if ui
                        .add_enabled(can_group, egui::Button::new("Component"))
                        .on_disabled_hover_text("Select geometry to create a component")
                        .clicked()
                    {
                        commands.push(Org::Create {
                            kind: DefinitionKind::Component,
                            name: "Component".into(),
                            vertices: self.selection.vertex_ids(self.history.model()),
                        });
                    }
                });
                if let Some(path) = self.organization.selected.clone() {
                    if let Some(node) = self.history.instance(&path).cloned() {
                        ui.horizontal(|ui| {
                            ui.text_edit_singleline(&mut self.organization.rename);
                            if ui.small_button("Rename").clicked() {
                                commands.push(Org::Rename {
                                    path: path.clone(),
                                    name: self.organization.rename.clone(),
                                });
                            }
                        });
                        ui.horizontal(|ui| {
                            let mut hidden = node.hidden;
                            if ui.checkbox(&mut hidden, "Hidden").changed() {
                                commands.push(Org::Hide {
                                    path: path.clone(),
                                    hidden,
                                });
                            }
                            if ui.small_button("Edit").clicked() {
                                enter = Some(path.clone());
                            }
                            if ui.small_button("Unique").clicked() {
                                commands.push(Org::MakeUnique(path.clone()));
                            }
                        });
                        ui.horizontal(|ui| {
                            if ui.small_button("Duplicate").clicked() {
                                commands.push(Org::Duplicate {
                                    path: path.clone(),
                                    transform: node.transform
                                        * DMat4::from_translation(DVec3::X * 2.),
                                });
                            }
                            if ui.small_button("Delete").clicked() {
                                commands.push(Org::Delete(path.clone()));
                            }
                        });
                    }
                }
            });
        egui::CollapsingHeader::new("Tags").show(ui, |ui| {
            let tags = self
                .history
                .tags()
                .iter()
                .map(|(id, t)| (*id, t.clone()))
                .collect::<Vec<_>>();
            for (id, tag) in tags {
                ui.horizontal(|ui| {
                    let mut visible = tag.visible;
                    if ui.checkbox(&mut visible, "").changed() {
                        commands.push(Org::TagVisibility { id, visible });
                    }
                    if ui
                        .selectable_label(self.organization.tag == Some(id), &tag.name)
                        .clicked()
                    {
                        self.organization.tag = Some(id);
                        self.organization.tag_name = tag.name.clone();
                    }
                    if ui.small_button("Delete").clicked() {
                        commands.push(Org::DeleteTag(id));
                    }
                });
            }
            ui.text_edit_singleline(&mut self.organization.tag_name);
            ui.horizontal(|ui| {
                if ui.small_button("New tag").clicked() {
                    commands.push(Org::CreateTag(self.organization.tag_name.clone()));
                }
                if ui.small_button("Rename tag").clicked() {
                    if let Some(id) = self.organization.tag {
                        commands.push(Org::RenameTag {
                            id,
                            name: self.organization.tag_name.clone(),
                        });
                    }
                }
            });
            if let Some(path) = self.organization.selected.clone() {
                ui.horizontal(|ui| {
                    if ui.small_button("Assign tag").clicked() {
                        commands.push(Org::AssignTag {
                            path: path.clone(),
                            tag: self.organization.tag,
                        });
                    }
                    if ui.small_button("Untag").clicked() {
                        commands.push(Org::AssignTag { path, tag: None });
                    }
                });
            }
        });
        egui::CollapsingHeader::new("Materials").show(ui, |ui| {
            let materials = self
                .history
                .materials()
                .iter()
                .map(|(id, m)| (*id, m.clone()))
                .collect::<Vec<_>>();
            let label = self
                .organization
                .material
                .and_then(|id| self.history.materials().get(&id))
                .map_or("Default material", |m| m.name.as_str());
            egui::ComboBox::from_id_salt("paint_material")
                .selected_text(label)
                .show_ui(ui, |ui| {
                    if ui
                        .selectable_label(self.organization.material.is_none(), "Default")
                        .clicked()
                    {
                        self.organization.material = None;
                    }
                    for (id, m) in &materials {
                        if ui
                            .selectable_label(self.organization.material == Some(*id), &m.name)
                            .clicked()
                        {
                            self.organization.material = Some(*id);
                            self.organization.material_name = m.name.clone();
                            self.organization.color = m.color;
                            self.organization.opacity = m.opacity;
                            self.organization.texture = m.texture.clone().unwrap_or_default();
                        }
                    }
                });
            ui.text_edit_singleline(&mut self.organization.material_name);
            ui.color_edit_button_rgb(&mut self.organization.color);
            ui.add(egui::Slider::new(&mut self.organization.opacity, 0.0..=1.).text("Opacity"));
            ui.add(
                egui::TextEdit::singleline(&mut self.organization.texture)
                    .hint_text("Texture asset slot"),
            );
            if ui
                .add_enabled(
                    self.organization.material.is_some(),
                    egui::Button::new("Load PNG/JPEG texture…"),
                )
                .clicked()
            {
                self.load_material_texture();
            }
            let material = Material {
                name: self.organization.material_name.clone(),
                color: self.organization.color,
                opacity: self.organization.opacity,
                texture: (!self.organization.texture.trim().is_empty())
                    .then(|| self.organization.texture.clone()),
            };
            ui.horizontal(|ui| {
                if ui.small_button("New material").clicked() {
                    commands.push(Org::CreateMaterial(material.clone()));
                }
                if let Some(id) = self.organization.material {
                    if ui.small_button("Update").clicked() {
                        commands.push(Org::UpdateMaterial {
                            id,
                            material: material.clone(),
                        });
                    }
                    if ui.small_button("Delete").clicked() {
                        commands.push(Org::DeleteMaterial(id));
                    }
                }
            });
            if ui.button("Paint selected").clicked() {
                if let Some(path) = self.organization.selected.clone() {
                    commands.push(Org::PaintInstance {
                        path,
                        material: self.organization.material,
                    });
                } else if !self.selection.faces.is_empty() {
                    commands.push(Org::PaintFaces {
                        faces: self.selection.faces.iter().copied().collect(),
                        material: self.organization.material,
                    });
                }
            }
        });
        if let Some(path) = expanded {
            if !self.organization.expanded.remove(&path) {
                self.organization.expanded.insert(path);
            }
            self.organization.revision = None;
        }
        if let Some(path) = selected {
            self.select_instance(path);
        }
        if let Some(path) = enter {
            if let Err(e) = self.enter_context(path) {
                self.error = Some(e.to_string());
            }
        }
        for command in commands {
            let create = matches!(command, Org::Create { .. });
            let new_material = matches!(command, Org::CreateMaterial(_));
            self.org_command(command);
            if self.error.is_none() {
                if create {
                    let mut path = self.history.context().to_vec();
                    if let Some(id) = self
                        .history
                        .instances(self.history.active_definition())
                        .and_then(|m| m.keys().next_back())
                    {
                        path.push(*id);
                        self.select_instance(path);
                    }
                }
                if new_material {
                    self.organization.material =
                        self.history.materials().keys().next_back().copied();
                }
            }
        }
    }
}

impl DrafterApp {
    fn reject_inactive_face(&mut self, hit: &cindral_tools::InstanceHit) -> bool {
        if matches!(hit.hit, Hit::Face(_)) && hit.path != self.history.context() {
            self.error = Some("Cannot edit this face: it belongs to another group or component. Double-click to enter it first.".into());
            true
        } else {
            false
        }
    }
    pub(crate) fn scene_hit(
        &self,
        ray: Ray,
        view: View,
        cursor: DVec2,
    ) -> Option<cindral_tools::InstanceHit> {
        let items = self
            .scene
            .occurrences()
            .iter()
            .filter_map(|o| {
                self.history.definition_model(o.definition).map(|model| {
                    cindral_tools::PickInstance {
                        path: o.path.clone(),
                        definition: o.definition,
                        model,
                        world: o.world,
                    }
                })
            })
            .collect::<Vec<_>>();
        cindral_tools::pick_instances_section(
            &items,
            ray,
            view,
            cursor,
            10.,
            self.presentation.sections.cut(),
        )
    }
    pub(crate) fn selected_world_bounds(
        &self,
        path: &[cindral_core::InstanceId],
    ) -> Option<(DVec3, DVec3)> {
        let mut bounds: Option<(DVec3, DVec3)> = None;
        let mut definitions = std::collections::BTreeMap::new();
        for item in self
            .scene
            .occurrences()
            .iter()
            .filter(|o| o.path.starts_with(path))
        {
            let Some((min, max)) = *definitions.entry(item.definition).or_insert_with(|| {
                self.history
                    .definition_model(item.definition)
                    .and_then(|m| m.bounds())
            }) else {
                continue;
            };
            for x in [min.x, max.x] {
                for y in [min.y, max.y] {
                    for z in [min.z, max.z] {
                        let p = item.world.transform_point3(DVec3::new(x, y, z));
                        bounds = Some(bounds.map_or((p, p), |(a, b)| (a.min(p), b.max(p))));
                    }
                }
            }
        }
        bounds
    }
    fn begin_instance_drag(
        &mut self,
        path: InstancePath,
        point: DVec3,
        cursor: DVec2,
    ) -> Result<(), cindral_core::KernelError> {
        let instance = self
            .history
            .instance(&path)
            .ok_or(cindral_core::KernelError::MissingEntity)?
            .clone();
        let world = self
            .scene
            .occurrences()
            .iter()
            .find(|o| o.path == path)
            .map(|o| o.world)
            .ok_or(cindral_core::KernelError::MissingEntity)?;
        let context = self.history.world_transform();
        let inverse = context.inverse();
        let pivot = self
            .selected_world_bounds(&path)
            .map_or(point, |(a, b)| inverse.transform_point3((a + b) * 0.5));
        let offset = point - pivot;
        let planar = offset - self.work_plane.normal * offset.dot(self.work_plane.normal);
        let radius = planar.length().max(1e-5);
        let reference = planar.try_normalize().unwrap_or(DVec3::X);
        self.organization.completed = None;
        self.organization.drag = Some(InstanceDrag {
            path: path.clone(),
            kind: self.modify.kind(),
            before: instance.transform,
            owner_world: world * instance.transform.inverse(),
            context_world: context,
            anchor: point,
            pivot,
            reference,
            radius,
            extents: self
                .selected_world_bounds(&path)
                .map_or(DVec3::ONE, |(a, b)| inverse.transform_vector3(b - a).abs()),
            measure_direction: None,
            measured: None,
            last: point,
            start: cursor,
        });
        Ok(())
    }
    fn instance_delta(&self, drag: &InstanceDrag, point: DVec3, typed: Option<f64>) -> DMat4 {
        match drag.kind {
            ModifyKind::Move | ModifyKind::Copy => {
                let offset = point - drag.anchor;
                let movement = if let Some(length) = typed {
                    offset.try_normalize().unwrap_or(DVec3::X) * length
                } else {
                    offset
                };
                DMat4::from_translation(movement)
            }
            ModifyKind::Rotate => {
                let normal = self.work_plane.normal.normalize_or_zero();
                let offset = point - drag.pivot;
                let planar = offset - normal * offset.dot(normal);
                let angle = typed.map(f64::to_radians).unwrap_or_else(|| {
                    cindral_tools::rotation::angle(normal, drag.reference, planar)
                });
                cindral_tools::rotation::matrix(drag.pivot, normal, angle)
            }
            ModifyKind::Scale => {
                let offset = point - drag.pivot;
                let planar = offset - self.work_plane.normal * offset.dot(self.work_plane.normal);
                let factor = typed.unwrap_or_else(|| planar.length() / drag.radius);
                DMat4::from_translation(drag.pivot)
                    * DMat4::from_scale(DVec3::splat(factor))
                    * DMat4::from_translation(-drag.pivot)
            }
            _ => DMat4::IDENTITY,
        }
    }
    fn preview_instance(&mut self, drag: &InstanceDrag, point: DVec3, typed: Option<f64>) {
        let _phase = cindral_core::profile::Scope::new(3);
        let delta = self.instance_delta(drag, point, typed);
        let transform = drag.owner_world.inverse()
            * drag.context_world
            * delta
            * drag.context_world.inverse()
            * drag.owner_world
            * drag.before;
        let result = if drag.kind == ModifyKind::Copy {
            self.history.preview_duplicate(&drag.path, transform)
        } else {
            self.history
                .preview_instance_transform(&drag.path, transform)
        };
        match result {
            Ok(_) => self.error = None,
            Err(e) => self.error = Some(e.to_string()),
        }
        self.refresh_mesh();
    }
    pub(crate) fn instance_readout(&self) -> Option<cindral_tools::measurements::Readout> {
        use cindral_tools::measurements::{Readout, ValueKind};
        let drag = self
            .organization
            .drag
            .as_ref()
            .or(self.organization.completed.as_ref())?;
        if let Some(out) = drag.measured {
            return Some(out);
        }
        let delta = drag.last - drag.anchor;
        let (label, value, kind) = match drag.kind {
            ModifyKind::Rotate => {
                let d = drag.last - drag.pivot;
                let normal = self.work_plane.normal.normalize_or_zero();
                (
                    "Angle",
                    d.dot(normal.cross(drag.reference))
                        .atan2(d.dot(drag.reference))
                        .to_degrees(),
                    ValueKind::Angle,
                )
            }
            ModifyKind::Scale => (
                "Scale",
                (drag.last - drag.pivot).length() / drag.radius,
                ValueKind::Factor,
            ),
            _ => ("Distance", delta.length(), ValueKind::Length),
        };
        Some(Readout {
            label,
            values: [value, 0., 0.],
            count: 1,
            kind,
        })
    }
    pub(crate) fn instance_measurement(&mut self) -> bool {
        use cindral_tools::measurements::{parse, Measurement, Quantity, Readout, ValueKind};
        let Some(mut drag) = self
            .organization
            .drag
            .clone()
            .or(self.organization.completed.clone())
        else {
            return false;
        };
        let replacing = self.organization.drag.is_none();
        let settings = self.presentation.measurements;
        let result = (|| -> Result<Org, String> {
            let input = parse(&self.measurements.text, settings).map_err(|e| e.to_string())?;
            let bad = || "Input is not supported by this container tool".to_string();
            let to_transform = |delta: DMat4| {
                drag.owner_world.inverse()
                    * drag.context_world
                    * delta
                    * drag.context_world.inverse()
                    * drag.owner_world
                    * drag.before
            };
            let mut readout = Readout {
                label: "Distance",
                values: [0.; 3],
                count: 1,
                kind: ValueKind::Length,
            };
            let command = match input {
                Measurement::Copies { count, divide }
                    if matches!(drag.kind, ModifyKind::Move | ModifyKind::Copy) =>
                {
                    let delta = drag.last - drag.anchor;
                    if delta.length() <= cindral_core::TOLERANCE {
                        return Err(bad());
                    }
                    Org::DuplicateMany {
                        path: drag.path.clone(),
                        transforms: (1..=count)
                            .map(|i| {
                                to_transform(DMat4::from_translation(
                                    delta * (i as f64) / if divide { count as f64 } else { 1. },
                                ))
                            })
                            .collect(),
                    }
                }
                input => {
                    let values = match input {
                        Measurement::Values(v) => v,
                        Measurement::Slope(deg) if drag.kind == ModifyKind::Rotate => {
                            vec![Quantity::Degrees(deg)]
                        }
                        Measurement::Copies {
                            count,
                            divide: false,
                        } if drag.kind == ModifyKind::Scale => vec![Quantity::Number(count as f64)],
                        _ => return Err(bad()),
                    };
                    if values.is_empty() || values.len() > 3 {
                        return Err(bad());
                    }
                    let delta = match drag.kind {
                        ModifyKind::Move | ModifyKind::Copy if values.len() == 1 => {
                            let n = values[0].length(settings).map_err(|e| e.to_string())?;
                            let direction = drag.measure_direction.unwrap_or_else(|| {
                                (drag.last - drag.anchor)
                                    .try_normalize()
                                    .unwrap_or(DVec3::X)
                            });
                            readout.values[0] = n;
                            self.instance_delta(&drag, drag.anchor + direction, Some(n))
                        }
                        ModifyKind::Rotate if values.len() == 1 => {
                            let n = values[0].number().map_err(|e| e.to_string())?;
                            readout.label = "Angle";
                            readout.kind = ValueKind::Angle;
                            readout.values[0] = n;
                            self.instance_delta(&drag, drag.last, Some(n))
                        }
                        ModifyKind::Scale => {
                            readout.label = "Scale";
                            readout.kind = ValueKind::Factor;
                            readout.count = values.len();
                            let mut factors = DVec3::ONE;
                            for (axis, q) in values.iter().enumerate() {
                                let extent = if values.len() == 1 {
                                    drag.extents.max_element()
                                } else {
                                    drag.extents[axis]
                                };
                                let n = match q {
                                    Quantity::Number(n) => *n,
                                    Quantity::Length(n) if extent > cindral_core::TOLERANCE => {
                                        n / extent
                                    }
                                    _ => return Err(bad()),
                                };
                                if !n.is_finite() || n.abs() <= cindral_core::TOLERANCE {
                                    return Err(bad());
                                }
                                factors[axis] = n;
                                readout.values[axis] = n;
                            }
                            if values.len() == 1 {
                                factors = DVec3::splat(factors.x);
                            }
                            DMat4::from_translation(drag.pivot)
                                * DMat4::from_scale(factors)
                                * DMat4::from_translation(-drag.pivot)
                        }
                        _ => return Err(bad()),
                    };
                    let transform = to_transform(delta);
                    if matches!(drag.kind, ModifyKind::Move | ModifyKind::Copy) {
                        let direction = drag.measure_direction.unwrap_or_else(|| {
                            (drag.last - drag.anchor)
                                .try_normalize()
                                .unwrap_or(DVec3::X)
                        });
                        drag.measure_direction = Some(direction);
                        drag.last = drag.anchor + direction * readout.values[0];
                    }
                    drag.measured = Some(readout);
                    if drag.kind == ModifyKind::Copy {
                        Org::Duplicate {
                            path: drag.path.clone(),
                            transform,
                        }
                    } else {
                        Org::Transform {
                            path: drag.path.clone(),
                            transform,
                        }
                    }
                }
            };
            Ok(command)
        })();
        match result {
            Ok(command) => {
                let validation = match &command {
                    Org::Transform { transform, .. } | Org::Duplicate { transform, .. } => {
                        cindral_core::validate_instance_transform(*transform)
                    }
                    Org::DuplicateMany { transforms, .. } => transforms
                        .iter()
                        .try_for_each(|m| cindral_core::validate_instance_transform(*m)),
                    _ => Ok(()),
                };
                if let Err(error) = validation {
                    self.error = Some(error.to_string());
                    self.measurements.invalid = true;
                    return true;
                }
                let applied = if replacing {
                    self.history.replace_organization(command)
                } else {
                    self.history.organization(command)
                };
                match applied {
                    Ok(true) => {
                        self.organization.drag = None;
                        self.organization.completed = Some(drag);
                        self.measurements.completed();
                        self.error = None;
                        self.refresh_mesh();
                    }
                    result => {
                        self.error = Some(
                            result
                                .err()
                                .map(|e| e.to_string())
                                .unwrap_or_else(|| "No change".into()),
                        )
                    }
                }
            }
            Err(error) => self.error = Some(error),
        }
        self.measurements.invalid = self.error.is_some();
        true
    }
    pub(crate) fn organization_pointer(&mut self, response: &egui::Response) -> bool {
        let ctx = &response.ctx;
        let keyboard = !ctx.wants_keyboard_input() && self.measurements.text.is_empty();
        if keyboard && ctx.input(|i| shortcuts::pressed(i, shortcuts::Action::Cancel)) {
            self.organization.completed = None;
            if self.organization.drag.is_some() {
                self.organization.drag = None;
                match self.history.cancel_preview() {
                    Ok(_) => self.refresh_mesh(),
                    Err(e) => self.error = Some(e.to_string()),
                }
                self.measurements.text.clear();
                return true;
            }
            if !self.history.context().is_empty()
                && !self.modify.active()
                && !self.modify.editing_completed()
                && !self.tool.editing_completed()
                && self.tool.anchor().is_none()
            {
                if let Err(e) = self.exit_context() {
                    self.error = Some(e.to_string());
                }
                return true;
            }
        }
        if let Some(mut drag) = self.organization.drag.clone() {
            if keyboard {
                let redo = ctx.input(|i| {
                    shortcuts::pressed(i, shortcuts::Action::Redo)
                });
                let undo = ctx.input(|i| shortcuts::pressed(i, shortcuts::Action::Undo) || shortcuts::pressed(i, shortcuts::Action::Redo));
                if undo || redo {
                    self.organization.drag = None;
                    self.history_step(redo);
                    return true;
                }
                ctx.input(|i| {
                    for (key, axis) in [
                        (shortcuts::Action::AxisX, Axis::X),
                        (shortcuts::Action::AxisY, Axis::Y),
                        (shortcuts::Action::AxisZ, Axis::Z),
                    ] {
                        if shortcuts::pressed(i, key) {
                            self.axis_lock = Some(axis);
                        }
                    }
                    if shortcuts::pressed(i, shortcuts::Action::AxisClear) {
                        self.axis_lock = None;
                    }
                });
            }
            let Some(position) = ctx.input(|i| i.pointer.hover_pos()) else {
                return true;
            };
            let rect = response.rect;
            let cursor = DVec2::new(
                (position.x - rect.left()) as f64,
                (position.y - rect.top()) as f64,
            );
            let aspect = rect.width() / rect.height();
            let (origin, direction) = self.camera.ray(
                Vec2::new(
                    2. * cursor.x as f32 / rect.width() - 1.,
                    1. - 2. * cursor.y as f32 / rect.height(),
                ),
                aspect,
            );
            let inv = drag.context_world.inverse();
            let ray = Ray {
                origin: inv.transform_point3(origin.as_dvec3()),
                direction: inv.transform_vector3(direction.as_dvec3()),
            };
            let mut query = Query::new(
                cursor,
                ray,
                View {
                    matrix: self.camera.view_projection(aspect).as_dmat4() * drag.context_world,
                    size: DVec2::new(rect.width() as f64, rect.height() as f64),
                },
            );
            query.anchor = Some(drag.anchor);
            query.axis_lock = self.axis_lock;
            query.plane = DrawingPlane {
                origin: if matches!(drag.kind, ModifyKind::Rotate | ModifyKind::Scale) {
                    drag.pivot
                } else {
                    drag.anchor
                },
                normal: self.work_plane.normal,
            };
            query.section = self.local_section();
            if let Some(snap) = self.engine.infer(self.history.model(), &query) {
                drag.last = snap.position;
                self.snap = Some(snap);
            }
            if cursor.distance(drag.start) > 3. {
                self.preview_instance(&drag, drag.last, None);
            }
            let released = ctx.input(|i| i.pointer.button_released(PointerButton::Primary));
            self.organization.drag = Some(drag.clone());
            if released && cursor.distance(drag.start) > 3. && self.error.is_none() {
                if let Err(e) = self.history.commit_preview() {
                    self.error = Some(e.to_string());
                }
                if self.error.is_none() {
                    self.organization.completed = Some(drag);
                    self.measurements.completed();
                }
                self.organization.drag = None;
            }
            return true;
        }
        if !response.hovered() {
            return false;
        }
        if ctx.input(|i| i.pointer.button_pressed(PointerButton::Primary)) {
            self.organization.completed = None;
            self.measurements.reset();
        }
        let rect = response.rect;
        let Some(position) = ctx.input(|i| i.pointer.hover_pos()) else {
            return false;
        };
        let cursor = DVec2::new(
            (position.x - rect.left()) as f64,
            (position.y - rect.top()) as f64,
        );
        let aspect = rect.width() / rect.height();
        let (origin, direction) = self.camera.ray(
            Vec2::new(
                2. * cursor.x as f32 / rect.width() - 1.,
                1. - 2. * cursor.y as f32 / rect.height(),
            ),
            aspect,
        );
        let ray = Ray {
            origin: origin.as_dvec3(),
            direction: direction.as_dvec3(),
        };
        let view = View {
            matrix: self.camera.view_projection(aspect).as_dmat4(),
            size: DVec2::new(rect.width() as f64, rect.height() as f64),
        };
        let clicked = response.clicked_by(PointerButton::Primary);
        let pressed = ctx.input(|i| i.pointer.button_pressed(PointerButton::Primary));
        if !clicked && !pressed && !response.double_clicked() {
            return false;
        }
        let hit = self.scene_hit(ray, view, cursor);
        let context = self.history.context().to_vec();
        if response.double_clicked() {
            if let Some(hit) = &hit {
                if hit.path.starts_with(&context) && hit.path.len() > context.len() {
                    let path = hit.path[..context.len() + 1].to_vec();
                    if let Err(e) = self.enter_context(path) {
                        self.error = Some(e.to_string());
                    }
                    return true;
                }
            }
        }
        if clicked && !context.is_empty() && !self.modify.active() && self.tool.anchor().is_none() {
            let outside = hit
                .as_ref()
                .is_some_and(|hit| !hit.path.starts_with(&context))
                || hit.is_none()
                    && self
                        .selected_world_bounds(&context)
                        .is_some_and(|(min, max)| {
                            let mut lo = DVec2::splat(f64::INFINITY);
                            let mut hi = DVec2::splat(f64::NEG_INFINITY);
                            for x in [min.x, max.x] {
                                for y in [min.y, max.y] {
                                    for z in [min.z, max.z] {
                                        if let Some(p) = view.project(DVec3::new(x, y, z)) {
                                            lo = lo.min(p);
                                            hi = hi.max(p);
                                        }
                                    }
                                }
                            }
                            cursor.cmplt(lo - DVec2::splat(8.)).any()
                                || cursor.cmpgt(hi + DVec2::splat(8.)).any()
                        });
            if outside {
                if let Err(e) = self.exit_context() {
                    self.error = Some(e.to_string());
                }
                return true;
            }
        }
        let child = hit
            .as_ref()
            .filter(|h| h.path.starts_with(&context) && h.path.len() > context.len())
            .map(|h| h.path[..context.len() + 1].to_vec());
        if self.modifying && self.modify.kind() == ModifyKind::Select && clicked {
            if let Some(path) = child {
                let loose = self.selection.clone();
                let mut multiple = self.organization.multiple.clone();
                if let Some(previous) = self.organization.selected.clone() {
                    multiple.insert(previous);
                }
                multiple.insert(path.clone());
                self.select_instance(path);
                if ctx.input(|i| i.modifiers.shift) {
                    self.selection = loose;
                    self.organization.multiple = multiple;
                }
                self.selection_start = None;
                return true;
            }
            if !ctx.input(|i| i.modifiers.shift) {
                self.organization.selected = None;
                self.organization.multiple.clear();
            }
        }
        if self.modifying && self.modify.kind() == ModifyKind::PaintBucket && pressed {
            if let Some(path) = child {
                self.org_command(Org::PaintInstance {
                    path,
                    material: self.organization.material,
                });
                return true;
            }
            if let Some(material) = self.organization.material {
                if let Some(hit) = &hit {
                    if hit.path == context {
                        if let Hit::Face(face) = hit.hit {
                            self.org_command(Org::PaintFaces {
                                faces: vec![face],
                                material: Some(material),
                            });
                            return true;
                        }
                    }
                }
            }
        }
        if pressed
            && self.modifying
            && matches!(
                self.modify.kind(),
                ModifyKind::PushPull
                    | ModifyKind::Offset
                    | ModifyKind::FollowMe
                    | ModifyKind::Eraser
            )
            && hit.as_ref().is_some_and(|h| self.reject_inactive_face(h))
        {
            return true;
        }
        if self.modifying
            && matches!(
                self.modify.kind(),
                ModifyKind::Move | ModifyKind::Copy | ModifyKind::Rotate | ModifyKind::Scale
            )
            && pressed
        {
            let path = self.organization.selected.clone().or(child);
            if let Some(path) = path {
                let point = hit
                    .map(|h| {
                        self.history
                            .world_transform()
                            .inverse()
                            .transform_point3(h.world_position)
                    })
                    .or_else(|| {
                        self.selected_world_bounds(&path).map(|(a, b)| {
                            self.history
                                .world_transform()
                                .inverse()
                                .transform_point3((a + b) * 0.5)
                        })
                    });
                if let Some(point) = point {
                    if let Err(e) = self.begin_instance_drag(path.clone(), point, cursor) {
                        self.error = Some(e.to_string());
                    } else {
                        self.select_instance(path);
                    }
                    return true;
                }
            }
        }
        false
    }
    pub(crate) fn organization_overlay(&self, ui: &egui::Ui, rect: egui::Rect) {
        if let Some(path) = &self.organization.selected {
            if let Some((min, max)) = self.selected_world_bounds(path) {
                let view = View {
                    matrix: self
                        .camera
                        .view_projection(rect.width() / rect.height())
                        .as_dmat4(),
                    size: DVec2::new(rect.width() as f64, rect.height() as f64),
                };
                let corners = (0..8)
                    .map(|i| {
                        DVec3::new(
                            if i & 1 == 0 { min.x } else { max.x },
                            if i & 2 == 0 { min.y } else { max.y },
                            if i & 4 == 0 { min.z } else { max.z },
                        )
                    })
                    .collect::<Vec<_>>();
                for i in 0..8 {
                    for bit in [1, 2, 4] {
                        let j = i ^ bit;
                        if i < j {
                            if let (Some(a), Some(b)) =
                                (view.project(corners[i]), view.project(corners[j]))
                            {
                                ui.painter().line_segment(
                                    [
                                        rect.min + egui::vec2(a.x as f32, a.y as f32),
                                        rect.min + egui::vec2(b.x as f32, b.y as f32),
                                    ],
                                    egui::Stroke::new(
                                        2_f32,
                                        self.viewport_overlay(Color32::from_rgb(255, 184, 68)),
                                    ),
                                );
                            }
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cindral_tools::Hit;
    #[test]
    fn inactive_face_reports_context_reason_and_entering_allows_edit() {
        let mut app = DrafterApp::default();
        let path = component(&mut app);
        let definition = app
            .history
            .occurrences()
            .unwrap()
            .into_iter()
            .find(|o| o.path == path)
            .unwrap()
            .definition;
        let face = *app
            .history
            .definition_model(definition)
            .unwrap()
            .faces()
            .keys()
            .next()
            .unwrap();
        let hit = cindral_tools::InstanceHit {
            path: path.clone(),
            definition,
            hit: Hit::Face(face),
            local_position: DVec3::ZERO,
            world_position: DVec3::ZERO,
        };
        assert!(app.reject_inactive_face(&hit));
        assert!(app
            .error
            .as_ref()
            .unwrap()
            .contains("Double-click to enter"));
        app.enter_context(path).unwrap();
        assert!(!app.reject_inactive_face(&hit));
    }
    fn component(app: &mut DrafterApp) -> InstancePath {
        app.execute(Command::AddPolyline {
            points: vec![DVec3::ZERO, DVec3::X, DVec3::X + DVec3::Y, DVec3::Y],
            closed: true,
        })
        .unwrap();
        let face = *app.model().faces().keys().next().unwrap();
        app.select_connected(Hit::Face(face));
        app.create_container(DefinitionKind::Component).unwrap()
    }
    #[test]
    fn scripted_three_instances_edit_in_place_exit_undo_reverts_all_and_redo_restores_all() {
        let mut app = DrafterApp::default();
        let path = component(&mut app);
        let definition = app.document().instance(&path).unwrap().definition;
        for x in [3., 6.] {
            app.organize(Org::AddInstance {
                definition,
                name: "Copy".into(),
                transform: DMat4::from_translation(DVec3::X * x),
            })
            .unwrap();
        }
        app.enter_context(path.clone()).unwrap();
        let face = *app.model().faces().keys().next().unwrap();
        let before = app.model().clone();
        let depth = app.history.undo_depth();
        app.preview(Command::PushPull { face, distance: 2. })
            .unwrap();
        assert_eq!(app.history.undo_depth(), depth);
        app.commit_preview().unwrap();
        assert_eq!(app.history.undo_depth(), depth + 1);
        app.exit_context().unwrap();
        let verify = |app: &DrafterApp, faces| {
            let occurrences = app.document().occurrences().unwrap();
            let copies: Vec<_> = occurrences
                .iter()
                .filter(|o| o.definition == definition)
                .collect();
            assert_eq!(copies.len(), 3);
            for item in copies {
                assert_eq!(
                    app.document()
                        .definition_model(item.definition)
                        .unwrap()
                        .faces()
                        .len(),
                    faces
                );
            }
        };
        verify(&app, 6);
        app.history_step(false);
        verify(&app, 1);
        assert_eq!(app.document().definition_model(definition), Some(&before));
        app.history_step(true);
        verify(&app, 6);
        app.enter_context(path).unwrap();
        app.preview(Command::PushPull {
            face,
            distance: 0.3,
        })
        .unwrap();
        app.exit_context().unwrap();
        verify(&app, 6);
        assert_eq!(app.history.undo_depth(), depth + 1);
    }
    #[test]
    fn container_drag_cancel_typed_commit_and_shared_context_dimming() {
        let mut app = DrafterApp::default();
        let path = component(&mut app);
        let definition = app.document().instance(&path).unwrap().definition;
        app.organize(Org::AddInstance {
            definition,
            name: "Other".into(),
            transform: DMat4::from_translation(DVec3::X * 3.),
        })
        .unwrap();
        app.choose_modify(ModifyKind::Move);
        let original = app.document().instance(&path).unwrap().transform;
        let depth = app.history.undo_depth();
        app.begin_instance_drag(path.clone(), DVec3::ZERO, DVec2::ZERO)
            .unwrap();
        let drag = app.organization.drag.clone().unwrap();
        app.preview_instance(&drag, DVec3::X * 2., None);
        assert_eq!(app.history.undo_depth(), depth);
        app.cancel_modify();
        assert_eq!(app.document().instance(&path).unwrap().transform, original);
        app.begin_instance_drag(path.clone(), DVec3::ZERO, DVec2::ZERO)
            .unwrap();
        app.organization.drag.as_mut().unwrap().last = DVec3::X;
        app.measurements.text = "2m".into();
        assert!(app.instance_measurement());
        assert_eq!(app.history.undo_depth(), depth + 1);
        assert_eq!(
            app.document()
                .instance(&path)
                .unwrap()
                .transform
                .transform_point3(DVec3::ZERO),
            original.transform_point3(DVec3::ZERO) + DVec3::X * 2.
        );
        app.enter_context(path).unwrap();
        let update = app.pending.as_ref().unwrap();
        let batch = update
            .batches
            .as_ref()
            .unwrap()
            .iter()
            .find(|b| b.definition == definition)
            .unwrap();
        assert_eq!(batch.instances.len(), 2);
        assert_eq!(
            batch
                .instances
                .iter()
                .filter(|i| i.parameters[1] == 1.)
                .count(),
            1
        );
        assert_eq!(
            batch
                .instances
                .iter()
                .filter(|i| i.parameters[1] == 0.25)
                .count(),
            1
        );
    }
    #[test]
    fn viewport_double_click_enters_and_escape_exits_without_history_entries() {
        let mut app = DrafterApp::default();
        let path = component(&mut app);
        app.choose_modify(ModifyKind::Select);
        let depth = app.history.undo_depth();
        let ctx = egui::Context::default();
        let view = View {
            matrix: app.camera.view_projection(1.5).as_dmat4(),
            size: DVec2::new(600., 400.),
        };
        let p = view.project(DVec3::new(0.5, 0.5, 0.)).unwrap();
        let pos = egui::pos2(p.x as f32 + 8., p.y as f32 + 8.);
        for _ in 0..2 {
            super::super::input_tests::frame(
                &mut app,
                &ctx,
                vec![egui::Event::PointerMoved(pos)],
                egui::Modifiers::NONE,
            );
            for pressed in [true, false] {
                super::super::input_tests::frame(
                    &mut app,
                    &ctx,
                    vec![egui::Event::PointerButton {
                        pos,
                        button: PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    }],
                    egui::Modifiers::NONE,
                );
            }
        }
        assert_eq!(app.history.context(), path.as_slice());
        assert_eq!(app.history.undo_depth(), depth);
        super::super::input_tests::frame(
            &mut app,
            &ctx,
            vec![egui::Event::Key {
                key: Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
            egui::Modifiers::NONE,
        );
        assert!(app.history.context().is_empty());
        assert_eq!(app.history.undo_depth(), depth);
        app.enter_context(path).unwrap();
        let p = view.project(DVec3::new(4., 4., 0.)).unwrap();
        let outside = egui::pos2(p.x as f32 + 8., p.y as f32 + 8.);
        super::super::input_tests::frame(
            &mut app,
            &ctx,
            vec![egui::Event::PointerMoved(outside)],
            egui::Modifiers::NONE,
        );
        for pressed in [true, false] {
            super::super::input_tests::frame(
                &mut app,
                &ctx,
                vec![egui::Event::PointerButton {
                    pos: outside,
                    button: PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                }],
                egui::Modifiers::NONE,
            );
        }
        assert!(app.history.context().is_empty());
        assert_eq!(app.history.undo_depth(), depth);
    }
    #[test]
    fn replacing_document_removes_previous_definition_gpu_resources() {
        let mut app = DrafterApp::default();
        let path = component(&mut app);
        let definition = app.document().instance(&path).unwrap().definition;
        app.pending = None; // Pretend the previous callback has consumed its upload.
        app.set_document_data(History::new().to_data()).unwrap();
        assert!(app.pending.as_ref().unwrap().removed.contains(&definition));
        assert!(app
            .document()
            .instances(DefinitionId(0))
            .unwrap()
            .is_empty());
    }
    #[test]
    fn construction_guides_stay_local_to_their_definition() {
        let mut app = DrafterApp::default();
        let path = component(&mut app);
        let root_guide = cindral_tools::Guide::new(DVec3::X, DVec3::Y);
        app.modify.replace_guides(vec![root_guide]);
        app.enter_context(path.clone()).unwrap();
        assert!(app.modify.guides().is_empty());
        let part_guide = cindral_tools::Guide::new(DVec3::Y, DVec3::X);
        app.modify.replace_guides(vec![part_guide]);
        app.exit_context().unwrap();
        assert_eq!(app.modify.guides(), &[root_guide]);
        app.enter_context(path).unwrap();
        assert_eq!(app.modify.guides(), &[part_guide]);
    }
}

#[cfg(test)]
mod measurement_replacement_tests {
    use super::*;
    fn container(kind: DefinitionKind) -> (DrafterApp, InstancePath) {
        let mut app = DrafterApp::default();
        app.execute(Command::AddPolyline {
            points: vec![
                DVec3::ZERO,
                DVec3::X * 2.,
                (DVec3::X + DVec3::Y) * 2.,
                DVec3::Y * 2.,
            ],
            closed: true,
        })
        .unwrap();
        app.selection
            .vertices
            .extend(app.model().vertices().keys().copied().collect::<Vec<_>>());
        let path = app.create_container(kind).unwrap();
        (app, path)
    }
    #[test]
    fn containers_retype_transform_and_arrays_one_undo_step() {
        for definition in [DefinitionKind::Group, DefinitionKind::Component] {
            for kind in [
                ModifyKind::Move,
                ModifyKind::Copy,
                ModifyKind::Rotate,
                ModifyKind::Scale,
            ] {
                let (mut app, path) = container(definition);
                let before = app.document().instance(&path).unwrap().transform;
                let depth = app.history.undo_depth();
                app.choose_modify(kind);
                app.begin_instance_drag(path.clone(), DVec3::X, DVec2::ZERO)
                    .unwrap();
                app.organization.drag.as_mut().unwrap().last = DVec3::X * 5.;
                let entries = match kind {
                    ModifyKind::Copy => ["5m", "3x", "/2"],
                    ModifyKind::Rotate => ["90deg", "8:12", "-90"],
                    ModifyKind::Scale => ["2", "4m", "2,3,1"],
                    _ => ["2m", "3m", "4m"],
                };
                for text in entries {
                    app.measurements.text = text.into();
                    app.apply_measurement();
                    assert!(
                        app.error.is_none(),
                        "{definition:?} {kind:?} {text}: {:?}",
                        app.error
                    );
                    assert_eq!(app.history.undo_depth(), depth + 1);
                }
                app.measurements.text = "bad".into();
                app.apply_measurement();
                assert!(app.error.is_some());
                assert_eq!(app.history.undo_depth(), depth + 1);
                let after = app.document().to_data();
                app.history_step(false);
                assert_eq!(app.document().instance(&path).unwrap().transform, before);
                if kind == ModifyKind::Copy {
                    assert_eq!(app.document().instances(DefinitionId(0)).unwrap().len(), 1);
                }
                app.history_step(true);
                assert_eq!(app.document().to_data(), after);
            }
        }
    }
    #[test]
    fn tape_selected_component_resizes_only_selected_occurrence() {
        let (mut app, path) = container(DefinitionKind::Component);
        let definition = app.document().instance(&path).unwrap().definition;
        let original_transform = app.document().instance(&path).unwrap().transform;
        let original = app.document().definition_model(definition).unwrap().clone();
        app.organize(Org::AddInstance {
            definition,
            name: "Other".into(),
            transform: DMat4::from_translation(DVec3::X * 10.),
        })
        .unwrap();
        let others = app.document().instances(DefinitionId(0)).unwrap().clone();
        app.organization.selected = Some(path.clone());
        app.choose_modify(ModifyKind::TapeMeasure);
        app.modify
            .begin(
                app.history.model(),
                &Selection::default(),
                None,
                DVec3::ZERO,
                DVec3::Z,
            )
            .unwrap();
        app.modify.update(DVec3::X * 2.).unwrap();
        app.modify.finish();
        let depth = app.history.undo_depth();
        for text in ["4m", "6m"] {
            app.measurements.text = text.into();
            app.apply_measurement();
            assert!(app.tape_resize.is_some());
            app.confirm_tape_resize(true);
            assert!(app.error.is_none(), "{:?}", app.error);
            assert_eq!(app.history.undo_depth(), depth + 1);
        }
        assert_eq!(app.document().definition_model(definition), Some(&original));
        for (id, node) in others {
            if id != path[0] {
                assert_eq!(app.document().instance(&[id]), Some(&node));
            }
        }
        app.history_step(false);
        assert_eq!(
            app.document().instance(&path).unwrap().transform,
            original_transform
        );
    }
}
