//! Runtime organization and cross-context command history.
use crate::*;
use glam::DMat4;
use std::collections::{BTreeMap, BTreeSet};
/// Occurrence in the visible document traversal.
#[derive(Clone, Debug)]
pub struct Occurrence {
    /// Nested instance path; empty means loose root geometry.
    pub path: InstancePath,
    /// Geometry source.
    pub definition: DefinitionId,
    /// Complete local-to-world affine transform.
    pub world: DMat4,
    /// Inherited material for unpainted faces.
    pub material: Option<MaterialId>,
    /// Whether this occurrence is the active editing context.
    pub active: bool,
}
/// Document dirty state, partitioned by geometry definition.
#[derive(Clone, Debug, Default)]
pub struct DocumentChanges {
    /// Monotonic runtime revision.
    pub revision: u64,
    /// Per-definition geometry changes.
    pub geometry: BTreeMap<DefinitionId, ChangeSet>,
    /// Changed instance batches, visibility or transforms.
    pub instances: bool,
    /// Material appearance changed without topology mutation.
    pub appearance: bool,
}
/// Organization mutations. Geometry continues to use ordinary `Command` values.
#[derive(Clone, Debug)]
pub enum OrganizationCommand {
    /// Move selected complete geometry into a new group/component in the active definition.
    Create {
        /// Ownership semantics for the new definition.
        kind: DefinitionKind,
        /// User-visible name.
        name: String,
        /// Complete connected geometry to extract.
        vertices: Vec<VertexId>,
    },
    /// Add a placed reference to a component definition in the active definition.
    AddInstance {
        /// Shared component definition.
        definition: DefinitionId,
        /// User-visible name.
        name: String,
        /// Absolute placement in the owning definition.
        transform: DMat4,
    },
    /// Duplicate a container in its current owner, with an absolute local placement.
    Duplicate {
        /// Target occurrence path from root.
        path: InstancePath,
        /// Absolute placement in the owning definition.
        transform: DMat4,
    },
    /// Remove a container occurrence (shared-parent changes affect every occurrence of that parent).
    Delete(InstancePath),
    /// Break one instance's definition-sharing link (including its private nested groups).
    MakeUnique(InstancePath),
    /// Change an instance name.
    Rename {
        /// Target occurrence path from root.
        path: InstancePath,
        /// User-visible name.
        name: String,
    },
    /// Set explicit visibility.
    Hide {
        /// Target occurrence path from root.
        path: InstancePath,
        /// Whether the occurrence is explicitly hidden.
        hidden: bool,
    },
    /// Assign or clear a container tag.
    AssignTag {
        /// Target occurrence path from root.
        path: InstancePath,
        /// Tag assignment, or none for untagged.
        tag: Option<TagId>,
    },
    /// Apply an inherited container material.
    PaintInstance {
        /// Target occurrence path from root.
        path: InstancePath,
        /// Material values or assignment.
        material: Option<MaterialId>,
    },
    /// Apply explicit face material references in the active geometry.
    PaintFaces {
        /// Active definition face identities.
        faces: Vec<FaceId>,
        /// Material values or assignment.
        material: Option<MaterialId>,
    },
    /// Set a local transform.
    Transform {
        /// Target occurrence path from root.
        path: InstancePath,
        /// Absolute placement in the owning definition.
        transform: DMat4,
    },
    /// Move an occurrence into a new parent while preserving its world placement.
    Reparent {
        /// Target occurrence path from root.
        path: InstancePath,
        /// Destination parent occurrence, empty for root.
        parent: InstancePath,
    },
    /// Create a tag (returns its ID through the document maps).
    CreateTag(String),
    /// Rename a tag.
    RenameTag {
        /// Document-local identity.
        id: TagId,
        /// User-visible name.
        name: String,
    },
    /// Remove a tag and clear all references.
    DeleteTag(TagId),
    /// Set tag visibility.
    TagVisibility {
        /// Document-local identity.
        id: TagId,
        /// Whether tagged occurrences are visible.
        visible: bool,
    },
    /// Restore scene visibility in one metadata transaction, ignoring deleted IDs.
    RestoreVisibility {
        /// Captured tag visible flags.
        tags: BTreeMap<TagId, bool>,
        /// Captured explicit hidden flags keyed by owner and instance identity.
        objects: BTreeMap<(DefinitionId, InstanceId), bool>,
    },
    /// Create a material.
    CreateMaterial(Material),
    /// Replace material values.
    UpdateMaterial {
        /// Document-local identity.
        id: MaterialId,
        /// Material values or assignment.
        material: Material,
    },
    /// Remove a material and clear all references.
    DeleteMaterial(MaterialId),
}

#[derive(Clone, Debug, PartialEq)]
struct DefinitionInfo {
    name: String,
    kind: DefinitionKind,
    instances: BTreeMap<InstanceId, Instance>,
}
#[derive(Clone, Debug, PartialEq)]
struct Graph {
    next_id: u64,
    definitions: BTreeMap<DefinitionId, DefinitionInfo>,
    tags: BTreeMap<TagId, Tag>,
    materials: BTreeMap<MaterialId, Material>,
}
type RecordEdits<K, V> = BTreeMap<K, (Option<V>, Option<V>)>;
#[derive(Clone, Debug, PartialEq)]
struct DefinitionHeader {
    name: String,
    kind: DefinitionKind,
}
#[derive(Clone, Debug, Default)]
struct GraphDelta {
    definitions: RecordEdits<DefinitionId, DefinitionHeader>,
    instances: RecordEdits<(DefinitionId, InstanceId), Instance>,
    tags: RecordEdits<TagId, Tag>,
    materials: RecordEdits<MaterialId, Material>,
    before_next: u64,
    after_next: u64,
}
fn record_edits<K: Ord + Copy, V: Clone + PartialEq>(
    before: &BTreeMap<K, V>,
    after: &BTreeMap<K, V>,
) -> RecordEdits<K, V> {
    before
        .keys()
        .chain(after.keys())
        .copied()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .filter_map(|id| {
            let a = before.get(&id);
            let b = after.get(&id);
            (a != b).then(|| (id, (a.cloned(), b.cloned())))
        })
        .collect()
}
impl GraphDelta {
    fn between(before: &Graph, after: &Graph) -> Self {
        let headers = |graph: &Graph| {
            graph
                .definitions
                .iter()
                .map(|(id, d)| {
                    (
                        *id,
                        DefinitionHeader {
                            name: d.name.clone(),
                            kind: d.kind,
                        },
                    )
                })
                .collect()
        };
        let nodes = |graph: &Graph| {
            graph
                .definitions
                .iter()
                .flat_map(|(id, d)| {
                    d.instances
                        .iter()
                        .map(move |(node, value)| ((*id, *node), value.clone()))
                })
                .collect()
        };
        Self {
            definitions: record_edits(&headers(before), &headers(after)),
            instances: record_edits(&nodes(before), &nodes(after)),
            tags: record_edits(&before.tags, &after.tags),
            materials: record_edits(&before.materials, &after.materials),
            before_next: before.next_id,
            after_next: after.next_id,
        }
    }
    fn apply(&self, graph: &mut Graph, forward: bool) -> Result<(), KernelError> {
        for (id, (before, after)) in &self.definitions {
            if let Some(header) = if forward { after } else { before } {
                let info = graph
                    .definitions
                    .entry(*id)
                    .or_insert_with(|| DefinitionInfo {
                        name: header.name.clone(),
                        kind: header.kind,
                        instances: BTreeMap::new(),
                    });
                info.name = header.name.clone();
                info.kind = header.kind;
            } else {
                graph.definitions.remove(id);
            }
        }
        for ((owner, id), (before, after)) in &self.instances {
            if let Some(instance) = if forward { after } else { before } {
                graph
                    .definitions
                    .get_mut(owner)
                    .ok_or(KernelError::MissingEntity)?
                    .instances
                    .insert(*id, instance.clone());
            } else if let Some(owner) = graph.definitions.get_mut(owner) {
                owner.instances.remove(id);
            }
        }
        fn apply_values<K: Ord + Copy, V: Clone>(
            map: &mut BTreeMap<K, V>,
            edits: &RecordEdits<K, V>,
            forward: bool,
        ) {
            for (id, (before, after)) in edits {
                if let Some(value) = if forward { after } else { before } {
                    map.insert(*id, value.clone());
                } else {
                    map.remove(id);
                }
            }
        }
        apply_values(&mut graph.tags, &self.tags, forward);
        apply_values(&mut graph.materials, &self.materials, forward);
        graph.next_id = if forward {
            self.after_next
        } else {
            self.before_next
        };
        Ok(())
    }
}
#[derive(Clone, Debug)]
enum DocumentEntry {
    Geometry(DefinitionId),
    Organization {
        delta: GraphDelta,
        steps: Vec<DefinitionId>,
    },
}
#[derive(Clone, Debug)]
struct InstancePreview {
    owner: DefinitionId,
    before: Instance,
    after: Instance,
}
/// Runtime document owner with one global undo timeline, shared local geometry and edit context.
#[derive(Clone, Debug)]
pub struct DocumentHistory {
    root: History,
    geometry: BTreeMap<DefinitionId, History>,
    graph: Graph,
    undo: Vec<DocumentEntry>,
    redo: Vec<DocumentEntry>,
    context: InstancePath,
    revision: u64,
    changes: DocumentChanges,
    high_water: u64,
    geometry_preview: Option<DefinitionId>,
    instance_preview: Option<InstancePreview>,
    organization_preview: Option<DocumentEntry>,
    duplicate_preview: Option<(DefinitionId, InstanceId)>,
    pending_cancel: BTreeMap<DefinitionId, ChangeSet>,
    preview_redo: Option<Vec<DocumentEntry>>,
}
fn invalid(message: &str) -> KernelError {
    KernelError::InvalidTopology(message.into())
}
/// Validate a supported nonsingular affine instance transform, including mirrors and nonuniform scales.
pub fn validate_instance_transform(matrix: DMat4) -> Result<(), KernelError> {
    if !matrix.is_finite()
        || matrix.x_axis.w.abs() > 1e-12
        || matrix.y_axis.w.abs() > 1e-12
        || matrix.z_axis.w.abs() > 1e-12
        || (matrix.w_axis.w - 1.).abs() > 1e-12
        || matrix.determinant() == 0.
        || !matrix.inverse().is_finite()
        || matrix
            .to_cols_array()
            .iter()
            .any(|v| v.abs() > MAX_COORDINATE)
    {
        return Err(invalid(
            "instance transform must be finite, affine, nonsingular and within the supported range",
        ));
    }
    Ok(())
}
fn name(value: &str) -> Result<String, KernelError> {
    let s = value.trim();
    if s.is_empty() || s.len() > 1024 || s.chars().any(char::is_control) {
        Err(invalid(
            "name must be nonempty, at most 1024 bytes, without control characters",
        ))
    } else {
        Ok(s.into())
    }
}
fn validate_material(m: &Material) -> Result<(), KernelError> {
    name(&m.name)?;
    if m.color
        .iter()
        .any(|v| !v.is_finite() || !(0.0..=1.).contains(v))
        || !m.opacity.is_finite()
        || !(0.0..=1.).contains(&m.opacity)
        || m.texture
            .as_ref()
            .is_some_and(|s| s.len() > 4096 || s.chars().any(char::is_control))
    {
        return Err(invalid("invalid material color, opacity or texture slot"));
    }
    Ok(())
}

fn validate_material_command(
    command: &Command,
    materials: &BTreeMap<MaterialId, Material>,
    depth: usize,
) -> Result<(), KernelError> {
    if depth > 64 {
        return Err(KernelError::BatchDepth);
    }
    match command {
        Command::SetFaceMaterial {
            material: Some(id), ..
        } if !materials.contains_key(id) => Err(KernelError::MissingEntity),
        Command::Batch(commands) => {
            for c in commands {
                validate_material_command(c, materials, depth + 1)?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

impl Default for DocumentHistory {
    fn default() -> Self {
        let root = DefinitionInfo {
            name: "Document".into(),
            kind: DefinitionKind::Root,
            instances: BTreeMap::new(),
        };
        Self {
            root: History::new(),
            geometry: BTreeMap::new(),
            graph: Graph {
                next_id: 1,
                definitions: BTreeMap::from([(DefinitionId(0), root)]),
                tags: BTreeMap::new(),
                materials: BTreeMap::new(),
            },
            undo: Vec::new(),
            redo: Vec::new(),
            context: Vec::new(),
            revision: 0,
            changes: DocumentChanges::default(),
            high_water: 1,
            geometry_preview: None,
            instance_preview: None,
            organization_preview: None,
            duplicate_preview: None,
            pending_cancel: BTreeMap::new(),
            preview_redo: None,
        }
    }
}
impl Graph {
    fn allocate(&mut self) -> Result<u64, KernelError> {
        let id = self.next_id;
        self.next_id = self.next_id.checked_add(1).ok_or(KernelError::Capacity)?;
        Ok(id)
    }
    fn resolve(
        &self,
        path: &[InstanceId],
    ) -> Result<(DefinitionId, DMat4, Option<MaterialId>), KernelError> {
        let mut definition = DefinitionId(0);
        let mut world = DMat4::IDENTITY;
        let mut material = None;
        if path.len() > 64 {
            return Err(invalid("instance nesting exceeds 64"));
        }
        for id in path {
            let node = self
                .definitions
                .get(&definition)
                .and_then(|d| d.instances.get(id))
                .ok_or(KernelError::MissingEntity)?;
            world *= node.transform;
            material = node.material.or(material);
            definition = node.definition;
        }
        Ok((definition, world, material))
    }
    fn visible_path(&self, path: &[InstanceId]) -> bool {
        let mut definition = DefinitionId(0);
        for id in path {
            let Some(node) = self
                .definitions
                .get(&definition)
                .and_then(|d| d.instances.get(id))
            else {
                return false;
            };
            if node.hidden
                || node
                    .tag
                    .and_then(|id| self.tags.get(&id))
                    .is_some_and(|t| !t.visible)
            {
                return false;
            }
            definition = node.definition;
        }
        true
    }
    fn node(&self, path: &[InstanceId]) -> Result<(DefinitionId, &Instance), KernelError> {
        let (last, parent) = path.split_last().ok_or(KernelError::MissingEntity)?;
        let (owner, _, _) = self.resolve(parent)?;
        Ok((
            owner,
            self.definitions
                .get(&owner)
                .and_then(|d| d.instances.get(last))
                .ok_or(KernelError::MissingEntity)?,
        ))
    }
    fn validate(&self) -> Result<(), KernelError> {
        if self
            .definitions
            .get(&DefinitionId(0))
            .is_none_or(|d| d.kind != DefinitionKind::Root)
        {
            return Err(invalid("missing document root"));
        }
        let mut ids = BTreeSet::new();
        let mut references = BTreeMap::<DefinitionId, usize>::new();
        for (id, d) in &self.definitions {
            name(&d.name)?;
            if *id != DefinitionId(0) && d.kind == DefinitionKind::Root {
                return Err(invalid("only one root is allowed"));
            }
            for (key, node) in &d.instances {
                if *key != node.id
                    || !ids.insert(node.id)
                    || node.definition == DefinitionId(0)
                    || !self.definitions.contains_key(&node.definition)
                {
                    return Err(invalid("invalid or duplicate instance reference"));
                }
                name(&node.name)?;
                validate_instance_transform(node.transform)?;
                if node.tag.is_some_and(|id| !self.tags.contains_key(&id))
                    || node
                        .material
                        .is_some_and(|id| !self.materials.contains_key(&id))
                {
                    return Err(KernelError::MissingEntity);
                }
                *references.entry(node.definition).or_default() += 1;
            }
        }
        for (id, d) in &self.definitions {
            if d.kind == DefinitionKind::Group && references.get(id).copied().unwrap_or(0) > 1 {
                return Err(invalid("groups must have a single structural owner"));
            }
        }
        fn visit(
            graph: &Graph,
            id: DefinitionId,
            active: &mut BTreeSet<DefinitionId>,
            done: &mut BTreeMap<DefinitionId, usize>,
            depth: usize,
        ) -> Result<usize, KernelError> {
            if depth > 64 {
                return Err(invalid("instance nesting exceeds 64"));
            }
            if active.contains(&id) {
                return Err(invalid("recursive component definition"));
            }
            if let Some(height) = done.get(&id) {
                if depth + height > 64 {
                    return Err(invalid("instance nesting exceeds 64"));
                }
                return Ok(*height);
            }
            active.insert(id);
            let mut height = 0;
            for child in graph
                .definitions
                .get(&id)
                .ok_or(KernelError::MissingEntity)?
                .instances
                .values()
            {
                height = height.max(1 + visit(graph, child.definition, active, done, depth + 1)?);
            }
            active.remove(&id);
            done.insert(id, height);
            Ok(height)
        }
        let mut done = BTreeMap::new();
        for id in self.definitions.keys() {
            visit(self, *id, &mut BTreeSet::new(), &mut done, 0)?;
        }
        for tag in self.tags.values() {
            name(&tag.name)?;
        }
        for material in self.materials.values() {
            validate_material(material)?;
        }
        Ok(())
    }
}
impl DocumentHistory {
    /// Construct an empty document.
    pub fn new() -> Self {
        Self::default()
    }
    fn history(&self, id: DefinitionId) -> &History {
        if id == DefinitionId(0) {
            &self.root
        } else {
            self.geometry.get(&id).unwrap_or(&self.root)
        }
    }
    fn history_mut(&mut self, id: DefinitionId) -> Result<&mut History, KernelError> {
        if id == DefinitionId(0) {
            Ok(&mut self.root)
        } else {
            self.geometry.get_mut(&id).ok_or(KernelError::MissingEntity)
        }
    }
    /// Current local read-only geometry.
    pub fn model(&self) -> &Model {
        self.history(self.active_definition()).model()
    }
    /// Definition currently being edited (shared by all matching component instances).
    pub fn active_definition(&self) -> DefinitionId {
        self.graph
            .resolve(&self.context)
            .map_or(DefinitionId(0), |x| x.0)
    }
    /// Active nested occurrence path.
    pub fn context(&self) -> &[InstanceId] {
        &self.context
    }
    /// Local-to-world transform of the active occurrence.
    pub fn world_transform(&self) -> DMat4 {
        self.graph
            .resolve(&self.context)
            .map_or(DMat4::IDENTITY, |x| x.1)
    }
    /// Document revision including transient previews and navigation appearance changes.
    pub fn revision(&self) -> u64 {
        self.revision
    }
    /// Number of committed undo steps across all contexts.
    pub fn undo_depth(&self) -> usize {
        self.undo.len()
    }
    /// Number of available global redo steps.
    pub fn redo_depth(&self) -> usize {
        self.redo.len()
    }
    /// Dirty entities for the active definition, compatible with M4 geometry history.
    pub fn changes(&self) -> &ChangeSet {
        self.changes
            .geometry
            .get(&self.active_definition())
            .unwrap_or_else(|| self.history(self.active_definition()).changes())
    }
    /// Dirty geometry partitions and scene metadata flags.
    pub fn document_changes(&self) -> &DocumentChanges {
        &self.changes
    }
    /// Local geometry for a definition; hidden definitions remain editable through the Outliner.
    pub fn definition_model(&self, id: DefinitionId) -> Option<&Model> {
        self.graph
            .definitions
            .contains_key(&id)
            .then(|| self.history(id).model())
    }
    /// Child instances stored by one definition.
    pub fn instances(&self, id: DefinitionId) -> Option<&BTreeMap<InstanceId, Instance>> {
        self.graph.definitions.get(&id).map(|d| &d.instances)
    }
    /// Capture presentation and visibility without cloning geometry or traversing occurrences.
    pub fn capture_scene(
        &self,
        name: String,
        camera: crate::SceneCamera,
        style: crate::ViewStyle,
    ) -> crate::SavedScene {
        crate::SavedScene {
            sections: Default::default(),
            shadows: Default::default(),
            name,
            camera,
            style,
            tags: self
                .graph
                .tags
                .iter()
                .map(|(id, tag)| (*id, tag.visible))
                .collect(),
            objects: self
                .graph
                .definitions
                .iter()
                .flat_map(|(owner, definition)| {
                    definition
                        .instances
                        .iter()
                        .map(move |(id, instance)| ((*owner, *id), instance.hidden))
                })
                .collect(),
        }
    }

    /// Read-only visibility tags.
    pub fn tags(&self) -> &BTreeMap<TagId, Tag> {
        &self.graph.tags
    }
    /// Read-only material palette.
    pub fn materials(&self) -> &BTreeMap<MaterialId, Material> {
        &self.graph.materials
    }
    /// Human-readable definition name.
    pub fn definition_name(&self, id: DefinitionId) -> Option<&str> {
        self.graph.definitions.get(&id).map(|d| d.name.as_str())
    }
    /// Private/shared ownership of a definition.
    pub fn definition_kind(&self, id: DefinitionId) -> Option<DefinitionKind> {
        self.graph.definitions.get(&id).map(|d| d.kind)
    }
    /// Resolve an instance occurrence.
    pub fn instance(&self, path: &[InstanceId]) -> Option<&Instance> {
        self.graph.node(path).ok().map(|x| x.1)
    }
    fn changed(
        &mut self,
        mut geometry: BTreeMap<DefinitionId, ChangeSet>,
        instances: bool,
        appearance: bool,
    ) {
        for (id, old) in std::mem::take(&mut self.pending_cancel) {
            let changes = geometry.entry(id).or_default();
            changes.rigid_translation = old.rigid_translation
                && (changes.rigid_translation
                    || (changes.vertices.is_empty()
                        && changes.edges.is_empty()
                        && changes.faces.is_empty()));
            changes.vertices.extend(old.vertices);
            changes.edges.extend(old.edges);
            changes.faces.extend(old.faces);
        }
        self.revision = self.revision.saturating_add(1);
        self.changes = DocumentChanges {
            revision: self.revision,
            geometry,
            instances,
            appearance,
        };
    }
    fn geometry_changed(&mut self, id: DefinitionId) {
        let change = self.history(id).changes().clone();
        self.changed(BTreeMap::from([(id, change)]), false, false);
    }
    fn normalize_context(&mut self) {
        while self.graph.resolve(&self.context).is_err() || !self.graph.visible_path(&self.context)
        {
            if self.context.pop().is_none() {
                break;
            }
        }
    }
    /// Enter a valid nested context without adding an undo entry.
    pub fn enter(&mut self, path: InstancePath) -> Result<(), KernelError> {
        self.graph.resolve(&path)?;
        if !self.graph.visible_path(&path) {
            return Err(invalid(
                "unhide the instance and its tags before entering it",
            ));
        }
        self.cancel_preview()?;
        self.context = path;
        self.changed(BTreeMap::new(), true, false);
        Ok(())
    }
    /// Exit one context level, cancelling a provisional edit first.
    pub fn exit(&mut self) -> Result<bool, KernelError> {
        self.cancel_preview()?;
        let result = self.context.pop().is_some();
        if result {
            self.changed(BTreeMap::new(), true, false);
        }
        Ok(result)
    }
    /// Commit a geometry command in the active local definition.
    pub fn execute(&mut self, command: Command) -> Result<bool, KernelError> {
        validate_material_command(&command, &self.graph.materials, 0)?;
        self.cancel_preview()?;
        let id = self.active_definition();
        let changed = self.history_mut(id)?.execute(command)?;
        if changed {
            self.undo.push(DocumentEntry::Geometry(id));
            self.redo.clear();
            self.geometry_changed(id);
        }
        Ok(changed)
    }
    /// Replace a live geometry preview in the active definition, shared by every instance.
    pub fn preview(&mut self, command: Command) -> Result<bool, KernelError> {
        validate_material_command(&command, &self.graph.materials, 0)?;
        if self.instance_preview.is_some() || self.organization_preview.is_some() {
            self.cancel_preview()?;
        }
        let id = self.active_definition();
        let result = self.history_mut(id)?.preview(command);
        self.geometry_preview = Some(id);
        self.geometry_changed(id);
        result
    }
    /// Cancel provisional geometry or an instance placement without touching global history.
    pub fn cancel_preview(&mut self) -> Result<bool, KernelError> {
        if let Some(entry) = self.organization_preview.take() {
            self.duplicate_preview = None;
            if let DocumentEntry::Organization { delta, .. } = &entry {
                for (id, (before, after)) in &delta.definitions {
                    if before.is_none() && after.is_some() {
                        self.geometry.remove(id);
                    }
                }
            }
            self.replay(&entry, false)?;
            if let Some(redo) = self.preview_redo.take() {
                self.redo = redo;
            }
            return Ok(true);
        }

        if let Some(preview) = self.instance_preview.take() {
            self.graph
                .definitions
                .get_mut(&preview.owner)
                .ok_or(KernelError::MissingEntity)?
                .instances
                .insert(preview.before.id, preview.before);
            self.changed(BTreeMap::new(), true, false);
            return Ok(true);
        }
        if let Some(id) = self.geometry_preview.take() {
            let changed = self.history_mut(id)?.cancel_preview()?;
            self.geometry_changed(id);
            self.pending_cancel = self.changes.geometry.clone();
            return Ok(changed);
        }
        Ok(false)
    }
    /// Commit the last provisional edit as exactly one global undo entry.
    pub fn commit_preview(&mut self) -> Result<bool, KernelError> {
        if let Some(entry) = self.organization_preview.take() {
            self.duplicate_preview = None;
            self.preview_redo = None;
            self.undo.push(entry);
            self.redo.clear();
            return Ok(true);
        }

        if let Some(preview) = self.instance_preview.take() {
            if preview.before == preview.after {
                return Ok(false);
            }
            let mut delta = GraphDelta {
                before_next: self.graph.next_id,
                after_next: self.graph.next_id,
                ..GraphDelta::default()
            };
            delta.instances.insert(
                (preview.owner, preview.before.id),
                (Some(preview.before), Some(preview.after)),
            );
            self.undo.push(DocumentEntry::Organization {
                delta,
                steps: Vec::new(),
            });
            self.redo.clear();
            return Ok(true);
        }
        if let Some(id) = self.geometry_preview.take() {
            let changed = self.history_mut(id)?.commit_preview()?;
            if changed {
                self.undo.push(DocumentEntry::Geometry(id));
                self.redo.clear();
            }
            return Ok(changed);
        }
        Ok(false)
    }
    /// Replace a single container transform preview without cloning geometry or the graph.
    pub fn preview_instance_transform(
        &mut self,
        path: &[InstanceId],
        transform: DMat4,
    ) -> Result<bool, KernelError> {
        validate_instance_transform(transform)?;
        if self.geometry_preview.is_some() || self.organization_preview.is_some() {
            self.cancel_preview()?;
        }
        let (owner, node) = self.graph.node(path)?;
        let node = node.clone();
        if self
            .instance_preview
            .as_ref()
            .is_some_and(|p| p.owner != owner || p.before.id != node.id)
        {
            self.cancel_preview()?;
        }
        let original = self
            .instance_preview
            .as_ref()
            .map_or(node.clone(), |p| p.before.clone());
        let mut after = original.clone();
        after.transform = transform;
        self.graph
            .definitions
            .get_mut(&owner)
            .ok_or(KernelError::MissingEntity)?
            .instances
            .insert(after.id, after.clone());
        self.instance_preview = Some(InstancePreview {
            owner,
            before: original.clone(),
            after: after.clone(),
        });
        self.changed(BTreeMap::new(), true, false);
        Ok(original != after)
    }
    /// Preview one copied container, retaining the source and reusing the new instance across frames.
    pub fn preview_duplicate(
        &mut self,
        path: &[InstanceId],
        transform: DMat4,
    ) -> Result<bool, KernelError> {
        validate_instance_transform(transform)?;
        if let Some((owner, id)) = self.duplicate_preview {
            let node = self
                .graph
                .definitions
                .get_mut(&owner)
                .and_then(|d| d.instances.get_mut(&id))
                .ok_or(KernelError::MissingEntity)?;
            node.transform = transform;
            if let Some(DocumentEntry::Organization { delta, .. }) = &mut self.organization_preview
            {
                if let Some((_, Some(instance))) = delta.instances.get_mut(&(owner, id)) {
                    instance.transform = transform;
                }
            }
            self.changed(BTreeMap::new(), true, false);
            return Ok(true);
        }
        self.cancel_preview()?;
        let (owner, _) = self.graph.node(path)?;
        let previous = self
            .instances(owner)
            .ok_or(KernelError::MissingEntity)?
            .keys()
            .copied()
            .collect::<BTreeSet<_>>();
        let redo = std::mem::take(&mut self.redo);
        if let Err(error) = self.organization(OrganizationCommand::Duplicate {
            path: path.to_vec(),
            transform,
        }) {
            self.redo = redo;
            return Err(error);
        }
        self.preview_redo = Some(redo);
        let id = *self
            .instances(owner)
            .ok_or(KernelError::MissingEntity)?
            .keys()
            .find(|id| !previous.contains(id))
            .ok_or(KernelError::MissingEntity)?;
        self.organization_preview = self.undo.pop();
        self.duplicate_preview = Some((owner, id));
        Ok(true)
    }
    /// Traverse visible occurrences, composing parent transforms and nearest inherited materials.
    pub fn occurrences(&self) -> Result<Vec<Occurrence>, KernelError> {
        let mut result = Vec::new();
        let mut stack = vec![(DefinitionId(0), Vec::new(), DMat4::IDENTITY, None)];
        while let Some((definition, path, world, material)) = stack.pop() {
            if result.len() >= 100_000 || path.len() > 64 {
                return Err(invalid("visible instance traversal budget exceeded"));
            }
            if !world.is_finite() || !world.inverse().is_finite() {
                return Err(invalid("composed instance transform is invalid"));
            }
            result.push(Occurrence {
                active: path == self.context,
                path: path.clone(),
                definition,
                world,
                material,
            });
            for node in self
                .graph
                .definitions
                .get(&definition)
                .ok_or(KernelError::MissingEntity)?
                .instances
                .values()
                .rev()
            {
                if node.hidden
                    || node
                        .tag
                        .and_then(|id| self.graph.tags.get(&id))
                        .is_some_and(|t| !t.visible)
                {
                    continue;
                }
                let mut child = path.clone();
                child.push(node.id);
                stack.push((
                    node.definition,
                    child,
                    world * node.transform,
                    node.material.or(material),
                ));
            }
        }
        Ok(result)
    }
    /// Export persistence values only, excluding caches, history and active navigation.
    pub fn to_data(&self) -> DocumentData {
        DocumentData {
            schema_version: SCHEMA_VERSION,
            next_id: self.high_water.max(self.graph.next_id),
            root: DefinitionId(0),
            definitions: self
                .graph
                .definitions
                .iter()
                .map(|(id, d)| {
                    (
                        *id,
                        DefinitionData {
                            name: d.name.clone(),
                            kind: d.kind,
                            geometry: self.history(*id).model().to_data(),
                            instances: d.instances.clone(),
                        },
                    )
                })
                .collect(),
            tags: self.graph.tags.clone(),
            materials: self.graph.materials.clone(),
        }
    }
    /// Validate a schema and rebuild local runtime indices, without importing undo/navigation state.
    pub fn from_data(data: DocumentData) -> Result<Self, KernelError> {
        Self::from_data_impl(data, true)
    }
    /// Rebuild known-topology benchmark data without quadratic intersection validation.
    /// Enabled only for deterministic benchmark fixtures, never for file loading.
    #[cfg(feature = "bench-fixtures")]
    pub fn benchmark_from_data(data: DocumentData) -> Result<Self, KernelError> {
        Self::from_data_impl(data, false)
    }
    fn from_data_impl(data: DocumentData, validate_geometry: bool) -> Result<Self, KernelError> {
        if data.schema_version != SCHEMA_VERSION || data.root != DefinitionId(0) {
            return Err(invalid("unsupported document schema or root"));
        }
        let mut result = Self::new();
        result.graph.definitions.clear();
        result.graph.next_id = data.next_id;
        result.high_water = data.next_id;
        result.graph.tags = data.tags;
        result.graph.materials = data.materials;
        for (id, d) in data.definitions {
            let model = if validate_geometry {
                Model::from_data(d.geometry)?
            } else {
                Model::from_values(d.geometry)?
            };
            if model.faces().values().any(|f| {
                f.material
                    .is_some_and(|id| !result.graph.materials.contains_key(&id))
            }) {
                return Err(invalid("missing face material"));
            }
            let history = History::from_model(model)?;
            if id == DefinitionId(0) {
                result.root = history;
            } else {
                result.geometry.insert(id, history);
            }
            result.graph.definitions.insert(
                id,
                DefinitionInfo {
                    name: d.name,
                    kind: d.kind,
                    instances: d.instances,
                },
            );
        }
        result.graph.validate()?;
        let max = result
            .graph
            .definitions
            .keys()
            .map(|id| id.0)
            .chain(
                result
                    .graph
                    .definitions
                    .values()
                    .flat_map(|d| d.instances.keys().map(|id| id.0)),
            )
            .chain(result.graph.tags.keys().map(|id| id.0))
            .chain(result.graph.materials.keys().map(|id| id.0))
            .max()
            .unwrap_or(0);
        if data.next_id <= max {
            return Err(invalid("invalid document ID allocator"));
        }
        result.occurrences()?;
        Ok(result)
    }
    /// Construct an imported document with exactly one reversible import entry.
    /// Crossings are rejected with bounded validation work, never split during construction.
    pub fn from_import_data(data: DocumentData) -> Result<Self, KernelError> {
        let mut result = Self::from_data_impl(data, false)?;
        for id in result.graph.definitions.keys() {
            result.history(*id).model().validate_mesh(true)?;
        }
        let steps: Vec<_> = result.graph.definitions.keys().copied().collect();
        for id in &steps {
            result.history_mut(*id)?.record_import();
        }
        let delta = GraphDelta::between(&Self::new().graph, &result.graph);
        result
            .undo
            .push(DocumentEntry::Organization { delta, steps });
        result.revision = 1;
        result.changes.instances = true;
        Ok(result)
    }
    /// Undo the most recent committed edit regardless of the current edit context.
    pub fn undo(&mut self) -> Result<bool, KernelError> {
        self.cancel_preview()?;
        let Some(entry) = self.undo.pop() else {
            return Ok(false);
        };
        self.replay(&entry, false)?;
        self.redo.push(entry);
        Ok(true)
    }
    /// Redo the most recently undone global edit.
    pub fn redo(&mut self) -> Result<bool, KernelError> {
        self.cancel_preview()?;
        let Some(entry) = self.redo.pop() else {
            return Ok(false);
        };
        self.replay(&entry, true)?;
        self.undo.push(entry);
        Ok(true)
    }
    fn replay(&mut self, entry: &DocumentEntry, forward: bool) -> Result<(), KernelError> {
        let mut dirty = BTreeMap::new();
        match entry {
            DocumentEntry::Geometry(id) => {
                if forward {
                    self.history_mut(*id)?.redo()?;
                } else {
                    self.history_mut(*id)?.undo()?;
                }
                dirty.insert(*id, self.history(*id).changes().clone());
                self.changed(dirty, false, false);
            }
            DocumentEntry::Organization { delta, steps } => {
                if forward {
                    for id in steps {
                        self.history_mut(*id)?.redo()?;
                        dirty.insert(*id, self.history(*id).changes().clone());
                    }
                } else {
                    for id in steps.iter().rev() {
                        self.history_mut(*id)?.undo()?;
                        dirty.insert(*id, self.history(*id).changes().clone());
                    }
                }
                delta.apply(&mut self.graph, forward)?;
                self.changed(dirty, true, !delta.materials.is_empty());
            }
        }
        self.normalize_context();
        Ok(())
    }
}

impl DocumentHistory {
    fn clone_definition(
        &self,
        source: DefinitionId,
        graph: &mut Graph,
        created: &mut BTreeMap<DefinitionId, Model>,
    ) -> Result<DefinitionId, KernelError> {
        let mut info = self
            .graph
            .definitions
            .get(&source)
            .ok_or(KernelError::MissingEntity)?
            .clone();
        let id = DefinitionId(graph.allocate()?);
        let mut children = BTreeMap::new();
        for mut child in info.instances.into_values() {
            child.id = InstanceId(graph.allocate()?);
            if self.definition_kind(child.definition) == Some(DefinitionKind::Group) {
                child.definition = self.clone_definition(child.definition, graph, created)?;
            }
            children.insert(child.id, child);
        }
        info.instances = children;
        created.insert(id, self.history(source).model().clone());
        graph.definitions.insert(id, info);
        Ok(id)
    }
    /// Apply a document-level organization command as one global undo step.
    pub fn organization(&mut self, command: OrganizationCommand) -> Result<bool, KernelError> {
        self.cancel_preview()?;
        if let OrganizationCommand::PaintFaces { faces, material } = &command {
            if material.is_some_and(|id| !self.graph.materials.contains_key(&id)) {
                return Err(KernelError::MissingEntity);
            }
            return self.execute(Command::SetFaceMaterial {
                faces: faces.clone(),
                material: *material,
            });
        }
        let before = self.graph.clone();
        let mut after = before.clone();
        after.next_id = after.next_id.max(self.high_water);
        let mut created = BTreeMap::<DefinitionId, Model>::new();
        let mut replacements = Vec::new();
        let mut commands = Vec::new();
        let active = self.active_definition();
        let node_mut = |graph: &mut Graph,
                        path: &[InstanceId]|
         -> Result<(DefinitionId, Instance), KernelError> {
            let (owner, node) = graph.node(path)?;
            Ok((owner, node.clone()))
        };
        match command {
            OrganizationCommand::Create {
                kind,
                name: label,
                vertices,
            } => {
                if kind == DefinitionKind::Root {
                    return Err(invalid("cannot create another root"));
                }
                let label = name(&label)?;
                let selected: BTreeSet<_> = vertices.into_iter().collect();
                if selected.is_empty() {
                    return Err(invalid("select connected geometry first"));
                }
                let model = self.model();
                for id in &selected {
                    if !model.vertices.contains_key(id) {
                        return Err(KernelError::MissingEntity);
                    }
                }
                if model.edges.values().any(|e| {
                    e.vertices.iter().any(|v| selected.contains(v))
                        && !e.vertices.iter().all(|v| selected.contains(v))
                }) {
                    return Err(invalid("grouping requires complete connected geometry"));
                }
                let mut child = Model {
                    next_id: model.next_id,
                    ..Model::default()
                };
                let mut remaining = model.clone();
                remaining.spatial = None;
                child.vertices = model
                    .vertices
                    .iter()
                    .filter(|(id, _)| selected.contains(id))
                    .map(|(id, v)| (*id, v.clone()))
                    .collect();
                child.edges = model
                    .edges
                    .iter()
                    .filter(|(_, e)| e.vertices.iter().all(|v| selected.contains(v)))
                    .map(|(id, e)| (*id, e.clone()))
                    .collect();
                child.faces = model
                    .faces
                    .iter()
                    .filter(|(_, f)| {
                        f.outer
                            .iter()
                            .chain(f.holes.iter().flatten())
                            .all(|v| selected.contains(v))
                    })
                    .map(|(id, f)| (*id, f.clone()))
                    .collect();
                remaining
                    .vertices
                    .retain(|id, _| !child.vertices.contains_key(id));
                remaining
                    .edges
                    .retain(|id, _| !child.edges.contains_key(id));
                remaining
                    .faces
                    .retain(|id, _| !child.faces.contains_key(id));
                remaining.validate()?;
                let (min, max) = child.bounds().ok_or(KernelError::MissingEntity)?;
                let origin = (min + max) * 0.5;
                let ids = child.vertices.keys().copied().collect::<Vec<_>>();
                crate::modification::transform(
                    &mut child,
                    &ids,
                    DMat4::from_translation(-origin),
                    false,
                )?;
                child.validate()?;
                let definition = DefinitionId(after.allocate()?);
                let id = InstanceId(after.allocate()?);
                after.definitions.insert(
                    definition,
                    DefinitionInfo {
                        name: label.clone(),
                        kind,
                        instances: BTreeMap::new(),
                    },
                );
                after
                    .definitions
                    .get_mut(&active)
                    .ok_or(KernelError::MissingEntity)?
                    .instances
                    .insert(
                        id,
                        Instance {
                            id,
                            name: label,
                            definition,
                            transform: DMat4::from_translation(origin),
                            tag: None,
                            material: None,
                            hidden: false,
                        },
                    );
                created.insert(definition, child);
                replacements.push((active, remaining));
            }
            OrganizationCommand::AddInstance {
                definition,
                name: label,
                transform,
            } => {
                if self.definition_kind(definition) != Some(DefinitionKind::Component) {
                    return Err(invalid(
                        "only component definitions may be shared; duplicate groups instead",
                    ));
                }
                validate_instance_transform(transform)?;
                let id = InstanceId(after.allocate()?);
                after
                    .definitions
                    .get_mut(&active)
                    .ok_or(KernelError::MissingEntity)?
                    .instances
                    .insert(
                        id,
                        Instance {
                            id,
                            name: name(&label)?,
                            definition,
                            transform,
                            tag: None,
                            material: None,
                            hidden: false,
                        },
                    );
            }
            OrganizationCommand::Duplicate { path, transform } => {
                validate_instance_transform(transform)?;
                let (owner, mut node) = node_mut(&mut after, &path)?;
                node.id = InstanceId(after.allocate()?);
                node.transform = transform;
                node.name = format!("{} copy", node.name);
                if self.definition_kind(node.definition) == Some(DefinitionKind::Group) {
                    node.definition =
                        self.clone_definition(node.definition, &mut after, &mut created)?;
                }
                after
                    .definitions
                    .get_mut(&owner)
                    .ok_or(KernelError::MissingEntity)?
                    .instances
                    .insert(node.id, node);
            }
            OrganizationCommand::Delete(path) => {
                let (owner, node) = node_mut(&mut after, &path)?;
                after
                    .definitions
                    .get_mut(&owner)
                    .ok_or(KernelError::MissingEntity)?
                    .instances
                    .remove(&node.id);
            }
            OrganizationCommand::MakeUnique(path) => {
                let (owner, mut node) = node_mut(&mut after, &path)?;
                node.definition =
                    self.clone_definition(node.definition, &mut after, &mut created)?;
                after
                    .definitions
                    .get_mut(&owner)
                    .ok_or(KernelError::MissingEntity)?
                    .instances
                    .insert(node.id, node);
            }
            OrganizationCommand::Rename { path, name: label } => {
                let (owner, mut node) = node_mut(&mut after, &path)?;
                node.name = name(&label)?;
                after
                    .definitions
                    .get_mut(&owner)
                    .ok_or(KernelError::MissingEntity)?
                    .instances
                    .insert(node.id, node);
            }
            OrganizationCommand::Hide { path, hidden } => {
                let (owner, mut node) = node_mut(&mut after, &path)?;
                node.hidden = hidden;
                after
                    .definitions
                    .get_mut(&owner)
                    .ok_or(KernelError::MissingEntity)?
                    .instances
                    .insert(node.id, node);
            }
            OrganizationCommand::AssignTag { path, tag } => {
                if tag.is_some_and(|id| !after.tags.contains_key(&id)) {
                    return Err(KernelError::MissingEntity);
                }
                let (owner, mut node) = node_mut(&mut after, &path)?;
                node.tag = tag;
                after
                    .definitions
                    .get_mut(&owner)
                    .ok_or(KernelError::MissingEntity)?
                    .instances
                    .insert(node.id, node);
            }
            OrganizationCommand::PaintInstance { path, material } => {
                if material.is_some_and(|id| !after.materials.contains_key(&id)) {
                    return Err(KernelError::MissingEntity);
                }
                let (owner, mut node) = node_mut(&mut after, &path)?;
                node.material = material;
                after
                    .definitions
                    .get_mut(&owner)
                    .ok_or(KernelError::MissingEntity)?
                    .instances
                    .insert(node.id, node);
            }
            OrganizationCommand::Transform { path, transform } => {
                validate_instance_transform(transform)?;
                let (owner, mut node) = node_mut(&mut after, &path)?;
                node.transform = transform;
                after
                    .definitions
                    .get_mut(&owner)
                    .ok_or(KernelError::MissingEntity)?
                    .instances
                    .insert(node.id, node);
            }
            OrganizationCommand::Reparent { path, parent } => {
                if path.is_empty() || parent.starts_with(&path) {
                    return Err(invalid("cannot reparent an instance inside itself"));
                }
                let (_, world, _) = after.resolve(&path)?;
                let (destination, parent_world, _) = after.resolve(&parent)?;
                let (owner, mut node) = node_mut(&mut after, &path)?;
                if owner == destination {
                    return Ok(false);
                }
                node.transform = parent_world.inverse() * world;
                validate_instance_transform(node.transform)?;
                after
                    .definitions
                    .get_mut(&owner)
                    .ok_or(KernelError::MissingEntity)?
                    .instances
                    .remove(&node.id);
                after
                    .definitions
                    .get_mut(&destination)
                    .ok_or(KernelError::MissingEntity)?
                    .instances
                    .insert(node.id, node);
            }
            OrganizationCommand::CreateTag(label) => {
                let id = TagId(after.allocate()?);
                after.tags.insert(
                    id,
                    Tag {
                        name: name(&label)?,
                        visible: true,
                    },
                );
            }
            OrganizationCommand::RenameTag { id, name: label } => {
                after
                    .tags
                    .get_mut(&id)
                    .ok_or(KernelError::MissingEntity)?
                    .name = name(&label)?;
            }
            OrganizationCommand::DeleteTag(id) => {
                after.tags.remove(&id).ok_or(KernelError::MissingEntity)?;
                for d in after.definitions.values_mut() {
                    for node in d.instances.values_mut() {
                        if node.tag == Some(id) {
                            node.tag = None;
                        }
                    }
                }
            }
            OrganizationCommand::TagVisibility { id, visible } => {
                after
                    .tags
                    .get_mut(&id)
                    .ok_or(KernelError::MissingEntity)?
                    .visible = visible;
            }
            OrganizationCommand::RestoreVisibility { tags, objects } => {
                for (id, visible) in tags {
                    if let Some(tag) = after.tags.get_mut(&id) {
                        tag.visible = visible;
                    }
                }
                for ((owner, id), hidden) in objects {
                    if let Some(instance) = after
                        .definitions
                        .get_mut(&owner)
                        .and_then(|d| d.instances.get_mut(&id))
                    {
                        instance.hidden = hidden;
                    }
                }
            }
            OrganizationCommand::CreateMaterial(material) => {
                validate_material(&material)?;
                let id = MaterialId(after.allocate()?);
                after.materials.insert(id, material);
            }
            OrganizationCommand::UpdateMaterial { id, material } => {
                validate_material(&material)?;
                *after
                    .materials
                    .get_mut(&id)
                    .ok_or(KernelError::MissingEntity)? = material;
            }
            OrganizationCommand::DeleteMaterial(id) => {
                after
                    .materials
                    .remove(&id)
                    .ok_or(KernelError::MissingEntity)?;
                for d in after.definitions.values_mut() {
                    for node in d.instances.values_mut() {
                        if node.material == Some(id) {
                            node.material = None;
                        }
                    }
                }
                for def in after.definitions.keys() {
                    let faces = self
                        .history(*def)
                        .model()
                        .faces()
                        .iter()
                        .filter_map(|(face, f)| (f.material == Some(id)).then_some(*face))
                        .collect::<Vec<_>>();
                    if !faces.is_empty() {
                        commands.push((
                            *def,
                            Command::SetFaceMaterial {
                                faces,
                                material: None,
                            },
                        ));
                    }
                }
            }
            OrganizationCommand::PaintFaces { .. } => {
                return Err(invalid("unreachable face material dispatch"))
            }
        }
        after.validate()?;
        if after == before && replacements.is_empty() && commands.is_empty() {
            return Ok(false);
        }
        // Geometry is prepared/validated before metadata becomes visible. Undo any successful
        // subordinate steps if a later preparation fails, preserving atomic organization edits.
        let mut steps = Vec::new();
        let mut dirty = BTreeMap::new();
        for (id, model) in replacements {
            match self.history_mut(id)?.replace_model(model) {
                Ok(true) => {
                    steps.push(id);
                    dirty.insert(id, self.history(id).changes().clone());
                }
                Ok(false) => {}
                Err(error) => {
                    for id in steps.iter().rev() {
                        self.history_mut(*id)?.undo()?;
                    }
                    return Err(error);
                }
            }
        }
        for (id, command) in commands {
            match self.history_mut(id)?.execute(command) {
                Ok(true) => {
                    steps.push(id);
                    dirty.insert(id, self.history(id).changes().clone());
                }
                Ok(false) => {}
                Err(error) => {
                    for id in steps.iter().rev() {
                        self.history_mut(*id)?.undo()?;
                    }
                    return Err(error);
                }
            }
        }
        for (id, model) in created {
            let changes = ChangeSet {
                rigid_translation: false,
                revision: 0,
                vertices: model.vertices.keys().copied().collect(),
                edges: model.edges.keys().copied().collect(),
                faces: model.faces.keys().copied().collect(),
            };
            self.geometry.insert(id, History::from_model(model)?);
            dirty.insert(id, changes);
        }
        let appearance = before.materials != after.materials;
        self.high_water = self.high_water.max(after.next_id);
        self.graph = after.clone();
        self.undo.push(DocumentEntry::Organization {
            delta: GraphDelta::between(&before, &after),
            steps,
        });
        self.redo.clear();
        self.normalize_context();
        // Palette edits affect only faces actually referring to changed materials; inherited
        // instance materials are handled by the transform/material instance batch.
        for id in self.graph.definitions.keys().filter(|_| appearance) {
            let assigned = self
                .history(*id)
                .model()
                .faces()
                .iter()
                .filter_map(|(face, f)| f.material.map(|_| *face))
                .collect::<BTreeSet<_>>();
            if !assigned.is_empty() {
                dirty.entry(*id).or_default().faces.extend(assigned);
            }
        }
        self.changed(dirty, true, appearance);
        Ok(true)
    }
}

impl DocumentHistory {
    /// Append validated imported definitions/geometry to the active context as one undo entry.
    /// Intended for an import worker operating on a cloned history. Existing undo/redo is retained.
    pub fn append_import(&mut self, data: DocumentData) -> Result<(), KernelError> {
        if self.geometry_preview.is_some()
            || self.instance_preview.is_some()
            || self.organization_preview.is_some()
        {
            return Err(invalid("finish the active preview before importing"));
        }
        let imported = Self::from_import_data(data)?;
        let mut after = self.graph.clone();
        after.next_id = after.next_id.max(self.high_water);
        let mut alloc = || {
            let id = after.next_id;
            after.next_id += 1;
            id
        };
        let definitions: BTreeMap<_, _> = imported
            .graph
            .definitions
            .keys()
            .filter(|id| **id != DefinitionId(0))
            .map(|id| (*id, DefinitionId(alloc())))
            .collect();
        let materials: BTreeMap<_, _> = imported
            .graph
            .materials
            .keys()
            .map(|id| (*id, MaterialId(alloc())))
            .collect();
        let tags: BTreeMap<_, _> = imported
            .graph
            .tags
            .keys()
            .map(|id| (*id, TagId(alloc())))
            .collect();
        for (id, m) in &imported.graph.materials {
            after.materials.insert(materials[id], m.clone());
        }
        for (id, t) in &imported.graph.tags {
            after.tags.insert(tags[id], t.clone());
        }
        let mut created = BTreeMap::new();
        let active = self.active_definition();
        for (id, info) in &imported.graph.definitions {
            let target = if *id == DefinitionId(0) {
                active
            } else {
                definitions[id]
            };
            let mut info = info.clone();
            for node in info.instances.values_mut() {
                node.id = InstanceId(after.next_id);
                after.next_id += 1;
                node.definition = *definitions
                    .get(&node.definition)
                    .ok_or(KernelError::MissingEntity)?;
                node.material = node.material.map(|id| materials[&id]);
                node.tag = node.tag.map(|id| tags[&id]);
            }
            info.instances = info.instances.into_values().map(|n| (n.id, n)).collect();
            if *id == DefinitionId(0) {
                after
                    .definitions
                    .get_mut(&target)
                    .ok_or(KernelError::MissingEntity)?
                    .instances
                    .extend(info.instances);
            } else {
                after.definitions.insert(target, info);
                let mut m = imported.history(*id).model().clone();
                for face in m.faces.values_mut() {
                    face.material = face.material.map(|id| materials[&id]);
                }
                created.insert(target, m);
            }
        }
        after.validate()?;
        let mut checker = Self::new();
        checker.graph = after.clone();
        checker.occurrences()?;
        let source = imported.model();
        let root_replacement = if source.vertices().is_empty() {
            None
        } else {
            // Preserve existing IDs and the geometry allocator's branch high-water mark.
            // Only incoming positions are interned; existing topology is never renumbered.
            let mut model = self.model().clone();
            model.next_id = model.next_id.max(self.history(active).allocation_floor());
            model.ensure_spatial()?;
            let mut vertices = BTreeMap::new();
            for (id, v) in source.vertices() {
                vertices.insert(*id, model.intern(v.position)?);
            }
            let mut pairs: BTreeMap<_, _> = model
                .edges
                .iter()
                .map(|(id, e)| (e.vertices, *id))
                .collect();
            for edge in source.edges().values() {
                let mut pair = edge.vertices.map(|v| vertices[&v]);
                pair.sort();
                if pair[0] == pair[1] {
                    continue;
                }
                if let Some(id) = pairs.get(&pair) {
                    let e = model.edges.get_mut(id).ok_or(KernelError::MissingEntity)?;
                    e.soft |= edge.soft;
                    e.smooth |= edge.smooth;
                } else {
                    let id = EdgeId::from_raw(model.allocate()?);
                    model.edges.insert(
                        id,
                        Edge {
                            vertices: pair,
                            soft: edge.soft,
                            smooth: edge.smooth,
                        },
                    );
                    pairs.insert(pair, id);
                }
            }
            let mut keys: BTreeSet<_> = model
                .faces
                .values()
                .map(crate::topology::face_key)
                .collect();
            for face in source.faces().values() {
                let mut face = face.clone();
                face.outer = face.outer.iter().map(|v| vertices[v]).collect();
                face.holes = face
                    .holes
                    .iter()
                    .map(|r| r.iter().map(|v| vertices[v]).collect())
                    .collect();
                face.material = face.material.map(|id| materials[&id]);
                if keys.insert(crate::topology::face_key(&face)) {
                    let id = FaceId::from_raw(model.allocate()?);
                    model.faces.insert(id, face);
                }
            }
            model.spatial = None;
            model.validate()?;
            Some(model)
        };
        let mut steps = Vec::new();
        // Every operation below is on prepared values. Derived index construction is already done.
        if let Some(model) = root_replacement {
            if self.history_mut(active)?.replace_import_model(model)? {
                steps.push(active);
            }
        }
        for (id, model) in created {
            self.geometry.insert(id, History::from_model(model)?);
        }
        let delta = GraphDelta::between(&self.graph, &after);
        self.graph = after;
        self.high_water = self.high_water.max(self.graph.next_id);
        let dirty = steps
            .iter()
            .map(|id| (*id, self.history(*id).changes().clone()))
            .collect();
        self.undo.push(DocumentEntry::Organization { delta, steps });
        self.redo.clear();
        self.changed(dirty, true, true);
        Ok(())
    }
}
