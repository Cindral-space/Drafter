//! Explicit surface edits prepared in bounded local transaction patches.
use crate::{Command, DVec3, EdgeId, FaceId, KernelError, Model, Plane, VertexId, TOLERANCE};
use std::collections::{BTreeMap, BTreeSet};
fn invalid(message: &str) -> KernelError {
    KernelError::InvalidTopology(message.into())
}

#[tracing::instrument(level = "info", skip_all, name = "modification.prepare")]
pub(crate) fn prepare(
    model: &Model,
    command: &Command,
    high_water: u64,
) -> Result<(Model, Model), KernelError> {
    let spatial = model.spatial.as_ref().ok_or(KernelError::MissingEntity)?;
    let mut faces = BTreeSet::new();
    let mut edges = BTreeSet::new();
    let mut expand = false;
    match command {
        Command::PaintFace { faces: ids, .. } | Command::SetFaceMaterial { faces: ids, .. } => {
            faces.extend(ids);
        }
        Command::PushPull { face, distance } => {
            let f = model.faces.get(face).ok_or(KernelError::MissingEntity)?;
            if !distance.is_finite() {
                return Err(KernelError::InvalidPosition);
            }
            let points = f
                .outer
                .iter()
                .map(|id| model.position(*id))
                .collect::<Result<Vec<_>, _>>()?;
            let points = points
                .iter()
                .copied()
                .chain(points.iter().map(|p| *p + f.plane.normal * *distance));
            if let Some(bounds) = crate::spatial::envelope(points) {
                faces.extend(spatial.query_faces(bounds));
                edges.extend(spatial.query_edges(bounds));
            }
            faces.insert(*face);
            expand = true;
        }
        Command::FollowMe { face, path } => {
            let f = model.faces.get(face).ok_or(KernelError::MissingEntity)?;
            faces.insert(*face);
            expand = true;
            let origin = path.first().copied().ok_or(KernelError::InvalidPolyline)?;
            let points = path.iter().flat_map(|p| {
                f.outer
                    .iter()
                    .map(move |id| model.vertices[id].position + *p - origin)
            });
            if let Some(bounds) = crate::spatial::envelope(points) {
                faces.extend(spatial.query_faces(bounds));
                edges.extend(spatial.query_edges(bounds));
            }
        }
        Command::Transform {
            vertices, matrix, ..
        } => {
            validate_matrix(*matrix)?;
            let mut points = Vec::new();
            for id in vertices {
                let p = model.position(*id)?;
                points.extend([p, matrix.transform_point3(p)]);
                if let Some(ids) = spatial.adjacency.get(id) {
                    edges.extend(ids);
                }
            }
            if let Some(bounds) = crate::spatial::envelope(points) {
                faces.extend(spatial.query_faces(bounds));
                edges.extend(spatial.query_edges(bounds));
            }
        }
        _ => return Err(invalid("not an explicit surface edit")),
    }
    for id in &edges {
        if let Some(ids) = spatial.edge_faces.get(id) {
            faces.extend(ids);
        }
    }
    let traversal_span = tracing::info_span!("prepare.connected_traversal").entered();
    let mut queue: Vec<_> = faces.iter().copied().collect();
    let mut visited = BTreeSet::new();
    while let Some(id) = queue.pop() {
        if !visited.insert(id) {
            continue;
        }
        model.faces.get(&id).ok_or(KernelError::MissingEntity)?;
        if let Some(boundary) = spatial.face_edges.get(&id) {
            edges.extend(boundary);
            if expand {
                for e in boundary {
                    if let Some(ids) = spatial.edge_faces.get(e) {
                        for other in ids {
                            if faces.insert(*other) {
                                queue.push(*other);
                            }
                        }
                    }
                }
            }
        }
    }
    drop(traversal_span);
    let copy_span = tracing::info_span!(
        "prepare.patch_clone",
        faces = faces.len(),
        edges = edges.len()
    )
    .entered();
    let mut before = Model {
        next_id: model.next_id,
        ..Model::default()
    };
    for id in edges {
        let e = model
            .edges
            .get(&id)
            .ok_or(KernelError::MissingEntity)?
            .clone();
        for v in e.vertices {
            before.vertices.insert(v, model.vertices[&v].clone());
        }
        before.edges.insert(id, e);
    }
    for id in faces {
        let f = model
            .faces
            .get(&id)
            .ok_or(KernelError::MissingEntity)?
            .clone();
        for v in f.outer.iter().chain(f.holes.iter().flatten()) {
            before.vertices.insert(*v, model.vertices[v].clone());
        }
        before.faces.insert(id, f);
    }
    let mut after = before.clone();
    after.next_id = high_water;
    drop(copy_span);
    match command {
        Command::Transform {
            vertices,
            matrix,
            copy,
        } => transform(&mut after, vertices, *matrix, *copy)?,
        Command::SetFaceMaterial { faces, material } => {
            for id in faces {
                after
                    .faces
                    .get_mut(id)
                    .ok_or(KernelError::MissingEntity)?
                    .material = *material;
            }
        }
        Command::PaintFace { faces, color } => {
            for id in faces {
                let face = after.faces.get_mut(id).ok_or(KernelError::MissingEntity)?;
                face.color = *color;
                face.material = None;
            }
        }
        Command::PushPull { face, distance } => push_pull(&mut after, *face, *distance)?,
        Command::FollowMe { face, path } => follow_me(&mut after, *face, path)?,
        _ => return Err(invalid("unsupported explicit surface edit")),
    }
    after.validate()?;
    Ok((before, after))
}
fn validate_matrix(m: glam::DMat4) -> Result<(), KernelError> {
    if !m.is_finite()
        || m.x_axis.w.abs() > TOLERANCE
        || m.y_axis.w.abs() > TOLERANCE
        || m.z_axis.w.abs() > TOLERANCE
        || (m.w_axis.w - 1.).abs() > TOLERANCE
        || !m.determinant().is_finite()
        || m.determinant().abs() <= TOLERANCE * TOLERANCE
    {
        return Err(invalid("transform must be finite, affine and nonsingular"));
    }
    Ok(())
}
#[tracing::instrument(level = "info", skip_all, name = "modification.transform")]
pub(crate) fn transform(
    m: &mut Model,
    ids: &[VertexId],
    matrix: glam::DMat4,
    copy: bool,
) -> Result<(), KernelError> {
    validate_matrix(matrix)?;
    let selected: BTreeSet<_> = ids.iter().copied().collect();
    let mut mapping = BTreeMap::new();
    for id in &selected {
        let p = matrix.transform_point3(m.position(*id)?);
        crate::geometry::validate_position(p)?;
        let target = if copy {
            m.intern(p)?
        } else {
            m.vertices
                .get_mut(id)
                .ok_or(KernelError::MissingEntity)?
                .position = p;
            *id
        };
        mapping.insert(*id, target);
    }
    if copy {
        let edges: Vec<_> = m
            .edges
            .values()
            .filter(|e| e.vertices.iter().all(|v| selected.contains(v)))
            .cloned()
            .collect();
        let faces: Vec<_> = m
            .faces
            .values()
            .filter(|f| {
                f.outer
                    .iter()
                    .chain(f.holes.iter().flatten())
                    .all(|v| selected.contains(v))
            })
            .cloned()
            .collect();
        for mut e in edges {
            for v in &mut e.vertices {
                *v = mapping[v];
            }
            e.vertices.sort();
            if !m.edges.values().any(|old| old.vertices == e.vertices) {
                let id = EdgeId::from_raw(m.allocate()?);
                m.edges.insert(id, e);
            }
        }
        for mut f in faces {
            for v in f.outer.iter_mut().chain(f.holes.iter_mut().flatten()) {
                *v = mapping[v];
            }
            update_plane(m, &mut f, matrix.determinant() < 0.)?;
            if !m
                .faces
                .values()
                .any(|old| crate::topology::face_key(old) == crate::topology::face_key(&f))
            {
                let id = FaceId::from_raw(m.allocate()?);
                m.faces.insert(id, f);
            }
        }
    } else {
        let mut faces = std::mem::take(&mut m.faces);
        for f in faces.values_mut() {
            let touched = f
                .outer
                .iter()
                .chain(f.holes.iter().flatten())
                .any(|v| selected.contains(v));
            if touched {
                update_plane(
                    m,
                    f,
                    matrix.determinant() < 0.
                        && f.outer
                            .iter()
                            .chain(f.holes.iter().flatten())
                            .all(|v| selected.contains(v)),
                )?;
            }
        }
        m.faces = faces;
    }
    Ok(())
}
fn update_plane(m: &Model, f: &mut crate::Face, _reflected: bool) -> Result<(), KernelError> {
    let points = f
        .outer
        .iter()
        .map(|id| m.position(*id))
        .collect::<Result<Vec<_>, _>>()?;
    let a = *points.first().ok_or(KernelError::Triangulation)?;
    let plane = points
        .windows(2)
        .find_map(|p| Plane::through(a, p[0] - a, p[1] - a))
        .ok_or(KernelError::Triangulation)?;
    if f.outer
        .iter()
        .chain(f.holes.iter().flatten())
        .any(|id| plane.signed_distance(m.vertices[id].position).abs() > TOLERANCE)
    {
        return Err(invalid("transform would make an adjacent face nonplanar"));
    }
    let (u, v) = plane.basis();
    let area = crate::geometry::signed_area(
        &points
            .iter()
            .map(|p| glam::DVec2::new((*p - a).dot(u), (*p - a).dot(v)))
            .collect::<Vec<_>>(),
    );
    if area < 0. {
        f.outer.reverse();
        for h in &mut f.holes {
            h.reverse();
        }
    }
    f.plane = plane;
    Ok(())
}
fn edge_usage(m: &Model) -> BTreeMap<[VertexId; 2], Vec<(FaceId, bool)>> {
    let mut usage: BTreeMap<[VertexId; 2], Vec<(FaceId, bool)>> = BTreeMap::new();
    for (id, f) in &m.faces {
        for ring in std::iter::once(&f.outer).chain(&f.holes) {
            for (a, b) in ring.iter().zip(ring.iter().cycle().skip(1)) {
                let mut pair = [*a, *b];
                pair.sort();
                usage.entry(pair).or_default().push((*id, *a < *b));
            }
        }
    }
    usage
}
type OrientedBoundary = (Vec<crate::csg::Polygon>, bool, BTreeMap<FaceId, bool>);
#[tracing::instrument(level = "info", skip_all, name = "modification.oriented")]
fn oriented(m: &Model) -> Result<OrientedBoundary, KernelError> {
    let usage = edge_usage(m);
    let closed = !m.faces.is_empty() && usage.values().all(|f| f.len() == 2);
    let mut flip = BTreeMap::new();
    for root in m.faces.keys() {
        if flip.contains_key(root) {
            continue;
        }
        flip.insert(*root, false);
        let mut queue = vec![*root];
        let mut component = Vec::new();
        while let Some(id) = queue.pop() {
            component.push(id);
            for incident in usage.values() {
                if let Some((_, direction)) = incident.iter().find(|(f, _)| *f == id) {
                    for (other, other_direction) in incident {
                        if *other == id {
                            continue;
                        }
                        let desired = flip[&id] ^ (*direction == *other_direction);
                        if let Some(existing) = flip.get(other) {
                            if *existing != desired && closed {
                                return Err(invalid("non-orientable boundary"));
                            }
                        } else {
                            flip.insert(*other, desired);
                            queue.push(*other);
                        }
                    }
                }
            }
        }
        let mut volume = 0.;
        let origin = m.position(m.faces[root].outer[0])?;
        for id in &component {
            for [a, b, c] in m.triangulate(*id)? {
                volume += (a - origin).dot((b - origin).cross(c - origin)) / 6.
                    * if flip[id] { -1. } else { 1. };
            }
        }
        if volume < 0. {
            for id in component {
                if let Some(f) = flip.get_mut(&id) {
                    *f = !*f;
                }
            }
        }
    }
    let mut polygons = Vec::new();
    for id in m.faces.keys() {
        for [a, mut b, mut c] in m.triangulate(*id)? {
            if flip[id] {
                std::mem::swap(&mut b, &mut c);
            }
            if let Some(p) = crate::csg::Polygon::new(vec![a, b, c]) {
                polygons.push(p);
            }
        }
    }
    Ok((polygons, closed, flip))
}
fn prism(m: &Model, face: FaceId, delta: DVec3) -> Result<Vec<crate::csg::Polygon>, KernelError> {
    let f = m.faces.get(&face).ok_or(KernelError::MissingEntity)?;
    let forward = delta.dot(f.plane.normal) > 0.;
    let mut polygons = Vec::new();
    let mut add = |points: Vec<DVec3>| {
        if let Some(p) = crate::csg::Polygon::new(points) {
            polygons.push(p);
        }
    };
    for [a, b, c] in m.triangulate(face)? {
        if forward {
            add(vec![c, b, a]);
            add(vec![a + delta, b + delta, c + delta]);
        } else {
            add(vec![a, b, c]);
            add(vec![c + delta, b + delta, a + delta]);
        }
    }
    for ring in std::iter::once(&f.outer).chain(&f.holes) {
        for (a, b) in ring.iter().zip(ring.iter().cycle().skip(1)) {
            let a = m.position(*a)?;
            let b = m.position(*b)?;
            let mut quad = vec![a, b, b + delta, a + delta];
            if !forward {
                quad.reverse();
            }
            add(quad);
        }
    }
    Ok(polygons)
}
fn polygon_normal(p: &crate::csg::Polygon) -> Option<DVec3> {
    let a = *p.vertices.first()?;
    p.vertices
        .windows(2)
        .find_map(|w| (w[0] - a).cross(w[1] - a).try_normalize())
}
fn polygon_contains(p: &crate::csg::Polygon, point: DVec3) -> bool {
    let Some(n) = polygon_normal(p) else {
        return false;
    };
    if n.dot(point - p.vertices[0]).abs() > TOLERANCE {
        return false;
    }
    p.vertices
        .iter()
        .zip(p.vertices.iter().cycle().skip(1))
        .all(|(a, b)| (*b - *a).cross(point - *a).dot(n) >= -TOLERANCE * (*b - *a).length())
}
#[tracing::instrument(level = "info", skip_all, name = "modification.surfaces")]
fn surfaces(
    m: &mut Model,
    polygons: Vec<crate::csg::Polygon>,
    color: [u8; 4],
    material: Option<crate::MaterialId>,
) -> Result<(), KernelError> {
    let next = m.next_id;
    let old = m.clone();
    let mut result = Model {
        next_id: next,
        vertices: old.vertices.clone(),
        ..Model::default()
    };
    let insertion_span =
        tracing::info_span!("surfaces.insert_segments", polygons = polygons.len()).entered();
    for p in &polygons {
        for (a, b) in p.vertices.iter().zip(p.vertices.iter().cycle().skip(1)) {
            if a.distance(*b) > TOLERANCE {
                result.insert_segment(*a, *b)?;
            }
        }
    }
    drop(insertion_span);
    let seam_span = tracing::info_span!("surfaces.seam_scan", edges = result.edges.len()).entered();
    // Split edges first, then remove boundaries shared by coplanar convex pieces.
    let mut seams = Vec::new();
    for (id, e) in &result.edges {
        let a = result.position(e.vertices[0])?;
        let b = result.position(e.vertices[1])?;
        let midpoint = (a + b) * 0.5;
        let mut normals = Vec::new();
        for p in &polygons {
            if p.vertices
                .iter()
                .zip(p.vertices.iter().cycle().skip(1))
                .any(|(c, d)| crate::geometry::on_segment(midpoint, *c, *d))
            {
                if let Some(n) = polygon_normal(p) {
                    normals.push(n);
                }
            }
        }
        if normals.len() >= 2
            && normals
                .iter()
                .all(|n| n.cross(normals[0]).length() <= TOLERANCE)
        {
            seams.push(*id);
        }
    }
    for id in seams {
        result.edges.remove(&id);
    }
    drop(seam_span);
    result.heal_scope = None;
    let heal_span = tracing::info_span!("surfaces.heal").entered();
    result.heal()?;
    drop(heal_span);
    let rebuild_span = tracing::info_span!("surfaces.rebuild_faces").entered();
    result.rebuild_faces()?;
    drop(rebuild_span);
    let mut remove = Vec::new();
    for id in result.faces.keys() {
        let triangles = result.triangulate(*id)?;
        if triangles.iter().any(|[a, b, c]| {
            !polygons
                .iter()
                .any(|p| polygon_contains(p, (*a + *b + *c) / 3.))
        }) {
            remove.push(*id);
        }
    }
    for id in remove {
        result.faces.remove(&id);
    }
    let usage = edge_usage(&result);
    result.edges.retain(|_, e| usage.contains_key(&e.vertices));
    let used: BTreeSet<_> = result.edges.values().flat_map(|e| e.vertices).collect();
    result.vertices.retain(|id, _| used.contains(id));
    for f in result.faces.values_mut() {
        f.color = color;
        f.material = material;
        // Preserve colors of surviving old surfaces by an interior sample.
        let sample = f
            .outer
            .iter()
            .map(|v| result.vertices[v].position)
            .sum::<DVec3>()
            / f.outer.len() as f64;
        if let Some(previous) = old.faces.values().find(|oldf| {
            oldf.plane.equivalent(&f.plane)
                && crate::transaction::contains_face(&old, oldf, sample).unwrap_or(false)
        }) {
            f.color = previous.color;
            f.material = previous.material;
        }
    }
    // Keep intentional loose drawing/path edges that were only spatial context.
    let old_usage = edge_usage(&old);
    for e in old
        .edges
        .values()
        .filter(|e| !old_usage.contains_key(&e.vertices))
    {
        result.insert_segment(old.position(e.vertices[0])?, old.position(e.vertices[1])?)?;
    }
    // Wire endpoints may split a retained surface edge; propagate those splits into its rings.
    let mut faces = std::mem::take(&mut result.faces);
    for face in faces.values_mut() {
        for ring in std::iter::once(&mut face.outer).chain(face.holes.iter_mut()) {
            let mut expanded = Vec::new();
            for (a, b) in ring.iter().zip(ring.iter().cycle().skip(1)) {
                let pa = result.position(*a)?;
                let pb = result.position(*b)?;
                let mut points: Vec<_> = result
                    .vertices
                    .iter()
                    .filter(|(_, v)| crate::geometry::on_segment(v.position, pa, pb))
                    .map(|(id, v)| (*id, crate::geometry::parameter(v.position, pa, pb)))
                    .collect();
                points.sort_by(|a, b| a.1.total_cmp(&b.1));
                expanded.extend(
                    points
                        .into_iter()
                        .filter(|(id, _)| id != b)
                        .map(|(id, _)| id),
                );
            }
            *ring = expanded;
        }
    }
    result.faces = faces;
    // Keep stable IDs/attributes for unchanged boundary entities so the renderer
    // never uploads an untouched surface merely because it shared a CSG shell.
    let old_edges: BTreeMap<_, _> = old
        .edges
        .iter()
        .map(|(id, e)| (e.vertices, (*id, e)))
        .collect();
    result.edges = std::mem::take(&mut result.edges)
        .into_iter()
        .map(|(id, e)| {
            if let Some((old_id, old_edge)) = old_edges.get(&e.vertices) {
                (*old_id, (*old_edge).clone())
            } else {
                (id, e)
            }
        })
        .collect();
    let old_faces: BTreeMap<_, _> = old
        .faces
        .iter()
        .map(|(id, f)| (crate::topology::face_key(f), (*id, f)))
        .collect();
    result.faces = std::mem::take(&mut result.faces)
        .into_iter()
        .map(|(id, f)| {
            if let Some((old_id, old_face)) = old_faces.get(&crate::topology::face_key(&f)) {
                (*old_id, (*old_face).clone())
            } else {
                (id, f)
            }
        })
        .collect();
    *m = result;
    Ok(())
}
#[tracing::instrument(level = "info", skip_all, name = "modification.push_pull")]
fn push_pull(m: &mut Model, face: FaceId, distance: f64) -> Result<(), KernelError> {
    if !distance.is_finite() {
        return Err(KernelError::InvalidPosition);
    }
    if distance.abs() <= TOLERANCE {
        return Ok(());
    }
    let f = m
        .faces
        .get(&face)
        .ok_or(KernelError::MissingEntity)?
        .clone();
    let delta = f.plane.normal * distance;
    for v in f.outer.iter().chain(f.holes.iter().flatten()) {
        crate::geometry::validate_position(m.position(*v)? + delta)?;
    }
    let (original, closed, flip) = oriented(m)?;
    let tool = prism(m, face, delta)?;
    let polygons = if closed {
        let outward = f.plane.normal * if flip[&face] { -1. } else { 1. };
        if delta.dot(outward) > 0. {
            crate::csg::union(original, tool)?
        } else {
            crate::csg::subtract(original, tool)?
        }
    } else {
        let mut polygons = tool;
        for id in m.faces.keys().filter(|id| **id != face) {
            for t in m.triangulate(*id)? {
                if let Some(p) = crate::csg::Polygon::new(t.to_vec()) {
                    polygons.push(p);
                }
            }
        }
        polygons
    };
    surfaces(m, polygons, f.color, f.material)?;
    if closed && !m.faces.is_empty() && edge_usage(m).values().any(|f| f.len() != 2) {
        return Err(invalid("extrusion would open a closed solid"));
    }
    Ok(())
}
fn follow_me(m: &mut Model, face: FaceId, path: &[DVec3]) -> Result<(), KernelError> {
    if path.len() < 2 {
        return Err(KernelError::InvalidPolyline);
    }
    for p in path {
        crate::geometry::validate_position(*p)?;
    }
    let source = m
        .faces
        .get(&face)
        .ok_or(KernelError::MissingEntity)?
        .clone();
    let origin = path[0];
    let mut rings: Vec<Vec<DVec3>> = std::iter::once(&source.outer)
        .chain(&source.holes)
        .map(|ring| ring.iter().map(|v| m.position(*v)).collect())
        .collect::<Result<_, _>>()?;
    let tangent = (path[1] - origin)
        .try_normalize()
        .ok_or(KernelError::DegenerateEdge)?;
    if tangent.cross(source.plane.normal).length() > TOLERANCE {
        return Err(invalid("sweep must start normal to the profile"));
    }
    let mut polygons = Vec::new();
    // Parallel transport frames around corners; miter at the path stations.
    let mut stations = vec![rings.clone()];
    let mut previous = tangent;
    for i in 1..path.len() {
        let direction = if i + 1 < path.len() {
            (path[i + 1] - path[i])
                .try_normalize()
                .ok_or(KernelError::DegenerateEdge)?
        } else {
            previous
        };
        let rotation = glam::DQuat::from_rotation_arc(previous, direction);
        for ring in &mut rings {
            for p in ring {
                *p = path[i] + rotation * (*p - path[i - 1]);
            }
        }
        stations.push(rings.clone());
        previous = direction;
    }
    for station in stations.windows(2) {
        for (a, b) in station[0].iter().zip(&station[1]) {
            for i in 0..a.len() {
                let j = (i + 1) % a.len();
                let quad = [a[i], a[j], b[j], b[i]];
                for tri in [[quad[0], quad[1], quad[2]], [quad[0], quad[2], quad[3]]] {
                    if let Some(p) = crate::csg::Polygon::new(tri.to_vec()) {
                        polygons.push(p);
                    }
                }
            }
        }
    }
    for t in m.triangulate(face)? {
        if let Some(p) = crate::csg::Polygon::new(vec![t[2], t[1], t[0]]) {
            polygons.push(p);
        }
    }
    let last = stations.last().ok_or(KernelError::InvalidPolyline)?;
    let mut end = m.clone();
    let mut f = source.clone();
    for (ids, points) in std::iter::once(&mut f.outer)
        .chain(f.holes.iter_mut())
        .zip(last)
    {
        for (id, p) in ids.iter().zip(points) {
            end.vertices
                .get_mut(id)
                .ok_or(KernelError::MissingEntity)?
                .position = *p;
        }
    }
    update_plane(&end, &mut f, false)?;
    end.faces.insert(face, f);
    for t in end.triangulate(face)? {
        if let Some(p) = crate::csg::Polygon::new(t.to_vec()) {
            polygons.push(p);
        }
    }
    for id in m.faces.keys().filter(|id| **id != face) {
        for t in m.triangulate(*id)? {
            if let Some(p) = crate::csg::Polygon::new(t.to_vec()) {
                polygons.push(p);
            }
        }
    }
    surfaces(m, polygons, source.color, source.material)
}

pub(crate) fn offset_command(
    model: &Model,
    face: FaceId,
    distance: f64,
) -> Result<Command, KernelError> {
    if !distance.is_finite() {
        return Err(KernelError::InvalidPosition);
    }
    let f = model.faces.get(&face).ok_or(KernelError::MissingEntity)?;
    let (u, v) = f.plane.basis();
    let origin = model.position(f.outer[0])?;
    let points = model.project_at(&f.outer, f.plane, origin)?;
    let mut result = Vec::new();
    for i in 0..points.len() {
        let a = points[(i + points.len() - 1) % points.len()];
        let b = points[i];
        let c = points[(i + 1) % points.len()];
        let ab = (b - a).try_normalize().ok_or(KernelError::DegenerateEdge)?;
        let bc = (c - b).try_normalize().ok_or(KernelError::DegenerateEdge)?;
        let n1 = glam::DVec2::new(-ab.y, ab.x);
        let n2 = glam::DVec2::new(-bc.y, bc.x);
        let denom = 1. + n1.dot(n2);
        if denom.abs() <= TOLERANCE {
            return Err(invalid("offset has a reversing corner"));
        }
        let p = b + (n1 + n2) * (distance / denom);
        result.push(origin + u * p.x + v * p.y);
    }
    // Reject collapsed/reversed offsets and self intersections rather than commit invalid loops.
    let projected: Vec<_> = result
        .iter()
        .map(|p| glam::DVec2::new((*p - origin).dot(u), (*p - origin).dot(v)))
        .collect();
    if crate::geometry::signed_area(&projected) <= TOLERANCE * TOLERANCE {
        return Err(invalid("offset collapses the face"));
    }
    if distance > 0.
        && projected
            .iter()
            .any(|p| !crate::geometry::contains(&points, *p))
    {
        return Err(invalid("offset exceeds the face boundary"));
    }
    Ok(Command::AddPolyline {
        points: result,
        closed: true,
    })
}

impl Model {
    /// Volume of an orientable closed face boundary, or `None` for open sheets/wires.
    /// Independent shells contribute their absolute volumes; nested shells are not solid groups yet.
    pub fn closed_solid_volume(&self) -> Result<Option<f64>, KernelError> {
        let (polygons, closed, _) = oriented(self)?;
        let usage = edge_usage(self);
        if !closed
            || self
                .edges
                .values()
                .any(|e| !usage.contains_key(&e.vertices))
        {
            return Ok(None);
        }
        let volume = crate::csg::total_signed_volume(&polygons);
        Ok(Some(volume.abs()))
    }
}
