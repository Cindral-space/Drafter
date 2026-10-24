//! Interchange round trips, hand-written COLLADA and mutated hostile inputs.
use cindral_core::*;
use cindral_io::*;
use glam::DMat4;
use std::{collections::BTreeMap, io::Cursor};
fn metrics(h: &DocumentHistory) -> (usize, usize, DVec3, DVec3) {
    let mut definitions = std::collections::BTreeSet::new();
    let mut faces = 0;
    let mut count = 0;
    let mut min = DVec3::splat(f64::INFINITY);
    let mut max = DVec3::splat(f64::NEG_INFINITY);
    for o in h.occurrences().unwrap() {
        let m = h.definition_model(o.definition).unwrap();
        if m.faces().is_empty() {
            continue;
        }
        count += 1;
        if definitions.insert(o.definition) {
            faces += m.faces().len();
        }
        for v in m.vertices().values() {
            let p = o.world.transform_point3(v.position);
            min = min.min(p);
            max = max.max(p);
        }
    }
    (faces, count, min, max)
}
fn png() -> Vec<u8> {
    let mut out = Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
        2,
        2,
        image::Rgba([80, 160, 240, 128]),
    ))
    .write_to(&mut out, image::ImageFormat::Png)
    .unwrap();
    out.into_inner()
}
#[test]
fn exported_gltf_preserves_faces_shared_instances_bounds_and_assets() {
    let mut h = DocumentHistory::new();
    h.execute(Command::AddPolyline {
        points: vec![
            DVec3::ZERO,
            DVec3::new(2., 0., 0.),
            DVec3::new(2., 3., 0.),
            DVec3::new(0., 3., 0.),
        ],
        closed: true,
    })
    .unwrap();
    let (slot, asset) = texture_from_bytes(png()).unwrap();
    h.organization(OrganizationCommand::CreateMaterial(Material {
        name: "Blue glass".into(),
        color: [51, 102, 204],
        opacity: 0.5,
        texture: Some(slot.clone()),
        texture_mapping: None,
    }))
    .unwrap();
    let material = *h.materials().keys().next().unwrap();
    h.execute(Command::SetFaceMaterial {
        faces: h.model().faces().keys().copied().collect(),
        material: Some(material),
    })
    .unwrap();
    h.organization(OrganizationCommand::Create {
        kind: DefinitionKind::Component,
        name: "Panel".into(),
        vertices: h.model().vertices().keys().copied().collect(),
    })
    .unwrap();
    let definition = h
        .instances(DefinitionId(0))
        .unwrap()
        .values()
        .next()
        .unwrap()
        .definition;
    h.organization(OrganizationCommand::AddInstance {
        definition,
        name: "Mirrored panel".into(),
        transform: DMat4::from_translation(DVec3::new(8., 1., 2.))
            * DMat4::from_scale(DVec3::new(-2., 1., 1.)),
    })
    .unwrap();
    let expected = metrics(&h);
    let bundle = DraftBundle {
        document: h.to_data(),
        textures: BTreeMap::from([(slot, asset.clone())]),
        ..Default::default()
    };
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("panel.gltf");
    let export = export_gltf(&path, &bundle).unwrap();
    assert_eq!(export.instances, 2);
    let mut imported =
        import_file(&path, &ImportOptions::default(), &ImportControl::default()).unwrap();
    let actual = metrics(&imported.history);
    assert_eq!((actual.0, actual.1), (expected.0, expected.1));
    assert!(actual.2.distance(expected.2) < 1e-6);
    assert!(actual.3.distance(expected.3) < 1e-6);
    let occurrences: Vec<_> = imported
        .history
        .occurrences()
        .unwrap()
        .into_iter()
        .filter(|o| {
            !imported
                .history
                .definition_model(o.definition)
                .unwrap()
                .faces()
                .is_empty()
        })
        .collect();
    assert_eq!(occurrences[0].definition, occurrences[1].definition);
    assert_eq!(imported.textures.len(), 1);
    assert_eq!(imported.textures.values().next().unwrap(), &asset);
    let material = imported
        .history
        .materials()
        .values()
        .find(|m| m.name == "Blue glass")
        .unwrap();
    assert_eq!(material.color, [51, 102, 204]);
    assert_eq!(material.opacity, 0.5);
    assert_eq!(imported.history.undo_depth(), 1);
    let before = imported.history.to_data();
    imported.history.undo().unwrap();
    assert_eq!(metrics(&imported.history).1, 0);
    imported.history.redo().unwrap();
    assert_eq!(imported.history.to_data().definitions, before.definitions);
    let native = dir.path().join("imported.cdraft");
    let bundle = DraftBundle {
        document: imported.history.to_data(),
        textures: imported.textures,
        ..Default::default()
    };
    save_draft(&native, &bundle).unwrap();
    assert_eq!(load_draft(&native).unwrap(), bundle);
}
#[test]
fn dae_fixture_units_materials_library_nodes_and_polygons() {
    let bytes = include_bytes!("fixtures/i1-sample.dae");
    let s = import_dae_bytes(bytes, None, &ImportControl::default()).unwrap();
    assert_eq!(s.meters_per_unit, 0.01);
    assert_eq!(s.materials.len(), 2);
    let r = convert_scene(s, &ImportOptions::default(), &ImportControl::default()).unwrap();
    let m = metrics(&r.history);
    assert_eq!((m.0, m.1), (2, 2));
    assert!(m.2.distance(DVec3::ZERO) < 1e-12);
    assert!(m.3.distance(DVec3::new(1.1, 1.2, 0.)) < 1e-12);
    let xml = String::from_utf8(bytes.to_vec())
        .unwrap()
        .replace("Z_UP", "Y_UP");
    let s = import_dae_bytes(xml.as_bytes(), None, &ImportControl::default()).unwrap();
    let r = convert_scene(s, &ImportOptions::default(), &ImportControl::default()).unwrap();
    assert!(metrics(&r.history).3.distance(DVec3::new(1.1, 0., 1.2)) < 1e-12);
}
#[test]
fn dae_texture_sidecar_and_path_guard() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("checker.png"), png()).unwrap();
    let xml = r##"<COLLADA><asset><up_axis>Z_UP</up_axis></asset><library_images><image id="im"><init_from>checker.png</init_from></image></library_images><library_effects><effect id="fx"><profile_COMMON><newparam sid="surface"><surface type="2D"><init_from>im</init_from></surface></newparam><newparam sid="sampler"><sampler2D><source>surface</source></sampler2D></newparam><technique><lambert><diffuse><texture texture="sampler" texcoord="UV"/></diffuse></lambert></technique></profile_COMMON></effect></library_effects><library_materials><material id="mat"><instance_effect url="#fx"/></material></library_materials><library_geometries><geometry id="geo"><mesh><source id="pos"><float_array count="9">0 0 0 1 0 0 0 1 0</float_array></source><vertices id="v"><input semantic="POSITION" source="#pos"/></vertices><triangles count="1" material="mat"><input semantic="VERTEX" source="#v"/><p>0 1 2</p></triangles></mesh></geometry></library_geometries><library_visual_scenes><visual_scene id="s"><node><instance_geometry url="#geo"/></node></visual_scene></library_visual_scenes></COLLADA>"##;
    let s = import_dae_bytes(xml.as_bytes(), Some(dir.path()), &ImportControl::default()).unwrap();
    assert_eq!(s.textures.len(), 1);
    assert!(s.materials[0].texture.is_some());
    let malicious = xml.replace("checker.png", "../checker.png");
    assert!(import_dae_bytes(
        malicious.as_bytes(),
        Some(dir.path()),
        &ImportControl::default()
    )
    .is_err());
}
#[test]
fn mutation_fuzz_includes_conversion_and_resource_abuse() {
    for (seed, is_dae) in [
        (include_bytes!("fixtures/i1-seed.gltf").as_slice(), false),
        (include_bytes!("fixtures/i1-seed.glb").as_slice(), false),
        (include_bytes!("fixtures/i1-sample.dae").as_slice(), true),
    ] {
        for i in 0..192 {
            let mut bytes = seed.to_vec();
            let at = (i * 7919) % bytes.len();
            bytes[at] ^= (i as u8).wrapping_mul(31).wrapping_add(1);
            if i % 4 == 0 {
                bytes.truncate(at);
            }
            let result = std::panic::catch_unwind(|| {
                let c = ImportControl::default();
                let scene = if is_dae {
                    import_dae_bytes(&bytes, None, &c)
                } else {
                    import_gltf_bytes(&bytes, None, &c)
                };
                if let Ok(scene) = scene {
                    let _ = convert_scene(scene, &ImportOptions::default(), &c);
                }
            });
            assert!(result.is_ok(), "panic on mutation {i}, dae={is_dae}");
        }
    }
    for xml in [
        "<!DOCTYPE COLLADA SYSTEM 'file:///etc/passwd'><COLLADA/>",
        "<!DOCTYPE COLLADA [<!ENTITY e SYSTEM 'https://invalid.test'>]><COLLADA>&e;</COLLADA>",
    ] {
        assert!(import_dae_bytes(xml.as_bytes(), None, &ImportControl::default()).is_err());
    }
    let deep = format!(
        "<COLLADA>{}{}</COLLADA>",
        "<node>".repeat(5000),
        "</node>".repeat(5000)
    );
    assert!(import_dae_bytes(deep.as_bytes(), None, &ImportControl::default()).is_err());
    let cycle =
        br#"{"asset":{"version":"2.0"},"nodes":[{"children":[0]}],"scenes":[{"nodes":[0]}]}"#;
    assert!(import_gltf_bytes(cycle, None, &ImportControl::default()).is_err());
    let enormous =
        br#"{"asset":{"version":"2.0"},"buffers":[{"byteLength":18446744073709551615}]}"#;
    assert!(import_gltf_bytes(enormous, None, &ImportControl::default()).is_err());
}
#[test]
fn opaque_texture_default_and_inferred_root_do_not_duplicate_children() {
    use base64::Engine;
    let mut value: serde_json::Value =
        serde_json::from_slice(include_bytes!("fixtures/i1-seed.gltf")).unwrap();
    value.as_object_mut().unwrap().remove("scene");
    value.as_object_mut().unwrap().remove("scenes");
    value["nodes"] = serde_json::json!([{"children":[1]},{"mesh":0}]);
    value["images"] = serde_json::json!([{"uri":format!("data:image/png;base64,{}",base64::engine::general_purpose::STANDARD.encode(png()))}]);
    value["textures"] = serde_json::json!([{"source":0}]);
    value["materials"][0]["pbrMetallicRoughness"]["baseColorFactor"] =
        serde_json::json!([1., 1., 1., 0.2]);
    value["materials"][0]["pbrMetallicRoughness"]["baseColorTexture"] =
        serde_json::json!({"index":0});
    let control = ImportControl::default();
    let scene = import_gltf_bytes(&serde_json::to_vec(&value).unwrap(), None, &control).unwrap();
    assert_eq!(scene.materials[0].opacity, 1.);
    let image = decode_texture(scene.textures.values().next().unwrap()).unwrap();
    assert!(image.rgba.as_chunks::<4>().0.iter().all(|p| p[3] == 255));
    let imported = convert_scene(scene, &ImportOptions::default(), &control).unwrap();
    assert_eq!(metrics(&imported.history).1, 1);
    value["meshes"][0]["primitives"][0]["mode"] = serde_json::json!(0);
    let scene = import_gltf_bytes(&serde_json::to_vec(&value).unwrap(), None, &control).unwrap();
    assert_eq!(scene.skipped, 1);
    let imported = convert_scene(scene, &ImportOptions::default(), &control).unwrap();
    assert_eq!(imported.report.skipped, 1);
}
#[test]
fn xml_preflight_respects_cdata_comments_and_quoted_attributes() {
    let xml = r#"<COLLADA marker="&lt;node&gt;"><!-- <node><node> --><extra><![CDATA[<node><node><node>]]></extra></COLLADA>"#;
    assert!(import_dae_bytes(xml.as_bytes(), None, &ImportControl::default()).is_ok());
}
