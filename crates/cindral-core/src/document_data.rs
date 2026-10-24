//! Versioned, serializable document values. No history, spatial index or GPU resources.
use crate::{Edge, EdgeId, Face, FaceId, KernelError, Model, Vertex, VertexId};
use glam::DMat4;
use std::collections::BTreeMap;
/// Current native document schema, before M6 container encoding/migrations.
pub const SCHEMA_VERSION: u32 = 2;
macro_rules! id {
    ($name:ident,$doc:literal) => {
        #[doc=$doc]
        #[derive(
            Clone,
            Copy,
            Debug,
            Default,
            PartialEq,
            Eq,
            PartialOrd,
            Ord,
            Hash,
            serde::Serialize,
            serde::Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name(#[doc = "Numeric document-local identity."] pub u64);
    };
}
id!(
    DefinitionId,
    "Document-local geometry definition identity; zero is the root."
);
id!(
    InstanceId,
    "Document-local instance identity, disambiguated by its ancestor path."
);
id!(TagId, "Document visibility tag identity.");
id!(MaterialId, "Document material identity.");
/// A nested occurrence path starting at the document root.
pub type InstancePath = Vec<InstanceId>;
/// Definition ownership semantics.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum DefinitionKind {
    /// Root-only container.
    Root,
    /// Private geometry owned by one structural instance.
    Group,
    /// Shared editable definition.
    Component,
}
/// Geometry values without runtime indices/history. IDs survive a save/load round trip.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct GeometryData {
    /// Stable vertex values.
    pub vertices: BTreeMap<VertexId, Vertex>,
    /// Stable edge values.
    pub edges: BTreeMap<EdgeId, Edge>,
    /// Planar surface values.
    pub faces: BTreeMap<FaceId, Face>,
    /// Next geometry identity, greater than all entity IDs.
    pub next_id: u64,
}
impl Model {
    /// Export only serializable geometry values, excluding all derived caches.
    pub fn to_data(&self) -> GeometryData {
        GeometryData {
            vertices: self.vertices.clone(),
            edges: self.edges.clone(),
            faces: self.faces.clone(),
            next_id: self.next_id,
        }
    }
    /// Validate serializable geometry and reconstruct its runtime indices.
    pub fn from_data(data: GeometryData) -> Result<Self, KernelError> {
        let mut model = Self::from_values(data)?;
        model.validate()?;
        model.ensure_spatial()?;
        Ok(model)
    }
    pub(crate) fn from_values(data: GeometryData) -> Result<Self, KernelError> {
        let max = data
            .vertices
            .keys()
            .map(|id| id.value())
            .chain(data.edges.keys().map(|id| id.value()))
            .chain(data.faces.keys().map(|id| id.value()))
            .max()
            .unwrap_or(0);
        if data.next_id <= max {
            return Err(KernelError::InvalidTopology(
                "invalid geometry ID allocator".into(),
            ));
        }
        Ok(Self {
            vertices: data.vertices,
            edges: data.edges,
            faces: data.faces,
            next_id: data.next_id,
            ..Self::default()
        })
    }
}
/// Placed reference to a definition, local to its owning definition.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Instance {
    /// Stable identity.
    pub id: InstanceId,
    /// Human-readable Outliner name.
    pub name: String,
    /// Shared/private geometry source.
    pub definition: DefinitionId,
    /// Finite nonsingular affine local-to-parent transform.
    pub transform: DMat4,
    /// Optional visibility tag.
    pub tag: Option<TagId>,
    /// Material inherited by unpainted descendant faces.
    pub material: Option<MaterialId>,
    /// Per-instance visibility, independent of tags.
    pub hidden: bool,
}
/// Named visibility layer. Tags are assigned to containers to keep loose topology coherent.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Tag {
    /// Human-readable name.
    pub name: String,
    /// Whether tagged instances are visible.
    pub visible: bool,
}
/// Material values; texture loading is intentionally deferred to M6.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Material {
    /// Human-readable palette name.
    pub name: String,
    /// Display sRGB albedo bytes; lighting converts to linear light.
    /// Schema 1 stored linear floats; [`material_color_from_legacy`] upgrades
    /// them on load so old files keep their exact rendered look.
    #[serde(deserialize_with = "material_color_from_legacy")]
    pub color: [u8; 3],
    /// Opacity in 0..=1.
    pub opacity: f32,
    /// Optional document-relative texture reference.
    pub texture: Option<String>,
}

/// Deserializes material color as current sRGB bytes or schema-1 linear floats.
fn material_color_from_legacy<'de, D>(deserializer: D) -> Result<[u8; 3], D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(serde::Deserialize)]
    #[serde(untagged)]
    enum Repr {
        Current([u8; 3]),
        Linear([f32; 3]),
    }
    use serde::Deserialize as _;
    match Repr::deserialize(deserializer)? {
        Repr::Current(color) => Ok(color),
        Repr::Linear(linear) => Ok(crate::color::linear_rgb_to_srgb(linear)),
    }
}
/// Serializable geometry definition and nested instance tree.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct DefinitionData {
    /// Definition name.
    pub name: String,
    /// Private/shared ownership.
    pub kind: DefinitionKind,
    /// Local geometry.
    pub geometry: GeometryData,
    /// Child references local to this definition.
    pub instances: BTreeMap<InstanceId, Instance>,
}
/// Complete versioned persistence boundary; no runtime state is serialized.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct DocumentData {
    /// Schema identifier used by future .cdraft migrations.
    pub schema_version: u32,
    /// Shared document identity allocator.
    pub next_id: u64,
    /// Root definition identity.
    pub root: DefinitionId,
    /// Geometry definitions, stored once irrespective of instance count.
    pub definitions: BTreeMap<DefinitionId, DefinitionData>,
    /// Named visibility tags.
    pub tags: BTreeMap<TagId, Tag>,
    /// Material palette.
    pub materials: BTreeMap<MaterialId, Material>,
}
