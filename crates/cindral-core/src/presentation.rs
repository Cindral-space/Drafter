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
    /// Section planes and active cut.
    #[serde(default)]
    pub sections: SectionState,
    /// Geographic sunlight settings.
    #[serde(default)]
    pub shadows: ShadowSettings,
}
/// Document presentation persisted alongside topology.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Presentation {
    /// Current style.
    pub style: ViewStyle,
    /// Section planes and active cut.
    #[serde(default)]
    pub sections: SectionState,
    /// Geographic sunlight settings.
    #[serde(default)]
    pub shadows: ShadowSettings,
    /// Ordered named scenes.
    pub scenes: Vec<SavedScene>,
}

/// World plane; nonnegative signed distances are retained.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SectionPlane {
    /// Display name.
    pub name: String,
    /// Unit normal pointing into the retained half space.
    pub normal: [f64; 3],
    /// Plane equation n dot p = distance, in metres.
    pub distance: f64,
    /// Show the editor widget independently of clipping.
    pub visible: bool,
}
/// Independent section widgets with at most one active cut.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SectionState {
    /// Ordered planes.
    pub planes: Vec<SectionPlane>,
    /// Selected cutting plane index.
    pub active: Option<usize>,
    /// Enable clipping independently of widget visibility.
    pub enabled: bool,
}
impl SectionState {
    /// Effective active plane.
    pub fn cut(&self) -> Option<&SectionPlane> {
        self.enabled.then_some(self.active).flatten().and_then(|i| self.planes.get(i))
    }
}
/// Shadow map resolution and filtering cost.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ShadowQuality {
    /// 1024 texels, four comparison samples.
    Low,
    /// 2048 texels, nine comparison samples.
    #[default]
    Medium,
    /// 4096 texels, nine comparison samples.
    High,
}
/// Sun date/time is local civil time with an explicit UTC offset.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ShadowSettings {
    /// Shadows are opt in.
    pub enabled: bool,
    /// Gregorian year.
    pub year: i32,
    /// Month 1..12.
    pub month: u32,
    /// Day of month.
    pub day: u32,
    /// Local minutes after midnight.
    pub minutes: f64,
    /// Local UTC offset in hours (no implicit DST).
    pub utc_offset: f64,
    /// Latitude north in degrees.
    pub latitude: f64,
    /// Longitude east in degrees.
    pub longitude: f64,
    /// Lit surface multiplier 0..1.
    pub light: f32,
    /// Occluded surface multiplier 0..1.
    pub dark: f32,
    /// Receive shadows on faces.
    pub on_faces: bool,
    /// Receive shadows on Z=0 ground.
    pub on_ground: bool,
    /// Resolution/filter quality.
    pub quality: ShadowQuality,
}
impl Default for ShadowSettings {
    fn default() -> Self {
        Self { enabled: false, year: 2026, month: 6, day: 21, minutes: 720.,
            utc_offset: 8., latitude: 14.5995, longitude: 120.9842,
            light: 1., dark: 0.35, on_faces: true, on_ground: true,
            quality: ShadowQuality::default() }
    }
}
