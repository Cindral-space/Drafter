//! Versioned zip/RON values, explicit migrations and atomic durable replacement.
use crate::{
    atomic_write, validate_bundle, DraftBundle, IoError, Result, TextureAsset, ARCHIVE_VERSION,
};
use cindral_core::DocumentData;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::File,
    io::{Read, Write},
    path::{Path, PathBuf},
};
use zip::{write::SimpleFileOptions, CompressionMethod, ZipArchive, ZipWriter};
const MAX_MANIFEST: u64 = 128 * 1024 * 1024;
const MAX_TOTAL: u64 = 512 * 1024 * 1024;
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
struct Coordinates {
    unit: String,
    up: String,
    handedness: String,
}
impl Default for Coordinates {
    fn default() -> Self {
        Self {
            unit: "metre".into(),
            up: "Z".into(),
            handedness: "right".into(),
        }
    }
}
#[derive(Debug, Serialize, Deserialize)]
struct AssetRecord {
    key: String,
    entry: String,
    mime: String,
    sha256: String,
}
#[derive(Serialize)]
struct ManifestRef<'a> {
    archive_version: u32,
    coordinates: Coordinates,
    document: &'a DocumentData,
    presentation: &'a cindral_core::Presentation,
    textures: Vec<AssetRecord>,
}
#[derive(Deserialize)]
struct Manifest {
    archive_version: u32,
    #[serde(default)]
    coordinates: Coordinates,
    document: DocumentData,
    #[serde(default)]
    presentation: cindral_core::Presentation,
    #[serde(default)]
    textures: Vec<AssetRecord>,
}
/// Source format information returned by a validated load/migration.
#[derive(Debug)]
pub struct LoadReport {
    /// Format version found on disk, before migration.
    pub source_version: u32,
    /// Current runtime-independent values.
    pub bundle: DraftBundle,
}
fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
fn migrate(mut manifest: Manifest) -> Result<Manifest> {
    match manifest.archive_version {
        1 => {
            if !manifest.textures.is_empty() {
                return Err(IoError::Invalid(
                    "v1 archives cannot contain an asset table".into(),
                ));
            }
            // v1 stores document schema1 only. v2 adds native coordinate metadata and
            // embedded-image records; unresolved legacy texture slots remain values.
            manifest.presentation = Default::default();
            manifest.coordinates = Coordinates::default();
            manifest.archive_version = ARCHIVE_VERSION;
        }
        2 => {
            manifest.presentation = Default::default();
            manifest.archive_version = ARCHIVE_VERSION;
        }
        3 => {
            manifest.archive_version = ARCHIVE_VERSION;
        }
        ARCHIVE_VERSION => {}
        other => {
            return Err(IoError::Invalid(format!(
                "unsupported .cdraft archive version {other}"
            )))
        }
    }
    // Document schema 2 moved material colors to sRGB bytes; schema-1 linear
    // float colors already upgraded themselves during deserialization, so the
    // stored rendered look is unchanged. Anything else is not a known schema.
    if manifest.document.schema_version == 1 {
        manifest.document.schema_version = cindral_core::SCHEMA_VERSION;
    }
    if manifest.coordinates != Coordinates::default() {
        return Err(IoError::Invalid(
            "unsupported native units or axis orientation".into(),
        ));
    }
    Ok(manifest)
}
/// Save a validated bundle as an atomic zip container; history/navigation/caches are excluded.
pub fn save_draft(path: &Path, bundle: &DraftBundle) -> Result<()> {
    validate_bundle(bundle)?;
    let textures = bundle
        .textures
        .iter()
        .enumerate()
        .map(|(i, (key, asset))| AssetRecord {
            key: key.clone(),
            entry: format!(
                "textures/{i}.{}",
                if asset.mime == "image/png" {
                    "png"
                } else {
                    "jpg"
                }
            ),
            mime: asset.mime.clone(),
            sha256: digest(&asset.bytes),
        })
        .collect::<Vec<_>>();
    let manifest = ManifestRef {
        archive_version: ARCHIVE_VERSION,
        coordinates: Coordinates::default(),
        document: &bundle.document,
        presentation: &bundle.presentation,
        textures,
    };
    let text = ron::ser::to_string(&manifest).map_err(|e| IoError::Invalid(e.to_string()))?;
    if text.len() as u64 > MAX_MANIFEST {
        return Err(IoError::Invalid("native manifest exceeds 128 MiB".into()));
    }
    let total = bundle
        .textures
        .values()
        .try_fold(text.len() as u64, |total, a| {
            total
                .checked_add(a.bytes.len() as u64)
                .ok_or_else(|| IoError::Invalid("archive size overflow".into()))
        })?;
    if total > MAX_TOTAL {
        return Err(IoError::Invalid(
            "native archive exceeds 512 MiB uncompressed".into(),
        ));
    }
    atomic_write(path, |file| {
        let mut zip = ZipWriter::new(file);
        zip.start_file(
            "manifest.ron",
            SimpleFileOptions::default()
                .compression_method(CompressionMethod::Deflated)
                .compression_level(Some(3)),
        )?;
        zip.write_all(text.as_bytes())?;
        for record in &manifest.textures {
            let asset = bundle
                .textures
                .get(&record.key)
                .ok_or_else(|| IoError::Invalid("missing texture during save".into()))?;
            zip.start_file(
                &record.entry,
                SimpleFileOptions::default().compression_method(CompressionMethod::Stored),
            )?;
            zip.write_all(&asset.bytes)?;
        }
        zip.finish()?;
        Ok(())
    })
}
fn entry(zip: &mut ZipArchive<File>, name: &str, limit: u64) -> Result<Vec<u8>> {
    let file = zip.by_name(name)?;
    if file.size() > limit {
        return Err(IoError::Invalid(format!(
            "archive entry {name} exceeds its size limit"
        )));
    }
    let mut bytes = Vec::new();
    file.take(limit + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err(IoError::Invalid(
            "archive decompression exceeds declared limit".into(),
        ));
    }
    Ok(bytes)
}
/// Load and explicitly migrate supported archive versions, then validate all document/asset values.
pub fn load_draft_report(path: &Path) -> Result<LoadReport> {
    let file = File::open(path)?;
    if file.metadata()?.len() > MAX_TOTAL {
        return Err(IoError::Invalid(
            "compressed archive exceeds 512 MiB".into(),
        ));
    }
    let mut zip = ZipArchive::new(file)?;
    if zip.len() > 128 {
        return Err(IoError::Invalid("too many native archive entries".into()));
    }
    let mut names = BTreeSet::new();
    let mut total = 0u64;
    for i in 0..zip.len() {
        let file = zip.by_index(i)?;
        if !names.insert(file.name().to_owned()) {
            return Err(IoError::Invalid("duplicate archive entry".into()));
        }
        total = total
            .checked_add(file.size())
            .ok_or_else(|| IoError::Invalid("archive size overflow".into()))?;
        if total > MAX_TOTAL {
            return Err(IoError::Invalid(
                "archive exceeds 512 MiB uncompressed".into(),
            ));
        }
    }
    let bytes = entry(&mut zip, "manifest.ron", MAX_MANIFEST)?;
    let manifest: Manifest = ron::Options::default()
        .with_recursion_limit(128)
        .from_bytes(&bytes)
        .map_err(|e| IoError::Invalid(e.to_string()))?;
    let source_version = manifest.archive_version;
    let manifest = migrate(manifest)?;
    if manifest.textures.len() > 127 {
        return Err(IoError::Invalid("too many embedded textures".into()));
    }
    let mut textures = BTreeMap::new();
    let mut used = BTreeSet::from(["manifest.ron".to_owned()]);
    for record in manifest.textures {
        let entry_path = Path::new(&record.entry);
        if !record.entry.starts_with("textures/")
            || entry_path
                .components()
                .any(|p| !matches!(p, std::path::Component::Normal(_)))
            || !used.insert(record.entry.clone())
        {
            return Err(IoError::Invalid(
                "invalid or duplicate texture archive path".into(),
            ));
        }
        let bytes = entry(
            &mut zip,
            &record.entry,
            crate::texture::MAX_TEXTURE_BYTES as u64,
        )?;
        if digest(&bytes) != record.sha256 {
            return Err(IoError::Invalid(
                "embedded texture checksum mismatch".into(),
            ));
        }
        if textures
            .insert(
                record.key,
                TextureAsset {
                    mime: record.mime,
                    bytes,
                },
            )
            .is_some()
        {
            return Err(IoError::Invalid("duplicate texture slot".into()));
        }
    }
    if used != names {
        return Err(IoError::Invalid("unexpected native archive entry".into()));
    }
    let bundle = DraftBundle {
        presentation: manifest.presentation,
        document: manifest.document,
        textures,
    };
    validate_bundle(&bundle)?;
    Ok(LoadReport {
        source_version,
        bundle,
    })
}
/// Load current values; undo/redo and active context are intentionally not persisted.
pub fn load_draft(path: &Path) -> Result<DraftBundle> {
    Ok(load_draft_report(path)?.bundle)
}
/// Write a crash-recovery snapshot using the same atomic protocol as a manual save.
pub fn save_autosave(path: &Path, bundle: &DraftBundle) -> Result<()> {
    save_draft(path, bundle)
}
/// List recovery snapshots without decoding them or following embedded asset paths.
pub fn recovery_files(directory: &Path) -> Result<Vec<PathBuf>> {
    if !directory.exists() {
        return Ok(Vec::new());
    }
    let mut files = Vec::new();
    for item in std::fs::read_dir(directory)? {
        let item = item?;
        if item.file_type()?.is_file()
            && item
                .file_name()
                .to_string_lossy()
                .ends_with(".cdraft.autosave")
        {
            files.push(item.path());
        }
    }
    files.sort();
    Ok(files)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn atomic_failure_preserves_previous_file_and_removes_partial_temp() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("model.cdraft");
        std::fs::write(&path, b"previous file").unwrap();
        let result = atomic_write(&path, |file| {
            file.write_all(b"half-written new file")?;
            Err(IoError::Invalid("simulated interruption".into()))
        });
        assert!(result.is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"previous file");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }
    #[test]
    fn saved_v1_fixture_migrates_and_resaves_as_v4() {
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/v1-square.cdraft");
        let report = load_draft_report(&fixture).unwrap();
        assert_eq!(report.source_version, 1);
        assert_eq!(
            report.bundle.document.schema_version,
            cindral_core::SCHEMA_VERSION
        );
        let doc = validate_bundle(&report.bundle).unwrap();
        assert_eq!(doc.model().faces().len(), 1);
        assert_eq!(doc.undo_depth(), 0);
        assert!(doc.context().is_empty());
        let dir = tempfile::tempdir().unwrap();
        let output = dir.path().join("new.cdraft");
        save_draft(&output, &report.bundle).unwrap();
        let saved = load_draft_report(&output).unwrap();
        assert_eq!(saved.source_version, 4);
        assert_eq!(saved.bundle, report.bundle);
    }
    #[test]
    fn schema1_linear_material_colors_migrate_preserving_the_rendered_look() {
        use cindral_core::linear_rgb_to_srgb;
        // Serialize a document shaped exactly like schema-1 writers produced:
        // identical nested types, only material colors are linear floats.
        #[derive(serde::Serialize)]
        struct LegacyMaterial {
            name: String,
            color: [f32; 3],
            opacity: f32,
            texture: Option<String>,
        }
        #[derive(serde::Serialize)]
        struct LegacyDocument {
            schema_version: u32,
            next_id: u64,
            root: cindral_core::DefinitionId,
            definitions: std::collections::BTreeMap<
                cindral_core::DefinitionId,
                cindral_core::DefinitionData,
            >,
            tags: std::collections::BTreeMap<cindral_core::TagId, cindral_core::Tag>,
            materials: std::collections::BTreeMap<cindral_core::MaterialId, LegacyMaterial>,
        }
        #[derive(serde::Serialize)]
        struct LegacyManifest<'a> {
            archive_version: u32,
            coordinates: Coordinates,
            document: &'a LegacyDocument,
            presentation: &'a cindral_core::Presentation,
            textures: Vec<AssetRecord>,
        }
        let base = DraftBundle::default();
        let linear = [0.5f32, 0.214, 1.0];
        let legacy = LegacyManifest {
            archive_version: ARCHIVE_VERSION,
            coordinates: Coordinates::default(),
            presentation: &base.presentation,
            textures: Vec::new(),
            document: &LegacyDocument {
                schema_version: 1,
                next_id: 8,
                root: cindral_core::DefinitionId(0),
                definitions: base.document.definitions.clone(),
                tags: base.document.tags.clone(),
                materials: std::collections::BTreeMap::from([(
                    cindral_core::MaterialId(7),
                    LegacyMaterial {
                        name: "Legacy paint".into(),
                        color: linear,
                        opacity: 0.8,
                        texture: Some("slot".into()),
                    },
                )]),
            },
        };
        let text = ron::ser::to_string(&legacy).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("legacy.cdraft");
        {
            let mut zip = ZipWriter::new(File::create(&path).unwrap());
            zip.start_file("manifest.ron", SimpleFileOptions::default())
                .unwrap();
            zip.write_all(text.as_bytes()).unwrap();
            zip.finish().unwrap();
        }
        let bundle = load_draft(&path).unwrap();
        assert_eq!(bundle.document.schema_version, cindral_core::SCHEMA_VERSION);
        let material = &bundle.document.materials[&cindral_core::MaterialId(7)];
        // linear_to_srgb per channel, quantized once to bytes: the rendered
        // look of the legacy linear values is exactly preserved.
        assert_eq!(material.color, linear_rgb_to_srgb(linear));
        assert_eq!(material.opacity, 0.8);
        assert_eq!(material.texture.as_deref(), Some("slot"));
        let output = dir.path().join("resaved.cdraft");
        save_draft(&output, &bundle).unwrap();
        assert_eq!(load_draft(&output).unwrap(), bundle);
    }
    #[test]
    fn autosave_recovery_and_invalid_archives() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("untitled.cdraft.autosave");
        let bundle = DraftBundle::default();
        save_autosave(&path, &bundle).unwrap();
        assert_eq!(recovery_files(dir.path()).unwrap(), vec![path.clone()]);
        assert_eq!(load_draft(&path).unwrap(), bundle);
        let invalid = dir.path().join("invalid.cdraft");
        std::fs::write(&invalid, b"partial zip").unwrap();
        assert!(load_draft(&invalid).is_err());
        let future = dir.path().join("future.cdraft");
        let text = ron::ser::to_string(&ManifestRef {
            archive_version: 99,
            coordinates: Coordinates::default(),
            document: &bundle.document,
            presentation: &bundle.presentation,
            textures: Vec::new(),
        })
        .unwrap();
        let mut zip = ZipWriter::new(File::create(&future).unwrap());
        zip.start_file("manifest.ron", SimpleFileOptions::default())
            .unwrap();
        zip.write_all(text.as_bytes()).unwrap();
        zip.finish().unwrap();
        assert!(load_draft(&future).is_err());
    }
}
