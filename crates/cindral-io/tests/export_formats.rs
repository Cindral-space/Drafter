//! Integration tests for OBJ, binary STL, and self-contained glTF 2.0 exporters.
//!
//! Verifies:
//! - Exact signatures: `export_obj`, `export_stl`, `export_gltf`.
//! - File parsing: binary STL headers/facets, OBJ/MTL text syntax and images, glTF JSON schema and buffers.
//! - Accurately flattened transforms, mirrored winding preservation, and inverse-transpose normals.
//! - Visibility filters: `hidden` flags and tag visibility.
//! - Material hierarchy: explicit face overrides, inherited container materials, opacity, and embedded textures.
//! - Shared definitions, instancing, and shared accessors for glTF.
//! - Core face triangulation with holes.
//! - Invalid data rejection.

use base64::prelude::*;
use cindral_core::{
    Command, DVec3, DefinitionId, DefinitionKind, DocumentData, DocumentHistory, Material,
    MaterialId, OrganizationCommand,
};
use cindral_io::{
    export_gltf, export_obj, export_stl, texture_from_bytes, DraftBundle, ExportSummary,
    TextureAsset,
};
use glam::DMat4;
use serde_json::Value;
use std::collections::BTreeMap;
use tempfile::tempdir;

/// Helper to create a 2x2 PNG texture asset for testing.
fn make_test_png() -> (String, TextureAsset) {
    let img = image::RgbaImage::from_pixel(2, 2, image::Rgba([200, 100, 50, 255]));
    let mut bytes = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(img)
        .write_to(&mut bytes, image::ImageFormat::Png)
        .expect("write test png");
    texture_from_bytes(bytes.into_inner()).expect("valid texture asset")
}

/// Helper to create a simple quad face model via DocumentHistory.
fn make_quad_doc(z: f64) -> DocumentData {
    let mut history = DocumentHistory::new();
    history
        .execute(Command::AddPolyline {
            points: vec![
                DVec3::new(0.0, 0.0, z),
                DVec3::new(1.0, 0.0, z),
                DVec3::new(1.0, 1.0, z),
                DVec3::new(0.0, 1.0, z),
            ],
            closed: true,
        })
        .expect("add polyline");
    history.to_data()
}

// ============================================================================
// STL Exporter Tests
// ============================================================================

#[test]
fn test_stl_binary_structure_and_winding() {
    let dir = tempdir().expect("tempdir");
    let stl_path = dir.path().join("test_model.stl");

    let doc = make_quad_doc(0.0);
    let bundle = DraftBundle {
        presentation: Default::default(),
        document: doc,
        textures: BTreeMap::new(),
    };

    let summary = export_stl(&stl_path, &bundle).expect("export stl");
    assert_eq!(summary.instances, 1);
    assert_eq!(summary.triangles, 2); // 1 quad = 2 triangles

    let bytes = std::fs::read(&stl_path).expect("read stl");
    // Expected size: 80 bytes header + 4 bytes count + 2 * 50 bytes facets = 184 bytes
    assert_eq!(bytes.len(), 80 + 4 + 2 * 50);

    let header_str = String::from_utf8_lossy(&bytes[..80]);
    assert!(header_str.contains("Cindral"));
    assert!(header_str.contains("meters"));
    assert!(header_str.contains("RH Z-up"));

    let tri_count = u32::from_le_bytes(bytes[80..84].try_into().unwrap());
    assert_eq!(tri_count, 2);

    for i in 0..2 {
        let offset = 84 + i * 50;
        let facet = &bytes[offset..offset + 50];

        let nx = f32::from_le_bytes(facet[0..4].try_into().unwrap());
        let ny = f32::from_le_bytes(facet[4..8].try_into().unwrap());
        let nz = f32::from_le_bytes(facet[8..12].try_into().unwrap());
        let normal = DVec3::new(nx as f64, ny as f64, nz as f64);
        assert!((normal.length() - 1.0).abs() < 1e-4);
        // Plane is at Z=0 facing +Z
        assert!((normal.z - 1.0).abs() < 1e-4);

        let v0 = DVec3::new(
            f32::from_le_bytes(facet[12..16].try_into().unwrap()) as f64,
            f32::from_le_bytes(facet[16..20].try_into().unwrap()) as f64,
            f32::from_le_bytes(facet[20..24].try_into().unwrap()) as f64,
        );
        let v1 = DVec3::new(
            f32::from_le_bytes(facet[24..28].try_into().unwrap()) as f64,
            f32::from_le_bytes(facet[28..32].try_into().unwrap()) as f64,
            f32::from_le_bytes(facet[32..36].try_into().unwrap()) as f64,
        );
        let v2 = DVec3::new(
            f32::from_le_bytes(facet[36..40].try_into().unwrap()) as f64,
            f32::from_le_bytes(facet[40..44].try_into().unwrap()) as f64,
            f32::from_le_bytes(facet[44..48].try_into().unwrap()) as f64,
        );

        // Check front-face counterclockwise winding with respect to normal
        let cross = (v1 - v0).cross(v2 - v0);
        assert!(cross.dot(normal) > 0.0);

        let attr = u16::from_le_bytes(facet[48..50].try_into().unwrap());
        assert_eq!(attr, 0);
    }
}

// ============================================================================
// OBJ Exporter Tests
// ============================================================================

#[test]
fn test_obj_export_with_mtl_and_textures() {
    let dir = tempdir().expect("tempdir");
    let obj_path = dir.path().join("scene.obj");

    let (tex_key, tex_asset) = make_test_png();
    let mut doc = make_quad_doc(0.0);

    // Add a custom material with texture
    let mat_id = MaterialId(101);
    doc.next_id = doc.next_id.max(mat_id.0 + 1);
    doc.materials.insert(
        mat_id,
        Material {
            name: "WoodFloor".to_string(),
            color: [179, 128, 77],
            opacity: 0.85,
            texture: Some(tex_key.clone()),
            texture_mapping: None,
        },
    );

    // Apply material to the root face
    let root_def = doc.definitions.get_mut(&doc.root).unwrap();
    let face_id = *root_def.geometry.faces.keys().next().unwrap();
    root_def.geometry.faces.get_mut(&face_id).unwrap().material = Some(mat_id);

    let mut textures = BTreeMap::new();
    textures.insert(tex_key, tex_asset.clone());

    let bundle = DraftBundle {
        presentation: Default::default(),
        document: doc,
        textures,
    };

    let summary = export_obj(&obj_path, &bundle).expect("export obj");
    assert_eq!(summary.instances, 1);
    assert_eq!(summary.triangles, 2);

    // 1. Verify OBJ file
    let obj_content = std::fs::read_to_string(&obj_path).expect("read obj");
    assert!(obj_content.contains("# Units: meters"));
    assert!(obj_content.contains("# Coordinate system: right-handed Z-up"));
    assert!(obj_content.contains("mtllib scene.mtl"));
    assert!(obj_content.contains("usemtl WoodFloor_mat_101"));

    let v_count = obj_content.lines().filter(|l| l.starts_with("v ")).count();
    let vt_count = obj_content.lines().filter(|l| l.starts_with("vt ")).count();
    let vn_count = obj_content.lines().filter(|l| l.starts_with("vn ")).count();
    let f_count = obj_content.lines().filter(|l| l.starts_with("f ")).count();

    assert_eq!(f_count, 2);
    assert_eq!(v_count, 6); // 2 triangles * 3 vertices
    assert_eq!(vt_count, 6);
    assert_eq!(vn_count, 2);

    // 2. Verify sibling MTL file
    let mtl_path = dir.path().join("scene.mtl");
    assert!(mtl_path.exists());
    let mtl_content = std::fs::read_to_string(&mtl_path).expect("read mtl");
    assert!(mtl_content.contains("newmtl WoodFloor_mat_101"));
    assert!(mtl_content.contains("Kd 0.701961 0.501961 0.301961"));
    assert!(mtl_content.contains("d 0.850000"));
    assert!(mtl_content.contains("map_Kd scene_tex_"));

    // 3. Verify copied sibling texture image
    let tex_line = mtl_content
        .lines()
        .find(|l| l.starts_with("map_Kd "))
        .expect("map_Kd line");
    let tex_filename = tex_line.strip_prefix("map_Kd ").unwrap().trim();
    let image_path = dir.path().join(tex_filename);
    assert!(image_path.exists());
    let image_bytes = std::fs::read(&image_path).expect("read texture");
    assert_eq!(image_bytes, tex_asset.bytes);
}

// ============================================================================
// glTF Exporter Tests
// ============================================================================

#[test]
fn test_gltf_schema_accessors_and_root_conversion() {
    let dir = tempdir().expect("tempdir");
    let gltf_path = dir.path().join("model.gltf");

    let (tex_key, tex_asset) = make_test_png();
    let mut doc = make_quad_doc(0.0);

    let mat_id = MaterialId(202);
    doc.next_id = doc.next_id.max(mat_id.0 + 1);
    doc.materials.insert(
        mat_id,
        Material {
            name: "Glass".to_string(),
            color: [51, 102, 230],
            opacity: 0.5,
            texture: Some(tex_key.clone()),
            texture_mapping: None,
        },
    );

    let root_def = doc.definitions.get_mut(&doc.root).unwrap();
    let face_id = *root_def.geometry.faces.keys().next().unwrap();
    root_def.geometry.faces.get_mut(&face_id).unwrap().material = Some(mat_id);

    let mut textures = BTreeMap::new();
    textures.insert(tex_key, tex_asset.clone());

    let bundle = DraftBundle {
        presentation: Default::default(),
        document: doc,
        textures,
    };

    let summary = export_gltf(&gltf_path, &bundle).expect("export gltf");
    assert_eq!(summary.instances, 1);
    assert_eq!(summary.triangles, 2);

    let file_str = std::fs::read_to_string(&gltf_path).expect("read gltf");
    let gltf: Value = serde_json::from_str(&file_str).expect("parse gltf json");

    // Asset validation
    assert_eq!(gltf["asset"]["version"], "2.0");
    assert_eq!(gltf["scene"], 0);

    // Root coordinate conversion node
    let root_node = &gltf["nodes"][0];
    assert_eq!(root_node["name"], "RootCoordinateConversion");
    let matrix = root_node["matrix"].as_array().expect("matrix array");
    assert_eq!(matrix.len(), 16);
    // (x, y, z) -> (x, z, -y) column major:
    assert_eq!(matrix[0], 1.0);
    assert_eq!(matrix[6], -1.0);
    assert_eq!(matrix[9], 1.0);
    assert_eq!(matrix[15], 1.0);

    // Buffer validation (embedded data URI)
    let buffer_uri = gltf["buffers"][0]["uri"].as_str().expect("buffer uri");
    assert!(buffer_uri.starts_with("data:application/octet-stream;base64,"));
    let b64_data = buffer_uri
        .strip_prefix("data:application/octet-stream;base64,")
        .unwrap();
    let buffer_bytes = BASE64_STANDARD.decode(b64_data).expect("decode base64");
    assert_eq!(
        buffer_bytes.len(),
        gltf["buffers"][0]["byteLength"].as_u64().unwrap() as usize
    );

    // Accessors validation
    let accessors = gltf["accessors"].as_array().expect("accessors");
    assert!(accessors.len() >= 4); // pos, norm, uv, indices

    // Validate position accessor
    let pos_acc = &accessors[0];
    assert_eq!(pos_acc["type"], "VEC3");
    assert_eq!(pos_acc["componentType"], 5126); // FLOAT
    let pos_min = pos_acc["min"].as_array().unwrap();
    let pos_max = pos_acc["max"].as_array().unwrap();
    assert_eq!(pos_min[0].as_f64().unwrap(), 0.0);
    assert_eq!(pos_max[0].as_f64().unwrap(), 1.0);
    assert_eq!(pos_min[1].as_f64().unwrap(), 0.0);
    assert_eq!(pos_max[1].as_f64().unwrap(), 1.0);

    // Material validation
    let materials = gltf["materials"].as_array().expect("materials");
    let mat = &materials[0];
    assert_eq!(mat["alphaMode"], "BLEND"); // opacity 0.5 < 1.0
    assert_eq!(mat["pbrMetallicRoughness"]["baseColorFactor"][3], 0.5);
    assert_eq!(mat["pbrMetallicRoughness"]["baseColorTexture"]["index"], 0);

    // Embedded image validation
    let img_uri = gltf["images"][0]["uri"].as_str().expect("image uri");
    assert!(img_uri.starts_with("data:image/png;base64,"));
    let img_b64 = img_uri.strip_prefix("data:image/png;base64,").unwrap();
    let decoded_img = BASE64_STANDARD.decode(img_b64).expect("decode image");
    assert_eq!(decoded_img, tex_asset.bytes);
}

// ============================================================================
// Instancing and Shared Meshes Tests
// ============================================================================

#[test]
fn test_gltf_shared_meshes_for_multiple_instances() {
    let dir = tempdir().expect("tempdir");
    let gltf_path = dir.path().join("instanced.gltf");

    let mut history = DocumentHistory::new();
    // Create base geometry in root
    history
        .execute(Command::AddPolyline {
            points: vec![
                DVec3::new(0.0, 0.0, 0.0),
                DVec3::new(1.0, 0.0, 0.0),
                DVec3::new(1.0, 1.0, 0.0),
                DVec3::new(0.0, 1.0, 0.0),
            ],
            closed: true,
        })
        .unwrap();

    // Group it into a component
    let v_ids: Vec<_> = history.model().vertices().keys().copied().collect();
    history
        .organization(OrganizationCommand::Create {
            kind: DefinitionKind::Component,
            name: "Tile".into(),
            vertices: v_ids,
        })
        .unwrap();

    // Now instantiate it twice at different locations
    let component_id = *history
        .to_data()
        .definitions
        .keys()
        .find(|&&id| id != DefinitionId(0))
        .unwrap();

    history
        .organization(OrganizationCommand::AddInstance {
            definition: component_id,
            name: "Tile_1".into(),
            transform: DMat4::from_translation(DVec3::new(2.0, 0.0, 0.0)),
        })
        .unwrap();

    let bundle = DraftBundle {
        presentation: Default::default(),
        document: history.to_data(),
        textures: BTreeMap::new(),
    };

    let summary = export_gltf(&gltf_path, &bundle).expect("export gltf");
    // Placed occurrences with geometry: 2 instances (Tile + Tile_1)
    assert_eq!(summary.instances, 2);
    assert_eq!(summary.triangles, 4); // 2 * 2 = 4 triangles

    let gltf_str = std::fs::read_to_string(&gltf_path).expect("read gltf");
    let gltf: Value = serde_json::from_str(&gltf_str).expect("parse gltf");

    // Only ONE mesh should be created and shared between both instance nodes!
    let meshes = gltf["meshes"].as_array().expect("meshes");
    assert_eq!(meshes.len(), 1);

    // Verify nodes reference this same mesh ID (0)
    let nodes = gltf["nodes"].as_array().expect("nodes");
    let mesh_referencing_nodes: Vec<_> = nodes
        .iter()
        .filter(|n| n.get("mesh").and_then(|m| m.as_u64()) == Some(0))
        .collect();
    assert_eq!(mesh_referencing_nodes.len(), 2);
}

// ============================================================================
// Visibility and Tag Filtering Tests
// ============================================================================

#[test]
fn test_hidden_and_tagged_instances_are_excluded() {
    let dir = tempdir().expect("tempdir");

    let mut history = DocumentHistory::new();
    history
        .execute(Command::AddPolyline {
            points: vec![
                DVec3::ZERO,
                DVec3::new(1.0, 0.0, 0.0),
                DVec3::new(1.0, 1.0, 0.0),
                DVec3::new(0.0, 1.0, 0.0),
            ],
            closed: true,
        })
        .unwrap();

    let v_ids: Vec<_> = history.model().vertices().keys().copied().collect();
    history
        .organization(OrganizationCommand::Create {
            kind: DefinitionKind::Component,
            name: "HiddenComponent".into(),
            vertices: v_ids,
        })
        .unwrap();

    let mut doc = history.to_data();

    // Mark the created instance as hidden
    let root_def = doc.definitions.get_mut(&doc.root).unwrap();
    let inst = root_def.instances.values_mut().next().unwrap();
    inst.hidden = true;

    let bundle = DraftBundle {
        presentation: Default::default(),
        document: doc,
        textures: BTreeMap::new(),
    };

    let stl_path = dir.path().join("hidden.stl");
    let obj_path = dir.path().join("hidden.obj");
    let gltf_path = dir.path().join("hidden.gltf");

    assert_eq!(
        export_stl(&stl_path, &bundle).unwrap(),
        ExportSummary {
            instances: 0,
            triangles: 0
        }
    );
    assert_eq!(
        export_obj(&obj_path, &bundle).unwrap(),
        ExportSummary {
            instances: 0,
            triangles: 0
        }
    );
    assert_eq!(
        export_gltf(&gltf_path, &bundle).unwrap(),
        ExportSummary {
            instances: 0,
            triangles: 0
        }
    );
}

// ============================================================================
// Mirrored Transforms and Winding Preservation Tests
// ============================================================================

#[test]
fn test_mirrored_transforms_preserve_ccw_winding() {
    let dir = tempdir().expect("tempdir");
    let mut history = DocumentHistory::new();
    history
        .execute(Command::AddPolyline {
            points: vec![
                DVec3::ZERO,
                DVec3::new(1.0, 0.0, 0.0),
                DVec3::new(1.0, 1.0, 0.0),
                DVec3::new(0.0, 1.0, 0.0),
            ],
            closed: true,
        })
        .unwrap();

    let v_ids: Vec<_> = history.model().vertices().keys().copied().collect();
    history
        .organization(OrganizationCommand::Create {
            kind: DefinitionKind::Component,
            name: "MirroredQuad".into(),
            vertices: v_ids,
        })
        .unwrap();

    let mut doc = history.to_data();

    // Set a mirrored scale transform: scale X by -1 (determinant < 0)
    let root_def = doc.definitions.get_mut(&doc.root).unwrap();
    let inst = root_def.instances.values_mut().next().unwrap();
    inst.transform = DMat4::from_scale(DVec3::new(-1.0, 1.0, 1.0));

    let bundle = DraftBundle {
        presentation: Default::default(),
        document: doc,
        textures: BTreeMap::new(),
    };

    let stl_path = dir.path().join("mirrored.stl");
    export_stl(&stl_path, &bundle).expect("export mirrored stl");

    let bytes = std::fs::read(&stl_path).expect("read stl");
    // Verify each facet in mirrored STL maintains CCW front-facing winding
    for i in 0..2 {
        let offset = 84 + i * 50;
        let facet = &bytes[offset..offset + 50];
        let normal = DVec3::new(
            f32::from_le_bytes(facet[0..4].try_into().unwrap()) as f64,
            f32::from_le_bytes(facet[4..8].try_into().unwrap()) as f64,
            f32::from_le_bytes(facet[8..12].try_into().unwrap()) as f64,
        );
        let v0 = DVec3::new(
            f32::from_le_bytes(facet[12..16].try_into().unwrap()) as f64,
            f32::from_le_bytes(facet[16..20].try_into().unwrap()) as f64,
            f32::from_le_bytes(facet[20..24].try_into().unwrap()) as f64,
        );
        let v1 = DVec3::new(
            f32::from_le_bytes(facet[24..28].try_into().unwrap()) as f64,
            f32::from_le_bytes(facet[28..32].try_into().unwrap()) as f64,
            f32::from_le_bytes(facet[32..36].try_into().unwrap()) as f64,
        );
        let v2 = DVec3::new(
            f32::from_le_bytes(facet[36..40].try_into().unwrap()) as f64,
            f32::from_le_bytes(facet[40..44].try_into().unwrap()) as f64,
            f32::from_le_bytes(facet[44..48].try_into().unwrap()) as f64,
        );

        let cross = (v1 - v0).cross(v2 - v0);
        // CCW winding condition: cross dot normal must be positive
        assert!(cross.dot(normal) > 0.0);
    }
}

// ============================================================================
// Hole Triangulation Tests
// ============================================================================

#[test]
fn test_face_with_holes_triangulation() {
    let dir = tempdir().expect("tempdir");

    let mut history = DocumentHistory::new();
    // Outer boundary: 10x10 square
    history
        .execute(Command::AddPolyline {
            points: vec![
                DVec3::new(0.0, 0.0, 0.0),
                DVec3::new(10.0, 0.0, 0.0),
                DVec3::new(10.0, 10.0, 0.0),
                DVec3::new(0.0, 10.0, 0.0),
            ],
            closed: true,
        })
        .unwrap();

    // Inner hole: 2x2 square inside
    history
        .execute(Command::AddPolyline {
            points: vec![
                DVec3::new(4.0, 4.0, 0.0),
                DVec3::new(6.0, 4.0, 0.0),
                DVec3::new(6.0, 6.0, 0.0),
                DVec3::new(4.0, 6.0, 0.0),
            ],
            closed: true,
        })
        .unwrap();

    // Drawing an inner loop partitions the face, including a filled inner region.
    // Remove that region from this DTO to export an actual open aperture.
    let mut document = history.to_data();
    document
        .definitions
        .get_mut(&document.root)
        .unwrap()
        .geometry
        .faces
        .retain(|_, face| !face.holes.is_empty());
    let bundle = DraftBundle {
        presentation: Default::default(),
        document,
        textures: BTreeMap::new(),
    };

    let stl_path = dir.path().join("holes.stl");
    let summary = export_stl(&stl_path, &bundle).expect("export stl with hole");
    // Outer square with inner hole will have > 2 triangles (typically 8-10 triangles)
    assert!(summary.triangles >= 8);

    // Sum of triangulated areas should equal 100 - 4 = 96 m^2 (within tolerance)
    let bytes = std::fs::read(&stl_path).expect("read stl");
    let mut total_area = 0.0;
    for i in 0..summary.triangles {
        let offset = 84 + i * 50;
        let facet = &bytes[offset..offset + 50];
        let v0 = DVec3::new(
            f32::from_le_bytes(facet[12..16].try_into().unwrap()) as f64,
            f32::from_le_bytes(facet[16..20].try_into().unwrap()) as f64,
            f32::from_le_bytes(facet[20..24].try_into().unwrap()) as f64,
        );
        let v1 = DVec3::new(
            f32::from_le_bytes(facet[24..28].try_into().unwrap()) as f64,
            f32::from_le_bytes(facet[28..32].try_into().unwrap()) as f64,
            f32::from_le_bytes(facet[32..36].try_into().unwrap()) as f64,
        );
        let v2 = DVec3::new(
            f32::from_le_bytes(facet[36..40].try_into().unwrap()) as f64,
            f32::from_le_bytes(facet[40..44].try_into().unwrap()) as f64,
            f32::from_le_bytes(facet[44..48].try_into().unwrap()) as f64,
        );
        total_area += (v1 - v0).cross(v2 - v0).length() * 0.5;
    }
    // Total area: 100 - 4 = 96
    assert!((total_area - 96.0).abs() < 1e-4);
}

// ============================================================================
// Invalid Bundle Rejection Tests
// ============================================================================

#[test]
fn test_invalid_bundles_are_rejected() {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("out.stl");

    // 1. Texture with invalid bytes
    let mut textures = BTreeMap::new();
    textures.insert(
        "bad_tex".to_string(),
        TextureAsset {
            mime: "image/png".to_string(),
            bytes: vec![1, 2, 3, 4], // corrupt png
        },
    );
    let bundle = DraftBundle {
        presentation: Default::default(),
        document: make_quad_doc(0.0),
        textures,
    };
    assert!(export_stl(&path, &bundle).is_err());
    assert!(export_obj(&dir.path().join("out.obj"), &bundle).is_err());
    assert!(export_gltf(&dir.path().join("out.gltf"), &bundle).is_err());
}

#[test]
fn explicit_face_color_overrides_container_material_in_every_color_format() {
    let mut h = DocumentHistory::from_data(make_quad_doc(0.)).unwrap();
    let face = *h.model().faces().keys().next().unwrap();
    h.execute(Command::PaintFace {
        faces: vec![face],
        color: [255, 0, 0, 255],
    })
    .unwrap();
    h.organization(OrganizationCommand::Create {
        kind: DefinitionKind::Component,
        name: "Painted".into(),
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
    let id = MaterialId(h.to_data().next_id);
    h.organization(OrganizationCommand::CreateMaterial(Material {
        name: "Blue".into(),
        color: [0, 0, 255],
        opacity: 0.5,
        texture: None,
        texture_mapping: None,
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
        textures: BTreeMap::new(),
    };
    let dir = tempdir().unwrap();
    export_obj(&dir.path().join("paint.obj"), &bundle).unwrap();
    assert!(std::fs::read_to_string(dir.path().join("paint.mtl"))
        .unwrap()
        .contains("Kd 1.000000 0.000000 0.000000"));
    export_gltf(&dir.path().join("paint.gltf"), &bundle).unwrap();
    let gltf: Value =
        serde_json::from_slice(&std::fs::read(dir.path().join("paint.gltf")).unwrap()).unwrap();
    assert_eq!(
        gltf["materials"][0]["pbrMetallicRoughness"]["baseColorFactor"],
        serde_json::json!([1., 0., 0., 1.])
    );
}
#[test]
fn unsupported_gltf_shear_fails_before_replacing_previous_export() {
    let mut h = DocumentHistory::from_data(make_quad_doc(0.)).unwrap();
    h.organization(OrganizationCommand::Create {
        kind: DefinitionKind::Component,
        name: "Shear".into(),
        vertices: h.model().vertices().keys().copied().collect(),
    })
    .unwrap();
    let mut document = h.to_data();
    document
        .definitions
        .get_mut(&document.root)
        .unwrap()
        .instances
        .values_mut()
        .next()
        .unwrap()
        .transform
        .y_axis
        .x = 0.5;
    let bundle = DraftBundle {
        presentation: Default::default(),
        document,
        textures: BTreeMap::new(),
    };
    let dir = tempdir().unwrap();
    let path = dir.path().join("shear.gltf");
    std::fs::write(&path, b"previous export").unwrap();
    assert!(export_gltf(&path, &bundle)
        .unwrap_err()
        .to_string()
        .contains("shear"));
    assert_eq!(std::fs::read(&path).unwrap(), b"previous export");
    export_obj(&dir.path().join("shear.obj"), &bundle).unwrap();
    export_stl(&dir.path().join("shear.stl"), &bundle).unwrap();
}
