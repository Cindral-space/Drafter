//! Native versioned archives, embedded assets, recovery and interchange formats.
use cindral_core::{DocumentData, DocumentHistory};
use std::{collections::BTreeMap, path::Path};
/// Current zip-container schema; document topology keeps its independent schema field.
pub const ARCHIVE_VERSION: u32 = 4;
/// Errors from validated file boundaries.
#[derive(Debug, thiserror::Error)]
pub enum IoError {
    /// Filesystem/stream failure.
    #[error(transparent)]
    Io(#[from] std::io::Error),
    /// Invalid archive structure or interchange data.
    #[error("{0}")]
    Invalid(String),
    /// Zip read/write failure.
    #[error(transparent)]
    Zip(#[from] zip::result::ZipError),
    /// Geometry/document validation failure.
    #[error(transparent)]
    Kernel(#[from] cindral_core::KernelError),
}
/// File-boundary result.
pub type Result<T> = std::result::Result<T, IoError>;
/// Original encoded image, embedded once irrespective of material references.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextureAsset {
    /// Validated PNG or JPEG media type.
    pub mime: String,
    /// Original encoded bytes, not decoded runtime pixels.
    pub bytes: Vec<u8>,
}
/// Validated image pixels for runtime GPU caches; never part of native schema values.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecodedTexture {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Row-major RGBA8 sRGB pixels.
    pub rgba: Vec<u8>,
}
/// Complete file values; contains no history, active context, spatial indices or GPU caches.
#[derive(Clone, Debug, PartialEq)]
pub struct DraftBundle {
    /// Versioned geometry and organization values.
    pub document: DocumentData,
    /// Current style and named camera/visibility/style snapshots.
    pub presentation: cindral_core::Presentation,
    /// Embedded texture assets keyed by `Material::texture` slots.
    pub textures: BTreeMap<String, TextureAsset>,
}
impl Default for DraftBundle {
    fn default() -> Self {
        Self {
            document: DocumentHistory::new().to_data(),
            presentation: Default::default(),
            textures: BTreeMap::new(),
        }
    }
}
/// Interchange workload statistics, excluding hidden occurrences.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ExportSummary {
    /// Exported occurrence count with geometry.
    pub instances: usize,
    /// Exported triangles across placed occurrences.
    pub triangles: usize,
}
/// Validate an untrusted bundle and rebuild independent runtime document state.
pub fn validate_bundle(bundle: &DraftBundle) -> Result<DocumentHistory> {
    if bundle.presentation.scenes.len() > 1000 {
        return Err(IoError::Invalid("too many scenes (maximum 1000)".into()));
    }
    validate_environment(&bundle.presentation.sections, &bundle.presentation.shadows)?;
    for scene in &bundle.presentation.scenes {
        validate_environment(&scene.sections, &scene.shadows)?;
        let camera = &scene.camera;
        if scene.name.trim().is_empty()
            || scene.name.len() > 512
            || scene.name.chars().any(char::is_control)
            || !camera
                .target
                .iter()
                .all(|v| v.is_finite() && v.abs() <= 1e9)
            || !camera.distance.is_finite()
            || !(1e-3..=1e6).contains(&camera.distance)
            || !camera.yaw.is_finite()
            || !camera.pitch.is_finite()
            || camera.pitch.abs() > std::f32::consts::FRAC_PI_2
            || scene.tags.len() > 200_000
            || scene.objects.len() > 200_000
        {
            return Err(IoError::Invalid(
                "invalid scene name, camera or visibility limits".into(),
            ));
        }
    }
    if bundle.textures.len() > 127 {
        return Err(IoError::Invalid(
            "at most 127 embedded textures are supported".into(),
        ));
    }
    for (key, asset) in &bundle.textures {
        if key.is_empty() || key.len() > 1024 || key.chars().any(char::is_control) {
            return Err(IoError::Invalid("invalid texture slot".into()));
        }
        decode_texture(asset)?;
    }
    Ok(DocumentHistory::from_data(bundle.document.clone())?)
}
fn validate_environment(
    sections: &cindral_core::SectionState,
    shadows: &cindral_core::ShadowSettings,
) -> Result<()> {
    if sections.planes.len() > 1000 || sections.active.is_some_and(|i| i >= sections.planes.len()) {
        return Err(IoError::Invalid(
            "invalid section count or active index".into(),
        ));
    }
    for p in &sections.planes {
        let normal = cindral_core::DVec3::from_array(p.normal);
        if p.name.trim().is_empty()
            || p.name.len() > 512
            || p.name.chars().any(char::is_control)
            || !normal.is_finite()
            || (normal.length() - 1.).abs() > 1e-8
            || !p.distance.is_finite()
            || p.distance.abs() > cindral_core::MAX_COORDINATE
        {
            return Err(IoError::Invalid("invalid section plane".into()));
        }
    }
    let s = shadows;
    if !cindral_core::valid_date(s.year, s.month, s.day)
        || !s.minutes.is_finite()
        || !(0. ..1440.).contains(&s.minutes)
        || !s.utc_offset.is_finite()
        || !(-14. ..=14.).contains(&s.utc_offset)
        || !s.latitude.is_finite()
        || !(-90. ..=90.).contains(&s.latitude)
        || !s.longitude.is_finite()
        || !(-180. ..=180.).contains(&s.longitude)
        || !s.light.is_finite()
        || !(0. ..=1.).contains(&s.light)
        || !s.dark.is_finite()
        || !(0. ..=1.).contains(&s.dark)
    {
        return Err(IoError::Invalid(
            "invalid sun date, location or intensity".into(),
        ));
    }
    Ok(())
}
/// Atomically replace one file; failure before rename preserves the previous destination.
/// Public for interchange writers; the closure must only write the supplied temporary file.
pub fn atomic_write(
    path: &Path,
    write: impl FnOnce(&mut std::fs::File) -> Result<()>,
) -> Result<()> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    write(temporary.as_file_mut())?;
    temporary.as_file_mut().sync_all()?;
    temporary.persist(path).map_err(|e| IoError::Io(e.error))?;
    #[cfg(unix)]
    std::fs::File::open(parent)?.sync_all()?;
    Ok(())
}
mod texture;
pub use texture::{decode_texture, load_texture, texture_from_bytes};
mod archive;
pub use archive::{
    load_draft, load_draft_report, recovery_files, save_autosave, save_draft, LoadReport,
};
mod import_obj;
pub use import_obj::{import_obj, import_obj_scene, import_obj_str};
mod export;
pub use export::{export_gltf, export_obj, export_stl};

pub(crate) mod import_scene;
pub use import_scene::*;
mod import_gltf;
pub use import_gltf::{import_gltf, import_gltf_bytes};
mod import_dae;
pub use import_dae::{import_dae, import_dae_bytes};

mod import_skp;
pub use import_skp::{import_skp, import_skp_bytes};
