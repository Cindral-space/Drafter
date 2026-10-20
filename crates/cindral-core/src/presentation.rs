//! Serializable presentation values independent of GPU and windowing APIs.
use crate::{DefinitionId, InstanceId, TagId};
use std::collections::BTreeMap;
/// Face and edge display mode.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum StyleMode {
    /// Lit material colors.
    Shaded,
    /// Lit colors and material images.
    #[default]
    Textured,
    /// All edges without face occlusion.
    Wireframe,
    /// Background-colored faces occlude rear edges.
    HiddenLine,
    /// Translucent faces and visible rear edges.
    XRay,
}
/// Viewport appearance; changing this never invalidates geometry.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ViewStyle {
    /// Surface mode.
    pub mode: StyleMode,
    /// Emphasize boundary profiles.
    pub profiles: bool,
    /// Display soft edges.
    pub soft_edges: bool,
    /// Display smooth edges.
    pub smooth_edges: bool,
    /// Background sRGB RGB bytes.
    pub background: [u8; 3],
    /// Optional sky color above the horizon.
    pub sky: Option<[u8; 3]>,
    /// Optional ground color below the horizon.
    pub ground: Option<[u8; 3]>,
}
impl Default for ViewStyle {
    fn default() -> Self {
        Self {
            mode: StyleMode::Textured,
            profiles: false,
            soft_edges: false,
            smooth_edges: false,
            background: [17, 22, 29],
            sky: None,
            ground: None,
        }
    }
}
/// Camera values in meters/radians, Z up.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SceneCamera {
    /// Orbit target.
    pub target: [f32; 3],
    /// Distance to target.
    pub distance: f32,
    /// Orbit yaw.
    pub yaw: f32,
    /// Orbit pitch.
    pub pitch: f32,
    /// Orthographic rather than perspective projection.
    pub orthographic: bool,
}
/// Named snapshot. Missing/deleted IDs are ignored on restore; new objects retain visibility.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SavedScene {
    /// User-visible name.
    pub name: String,
    /// Camera at capture time.
    pub camera: SceneCamera,
    /// Tag visibility by stable ID.
    pub tags: BTreeMap<TagId, bool>,
    /// Explicit hidden flags by owning definition and instance ID (shared definitions agree).
    pub objects: BTreeMap<(DefinitionId, InstanceId), bool>,
    /// Captured appearance.
    pub style: ViewStyle,
}
/// Document presentation persisted alongside topology.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Presentation {
    /// Current style.
    pub style: ViewStyle,
    /// Ordered named scenes.
    pub scenes: Vec<SavedScene>,
}
