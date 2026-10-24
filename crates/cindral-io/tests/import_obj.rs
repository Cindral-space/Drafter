//! OBJ parser and associated material/image regressions.
use cindral_core::DefinitionKind;
use cindral_io::{import_obj, import_obj_str, validate_bundle};
use glam::DMat4;
use std::{fs::File, io::Write};
use tempfile::tempdir;

#[test]
fn test_positive_indices_triangle_and_quad() {
    let obj = r#"
# Simple planar triangle and quad
v 0.0 0.0 0.0
v 1.0 0.0 0.0
v 1.0 1.0 0.0
v 0.0 1.0 0.0
f 1 2 3
f 1 3 4
"#;
    let bundle = import_obj_str(obj).expect("valid triangle and quad");
    validate_bundle(&bundle).expect("bundle validates");
    let root = bundle
        .document
        .definitions
        .get(&bundle.document.root)
        .unwrap();
    assert_eq!(root.geometry.faces.len(), 2);
    assert_eq!(root.geometry.vertices.len(), 4);
    assert_eq!(root.geometry.edges.len(), 5); // 4 boundary + 1 shared diagonal
}

#[test]
fn test_planar_quad_polygon() {
    let obj = r#"
v 0.0 0.0 0.0
v 2.0 0.0 0.0
v 2.0 2.0 0.0
v 0.0 2.0 0.0
f 1 2 3 4
"#;
    let bundle = import_obj_str(obj).expect("valid quad polygon");
    let root = bundle
        .document
        .definitions
        .get(&bundle.document.root)
        .unwrap();
    assert_eq!(root.geometry.faces.len(), 1);
    let face = root.geometry.faces.values().next().unwrap();
    assert_eq!(face.outer.len(), 4);
    assert_eq!(root.geometry.edges.len(), 4);
}

#[test]
fn test_negative_indices() {
    let obj = r#"
v 0.0 0.0 0.0
v 1.0 0.0 0.0
v 1.0 1.0 0.0
# Negative indices referring to the 3 vertices above (-3 is 1, -2 is 2, -1 is 3)
f -3 -2 -1
# Interleaved vertex definitions
v 0.0 1.0 0.0
# Reference vertex 1 (now -4), vertex 3 (now -2), and vertex 4 (-1)
f -4 -2 -1
"#;
    let bundle = import_obj_str(obj).expect("negative indices should resolve");
    let root = bundle
        .document
        .definitions
        .get(&bundle.document.root)
        .unwrap();
    assert_eq!(root.geometry.faces.len(), 2);
    assert_eq!(root.geometry.vertices.len(), 4);
}

#[test]
fn test_vertex_welding_within_tolerance() {
    let obj = r#"
# Vertices 1 and 4 have distance < 1e-7 and should weld together
v 0.0 0.0 0.0
v 1.0 0.0 0.0
v 1.0 1.0 0.0
v 1e-8 0.0 0.0
v 0.0 1.0 0.0
f 1 2 3
f 4 3 5
"#;
    let bundle = import_obj_str(obj).expect("epsilon-aware welding succeeds");
    let root = bundle
        .document
        .definitions
        .get(&bundle.document.root)
        .unwrap();
    // Vertices 1 and 4 weld, leaving 4 unique vertices in geometry
    assert_eq!(root.geometry.vertices.len(), 4);
    assert_eq!(root.geometry.faces.len(), 2);
}

#[test]
fn test_grouped_materials_and_private_definitions() {
    let obj = r#"
v 0 0 0
v 1 0 0
v 1 1 0
v 0 1 0

v 10 0 0
v 11 0 0
v 11 1 0

g Body
usemtl Metal
f 1 2 3 4

g Wing
usemtl Plastic
f 5 6 7
"#;
    let bundle = import_obj_str(obj).expect("grouped obj succeeds");
    validate_bundle(&bundle).expect("bundle validates");

    // Root definition should have instances pointing to the groups
    let root = bundle
        .document
        .definitions
        .get(&bundle.document.root)
        .unwrap();
    assert_eq!(root.instances.len(), 2);

    let mut group_names = Vec::new();
    for instance in root.instances.values() {
        assert_eq!(instance.transform, DMat4::IDENTITY);
        let group_def = bundle
            .document
            .definitions
            .get(&instance.definition)
            .unwrap();
        assert_eq!(group_def.kind, DefinitionKind::Group);
        group_names.push(group_def.name.clone());
    }
    group_names.sort();
    assert_eq!(group_names, vec!["Body", "Wing"]);

    // Check materials created
    assert_eq!(bundle.document.materials.len(), 2);
    let mat_names: Vec<_> = bundle
        .document
        .materials
        .values()
        .map(|m| m.name.as_str())
        .collect();
    assert!(mat_names.contains(&"Metal"));
    assert!(mat_names.contains(&"Plastic"));
}

#[test]
fn test_real_mtl_and_png_texture_loading() {
    let dir = tempdir().expect("tempdir");
    let png_path = dir.path().join("wood.png");

    // Create a minimal valid 2x2 RGBA PNG image
    let img = image::RgbaImage::from_pixel(2, 2, image::Rgba([120, 80, 40, 255]));
    img.save_with_format(&png_path, image::ImageFormat::Png)
        .expect("save png");

    let mtl_content = r#"
newmtl Wood
Kd 0.6 0.4 0.2
d 0.95
map_Kd wood.png
"#;
    let mtl_path = dir.path().join("material.mtl");
    File::create(&mtl_path)
        .expect("create mtl")
        .write_all(mtl_content.as_bytes())
        .expect("write mtl");

    let obj_content = r#"
mtllib material.mtl
v 0 0 0
v 1 0 0
v 1 1 0
usemtl Wood
f 1 2 3
"#;
    let obj_path = dir.path().join("model.obj");
    File::create(&obj_path)
        .expect("create obj")
        .write_all(obj_content.as_bytes())
        .expect("write obj");

    let bundle = import_obj(&obj_path).expect("import obj with real mtl and png");
    validate_bundle(&bundle).expect("bundle validates");

    assert_eq!(bundle.textures.len(), 1);
    let (tex_key, tex_asset) = bundle.textures.iter().next().unwrap();
    assert_eq!(tex_asset.mime, "image/png");

    let mat = bundle.document.materials.values().next().unwrap();
    assert_eq!(mat.name, "Wood");
    assert_eq!(mat.texture.as_ref(), Some(tex_key));
    assert!((mat.opacity - 0.95).abs() < 1e-4);
    assert_eq!(mat.color[0], 153);
}

#[test]
fn test_rejection_of_path_escapes() {
    let dir = tempdir().expect("tempdir");
    let outside_dir = tempdir().expect("outside tempdir");
    let outside_mtl = outside_dir.path().join("outside.mtl");
    File::create(&outside_mtl).expect("create outside mtl");

    // Parent path escape via ..
    let obj_parent = "mtllib ../outside.mtl\nv 0 0 0\nv 1 0 0\nv 1 1 0\nf 1 2 3\n".to_string();
    let obj_path = dir.path().join("model.obj");
    File::create(&obj_path)
        .expect("create obj")
        .write_all(obj_parent.as_bytes())
        .expect("write obj");
    assert!(
        import_obj(&obj_path).is_err(),
        "parent path escape must be rejected"
    );

    // Absolute path escape
    let obj_abs = format!(
        "mtllib {}\nv 0 0 0\nv 1 0 0\nv 1 1 0\nf 1 2 3\n",
        outside_mtl.display()
    );
    File::create(&obj_path)
        .expect("create obj")
        .write_all(obj_abs.as_bytes())
        .expect("write obj");
    assert!(
        import_obj(&obj_path).is_err(),
        "absolute path escape must be rejected"
    );
}

#[test]
fn test_rejection_of_invalid_geometries() {
    // Zero index
    assert!(import_obj_str("v 0 0 0\nv 1 0 0\nv 0 1 0\nf 0 1 2").is_err());

    // Out of range index
    assert!(import_obj_str("v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 5").is_err());

    // Negative out of range index
    assert!(import_obj_str("v 0 0 0\nv 1 0 0\nv 0 1 0\nf -4 -2 -1").is_err());

    // Nonfinite coordinate (NaN)
    assert!(import_obj_str("v NaN 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3").is_err());

    // Nonfinite coordinate (Infinity)
    assert!(import_obj_str("v inf 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3").is_err());

    // Out of range coordinate (> 1e6 meters)
    assert!(import_obj_str("v 2000000 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3").is_err());

    // Nonplanar quad
    let nonplanar_quad = r#"
v 0 0 0
v 1 0 0
v 1 1 0
v 0 1 0.5
f 1 2 3 4
"#;
    assert!(
        import_obj_str(nonplanar_quad).is_err(),
        "nonplanar face must be rejected"
    );

    // Collinear/degenerate triangle
    let degenerate = r#"
v 0 0 0
v 1 0 0
v 2 0 0
f 1 2 3
"#;
    assert!(
        import_obj_str(degenerate).is_err(),
        "degenerate face must be rejected"
    );

    // Face with fewer than 3 vertices
    assert!(import_obj_str("v 0 0 0\nv 1 0 0\nf 1 2").is_err());
}

#[test]
fn test_string_import_never_reads_files() {
    let obj_with_mtl = r#"
mtllib /etc/passwd
v 0 0 0
v 1 0 0
v 1 1 0
f 1 2 3
"#;
    // import_obj_str must not access filesystem or fail due to mtllib
    let bundle = import_obj_str(obj_with_mtl).expect("string import ignores mtllib safely");
    validate_bundle(&bundle).expect("bundle validates");
    assert!(bundle.textures.is_empty());
}
