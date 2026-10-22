//! Proven exterior extrusion of a supporting boundary face, preserving untouched topology.
use crate::{
    Command, DVec3, Edge, EdgeId, Face, FaceId, KernelError, Model, Plane, Vertex, VertexId,
    TOLERANCE,
};
use std::collections::BTreeMap;
/// Return no candidate unless a closed boundary or disjoint planar-sheet proof applies.
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
    if model.faces.values().any(|face| !face.holes.is_empty()) {
        return Ok(None);
    }
    let index = model.spatial.as_ref().ok_or(KernelError::MissingEntity)?;
    // No shell walk or orientation propagation: prove that the entire model is behind
    // the starting plane in the requested direction. The new prism is then disjoint
    // from all old surfaces except its base. This also handles reflections/holes.
    let closed = index
        .edge_faces
        .values()
        .filter(|v| !v.is_empty())
        .all(|v| v.len() == 2);
    if !closed && !planar_sheet(model, *face)? {
        return Ok(None);
    }
    let normal = source.plane.normal * distance.signum();
    let origin = model.position(source.outer[0])?;
    let supported = |direction: DVec3| {
        model
            .vertices
            .values()
            .all(|v| (v.position - origin).dot(direction) <= TOLERANCE)
    };
    let delta = source.plane.normal * *distance;
    if !supported(normal) {
        // A blind interior pocket is local only if its coplanar neighboring faces stay
        // above the cavity, and the swept prism does not reach any other shell surface.
        if !supported(-normal) {
            return Ok(None);
        }
        let boundary = index
            .face_edges
            .get(face)
            .ok_or(KernelError::MissingEntity)?;
        if boundary
            .iter()
            .flat_map(|e| index.edge_faces.get(e).into_iter().flatten())
            .filter(|other| **other != *face)
            .any(|other| !model.faces[other].plane.equivalent(&source.plane))
        {
            return Ok(None);
        }
        let points = source
            .outer
            .iter()
            .map(|id| model.position(*id))
            .collect::<Result<Vec<_>, _>>()?;
        let bounds = crate::spatial::envelope(
            points
                .iter()
                .copied()
                .chain(points.iter().map(|p| *p + delta)),
        )
        .ok_or(KernelError::MissingEntity)?;
        if index
            .query_faces(bounds)
            .iter()
            .any(|other| !model.faces[other].plane.equivalent(&source.plane))
        {
            return Ok(None);
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
    if !closed {
        // The general open-surface path extrudes a complete prism, including its
        // base. Closed solids instead replace the source boundary with the cap.
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
    after.validate()?;
    Ok(Some((before, after)))
}

/// Coplanarity excludes swept-volume collisions in either normal direction.
/// Triangle separation additionally excludes overlapping source surfaces at the
/// base. Work is bounded; ambiguous/expensive inputs retain the general path.
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
