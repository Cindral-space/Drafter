//! Read-only SketchUp import into the shared polygon scene boundary.
use crate::{ImportControl, ImportedDefinition, ImportedInstance, ImportedScene, IoError, Result};
use cindral_core::BulkGeometry;
use std::{collections::BTreeMap, path::Path};

/// Read a bounded native SketchUp file; never writes or modifies the source.
pub fn import_skp(path: &Path, control: &ImportControl) -> Result<ImportedScene> {
    let bytes = crate::import_scene::read_bounded(path, cindral_skp::MAX_BYTES)?;
    import_skp_bytes(&bytes, control)
}
/// Decode container records, preserving authored faces/holes and shared placements.
pub fn import_skp_bytes(bytes: &[u8], control: &ImportControl) -> Result<ImportedScene> {
    control.check()?;
    let source = cindral_skp::read(bytes).map_err(|e| IoError::Invalid(e.to_string()))?;
    control.check()?;
    let mut scene = ImportedScene {
        meters_per_unit: 0.0254,
        materials: source.materials,
        tags: source.tags,
        warnings: source.warnings,
        skipped: source.skipped,
        ..Default::default()
    };
    scene.warnings.push(format!("Imported from .skp {} (read-only); Save As .cdraft. Coordinates converted from inches to meters. Scenes/cameras, styles, dynamic behavior, section planes, text/dimensions and source UV pins are not imported.", source.version));
    let mut texture_keys = BTreeMap::new();
    let mut decoded_texture_bytes = 0usize;
    for (source_key, bytes) in source.textures {
        control.check()?;
        if scene.textures.len() >= 127 {
            warn(&mut scene, "Texture count exceeds 127; extra image skipped");
            continue;
        }
        match crate::texture_from_bytes(bytes) {
            Ok((key, asset)) => {
                if scene.textures.contains_key(&key) {
                    texture_keys.insert(source_key, key);
                    continue;
                }
                let decoded = crate::decode_texture(&asset)?;
                let bytes = decoded.rgba.len();
                if decoded_texture_bytes.saturating_add(bytes) > 256 * 1024 * 1024 {
                    warn(
                        &mut scene,
                        "Aggregate decoded texture limit (256 MiB) reached; image skipped",
                    );
                    continue;
                }
                decoded_texture_bytes += bytes;
                texture_keys.insert(source_key, key.clone());
                scene.textures.insert(key, asset);
            }
            Err(e) => warn(
                &mut scene,
                &format!("Unsupported/corrupt texture skipped: {e}"),
            ),
        }
    }
    for tag in &mut scene.tags {
        tag.name = source_label(&tag.name);
    }
    for material in &mut scene.materials {
        material.name = source_label(&material.name);
        material.texture = material
            .texture
            .as_ref()
            .and_then(|key| texture_keys.get(key))
            .cloned();
        if !material.opacity.is_finite() {
            material.opacity = 1.;
        }
        material.opacity = material.opacity.clamp(0., 1.);
        for channel in &mut material.color {
            *channel = if channel.is_finite() {
                channel.clamp(0., 1.)
            } else {
                0.5
            };
        }
    }
    let mut assignments = Vec::new();
    let parents: Vec<Vec<String>> = (0..source.definitions.len()).map(|index| {
        source.definitions.iter().enumerate().flat_map(|(parent, d)| d.instances.iter().filter(move |i| i.definition == index).map(move |i| format!("{}[{}]/{}", d.name, parent, i.name))).collect()
    }).collect();
    for (index, mut def) in source.definitions.into_iter().enumerate() {
        let labels: Vec<_> = (0..def.geometry.faces.len()).map(|face| {
            let tag = def.face_assignments.get(&face).and_then(|v| v.0).and_then(|i| scene.tags.get(i)).map_or("Untagged", |t| t.name.as_str());
            format!("entity {}, definition {}[{}], parent instances {:?}, tag {}", def.face_ids.get(&face).map_or_else(|| format!("unavailable (face {face})"), ToString::to_string), def.name, index, parents[index], tag)
        }).collect();
        if scaled(def.geometry.clone()).build_cancellable(false, false, || control.is_cancelled()).is_err() {
            control.check()?;
            def.geometry = crate::skp_repair::recover(def.geometry, &labels, &mut def.face_assignments, &mut scene, control)?;
        }
        assignments.push((def.face_assignments, def.edge_tags));
        control.check()?;
        scene.definitions.push(ImportedDefinition {
            name: source_label(&def.name),
            geometry: def.geometry,
            instances: def.instances.into_iter().map(placement).collect(),
        });
    }
    // Both independent readers expose the source model as one synthetic identity
    // placement. Keep its identity explicit so conversion can open loose geometry
    // at root without exploding actual component placements.
    scene.root_definition = source.roots.first().map(|n| n.definition);
    scene.roots = source.roots.into_iter().map(placement).collect();
    for (index, (faces, edges)) in assignments.into_iter().enumerate() {
        partition_tagged_geometry(&mut scene, index, faces, edges, control)?;
    }
    sanitize_graph(&mut scene);
    control.set_progress(0.4);
    Ok(scene)
}
fn source_label(value: &str) -> String {
    let label: String = value
        .trim()
        .chars()
        .filter(|c| !c.is_control())
        .take(256)
        .collect();
    if label.is_empty() {
        "Imported".into()
    } else {
        label
    }
}
fn placement(source: cindral_skp::Instance) -> ImportedInstance {
    ImportedInstance {
        name: source_label(&source.name),
        definition: source.definition,
        transform: source.transform,
        tag: source.tag,
        material: source.material,
        hidden: source.hidden,
    }
}
fn warn(scene: &mut ImportedScene, message: &str) {
    scene.skipped = scene.skipped.saturating_add(1);
    if scene.warnings.len() < 256 {
        scene.warnings.push(message.chars().take(1024).collect());
    }
}
fn scaled(mut geometry: BulkGeometry) -> BulkGeometry {
    for point in &mut geometry.vertices {
        *point *= 0.0254;
    }
    geometry
}
fn sanitize_graph(scene: &mut ImportedScene) {
    let count = scene.definitions.len();
    let tags = scene.tags.len();
    let mut skipped = 0usize;
    let valid = |n: &mut ImportedInstance| {
        if n.tag.is_some_and(|tag| tag >= tags) {
            n.tag = None;
        }
        let basis = glam::DMat4::from_scale(glam::DVec3::splat(0.0254));
        n.definition < count
            && cindral_core::validate_instance_transform(basis * n.transform * basis.inverse())
                .is_ok()
    };
    scene.roots.retain_mut(|n| {
        let keep = valid(n);
        if !keep {
            skipped += 1;
        }
        keep
    });
    for def in &mut scene.definitions {
        def.instances.retain_mut(|n| {
            let keep = valid(n);
            if !keep {
                skipped += 1;
            }
            keep
        });
    }
    // Iterative DFS, one global color pass: drop cycles/deep child links, O(V+E).
    let mut color = vec![0u8; count];
    let mut height = vec![1usize; count];
    for start in 0..count {
        if color[start] != 0 {
            continue;
        }
        color[start] = 1;
        let mut stack = vec![(start, 0usize)];
        while let Some(&(node, next)) = stack.last() {
            if next == scene.definitions[node].instances.len() {
                height[node] = 1 + scene.definitions[node]
                    .instances
                    .iter()
                    .map(|n| height[n.definition])
                    .max()
                    .unwrap_or(0);
                color[node] = 2;
                stack.pop();
                continue;
            }
            let target = scene.definitions[node].instances[next].definition;
            if color[target] == 1
                || stack.len() >= 62
                || (color[target] == 2 && stack.len() + height[target] > 62)
            {
                scene.definitions[node].instances.swap_remove(next);
                skipped += 1;
                continue;
            }
            if let Some(last) = stack.last_mut() {
                last.1 += 1;
            }
            if color[target] == 0 {
                color[target] = 1;
                stack.push((target, 0));
            }
        }
    }
    if skipped > 0 {
        warn(
            scene,
            &format!("Skipped {skipped} invalid, cyclic or over-deep placements"),
        );
        scene.skipped = scene.skipped.saturating_add(skipped - 1);
    }
}

// Core tags belong to containers. Split only authored per-primitive assignments;
// ordinary untagged models retain the original geometry and the fast path.
fn partition_tagged_geometry(
    scene: &mut ImportedScene,
    owner: usize,
    faces: BTreeMap<usize, (Option<usize>, bool)>,
    edges: BTreeMap<usize, usize>,
    control: &ImportControl,
) -> Result<()> {
    let explicit = |tag: Option<usize>| {
        tag.filter(|&i| {
            scene.tags.get(i).is_some_and(|tag| {
                !tag.name.eq_ignore_ascii_case("Layer0")
                    && !tag.name.eq_ignore_ascii_case("Untagged")
            })
        })
    };
    let faces: BTreeMap<_, _> = faces
        .into_iter()
        .map(|(i, (tag, hidden))| (i, (explicit(tag), hidden)))
        .collect();
    let edges: BTreeMap<_, _> = edges
        .into_iter()
        .filter_map(|(i, tag)| explicit(Some(tag)).map(|tag| (i, tag)))
        .collect();
    if !faces.values().any(|&(tag, hidden)| tag.is_some() || hidden) && edges.is_empty() {
        return Ok(());
    }
    let source = std::mem::take(&mut scene.definitions[owner].geometry);
    #[derive(Default)]
    struct Part {
        geometry: BulkGeometry,
        indices: BTreeMap<usize, usize>,
    }
    impl Part {
        fn vertex(&mut self, old: usize, source: &BulkGeometry) -> Option<usize> {
            if let Some(&i) = self.indices.get(&old) {
                return Some(i);
            }
            let point = source.vertices.get(old).copied()?;
            let i = self.geometry.vertices.len();
            self.geometry.vertices.push(point);
            self.indices.insert(old, i);
            Some(i)
        }
    }
    let pair = |a: usize, b: usize| [a.min(b), a.max(b)];
    let styles: BTreeMap<_, _> = source
        .edges
        .iter()
        .map(|edge| (pair(edge.vertices[0], edge.vertices[1]), edge))
        .collect();
    let mut used = std::collections::BTreeSet::new();
    let mut parts: BTreeMap<(Option<usize>, bool), Part> = BTreeMap::new();
    for (index, face) in source.faces.iter().enumerate() {
        control.check()?;
        let key = faces.get(&index).copied().unwrap_or_default();
        let part = parts.entry(key).or_default();
        let mut face = face.clone();
        let old = face.clone();
        let mut good = true;
        for ring in std::iter::once(&mut face.outer).chain(&mut face.holes) {
            for vertex in ring {
                if let Some(i) = part.vertex(*vertex, &source) {
                    *vertex = i;
                } else {
                    good = false;
                }
            }
        }
        if !good {
            warn(scene, "Tagged face references missing vertices; skipped");
            continue;
        }
        part.geometry.faces.push(face);
        for ring in std::iter::once(&old.outer).chain(&old.holes) {
            for (&a, &b) in ring
                .iter()
                .zip(ring.iter().cycle().skip(1))
                .take(ring.len())
            {
                let key = pair(a, b);
                used.insert(key);
                if let Some(style) = styles.get(&key) {
                    let a = part.indices[&a];
                    let b = part.indices[&b];
                    part.geometry.edges.push(cindral_core::BulkEdge {
                        vertices: [a, b],
                        ..(*style).clone()
                    });
                }
            }
        }
    }
    for (index, edge) in source.edges.iter().enumerate() {
        let [a, b] = edge.vertices;
        if used.contains(&pair(a, b)) {
            continue;
        }
        let part = parts
            .entry((edges.get(&index).copied(), false))
            .or_default();
        if let (Some(a), Some(b)) = (part.vertex(a, &source), part.vertex(b, &source)) {
            part.geometry.edges.push(cindral_core::BulkEdge {
                vertices: [a, b],
                ..edge.clone()
            });
        } else {
            warn(scene, "Tagged edge references missing vertices; skipped");
        }
    }
    for ((tag, hidden), part) in parts {
        if scene.definitions.len() >= 100_000 {
            warn(
                scene,
                "Tagged geometry exceeds definition limit; part skipped",
            );
            continue;
        }
        let definition = scene.definitions.len();
        let name = tag.map_or_else(|| "Geometry".into(), |i| scene.tags[i].name.clone());
        scene.definitions.push(ImportedDefinition {
            name: name.clone(),
            geometry: part.geometry,
            ..Default::default()
        });
        scene.definitions[owner].instances.push(ImportedInstance {
            name,
            definition,
            tag,
            hidden,
            ..Default::default()
        });
    }
    Ok(())
}

#[cfg(test)]
mod diagnostics {
    use super::*;
    #[test]
    fn private_face_diagnosis() {
        let path = Path::new("/home/earl/Desktop/Cindral/Drafter/SKP/mybedroom.skp");
        if !path.exists() { eprintln!("SKIP: private model absent"); return; }
        let source = cindral_skp::read(&std::fs::read(path).unwrap()).unwrap();
        eprintln!("SOURCE defs={} roots={} vertices={} edges={} faces={} placements={}", source.definitions.len(), source.roots.len(), source.definitions.iter().map(|d| d.geometry.vertices.len()).sum::<usize>(), source.definitions.iter().map(|d| d.geometry.edges.len()).sum::<usize>(), source.definitions.iter().map(|d| d.geometry.faces.len()).sum::<usize>(), source.definitions.iter().map(|d| d.instances.len()).sum::<usize>());
        for (di, def) in source.definitions.iter().enumerate() {
            for (fi, face) in def.geometry.faces.iter().enumerate() {
                let local = BulkGeometry { vertices: def.geometry.vertices.clone(), faces: vec![face.clone()], edges: vec![] };
                if let Err(error) = scaled(local).build(false, false) {
                    let points: Vec<_> = std::iter::once(&face.outer).chain(&face.holes).flatten().map(|&v| def.geometry.vertices[v]*0.0254).collect();
                    let (center, normal, deviation, size) = crate::skp_plane::fit(&points);
                    for a in 0..face.outer.len() { for b in a+1..face.outer.len() { let distance = (def.geometry.vertices[face.outer[a]]-def.geometry.vertices[face.outer[b]]).length()*0.0254; if distance <= cindral_core::TOLERANCE { eprintln!("WELD ring {a}/{b} indices {}/{} distance={distance:e}", face.outer[a], face.outer[b]); } }}
                    let area = |ring: &[usize]| { let p = def.geometry.vertices[ring[0]]*0.0254; ring[1..].windows(2).map(|w| (def.geometry.vertices[w[0]]*0.0254-p).cross(def.geometry.vertices[w[1]]*0.0254-p).dot(normal)*0.5).sum::<f64>().abs() };
                    eprintln!("FACE id={:?} def={di} name={:?} tag={:?} index={fi} center={center:?} area={} vertices={} holes={} deviation={deviation:e} fraction={:e} rejection={error}", def.face_ids.get(&fi), def.name, def.face_assignments.get(&fi), area(&face.outer)-face.holes.iter().map(|h| area(h)).sum::<f64>(), points.len(), face.holes.len(), deviation/size);
                    for (parent, d) in source.definitions.iter().enumerate() { for i in &d.instances { if i.definition==di {eprintln!("PARENT {parent} {:?} {:?}", i.name, i.transform);}}}
                }
            }
        }
        let imported = crate::import_file(path, &crate::ImportOptions::default(), &ImportControl::default()).unwrap();
        eprintln!("SUMMARY {:?}", imported.report);
    }
}
