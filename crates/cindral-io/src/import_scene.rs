//! Shared bounded import scene and single conversion boundary.
use crate::{IoError, Result, TextureAsset};
use cindral_core::*;
use glam::DMat4;
use std::{
    collections::BTreeMap,
    path::Path,
    sync::{
        atomic::{AtomicBool, AtomicU32, Ordering},
        Arc,
    },
};
/// Source coordinate convention.
#[derive(Clone, Copy, Debug, Default)]
pub enum UpAxis {
    /// X up.
    X,
    /// Y up.
    Y,
    /// Native Z up.
    #[default]
    Z,
}
/// Format-neutral scene, with zero-based references.
#[derive(Clone, Debug)]
pub struct ImportedScene {
    /// Shared local definitions.
    pub definitions: Vec<ImportedDefinition>,
    /// Source model container, when the format distinguishes loose root geometry.
    /// Its sole identity placement is promoted; child components remain instances.
    pub root_definition: Option<usize>,
    /// Top-level placements.
    pub roots: Vec<ImportedInstance>,
    /// Material table; polygon MaterialId is index plus one.
    pub materials: Vec<Material>,
    /// Encoded embedded images.
    pub textures: BTreeMap<String, TextureAsset>,
    /// Visibility tags.
    pub tags: Vec<Tag>,
    /// Source length unit in meters.
    pub meters_per_unit: f64,
    /// Source up direction.
    pub up_axis: UpAxis,
    /// Unsupported/skipped feature diagnostics.
    pub warnings: Vec<String>,
    /// Number of skipped source primitives or degenerate polygons.
    pub skipped: usize,
}
impl Default for ImportedScene {
    fn default() -> Self {
        Self {
            definitions: Vec::new(),
            root_definition: None,
            roots: Vec::new(),
            materials: Vec::new(),
            textures: BTreeMap::new(),
            tags: Vec::new(),
            meters_per_unit: 1.,
            up_axis: UpAxis::Z,
            warnings: Vec::new(),
            skipped: 0,
        }
    }
}
/// One reusable local geometry container.
#[derive(Clone, Debug, Default)]
pub struct ImportedDefinition {
    /// Outliner label.
    pub name: String,
    /// Indexed local geometry.
    pub geometry: BulkGeometry,
    /// Child placements.
    pub instances: Vec<ImportedInstance>,
}
/// Placed definition reference.
#[derive(Clone, Debug)]
pub struct ImportedInstance {
    /// Outliner label.
    pub name: String,
    /// Zero-based definition index.
    pub definition: usize,
    /// Source local-to-parent matrix.
    pub transform: DMat4,
    /// Optional tag table index.
    pub tag: Option<usize>,
    /// Optional instance paint inherited by unpainted faces.
    pub material: Option<MaterialId>,
    /// Placement visibility independent of tags.
    pub hidden: bool,
}
impl Default for ImportedInstance {
    fn default() -> Self {
        Self {
            name: String::new(),
            definition: 0,
            transform: DMat4::IDENTITY,
            tag: None,
            material: None,
            hidden: false,
        }
    }
}
/// Triangle cleanup and placement preferences.
#[derive(Clone, Debug)]
pub struct ImportOptions {
    /// Join disjoint coplanar triangle pairs into planar quads.
    pub merge_coplanar: bool,
    /// Hide/smooth coplanar triangulation seams.
    pub soften_coplanar: bool,
    /// Wrap scene in a single component; false flattens visible occurrences.
    pub as_component: bool,
}
impl Default for ImportOptions {
    fn default() -> Self {
        Self {
            merge_coplanar: true,
            soften_coplanar: true,
            as_component: true,
        }
    }
}
/// Shared cancellation and stage progress, safe across worker/UI threads.
#[derive(Clone, Debug, Default)]
pub struct ImportControl {
    cancelled: Arc<AtomicBool>,
    progress: Arc<AtomicU32>,
}
impl ImportControl {
    /// Request cancellation. Results must be discarded after this request.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }
    /// Whether cancellation was requested.
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Relaxed)
    }
    /// Completed fraction, from zero to one.
    pub fn progress(&self) -> f32 {
        self.progress.load(Ordering::Relaxed) as f32 / 1000.
    }
    /// Fail at a cooperative cancellation boundary.
    pub fn check(&self) -> Result<()> {
        if self.is_cancelled() {
            Err(IoError::Invalid("import cancelled".into()))
        } else {
            Ok(())
        }
    }
    /// Publish stage progress.
    pub fn set_progress(&self, value: f32) {
        self.progress
            .store((value.clamp(0., 1.) * 1000.) as u32, Ordering::Relaxed);
    }
}
/// Summary of constructed geometry, counted once per definition.
#[derive(Clone, Debug, Default)]
pub struct ImportReport {
    /// Welded vertices.
    pub vertices: usize,
    /// Topological edges.
    pub edges: usize,
    /// Cleaned faces.
    pub faces: usize,
    /// Placed instances.
    pub instances: usize,
    /// Explicitly skipped source items.
    pub skipped: usize,
    /// Format/conversion diagnostics.
    pub warnings: Vec<String>,
}
/// Ready-to-install worker result, retaining the single import undo entry.
#[derive(Clone, Debug)]
pub struct ImportResult {
    /// Validated editable document.
    pub history: DocumentHistory,
    /// Encoded texture assets.
    pub textures: BTreeMap<String, TextureAsset>,
    /// Import diagnostics.
    pub report: ImportReport,
}
/// Read a bounded file without allocating its advertised size.
pub(crate) fn read_bounded(path: &Path, limit: usize) -> Result<Vec<u8>> {
    use std::io::Read;
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > limit {
        return Err(IoError::Invalid("import size limit exceeded".into()));
    }
    Ok(bytes)
}
/// Resolve local sidecars without traversal, absolute paths, symlink escapes or network access.
pub(crate) fn read_asset(base: &Path, uri: &str, limit: usize) -> Result<Vec<u8>> {
    use std::path::Component;
    if uri.is_empty()
        || uri.len() > 4096
        || uri.contains([':', '\\', '%'])
        || uri.chars().any(char::is_control)
        || Path::new(uri)
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
    {
        return Err(IoError::Invalid("unsafe import asset path".into()));
    }
    let base = base.canonicalize()?;
    let file = base.join(uri).canonicalize()?;
    if !file.starts_with(base) {
        return Err(IoError::Invalid("asset escapes import directory".into()));
    }
    read_bounded(&file, limit)
}
fn label(s: &str) -> String {
    if s.trim().is_empty() {
        "Imported".into()
    } else {
        s.chars().filter(|c| !c.is_control()).take(512).collect()
    }
}
/// Convert source units/basis, weld/build all definitions, and create one undoable import.
pub fn convert_scene(
    mut scene: ImportedScene,
    options: &ImportOptions,
    control: &ImportControl,
) -> Result<ImportResult> {
    control.check()?;
    if !scene.meters_per_unit.is_finite()
        || scene.meters_per_unit <= 0.
        || scene.definitions.len() > 100_000
        || scene.roots.len() > 100_000
        || scene.materials.len() > 100_000
        || scene.tags.len() > 100_000
        || scene.textures.len() > 127
    {
        return Err(IoError::Invalid(
            "invalid scene units or count limits".into(),
        ));
    }
    let total_vertices: usize = scene
        .definitions
        .iter()
        .map(|d| d.geometry.vertices.len())
        .sum();
    let total_faces: usize = scene
        .definitions
        .iter()
        .map(|d| d.geometry.faces.len())
        .sum();
    let total_indices: usize = scene
        .definitions
        .iter()
        .flat_map(|d| &d.geometry.faces)
        .map(|f| f.outer.len() + f.holes.iter().map(Vec::len).sum::<usize>())
        .sum();
    let total_edges: usize = scene
        .definitions
        .iter()
        .map(|d| d.geometry.edges.len())
        .sum();
    if total_indices > 4_000_000 || total_edges > 3_000_000 {
        return Err(IoError::Invalid("scene index count limit exceeded".into()));
    }
    if total_vertices > 1_000_000 || total_faces > 1_000_000 {
        return Err(IoError::Invalid(
            "scene geometry count limit exceeded".into(),
        ));
    }
    let bytes: usize = scene.textures.values().map(|t| t.bytes.len()).sum();
    if bytes > 64 * 1024 * 1024 {
        return Err(IoError::Invalid("texture bytes limit exceeded".into()));
    }
    if !scene.textures.is_empty() {
        scene.warnings.push("Texture images are preserved; Cindral uses planar texture mapping rather than source UV sets".into());
    }
    if scene.warnings.len() > 1000 {
        scene.warnings.truncate(999);
        scene
            .warnings
            .push("Additional import diagnostics omitted (1000-warning limit)".into());
    }
    for material in &mut scene.materials {
        if material
            .texture
            .as_ref()
            .is_some_and(|key| !scene.textures.contains_key(key))
        {
            material.texture = None;
            scene
                .warnings
                .push("Missing texture reference removed from material".into());
        }
    }
    scene.warnings.truncate(1000);
    for asset in scene.textures.values() {
        control.check()?;
        crate::decode_texture(asset)?;
    }
    let basis = match scene.up_axis {
        UpAxis::Z => DMat4::IDENTITY,
        UpAxis::Y => DMat4::from_rotation_x(std::f64::consts::FRAC_PI_2),
        UpAxis::X => DMat4::from_rotation_y(-std::f64::consts::FRAC_PI_2),
    } * DMat4::from_scale(DVec3::splat(scene.meters_per_unit));
    let inverse = basis.inverse();
    let mut data = DocumentHistory::new().to_data();
    data.materials = scene
        .materials
        .into_iter()
        .enumerate()
        .map(|(i, m)| (MaterialId(i as u64 + 1), m))
        .collect();
    data.tags = scene
        .tags
        .into_iter()
        .enumerate()
        .map(|(i, t)| (TagId(i as u64 + 1), t))
        .collect();
    let mut next = (data.materials.len().max(data.tags.len()) + scene.definitions.len() + 2) as u64;
    let definition_count = scene.definitions.len();
    let source_root = scene.root_definition;
    if let Some(index) = source_root {
        if index >= definition_count || scene.roots.len() != 1
            || scene.roots[0].definition != index
            || scene.roots[0].transform != DMat4::IDENTITY
            || scene.roots[0].tag.is_some() || scene.roots[0].material.is_some()
            || scene.roots[0].hidden
            || scene.definitions.iter().any(|d| d.instances.iter().any(|n| n.definition == index))
        {
            return Err(IoError::Invalid("invalid source root container".into()));
        }
    }
    let mut nodes = 0;
    let mut instances = |items: Vec<ImportedInstance>| -> Result<BTreeMap<InstanceId, Instance>> {
        let mut out = BTreeMap::new();
        for n in items {
            nodes += 1;
            if nodes > 100_000
                || n.definition >= definition_count
                || n.tag.is_some_and(|t| t >= data.tags.len())
            {
                return Err(IoError::Invalid("invalid instance reference/count".into()));
            }
            control.check()?;
            let id = InstanceId(next);
            next += 1;
            out.insert(
                id,
                Instance {
                    id,
                    name: label(&n.name),
                    definition: DefinitionId(n.definition as u64 + 1),
                    transform: basis * n.transform * inverse,
                    tag: n.tag.map(|i| TagId(i as u64 + 1)),
                    material: n.material,
                    hidden: n.hidden,
                },
            );
        }
        Ok(out)
    };
    for (i, mut def) in scene.definitions.drain(..).enumerate() {
        control.check()?;
        control.set_progress(0.45 + 0.35 * i as f32 / definition_count.max(1) as f32);
        for p in &mut def.geometry.vertices {
            *p = basis.transform_point3(*p);
        }
        let model = def.geometry.build_cancellable(
            options.merge_coplanar,
            options.soften_coplanar,
            || control.is_cancelled(),
        )?;
        data.definitions.insert(
            DefinitionId(i as u64 + 1),
            DefinitionData {
                name: label(&def.name),
                kind: DefinitionKind::Component,
                geometry: model.to_data(),
                instances: instances(def.instances)?,
            },
        );
    }
    let roots = instances(scene.roots)?;
    data.definitions
        .get_mut(&DefinitionId(0))
        .ok_or_else(|| IoError::Invalid("missing root".into()))?
        .instances = roots;
    if let Some(index) = source_root {
        let root = data.definitions.remove(&DefinitionId(index as u64 + 1))
            .ok_or_else(|| IoError::Invalid("missing source root".into()))?;
        data.definitions.insert(DefinitionId(0), DefinitionData {
            name: "Root".into(), kind: DefinitionKind::Root, ..root
        });
    }
    data.next_id = next;
    // Validate the graph before traversing it (including cycles and expanded occurrence cap).
    let history = if options.as_component {
        let root = data
            .definitions
            .get_mut(&DefinitionId(0))
            .ok_or_else(|| IoError::Invalid("missing root".into()))?;
        let children = std::mem::take(&mut root.instances);
        let geometry = std::mem::replace(&mut root.geometry, Model::default().to_data());
        let id = DefinitionId(data.next_id);
        data.next_id += 1;
        let node = InstanceId(data.next_id);
        data.next_id += 1;
        data.definitions.insert(
            id,
            DefinitionData {
                name: "Imported scene".into(),
                kind: DefinitionKind::Component,
                geometry,
                instances: children,
            },
        );
        data.definitions
            .get_mut(&DefinitionId(0))
            .ok_or_else(|| IoError::Invalid("missing root".into()))?
            .instances
            .insert(
                node,
                Instance {
                    id: node,
                    name: "Imported scene".into(),
                    definition: id,
                    transform: DMat4::IDENTITY,
                    tag: None,
                    material: None,
                    hidden: false,
                },
            );
        DocumentHistory::from_import_data(data)?
    } else if source_root.is_some() {
        DocumentHistory::from_import_data(data)?
    } else {
        let source = DocumentHistory::from_import_data(data)?;
        let mut flat = BulkGeometry::default();
        for occurrence in source.occurrences()? {
            control.check()?;
            let m = source
                .definition_model(occurrence.definition)
                .ok_or_else(|| IoError::Invalid("missing definition".into()))?;
            if flat.vertices.len() + m.vertices().len() > 1_000_000
                || flat.faces.len() + m.faces().len() > 1_000_000
            {
                return Err(IoError::Invalid(
                    "expanded geometry exceeds import limits".into(),
                ));
            }
            let mut map = BTreeMap::new();
            for (id, v) in m.vertices() {
                map.insert(*id, flat.vertices.len());
                flat.vertices
                    .push(occurrence.world.transform_point3(v.position));
            }
            for f in m.faces().values() {
                flat.faces.push(BulkFace {
                    outer: f.outer.iter().map(|v| map[v]).collect(),
                    holes: f
                        .holes
                        .iter()
                        .map(|r| r.iter().map(|v| map[v]).collect())
                        .collect(),
                    material: f.material.or(occurrence.material),
                    color: f.color,
                });
            }
            for e in m.edges().values() {
                flat.edges.push(BulkEdge {
                    vertices: e.vertices.map(|v| map[&v]),
                    soft: e.soft,
                    smooth: e.smooth,
                    hidden: e.hidden,
                });
            }
        }
        let mut data = DocumentHistory::new().to_data();
        data.materials = source.materials().clone();
        data.tags = source.tags().clone();
        data.next_id = data.materials.len().max(data.tags.len()) as u64 + 1;
        data.definitions
            .get_mut(&DefinitionId(0))
            .ok_or_else(|| IoError::Invalid("missing root".into()))?
            .geometry = flat
            .build_cancellable(options.merge_coplanar, options.soften_coplanar, || {
                control.is_cancelled()
            })?
            .to_data();
        DocumentHistory::from_import_data(data)?
    };
    let data = history.to_data();
    let mut report = ImportReport {
        warnings: scene.warnings,
        skipped: scene.skipped,
        ..Default::default()
    };
    for d in data.definitions.values() {
        report.vertices += d.geometry.vertices.len();
        report.edges += d.geometry.edges.len();
        report.faces += d.geometry.faces.len();
        report.instances += d.instances.len();
    }
    control.check()?;
    control.set_progress(1.);
    Ok(ImportResult {
        history,
        textures: scene.textures,
        report,
    })
}
/// Dispatch a supported format through the common scene converter.
pub fn import_file(
    path: &Path,
    options: &ImportOptions,
    control: &ImportControl,
) -> Result<ImportResult> {
    control.check()?;
    let scene = match path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "gltf" | "glb" => crate::import_gltf(path, control)?,
        "dae" => crate::import_dae(path, control)?,
        "obj" => crate::import_obj_scene(path, control)?,
        "skp" => crate::import_skp(path, control)?,
        _ => return Err(IoError::Invalid("unsupported import format".into())),
    };
    if path
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("skp"))
    {
        let options = ImportOptions {
            merge_coplanar: false,
            soften_coplanar: false,
            ..options.clone()
        };
        convert_scene(scene, &options, control)
    } else {
        convert_scene(scene, options, control)
    }
}
pub(crate) fn scene_from_bundle(bundle: crate::DraftBundle) -> ImportedScene {
    let d = bundle.document;
    let indices: BTreeMap<_, _> = d
        .definitions
        .keys()
        .enumerate()
        .map(|(i, id)| (*id, i))
        .collect();
    let mats: BTreeMap<_, _> = d
        .materials
        .keys()
        .enumerate()
        .map(|(i, id)| (*id, MaterialId(i as u64 + 1)))
        .collect();
    let tags: BTreeMap<_, _> = d.tags.keys().enumerate().map(|(i, id)| (*id, i)).collect();
    let mut scene = ImportedScene {
        materials: d.materials.into_values().collect(),
        tags: d.tags.into_values().collect(),
        textures: bundle.textures,
        ..Default::default()
    };
    for def in d.definitions.into_values() {
        let map: BTreeMap<_, _> = def
            .geometry
            .vertices
            .keys()
            .enumerate()
            .map(|(i, id)| (*id, i))
            .collect();
        scene.definitions.push(ImportedDefinition {
            name: def.name,
            geometry: BulkGeometry {
                vertices: def
                    .geometry
                    .vertices
                    .into_values()
                    .map(|v| v.position)
                    .collect(),
                faces: def
                    .geometry
                    .faces
                    .into_values()
                    .map(|f| BulkFace {
                        outer: f.outer.iter().map(|v| map[v]).collect(),
                        holes: f
                            .holes
                            .iter()
                            .map(|r| r.iter().map(|v| map[v]).collect())
                            .collect(),
                        material: f.material.map(|m| mats[&m]),
                        color: f.color,
                    })
                    .collect(),
                edges: def
                    .geometry
                    .edges
                    .into_values()
                    .map(|e| BulkEdge {
                        vertices: e.vertices.map(|v| map[&v]),
                        soft: e.soft,
                        smooth: e.smooth,
                        hidden: e.hidden,
                    })
                    .collect(),
            },
            instances: def
                .instances
                .into_values()
                .map(|n| ImportedInstance {
                    name: n.name,
                    definition: indices[&n.definition],
                    transform: n.transform,
                    tag: n.tag.map(|t| tags[&t]),
                    material: n.material.and_then(|m| mats.get(&m).copied()),
                    hidden: n.hidden,
                })
                .collect(),
        });
    }
    scene.roots.push(ImportedInstance {
        definition: indices[&d.root],
        ..Default::default()
    });
    scene
}
