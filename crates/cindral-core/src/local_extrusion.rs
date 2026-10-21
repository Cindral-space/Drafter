//! Proven exterior extrusion of a supporting boundary face, preserving untouched topology.
use crate::{
    Command, DVec3, Edge, EdgeId, Face, FaceId, KernelError, Model, Plane, Vertex, VertexId,
    TOLERANCE,
};
use std::collections::BTreeMap;
/// Return no candidate unless the support-plane and closed-boundary proof applies.
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
    if index
        .edge_faces
        .values()
        .filter(|v| !v.is_empty())
        .any(|v| v.len() != 2)
    {
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
