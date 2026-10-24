//! COLLADA 1.4.1 export from the public Khronos specification.
use crate::{DraftBundle, ExportSummary, IoError, Result};
use cindral_core::{
    DVec3, DefinitionId, DocumentData, EdgeId, Face, FaceId, InstanceId, InstancePath, Material,
    MaterialId, Model, VertexId,
};
use glam::DMat4;
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    fmt::Write as _,
    io::Write,
    path::Path,
    sync::{
        atomic::{AtomicU32, Ordering},
        Arc, Mutex,
    },
};

/// Output length convention. Both choices retain right-handed Z-up axes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DaeUnits {
    /// One coordinate unit is one meter.
    #[default]
    Meters,
    /// One coordinate unit is one inch (0.0254 meters).
    Inches,
}
impl DaeUnits {
    fn meter(self) -> f64 {
        match self {
            Self::Meters => 1.,
            Self::Inches => 0.0254,
        }
    }
}
/// Explicit entities selected in one editing context; empty means empty output.
#[derive(Clone, Debug, Default)]
pub struct DaeSelection {
    /// Root-relative occurrence path of the editing context.
    pub context: InstancePath,
    /// Selected local faces.
    pub faces: BTreeSet<FaceId>,
    /// Selected local edges.
    pub edges: BTreeSet<EdgeId>,
    /// Selected direct child containers, exported with their complete contents.
    pub instances: BTreeSet<InstanceId>,
}
/// Export settings shared by the desktop and CLI.
#[derive(Clone, Debug)]
pub struct DaeOptions {
    /// Length convention.
    pub units: DaeUnits,
    /// Triangulate convex faces too.
    pub triangulate_all: bool,
    /// Include containers hidden explicitly or by their tags.
    pub include_hidden: bool,
    /// Copy referenced image assets into a sibling directory.
    pub include_textures: bool,
    /// Permit replacing an existing destination after caller confirmation.
    pub force: bool,
    /// Export only these entities; None exports the complete document.
    pub selection: Option<DaeSelection>,
}
impl Default for DaeOptions {
    fn default() -> Self {
        Self {
            units: DaeUnits::Meters,
            triangulate_all: false,
            include_hidden: false,
            include_textures: true,
            force: false,
            selection: None,
        }
    }
}
#[derive(Default)]
struct State {
    cancelled: bool,
    committed: bool,
}
/// Cooperative cancellation/progress with an indivisible publication boundary.
#[derive(Clone, Default)]
pub struct DaeControl {
    state: Arc<Mutex<State>>,
    progress: Arc<AtomicU32>,
}
impl DaeControl {
    /// Request cancellation. False means publication already completed.
    pub fn cancel(&self) -> bool {
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        if state.committed {
            return false;
        }
        state.cancelled = true;
        true
    }
    /// Completed fraction in 0..=1.
    pub fn progress(&self) -> f32 {
        self.progress.load(Ordering::Relaxed) as f32 / 1000.
    }
    fn set_progress(&self, p: f32) {
        self.progress.store((p * 1000.) as u32, Ordering::Relaxed);
    }
    fn check(&self) -> Result<()> {
        if self
            .state
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .cancelled
        {
            Err(IoError::Invalid("COLLADA export cancelled".into()))
        } else {
            Ok(())
        }
    }
}
fn xml(s: &str) -> String {
    s.chars()
        .filter(|&c| {
            matches!(c, '\t' | '\n' | '\r') || c >= ' ' && c != '\u{fffe}' && c != '\u{ffff}'
        })
        .fold(String::new(), |mut out, c| {
            out.push_str(match c {
                '&' => "&amp;",
                '<' => "&lt;",
                '>' => "&gt;",
                '"' => "&quot;",
                '\'' => "&apos;",
                _ => {
                    out.push(c);
                    return out;
                }
            });
            out
        })
}
fn floats(out: &mut String, values: impl IntoIterator<Item = f64>) {
    for v in values {
        let _ = write!(out, "{v} ");
    }
}
fn source(out: &mut String, id: &str, values: &[f64], params: &[&str]) {
    let _ = write!(
        out,
        "<source id=\"{id}\"><float_array id=\"{id}-array\" count=\"{}\">",
        values.len()
    );
    floats(out, values.iter().copied());
    let _ = write!(out, "</float_array><technique_common><accessor source=\"#{id}-array\" count=\"{}\" stride=\"{}\">", values.len()/params.len(), params.len());
    for p in params {
        let _ = write!(out, "<param name=\"{p}\" type=\"float\"/>");
    }
    out.push_str("</accessor></technique_common></source>");
}
fn color_key(c: [u8; 4]) -> String {
    format!("c{}_{}_{}_{}", c[0], c[1], c[2], c[3])
}
fn face_symbol(face: &Face) -> String {
    face.material.map_or_else(
        || {
            if face.color == [160, 177, 193, 255] {
                "inherit".into()
            } else {
                color_key(face.color)
            }
        },
        |id| format!("m{}", id.0),
    )
}
fn convex(model: &Model, face: &Face) -> bool {
    if !face.holes.is_empty() {
        return false;
    }
    let ring = &face.outer;
    (0..ring.len()).all(|i| {
        let a = model.vertices()[&ring[i]].position;
        let b = model.vertices()[&ring[(i + 1) % ring.len()]].position;
        let c = model.vertices()[&ring[(i + 2) % ring.len()]].position;
        (b - a).cross(c - b).dot(face.plane.normal) >= 0.
    })
}
fn uv(p: DVec3, normal: DVec3) -> [f64; 2] {
    let n = normal.abs();
    if n.x >= n.y && n.x >= n.z {
        [p.y, p.z]
    } else if n.y >= n.z {
        [p.x, p.z]
    } else {
        [p.x, p.y]
    }
}
#[derive(Default)]
struct Mesh {
    xml: String,
    symbols: BTreeSet<String>,
    triangles: usize,
    nonempty: bool,
}
fn geometry(
    id: DefinitionId,
    model: &Model,
    options: &DaeOptions,
    selection: Option<&DaeSelection>,
    control: &DaeControl,
) -> Result<Mesh> {
    let mut mesh = Mesh::default();
    let faces: Vec<_> = model
        .faces()
        .iter()
        .filter(|(id, _)| selection.is_none_or(|s| s.faces.contains(id)))
        .collect();
    let boundary: BTreeSet<_> = model
        .faces()
        .values()
        .flat_map(|f| std::iter::once(&f.outer).chain(f.holes.iter()))
        .flat_map(|r| {
            (0..r.len()).map(|i| {
                let a = r[i];
                let b = r[(i + 1) % r.len()];
                (a.min(b), a.max(b))
            })
        })
        .collect();
    let edges: Vec<_> = model
        .edges()
        .iter()
        .filter(|(id, e)| {
            (options.include_hidden || !e.hidden)
                && selection.map_or_else(
                    || {
                        !boundary.contains(&(
                            e.vertices[0].min(e.vertices[1]),
                            e.vertices[0].max(e.vertices[1]),
                        ))
                    },
                    |s| s.edges.contains(id),
                )
        })
        .collect();
    if faces.is_empty() && edges.is_empty() {
        return Ok(mesh);
    }
    mesh.nonempty = true;
    let used: BTreeSet<_> = faces
        .iter()
        .flat_map(|(_, f)| {
            std::iter::once(&f.outer)
                .chain(f.holes.iter())
                .flatten()
                .copied()
        })
        .chain(edges.iter().flat_map(|(_, e)| e.vertices))
        .collect();
    let mut indices = BTreeMap::<VertexId, usize>::new();
    let mut positions = Vec::new();
    let mut by_position = HashMap::new();
    for vertex in used {
        let p = model.vertices()[&vertex].position;
        let index = positions.len() / 3;
        indices.insert(vertex, index);
        by_position.insert(p.to_array().map(f64::to_bits), index);
        positions.extend(p.to_array().map(|v| v / options.units.meter()));
    }
    let mut normals = Vec::new();
    let mut uvs = Vec::new();
    let mut primitives = BTreeMap::<(String, bool), Vec<Vec<[usize; 3]>>>::new();
    for (face_id, face) in faces {
        control.check()?;
        let normal = normals.len() / 3;
        normals.extend(face.plane.normal.to_array());
        let mut corners = BTreeMap::new();
        for vertex in std::iter::once(&face.outer)
            .chain(face.holes.iter())
            .flatten()
        {
            let uv_index = uvs.len() / 2;
            uvs.extend(uv(model.vertices()[vertex].position, face.plane.normal));
            corners.insert(indices[vertex], [indices[vertex], normal, uv_index]);
        }
        let symbol = face_symbol(face);
        mesh.symbols.insert(symbol.clone());
        let triangles = options.triangulate_all || !convex(model, face);
        let output = primitives.entry((symbol, triangles)).or_default();
        if triangles {
            for mut tri in model.triangulate(*face_id)? {
                if (tri[1] - tri[0])
                    .cross(tri[2] - tri[0])
                    .dot(face.plane.normal)
                    < 0.
                {
                    tri.swap(1, 2);
                }
                let mut polygon = Vec::new();
                for p in tri {
                    let index = by_position
                        .get(&p.to_array().map(f64::to_bits))
                        .ok_or_else(|| {
                            IoError::Invalid("triangulation introduced an unknown vertex".into())
                        })?;
                    polygon.push(corners[index]);
                }
                output.push(polygon);
                mesh.triangles += 1;
            }
        } else {
            output.push(face.outer.iter().map(|v| corners[&indices[v]]).collect());
            mesh.triangles += face.outer.len() - 2;
        }
    }
    let out = &mut mesh.xml;
    let gid = format!("g{}", id.0);
    let _ = write!(out, "<geometry id=\"{gid}\"><mesh>");
    source(out, &format!("{gid}-pos"), &positions, &["X", "Y", "Z"]);
    if !normals.is_empty() {
        source(out, &format!("{gid}-norm"), &normals, &["X", "Y", "Z"]);
        source(out, &format!("{gid}-uv"), &uvs, &["S", "T"]);
    }
    let _ = write!(
        out,
        "<vertices id=\"{gid}-v\"><input semantic=\"POSITION\" source=\"#{gid}-pos\"/></vertices>"
    );
    for ((symbol, triangles), polygons) in primitives {
        let tag = if triangles { "triangles" } else { "polylist" };
        let _=write!(out,"<{tag} material=\"{symbol}\" count=\"{}\"><input semantic=\"VERTEX\" source=\"#{gid}-v\" offset=\"0\"/><input semantic=\"NORMAL\" source=\"#{gid}-norm\" offset=\"1\"/><input semantic=\"TEXCOORD\" source=\"#{gid}-uv\" offset=\"2\" set=\"0\"/>",polygons.len());
        if !triangles {
            out.push_str("<vcount>");
            for p in &polygons {
                let _ = write!(out, "{} ", p.len());
            }
            out.push_str("</vcount>");
        }
        out.push_str("<p>");
        for p in polygons {
            for corner in p {
                let _ = write!(out, "{} {} {} ", corner[0], corner[1], corner[2]);
            }
        }
        let _ = write!(out, "</p></{tag}>");
    }
    if !edges.is_empty() {
        let _ = write!(
            out,
            "<lines count=\"{}\"><input semantic=\"VERTEX\" source=\"#{gid}-v\" offset=\"0\"/><p>",
            edges.len()
        );
        for (_, e) in edges {
            let _ = write!(
                out,
                "{} {} ",
                indices[&e.vertices[0]], indices[&e.vertices[1]]
            );
        }
        out.push_str("</p></lines>");
    }
    out.push_str("</mesh></geometry>");
    Ok(mesh)
}
fn tinted_png(
    asset: &crate::TextureAsset,
    color: [f32; 3],
    control: &DaeControl,
) -> Result<Vec<u8>> {
    let decoded = crate::decode_texture(asset)?;
    let mut pixels = decoded.rgba;
    for row in pixels.chunks_mut(decoded.width as usize * 4) {
        control.check()?;
        for pixel in row.chunks_mut(4) {
            for channel in 0..3 {
                let s = pixel[channel] as f32 / 255.;
                let linear = if s <= 0.04045 {
                    s / 12.92
                } else {
                    ((s + 0.055) / 1.055).powf(2.4)
                };
                let value = linear * color[channel];
                let srgb = if value <= 0.0031308 {
                    12.92 * value
                } else {
                    1.055 * value.powf(1. / 2.4) - 0.055
                };
                pixel[channel] = (srgb.clamp(0., 1.) * 255.).round() as u8;
            }
        }
    }
    let image = image::RgbaImage::from_raw(decoded.width, decoded.height, pixels)
        .ok_or_else(|| IoError::Invalid("invalid decoded image".into()))?;
    let mut bytes = std::io::Cursor::new(Vec::new());
    image
        .write_to(&mut bytes, image::ImageFormat::Png)
        .map_err(|e| IoError::Invalid(format!("texture encoding failed: {e}")))?;
    Ok(bytes.into_inner())
}
type Variant = (DefinitionId, Option<MaterialId>);
fn node_id((def, mat): Variant) -> String {
    format!(
        "n{}-{}",
        def.0,
        mat.map_or_else(|| "default".into(), |m| m.0.to_string())
    )
}
struct Scene<'a> {
    doc: &'a DocumentData,
    options: &'a DaeOptions,
    meshes: BTreeMap<DefinitionId, Mesh>,
    variants: BTreeSet<Variant>,
    control: &'a DaeControl,
}
impl Scene<'_> {
    fn visible(&self, inst: &cindral_core::Instance) -> bool {
        self.options.include_hidden
            || !inst.hidden
                && inst
                    .tag
                    .is_none_or(|t| self.doc.tags.get(&t).is_none_or(|t| t.visible))
    }
    fn collect(&mut self, variant: Variant, selected: bool) -> Result<()> {
        self.control.check()?;
        if !self.variants.insert(variant) {
            return Ok(());
        }
        let def = &self.doc.definitions[&variant.0];
        for inst in def.instances.values() {
            if self.visible(inst)
                && (!selected
                    || self
                        .options
                        .selection
                        .as_ref()
                        .is_some_and(|s| s.instances.contains(&inst.id)))
            {
                self.collect((inst.definition, inst.material.or(variant.1)), false)?;
            }
        }
        Ok(())
    }
    fn contents(&self, out: &mut String, variant: Variant, selected: bool) {
        let (def, inherited) = variant;
        if let Some(mesh) = self.meshes.get(&def).filter(|m| m.nonempty) {
            let _ = write!(out, "<instance_geometry url=\"#g{}\">", def.0);
            if !mesh.symbols.is_empty() {
                out.push_str("<bind_material><technique_common>");
                for symbol in &mesh.symbols {
                    let target = if symbol == "inherit" {
                        inherited.map_or_else(
                            || color_key([160, 177, 193, 255]),
                            |id| format!("m{}", id.0),
                        )
                    } else {
                        symbol.clone()
                    };
                    let _=write!(out,"<instance_material symbol=\"{symbol}\" target=\"#{target}\"><bind_vertex_input semantic=\"UVSET0\" input_semantic=\"TEXCOORD\" input_set=\"0\"/></instance_material>");
                }
                out.push_str("</technique_common></bind_material>");
            }
            out.push_str("</instance_geometry>");
        }
        for inst in self.doc.definitions[&def].instances.values() {
            if !self.visible(inst)
                || selected
                    && !self
                        .options
                        .selection
                        .as_ref()
                        .is_some_and(|s| s.instances.contains(&inst.id))
            {
                continue;
            }
            let _ = write!(out, "<node name=\"{}\"><matrix>", xml(&inst.name));
            let mut matrix = inst.transform;
            matrix.w_axis.x /= self.options.units.meter();
            matrix.w_axis.y /= self.options.units.meter();
            matrix.w_axis.z /= self.options.units.meter();
            floats(out, matrix.transpose().to_cols_array());
            let _ = write!(
                out,
                "</matrix><instance_node url=\"#{}\"/></node>",
                node_id((inst.definition, inst.material.or(inherited)))
            );
        }
    }
    fn counts(
        &self,
        variant: Variant,
        root: Variant,
        memo: &mut BTreeMap<Variant, ExportSummary>,
    ) -> Result<ExportSummary> {
        if let Some(s) = memo.get(&variant) {
            return Ok(*s);
        }
        self.control.check()?;
        let mut s = ExportSummary::default();
        if let Some(m) = self.meshes.get(&variant.0).filter(|m| m.nonempty) {
            s.instances = 1;
            s.triangles = m.triangles;
        }
        for i in self.doc.definitions[&variant.0].instances.values() {
            if !self.visible(i)
                || variant == root
                    && self
                        .options
                        .selection
                        .as_ref()
                        .is_some_and(|sel| !sel.instances.contains(&i.id))
            {
                continue;
            }
            let child = self.counts((i.definition, i.material.or(variant.1)), root, memo)?;
            s.instances = s
                .instances
                .checked_add(child.instances)
                .ok_or_else(|| IoError::Invalid("export occurrence count overflow".into()))?;
            s.triangles = s
                .triangles
                .checked_add(child.triangles)
                .ok_or_else(|| IoError::Invalid("export triangle count overflow".into()))?;
        }
        memo.insert(variant, s);
        Ok(s)
    }
}
/// Export a complete COLLADA 1.4.1 document and relative texture sidecars.
/// Validates before writing. Failure/cancellation preserves the prior destination;
/// cancellation after the publication boundary returns false and keeps the complete export.
pub fn export_dae(
    path: &Path,
    bundle: &DraftBundle,
    options: &DaeOptions,
    control: &DaeControl,
) -> Result<ExportSummary> {
    export_inner(path, bundle, options, control).map_err(|e| match e {
        IoError::Io(e) => IoError::Io(std::io::Error::new(
            e.kind(),
            format!("COLLADA export to {} failed: {e}", path.display()),
        )),
        other => other,
    })
}
fn export_inner(
    path: &Path,
    bundle: &DraftBundle,
    options: &DaeOptions,
    control: &DaeControl,
) -> Result<ExportSummary> {
    control.check()?;
    if !options.force && path.try_exists()? {
        return Err(IoError::Invalid(format!(
            "{} already exists; confirm overwrite or use --force",
            path.display()
        )));
    }
    let history = crate::validate_bundle(bundle)?;
    control.check()?;
    control.set_progress(0.1);
    let doc = &bundle.document;
    let mut root = doc.root;
    let mut world = DMat4::IDENTITY;
    let mut inherited = None;
    if let Some(s) = &options.selection {
        for id in &s.context {
            let i = doc.definitions[&root]
                .instances
                .get(id)
                .ok_or_else(|| IoError::Invalid("invalid export selection context".into()))?;
            if !options.include_hidden
                && (i.hidden
                    || i.tag
                        .is_some_and(|t| doc.tags.get(&t).is_some_and(|t| !t.visible)))
            {
                return Err(IoError::Invalid(
                    "selection context is hidden; enable include hidden".into(),
                ));
            }
            root = i.definition;
            world *= i.transform;
            inherited = i.material.or(inherited);
        }
        let def = &doc.definitions[&root];
        if !s.faces.iter().all(|id| def.geometry.faces.contains_key(id))
            || !s.edges.iter().all(|id| def.geometry.edges.contains_key(id))
            || !s.instances.iter().all(|id| def.instances.contains_key(id))
        {
            return Err(IoError::Invalid(
                "export selection contains missing entities".into(),
            ));
        }
    }
    let root_variant = (root, inherited);
    let mut scene = Scene {
        doc,
        options,
        meshes: BTreeMap::new(),
        variants: BTreeSet::new(),
        control,
    };
    scene.collect(root_variant, options.selection.is_some())?;
    let definitions: BTreeSet<_> = scene.variants.iter().map(|v| v.0).collect();
    for (index, id) in definitions.iter().enumerate() {
        let model = history
            .definition_model(*id)
            .ok_or(cindral_core::KernelError::MissingEntity)?;
        let mesh = geometry(
            *id,
            model,
            options,
            if *id == root {
                options.selection.as_ref()
            } else {
                None
            },
            control,
        )?;
        scene.meshes.insert(*id, mesh);
        control.set_progress(0.1 + 0.5 * (index + 1) as f32 / definitions.len().max(1) as f32);
    }
    let summary = scene.counts(root_variant, root_variant, &mut BTreeMap::new())?;
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let staging = tempfile::Builder::new()
        .prefix("cindral-textures-")
        .tempdir_in(parent)?;
    let folder = staging
        .path()
        .file_name()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    let staged_images = staging.path();
    let mut materials = BTreeMap::<String, Material>::new();
    materials.insert(
        color_key([160, 177, 193, 255]),
        Material {
            name: "Default".into(),
            color: [160, 177, 193],
            opacity: 1.,
            texture: None,
        },
    );
    for (id, m) in &doc.materials {
        materials.insert(format!("m{}", id.0), m.clone());
    }
    for id in definitions {
        for face in doc.definitions[&id].geometry.faces.values() {
            if face.material.is_none() && face.color != [160, 177, 193, 255] {
                materials
                    .entry(color_key(face.color))
                    .or_insert_with(|| Material {
                        name: color_key(face.color),
                        color: [face.color[0], face.color[1], face.color[2]],
                        opacity: face.color[3] as f32 / 255.,
                        texture: None,
                    });
            }
        }
    }
    let mut needed = BTreeSet::new();
    for &(id, inherited) in &scene.variants {
        if let Some(mesh) = scene.meshes.get(&id) {
            for symbol in &mesh.symbols {
                needed.insert(if symbol == "inherit" {
                    inherited
                        .map_or_else(|| color_key([160, 177, 193, 255]), |m| format!("m{}", m.0))
                } else {
                    symbol.clone()
                });
            }
        }
    }
    materials.retain(|id, _| needed.contains(id));
    let mut textures = BTreeMap::new();
    if options.include_textures {
        for (material_id, m) in &mut materials {
            if let Some(key) = m.texture.clone() {
                control.check()?;
                let asset = bundle
                    .textures
                    .get(&key)
                    .ok_or_else(|| IoError::Invalid(format!("missing texture asset {key}")))?;
                // COLLADA diffuse is color OR texture. Bake the native linear tint into
                // an image so standard readers retain the rendered appearance.
                let tinted = m.color != [255, 255, 255];
                let output_key = if tinted {
                    format!("tint-{material_id}")
                } else {
                    format!("asset-{key}")
                };
                if !textures.contains_key(&output_key) {
                    let bytes = if tinted {
                        tinted_png(asset, cindral_core::srgb_to_linear_rgb(m.color), control)?
                    } else {
                        asset.bytes.clone()
                    };
                    let index = textures.len();
                    let ext = if tinted || asset.mime == "image/png" {
                        "png"
                    } else {
                        "jpg"
                    };
                    let filename = format!("image{index}.{ext}");
                    let mut file = std::fs::File::create(staged_images.join(&filename))?;
                    for chunk in bytes.chunks(64 * 1024) {
                        control.check()?;
                        file.write_all(chunk)?;
                    }
                    file.sync_all()?;
                    textures.insert(
                        output_key.clone(),
                        (format!("image{index}"), format!("{folder}/{filename}")),
                    );
                }
                m.texture = Some(output_key);
            }
        }
    }
    let mut out=String::from("<?xml version=\"1.0\" encoding=\"utf-8\"?><COLLADA xmlns=\"http://www.collada.org/2005/11/COLLADASchema\" version=\"1.4.1\"><asset><contributor><authoring_tool>Cindral Drafter</authoring_tool></contributor><created>1970-01-01T00:00:00Z</created><modified>1970-01-01T00:00:00Z</modified>");
    let _ = write!(
        out,
        "<unit name=\"{}\" meter=\"{}\"/><up_axis>Z_UP</up_axis></asset>",
        if options.units == DaeUnits::Meters {
            "meter"
        } else {
            "inch"
        },
        options.units.meter()
    );
    if !textures.is_empty() {
        out.push_str("<library_images>");
        for (id, path) in textures.values() {
            let _ = write!(
                out,
                "<image id=\"{id}\"><init_from>{path}</init_from></image>"
            );
        }
        out.push_str("</library_images>");
    }
    if !materials.is_empty() {
        out.push_str("<library_effects>");
        for (id, m) in &materials {
            let _ = write!(out, "<effect id=\"{id}-fx\"><profile_COMMON>");
            let texture = m.texture.as_ref().and_then(|key| textures.get(key));
            if let Some((image, _)) = texture {
                let _=write!(out,"<newparam sid=\"surface\"><surface type=\"2D\"><init_from>{image}</init_from></surface></newparam><newparam sid=\"sampler\"><sampler2D><source>surface</source><wrap_s>WRAP</wrap_s><wrap_t>WRAP</wrap_t></sampler2D></newparam>");
            }
            out.push_str("<technique sid=\"common\"><lambert><diffuse>");
            if texture.is_some() {
                out.push_str("<texture texture=\"sampler\" texcoord=\"UVSET0\"/>");
            } else {
                let rgb = crate::srgb_floats(m.color);
                let _ = write!(out, "<color>{} {} {} 1</color>", rgb[0], rgb[1], rgb[2]);
            }
            out.push_str("</diffuse><transparent opaque=\"A_ONE\">");
            if texture.is_some() {
                out.push_str("<texture texture=\"sampler\" texcoord=\"UVSET0\"/>");
            } else {
                out.push_str("<color>1 1 1 1</color>");
            }
            let _=write!(out,"</transparent><transparency><float>{}</float></transparency></lambert></technique></profile_COMMON></effect>",m.opacity);
        }
        out.push_str("</library_effects><library_materials>");
        for (id, m) in &materials {
            let _ = write!(
                out,
                "<material id=\"{id}\" name=\"{}\"><instance_effect url=\"#{id}-fx\"/></material>",
                xml(&m.name)
            );
        }
        out.push_str("</library_materials>");
    }
    if scene.meshes.values().any(|m| m.nonempty) {
        out.push_str("<library_geometries>");
        for mesh in scene.meshes.values() {
            out.push_str(&mesh.xml);
        }
        out.push_str("</library_geometries>");
    }
    if scene.variants.len() > 1 {
        out.push_str("<library_nodes>");
        for &variant in &scene.variants {
            if variant == root_variant {
                continue;
            }
            control.check()?;
            let _ = write!(
                out,
                "<node id=\"{}\" name=\"{}\">",
                node_id(variant),
                xml(&doc.definitions[&variant.0].name)
            );
            scene.contents(&mut out, variant, false);
            out.push_str("</node>");
        }
        out.push_str("</library_nodes>");
    }
    out.push_str("<library_visual_scenes><visual_scene id=\"scene\"><node name=\"Model\"><matrix>");
    world.w_axis.x /= options.units.meter();
    world.w_axis.y /= options.units.meter();
    world.w_axis.z /= options.units.meter();
    floats(&mut out, world.transpose().to_cols_array());
    out.push_str("</matrix>");
    scene.contents(&mut out, root_variant, options.selection.is_some());
    out.push_str("</node></visual_scene></library_visual_scenes><scene><instance_visual_scene url=\"#scene\"/></scene></COLLADA>");
    control.check()?;
    control.set_progress(0.85);
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    for chunk in out.as_bytes().chunks(64 * 1024) {
        control.check()?;
        temporary.write_all(chunk)?;
    }
    temporary.as_file_mut().sync_all()?;
    let mut state = control.state.lock().unwrap_or_else(|p| p.into_inner());
    if state.cancelled {
        return Err(IoError::Invalid("COLLADA export cancelled".into()));
    }
    let published = if options.force {
        temporary.persist(path)
    } else {
        temporary.persist_noclobber(path)
    };
    if let Err(error) = published {
        return Err(IoError::Io(error.error));
    }
    if !textures.is_empty() {
        let _ = staging.keep();
    }
    state.committed = true;
    control.set_progress(1.);
    Ok(summary)
}
