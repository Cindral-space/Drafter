//! Bounded COLLADA (`.dae`) XML parser producing [`ImportedScene`].

use crate::{
    import_scene::{
        read_asset, read_bounded, ImportControl, ImportedDefinition, ImportedInstance,
        ImportedScene, UpAxis,
    },
    texture_from_bytes, IoError, Result, TextureAsset,
};
use cindral_core::{
    BulkFace, BulkGeometry, DVec3, Material, MaterialId, MAX_COORDINATE, TOLERANCE,
};
use glam::DMat4;
use roxmltree::{Document, Node, ParsingOptions};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    path::Path,
};

macro_rules! import_warning {
    ($warnings:expr, $value:expr) => {
        if $warnings.len() < 1000 {
            $warnings.push($value);
        }
    };
}

const MAX_ASSET_BYTES: usize = 64 * 1024 * 1024;
const MAX_VERTICES: usize = 1_000_000;
const MAX_TRIANGLES: usize = 1_000_000;
const MAX_NODES: usize = 100_000;
const MAX_DEPTH: usize = 64;
const MAX_TEXTURES: usize = 127;
const MAX_XML_NODES: u32 = 300_000;

/// Parse a `.dae` file from disk within bounded size and XML limits.
pub fn import_dae(path: &Path, control: &ImportControl) -> Result<ImportedScene> {
    control.check()?;
    let bytes = read_bounded(path, MAX_ASSET_BYTES)?;
    let base_dir = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .or(Some(Path::new(".")));
    import_dae_bytes(&bytes, base_dir, control)
}

/// Parse `.dae` XML bytes into an [`ImportedScene`].
pub fn import_dae_bytes(
    bytes: &[u8],
    base_dir: Option<&Path>,
    control: &ImportControl,
) -> Result<ImportedScene> {
    control.check()?;
    if bytes.is_empty() {
        return Err(IoError::Invalid("empty COLLADA input".into()));
    }
    if bytes.len() > MAX_ASSET_BYTES {
        return Err(IoError::Invalid("COLLADA file exceeds 64 MiB limit".into()));
    }

    let stripped = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    let xml_str = std::str::from_utf8(stripped)
        .map_err(|e| IoError::Invalid(format!("COLLADA file is not valid UTF-8: {e}")))?;

    reject_dtd_and_entities(xml_str)?;
    preflight_xml(xml_str)?;

    let doc = Document::parse_with_options(
        xml_str,
        ParsingOptions {
            allow_dtd: false,
            nodes_limit: MAX_XML_NODES,
        },
    )
    .map_err(|e| IoError::Invalid(format!("invalid COLLADA XML: {e}")))?;

    let root = doc.root_element();
    if root.tag_name().name() != "COLLADA" {
        return Err(IoError::Invalid(
            "root XML element must be <COLLADA>".into(),
        ));
    }

    validate_xml_bounds(root, control)?;

    let mut total_bytes = bytes.len();
    let mut warnings = Vec::new();
    let (meters_per_unit, up_axis) = parse_asset_metadata(root)?;

    for child in root.children().filter(Node::is_element) {
        let name = child.tag_name().name();
        if matches!(
            name,
            "library_animations"
                | "library_controllers"
                | "library_cameras"
                | "library_lights"
                | "library_physics_scenes"
                | "library_force_fields"
        ) {
            import_warning!(
                warnings,
                format!("unsupported COLLADA section ignored: {name}")
            );
        }
    }

    control.check()?;
    control.set_progress(0.1);

    let images = parse_library_images(root);
    let effects = parse_library_effects(root, &images);
    let mut textures: BTreeMap<String, TextureAsset> = BTreeMap::new();
    let (materials, mat_id_map, mat_name_map) = parse_library_materials(
        root,
        &effects,
        &images,
        base_dir,
        &mut total_bytes,
        &mut textures,
        &mut warnings,
        control,
    )?;

    control.check()?;
    control.set_progress(0.25);

    let geometries = parse_library_geometries(root, &mut warnings, control)?;
    let lib_nodes = collect_library_nodes(root);

    control.check()?;
    control.set_progress(0.4);

    let mut scene = ImportedScene {
        notes: Vec::new(),
        not_imported: Vec::new(),
        root_definition: None,
        definitions: Vec::new(),
        roots: Vec::new(),
        materials,
        textures,
        tags: Vec::new(),
        meters_per_unit,
        up_axis,
        warnings,
        skipped: 0,
    };

    build_visual_scene(
        root,
        &geometries,
        &lib_nodes,
        &mat_id_map,
        &mat_name_map,
        &mut scene,
        control,
    )?;

    Ok(scene)
}

fn reject_dtd_and_entities(xml: &str) -> Result<()> {
    let bytes = xml.as_bytes();
    for window in bytes.windows(9) {
        if window.eq_ignore_ascii_case(b"<!doctype")
            || window[..8].eq_ignore_ascii_case(b"<!entity")
        {
            return Err(IoError::Invalid(
                "COLLADA DTD and ENTITY declarations are forbidden".into(),
            ));
        }
    }
    Ok(())
}

// Bound nesting before roxmltree's recursive parser sees untrusted XML.
// This scanner recognizes quoted attributes, comments, CDATA and processing
// instructions; the XML parser remains responsible for syntax and name validation.
fn preflight_xml(xml: &str) -> Result<()> {
    let bytes = xml.as_bytes();
    let mut at = 0;
    let mut depth = 0usize;
    let mut count = 0usize;
    while at < bytes.len() {
        let Some(relative) = bytes[at..].iter().position(|b| *b == b'<') else {
            break;
        };
        at += relative;
        let tail = &bytes[at..];
        let terminator: Option<&[u8]> = if tail.starts_with(b"<!--") {
            Some(b"-->")
        } else if tail.starts_with(b"<![CDATA[") {
            Some(b"]]>")
        } else if tail.starts_with(b"<?") {
            Some(b"?>")
        } else {
            None
        };
        if let Some(end) = terminator {
            let relative = tail
                .windows(end.len())
                .position(|w| w == end)
                .ok_or_else(|| IoError::Invalid("unterminated XML markup".into()))?;
            at += relative + end.len();
            continue;
        }
        if tail.starts_with(b"<!") {
            return Err(IoError::Invalid("XML declarations are disabled".into()));
        }
        let closing = tail.starts_with(b"</");
        let mut end = at + 1;
        let mut quote = None;
        while end < bytes.len() {
            let b = bytes[end];
            if let Some(q) = quote {
                if b == q {
                    quote = None;
                }
            } else if b == b'\'' || b == b'"' {
                quote = Some(b);
            } else if b == b'>' {
                break;
            }
            end += 1;
        }
        if end == bytes.len() {
            return Err(IoError::Invalid("unterminated XML tag".into()));
        }
        if closing {
            depth = depth.saturating_sub(1);
        } else {
            count += 1;
            if count > MAX_NODES {
                return Err(IoError::Invalid("XML element count exceeds limit".into()));
            }
            let self_closing = bytes[at + 1..end]
                .iter()
                .rev()
                .find(|b| !b.is_ascii_whitespace())
                == Some(&b'/');
            if depth + 1 > MAX_DEPTH {
                return Err(IoError::Invalid("XML depth exceeds 64".into()));
            }
            if !self_closing {
                depth += 1;
            }
        }
        at = end + 1;
    }
    Ok(())
}

fn validate_xml_bounds(root: Node<'_, '_>, control: &ImportControl) -> Result<()> {
    let mut element_count = 0usize;
    let mut stack = vec![(root, 1usize)];
    while let Some((node, depth)) = stack.pop() {
        if depth > MAX_DEPTH {
            return Err(IoError::Invalid("COLLADA XML depth exceeds 64".into()));
        }
        element_count += 1;
        if element_count > MAX_NODES {
            return Err(IoError::Invalid(
                "COLLADA XML element count exceeds 100k limit".into(),
            ));
        }
        if element_count.is_multiple_of(2048) {
            control.check()?;
        }
        for child in node.children().filter(Node::is_element) {
            stack.push((child, depth + 1));
        }
    }
    Ok(())
}

fn parse_asset_metadata(root: Node<'_, '_>) -> Result<(f64, UpAxis)> {
    let mut meters = 1.0f64;
    let mut up_axis = UpAxis::Y;

    if let Some(asset) = root
        .children()
        .find(|n| n.is_element() && n.tag_name().name() == "asset")
    {
        if let Some(unit) = asset
            .children()
            .find(|n| n.is_element() && n.tag_name().name() == "unit")
        {
            if let Some(m_str) = unit.attribute("meter") {
                let m: f64 = m_str
                    .trim()
                    .parse()
                    .map_err(|_| IoError::Invalid("invalid COLLADA unit meter".into()))?;
                if !m.is_finite() || m <= 0.0 || m > MAX_COORDINATE {
                    return Err(IoError::Invalid("COLLADA unit meter out of bounds".into()));
                }
                meters = m;
            }
        }
        if let Some(up) = asset
            .children()
            .find(|n| n.is_element() && n.tag_name().name() == "up_axis")
        {
            let text = up.text().unwrap_or_default().trim();
            up_axis = match text {
                "X_UP" => UpAxis::X,
                "Y_UP" | "" => UpAxis::Y,
                "Z_UP" => UpAxis::Z,
                other => {
                    return Err(IoError::Invalid(format!(
                        "unsupported COLLADA up_axis: {other}"
                    )))
                }
            };
        }
    }

    Ok((meters, up_axis))
}

fn strip_hash(url: &str) -> &str {
    url.trim().strip_prefix('#').unwrap_or_else(|| url.trim())
}

fn parse_library_images(root: Node<'_, '_>) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for lib in root
        .children()
        .filter(|n| n.is_element() && n.tag_name().name() == "library_images")
    {
        for img in lib
            .children()
            .filter(|n| n.is_element() && n.tag_name().name() == "image")
        {
            let Some(id) = img.attribute("id") else {
                continue;
            };
            if let Some(init) = img
                .children()
                .find(|n| n.is_element() && n.tag_name().name() == "init_from")
            {
                let uri = if let Some(ref_node) = init
                    .children()
                    .find(|n| n.is_element() && n.tag_name().name() == "ref")
                {
                    ref_node.text().unwrap_or_default().trim()
                } else {
                    init.text().unwrap_or_default().trim()
                };
                if !uri.is_empty() {
                    map.insert(id.to_string(), uri.to_string());
                }
            }
        }
    }
    map
}

#[derive(Clone, Debug)]
struct EffectDesc {
    color: [f32; 3],
    opacity: f32,
    image_id: Option<String>,
}

impl Default for EffectDesc {
    fn default() -> Self {
        Self {
            color: [0.8, 0.8, 0.8],
            opacity: 1.0,
            image_id: None,
        }
    }
}

fn parse_library_effects(
    root: Node<'_, '_>,
    images: &HashMap<String, String>,
) -> HashMap<String, EffectDesc> {
    let mut effects = HashMap::new();
    for lib in root
        .children()
        .filter(|n| n.is_element() && n.tag_name().name() == "library_effects")
    {
        for effect in lib
            .children()
            .filter(|n| n.is_element() && n.tag_name().name() == "effect")
        {
            let Some(id) = effect.attribute("id") else {
                continue;
            };
            let mut desc = EffectDesc::default();
            let mut surfaces: HashMap<String, String> = HashMap::new();
            let mut samplers: HashMap<String, String> = HashMap::new();

            for node in effect.descendants().filter(Node::is_element) {
                if node.tag_name().name() == "newparam" {
                    let Some(sid) = node.attribute("sid") else {
                        continue;
                    };
                    for child in node.children().filter(Node::is_element) {
                        if child.tag_name().name() == "surface" {
                            if let Some(init) = child
                                .children()
                                .find(|n| n.is_element() && n.tag_name().name() == "init_from")
                            {
                                let img_ref = init.text().unwrap_or_default().trim();
                                if !img_ref.is_empty() {
                                    surfaces
                                        .insert(sid.to_string(), strip_hash(img_ref).to_string());
                                }
                            }
                        } else if child.tag_name().name() == "sampler2D" {
                            if let Some(src) = child
                                .children()
                                .find(|n| n.is_element() && n.tag_name().name() == "source")
                            {
                                let surf_ref = src.text().unwrap_or_default().trim();
                                if !surf_ref.is_empty() {
                                    samplers
                                        .insert(sid.to_string(), strip_hash(surf_ref).to_string());
                                }
                            }
                        }
                    }
                }
            }

            let mut diffuse_alpha = 1.0f32;
            let mut transparency: Option<f32> = None;
            let mut opaque_mode = "A_ONE";

            for node in effect.descendants().filter(Node::is_element) {
                match node.tag_name().name() {
                    "diffuse" => {
                        for child in node.children().filter(Node::is_element) {
                            if child.tag_name().name() == "color" {
                                let vals: Vec<f32> = child
                                    .text()
                                    .unwrap_or_default()
                                    .split_whitespace()
                                    .filter_map(|s| s.parse::<f32>().ok())
                                    .collect();
                                if vals.len() >= 3 && vals.iter().copied().all(f32::is_finite) {
                                    desc.color = [
                                        vals[0].clamp(0.0, 1.0),
                                        vals[1].clamp(0.0, 1.0),
                                        vals[2].clamp(0.0, 1.0),
                                    ];
                                    if vals.len() >= 4 {
                                        diffuse_alpha = vals[3].clamp(0.0, 1.0);
                                    }
                                }
                            } else if child.tag_name().name() == "texture" {
                                if let Some(tex_attr) = child.attribute("texture") {
                                    let key = strip_hash(tex_attr);
                                    let resolved = samplers
                                        .get(key)
                                        .and_then(|surf| surfaces.get(surf))
                                        .or_else(|| surfaces.get(key))
                                        .cloned()
                                        .or_else(|| {
                                            images.contains_key(key).then(|| key.to_string())
                                        });
                                    desc.image_id = resolved;
                                }
                            }
                        }
                    }
                    "transparent" => {
                        if let Some(op) = node.attribute("opaque") {
                            opaque_mode = op;
                        }
                    }
                    "transparency" => {
                        if let Some(fl) = node
                            .children()
                            .find(|n| n.is_element() && n.tag_name().name() == "float")
                        {
                            if let Ok(v) = fl.text().unwrap_or_default().trim().parse::<f32>() {
                                if v.is_finite() {
                                    transparency = Some(v.clamp(0.0, 1.0));
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }

            let trans_alpha = match transparency {
                Some(t) if opaque_mode == "RGB_ZERO" => 1.0 - t,
                Some(t) => t,
                None => 1.0,
            };
            desc.opacity = (diffuse_alpha * trans_alpha).clamp(0.0, 1.0);
            effects.insert(id.to_string(), desc);
        }
    }
    effects
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn parse_library_materials(
    root: Node<'_, '_>,
    effects: &HashMap<String, EffectDesc>,
    images: &HashMap<String, String>,
    base_dir: Option<&Path>,
    total_bytes: &mut usize,
    textures_out: &mut BTreeMap<String, TextureAsset>,
    warnings: &mut Vec<String>,
    control: &ImportControl,
) -> Result<(
    Vec<Material>,
    HashMap<String, usize>,
    HashMap<String, usize>,
)> {
    let mut materials = Vec::new();
    let mut id_map = HashMap::new();
    let mut name_map = HashMap::new();
    let mut loaded_images: HashMap<String, String> = HashMap::new();

    for lib in root
        .children()
        .filter(|n| n.is_element() && n.tag_name().name() == "library_materials")
    {
        for mat_node in lib
            .children()
            .filter(|n| n.is_element() && n.tag_name().name() == "material")
        {
            control.check()?;
            let mat_id = mat_node.attribute("id").unwrap_or_default().trim();
            let raw_name = mat_node
                .attribute("name")
                .or(Some(mat_id))
                .unwrap_or_default()
                .trim();
            let name: String = if raw_name.is_empty() {
                format!("Material_{}", materials.len())
            } else {
                raw_name
                    .chars()
                    .filter(|c| !c.is_control())
                    .take(512)
                    .collect()
            };

            let effect_desc = mat_node
                .children()
                .find(|n| n.is_element() && n.tag_name().name() == "instance_effect")
                .and_then(|ie| ie.attribute("url"))
                .and_then(|url| effects.get(strip_hash(url)))
                .cloned()
                .unwrap_or_default();

            let mut texture_slot = None;
            if let Some(img_id) = &effect_desc.image_id {
                if let Some(existing) = loaded_images.get(img_id) {
                    texture_slot = Some(existing.clone());
                } else if let Some(uri) = images.get(img_id) {
                    let trimmed = uri.trim();
                    let lower = trimmed.to_ascii_lowercase();
                    if lower.starts_with("http://")
                        || lower.starts_with("https://")
                        || lower.starts_with("ftp://")
                        || lower.starts_with("file:")
                        || trimmed.contains(':')
                    {
                        return Err(IoError::Invalid(
                            "network or scheme image URIs are forbidden in COLLADA".into(),
                        ));
                    }
                    let base = base_dir.ok_or_else(|| {
                        IoError::Invalid("COLLADA texture URI requires a base directory".into())
                    })?;
                    let remaining = MAX_ASSET_BYTES.checked_sub(*total_bytes).ok_or_else(|| {
                        IoError::Invalid("combined COLLADA assets exceed 64 MiB".into())
                    })?;
                    let rel = trimmed.strip_prefix("./").unwrap_or(trimmed);
                    let bytes = read_asset(base, rel, remaining)?;
                    *total_bytes = total_bytes
                        .checked_add(bytes.len())
                        .ok_or_else(|| IoError::Invalid("asset size overflow".into()))?;
                    let (key, asset) = texture_from_bytes(bytes)?;
                    if !textures_out.contains_key(&key) && textures_out.len() >= MAX_TEXTURES {
                        return Err(IoError::Invalid("exceeded 127 texture limit".into()));
                    }
                    textures_out.insert(key.clone(), asset);
                    loaded_images.insert(img_id.clone(), key.clone());
                    texture_slot = Some(key);
                } else {
                    import_warning!(warnings, format!("missing COLLADA image {img_id}"));
                }
            }

            let idx = materials.len();
            materials.push(Material {
                name: name.clone(),
                // COLLADA diffuse channels are interchange sRGB values.
                color: crate::srgb_bytes(effect_desc.color),
                opacity: effect_desc.opacity,
                texture: texture_slot,
                texture_mapping: None,
            });
            if !mat_id.is_empty() {
                id_map.insert(mat_id.to_string(), idx);
            }
            name_map.insert(name, idx);
        }
    }

    Ok((materials, id_map, name_map))
}

#[derive(Clone, Debug)]
struct RawPolygon {
    indices: Vec<usize>,
    material_symbol: Option<String>,
}

#[derive(Clone, Debug)]
struct ParsedGeometry {
    name: String,
    positions: Vec<DVec3>,
    polygons: Vec<RawPolygon>,
}

fn parse_library_geometries(
    root: Node<'_, '_>,
    warnings: &mut Vec<String>,
    control: &ImportControl,
) -> Result<HashMap<String, ParsedGeometry>> {
    let mut geometries = HashMap::new();
    let mut total_vertices = 0usize;
    let mut total_faces = 0usize;

    for lib in root
        .children()
        .filter(|n| n.is_element() && n.tag_name().name() == "library_geometries")
    {
        for geom_node in lib
            .children()
            .filter(|n| n.is_element() && n.tag_name().name() == "geometry")
        {
            control.check()?;
            let geom_id = geom_node
                .attribute("id")
                .ok_or_else(|| IoError::Invalid("COLLADA <geometry> missing id".into()))?
                .trim();
            let raw_name = geom_node.attribute("name").unwrap_or(geom_id).trim();
            let name: String = if raw_name.is_empty() {
                "Geometry".into()
            } else {
                raw_name
                    .chars()
                    .filter(|c| !c.is_control())
                    .take(512)
                    .collect()
            };

            let Some(mesh_node) = geom_node
                .children()
                .find(|n| n.is_element() && n.tag_name().name() == "mesh")
            else {
                import_warning!(warnings, format!("skipped non-mesh geometry {geom_id}"));
                continue;
            };

            let sources = parse_mesh_sources(mesh_node)?;
            let vertices_map = parse_mesh_vertices(mesh_node, &sources)?;

            let mut canonical_source_id: Option<String> = None;
            let mut polygons = Vec::new();

            for prim in mesh_node.children().filter(Node::is_element) {
                let tag = prim.tag_name().name();
                match tag {
                    "triangles" | "polylist" | "polygons" => {
                        let (src_id, prim_polys) =
                            parse_primitive_group(prim, tag, &sources, &vertices_map)?;
                        if let Some(existing) = &canonical_source_id {
                            if existing != &src_id {
                                return Err(IoError::Invalid(
                                    "mesh primitives must share the same position source".into(),
                                ));
                            }
                        } else {
                            canonical_source_id = Some(src_id);
                        }
                        total_faces = total_faces
                            .checked_add(prim_polys.len())
                            .ok_or_else(|| IoError::Invalid("face count overflow".into()))?;
                        if total_faces > MAX_TRIANGLES {
                            return Err(IoError::Invalid(
                                "COLLADA face count exceeds 1M limit".into(),
                            ));
                        }
                        polygons.extend(prim_polys);
                    }
                    "source" | "vertices" | "extra" => {}
                    other => {
                        import_warning!(
                            warnings,
                            format!("unsupported COLLADA mesh primitive <{other}> ignored")
                        );
                    }
                }
            }

            let positions = if let Some(src_id) = canonical_source_id {
                sources.get(&src_id).cloned().unwrap_or_default()
            } else {
                Vec::new()
            };

            total_vertices = total_vertices
                .checked_add(positions.len())
                .ok_or_else(|| IoError::Invalid("vertex count overflow".into()))?;
            if total_vertices > MAX_VERTICES {
                return Err(IoError::Invalid(
                    "COLLADA vertex count exceeds 1M limit".into(),
                ));
            }

            geometries.insert(
                geom_id.to_string(),
                ParsedGeometry {
                    name,
                    positions,
                    polygons,
                },
            );
        }
    }

    Ok(geometries)
}

fn parse_mesh_sources(mesh_node: Node<'_, '_>) -> Result<HashMap<String, Vec<DVec3>>> {
    let mut sources = HashMap::new();
    for src in mesh_node
        .children()
        .filter(|n| n.is_element() && n.tag_name().name() == "source")
    {
        let Some(src_id) = src.attribute("id") else {
            continue;
        };
        let Some(fa) = src
            .children()
            .find(|n| n.is_element() && n.tag_name().name() == "float_array")
        else {
            continue;
        };
        let declared_count = fa
            .attribute("count")
            .and_then(|s| s.trim().parse::<usize>().ok())
            .unwrap_or(0);
        if declared_count > MAX_VERTICES * 16 {
            return Err(IoError::Invalid(
                "COLLADA float_array count exceeds limit".into(),
            ));
        }

        let mut floats = Vec::with_capacity(declared_count.min(65536));
        for tok in fa.text().unwrap_or_default().split_whitespace() {
            let v: f64 = tok
                .parse()
                .map_err(|_| IoError::Invalid("invalid float in COLLADA float_array".into()))?;
            if !v.is_finite() {
                return Err(IoError::Invalid(
                    "non-finite float in COLLADA float_array".into(),
                ));
            }
            floats.push(v);
            if floats.len() > MAX_VERTICES * 16 {
                return Err(IoError::Invalid("COLLADA float_array exceeds limit".into()));
            }
        }

        let accessor = src
            .children()
            .find(|n| n.is_element() && n.tag_name().name() == "technique_common")
            .and_then(|tc| {
                tc.children()
                    .find(|n| n.is_element() && n.tag_name().name() == "accessor")
            });

        let (offset, stride, count, x_idx, y_idx, z_idx) = if let Some(acc) = accessor {
            let offset = acc
                .attribute("offset")
                .map(|s| {
                    s.trim()
                        .parse::<usize>()
                        .map_err(|_| IoError::Invalid("invalid accessor offset".into()))
                })
                .transpose()?
                .unwrap_or(0);
            let params: Vec<Node<'_, '_>> = acc
                .children()
                .filter(|n| n.is_element() && n.tag_name().name() == "param")
                .collect();
            let stride = acc
                .attribute("stride")
                .map(|s| {
                    s.trim()
                        .parse::<usize>()
                        .map_err(|_| IoError::Invalid("invalid accessor stride".into()))
                })
                .transpose()?
                .unwrap_or_else(|| params.len().max(3));
            if stride < 3 {
                // Non-3D source (e.g. 2D UV source); skip storing as 3D position source.
                continue;
            }
            let count = acc
                .attribute("count")
                .map(|s| {
                    s.trim()
                        .parse::<usize>()
                        .map_err(|_| IoError::Invalid("invalid accessor count".into()))
                })
                .transpose()?
                .unwrap_or(floats.len().saturating_sub(offset) / stride);

            let mut xi = 0usize;
            let mut yi = 1usize;
            let mut zi = 2usize;
            for (idx, p) in params.iter().enumerate() {
                match p.attribute("name").unwrap_or_default() {
                    "X" => xi = idx,
                    "Y" => yi = idx,
                    "Z" => zi = idx,
                    _ => {}
                }
            }
            if xi >= stride || yi >= stride || zi >= stride {
                return Err(IoError::Invalid(
                    "accessor param index exceeds stride".into(),
                ));
            }
            (offset, stride, count, xi, yi, zi)
        } else {
            if floats.len() < 3 {
                continue;
            }
            (0, 3, floats.len() / 3, 0, 1, 2)
        };

        if count > MAX_VERTICES {
            return Err(IoError::Invalid(
                "COLLADA accessor count exceeds 1M vertices".into(),
            ));
        }
        if count > 0 {
            let needed = offset
                .checked_add(
                    (count - 1)
                        .checked_mul(stride)
                        .and_then(|s| s.checked_add(stride))
                        .ok_or_else(|| IoError::Invalid("accessor span overflow".into()))?,
                )
                .ok_or_else(|| IoError::Invalid("accessor offset overflow".into()))?;
            if needed > floats.len() {
                return Err(IoError::Invalid(
                    "COLLADA accessor exceeds float_array length".into(),
                ));
            }
        }

        let mut pts = Vec::with_capacity(count);
        for i in 0..count {
            let base = offset + i * stride;
            let (x, y, z) = (
                floats[base + x_idx],
                floats[base + y_idx],
                floats[base + z_idx],
            );
            if x.abs() > MAX_COORDINATE || y.abs() > MAX_COORDINATE || z.abs() > MAX_COORDINATE {
                return Err(IoError::Invalid(
                    "COLLADA vertex exceeds coordinate bounds".into(),
                ));
            }
            pts.push(DVec3::new(x, y, z));
        }
        sources.insert(src_id.to_string(), pts);
    }
    Ok(sources)
}

fn parse_mesh_vertices(
    mesh_node: Node<'_, '_>,
    sources: &HashMap<String, Vec<DVec3>>,
) -> Result<HashMap<String, String>> {
    let mut map = HashMap::new();
    for v_node in mesh_node
        .children()
        .filter(|n| n.is_element() && n.tag_name().name() == "vertices")
    {
        let vid = v_node
            .attribute("id")
            .ok_or_else(|| IoError::Invalid("<vertices> missing id".into()))?;
        let pos_input = v_node
            .children()
            .find(|n| {
                n.is_element()
                    && n.tag_name().name() == "input"
                    && n.attribute("semantic") == Some("POSITION")
            })
            .ok_or_else(|| IoError::Invalid("<vertices> missing POSITION input".into()))?;
        let src = pos_input
            .attribute("source")
            .ok_or_else(|| IoError::Invalid("<input> missing source".into()))?;
        let src_id = strip_hash(src);
        if !sources.contains_key(src_id) {
            return Err(IoError::Invalid(format!(
                "missing POSITION source #{src_id}"
            )));
        }
        map.insert(vid.to_string(), src_id.to_string());
    }
    Ok(map)
}

fn parse_primitive_group(
    prim: Node<'_, '_>,
    tag: &str,
    sources: &HashMap<String, Vec<DVec3>>,
    vertices_map: &HashMap<String, String>,
) -> Result<(String, Vec<RawPolygon>)> {
    let mat_symbol = prim
        .attribute("material")
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned);

    let mut vertex_offset: Option<usize> = None;
    let mut pos_source_id: Option<String> = None;
    let mut max_offset = 0usize;

    for input in prim
        .children()
        .filter(|n| n.is_element() && n.tag_name().name() == "input")
    {
        let off = input
            .attribute("offset")
            .map(|s| {
                s.trim()
                    .parse::<usize>()
                    .map_err(|_| IoError::Invalid("invalid input offset".into()))
            })
            .transpose()?
            .unwrap_or(0);
        if off > 64 {
            return Err(IoError::Invalid("input offset too large".into()));
        }
        max_offset = max_offset.max(off);
        let semantic = input.attribute("semantic").unwrap_or_default();
        if semantic == "VERTEX" || (semantic == "POSITION" && vertex_offset.is_none()) {
            let src = input
                .attribute("source")
                .ok_or_else(|| IoError::Invalid("primitive input missing source".into()))?;
            let target = strip_hash(src);
            let resolved_src = if semantic == "VERTEX" {
                vertices_map
                    .get(target)
                    .ok_or_else(|| IoError::Invalid(format!("unknown vertices id #{target}")))?
                    .clone()
            } else {
                if !sources.contains_key(target) {
                    return Err(IoError::Invalid(format!("unknown source id #{target}")));
                }
                target.to_string()
            };
            vertex_offset = Some(off);
            pos_source_id = Some(resolved_src);
        }
    }

    let v_offset = vertex_offset
        .ok_or_else(|| IoError::Invalid("primitive missing VERTEX/POSITION input".into()))?;
    let src_id = pos_source_id.ok_or_else(|| IoError::Invalid("missing position source".into()))?;
    let num_positions = sources.get(&src_id).map_or(0, Vec::len);
    let input_stride = max_offset + 1;

    let declared_count = prim
        .attribute("count")
        .map(|s| {
            s.trim()
                .parse::<usize>()
                .map_err(|_| IoError::Invalid("invalid primitive count".into()))
        })
        .transpose()?
        .unwrap_or(0);
    if declared_count > MAX_TRIANGLES {
        return Err(IoError::Invalid("primitive count exceeds 1M limit".into()));
    }

    let mut out = Vec::new();

    match tag {
        "triangles" => {
            let Some(p_node) = prim
                .children()
                .find(|n| n.is_element() && n.tag_name().name() == "p")
            else {
                return if declared_count == 0 {
                    Ok((src_id, out))
                } else {
                    Err(IoError::Invalid("<triangles> missing <p>".into()))
                };
            };
            let p_vals = parse_index_list(p_node.text().unwrap_or_default())?;
            let tri_step = 3usize
                .checked_mul(input_stride)
                .ok_or_else(|| IoError::Invalid("stride overflow".into()))?;
            let expected = declared_count
                .checked_mul(tri_step)
                .ok_or_else(|| IoError::Invalid("triangle index count overflow".into()))?;
            if p_vals.len() < expected
                || (!p_vals.len().is_multiple_of(tri_step) && declared_count == 0)
            {
                return Err(IoError::Invalid("truncated <triangles> index array".into()));
            }
            let tri_count = if declared_count > 0 {
                declared_count
            } else {
                p_vals.len() / tri_step
            };
            if tri_count > MAX_TRIANGLES {
                return Err(IoError::Invalid("triangle count exceeds limit".into()));
            }
            for t in 0..tri_count {
                let base = t * tri_step;
                let i0 = p_vals[base + v_offset];
                let i1 = p_vals[base + input_stride + v_offset];
                let i2 = p_vals[base + 2 * input_stride + v_offset];
                if i0 >= num_positions || i1 >= num_positions || i2 >= num_positions {
                    return Err(IoError::Invalid(
                        "COLLADA triangle vertex index out of bounds".into(),
                    ));
                }
                out.push(RawPolygon {
                    indices: vec![i0, i1, i2],
                    material_symbol: mat_symbol.clone(),
                });
            }
        }
        "polylist" => {
            let Some(vcount_node) = prim
                .children()
                .find(|n| n.is_element() && n.tag_name().name() == "vcount")
            else {
                return if declared_count == 0 {
                    Ok((src_id, out))
                } else {
                    Err(IoError::Invalid("<polylist> missing <vcount>".into()))
                };
            };
            let Some(p_node) = prim
                .children()
                .find(|n| n.is_element() && n.tag_name().name() == "p")
            else {
                return if declared_count == 0 {
                    Ok((src_id, out))
                } else {
                    Err(IoError::Invalid("<polylist> missing <p>".into()))
                };
            };
            let vcounts = parse_index_list(vcount_node.text().unwrap_or_default())?;
            if declared_count > 0 && vcounts.len() < declared_count {
                return Err(IoError::Invalid(
                    "<polylist> vcount shorter than count".into(),
                ));
            }
            let p_vals = parse_index_list(p_node.text().unwrap_or_default())?;
            let poly_count = if declared_count > 0 {
                declared_count
            } else {
                vcounts.len()
            };

            if poly_count > MAX_TRIANGLES {
                return Err(IoError::Invalid("polygon count exceeds limit".into()));
            }
            let mut cursor = 0usize;
            for &vc in &vcounts[..poly_count] {
                if !(3..=4096).contains(&vc) {
                    return Err(IoError::Invalid(
                        "COLLADA polylist polygon vertex count must be in 3..=4096".into(),
                    ));
                }
                let step = vc
                    .checked_mul(input_stride)
                    .ok_or_else(|| IoError::Invalid("polylist stride overflow".into()))?;
                let next_cursor = cursor
                    .checked_add(step)
                    .ok_or_else(|| IoError::Invalid("polylist cursor overflow".into()))?;
                if next_cursor > p_vals.len() {
                    return Err(IoError::Invalid("truncated <polylist> <p> array".into()));
                }
                let mut ring = Vec::with_capacity(vc);
                for k in 0..vc {
                    let v_idx = p_vals[cursor + k * input_stride + v_offset];
                    if v_idx >= num_positions {
                        return Err(IoError::Invalid(
                            "COLLADA polylist vertex index out of bounds".into(),
                        ));
                    }
                    ring.push(v_idx);
                }
                out.push(RawPolygon {
                    indices: ring,
                    material_symbol: mat_symbol.clone(),
                });
                cursor = next_cursor;
            }
        }
        "polygons" => {
            for p_node in prim
                .children()
                .filter(|n| n.is_element() && n.tag_name().name() == "p")
            {
                let p_vals = parse_index_list(p_node.text().unwrap_or_default())?;
                if !p_vals.len().is_multiple_of(input_stride) {
                    return Err(IoError::Invalid(
                        "<polygons> <p> length not divisible by input stride".into(),
                    ));
                }
                let vc = p_vals.len() / input_stride;
                if !(3..=4096).contains(&vc) {
                    return Err(IoError::Invalid(
                        "COLLADA polygon vertex count must be in 3..=4096".into(),
                    ));
                }
                let mut ring = Vec::with_capacity(vc);
                for k in 0..vc {
                    let v_idx = p_vals[k * input_stride + v_offset];
                    if v_idx >= num_positions {
                        return Err(IoError::Invalid(
                            "COLLADA polygon vertex index out of bounds".into(),
                        ));
                    }
                    ring.push(v_idx);
                }
                out.push(RawPolygon {
                    indices: ring,
                    material_symbol: mat_symbol.clone(),
                });
            }
        }
        _ => {}
    }

    Ok((src_id, out))
}

fn parse_index_list(text: &str) -> Result<Vec<usize>> {
    let mut out = Vec::new();
    for tok in text.split_whitespace() {
        let v: usize = tok
            .parse()
            .map_err(|_| IoError::Invalid("invalid integer index in COLLADA".into()))?;
        out.push(v);
        if out.len() > MAX_TRIANGLES * 12 {
            return Err(IoError::Invalid("COLLADA index array exceeds limit".into()));
        }
    }
    Ok(out)
}

fn collect_library_nodes<'a, 'input>(root: Node<'a, 'input>) -> HashMap<String, Node<'a, 'input>> {
    let mut map = HashMap::new();
    for node in root
        .descendants()
        .filter(|n| n.is_element() && n.tag_name().name() == "node" && n.attribute("id").is_some())
    {
        if let Some(id) = node.attribute("id") {
            map.insert(id.trim().to_string(), node);
        }
    }
    map
}

fn parse_dae_node_transform(node: Node<'_, '_>, warnings: &mut Vec<String>) -> Result<DMat4> {
    let mut mat = DMat4::IDENTITY;
    for child in node.children().filter(Node::is_element) {
        match child.tag_name().name() {
            "matrix" => {
                let vals = parse_float_n::<16>(child.text().unwrap_or_default())?;
                let m = DMat4::from_cols_array(&vals).transpose();
                mat *= m;
            }
            "translate" => {
                let [x, y, z] = parse_float_n::<3>(child.text().unwrap_or_default())?;
                mat *= DMat4::from_translation(DVec3::new(x, y, z));
            }
            "rotate" => {
                let [ax, ay, az, deg] = parse_float_n::<4>(child.text().unwrap_or_default())?;
                let axis = DVec3::new(ax, ay, az)
                    .try_normalize()
                    .ok_or_else(|| IoError::Invalid("zero rotation axis in COLLADA".into()))?;
                mat *= DMat4::from_axis_angle(axis, deg.to_radians());
            }
            "scale" => {
                let [sx, sy, sz] = parse_float_n::<3>(child.text().unwrap_or_default())?;
                mat *= DMat4::from_scale(DVec3::new(sx, sy, sz));
            }
            "lookat" | "skew" => {
                import_warning!(
                    warnings,
                    format!(
                        "unsupported COLLADA transform <{}> ignored",
                        child.tag_name().name()
                    )
                );
            }
            _ => {}
        }
    }
    cindral_core::validate_instance_transform(mat)?;
    Ok(mat)
}

fn parse_float_n<const N: usize>(text: &str) -> Result<[f64; N]> {
    let mut out = [0.0f64; N];
    let mut iter = text.split_whitespace();
    for slot in &mut out {
        let tok = iter
            .next()
            .ok_or_else(|| IoError::Invalid("too few floats in COLLADA transform".into()))?;
        let val: f64 = tok
            .parse()
            .map_err(|_| IoError::Invalid("invalid float in COLLADA transform".into()))?;
        if !val.is_finite() {
            return Err(IoError::Invalid(
                "non-finite float in COLLADA transform".into(),
            ));
        }
        *slot = val;
    }
    if iter.next().is_some() {
        return Err(IoError::Invalid(
            "excess floats in COLLADA transform".into(),
        ));
    }
    Ok(out)
}

#[derive(Default)]
struct GeometryCache {
    entries: HashMap<(String, BTreeMap<String, usize>), usize>,
    vertices: usize,
    faces: usize,
}
fn instantiate_geometry_def(
    geom_id: &str,
    bindings: &BTreeMap<String, usize>,
    geometries: &HashMap<String, ParsedGeometry>,
    mat_id_map: &HashMap<String, usize>,
    mat_name_map: &HashMap<String, usize>,
    geom_cache: &mut GeometryCache,
    scene: &mut ImportedScene,
) -> Result<Option<usize>> {
    let key = (geom_id.to_string(), bindings.clone());
    if let Some(&existing) = geom_cache.entries.get(&key) {
        return Ok(Some(existing));
    }
    let pg = geometries
        .get(geom_id)
        .ok_or_else(|| IoError::Invalid(format!("unknown COLLADA geometry #{geom_id}")))?;
    if pg.polygons.is_empty() {
        return Ok(None);
    }

    if geom_cache.vertices.saturating_add(pg.positions.len()) > MAX_VERTICES {
        return Err(IoError::Invalid(
            "material variants exceed scene vertex limit".into(),
        ));
    }
    let eff_tol = TOLERANCE / scene.meters_per_unit.max(1e-6);
    let mut faces = Vec::with_capacity(pg.polygons.len());
    let mut skipped_degenerate = 0usize;

    for poly in &pg.polygons {
        if faces.len().saturating_add(poly.indices.len()) > MAX_TRIANGLES {
            return Err(IoError::Invalid(
                "triangulated polygon count exceeds limit".into(),
            ));
        }
        let resolved_mat_idx = poly.material_symbol.as_ref().and_then(|sym| {
            bindings
                .get(sym)
                .copied()
                .or_else(|| mat_id_map.get(sym).copied())
                .or_else(|| mat_name_map.get(sym).copied())
        });

        let (mat_id, color) = if let Some(m_idx) = resolved_mat_idx {
            let m = &scene.materials[m_idx];
            (
                Some(MaterialId((m_idx + 1) as u64)),
                [
                    m.color[0],
                    m.color[1],
                    m.color[2],
                    (m.opacity * 255.0).round() as u8,
                ],
            )
        } else {
            (None, [160, 177, 193, 255])
        };

        // Clean consecutive duplicate vertices within tolerance.
        let mut ring: Vec<usize> = Vec::with_capacity(poly.indices.len());
        for &idx in &poly.indices {
            if ring
                .last()
                .is_none_or(|&prev| pg.positions[prev].distance(pg.positions[idx]) > eff_tol)
            {
                ring.push(idx);
            }
        }
        while ring.len() >= 2
            && pg.positions[*ring.first().unwrap()].distance(pg.positions[*ring.last().unwrap()])
                <= eff_tol
        {
            ring.pop();
        }
        if ring.len() < 3 {
            skipped_degenerate += 1;
            continue;
        }

        // Check normal & planarity; fan-triangulate if non-planar.
        let a = pg.positions[ring[0]];
        let mut normal = DVec3::ZERO;
        for w in ring[1..].windows(2) {
            normal += (pg.positions[w[0]] - a).cross(pg.positions[w[1]] - a);
        }
        if normal.length() <= eff_tol * eff_tol * 2.0 {
            skipped_degenerate += 1;
            continue;
        }
        let unit_n = normal.normalize();
        let is_planar = ring
            .iter()
            .all(|&vi| (pg.positions[vi] - a).dot(unit_n).abs() <= eff_tol * 0.5);

        if is_planar {
            faces.push(BulkFace {
                outer: ring,
                holes: Vec::new(),
                material: mat_id,
                color,
            });
        } else {
            for w in ring[1..].windows(2) {
                let p1 = pg.positions[w[0]];
                let p2 = pg.positions[w[1]];
                if a.distance(p1) <= eff_tol
                    || p1.distance(p2) <= eff_tol
                    || p2.distance(a) <= eff_tol
                    || (p1 - a).cross(p2 - a).length() <= eff_tol * eff_tol * 2.0
                {
                    skipped_degenerate += 1;
                    continue;
                }
                faces.push(BulkFace {
                    outer: vec![ring[0], w[0], w[1]],
                    holes: Vec::new(),
                    material: mat_id,
                    color,
                });
            }
        }
    }

    if skipped_degenerate > 0 {
        scene.skipped += skipped_degenerate;
        import_warning!(
            scene.warnings,
            format!(
                "skipped {skipped_degenerate} degenerate polygons in {}",
                pg.name
            )
        );
    }
    if faces.is_empty() {
        return Ok(None);
    }

    geom_cache.vertices += pg.positions.len();
    geom_cache.faces = geom_cache.faces.saturating_add(faces.len());
    if geom_cache.faces > MAX_TRIANGLES {
        return Err(IoError::Invalid(
            "material variants exceed scene face limit".into(),
        ));
    }
    let def_idx = scene.definitions.len();
    scene.definitions.push(ImportedDefinition {
        name: pg.name.clone(),
        geometry: BulkGeometry {
            vertices: pg.positions.clone(),
            faces,
            edges: Vec::new(),
        },
        instances: Vec::new(),
    });
    geom_cache.entries.insert(key, def_idx);
    Ok(Some(def_idx))
}

#[allow(clippy::too_many_arguments)]
fn build_visual_scene(
    root: Node<'_, '_>,
    geometries: &HashMap<String, ParsedGeometry>,
    lib_nodes: &HashMap<String, Node<'_, '_>>,
    mat_id_map: &HashMap<String, usize>,
    mat_name_map: &HashMap<String, usize>,
    scene: &mut ImportedScene,
    control: &ImportControl,
) -> Result<()> {
    let mut target_vs_id: Option<String> = None;
    if let Some(scene_el) = root
        .children()
        .find(|n| n.is_element() && n.tag_name().name() == "scene")
    {
        if let Some(ivs) = scene_el
            .children()
            .find(|n| n.is_element() && n.tag_name().name() == "instance_visual_scene")
        {
            if let Some(url) = ivs.attribute("url") {
                target_vs_id = Some(strip_hash(url).to_string());
            }
        }
    }

    let mut visual_scenes = Vec::new();
    for lib in root
        .children()
        .filter(|n| n.is_element() && n.tag_name().name() == "library_visual_scenes")
    {
        for vs in lib
            .children()
            .filter(|n| n.is_element() && n.tag_name().name() == "visual_scene")
        {
            visual_scenes.push(vs);
        }
    }
    if visual_scenes.is_empty() {
        return Ok(());
    }

    let chosen_vs = if let Some(ref want_id) = target_vs_id {
        visual_scenes
            .iter()
            .copied()
            .find(|vs| vs.attribute("id").map(str::trim) == Some(want_id.as_str()))
            .ok_or_else(|| IoError::Invalid(format!("missing visual_scene #{want_id}")))?
    } else {
        visual_scenes[0]
    };

    let mut geom_cache = GeometryCache::default();
    let mut node_def_cache: HashMap<String, (usize, DMat4)> = HashMap::new();
    let mut visited_nodes = 0usize;

    #[allow(clippy::too_many_arguments)]
    fn visit_dae_node(
        node: Node<'_, '_>,
        depth: usize,
        active_ids: &mut BTreeSet<String>,
        geometries: &HashMap<String, ParsedGeometry>,
        lib_nodes: &HashMap<String, Node<'_, '_>>,
        mat_id_map: &HashMap<String, usize>,
        mat_name_map: &HashMap<String, usize>,
        geom_cache: &mut GeometryCache,
        node_def_cache: &mut HashMap<String, (usize, DMat4)>,
        scene: &mut ImportedScene,
        visited_nodes: &mut usize,
        control: &ImportControl,
    ) -> Result<Option<ImportedInstance>> {
        if depth > MAX_DEPTH {
            return Err(IoError::Invalid(
                "COLLADA node hierarchy exceeds depth 64".into(),
            ));
        }
        *visited_nodes += 1;
        if *visited_nodes > MAX_NODES {
            return Err(IoError::Invalid(
                "COLLADA node count exceeds 100k limit".into(),
            ));
        }
        if (*visited_nodes).is_multiple_of(512) {
            control.check()?;
        }

        let node_id = node
            .attribute("id")
            .map(str::trim)
            .filter(|s| !s.is_empty());
        if let Some(id) = node_id {
            if !active_ids.insert(id.to_string()) {
                return Err(IoError::Invalid("cycle detected in COLLADA nodes".into()));
            }
        }

        let raw_name = node.attribute("name").or(node_id).unwrap_or("Node").trim();
        let node_name: String = if raw_name.is_empty() {
            "Node".into()
        } else {
            raw_name
                .chars()
                .filter(|c| !c.is_control())
                .take(512)
                .collect()
        };

        let transform = parse_dae_node_transform(node, &mut scene.warnings)?;
        let mut child_instances: Vec<ImportedInstance> = Vec::new();

        for child in node.children().filter(Node::is_element) {
            match child.tag_name().name() {
                "instance_geometry" => {
                    let url = child.attribute("url").ok_or_else(|| {
                        IoError::Invalid("<instance_geometry> missing url".into())
                    })?;
                    let geom_id = strip_hash(url);
                    let mut bindings = BTreeMap::new();
                    for im in child
                        .descendants()
                        .filter(|n| n.is_element() && n.tag_name().name() == "instance_material")
                    {
                        let Some(sym) = im.attribute("symbol") else {
                            continue;
                        };
                        let Some(target) = im.attribute("target") else {
                            continue;
                        };
                        let target_id = strip_hash(target);
                        let m_idx = mat_id_map
                            .get(target_id)
                            .or_else(|| mat_name_map.get(target_id))
                            .copied()
                            .ok_or_else(|| {
                                IoError::Invalid(format!(
                                    "unknown bound material target #{target_id}"
                                ))
                            })?;
                        bindings.insert(sym.trim().to_string(), m_idx);
                    }
                    if let Some(def_idx) = instantiate_geometry_def(
                        geom_id,
                        &bindings,
                        geometries,
                        mat_id_map,
                        mat_name_map,
                        geom_cache,
                        scene,
                    )? {
                        child_instances.push(ImportedInstance {
                            name: scene.definitions[def_idx].name.clone(),
                            definition: def_idx,
                            transform: DMat4::IDENTITY,
                            tag: None,
                            ..Default::default()
                        });
                    }
                }
                "instance_node" => {
                    let url = child
                        .attribute("url")
                        .ok_or_else(|| IoError::Invalid("<instance_node> missing url".into()))?;
                    let target_id = strip_hash(url);
                    if active_ids.contains(target_id) {
                        return Err(IoError::Invalid(
                            "cycle detected in COLLADA <instance_node>".into(),
                        ));
                    }
                    if let Some(&(cached_def, cached_xform)) = node_def_cache.get(target_id) {
                        child_instances.push(ImportedInstance {
                            name: scene.definitions[cached_def].name.clone(),
                            definition: cached_def,
                            transform: cached_xform,
                            tag: None,
                            ..Default::default()
                        });
                    } else {
                        let target_node = *lib_nodes.get(target_id).ok_or_else(|| {
                            IoError::Invalid(format!("unknown <instance_node> target #{target_id}"))
                        })?;
                        if let Some(inst) = visit_dae_node(
                            target_node,
                            depth + 1,
                            active_ids,
                            geometries,
                            lib_nodes,
                            mat_id_map,
                            mat_name_map,
                            geom_cache,
                            node_def_cache,
                            scene,
                            visited_nodes,
                            control,
                        )? {
                            node_def_cache
                                .insert(target_id.to_string(), (inst.definition, inst.transform));
                            child_instances.push(inst);
                        }
                    }
                }
                "node" => {
                    if let Some(inst) = visit_dae_node(
                        child,
                        depth + 1,
                        active_ids,
                        geometries,
                        lib_nodes,
                        mat_id_map,
                        mat_name_map,
                        geom_cache,
                        node_def_cache,
                        scene,
                        visited_nodes,
                        control,
                    )? {
                        child_instances.push(inst);
                    }
                }
                "instance_controller" | "instance_camera" | "instance_light" => {
                    import_warning!(
                        scene.warnings,
                        format!(
                            "unsupported COLLADA node child <{}> ignored",
                            child.tag_name().name()
                        )
                    );
                }
                _ => {}
            }
        }

        if let Some(id) = node_id {
            active_ids.remove(id);
        }

        if child_instances.is_empty() {
            return Ok(None);
        }

        if child_instances.len() == 1 {
            let mut single = child_instances.pop().unwrap();
            let combined = transform * single.transform;
            cindral_core::validate_instance_transform(combined)?;
            single.name = node_name;
            single.transform = combined;
            return Ok(Some(single));
        }

        let def_idx = scene.definitions.len();
        scene.definitions.push(ImportedDefinition {
            name: node_name.clone(),
            geometry: BulkGeometry::default(),
            instances: child_instances,
        });
        Ok(Some(ImportedInstance {
            name: node_name,
            definition: def_idx,
            transform,
            tag: None,
            ..Default::default()
        }))
    }

    let mut active_ids = BTreeSet::new();
    for top_node in chosen_vs
        .children()
        .filter(|n| n.is_element() && n.tag_name().name() == "node")
    {
        if let Some(inst) = visit_dae_node(
            top_node,
            1,
            &mut active_ids,
            geometries,
            lib_nodes,
            mat_id_map,
            mat_name_map,
            &mut geom_cache,
            &mut node_def_cache,
            scene,
            &mut visited_nodes,
            control,
        )? {
            scene.roots.push(inst);
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::import_scene::{convert_scene, ImportOptions};

    const HANDWRITTEN_DAE: &str = r##"<?xml version="1.0" encoding="utf-8"?>
<COLLADA xmlns="http://www.collada.org/2005/11/COLLADASchema" version="1.4.1">
  <asset>
    <unit name="centimeter" meter="0.01"/>
    <up_axis>Z_UP</up_axis>
  </asset>
  <library_effects>
    <effect id="RedEffect">
      <profile_COMMON>
        <technique sid="common">
          <phong>
            <diffuse><color>0.9 0.1 0.2 1.0</color></diffuse>
            <transparency><float>0.75</float></transparency>
          </phong>
        </technique>
      </profile_COMMON>
    </effect>
  </library_effects>
  <library_materials>
    <material id="RedMat" name="Crimson">
      <instance_effect url="#RedEffect"/>
    </material>
  </library_materials>
  <library_geometries>
    <geometry id="QuadMesh" name="Quad">
      <mesh>
        <source id="QuadPos">
          <float_array id="QuadPosArr" count="12">
            0 0 0  100 0 0  100 200 0  0 200 0
          </float_array>
          <technique_common>
            <accessor source="#QuadPosArr" count="4" stride="3">
              <param name="X" type="float"/>
              <param name="Y" type="float"/>
              <param name="Z" type="float"/>
            </accessor>
          </technique_common>
        </source>
        <vertices id="QuadVerts">
          <input semantic="POSITION" source="#QuadPos"/>
        </vertices>
        <polylist material="MatSym" count="1">
          <input semantic="VERTEX" source="#QuadVerts" offset="0"/>
          <vcount>4</vcount>
          <p>0 1 2 3</p>
        </polylist>
      </mesh>
    </geometry>
  </library_geometries>
  <library_nodes>
    <node id="SharedComp" name="SharedComp">
      <instance_geometry url="#QuadMesh">
        <bind_material>
          <technique_common>
            <instance_material symbol="MatSym" target="#RedMat"/>
          </technique_common>
        </bind_material>
      </instance_geometry>
    </node>
  </library_nodes>
  <library_visual_scenes>
    <visual_scene id="Scene" name="Scene">
      <node id="PlaceA" name="PlaceA">
        <translate>100 0 0</translate>
        <instance_node url="#SharedComp"/>
      </node>
      <node id="PlaceB" name="PlaceB">
        <matrix>
          1 0 0 0
          0 1 0 300
          0 0 1 0
          0 0 0 1
        </matrix>
        <instance_node url="#SharedComp"/>
      </node>
    </visual_scene>
  </library_visual_scenes>
  <scene>
    <instance_visual_scene url="#Scene"/>
  </scene>
</COLLADA>"##;

    #[test]
    fn dae_polylist_unit_library_nodes_and_materials_parse() {
        let control = ImportControl::default();
        let scene = import_dae_bytes(HANDWRITTEN_DAE.as_bytes(), None, &control).unwrap();
        assert!((scene.meters_per_unit - 0.01).abs() < 1e-9);
        assert!(matches!(scene.up_axis, UpAxis::Z));
        assert_eq!(scene.materials.len(), 1);
        assert!((scene.materials[0].opacity - 0.75).abs() < 1e-5);
        assert_eq!(scene.definitions.len(), 1);
        assert_eq!(scene.roots.len(), 2);
        assert_eq!(scene.roots[0].definition, 0);
        assert_eq!(scene.roots[1].definition, 0);

        let res = convert_scene(scene, &ImportOptions::default(), &control).unwrap();
        assert_eq!(res.report.faces, 1);
        assert_eq!(res.report.vertices, 4);
    }

    #[test]
    fn dae_dtd_entities_and_mutated_bytes_rejected_without_panic() {
        let control = ImportControl::default();
        let dtd_xml = br#"<?xml version="1.0"?><!DOCTYPE lolz [<!ENTITY lol "lol">]><COLLADA/>"#;
        assert!(import_dae_bytes(dtd_xml, None, &control).is_err());

        let bytes = HANDWRITTEN_DAE.as_bytes();
        for cut in (0..bytes.len()).step_by(19) {
            let _ = import_dae_bytes(&bytes[..cut], None, &control);
        }
        for idx in (0..bytes.len()).step_by(23) {
            let mut mutated = bytes.to_vec();
            mutated[idx] ^= 0x80;
            let _ = import_dae_bytes(&mutated, None, &control);
        }
    }
}
