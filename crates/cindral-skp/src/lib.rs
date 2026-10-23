//! Independent, bounded, read-only SketchUp container reader.
#![forbid(unsafe_code)]
use cindral_core::{BulkGeometry, Material, Tag};
use glam::DMat4;
use std::collections::BTreeMap;
mod container;
pub use container::read;
mod legacy;
mod modern;
/// Maximum source/expanded payload size.
pub const MAX_BYTES: usize = 64 * 1024 * 1024;
/// Maximum aggregate geometry records.
pub const MAX_ITEMS: usize = 1_000_000;
/// Maximum object/TLV nesting.
pub const MAX_DEPTH: usize = 64;
/// Reader failure, including source version when available.
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct Error(pub String);
/// Reader result.
pub type Result<T> = std::result::Result<T, Error>;
/// Shared local geometry; coordinates remain inches and Z up.
#[derive(Debug, Default)]
pub struct Definition {
    /// Source label.
    pub name: String,
    /// Authored polygons and edge styles.
    pub geometry: BulkGeometry,
    /// Nested placements.
    pub instances: Vec<Instance>,
    /// Face index to explicit tag/hidden state; absent means inherited and visible.
    pub face_assignments: BTreeMap<usize, (Option<usize>, bool)>,
    /// Source entity IDs keyed by authored face index (when available).
    pub face_ids: BTreeMap<usize, u64>,
    /// Explicit loose-edge tag assignments, indexed in geometry.edges.
    pub edge_tags: BTreeMap<usize, usize>,
}
/// Placement of a shared definition.
#[derive(Debug)]
pub struct Instance {
    /// Source label.
    pub name: String,
    /// Zero-based definition index.
    pub definition: usize,
    /// Affine local-to-parent placement, translations in inches.
    pub transform: DMat4,
    /// Zero-based tag index.
    pub tag: Option<usize>,
    /// Instance paint, palette index plus one.
    pub material: Option<cindral_core::MaterialId>,
    /// User-hidden placement.
    pub hidden: bool,
}
/// Parsed source scene, independent of IO/UI.
#[derive(Debug, Default)]
pub struct Scene {
    /// Header version.
    pub version: String,
    /// Shared definitions.
    pub definitions: Vec<Definition>,
    /// Root placements.
    pub roots: Vec<Instance>,
    /// Palette; face material references use index plus one.
    pub materials: Vec<Material>,
    /// Embedded encoded texture images, keyed by material texture reference.
    pub textures: BTreeMap<String, Vec<u8>>,
    /// Visibility tags.
    pub tags: Vec<Tag>,
    /// Bounded diagnostics.
    pub warnings: Vec<String>,
    /// Skipped records.
    pub skipped: usize,
}
impl Scene {
    /// Record a skipped/unsupported item without unbounded diagnostic growth.
    pub fn warn(&mut self, message: impl Into<String>) {
        self.skipped = self.skipped.saturating_add(1);
        if self.warnings.len() < 256 {
            self.warnings
                .push(message.into().chars().take(1024).collect());
        }
    }
}
