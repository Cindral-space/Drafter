//! Bounded Wavefront OBJ and MTL parser into native DraftBundle documents.

use crate::{load_texture, validate_bundle, DraftBundle, IoError, Result, TextureAsset};
use cindral_core::{
    DefinitionData, DefinitionId, DefinitionKind, DocumentData, Edge, EdgeId, Face, FaceId,
    GeometryData, Instance, InstanceId, Material, MaterialId, Plane, Vertex, VertexId,
    MAX_COORDINATE, TOLERANCE,
};
use glam::{DMat4, DVec2, DVec3};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    fs::File,
    io::Read,
    path::{Component, Path, PathBuf},
};

/// Maximum allowed OBJ file size (32 MiB).
const MAX_OBJ_BYTES: usize = 32 * 1024 * 1024;
/// Maximum allowed MTL file size (4 MiB).
const MAX_MTL_BYTES: usize = 4 * 1024 * 1024;
/// Maximum vertices allowed per import to prevent memory exhaustion.
const MAX_VERTICES: usize = 1_000_000;
/// Maximum faces allowed per import.
const MAX_FACES: usize = 1_000_000;

/// Safely resolve a relative path strictly within `base_dir`.
/// Rejects absolute paths, parent directories (`..`), network prefixes, control characters, and escapes.
fn safe_resolve_under(base_dir: &Path, rel_str: &str) -> Result<PathBuf> {
    let trimmed = rel_str.trim();
    if trimmed.is_empty() {
        return Err(IoError::Invalid("empty relative path".into()));
    }
    if trimmed.len() > 4096 || trimmed.chars().any(char::is_control) {
        return Err(IoError::Invalid("path contains invalid characters".into()));
    }
    if trimmed.starts_with('/') || trimmed.starts_with('\\') || trimmed.contains(':') {
        return Err(IoError::Invalid(
            "absolute, drive-letter, or network paths are forbidden".into(),
        ));
    }
    let rel_path = Path::new(trimmed);
    for component in rel_path.components() {
        match component {
            Component::Normal(_) => {}
            _ => {
                return Err(IoError::Invalid(
                    "path contains non-normal components (parent or root navigation)".into(),
                ));
            }
        }
    }
    let candidate = base_dir.join(rel_path);
    let canonical_base = base_dir
        .canonicalize()
        .map_err(|e| IoError::Invalid(format!("failed to canonicalize base dir: {e}")))?;
    if candidate.exists() {
        let canonical_target = candidate
            .canonicalize()
            .map_err(|e| IoError::Invalid(format!("failed to canonicalize target path: {e}")))?;
        if !canonical_target.starts_with(&canonical_base) {
            return Err(IoError::Invalid(
                "target path escapes the base directory".into(),
            ));
        }
        Ok(canonical_target)
    } else {
        Ok(candidate)
    }
}

/// Validate a human-readable identifier.
fn validate_name(value: &str) -> Result<String> {
    let s = value.trim();
    if s.is_empty() || s.len() > 1024 || s.chars().any(char::is_control) {
        Err(IoError::Invalid(
            "name must be nonempty, at most 1024 bytes, without control characters".into(),
        ))
    } else {
        Ok(s.into())
    }
}

/// In-progress material definition from an MTL file.
#[derive(Clone, Debug)]
struct MtlDef {
    name: String,
    color: [f32; 3],
    opacity: f32,
    texture: Option<String>,
}

impl Default for MtlDef {
    fn default() -> Self {
        Self {
            name: String::new(),
            color: [0.8, 0.8, 0.8],
            opacity: 1.0,
            texture: None,
        }
    }
}

/// Parse an MTL file's contents into material records and loaded texture assets.
fn parse_mtl(
    content: &str,
    base_dir: Option<&Path>,
    textures: &mut BTreeMap<String, TextureAsset>,
) -> Result<BTreeMap<String, MtlDef>> {
    let mut materials: BTreeMap<String, MtlDef> = BTreeMap::new();
    let mut current_mtl: Option<MtlDef> = None;

    for line in content.lines() {
        let line = line.split('#').next().unwrap_or_default().trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut tokens = line.split_whitespace();
        let Some(tag) = tokens.next() else {
            continue;
        };

        match tag {
            "newmtl" => {
                if let Some(prev) = current_mtl.take() {
                    materials.insert(prev.name.clone(), prev);
                }
                let rest = line["newmtl".len()..].trim();
                let name = validate_name(rest)?;
                current_mtl = Some(MtlDef {
                    name,
                    ..MtlDef::default()
                });
            }
            "Kd" => {
                let Some(mtl) = current_mtl.as_mut() else {
                    return Err(IoError::Invalid("Kd directive before newmtl".into()));
                };
                let r: f32 = tokens
                    .next()
                    .ok_or_else(|| IoError::Invalid("missing red in Kd".into()))?
                    .parse()
                    .map_err(|_| IoError::Invalid("invalid float in Kd".into()))?;
                let g: f32 = tokens
                    .next()
                    .ok_or_else(|| IoError::Invalid("missing green in Kd".into()))?
                    .parse()
                    .map_err(|_| IoError::Invalid("invalid float in Kd".into()))?;
                let b: f32 = tokens
                    .next()
                    .ok_or_else(|| IoError::Invalid("missing blue in Kd".into()))?
                    .parse()
                    .map_err(|_| IoError::Invalid("invalid float in Kd".into()))?;
                if tokens.next().is_some() {
                    return Err(IoError::Invalid("excess arguments in Kd".into()));
                }
                if !r.is_finite()
                    || !g.is_finite()
                    || !b.is_finite()
                    || !(0.0..=1.0).contains(&r)
                    || !(0.0..=1.0).contains(&g)
                    || !(0.0..=1.0).contains(&b)
                {
                    return Err(IoError::Invalid("Kd color values must be in 0..=1".into()));
                }
                mtl.color = [r, g, b];
            }
            "d" => {
                let Some(mtl) = current_mtl.as_mut() else {
                    return Err(IoError::Invalid("d directive before newmtl".into()));
                };
                let alpha: f32 = tokens
                    .next()
                    .ok_or_else(|| IoError::Invalid("missing alpha in d".into()))?
                    .parse()
                    .map_err(|_| IoError::Invalid("invalid float in d".into()))?;
                if !alpha.is_finite() || !(0.0..=1.0).contains(&alpha) {
                    return Err(IoError::Invalid("d opacity must be in 0..=1".into()));
                }
                mtl.opacity = alpha;
            }
            "Tr" => {
                let Some(mtl) = current_mtl.as_mut() else {
                    return Err(IoError::Invalid("Tr directive before newmtl".into()));
                };
                let tr: f32 = tokens
                    .next()
                    .ok_or_else(|| IoError::Invalid("missing transparency in Tr".into()))?
                    .parse()
                    .map_err(|_| IoError::Invalid("invalid float in Tr".into()))?;
                if !tr.is_finite() || !(0.0..=1.0).contains(&tr) {
                    return Err(IoError::Invalid("Tr transparency must be in 0..=1".into()));
                }
                mtl.opacity = 1.0 - tr;
            }
            "map_Kd" => {
                let Some(mtl) = current_mtl.as_mut() else {
                    return Err(IoError::Invalid("map_Kd directive before newmtl".into()));
                };
                let rest = line["map_Kd".len()..].trim();
                let Some(base) = base_dir else {
                    return Err(IoError::Invalid(
                        "cannot load map_Kd texture in string import mode".into(),
                    ));
                };
                let tex_path = safe_resolve_under(base, rest)?;
                let (key, asset) = load_texture(&tex_path)?;
                textures.insert(key.clone(), asset);
                if textures.len() > 127 {
                    return Err(IoError::Invalid("too many texture assets".into()));
                }
                mtl.texture = Some(key);
            }
            _ => {
                // Ignore other standard MTL directives (Ka, Ks, Ns, illum, etc.)
            }
        }
    }
    if let Some(prev) = current_mtl {
        materials.insert(prev.name.clone(), prev);
    }
    Ok(materials)
}

/// Unprocessed face read from an OBJ stream.
#[derive(Clone, Debug)]
struct RawFace {
    vertex_indices: Vec<usize>,
    material: Option<String>,
}

/// Epsilon-aware 3D vertex welder.
struct Welder {
    vertices: BTreeMap<VertexId, Vertex>,
    grid: HashMap<(i64, i64, i64), Vec<(VertexId, DVec3)>>,
    next_id: u64,
}

impl Welder {
    fn new(start_id: u64) -> Self {
        Self {
            vertices: BTreeMap::new(),
            grid: HashMap::new(),
            next_id: start_id,
        }
    }

    fn intern(&mut self, pos: DVec3) -> VertexId {
        let cell = TOLERANCE;
        let ix = (pos.x / cell).floor() as i64;
        let iy = (pos.y / cell).floor() as i64;
        let iz = (pos.z / cell).floor() as i64;

        let mut closest: Option<(VertexId, f64)> = None;
        for dx in -1..=1 {
            for dy in -1..=1 {
                for dz in -1..=1 {
                    if let Some(list) = self.grid.get(&(ix + dx, iy + dy, iz + dz)) {
                        for &(id, existing_pos) in list {
                            let dist = existing_pos.distance(pos);
                            if dist <= TOLERANCE {
                                if let Some((_, best_dist)) = closest {
                                    if dist < best_dist {
                                        closest = Some((id, dist));
                                    }
                                } else {
                                    closest = Some((id, dist));
                                }
                            }
                        }
                    }
                }
            }
        }

        if let Some((id, _)) = closest {
            return id;
        }

        let id = VertexId::from_raw(self.next_id);
        self.next_id += 1;
        self.vertices.insert(id, Vertex { position: pos });
        self.grid.entry((ix, iy, iz)).or_default().push((id, pos));
        id
    }

    fn position(&self, id: VertexId) -> DVec3 {
        self.vertices[&id].position
    }
}

/// Document-level entity ID allocator.
struct DocumentAllocator {
    next: u64,
}

impl DocumentAllocator {
    fn new(start: u64) -> Self {
        Self { next: start }
    }

    fn alloc(&mut self) -> Result<u64> {
        let id = self.next;
        self.next = self
            .next
            .checked_add(1)
            .ok_or_else(|| IoError::Invalid("document entity ID overflow".into()))?;
        Ok(id)
    }
}

/// Parse Wavefront OBJ text into a DraftBundle.
fn import_obj_internal(text: &str, base_dir: Option<&Path>) -> Result<DraftBundle> {
    if text.len() > MAX_OBJ_BYTES {
        return Err(IoError::Invalid(
            "OBJ input exceeds maximum size limit (32 MiB)".into(),
        ));
    }

    let mut textures: BTreeMap<String, TextureAsset> = BTreeMap::new();
    let mut mtl_defs: BTreeMap<String, MtlDef> = BTreeMap::new();

    let mut v_positions: Vec<DVec3> = Vec::new();
    let mut vt_count: usize = 0;
    let mut vn_count: usize = 0;

    let mut current_group: Option<String> = None;
    let mut current_material: Option<String> = None;

    // Preserve order of group creation
    let mut group_order: Vec<Option<String>> = Vec::new();
    let mut group_faces: HashMap<Option<String>, Vec<RawFace>> = HashMap::new();

    let mut total_faces: usize = 0;

    for line in text.lines() {
        let line = line.split('#').next().unwrap_or_default().trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut tokens = line.split_whitespace();
        let Some(directive) = tokens.next() else {
            continue;
        };

        match directive {
            "v" => {
                if v_positions.len() >= MAX_VERTICES {
                    return Err(IoError::Invalid("too many vertices in OBJ file".into()));
                }
                let x_str = tokens
                    .next()
                    .ok_or_else(|| IoError::Invalid("missing x coordinate in v".into()))?;
                let y_str = tokens
                    .next()
                    .ok_or_else(|| IoError::Invalid("missing y coordinate in v".into()))?;
                let z_str = tokens
                    .next()
                    .ok_or_else(|| IoError::Invalid("missing z coordinate in v".into()))?;
                let mut x: f64 = x_str
                    .parse()
                    .map_err(|_| IoError::Invalid("invalid float in v.x".into()))?;
                let mut y: f64 = y_str
                    .parse()
                    .map_err(|_| IoError::Invalid("invalid float in v.y".into()))?;
                let mut z: f64 = z_str
                    .parse()
                    .map_err(|_| IoError::Invalid("invalid float in v.z".into()))?;
                if let Some(w_str) = tokens.next() {
                    let w: f64 = w_str
                        .parse()
                        .map_err(|_| IoError::Invalid("invalid float in v.w".into()))?;
                    if !w.is_finite() || w == 0.0 {
                        return Err(IoError::Invalid(
                            "w component in vertex must be nonzero and finite".into(),
                        ));
                    }
                    x /= w;
                    y /= w;
                    z /= w;
                }
                if tokens.next().is_some() {
                    return Err(IoError::Invalid("excess arguments in v line".into()));
                }
                let pos = DVec3::new(x, y, z);
                if !pos.is_finite() || pos.abs().max_element() > MAX_COORDINATE {
                    return Err(IoError::Invalid(
                        "vertex coordinate is nonfinite or exceeds maximum coordinate range".into(),
                    ));
                }
                v_positions.push(pos);
            }
            "vt" => {
                let u_str = tokens
                    .next()
                    .ok_or_else(|| IoError::Invalid("missing u in vt".into()))?;
                let u: f64 = u_str
                    .parse()
                    .map_err(|_| IoError::Invalid("invalid float in vt.u".into()))?;
                if !u.is_finite() {
                    return Err(IoError::Invalid("vt coordinate must be finite".into()));
                }
                if let Some(v_str) = tokens.next() {
                    let v: f64 = v_str
                        .parse()
                        .map_err(|_| IoError::Invalid("invalid float in vt.v".into()))?;
                    if !v.is_finite() {
                        return Err(IoError::Invalid("vt coordinate must be finite".into()));
                    }
                }
                vt_count += 1;
            }
            "vn" => {
                let x: f64 = tokens
                    .next()
                    .ok_or_else(|| IoError::Invalid("missing x in vn".into()))?
                    .parse()
                    .map_err(|_| IoError::Invalid("invalid float in vn.x".into()))?;
                let y: f64 = tokens
                    .next()
                    .ok_or_else(|| IoError::Invalid("missing y in vn".into()))?
                    .parse()
                    .map_err(|_| IoError::Invalid("invalid float in vn.y".into()))?;
                let z: f64 = tokens
                    .next()
                    .ok_or_else(|| IoError::Invalid("missing z in vn".into()))?
                    .parse()
                    .map_err(|_| IoError::Invalid("invalid float in vn.z".into()))?;
                if !x.is_finite() || !y.is_finite() || !z.is_finite() {
                    return Err(IoError::Invalid("normal component must be finite".into()));
                }
                vn_count += 1;
            }
            "g" | "o" => {
                let rest = line[directive.len()..].trim();
                if rest.is_empty() {
                    current_group = None;
                } else {
                    let name = validate_name(rest)?;
                    current_group = Some(name);
                }
            }
            "usemtl" => {
                let rest = line["usemtl".len()..].trim();
                if rest.is_empty() || rest == "(null)" || rest == "none" {
                    current_material = None;
                } else {
                    let name = validate_name(rest)?;
                    current_material = Some(name);
                }
            }
            "mtllib" => {
                if let Some(base) = base_dir {
                    for mtl_filename in tokens {
                        let mtl_path = safe_resolve_under(base, mtl_filename)?;
                        let mut mtl_bytes = Vec::new();
                        File::open(&mtl_path)?
                            .take(MAX_MTL_BYTES as u64 + 1)
                            .read_to_end(&mut mtl_bytes)?;
                        if mtl_bytes.len() > MAX_MTL_BYTES {
                            return Err(IoError::Invalid(
                                "associated MTL exceeds size limit".into(),
                            ));
                        }
                        let mtl_text = std::str::from_utf8(&mtl_bytes)
                            .map_err(|_| IoError::Invalid("MTL file is not valid UTF-8".into()))?;
                        let parsed = parse_mtl(mtl_text, mtl_path.parent(), &mut textures)?;
                        for (k, v) in parsed {
                            mtl_defs.insert(k, v);
                        }
                    }
                }
                // String variant ignores mtllib without reading files.
            }
            "f" => {
                if total_faces >= MAX_FACES {
                    return Err(IoError::Invalid("too many faces in OBJ file".into()));
                }
                let mut face_v_indices = Vec::new();
                for vertex_token in tokens {
                    let mut parts = vertex_token.split('/');
                    let v_idx_str = parts.next().ok_or_else(|| {
                        IoError::Invalid("empty face vertex specification".into())
                    })?;
                    if v_idx_str.is_empty() {
                        return Err(IoError::Invalid("missing vertex index in face".into()));
                    }
                    let v_idx: i64 = v_idx_str
                        .parse()
                        .map_err(|_| IoError::Invalid("invalid integer in face index".into()))?;
                    if v_idx == 0 {
                        return Err(IoError::Invalid(
                            "OBJ indices are 1-based, zero is invalid".into(),
                        ));
                    }
                    let resolved_v = if v_idx > 0 {
                        let idx = v_idx as usize;
                        if idx > v_positions.len() {
                            return Err(IoError::Invalid("vertex index out of range".into()));
                        }
                        idx - 1
                    } else {
                        let current_len = v_positions.len() as i64;
                        let resolved = current_len + v_idx;
                        if resolved < 0 {
                            return Err(IoError::Invalid(
                                "negative vertex index out of range".into(),
                            ));
                        }
                        resolved as usize
                    };

                    if let Some(vt_idx_str) = parts.next() {
                        if !vt_idx_str.is_empty() {
                            let vt_idx: i64 = vt_idx_str.parse().map_err(|_| {
                                IoError::Invalid("invalid integer in vt index".into())
                            })?;
                            if vt_idx == 0 {
                                return Err(IoError::Invalid("vt index 0 is invalid".into()));
                            }
                            let resolved_vt = if vt_idx > 0 {
                                let idx = vt_idx as usize;
                                if idx > vt_count {
                                    return Err(IoError::Invalid("vt index out of range".into()));
                                }
                                idx - 1
                            } else {
                                let cur = vt_count as i64;
                                let res = cur + vt_idx;
                                if res < 0 {
                                    return Err(IoError::Invalid(
                                        "negative vt index out of range".into(),
                                    ));
                                }
                                res as usize
                            };
                            let _ = resolved_vt;
                        }
                    }

                    if let Some(vn_idx_str) = parts.next() {
                        if !vn_idx_str.is_empty() {
                            let vn_idx: i64 = vn_idx_str.parse().map_err(|_| {
                                IoError::Invalid("invalid integer in vn index".into())
                            })?;
                            if vn_idx == 0 {
                                return Err(IoError::Invalid("vn index 0 is invalid".into()));
                            }
                            let resolved_vn = if vn_idx > 0 {
                                let idx = vn_idx as usize;
                                if idx > vn_count {
                                    return Err(IoError::Invalid("vn index out of range".into()));
                                }
                                idx - 1
                            } else {
                                let cur = vn_count as i64;
                                let res = cur + vn_idx;
                                if res < 0 {
                                    return Err(IoError::Invalid(
                                        "negative vn index out of range".into(),
                                    ));
                                }
                                res as usize
                            };
                            let _ = resolved_vn;
                        }
                    }

                    if parts.next().is_some() {
                        return Err(IoError::Invalid("too many slashes in face token".into()));
                    }

                    face_v_indices.push(resolved_v);
                }

                if face_v_indices.len() < 3 {
                    return Err(IoError::Invalid(
                        "face must have at least 3 vertices".into(),
                    ));
                }

                if !group_faces.contains_key(&current_group) {
                    group_order.push(current_group.clone());
                }
                group_faces
                    .entry(current_group.clone())
                    .or_default()
                    .push(RawFace {
                        vertex_indices: face_v_indices,
                        material: current_material.clone(),
                    });
                total_faces += 1;
            }
            _ => {
                // Ignore other directives (s, l, etc.)
            }
        }
    }

    // Material table preparation
    let mut doc_alloc = DocumentAllocator::new(1);
    let mut material_name_to_id: BTreeMap<String, MaterialId> = BTreeMap::new();
    let mut document_materials: BTreeMap<MaterialId, Material> = BTreeMap::new();

    // Ensure all referenced materials exist in document materials
    for faces in group_faces.values() {
        for f in faces {
            if let Some(mat_name) = &f.material {
                if !material_name_to_id.contains_key(mat_name) {
                    let mat_id = MaterialId(doc_alloc.alloc()?);
                    let mat_def = mtl_defs.get(mat_name).cloned().unwrap_or_else(|| MtlDef {
                        name: mat_name.clone(),
                        color: [0.8, 0.8, 0.8],
                        opacity: 1.0,
                        texture: None,
                    });
                    let material = Material {
                        name: mat_def.name,
                        color: mat_def.color,
                        opacity: mat_def.opacity,
                        texture: mat_def.texture,
                    };
                    material_name_to_id.insert(mat_name.clone(), mat_id);
                    document_materials.insert(mat_id, material);
                }
            }
        }
    }

    // Build geometry per group
    let mut definitions: BTreeMap<DefinitionId, DefinitionData> = BTreeMap::new();
    let mut root_instances: BTreeMap<InstanceId, Instance> = BTreeMap::new();
    let mut root_geometry: Option<GeometryData> = None;

    for group_key in group_order {
        let Some(raw_faces) = group_faces.remove(&group_key) else {
            continue;
        };

        let mut welder = Welder::new(1);
        let mut built_faces: BTreeMap<FaceId, Face> = BTreeMap::new();
        let mut edge_pairs: BTreeSet<[VertexId; 2]> = BTreeSet::new();

        for raw_face in raw_faces {
            // Weld vertices within tolerance and deduplicate consecutive identical vertices
            let mut ring: Vec<VertexId> = Vec::with_capacity(raw_face.vertex_indices.len());
            for &v_idx in &raw_face.vertex_indices {
                let pos = v_positions[v_idx];
                let vid = welder.intern(pos);
                if ring.last() != Some(&vid) {
                    ring.push(vid);
                }
            }
            if ring.len() > 1 && ring.first() == ring.last() {
                ring.pop();
            }

            if ring.len() < 3 {
                return Err(IoError::Invalid(
                    "face ring degenerated to fewer than 3 vertices after welding".into(),
                ));
            }
            if ring.iter().collect::<BTreeSet<_>>().len() != ring.len() {
                return Err(IoError::Invalid(
                    "face ring is not simple (contains repeated vertices)".into(),
                ));
            }

            // Retrieve welded 3D positions
            let pts: Vec<DVec3> = ring.iter().map(|id| welder.position(*id)).collect();

            // Compute polygon normal via Newell's method
            let mut normal = DVec3::ZERO;
            for i in 0..pts.len() {
                let curr = pts[i];
                let next = pts[(i + 1) % pts.len()];
                normal.x += (curr.y - next.y) * (curr.z + next.z);
                normal.y += (curr.z - next.z) * (curr.x + next.x);
                normal.z += (curr.x - next.x) * (curr.y + next.y);
            }
            let normal_len = normal.length();
            if normal_len <= TOLERANCE * TOLERANCE {
                return Err(IoError::Invalid(
                    "degenerate polygon area or collinear vertices in face".into(),
                ));
            }
            let poly_normal = normal / normal_len;

            // Form canonical plane with dominant component positive
            let mut plane_normal = poly_normal;
            let abs_n = plane_normal.abs();
            let dom_axis = if abs_n.x >= abs_n.y && abs_n.x >= abs_n.z {
                0
            } else if abs_n.y >= abs_n.z {
                1
            } else {
                2
            };
            if plane_normal[dom_axis] < 0.0 {
                plane_normal = -plane_normal;
            }
            let plane = Plane {
                normal: plane_normal,
                distance: plane_normal.dot(pts[0]),
            };

            // Enforce strict planarity
            for &p in &pts {
                if plane.signed_distance(p).abs() > TOLERANCE {
                    return Err(IoError::Invalid(
                        "nonplanar face: vertices deviate from plane beyond tolerance".into(),
                    ));
                }
            }

            // Compute canonical 2D basis matching core's Plane::basis()
            let axis = if plane.normal.x.abs() < 0.9 {
                DVec3::X
            } else {
                DVec3::Y
            };
            let u = plane.normal.cross(axis).normalize();
            let v = plane.normal.cross(u);

            // Project loop to 2D
            let origin = pts[0];
            let mut signed_area = 0.0;
            for i in 0..pts.len() {
                let curr = pts[i] - origin;
                let next = pts[(i + 1) % pts.len()] - origin;
                let a = DVec2::new(curr.dot(u), curr.dot(v));
                let b = DVec2::new(next.dot(u), next.dot(v));
                signed_area += a.perp_dot(b);
            }
            signed_area *= 0.5;

            if signed_area.abs() <= TOLERANCE * TOLERANCE {
                return Err(IoError::Invalid("degenerate projected face area".into()));
            }

            // Outer loop must be counterclockwise relative to plane normal
            if signed_area < -TOLERANCE * TOLERANCE {
                ring.reverse();
            }

            // Record boundary edges
            for window in ring.windows(2) {
                let mut pair = [window[0], window[1]];
                pair.sort();
                edge_pairs.insert(pair);
            }
            let mut closing_pair = [ring[ring.len() - 1], ring[0]];
            closing_pair.sort();
            edge_pairs.insert(closing_pair);

            // Determine face color and material reference
            let (mat_id_opt, color) = match &raw_face.material {
                Some(name) => {
                    let id = material_name_to_id.get(name).copied();
                    let color = if let Some(id) = id {
                        let m = &document_materials[&id];
                        let r = (m.color[0].clamp(0.0, 1.0) * 255.0).round() as u8;
                        let g = (m.color[1].clamp(0.0, 1.0) * 255.0).round() as u8;
                        let b = (m.color[2].clamp(0.0, 1.0) * 255.0).round() as u8;
                        let a = (m.opacity.clamp(0.0, 1.0) * 255.0).round() as u8;
                        [r, g, b, a]
                    } else {
                        [160, 177, 193, 255]
                    };
                    (id, color)
                }
                None => (None, [160, 177, 193, 255]),
            };

            let face_id = FaceId::from_raw(welder.next_id);
            welder.next_id += 1;
            built_faces.insert(
                face_id,
                Face {
                    outer: ring,
                    holes: Vec::new(),
                    plane,
                    color,
                    material: mat_id_opt,
                },
            );
        }

        // Build edges with shared boundary edge topology
        let mut built_edges: BTreeMap<EdgeId, Edge> = BTreeMap::new();
        for pair in edge_pairs {
            let edge_id = EdgeId::from_raw(welder.next_id);
            welder.next_id += 1;
            built_edges.insert(
                edge_id,
                Edge {
                    vertices: pair,
                    soft: false,
                    smooth: false,
                },
            );
        }

        let geom_data = GeometryData {
            vertices: welder.vertices,
            edges: built_edges,
            faces: built_faces,
            next_id: welder.next_id,
        };

        match group_key {
            None => {
                root_geometry = Some(geom_data);
            }
            Some(name) => {
                let def_id = DefinitionId(doc_alloc.alloc()?);
                definitions.insert(
                    def_id,
                    DefinitionData {
                        name: name.clone(),
                        kind: DefinitionKind::Group,
                        geometry: geom_data,
                        instances: BTreeMap::new(),
                    },
                );
                let inst_id = InstanceId(doc_alloc.alloc()?);
                root_instances.insert(
                    inst_id,
                    Instance {
                        id: inst_id,
                        name,
                        definition: def_id,
                        transform: DMat4::IDENTITY,
                        tag: None,
                        material: None,
                        hidden: false,
                    },
                );
            }
        }
    }

    let root_geom = root_geometry.unwrap_or_else(|| GeometryData {
        vertices: BTreeMap::new(),
        edges: BTreeMap::new(),
        faces: BTreeMap::new(),
        next_id: 1,
    });

    definitions.insert(
        DefinitionId(0),
        DefinitionData {
            name: "root".into(),
            kind: DefinitionKind::Root,
            geometry: root_geom,
            instances: root_instances,
        },
    );

    let doc_next_id = doc_alloc.alloc()?;
    let document = DocumentData {
        schema_version: cindral_core::SCHEMA_VERSION,
        next_id: doc_next_id,
        root: DefinitionId(0),
        definitions,
        tags: BTreeMap::new(),
        materials: document_materials,
    };

    let bundle = DraftBundle {
        presentation: Default::default(), document, textures };

    // Validate the bundle using DocumentHistory::from_data (via validate_bundle)
    validate_bundle(&bundle)?;

    Ok(bundle)
}

/// Import a Wavefront OBJ file from disk with associated MTL and texture assets.
/// Associated files must be located under the OBJ file's directory.
pub fn import_obj(path: &Path) -> Result<DraftBundle> {
    let mut bytes = Vec::new();
    File::open(path)?
        .take(MAX_OBJ_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_OBJ_BYTES {
        return Err(IoError::Invalid(
            "OBJ file exceeds maximum size limit (32 MiB)".into(),
        ));
    }
    let text = std::str::from_utf8(&bytes)
        .map_err(|_| IoError::Invalid("OBJ file is not valid UTF-8".into()))?;
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    import_obj_internal(text, Some(parent))
}

/// Import a Wavefront OBJ from an in-memory string without reading any external files.
pub fn import_obj_str(text: &str) -> Result<DraftBundle> {
    import_obj_internal(text, None)
}
