//! Runtime organization and cross-context command history.
use crate::*;
use glam::DMat4;
use std::collections::BTreeMap;
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
#[allow(missing_docs)]
pub enum OrganizationCommand {
    /// Move selected complete geometry into a new group/component in the active definition.
    Create {
        kind: DefinitionKind,
        name: String,
        vertices: Vec<VertexId>,
    },
    /// Add a placed reference to a component definition in the active definition.
    AddInstance {
        definition: DefinitionId,
        name: String,
        transform: DMat4,
    },
    /// Break one instance's definition-sharing link (including its private nested groups).
    MakeUnique(InstancePath),
    /// Change an instance name.
    Rename { path: InstancePath, name: String },
    /// Set explicit visibility.
    Hide { path: InstancePath, hidden: bool },
    /// Assign or clear a container tag.
    AssignTag {
        path: InstancePath,
        tag: Option<TagId>,
    },
    /// Apply an inherited container material.
    PaintInstance {
        path: InstancePath,
        material: Option<MaterialId>,
    },
    /// Apply explicit face material references in the active geometry.
    PaintFaces {
        faces: Vec<FaceId>,
        material: Option<MaterialId>,
    },
    /// Set a local transform.
    Transform {
        path: InstancePath,
        transform: DMat4,
    },
    /// Move an occurrence into a new parent while preserving its world placement.
    Reparent {
        path: InstancePath,
        parent: InstancePath,
    },
    /// Create a tag (returns its ID through the document maps).
    CreateTag(String),
    /// Rename a tag.
    RenameTag { id: TagId, name: String },
    /// Remove a tag and clear all references.
    DeleteTag(TagId),
    /// Set tag visibility.
    TagVisibility { id: TagId, visible: bool },
    /// Create a material.
    CreateMaterial(Material),
    /// Replace material values.
    UpdateMaterial { id: MaterialId, material: Material },
    /// Remove a material and clear all references.
    DeleteMaterial(MaterialId),
}
