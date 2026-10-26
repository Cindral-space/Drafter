//! Proven exterior extrusion of a supporting boundary face, preserving untouched topology.
use crate::{
    Command, DVec3, Edge, EdgeId, Face, FaceId, KernelError, Model, Plane, Vertex, VertexId,
    TOLERANCE,
};
use std::collections::{BTreeMap, BTreeSet};
/// Return no candidate unless a closed boundary or disjoint planar-sheet proof applies.
#[tracing::instrument(level = "info", skip_all, name = "extrusion.local_proof")]
pub(crate) fn prepare(
    model: &Model,
    command: &Command,
    high_water: u64,
) -> Result<Option<(Model, Model)>, KernelError> {
    let Command::PushPull { face, distance } = command else {
        return Ok(None);
    };
    if !distance.is_finite() {
        return Err(KernelError::InvalidPosition);
    }
    if distance.abs() <= TOLERANCE {
        return Ok(None);
    }
    let source = model.faces.get(face).ok_or(KernelError::MissingEntity)?;
    let index = model.spatial.as_ref().ok_or(KernelError::MissingEntity)?;
    let boundary = index
        .face_edges
        .get(face)
        .ok_or(KernelError::MissingEntity)?;
    let attached = boundary
        .iter()
        .any(|edge| index.edge_faces[edge].iter().any(|other| other != face));
    if !attached && !planar_sheet(model, *face)? {
        return Ok(None);
    }
    let closed = index
        .edge_faces
        .values()
        .filter(|v| !v.is_empty())
        .all(|v| v.len() == 2);
    let normal = source.plane.normal * distance.signum();
    let origin = model.position(source.outer[0])?;
    let delta = source.plane.normal * *distance;

    if closed && distance < &0.0 {
        return Ok(None);
    }
    if !source.holes.is_empty() && closed {
        return Ok(None);
    }

    if !source.holes.is_empty() {
        let moves_into_adjacent = boundary.iter().any(|edge| {
            index.edge_faces[edge].iter().any(|other| {
                *other != *face
                    && !model.faces[other].plane.equivalent(&source.plane)
                    && model.faces[other]
                        .outer
                        .iter()
                        .chain(model.faces[other].holes.iter().flatten())
                        .any(|v| (model.vertices[v].position - origin).dot(delta) > TOLERANCE)
            })
        });
        if moves_into_adjacent {
            return Ok(None);
        }
    }

    for v in source.outer.iter().chain(source.holes.iter().flatten()) {
        let pv = model.vertices[v].position;
        for &e in &index.adjacency[v] {
            let ov = if model.edges[&e].vertices[0] == *v {
                model.edges[&e].vertices[1]
            } else {
                model.edges[&e].vertices[0]
            };
            let dir = model.vertices[&ov].position - pv;
            if dir.dot(delta) > TOLERANCE
                && dir.cross(delta).length_squared()
                    <= TOLERANCE * TOLERANCE * dir.length_squared() * delta.length_squared()
            {
                return Ok(None);
            }
        }
    }

    let walls: BTreeSet<_> = if source.holes.is_empty() {
        boundary
            .iter()
            .flat_map(|edge| &index.edge_faces[edge])
            .filter(|other| {
                **other != *face
                    && !model.faces[other].plane.equivalent(&source.plane)
                    && model.faces[other].plane.normal.dot(normal).abs() <= TOLERANCE
                    && model.faces[other].color == source.color
                    && model.faces[other].material == source.material
            })
            .copied()
            .collect()
    } else {
        BTreeSet::new()
    };
    let points = source
        .outer
        .iter()
        .chain(source.holes.iter().flatten())
        .map(|id| model.position(*id))
        .collect::<Result<Vec<_>, _>>()?;
    let bounds = crate::spatial::envelope(
        points
            .iter()
            .copied()
            .chain(points.iter().map(|p| *p + delta)),
    )
    .ok_or(KernelError::MissingEntity)?;

    // Fall back to full check if the swept prism penetrates non-incident geometry.
    {
        let slab_tol = TOLERANCE;
        let sweep_min = points
            .iter()
            .copied()
            .chain(points.iter().map(|p| *p + delta))
            .fold(DVec3::splat(f64::INFINITY), |a, b| a.min(b));
        let sweep_max = points
            .iter()
            .copied()
            .chain(points.iter().map(|p| *p + delta))
            .fold(DVec3::splat(f64::NEG_INFINITY), |a, b| a.max(b));

        let (u, v) = source.plane.basis();
        let project2d = |p: DVec3| glam::DVec2::new((p - origin).dot(u), (p - origin).dot(v));
        let outer_2d = model.project_at(&source.outer, source.plane, origin)?;
        let holes_2d = source
            .holes
            .iter()
            .map(|h| model.project_at(h, source.plane, origin))
            .collect::<Result<Vec<_>, _>>()?;

        for other in index.query_faces(bounds) {
            if other == *face || walls.contains(&other) {
                continue;
            }
            let of = &model.faces[&other];
            if of.plane.equivalent(&source.plane) {
                let of_triangles = model.triangulate(other)?;
                let source_triangles = model.triangulate(*face)?;
                let (u, v) = source.plane.basis();
                let project = |t: [DVec3; 3]| {
                    t.map(|p| glam::DVec2::new((p - origin).dot(u), (p - origin).dot(v)))
                };
                let a_tris: Vec<_> = source_triangles.into_iter().map(project).collect();
                for b_tri in of_triangles.into_iter().map(project) {
                    for a in &a_tris {
                        if triangles_overlap(*a, b_tri) {
                            return Ok(None);
                        }
                    }
                }
                continue;
            }
            let of_points = of
                .outer
                .iter()
                .chain(of.holes.iter().flatten())
                .map(|v| model.vertices[v].position);
            let of_min = of_points
                .clone()
                .fold(DVec3::splat(f64::INFINITY), |a, b| a.min(b));
            let of_max = of_points.fold(DVec3::splat(f64::NEG_INFINITY), |a, b| a.max(b));

            if of_max.x <= sweep_min.x + slab_tol
                || of_min.x >= sweep_max.x - slab_tol
                || of_max.y <= sweep_min.y + slab_tol
                || of_min.y >= sweep_max.y - slab_tol
                || of_max.z <= sweep_min.z + slab_tol
                || of_min.z >= sweep_max.z - slab_tol
            {
                continue;
            }

            // Check if the sweep path from any source vertex penetrates `of`
            let dot = of.plane.normal.dot(delta);
            if dot.abs() > TOLERANCE {
                for v in source.outer.iter().chain(source.holes.iter().flatten()) {
                    let p = model.vertices[v].position;
                    let t = -of.plane.signed_distance(p) / dot;
                    if t > TOLERANCE && t < 1.0 - TOLERANCE {
                        let hit = p + delta * t;
                        if crate::transaction::contains_face(model, of, hit)? {
                            return Ok(None);
                        }
                    }
                }
            }

            for vert in of.outer.iter().chain(of.holes.iter().flatten()) {
                let p = model.vertices[vert].position;
                let dot = (p - origin).dot(normal);
                if dot > slab_tol && dot < distance.abs() - slab_tol {
                    let p2d = project2d(p);
                    let on_boundary = is_on_loop_boundary(&outer_2d, p2d)
                        || holes_2d.iter().any(|h| is_on_loop_boundary(h, p2d));
                    if !on_boundary
                        && crate::geometry::contains(&outer_2d, p2d)
                        && !holes_2d.iter().any(|h| crate::geometry::contains(h, p2d))
                    {
                        return Ok(None);
                    }
                }
            }
        }
    }
    let mut before = Model {
        next_id: model.next_id,
        ..Default::default()
    };
    let mut after = Model {
        next_id: high_water.max(model.next_id),
        ..Default::default()
    };
    let boundary = index
        .face_edges
        .get(face)
        .ok_or(KernelError::MissingEntity)?;
    for id in boundary {
        let edge = model
            .edges
            .get(id)
            .ok_or(KernelError::MissingEntity)?
            .clone();
        for v in edge.vertices {
            before.vertices.insert(v, model.vertices[&v].clone());
        }
        before.edges.insert(*id, edge);
    }
    before.faces.insert(*face, source.clone());
    after.vertices = before.vertices.clone();
    after.edges = before.edges.clone();
    let mut raised = BTreeMap::new();
    for (id, v) in &before.vertices {
        let position = v.position + delta;
        crate::geometry::validate_position(position)?;
        let new_id = VertexId::from_raw(after.allocate()?);
        after.vertices.insert(new_id, Vertex { position });
        raised.insert(*id, new_id);
    }
    let mut cap = source.clone();
    for v in cap.outer.iter_mut().chain(cap.holes.iter_mut().flatten()) {
        *v = raised[v];
    }
    cap.plane.distance += cap.plane.normal.dot(delta);
    after.faces.insert(*face, cap);
    if !closed && !attached {
        // An isolated profile needs a bottom closure. An attached sheet instead
        // replaces its starting face; keeping it creates a three-face junction.
        let base = FaceId::from_raw(after.allocate()?);
        after.faces.insert(base, source.clone());
    }
    for ring in std::iter::once(&source.outer).chain(source.holes.iter()) {
        for (a, b) in ring.iter().zip(ring.iter().cycle().skip(1)) {
            let mut outer = vec![*a, *b, raised[b], raised[a]];
            let pa = after.position(outer[0])?;
            let pb = after.position(outer[1])?;
            let pc = after.position(outer[2])?;
            let plane = Plane::through(pa, pb - pa, pc - pa).ok_or(KernelError::DegenerateEdge)?;
            if crate::geometry::signed_area(&after.project_loop(&outer, plane)?) < 0. {
                outer.reverse();
            }
            let id = FaceId::from_raw(after.allocate()?);
            after.faces.insert(
                id,
                Face {
                    outer,
                    holes: Vec::new(),
                    plane,
                    color: source.color,
                    material: source.material,
                },
            );
        }
    }
    let mut pairs = after
        .edges
        .values()
        .map(|e| (e.vertices, ()))
        .collect::<BTreeMap<_, _>>();
    let mut new_pairs = Vec::new();
    for f in after.faces.values() {
        for ring in std::iter::once(&f.outer).chain(f.holes.iter()) {
            for (a, b) in ring.iter().zip(ring.iter().cycle().skip(1)) {
                let mut pair = [*a, *b];
                pair.sort();
                if pairs.insert(pair, ()).is_none() {
                    new_pairs.push(pair);
                }
            }
        }
    }
    for vertices in new_pairs {
        let id = EdgeId::from_raw(after.allocate()?);
        after.edges.insert(
            id,
            Edge {
                vertices,
                soft: false,
                smooth: false,
                hidden: false,
            },
        );
    }
    if !walls.is_empty() {
        for id in walls {
            before.faces.insert(id, model.faces[&id].clone());
            after.faces.insert(id, model.faces[&id].clone());
            for edge in &index.face_edges[&id] {
                let e = &model.edges[edge];
                before.edges.insert(*edge, e.clone());
                after.edges.insert(*edge, e.clone());
                for v in e.vertices {
                    before.vertices.insert(v, model.vertices[&v].clone());
                    after.vertices.insert(v, model.vertices[&v].clone());
                }
            }
        }
        // Border vertices still used outside this patch must not be dissolved
        // while healing coplanar walls inside it.
        for v in before.vertices.keys() {
            if index.adjacency[v].iter().any(|edge| {
                !before.edges.contains_key(edge)
                    || index.edge_faces[edge]
                        .iter()
                        .any(|id| !before.faces.contains_key(id))
            }) {
                after.protected.insert(*v);
            }
        }
        for (edge_id, e) in &before.edges {
            if index.edge_faces[edge_id]
                .iter()
                .any(|id| !before.faces.contains_key(id))
            {
                after.protected_edges.insert(e.vertices);
            }
        }
        let mut polygons = Vec::new();
        for id in after.faces.keys() {
            for triangle in after.triangulate(*id)? {
                if let Some(p) = crate::csg::Polygon::new(triangle.to_vec()) {
                    polygons.push(p);
                }
            }
        }
        crate::modification::surfaces(&mut after, polygons, source.color, source.material)?;
    }
    after.validate()?;
    Ok(Some((before, after)))
}

/// Coplanarity excludes swept-volume collisions in either normal direction.
/// Triangle separation additionally excludes overlapping source surfaces at the
/// base. Work is bounded; ambiguous/expensive inputs retain the general path.
#[allow(dead_code)]
#[tracing::instrument(level = "info", skip_all, name = "extrusion.planar_sheet")]
fn planar_sheet(model: &Model, face: FaceId) -> Result<bool, KernelError> {
    let source = &model.faces[&face];
    let index = model.spatial.as_ref().ok_or(KernelError::MissingEntity)?;
    if index.edge_faces.len() != model.edges.len()
        || index.edge_faces.values().any(|faces| faces.is_empty())
        || model
            .vertices
            .values()
            .any(|v| source.plane.signed_distance(v.position).abs() > TOLERANCE)
    {
        return Ok(false);
    }
    let origin = model.position(source.outer[0])?;
    let (u, v) = source.plane.basis();
    let project =
        |t: [DVec3; 3]| t.map(|p| glam::DVec2::new((p - origin).dot(u), (p - origin).dot(v)));
    let triangles = model
        .triangulate(face)?
        .into_iter()
        .map(project)
        .collect::<Vec<_>>();
    let bounds =
        crate::spatial::envelope(source.outer.iter().map(|id| model.vertices[id].position))
            .ok_or(KernelError::MissingEntity)?;
    let mut work = 4096usize;
    for other in index
        .query_faces(bounds)
        .into_iter()
        .filter(|id| *id != face)
    {
        for triangle in model.triangulate(other)? {
            let b = project(triangle);
            for a in &triangles {
                let Some(left) = work.checked_sub(1) else {
                    return Ok(false);
                };
                work = left;
                if triangles_overlap(*a, b) {
                    return Ok(false);
                }
            }
        }
    }
    Ok(true)
}
#[allow(dead_code)]
fn triangles_overlap(a: [glam::DVec2; 3], b: [glam::DVec2; 3]) -> bool {
    // Separating axis theorem; boundary-only contact is allowed at model tolerance.
    for triangle in [a, b] {
        for i in 0..3 {
            let Some(axis) = (triangle[(i + 1) % 3] - triangle[i]).perp().try_normalize() else {
                return true;
            };
            let range = |t: [glam::DVec2; 3]| {
                t.into_iter()
                    .map(|p| p.dot(axis))
                    .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), d| {
                        (lo.min(d), hi.max(d))
                    })
            };
            let (alo, ahi) = range(a);
            let (blo, bhi) = range(b);
            if ahi.min(bhi) - alo.max(blo) <= TOLERANCE {
                return false;
            }
        }
    }
    true
}

fn is_on_loop_boundary(loop_pts: &[glam::DVec2], p: glam::DVec2) -> bool {
    for (a, b) in loop_pts.iter().zip(loop_pts.iter().cycle().skip(1)) {
        let ab = *b - *a;
        let ap = p - *a;
        let len_sq = ab.length_squared();
        if len_sq > TOLERANCE * TOLERANCE {
            let t = (ap.dot(ab) / len_sq).clamp(0.0, 1.0);
            if (*a + ab * t).distance(p) <= TOLERANCE {
                return true;
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn open_sheet_proof_declines_overlaps_nonplanarity_and_loose_wires() {
        let square = |origin: DVec3, size: f64| {
            [
                origin,
                origin + DVec3::X * size,
                origin + (DVec3::X + DVec3::Y) * size,
                origin + DVec3::Y * size,
            ]
        };
        let mut geometry = crate::BulkGeometry {
            vertices: square(DVec3::ZERO, 2.)
                .into_iter()
                .chain(square(DVec3::new(0.5, 0.5, 0.), 1.))
                .collect(),
            faces: [vec![0, 1, 2, 3], vec![4, 5, 6, 7]]
                .into_iter()
                .map(|outer| crate::BulkFace {
                    outer,
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        };
        let candidate = |g: crate::BulkGeometry| {
            let model = g.build(false, false).unwrap();
            let face = *model.faces.keys().next().unwrap();
            prepare(
                &model,
                &Command::PushPull {
                    face,
                    distance: 0.2,
                },
                model.next_id,
            )
            .unwrap()
            .is_some()
        };
        assert!(!candidate(geometry.clone()), "overlapping planar interiors");
        for p in &mut geometry.vertices[4..] {
            p.z = 0.1;
        }
        assert!(!candidate(geometry), "noncoplanar swept collision");
        let geometry = crate::BulkGeometry {
            vertices: square(DVec3::ZERO, 1.)
                .into_iter()
                .chain([DVec3::X * 3., DVec3::X * 4.])
                .collect(),
            faces: vec![crate::BulkFace {
                outer: vec![0, 1, 2, 3],
                ..Default::default()
            }],
            edges: vec![crate::BulkEdge {
                vertices: [4, 5],
                ..Default::default()
            }],
        };
        assert!(
            !candidate(geometry),
            "loose wire must keep general behavior"
        );
    }
    #[test]
    fn imported_open_sheet_has_local_extrusion_candidate() {
        let model = crate::BulkGeometry {
            vertices: vec![
                DVec3::ZERO,
                DVec3::X,
                DVec3::X + DVec3::Y,
                DVec3::Y,
                DVec3::X * 2.,
                DVec3::X * 2. + DVec3::Y,
            ],
            faces: [vec![0, 1, 2, 3], vec![1, 4, 5, 2]]
                .into_iter()
                .map(|outer| crate::BulkFace {
                    outer,
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        }
        .build(false, false)
        .unwrap();
        let face = *model.faces.keys().next().unwrap();
        assert!(
            prepare(
                &model,
                &Command::PushPull {
                    face,
                    distance: 0.2
                },
                model.next_id
            )
            .unwrap()
            .is_some(),
            "coplanar open sheet must avoid global surface reconstruction"
        );
    }
}
