// High-fidelity export for OBJ, binary STL, and self-contained glTF 2.0.
//
// Conforms to M6 frozen boundaries:
// - Native/OBJ/STL: meters, right-handed Z-up.
// - glTF 2.0: meters, right-handed Y-up with root conversion `(x, y, z) -> (x, z, -y)`.
// - Validates untrusted bundles via [`validate_bundle`].
// - Safe atomic replacement via [`atomic_write`].
// - Core face triangulation with hole support via `cindral_core::Model::triangulate`.

use crate::{atomic_write, validate_bundle, DraftBundle, ExportSummary, IoError, Result};
use base64::prelude::*;
use cindral_core::{DVec3, DefinitionId, DocumentData, Face, Material, MaterialId};
use glam::DMat4;
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, HashMap},
    io::{BufWriter, Write},
    path::Path,
};

/// Header bytes for binary STL (80 bytes).
const STL_HEADER: &[u8] = b"Cindral binary STL export; meters, RH Z-up                      ";

/// Root transformation matrix converting right-handed Z-up to right-handed Y-up:
/// `(x, y, z) -> (x, z, -y)`. Column-major order for glTF.
const GLTF_ROOT_CONVERSION_MATRIX: [f32; 16] = [
    1.0, 0.0, 0.0, 0.0, // col 0
    0.0, 0.0, -1.0, 0.0, // col 1
    0.0, 1.0, 0.0, 0.0, // col 2
    0.0, 0.0, 0.0, 1.0, // col 3
];

/// Derive planar 2D texture coordinates from 3D position and dominant face normal.
fn planar_uv(p: DVec3, normal: DVec3) -> [f32; 2] {
    let abs_n = normal.abs();
    if abs_n.x >= abs_n.y && abs_n.x >= abs_n.z {
        [p.y as f32, p.z as f32]
    } else if abs_n.y >= abs_n.z {
        [p.x as f32, p.z as f32]
    } else {
        [p.x as f32, p.y as f32]
    }
}

/// Transform a normal vector by the inverse-transpose of an affine matrix and normalize.
fn transform_normal(inv_transpose_3x3: DMat4, normal: DVec3) -> DVec3 {
    let transformed = inv_transpose_3x3.transform_vector3(normal);
    let len = transformed.length();
    if len > 1e-12 {
        transformed / len
    } else {
        normal
    }
}

/// Resolved material color and texture properties for a single face.
#[derive(Clone, Debug, PartialEq)]
struct ResolvedMaterial {
    key: String,
    name: String,
    color: [f32; 3],
    opacity: f32,
    texture: Option<String>,
    /// Real-world mapping to apply to planar UVs; `None` keeps 1 repeat/m.
    texture_mapping: Option<cindral_core::TextureMapping>,
}

fn resolve_face_material(
    face: &Face,
    inherited_material: Option<MaterialId>,
    doc_materials: &BTreeMap<MaterialId, Material>,
) -> ResolvedMaterial {
    let material_id = face.material.or_else(|| {
        (face.color == [160, 177, 193, 255])
            .then_some(inherited_material)
            .flatten()
    });
    if let Some(id) = material_id {
        if let Some(mat) = doc_materials.get(&id) {
            let opacity = if face.material.is_none() && face.color != [160, 177, 193, 255] {
                (face.color[3] as f32 / 255.0) * mat.opacity
            } else {
                mat.opacity
            };
            return ResolvedMaterial {
                key: format!("mat_{}", id.0),
                name: if mat.name.is_empty() {
                    format!("mat_{}", id.0)
                } else {
                    mat.name.clone()
                },
                color: crate::srgb_floats(mat.color),
                opacity,
                texture: mat.texture.clone(),
                texture_mapping: mat.texture_mapping,
            };
        }
    }
    // Fall back to face color.
    let r = face.color[0] as f32 / 255.0;
    let g = face.color[1] as f32 / 255.0;
    let b = face.color[2] as f32 / 255.0;
    let a = face.color[3] as f32 / 255.0;
    ResolvedMaterial {
        key: format!(
            "color_{}_{}_{}_{}",
            face.color[0], face.color[1], face.color[2], face.color[3]
        ),
        name: format!(
            "color_{}_{}_{}",
            face.color[0], face.color[1], face.color[2]
        ),
        color: [r, g, b],
        opacity: a,
        texture: None,
        texture_mapping: None,
    }
}

/// Clean up string for safe OBJ/MTL identifiers without spaces or control characters.
fn sanitize_obj_identifier(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if s.is_empty() {
        "material".to_string()
    } else {
        s
    }
}

// ============================================================================
// OBJ Exporter
// ============================================================================

/// Export document geometry to Wavefront OBJ format with sibling MTL and copied image assets.
///
/// Units: meters, right-handed Z-up.
/// Flattens all visible occurrences.
pub fn export_obj(path: &Path, bundle: &DraftBundle) -> Result<ExportSummary> {
    let history = validate_bundle(bundle)?;
    let occurrences = history.occurrences()?;

    let parent_dir = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));

    let file_stem =
        sanitize_obj_identifier(path.file_stem().and_then(|s| s.to_str()).unwrap_or("model"));
    let mtl_filename = format!("{file_stem}.mtl");
    let mtl_path = parent_dir.join(&mtl_filename);

    let mut used_materials: BTreeMap<String, ResolvedMaterial> = BTreeMap::new();
    let mut written_textures: BTreeMap<String, String> = BTreeMap::new(); // texture slot key -> relative filename

    // First pass: collect used materials and write sibling texture files.
    for occ in &occurrences {
        let Some(model) = history.definition_model(occ.definition) else {
            continue;
        };
        for face in model.faces().values() {
            let res_mat = resolve_face_material(face, occ.material, history.materials());
            if let Some(tex_key) = &res_mat.texture {
                if !written_textures.contains_key(tex_key) {
                    if let Some(asset) = bundle.textures.get(tex_key) {
                        let ext = if asset.mime == "image/png" {
                            "png"
                        } else {
                            "jpg"
                        };
                        let clean_key = written_textures.len();
                        let img_filename = format!("{file_stem}_tex_{clean_key}.{ext}");
                        let img_path = parent_dir.join(&img_filename);
                        atomic_write(&img_path, |f| {
                            f.write_all(&asset.bytes)?;
                            Ok(())
                        })?;
                        written_textures.insert(tex_key.clone(), img_filename);
                    }
                }
            }
            used_materials.entry(res_mat.key.clone()).or_insert(res_mat);
        }
    }

    // Write sibling MTL file.
    atomic_write(&mtl_path, |mtl_file| {
        let mut w = BufWriter::new(mtl_file);
        writeln!(w, "# Cindral MTL export; units: meters, right-handed Z-up")?;
        for mat in used_materials.values() {
            let clean_name = sanitize_obj_identifier(&mat.name);
            let mtl_id = format!("{}_{}", clean_name, sanitize_obj_identifier(&mat.key));
            writeln!(w, "\nnewmtl {mtl_id}")?;
            writeln!(
                w,
                "Kd {:.6} {:.6} {:.6}",
                mat.color[0], mat.color[1], mat.color[2]
            )?;
            writeln!(w, "d {:.6}", mat.opacity)?;
            writeln!(w, "Tr {:.6}", 1.0 - mat.opacity)?;
            writeln!(w, "illum 2")?;
            if let Some(tex_key) = &mat.texture {
                if let Some(img_file) = written_textures.get(tex_key) {
                    writeln!(w, "map_Kd {img_file}")?;
                }
            }
        }
        w.flush()?;
        Ok(())
    })?;

    // Second pass: triangulate and write OBJ file.
    let mut instances = 0;
    let mut triangles = 0;

    atomic_write(path, |obj_file| {
        let mut w = BufWriter::new(obj_file);
        writeln!(w, "# Cindral OBJ export")?;
        writeln!(w, "# Units: meters")?;
        writeln!(w, "# Coordinate system: right-handed Z-up")?;
        writeln!(w, "mtllib {mtl_filename}\n")?;

        let mut v_index: usize = 1;
        let mut vt_index: usize = 1;
        let mut vn_index: usize = 1;

        for (occ_idx, occ) in occurrences.iter().enumerate() {
            let Some(model) = history.definition_model(occ.definition) else {
                continue;
            };
            if model.faces().is_empty() {
                continue;
            }

            let inv_transpose = occ.world.inverse().transpose();
            let mut occ_triangles = 0;

            let occ_name = if occ.path.is_empty() {
                "Root".to_string()
            } else {
                format!("Occurrence_{}", occ_idx)
            };
            writeln!(w, "o {occ_name}")?;
            writeln!(w, "g {occ_name}")?;

            let mut current_mat_key: Option<String> = None;

            for (face_id, face) in model.faces() {
                let tris = model.triangulate(*face_id)?;

                let res_mat = resolve_face_material(face, occ.material, history.materials());
                let clean_name = sanitize_obj_identifier(&res_mat.name);
                let mtl_id = format!("{}_{}", clean_name, sanitize_obj_identifier(&res_mat.key));

                if current_mat_key.as_deref() != Some(&res_mat.key) {
                    writeln!(w, "usemtl {mtl_id}")?;
                    current_mat_key = Some(res_mat.key.clone());
                }

                let normal = transform_normal(inv_transpose, face.plane.normal);

                for tri in tris {
                    let p0 = occ.world.transform_point3(tri[0]);
                    let mut p1 = occ.world.transform_point3(tri[1]);
                    let mut p2 = occ.world.transform_point3(tri[2]);

                    let mapping = res_mat.texture_mapping.as_ref();
                    let uv0 = cindral_core::TextureMapping::map_uv(
                        planar_uv(tri[0], face.plane.normal).map(f64::from),
                        mapping,
                    );
                    let mut uv1 = cindral_core::TextureMapping::map_uv(
                        planar_uv(tri[1], face.plane.normal).map(f64::from),
                        mapping,
                    );
                    let mut uv2 = cindral_core::TextureMapping::map_uv(
                        planar_uv(tri[2], face.plane.normal).map(f64::from),
                        mapping,
                    );

                    if (p1 - p0).cross(p2 - p0).dot(normal) < 0.0 {
                        std::mem::swap(&mut p1, &mut p2);
                        std::mem::swap(&mut uv1, &mut uv2);
                    }

                    // Write 3 vertices
                    writeln!(w, "v {:.6} {:.6} {:.6}", p0.x, p0.y, p0.z)?;
                    writeln!(w, "v {:.6} {:.6} {:.6}", p1.x, p1.y, p1.z)?;
                    writeln!(w, "v {:.6} {:.6} {:.6}", p2.x, p2.y, p2.z)?;

                    // Write 3 UVs. OBJ uses a bottom-left UV origin (V up)
                    // while the viewport and glTF use top-left; invert V as
                    // the final step on the final UVs (after size, rotation
                    // and flip) so consumers show the texture upright.
                    writeln!(w, "vt {:.6} {:.6}", uv0[0], 1.0 - uv0[1])?;
                    writeln!(w, "vt {:.6} {:.6}", uv1[0], 1.0 - uv1[1])?;
                    writeln!(w, "vt {:.6} {:.6}", uv2[0], 1.0 - uv2[1])?;

                    // Write 1 normal
                    writeln!(w, "vn {:.6} {:.6} {:.6}", normal.x, normal.y, normal.z)?;

                    // Write face
                    writeln!(
                        w,
                        "f {}/{}/{} {}/{}/{} {}/{}/{}",
                        v_index,
                        vt_index,
                        vn_index,
                        v_index + 1,
                        vt_index + 1,
                        vn_index,
                        v_index + 2,
                        vt_index + 2,
                        vn_index
                    )?;

                    v_index += 3;
                    vt_index += 3;
                    vn_index += 1;
                    occ_triangles += 1;
                }
            }

            if occ_triangles > 0 {
                instances += 1;
                triangles += occ_triangles;
            }
        }

        w.flush()?;
        Ok(())
    })?;

    Ok(ExportSummary {
        instances,
        triangles,
    })
}

// ============================================================================
// Binary STL Exporter
// ============================================================================

/// Export document geometry to binary STL format.
///
/// Units: meters, right-handed Z-up.
/// Flattens all visible occurrences.
pub fn export_stl(path: &Path, bundle: &DraftBundle) -> Result<ExportSummary> {
    let history = validate_bundle(bundle)?;
    let occurrences = history.occurrences()?;

    let mut instances = 0;
    let mut triangles = 0;
    let mut facet_data: Vec<u8> = Vec::new();

    for occ in &occurrences {
        let Some(model) = history.definition_model(occ.definition) else {
            continue;
        };
        if model.faces().is_empty() {
            continue;
        }

        let inv_transpose = occ.world.inverse().transpose();
        let mut occ_triangles = 0;

        for (face_id, face) in model.faces() {
            let tris = model.triangulate(*face_id)?;

            let normal = transform_normal(inv_transpose, face.plane.normal);
            let n = [normal.x as f32, normal.y as f32, normal.z as f32];

            for tri in tris {
                let p0 = occ.world.transform_point3(tri[0]);
                let mut p1 = occ.world.transform_point3(tri[1]);
                let mut p2 = occ.world.transform_point3(tri[2]);

                if (p1 - p0).cross(p2 - p0).dot(normal) < 0.0 {
                    std::mem::swap(&mut p1, &mut p2);
                }

                // 50 bytes per facet:
                // Normal (3 * f32)
                facet_data.extend_from_slice(&n[0].to_le_bytes());
                facet_data.extend_from_slice(&n[1].to_le_bytes());
                facet_data.extend_from_slice(&n[2].to_le_bytes());
                // Vertex 1 (3 * f32)
                facet_data.extend_from_slice(&(p0.x as f32).to_le_bytes());
                facet_data.extend_from_slice(&(p0.y as f32).to_le_bytes());
                facet_data.extend_from_slice(&(p0.z as f32).to_le_bytes());
                // Vertex 2 (3 * f32)
                facet_data.extend_from_slice(&(p1.x as f32).to_le_bytes());
                facet_data.extend_from_slice(&(p1.y as f32).to_le_bytes());
                facet_data.extend_from_slice(&(p1.z as f32).to_le_bytes());
                // Vertex 3 (3 * f32)
                facet_data.extend_from_slice(&(p2.x as f32).to_le_bytes());
                facet_data.extend_from_slice(&(p2.y as f32).to_le_bytes());
                facet_data.extend_from_slice(&(p2.z as f32).to_le_bytes());
                // Attribute byte count (u16 = 0)
                facet_data.extend_from_slice(&0u16.to_le_bytes());

                occ_triangles += 1;
            }
        }

        if occ_triangles > 0 {
            instances += 1;
            triangles += occ_triangles;
        }
    }

    if triangles > u32::MAX as usize {
        return Err(IoError::Invalid(
            "triangle count exceeds binary STL u32 limit".into(),
        ));
    }

    atomic_write(path, |file| {
        let mut w = BufWriter::new(file);
        let mut header = [0u8; 80];
        let copy_len = STL_HEADER.len().min(80);
        header[..copy_len].copy_from_slice(&STL_HEADER[..copy_len]);
        w.write_all(&header)?;
        w.write_all(&(triangles as u32).to_le_bytes())?;
        w.write_all(&facet_data)?;
        w.flush()?;
        Ok(())
    })?;

    Ok(ExportSummary {
        instances,
        triangles,
    })
}

// ============================================================================
// Self-contained glTF 2.0 Exporter
// ============================================================================

/// Raw vertex record for glTF mesh construction.
#[derive(Clone, Copy, Debug)]
struct GltfVertex {
    pos: [f32; 3],
    norm: [f32; 3],
    uv: [f32; 2],
}

/// A primitive slice inside a definition's local geometry.
struct DefinitionPrimitive {
    /// Resolved material slot or ID.
    resolved_material: ResolvedMaterial,
    /// Indices referencing the definition's vertex buffer.
    indices: Vec<u32>,
}

/// Precomputed definition geometry for glTF sharing.
struct DefinitionGeometry {
    vertices: Vec<GltfVertex>,
    /// Primitives grouped by material, with indices into `vertices`.
    primitives: Vec<DefinitionPrimitive>,
    pos_min: [f32; 3],
    pos_max: [f32; 3],
    norm_min: [f32; 3],
    norm_max: [f32; 3],
    uv_min: [f32; 2],
    uv_max: [f32; 2],
}

/// Export document geometry to self-contained glTF 2.0 format (.gltf).
///
/// Units: meters, right-handed Y-up with root conversion matrix `(x, y, z) -> (x, z, -y)`.
/// Shares meshes and accessors across repeated definitions.
/// Embeds all binary buffers and texture images as data URIs.
pub fn export_gltf(path: &Path, bundle: &DraftBundle) -> Result<ExportSummary> {
    let history = validate_bundle(bundle)?;
    let occurrences = history.occurrences()?;
    for occurrence in &occurrences {
        if let Some(node) = history.instance(&occurrence.path) {
            let m = node.transform;
            let axes = [
                m.x_axis.truncate(),
                m.y_axis.truncate(),
                m.z_axis.truncate(),
            ];
            let normalized = axes.map(|axis| axis.try_normalize());
            let [Some(x), Some(y), Some(z)] = normalized else {
                return Err(IoError::Invalid(
                    "glTF cannot represent this degenerate transform".into(),
                ));
            };
            if !m.as_mat4().is_finite()
                || m.as_mat4().determinant() == 0.
                || x.dot(y).abs() > 1e-7
                || x.dot(z).abs() > 1e-7
                || y.dot(z).abs() > 1e-7
            {
                return Err(IoError::Invalid("glTF node transforms require finite float32 TRS matrices; shear is unsupported".into()));
            }
        }
    }

    // 1. Calculate occurrence statistics.
    let mut instances = 0;
    let mut triangles = 0;
    for occ in &occurrences {
        if let Some(model) = history.definition_model(occ.definition) {
            let mut occ_tris = 0;
            for face_id in model.faces().keys() {
                occ_tris += model.triangulate(*face_id)?.len();
            }
            if occ_tris > 0 {
                instances += 1;
                triangles += occ_tris;
            }
        }
    }

    // 2. Build local geometry for all definitions that contain faces.
    // For each definition, build geometry once in definition-local space.
    let mut def_geometries: BTreeMap<DefinitionId, DefinitionGeometry> = BTreeMap::new();

    for &def_id in bundle.document.definitions.keys() {
        let Some(model) = history.definition_model(def_id) else {
            continue;
        };
        if model.faces().is_empty() {
            continue;
        }

        let mut vertices: Vec<GltfVertex> = Vec::new();
        let mut pos_min = [f32::INFINITY; 3];
        let mut pos_max = [f32::NEG_INFINITY; 3];
        let mut norm_min = [f32::INFINITY; 3];
        let mut norm_max = [f32::NEG_INFINITY; 3];
        let mut uv_min = [f32::INFINITY; 2];
        let mut uv_max = [f32::NEG_INFINITY; 2];

        // Group triangles by resolved material (assuming default unpainted inheritance for base definition).
        // Each face stores its local vertices and indices.
        let mut prim_map: BTreeMap<String, (ResolvedMaterial, Vec<u32>)> = BTreeMap::new();
        let mut total_tris = 0;

        for (face_id, face) in model.faces() {
            let tris = model.triangulate(*face_id)?;

            let normal = face.plane.normal;
            let norm = [normal.x as f32, normal.y as f32, normal.z as f32];

            // In definition local space, use face's explicit material or default color.
            let res_mat = resolve_face_material(face, None, history.materials());
            let mapping = res_mat.texture_mapping;
            let entry = prim_map
                .entry(res_mat.key.clone())
                .or_insert_with(|| (res_mat, Vec::new()));

            for tri in tris {
                let mapping = mapping.as_ref();

                let uv0 = cindral_core::TextureMapping::map_uv(
                    planar_uv(tri[0], normal).map(f64::from),
                    mapping,
                );
                let uv1 = cindral_core::TextureMapping::map_uv(
                    planar_uv(tri[1], normal).map(f64::from),
                    mapping,
                );
                let uv2 = cindral_core::TextureMapping::map_uv(
                    planar_uv(tri[2], normal).map(f64::from),
                    mapping,
                );

                let pts = [(tri[0], uv0), (tri[1], uv1), (tri[2], uv2)];

                for (p, uv) in pts {
                    let uv = [uv[0] as f32, uv[1] as f32];
                    let pos = [p.x as f32, p.y as f32, p.z as f32];
                    for i in 0..3 {
                        pos_min[i] = pos_min[i].min(pos[i]);
                        pos_max[i] = pos_max[i].max(pos[i]);
                        norm_min[i] = norm_min[i].min(norm[i]);
                        norm_max[i] = norm_max[i].max(norm[i]);
                    }
                    for i in 0..2 {
                        uv_min[i] = uv_min[i].min(uv[i]);
                        uv_max[i] = uv_max[i].max(uv[i]);
                    }

                    let idx = vertices.len() as u32;
                    vertices.push(GltfVertex { pos, norm, uv });
                    entry.1.push(idx);
                }
                total_tris += 1;
            }
        }

        if total_tris > 0 {
            let primitives = prim_map
                .into_values()
                .map(|(resolved_material, indices)| DefinitionPrimitive {
                    resolved_material,
                    indices,
                })
                .collect();

            def_geometries.insert(
                def_id,
                DefinitionGeometry {
                    vertices,
                    primitives,
                    pos_min,
                    pos_max,
                    norm_min,
                    norm_max,
                    uv_min,
                    uv_max,
                },
            );
        }
    }

    // 3. Register materials, textures, and images.
    let mut images: Vec<Value> = Vec::new();
    let mut image_map: HashMap<String, usize> = HashMap::new(); // texture slot key -> image index

    for (key, asset) in &bundle.textures {
        let b64 = BASE64_STANDARD.encode(&asset.bytes);
        let uri = format!("data:{};base64,{}", asset.mime, b64);
        let img_idx = images.len();
        images.push(json!({
            "mimeType": asset.mime,
            "uri": uri,
        }));
        image_map.insert(key.clone(), img_idx);
    }

    let mut textures: Vec<Value> = Vec::new();
    let mut texture_map: HashMap<String, usize> = HashMap::new(); // texture slot key -> texture index

    for (key, &img_idx) in &image_map {
        let tex_idx = textures.len();
        textures.push(json!({
            "sampler": 0,
            "source": img_idx,
        }));
        texture_map.insert(key.clone(), tex_idx);
    }

    let mut materials: Vec<Value> = Vec::new();
    let mut material_map: HashMap<String, usize> = HashMap::new(); // resolved material key -> gltf material index

    let register_material = |mat: &ResolvedMaterial,
                             materials: &mut Vec<Value>,
                             material_map: &mut HashMap<String, usize>,
                             texture_map: &HashMap<String, usize>|
     -> usize {
        if let Some(&idx) = material_map.get(&mat.key) {
            return idx;
        }

        // glTF baseColorFactor is linear light; ResolvedMaterial colors are sRGB.
        let linear = [
            cindral_core::srgb_to_linear(mat.color[0]),
            cindral_core::srgb_to_linear(mat.color[1]),
            cindral_core::srgb_to_linear(mat.color[2]),
        ];
        let mut pbr = json!({
            "baseColorFactor": [linear[0], linear[1], linear[2], mat.opacity],
            "metallicFactor": 0.0,
            "roughnessFactor": 0.5,
        });

        if let Some(tex_key) = &mat.texture {
            if let Some(&tex_idx) = texture_map.get(tex_key) {
                pbr["baseColorTexture"] = json!({ "index": tex_idx });
            }
        }

        let alpha_mode = if mat.opacity < 1.0 || mat.texture.is_some() {
            "BLEND"
        } else {
            "OPAQUE"
        };
        let mat_val = json!({
            "name": mat.name,
            "pbrMetallicRoughness": pbr,
            "alphaMode": alpha_mode,
            "doubleSided": true,
        });

        let idx = materials.len();
        materials.push(mat_val);
        material_map.insert(mat.key.clone(), idx);
        idx
    };

    // 4. Build binary buffers, bufferViews, and accessors.
    let mut buffer_bytes: Vec<u8> = Vec::new();
    let mut buffer_views: Vec<Value> = Vec::new();
    let mut accessors: Vec<Value> = Vec::new();

    // Store accessor indices for each definition geometry:
    // (pos_acc, norm_acc, uv_acc, Vec<index_acc>)
    struct DefAccessors {
        pos_acc: usize,
        norm_acc: usize,
        uv_acc: usize,
        index_accs: Vec<(usize, usize)>, // (index_accessor, index_count)
    }

    let mut def_accessors: BTreeMap<DefinitionId, DefAccessors> = BTreeMap::new();

    for (&def_id, geom) in &def_geometries {
        // Pack vertices: pos (12 bytes), norm (12 bytes), uv (8 bytes) = 32 bytes per vertex
        let v_count = geom.vertices.len();
        let pos_offset = buffer_bytes.len();
        for v in &geom.vertices {
            for c in v.pos {
                buffer_bytes.extend_from_slice(&c.to_le_bytes());
            }
        }
        let pos_view_idx = buffer_views.len();
        buffer_views.push(json!({
            "buffer": 0,
            "byteOffset": pos_offset,
            "byteLength": v_count * 12,
            "target": 34962, // ARRAY_BUFFER
        }));
        let pos_acc_idx = accessors.len();
        accessors.push(json!({
            "bufferView": pos_view_idx,
            "byteOffset": 0,
            "componentType": 5126, // FLOAT
            "count": v_count,
            "type": "VEC3",
            "min": geom.pos_min,
            "max": geom.pos_max,
        }));

        let norm_offset = buffer_bytes.len();
        for v in &geom.vertices {
            for c in v.norm {
                buffer_bytes.extend_from_slice(&c.to_le_bytes());
            }
        }
        let norm_view_idx = buffer_views.len();
        buffer_views.push(json!({
            "buffer": 0,
            "byteOffset": norm_offset,
            "byteLength": v_count * 12,
            "target": 34962,
        }));
        let norm_acc_idx = accessors.len();
        accessors.push(json!({
            "bufferView": norm_view_idx,
            "byteOffset": 0,
            "componentType": 5126,
            "count": v_count,
            "type": "VEC3",
            "min": geom.norm_min,
            "max": geom.norm_max,
        }));

        let uv_offset = buffer_bytes.len();
        for v in &geom.vertices {
            for c in v.uv {
                buffer_bytes.extend_from_slice(&c.to_le_bytes());
            }
        }
        let uv_view_idx = buffer_views.len();
        buffer_views.push(json!({
            "buffer": 0,
            "byteOffset": uv_offset,
            "byteLength": v_count * 8,
            "target": 34962,
        }));
        let uv_acc_idx = accessors.len();
        accessors.push(json!({
            "bufferView": uv_view_idx,
            "byteOffset": 0,
            "componentType": 5126,
            "count": v_count,
            "type": "VEC2",
            "min": geom.uv_min,
            "max": geom.uv_max,
        }));

        let mut index_accs = Vec::new();
        for prim in &geom.primitives {
            let i_count = prim.indices.len();
            let mut i_min = u32::MAX;
            let mut i_max = 0;
            let idx_offset = buffer_bytes.len();
            for &idx in &prim.indices {
                i_min = i_min.min(idx);
                i_max = i_max.max(idx);
                buffer_bytes.extend_from_slice(&idx.to_le_bytes());
            }
            // Pad to 4-byte boundary
            while !buffer_bytes.len().is_multiple_of(4) {
                buffer_bytes.push(0);
            }
            let idx_view_idx = buffer_views.len();
            buffer_views.push(json!({
                "buffer": 0,
                "byteOffset": idx_offset,
                "byteLength": i_count * 4,
                "target": 34963, // ELEMENT_ARRAY_BUFFER
            }));
            let idx_acc_idx = accessors.len();
            accessors.push(json!({
                "bufferView": idx_view_idx,
                "byteOffset": 0,
                "componentType": 5125, // UNSIGNED_INT
                "count": i_count,
                "type": "SCALAR",
                "min": [i_min],
                "max": [i_max],
            }));
            index_accs.push((idx_acc_idx, i_count));
        }

        def_accessors.insert(
            def_id,
            DefAccessors {
                pos_acc: pos_acc_idx,
                norm_acc: norm_acc_idx,
                uv_acc: uv_acc_idx,
                index_accs,
            },
        );
    }

    // 5. Build glTF meshes for definitions and material variants.
    // Key: (DefinitionId, Option<MaterialId>) -> glTF mesh index.
    // Material variants share the exact same accessors!
    let mut meshes: Vec<Value> = Vec::new();
    let mut mesh_variants: HashMap<(DefinitionId, Option<MaterialId>), usize> = HashMap::new();

    let get_or_create_mesh =
        |def_id: DefinitionId,
         inherited_mat: Option<MaterialId>,
         meshes: &mut Vec<Value>,
         mesh_variants: &mut HashMap<(DefinitionId, Option<MaterialId>), usize>,
         def_geometries: &BTreeMap<DefinitionId, DefinitionGeometry>,
         def_accessors: &BTreeMap<DefinitionId, DefAccessors>,
         materials: &mut Vec<Value>,
         material_map: &mut HashMap<String, usize>,
         texture_map: &HashMap<String, usize>,
         bundle_materials: &BTreeMap<MaterialId, Material>|
         -> Option<usize> {
            let geom = def_geometries.get(&def_id)?;
            let accs = def_accessors.get(&def_id)?;

            let key = (def_id, inherited_mat);
            if let Some(&mesh_idx) = mesh_variants.get(&key) {
                return Some(mesh_idx);
            }

            let mut primitives = Vec::new();
            for (prim_idx, prim) in geom.primitives.iter().enumerate() {
                let (idx_acc, _) = accs.index_accs[prim_idx];
                // If the face had an explicit material, use it; otherwise use inherited_mat
                let effective_mat = if prim.resolved_material.key.starts_with("mat_") {
                    prim.resolved_material.clone()
                } else if let Some(inh_id) =
                    inherited_mat.filter(|_| prim.resolved_material.key == "color_160_177_193_255")
                {
                    if let Some(mat) = bundle_materials.get(&inh_id) {
                        ResolvedMaterial {
                            key: format!("mat_{}", inh_id.0),
                            name: if mat.name.is_empty() {
                                format!("mat_{}", inh_id.0)
                            } else {
                                mat.name.clone()
                            },
                            color: crate::srgb_floats(mat.color),
                            opacity: mat.opacity,
                            texture: mat.texture.clone(),
                            texture_mapping: mat.texture_mapping,
                        }
                    } else {
                        prim.resolved_material.clone()
                    }
                } else {
                    prim.resolved_material.clone()
                };

                let mat_idx =
                    register_material(&effective_mat, materials, material_map, texture_map);

                primitives.push(json!({
                    "attributes": {
                        "POSITION": accs.pos_acc,
                        "NORMAL": accs.norm_acc,
                        "TEXCOORD_0": accs.uv_acc,
                    },
                    "indices": idx_acc,
                    "material": mat_idx,
                }));
            }

            let mesh_idx = meshes.len();
            meshes.push(json!({
                "name": format!("Mesh_def_{}", def_id.0),
                "primitives": primitives,
            }));
            mesh_variants.insert(key, mesh_idx);
            Some(mesh_idx)
        };

    // 6. Build nested node hierarchy.
    // Node 0: Root coordinate conversion matrix.
    // Children of Node 0: top-level placed instances and loose root geometry.
    let mut nodes: Vec<Value> = Vec::new();
    let root_node_idx = 0;
    nodes.push(json!({
        "name": "RootCoordinateConversion",
        "matrix": GLTF_ROOT_CONVERSION_MATRIX,
        "children": [],
    }));

    // If root definition has geometry, attach a root geometry node.
    let root_def_id = bundle.document.root;
    if let Some(mesh_idx) = get_or_create_mesh(
        root_def_id,
        None,
        &mut meshes,
        &mut mesh_variants,
        &def_geometries,
        &def_accessors,
        &mut materials,
        &mut material_map,
        &texture_map,
        &bundle.document.materials,
    ) {
        let root_geom_node = nodes.len();
        nodes.push(json!({
            "name": "RootGeometry",
            "mesh": mesh_idx,
        }));
        if let Some(children) = nodes[root_node_idx]["children"].as_array_mut() {
            children.push(json!(root_geom_node));
        }
    }

    // Helper to recursively instantiate visible instances.
    #[allow(clippy::too_many_arguments, clippy::type_complexity)] // Explicit exporter traversal state, with shared accessor ownership.
    fn build_instance_nodes(
        def_id: DefinitionId,
        inherited_mat: Option<MaterialId>,
        nodes: &mut Vec<Value>,
        meshes: &mut Vec<Value>,
        mesh_variants: &mut HashMap<(DefinitionId, Option<MaterialId>), usize>,
        def_geometries: &BTreeMap<DefinitionId, DefinitionGeometry>,
        def_accessors: &BTreeMap<DefinitionId, DefAccessors>,
        materials: &mut Vec<Value>,
        material_map: &mut HashMap<String, usize>,
        texture_map: &HashMap<String, usize>,
        doc: &DocumentData,
        get_or_create_mesh: &dyn Fn(
            DefinitionId,
            Option<MaterialId>,
            &mut Vec<Value>,
            &mut HashMap<(DefinitionId, Option<MaterialId>), usize>,
            &BTreeMap<DefinitionId, DefinitionGeometry>,
            &BTreeMap<DefinitionId, DefAccessors>,
            &mut Vec<Value>,
            &mut HashMap<String, usize>,
            &HashMap<String, usize>,
            &BTreeMap<MaterialId, Material>,
        ) -> Option<usize>,
    ) -> Vec<usize> {
        let Some(def) = doc.definitions.get(&def_id) else {
            return Vec::new();
        };

        let mut child_node_indices = Vec::new();

        for inst in def.instances.values() {
            // Check visibility
            if inst.hidden {
                continue;
            }
            if let Some(tag_id) = inst.tag {
                if let Some(tag) = doc.tags.get(&tag_id) {
                    if !tag.visible {
                        continue;
                    }
                }
            }

            let effective_mat = inst.material.or(inherited_mat);

            // Get mesh for target definition if it has geometry
            let mesh_idx = get_or_create_mesh(
                inst.definition,
                effective_mat,
                meshes,
                mesh_variants,
                def_geometries,
                def_accessors,
                materials,
                material_map,
                texture_map,
                &doc.materials,
            );

            // Recursively build children
            let grandchildren = build_instance_nodes(
                inst.definition,
                effective_mat,
                nodes,
                meshes,
                mesh_variants,
                def_geometries,
                def_accessors,
                materials,
                material_map,
                texture_map,
                doc,
                get_or_create_mesh,
            );

            let node_idx = nodes.len();
            let matrix_f32: Vec<f32> = inst
                .transform
                .to_cols_array()
                .iter()
                .map(|&v| v as f32)
                .collect();

            let mut node_json = json!({
                "name": inst.name,
                "matrix": matrix_f32,
            });

            if let Some(m_idx) = mesh_idx {
                node_json["mesh"] = json!(m_idx);
            }
            if !grandchildren.is_empty() {
                node_json["children"] = json!(grandchildren);
            }

            nodes.push(node_json);
            child_node_indices.push(node_idx);
        }

        child_node_indices
    }

    let top_children = build_instance_nodes(
        root_def_id,
        None,
        &mut nodes,
        &mut meshes,
        &mut mesh_variants,
        &def_geometries,
        &def_accessors,
        &mut materials,
        &mut material_map,
        &texture_map,
        &bundle.document,
        &get_or_create_mesh,
    );

    if let Some(children) = nodes[root_node_idx]["children"].as_array_mut() {
        for idx in top_children {
            children.push(json!(idx));
        }
    }

    if nodes[0]["children"].as_array().is_some_and(Vec::is_empty) {
        if let Some(root) = nodes[0].as_object_mut() {
            root.remove("children");
        }
    }

    // 7. Base64 encode the binary buffer.
    let mut buffers: Vec<Value> = Vec::new();
    if !buffer_bytes.is_empty() {
        let b64 = BASE64_STANDARD.encode(&buffer_bytes);
        buffers.push(json!({
            "byteLength": buffer_bytes.len(),
            "uri": format!("data:application/octet-stream;base64,{}", b64),
        }));
    }

    let samplers = vec![json!({
        "wrapS": 10497, // REPEAT
        "wrapT": 10497, // REPEAT
    })];

    // 8. Assemble final glTF JSON.
    let mut gltf = json!({
        "asset": {
            "version": "2.0",
            "generator": "Cindral glTF Exporter",
        },
        "scene": 0,
        "scenes": [
            { "nodes": [0] }
        ],
        "nodes": nodes,
    });

    if !meshes.is_empty() {
        gltf["meshes"] = json!(meshes);
    }
    if !accessors.is_empty() {
        gltf["accessors"] = json!(accessors);
    }
    if !buffer_views.is_empty() {
        gltf["bufferViews"] = json!(buffer_views);
    }
    if !buffers.is_empty() {
        gltf["buffers"] = json!(buffers);
    }
    if !materials.is_empty() {
        gltf["materials"] = json!(materials);
    }
    if !textures.is_empty() {
        gltf["textures"] = json!(textures);
        gltf["samplers"] = json!(samplers);
    }
    if !images.is_empty() {
        gltf["images"] = json!(images);
    }

    atomic_write(path, |file| {
        let mut w = BufWriter::new(file);
        serde_json::to_writer_pretty(&mut w, &gltf).map_err(|e| IoError::Invalid(e.to_string()))?;
        w.flush()?;
        Ok(())
    })?;

    Ok(ExportSummary {
        instances,
        triangles,
    })
}
