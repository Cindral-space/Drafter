//! Generate a native textured solid for capture_viewport via CINDRAL_CAPTURE_DRAFT.
use cindral_core::*;
use cindral_io::*;
use std::{collections::BTreeMap, io::Cursor, path::Path};
fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let mut h = DocumentHistory::new();
    h.execute(Command::AddPolyline {
        points: vec![
            DVec3::new(-2., -2., 0.),
            DVec3::new(2., -2., 0.),
            DVec3::new(2., 2., 0.),
            DVec3::new(-2., 2., 0.),
        ],
        closed: true,
    })?;
    let face = *h
        .model()
        .faces()
        .keys()
        .next()
        .ok_or(KernelError::MissingEntity)?;
    h.execute(Command::PushPull { face, distance: 3. })?;
    let image = image::RgbaImage::from_fn(32, 32, |x, y| {
        if (x / 8 + y / 8).is_multiple_of(2) {
            image::Rgba([245, 170, 35, 255])
        } else {
            image::Rgba([30, 90, 190, 180])
        }
    });
    let mut bytes = Cursor::new(Vec::new());
    image.write_to(&mut bytes, image::ImageFormat::Png)?;
    let (key, asset) = texture_from_bytes(bytes.into_inner())?;
    let id = MaterialId(h.to_data().next_id);
    h.organization(OrganizationCommand::CreateMaterial(Material {
        name: "Checker".into(),
        color: [255, 255, 255],
        opacity: 1.,
        texture: Some(key.clone()),
        texture_mapping: None,
    }))?;
    let faces = h.model().faces().keys().copied().collect::<Vec<_>>();
    for face in faces {
        h.organization(OrganizationCommand::PaintFaces {
            faces: vec![face],
            material: Some(id),
        })?;
    }
    h.organization(OrganizationCommand::Create {
        kind: DefinitionKind::Component,
        name: "Textured box".into(),
        vertices: h.model().vertices().keys().copied().collect(),
    })?;
    let definition = h
        .instances(DefinitionId(0))
        .and_then(|items| items.values().next())
        .ok_or(KernelError::MissingEntity)?
        .definition;
    h.organization(OrganizationCommand::AddInstance {
        definition,
        name: "Mirrored copy".into(),
        transform: glam::DMat4::from_scale_rotation_translation(
            DVec3::new(-0.5, 0.7, 1.),
            glam::DQuat::IDENTITY,
            DVec3::X * 6.,
        ),
    })?;
    let bundle = DraftBundle {
        presentation: Default::default(),
        document: h.to_data(),
        textures: BTreeMap::from([(key, asset)]),
    };
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "texture-demo.cdraft".into());
    save_draft(Path::new(&path), &bundle)?;
    export_gltf(&Path::new(&path).with_extension("gltf"), &bundle)?;
    export_obj(&Path::new(&path).with_extension("obj"), &bundle)?;
    export_stl(&Path::new(&path).with_extension("stl"), &bundle)?;
    export_gltf(
        &Path::new(&path).with_extension("empty.gltf"),
        &DraftBundle::default(),
    )?;
    Ok(())
}
