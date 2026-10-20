use std::collections::{BTreeMap, BTreeSet};

use glam::{DVec2, DVec3};

use crate::{
    geometry::{self, TOLERANCE},
    KernelError, Plane,
};

macro_rules! entity_id {
    ($name:ident, $doc:literal) => {
        #[doc = $doc]
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(u64);
        impl $name {
            /// Reconstruct an ID; callers must still check model membership.
            pub const fn from_raw(value: u64) -> Self {
                Self(value)
            }
            /// Numeric identity, scoped to one document history.
            pub const fn value(self) -> u64 {
                self.0
            }
        }
    };
}
entity_id!(VertexId, "Stable vertex identity within a document.");
entity_id!(EdgeId, "Stable edge identity within a document.");
entity_id!(FaceId, "Stable face identity within a document.");

/// A vertex position in meters.
#[derive(Debug, Clone, PartialEq)]
pub struct Vertex {
    /// Double-precision world position.
    pub position: DVec3,
}

/// An undirected topological edge.
#[derive(Debug, Clone, PartialEq)]
pub struct Edge {
    /// Distinct endpoint IDs, in ascending order.
    pub vertices: [VertexId; 2],
    /// Hide the edge in ordinary shaded views.
    pub soft: bool,
    /// Interpolate shading normals across this edge.
    pub smooth: bool,
}

/// A bounded planar region. Inner loops remove area from the outer loop.
#[derive(Debug, Clone, PartialEq)]
pub struct Face {
    /// Counterclockwise boundary relative to the plane normal.
    pub outer: Vec<VertexId>,
    /// Clockwise inner boundaries; the enclosed regions may have their own faces.
    pub holes: Vec<Vec<VertexId>>,
    /// Supporting plane.
    pub plane: Plane,
}

/// Read-only topology. Mutations are available only through command history.
#[derive(Debug, Clone, PartialEq)]
pub struct Model {
    pub(crate) vertices: BTreeMap<VertexId, Vertex>,
    pub(crate) edges: BTreeMap<EdgeId, Edge>,
    pub(crate) faces: BTreeMap<FaceId, Face>,
    pub(crate) next_id: u64,
}

impl Default for Model {
    fn default() -> Self {
        Self {
            vertices: BTreeMap::new(),
            edges: BTreeMap::new(),
            faces: BTreeMap::new(),
            next_id: 1,
        }
    }
}

impl Model {
    /// All vertices; no mutable map is exposed.
    pub fn vertices(&self) -> &BTreeMap<VertexId, Vertex> {
        &self.vertices
    }
    /// All edges; no mutable map is exposed.
    pub fn edges(&self) -> &BTreeMap<EdgeId, Edge> {
        &self.edges
    }
    /// All automatically derived faces.
    pub fn faces(&self) -> &BTreeMap<FaceId, Face> {
        &self.faces
    }
    /// Edges incident to a vertex, in stable ID order.
    pub fn incident_edges(&self, vertex: VertexId) -> impl Iterator<Item = (EdgeId, &Edge)> {
        self.edges
            .iter()
            .filter(move |(_, edge)| edge.vertices.contains(&vertex))
            .map(|(id, edge)| (*id, edge))
    }
    /// Axis-aligned model bounds, or `None` for an empty model.
    pub fn bounds(&self) -> Option<(DVec3, DVec3)> {
        self.vertices
            .values()
            .map(|v| v.position)
            .fold(None, |bounds, p| {
                Some(bounds.map_or((p, p), |(min, max)| (min.min(p), max.max(p))))
            })
    }
    /// Area of a face in square meters, subtracting its holes.
    pub fn face_area(&self, id: FaceId) -> Result<f64, KernelError> {
        let face = self.faces.get(&id).ok_or(KernelError::MissingEntity)?;
        let outer = geometry::signed_area(&self.project_loop(&face.outer, face.plane)?).abs();
        face.holes.iter().try_fold(outer, |area, ring| {
            Ok(area - geometry::signed_area(&self.project_loop(ring, face.plane)?).abs())
        })
    }

    /// Triangulate one face, including holes, into world-space triangles.
    pub fn triangulate(&self, id: FaceId) -> Result<Vec<[DVec3; 3]>, KernelError> {
        let face = self.faces.get(&id).ok_or(KernelError::MissingEntity)?;
        let origin = self.position(*face.outer.first().ok_or(KernelError::Triangulation)?)?;
        let mut points = Vec::new();
        let mut flattened = Vec::new();
        let mut holes = Vec::new();
        for (index, ring) in std::iter::once(&face.outer)
            .chain(face.holes.iter())
            .enumerate()
        {
            if index > 0 {
                holes.push(points.len());
            }
            for (id, p) in ring.iter().zip(self.project_at(ring, face.plane, origin)?) {
                points.push(self.position(*id)?);
                flattened.extend([p.x, p.y]);
            }
        }
        let indices =
            earcutr::earcut(&flattened, &holes, 2).map_err(|_| KernelError::Triangulation)?;
        let mut triangles = Vec::new();
        for triangle in indices.as_chunks::<3>().0 {
            let a = *points.get(triangle[0]).ok_or(KernelError::Triangulation)?;
            let mut b = *points.get(triangle[1]).ok_or(KernelError::Triangulation)?;
            let mut c = *points.get(triangle[2]).ok_or(KernelError::Triangulation)?;
            if (b - a).cross(c - a).dot(face.plane.normal) < 0.0 {
                std::mem::swap(&mut b, &mut c);
            }
            if (b - a).cross(c - a).length() > TOLERANCE * TOLERANCE {
                triangles.push([a, b, c]);
            }
        }
        Ok(triangles)
    }

    pub(crate) fn allocate(&mut self) -> Result<u64, KernelError> {
        let id = self.next_id;
        self.next_id = id.checked_add(1).ok_or(KernelError::Capacity)?;
        Ok(id)
    }
    pub(crate) fn position(&self, id: VertexId) -> Result<DVec3, KernelError> {
        self.vertices
            .get(&id)
            .map(|v| v.position)
            .ok_or(KernelError::MissingEntity)
    }
    pub(crate) fn intern(&mut self, position: DVec3) -> Result<VertexId, KernelError> {
        geometry::validate_position(position)?;
        if let Some((id, _)) = self
            .vertices
            .iter()
            .find(|(_, v)| v.position.distance(position) <= TOLERANCE)
        {
            return Ok(*id);
        }
        let id = VertexId::from_raw(self.allocate()?);
        self.vertices.insert(id, Vertex { position });
        Ok(id)
    }
    pub(crate) fn project_loop(
        &self,
        ring: &[VertexId],
        plane: Plane,
    ) -> Result<Vec<DVec2>, KernelError> {
        let origin = self.position(*ring.first().ok_or(KernelError::Triangulation)?)?;
        self.project_at(ring, plane, origin)
    }
    pub(crate) fn project_at(
        &self,
        ring: &[VertexId],
        plane: Plane,
        origin: DVec3,
    ) -> Result<Vec<DVec2>, KernelError> {
        let (u, v) = plane.basis();
        ring.iter()
            .map(|id| {
                self.position(*id)
                    .map(|p| DVec2::new((p - origin).dot(u), (p - origin).dot(v)))
            })
            .collect()
    }
    pub(crate) fn same_geometry(&self, other: &Self) -> bool {
        self.vertices == other.vertices && self.edges == other.edges && self.faces == other.faces
    }

    /// Check endpoints, intersections, face planarity, loop winding and triangulated area.
    ///
    /// Full validation is deliberately conservative and quadratic in edge count.
    pub fn validate(&self) -> Result<(), KernelError> {
        let invalid = |message: &str| KernelError::InvalidTopology(message.to_owned());
        for vertex in self.vertices.values() {
            geometry::validate_position(vertex.position)?;
        }
        let mut pairs = BTreeSet::new();
        for edge in self.edges.values() {
            if edge.vertices[0] >= edge.vertices[1] || !pairs.insert(edge.vertices) {
                return Err(invalid("duplicate or unordered edge"));
            }
            if self
                .position(edge.vertices[0])?
                .distance(self.position(edge.vertices[1])?)
                <= TOLERANCE
            {
                return Err(KernelError::DegenerateEdge);
            }
        }
        let edges: Vec<_> = self.edges.values().collect();
        for (i, a) in edges.iter().enumerate() {
            for b in &edges[i + 1..] {
                let crossings = geometry::intersections(
                    self.position(a.vertices[0])?,
                    self.position(a.vertices[1])?,
                    self.position(b.vertices[0])?,
                    self.position(b.vertices[1])?,
                );
                for point in crossings {
                    let shared = a
                        .vertices
                        .iter()
                        .filter(|id| b.vertices.contains(id))
                        .any(|id| self.vertices[id].position.distance(point) <= TOLERANCE);
                    if !shared {
                        return Err(invalid("intersection has not been split"));
                    }
                }
            }
        }
        for (id, face) in &self.faces {
            if (face.plane.normal.length() - 1.0).abs() > TOLERANCE {
                return Err(invalid("non-unit face normal"));
            }
            for (index, ring) in std::iter::once(&face.outer)
                .chain(face.holes.iter())
                .enumerate()
            {
                if ring.len() < 3 || ring.iter().collect::<BTreeSet<_>>().len() != ring.len() {
                    return Err(invalid("face ring is not simple"));
                }
                for (a, b) in ring.iter().zip(ring.iter().cycle().skip(1)) {
                    let mut pair = [*a, *b];
                    pair.sort();
                    if !pairs.contains(&pair) {
                        return Err(invalid("face boundary edge missing"));
                    }
                    if face.plane.signed_distance(self.position(*a)?).abs() > TOLERANCE {
                        return Err(invalid("nonplanar face"));
                    }
                }
                let area = geometry::signed_area(&self.project_loop(ring, face.plane)?);
                if (index == 0 && area <= TOLERANCE * TOLERANCE)
                    || (index > 0 && area >= -TOLERANCE * TOLERANCE)
                {
                    return Err(invalid("incorrect face winding"));
                }
            }
            let area = self.face_area(*id)?;
            let triangle_area: f64 = self
                .triangulate(*id)?
                .iter()
                .map(|[a, b, c]| (*b - *a).cross(*c - *a).length() * 0.5)
                .sum();
            if area <= TOLERANCE * TOLERANCE
                || (area - triangle_area).abs() > TOLERANCE * area.max(1.0)
            {
                return Err(invalid("face/triangulation area mismatch"));
            }
        }
        Ok(())
    }
}
