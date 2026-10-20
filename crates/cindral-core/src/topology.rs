use std::collections::{BTreeMap, BTreeSet};

use glam::{DVec2, DVec3};

use crate::{
    geometry::{self, TOLERANCE},
    Edge, EdgeId, Face, FaceId, KernelError, Model, Plane, VertexId,
};

impl Model {
    fn add_edge_ids(
        &mut self,
        a: VertexId,
        b: VertexId,
        style: (bool, bool),
        preferred: Option<EdgeId>,
    ) -> Result<(), KernelError> {
        if a == b || self.position(a)?.distance(self.position(b)?) <= TOLERANCE {
            return Ok(());
        }
        let mut vertices = [a, b];
        vertices.sort();
        if self.edges.values().any(|e| e.vertices == vertices) {
            return Ok(());
        }
        let id = match preferred {
            Some(id) => id,
            None => EdgeId::from_raw(self.allocate()?),
        };
        self.edges.insert(
            id,
            Edge {
                vertices,
                soft: style.0,
                smooth: style.1,
            },
        );
        Ok(())
    }

    pub(crate) fn insert_segment(&mut self, start: DVec3, end: DVec3) -> Result<(), KernelError> {
        geometry::validate_position(start)?;
        geometry::validate_position(end)?;
        if start.distance(end) <= TOLERANCE {
            return Err(KernelError::DegenerateEdge);
        }
        let a = self.intern(start)?;
        let b = self.intern(end)?;
        if a == b {
            return Err(KernelError::DegenerateEdge);
        }
        let start = self.position(a)?;
        let end = self.position(b)?;
        let mut cuts = vec![a, b];
        let edges: Vec<_> = self
            .edges
            .iter()
            .map(|(id, edge)| (*id, edge.clone()))
            .collect();
        for (id, edge) in edges {
            let c = self.position(edge.vertices[0])?;
            let d = self.position(edge.vertices[1])?;
            let points = geometry::intersections(start, end, c, d);
            let mut edge_cuts = edge.vertices.to_vec();
            for point in points {
                let vertex = self.intern(point)?;
                cuts.push(vertex);
                edge_cuts.push(vertex);
            }
            self.sort_along(&mut edge_cuts, c, d);
            if edge_cuts.len() > 2 {
                self.edges.remove(&id);
                for (index, pair) in edge_cuts.windows(2).enumerate() {
                    self.add_edge_ids(
                        pair[0],
                        pair[1],
                        (edge.soft, edge.smooth),
                        (index == 0).then_some(id),
                    )?;
                }
            }
        }
        self.sort_along(&mut cuts, start, end);
        for pair in cuts.windows(2) {
            self.add_edge_ids(pair[0], pair[1], (false, false), None)?;
        }
        Ok(())
    }

    fn sort_along(&self, vertices: &mut Vec<VertexId>, start: DVec3, end: DVec3) {
        vertices.sort_by(|a, b| {
            geometry::parameter(self.vertices[a].position, start, end)
                .total_cmp(&geometry::parameter(self.vertices[b].position, start, end))
                .then(a.cmp(b))
        });
        vertices.dedup();
    }

    /// Erasure dissolves redundant collinear degree-two vertices, preserving style seams.
    pub(crate) fn heal(&mut self) -> Result<(), KernelError> {
        loop {
            let used: BTreeSet<_> = self.edges.values().flat_map(|e| e.vertices).collect();
            self.vertices
                .retain(|id, _| used.contains(id) || self.protected.contains(id));
            let mut merge = None;
            for (vertex, data) in &self.vertices {
                if self.protected.contains(vertex)
                    || self
                        .heal_scope
                        .as_ref()
                        .is_some_and(|scope| !scope.contains(vertex))
                {
                    continue;
                }
                let incident: Vec<_> = self.incident_edges(*vertex).collect();
                if let [(id_a, a), (id_b, b)] = incident.as_slice() {
                    if (a.soft, a.smooth) != (b.soft, b.smooth) {
                        continue;
                    }
                    let Some(other_a) = a.vertices.iter().find(|id| *id != vertex) else {
                        continue;
                    };
                    let Some(other_b) = b.vertices.iter().find(|id| *id != vertex) else {
                        continue;
                    };
                    let pa = self.position(*other_a)?;
                    let pb = self.position(*other_b)?;
                    if geometry::on_segment(data.position, pa, pb)
                        && (pa - data.position).dot(pb - data.position) < 0.0
                    {
                        merge = Some((
                            *vertex,
                            *id_a,
                            *id_b,
                            *other_a,
                            *other_b,
                            (a.soft, a.smooth),
                        ));
                        break;
                    }
                }
            }
            let Some((vertex, a, b, start, end, style)) = merge else {
                break;
            };
            if let Some(scope) = &mut self.heal_scope {
                scope.insert(start);
                scope.insert(end);
            }
            self.edges.remove(&a);
            self.edges.remove(&b);
            self.vertices.remove(&vertex);
            self.add_edge_ids(start, end, style, Some(a.min(b)))?;
        }
        Ok(())
    }

    pub(crate) fn rebuild_faces(&mut self) -> Result<(), KernelError> {
        let mut planes = Vec::<Plane>::new();
        for face in self.faces.values() {
            if !planes.iter().any(|plane| plane.equivalent(&face.plane)) {
                planes.push(face.plane);
            }
        }
        for (id, vertex) in &self.vertices {
            let neighbors: Vec<_> = self
                .incident_edges(*id)
                .filter_map(|(_, edge)| edge.vertices.into_iter().find(|other| other != id))
                .collect();
            for (i, a) in neighbors.iter().enumerate() {
                for b in &neighbors[i + 1..] {
                    if let Some(plane) = Plane::through(
                        vertex.position,
                        self.position(*a)? - vertex.position,
                        self.position(*b)? - vertex.position,
                    ) {
                        if !planes.iter().any(|p| p.equivalent(&plane)) {
                            planes.push(plane);
                        }
                    }
                }
            }
        }
        let old: BTreeMap<_, _> = self
            .faces
            .iter()
            .map(|(id, f)| (face_key(f), *id))
            .collect();
        let mut faces = BTreeMap::new();
        let mut seen = BTreeSet::new();
        for plane in planes {
            for face in self.plane_faces(plane)? {
                if !seen.insert(face_key(&face)) {
                    continue;
                }
                let id = if let Some(id) = old.get(&face_key(&face)) {
                    *id
                } else {
                    FaceId::from_raw(self.allocate()?)
                };
                let mut face = face;
                if let Some(previous) = self.faces.get(&id) {
                    face.color = previous.color;
                    face.material = previous.material;
                }
                faces.insert(id, face);
            }
        }
        self.faces = faces;
        Ok(())
    }

    fn plane_faces(&self, plane: Plane) -> Result<Vec<Face>, KernelError> {
        let mut adjacency = BTreeMap::<VertexId, Vec<VertexId>>::new();
        for edge in self.edges.values() {
            let [a, b] = edge.vertices;
            if plane.signed_distance(self.position(a)?).abs() <= TOLERANCE
                && plane.signed_distance(self.position(b)?).abs() <= TOLERANCE
            {
                adjacency.entry(a).or_default().push(b);
                adjacency.entry(b).or_default().push(a);
            }
        }
        let Some(first) = adjacency.keys().next() else {
            return Ok(Vec::new());
        };
        let origin = self.position(*first)?;
        let (u, v) = plane.basis();
        for (id, neighbors) in &mut adjacency {
            let p = self.position(*id)?;
            neighbors.sort_by(|a, b| {
                let da = self.vertices[a].position - p;
                let db = self.vertices[b].position - p;
                da.dot(v)
                    .atan2(da.dot(u))
                    .total_cmp(&db.dot(v).atan2(db.dot(u)))
                    .then(a.cmp(b))
            });
        }
        let mut visited = BTreeSet::new();
        let mut rings = BTreeMap::new();
        let half_edges = adjacency.values().map(Vec::len).sum::<usize>();
        for (start, neighbors) in &adjacency {
            for neighbor in neighbors {
                let initial = (*start, *neighbor);
                if visited.contains(&initial) {
                    continue;
                }
                let mut edge = initial;
                let mut walk = Vec::new();
                loop {
                    if !visited.insert(edge) {
                        if edge != initial {
                            return Err(KernelError::InvalidTopology(
                                "half-edge traversal did not close".into(),
                            ));
                        }
                        break;
                    }
                    walk.push(edge.0);
                    if walk.len() > half_edges {
                        return Err(KernelError::InvalidTopology(
                            "half-edge traversal overflow".into(),
                        ));
                    }
                    let next = adjacency.get(&edge.1).ok_or(KernelError::MissingEntity)?;
                    let index = next
                        .iter()
                        .position(|id| *id == edge.0)
                        .ok_or(KernelError::MissingEntity)?;
                    edge = (edge.1, next[(index + next.len() - 1) % next.len()]);
                }
                for ring in simple_rings(&walk) {
                    let projected = self.project_at(&ring, plane, origin)?;
                    let area = geometry::signed_area(&projected);
                    if area > TOLERANCE * TOLERANCE {
                        rings
                            .entry(canonical_ring(&ring))
                            .or_insert((ring, projected, area));
                    }
                }
            }
        }
        let rings: Vec<_> = rings.into_values().collect();
        let mut parent = vec![None; rings.len()];
        for (i, (_, polygon, area)) in rings.iter().enumerate() {
            let sample = interior_sample(polygon)?;
            parent[i] = rings
                .iter()
                .enumerate()
                .filter(|(j, (_, other, other_area))| {
                    *j != i
                        && *other_area > *area + TOLERANCE * TOLERANCE
                        && geometry::contains(other, sample)
                })
                .min_by(|(_, a), (_, b)| a.2.total_cmp(&b.2))
                .map(|(j, _)| j);
        }
        let mut faces = Vec::new();
        for (i, (ring, _, _)) in rings.iter().enumerate() {
            let mut holes: Vec<_> = rings
                .iter()
                .enumerate()
                .filter(|(j, _)| parent[*j] == Some(i))
                .map(|(_, (ring, _, _))| {
                    let mut ring = ring.clone();
                    ring.reverse();
                    ring
                })
                .collect();
            holes.sort_by_key(|ring| canonical_ring(ring));
            faces.push(Face {
                color: [160, 177, 193, 255],
                material: None,
                outer: ring.clone(),
                holes,
                plane,
            });
        }
        Ok(faces)
    }
}

/// Decompose a closed half-edge walk at repeated vertices, discarding bridges.
fn simple_rings(walk: &[VertexId]) -> Vec<Vec<VertexId>> {
    let mut stack = Vec::new();
    let mut positions = BTreeMap::new();
    let mut rings = Vec::new();
    for vertex in walk.iter().chain(walk.first()) {
        if let Some(index) = positions.get(vertex).copied() {
            let ring = stack[index..].to_vec();
            if ring.len() >= 3 {
                rings.push(ring);
            }
            for removed in stack.drain(index + 1..) {
                positions.remove(&removed);
            }
        } else {
            positions.insert(*vertex, stack.len());
            stack.push(*vertex);
        }
    }
    rings
}

fn interior_sample(polygon: &[DVec2]) -> Result<DVec2, KernelError> {
    let flattened: Vec<_> = polygon.iter().flat_map(|p| [p.x, p.y]).collect();
    let indices = earcutr::earcut(&flattened, &[], 2).map_err(|_| KernelError::Triangulation)?;
    for triangle in indices.as_chunks::<3>().0 {
        let a = polygon[triangle[0]];
        let b = polygon[triangle[1]];
        let c = polygon[triangle[2]];
        if (b - a).perp_dot(c - a).abs() > TOLERANCE * TOLERANCE {
            return Ok((a + b + c) / 3.0);
        }
    }
    Err(KernelError::Triangulation)
}

fn canonical_ring(ring: &[VertexId]) -> Vec<VertexId> {
    let Some((start, _)) = ring.iter().enumerate().min_by_key(|(_, id)| *id) else {
        return Vec::new();
    };
    ring.iter()
        .cycle()
        .skip(start)
        .take(ring.len())
        .copied()
        .collect()
}

pub(crate) type FaceKey = (Vec<VertexId>, Vec<Vec<VertexId>>);
pub(crate) fn face_key(face: &Face) -> FaceKey {
    let mut holes: Vec<_> = face
        .holes
        .iter()
        .map(|ring| unoriented_ring(ring))
        .collect();
    holes.sort();
    (unoriented_ring(&face.outer), holes)
}

fn unoriented_ring(ring: &[VertexId]) -> Vec<VertexId> {
    let reversed: Vec<_> = ring.iter().rev().copied().collect();
    canonical_ring(ring).min(canonical_ring(&reversed))
}
