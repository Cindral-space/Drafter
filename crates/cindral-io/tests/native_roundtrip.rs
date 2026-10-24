//! Complete organization/material/asset boundary, independent of windowing and GPU caches.
use cindral_core::*;
use cindral_io::*;
use glam::DMat4;
use std::{collections::BTreeMap, io::Cursor};
fn populated() -> DraftBundle {
    let mut doc = DocumentHistory::new();
    doc.execute(Command::AddPolyline {
        points: vec![
            DVec3::ZERO,
            DVec3::X * 2.,
            (DVec3::X + DVec3::Y) * 2.,
            DVec3::Y * 2.,
        ],
        closed: true,
    })
    .unwrap();
    let face = *doc.model().faces().keys().next().unwrap();
    doc.execute(Command::PushPull { face, distance: 3. })
        .unwrap();
    let vertices = doc.model().vertices().keys().copied().collect();
    doc.organization(OrganizationCommand::Create {
        kind: DefinitionKind::Group,
        name: "Assembly".into(),
        vertices,
    })
    .unwrap();
    let group = doc
        .instances(DefinitionId(0))
        .unwrap()
        .values()
        .next()
        .unwrap()
        .clone();
    doc.enter(vec![group.id]).unwrap();
    let vertices = doc.model().vertices().keys().copied().collect();
    doc.organization(OrganizationCommand::Create {
        kind: DefinitionKind::Component,
        name: "Block".into(),
        vertices,
    })
    .unwrap();
    let component = doc
        .instances(group.definition)
        .unwrap()
        .values()
        .next()
        .unwrap()
        .clone();
    for x in [4., 8.] {
        doc.organization(OrganizationCommand::AddInstance {
            definition: component.definition,
            name: "Copy".into(),
            transform: DMat4::from_translation(DVec3::X * x)
                * DMat4::from_scale(DVec3::new(-1., 2., 1.)),
        })
        .unwrap();
    }
    doc.organization(OrganizationCommand::CreateTag("Structure".into()))
        .unwrap();
    let tag = *doc.tags().keys().next().unwrap();
    doc.organization(OrganizationCommand::AssignTag {
        path: vec![group.id],
        tag: Some(tag),
    })
    .unwrap();
    let mut encoded = Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
        2,
        2,
        image::Rgba([80, 160, 220, 128]),
    ))
    .write_to(&mut encoded, image::ImageFormat::Png)
    .unwrap();
    let (key, asset) = texture_from_bytes(encoded.into_inner()).unwrap();
    doc.organization(OrganizationCommand::CreateMaterial(Material {
        name: "Glazed".into(),
        color: [51, 128, 204],
        opacity: 0.7,
        texture: Some(key.clone()),
        texture_mapping: None,
    }))
    .unwrap();
    let material = *doc.materials().keys().next().unwrap();
    doc.organization(OrganizationCommand::PaintInstance {
        path: vec![group.id],
        material: Some(material),
    })
    .unwrap();
    doc.enter(vec![group.id, component.id]).unwrap();
    let face = *doc.model().faces().keys().next().unwrap();
    doc.organization(OrganizationCommand::PaintFaces {
        faces: vec![face],
        material: Some(material),
    })
    .unwrap();
    DraftBundle {
        presentation: Default::default(),
        document: doc.to_data(),
        textures: BTreeMap::from([(key, asset)]),
    }
}
#[test]
fn draw_extrude_nested_component_tag_material_texture_save_load_is_identical() {
    let original = populated();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("assembly.cdraft");
    save_draft(&path, &original).unwrap();
    let loaded = load_draft(&path).unwrap();
    assert_eq!(loaded, original);
    let mut runtime = validate_bundle(&loaded).unwrap();
    assert_eq!(runtime.undo_depth(), 0);
    assert_eq!(runtime.redo_depth(), 0);
    assert!(runtime.context().is_empty());
    let group = *runtime
        .instances(DefinitionId(0))
        .unwrap()
        .keys()
        .next()
        .unwrap();
    runtime
        .organization(OrganizationCommand::Rename {
            path: vec![group],
            name: "Renamed after load".into(),
        })
        .unwrap();
    assert_eq!(runtime.undo_depth(), 1);
    runtime.undo().unwrap();
    assert_eq!(runtime.to_data(), original.document);
    let zip = zip::ZipArchive::new(std::fs::File::open(&path).unwrap()).unwrap();
    assert_eq!(
        zip.len(),
        2,
        "one original image, independent of instance count"
    );
    let old = std::fs::read(&path).unwrap();
    let mut invalid = original.clone();
    invalid.document.schema_version = 99;
    assert!(save_draft(&path, &invalid).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), old);
}
#[test]
fn unexpected_entries_are_rejected_and_recovery_roundtrips() {
    use std::io::Write;
    let bundle = populated();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("recovery.cdraft.autosave");
    save_autosave(&path, &bundle).unwrap();
    assert_eq!(recovery_files(dir.path()).unwrap(), vec![path.clone()]);
    assert_eq!(load_draft(&path).unwrap(), bundle);
    let file = std::fs::File::options()
        .read(true)
        .write(true)
        .open(&path)
        .unwrap();
    let mut zip = zip::ZipWriter::new_append(file).unwrap();
    zip.start_file("unexpected.txt", zip::write::SimpleFileOptions::default())
        .unwrap();
    zip.write_all(b"extra").unwrap();
    zip.finish().unwrap();
    assert!(load_draft(&path).is_err());
}
#[test]
fn valid_zip_with_tampered_image_fails_the_manifest_checksum() {
    use std::io::{Read, Write};
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("source.cdraft");
    let altered = directory.path().join("altered.cdraft");
    save_draft(&source, &populated()).unwrap();
    let mut zip = zip::ZipArchive::new(std::fs::File::open(source).unwrap()).unwrap();
    let mut output = zip::ZipWriter::new(std::fs::File::create(&altered).unwrap());
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).unwrap();
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes).unwrap();
        if entry.name().starts_with("textures/") {
            bytes[0] ^= 1;
        }
        output
            .start_file(entry.name(), zip::write::SimpleFileOptions::default())
            .unwrap();
        output.write_all(&bytes).unwrap();
    }
    output.finish().unwrap();
    assert!(load_draft(&altered)
        .unwrap_err()
        .to_string()
        .contains("checksum mismatch"));
}
