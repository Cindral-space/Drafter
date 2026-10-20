//! Reproduce the checked-in document-only v1 migration fixture; never part of normal saving.
use cindral_core::{Command, DVec3, DocumentHistory};
use std::io::Write;
#[derive(serde::Serialize)]
struct V1 {
    archive_version: u32,
    document: cindral_core::DocumentData,
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut doc = DocumentHistory::new();
    doc.execute(Command::AddPolyline {
        points: vec![DVec3::ZERO, DVec3::X, DVec3::X + DVec3::Y, DVec3::Y],
        closed: true,
    })?;
    let text = ron::ser::to_string(&V1 {
        archive_version: 1,
        document: doc.to_data(),
    })?;
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/v1-square.cdraft");
    let mut zip = zip::ZipWriter::new(std::fs::File::create(path)?);
    zip.start_file("manifest.ron", zip::write::SimpleFileOptions::default())?;
    zip.write_all(text.as_bytes())?;
    zip.finish()?;
    Ok(())
}
