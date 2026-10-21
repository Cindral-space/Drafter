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
    for def in source.definitions {
        assignments.push((def.face_assignments, def.edge_tags));
        control.check()?;
        scene.definitions.push(ImportedDefinition {
            name: source_label(&def.name),
            geometry: def.geometry,
            instances: def.instances.into_iter().map(placement).collect(),
        });
    }
    scene.roots = source.roots.into_iter().map(placement).collect();
    for (index, (faces, edges)) in assignments.into_iter().enumerate() {
        partition_tagged_geometry(&mut scene, index, faces, edges, control)?;
    }
    // A valid fast path pays one bounded validation pass. On damaged geometry,
    // isolate polygon failures instead of rejecting unrelated source records.
    for index in 0..scene.definitions.len() {
        control.check()?;
        let geometry = &scene.definitions[index].geometry;
        let valid = scaled(geometry.clone())
            .build_cancellable(false, false, || control.is_cancelled())
            .is_ok();
        control.check()?;
        if !valid {
            let original = std::mem::take(&mut scene.definitions[index].geometry);
            let fixed = recover_geometry(original, &mut scene, control)?;
            scene.definitions[index].geometry = fixed;
        }
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
fn recover_geometry(
    mut source: BulkGeometry,
    scene: &mut ImportedScene,
    control: &ImportControl,
) -> Result<BulkGeometry> {
    let mut remap = vec![None; source.vertices.len()];
    let mut fixed = BulkGeometry::default();
    for (i, point) in source.vertices.drain(..).enumerate() {
        if point.is_finite() && (point * 0.0254).abs().max_element() <= cindral_core::MAX_COORDINATE
        {
            remap[i] = Some(fixed.vertices.len());
            fixed.vertices.push(point);
        } else {
            warn(scene, "Invalid/out-of-range vertex skipped");
        }
    }
    let mut work = 0usize;
    for mut face in source.faces {
        control.check()?;
        let size: usize = std::iter::once(&face.outer)
            .chain(&face.holes)
            .map(Vec::len)
            .sum();
        work = work.saturating_add(size.saturating_mul(size));
        if size > 4096 || work > 64_000_000 {
            warn(scene, "Face exceeds recovery work limit; skipped");
            continue;
        }
        let mut local = BulkGeometry::default();
        let mut good = true;
        for ring in std::iter::once(&mut face.outer).chain(&mut face.holes) {
            for vertex in ring {
                if let Some(Some(mapped)) = remap.get(*vertex) {
                    *vertex = *mapped;
                } else {
                    good = false;
                }
            }
        }
        if !good {
            warn(scene, "Face references a missing vertex; skipped");
            continue;
        }
        let mut sample = face.clone();
        for ring in std::iter::once(&mut sample.outer).chain(&mut sample.holes) {
            for vertex in ring {
                let point = fixed.vertices[*vertex];
                *vertex = local.vertices.len();
                local.vertices.push(point);
            }
        }
        local.faces.push(sample);
        if scaled(local).build(false, false).is_ok() {
            fixed.faces.push(face);
        } else {
            warn(scene, "Corrupt/nonplanar polygon skipped");
        }
    }
    for mut edge in source.edges {
        match (remap.get(edge.vertices[0]), remap.get(edge.vertices[1])) {
            (Some(Some(a)), Some(Some(b)))
                if a != b
                    && fixed.vertices[*a].distance(fixed.vertices[*b]) * 0.0254
                        > cindral_core::TOLERANCE =>
            {
                edge.vertices = [*a, *b];
                fixed.edges.push(edge);
            }
            _ => warn(scene, "Invalid edge skipped"),
        }
    }
    // All retained positions and rings have independently passed kernel validation.
    Ok(fixed)
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
