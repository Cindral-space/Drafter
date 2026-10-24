//! Cross-boundary OBJ/MTL/image export/import with shared mirrored placements.
use cindral_core::*;
use cindral_io::*;
use std::{collections::BTreeMap, io::Cursor};
#[test]
fn textured_component_export_import_preserves_placed_surface_area_and_assets() {
    let mut h = DocumentHistory::new();
    h.execute(Command::AddPolyline {
        points: vec![DVec3::ZERO, DVec3::X, DVec3::X + DVec3::Y, DVec3::Y],
        closed: true,
    })
    .unwrap();
    h.organization(OrganizationCommand::Create {
        kind: DefinitionKind::Component,
        name: "Tile".into(),
        vertices: h.model().vertices().keys().copied().collect(),
    })
    .unwrap();
    let node = h
        .instances(DefinitionId(0))
        .unwrap()
        .values()
        .next()
        .unwrap()
        .clone();
    h.organization(OrganizationCommand::AddInstance {
        definition: node.definition,
        name: "Mirror".into(),
        transform: glam::DMat4::from_scale_rotation_translation(
            DVec3::new(-2., 3., 1.),
            glam::DQuat::IDENTITY,
            DVec3::X * 4.,
        ),
    })
    .unwrap();
    let mut bytes = Cursor::new(Vec::new());
    image::RgbaImage::from_pixel(2, 2, image::Rgba([60, 120, 180, 128]))
        .write_to(&mut bytes, image::ImageFormat::Png)
        .unwrap();
    let (key, asset) = texture_from_bytes(bytes.into_inner()).unwrap();
    let id = MaterialId(h.to_data().next_id);
    h.organization(OrganizationCommand::CreateMaterial(Material {
        name: "Texture".into(),
        color: [77, 128, 179],
        opacity: 0.75,
        texture: Some(key.clone()),
    }))
    .unwrap();
    h.organization(OrganizationCommand::PaintInstance {
        path: vec![node.id],
        material: Some(id),
    })
    .unwrap();
    let bundle = DraftBundle {
        presentation: Default::default(),
        document: h.to_data(),
        textures: BTreeMap::from([(key, asset.clone())]),
    };
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("model with spaces.obj");
    let summary = export_obj(&path, &bundle).unwrap();
    assert_eq!(summary.instances, 2);
    assert_eq!(summary.triangles, 4);
    let imported = import_obj(&path).unwrap();
    assert!(imported.textures.values().any(|a| a == &asset));
    let doc = validate_bundle(&imported).unwrap();
    let area: f64 = doc
        .occurrences()
        .unwrap()
        .iter()
        .map(|o| {
            let m = doc.definition_model(o.definition).unwrap();
            m.faces()
                .keys()
                .flat_map(|f| m.triangulate(*f).unwrap())
                .map(|[a, b, c]| (b - a).cross(c - a).length() * 0.5)
                .sum::<f64>()
        })
        .sum();
    assert!((area - 7.).abs() < 1e-6);
}
