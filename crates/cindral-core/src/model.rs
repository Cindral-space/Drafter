use std::collections::{BTreeMap, BTreeSet};

use glam::{DVec2, DVec3};

use crate::{
    geometry::{self, TOLERANCE},
    KernelError, Plane,
};

macro_rules! entity_id {
    ($name:ident, $doc:literal) => {
        #[doc = $doc]
        #[derive(
            Debug,
            Clone,
            Copy,
            PartialEq,
            Eq,
            PartialOrd,
            Ord,
            Hash,
            serde::Serialize,
            serde::Deserialize,
        )]
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
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Vertex {
    /// Double-precision world position.
    pub position: DVec3,
}

/// An undirected topological edge.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Edge {
    /// Distinct endpoint IDs, in ascending order.
    pub vertices: [VertexId; 2],
    /// Hide the edge in ordinary shaded views.
    pub soft: bool,
    /// Interpolate shading normals across this edge.
    pub smooth: bool,
    /// Explicit user-hidden edge, independent of soft/smooth shading.
    #[serde(default)]
    pub hidden: bool,
}

/// A bounded planar region. Inner loops remove area from the outer loop.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Face {
    /// Counterclockwise boundary relative to the plane normal.
    pub outer: Vec<VertexId>,
    /// Clockwise inner boundaries; the enclosed regions may have their own faces.
    pub holes: Vec<Vec<VertexId>>,
    /// Supporting plane.
    pub plane: Plane,
    /// Linear display color as RGBA bytes.
    pub color: [u8; 4],
    /// Optional document material reference.
    #[serde(default)]
    pub material: Option<crate::MaterialId>,
}

/// Read-only topology. Mutations are available only through command history.
#[derive(Debug, Clone)]
pub struct Model {
    pub(crate) vertices: BTreeMap<VertexId, Vertex>,
    pub(crate) edges: BTreeMap<EdgeId, Edge>,
    pub(crate) faces: BTreeMap<FaceId, Face>,
    pub(crate) next_id: u64,
    pub(crate) spatial: Option<Box<crate::spatial::Spatial>>,
    pub(crate) protected: BTreeSet<VertexId>,
    pub(crate) protected_edges: BTreeSet<[VertexId; 2]>,
    pub(crate) heal_scope: Option<BTreeSet<VertexId>>,
}

impl Default for Model {
    fn default() -> Self {
        Self {
            vertices: BTreeMap::new(),
            edges: BTreeMap::new(),
            faces: BTreeMap::new(),
            next_id: 1,
            spatial: None,
            protected: BTreeSet::new(),
            protected_edges: BTreeSet::new(),
            heal_scope: None,
        }
    }
}

impl Model {
    /// Runtime affine spatial/geometry cache frame, never serialized.
    pub fn geometry_frame(&self) -> glam::DMat4 {
        self.spatial.as_ref().map_or(glam::DMat4::IDENTITY, |s| {
            glam::DMat4::from_translation(s.offset) * s.basis
        })
    }
    /// Runtime translation of the spatial coordinate frame (never serialized).
    pub fn translation_offset(&self) -> DVec3 {
        self.geometry_frame().w_axis.truncate()
    }
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
        let mut indexed = self
            .spatial
            .as_ref()
            .map(|s| s.adjacency.get(&vertex).into_iter().flatten());
        let mut scan = self.edges.iter();
        std::iter::from_fn(move || {
            if let Some(ids) = &mut indexed {
                return ids.find_map(|id| self.edges.get(id).map(|edge| (*id, edge)));
            }
            scan.find(|(_, edge)| edge.vertices.contains(&vertex))
                .map(|(id, edge)| (*id, edge))
        })
    }

    /// Conservative face candidates along a forward ray, in stable ID order.
    /// Uses the maintained spatial cache, falling back to all faces when unindexed.
    /// Direction is normalized here; `max_distance` is in meters (infinity allowed).
    /// Callers must still test exact face boundaries, holes and visibility.
    pub fn face_candidates_for_ray(
        &self,
        origin: DVec3,
        direction: DVec3,
        max_distance: f64,
    ) -> Vec<FaceId> {
        let Some(direction) = direction.try_normalize() else {
            return Vec::new();
        };
        if !origin.is_finite()
            || !direction.is_finite()
            || max_distance.is_nan()
            || max_distance < 0.
        {
            return Vec::new();
        }
        self.spatial.as_ref().map_or_else(
            || self.faces.keys().copied().collect(),
            |index| index.ray_faces(origin, direction, max_distance),
        )
    }

    /// Boundary edge identities for a face, including inner loops.
    pub fn face_edges(&self, id: FaceId) -> Vec<EdgeId> {
        if let Some(spatial) = &self.spatial {
            return spatial
                .face_edges
                .get(&id)
                .into_iter()
                .flatten()
                .copied()
                .collect();
        }
        let Some(face) = self.faces.get(&id) else {
            return Vec::new();
        };
        std::iter::once(&face.outer)
            .chain(face.holes.iter())
            .flat_map(|ring| {
                ring.iter()
                    .zip(ring.iter().cycle().skip(1))
                    .take(ring.len())
            })
            .filter_map(|(a, b)| {
                self.incident_edges(*a)
                    .find(|(_, edge)| edge.vertices.contains(b))
                    .map(|(id, _)| id)
            })
            .collect()
    }

    /// Incident faces of an edge, using the maintained adjacency index.
    pub fn edge_faces(&self, id: EdgeId) -> impl Iterator<Item = (FaceId, &Face)> {
        let indexed = self
            .spatial
            .as_ref()
            .map(|s| s.edge_faces.get(&id).into_iter().flatten());
        let edge = self.edges.get(&id);
        let mut indexed = indexed;
        let mut scan = self.faces.iter();
        std::iter::from_fn(move || {
            if let Some(ids) = &mut indexed {
                return ids.find_map(|id| self.faces.get(id).map(|face| (*id, face)));
            }
            let edge = edge?;
            scan.find(|(_, face)| {
                std::iter::once(&face.outer)
                    .chain(face.holes.iter())
                    .any(|ring| {
                        ring.iter()
                            .zip(ring.iter().cycle().skip(1))
                            .take(ring.len())
                            .any(|(a, b)| edge.vertices.contains(a) && edge.vertices.contains(b))
                    })
            })
            .map(|(id, face)| (*id, face))
        })
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
        if let Some(index) = &self.spatial {
            if let Some(id) = index
                .vertices_near(position)
                .into_iter()
                .filter(|id| self.vertices[id].position.distance(position) <= TOLERANCE)
                .min()
            {
                return Ok(id);
            }
        } else if let Some((id, _)) = self
            .vertices
            .iter()
            .find(|(_, v)| v.position.distance(position) <= TOLERANCE)
        {
            return Ok(*id);
        }
        let id = VertexId::from_raw(self.allocate()?);
        self.vertices.insert(id, Vertex { position });
        if let Some(index) = &mut self.spatial {
            index.add_vertex(id, position);
        }
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
    #[tracing::instrument(level = "info", skip_all, name = "model.validate")]
    pub fn validate(&self) -> Result<(), KernelError> {
        self.validate_mesh(true)
    }
    // Mesh import preserves overlapping surfaces; intersections are not edit arrangements.
    pub(crate) fn validate_mesh(&self, intersections: bool) -> Result<(), KernelError> {
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
        if intersections {
            let intersection_span =
                tracing::info_span!("validate.edge_pairs", edges = self.edges.len()).entered();
            // Conservative R-tree broad phase; the exact tolerance-aware predicate below
            // is unchanged. Expanded AABBs retain touching and shallow-angle cases.
            let edges: Vec<_> = self
                .edges
                .values()
                .map(|edge| {
                    Ok((
                        edge,
                        self.position(edge.vertices[0])?,
                        self.position(edge.vertices[1])?,
                    ))
                })
                .collect::<Result<_, KernelError>>()?;
            let tree = rstar::RTree::bulk_load(
                edges
                    .iter()
                    .enumerate()
                    .map(|(i, (_, a, b))| {
                        rstar::primitives::GeomWithData::new(
                            rstar::primitives::Line::new(a.to_array(), b.to_array()),
                            i,
                        )
                    })
                    .collect(),
            );
            let mut pair_budget = 5_000_000usize;
            for (i, (a, pa, pb)) in edges.iter().enumerate() {
                let bounds = rstar::AABB::from_corners(
                    (pa.min(*pb) - DVec3::splat(TOLERANCE)).to_array(),
                    (pa.max(*pb) + DVec3::splat(TOLERANCE)).to_array(),
                );
                for candidate in tree.locate_in_envelope_intersecting(&bounds) {
                    let j = candidate.data;
                    if j <= i {
                        continue;
                    }
                    pair_budget = pair_budget.checked_sub(1).ok_or(KernelError::Capacity)?;
                    let (b, pc, pd) = edges[j];
                    for point in geometry::intersections(*pa, *pb, pc, pd) {
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
            drop(intersection_span);
        }
        let _faces_span = tracing::info_span!("validate.faces").entered();
        for (id, face) in &self.faces {
            if !face.plane.normal.is_finite()
                || !face.plane.distance.is_finite()
                || (face.plane.normal.length() - 1.0).abs() > TOLERANCE
            {
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
        if let Some(spatial) = &self.spatial {
            spatial.validate(self)?;
        }
        Ok(())
    }
}

impl PartialEq for Model {
    fn eq(&self, other: &Self) -> bool {
        self.same_geometry(other) && self.next_id == other.next_id
    }
}
impl Model {
    pub(crate) fn ensure_spatial(&mut self) -> Result<(), KernelError> {
        if self.spatial.is_none() {
            self.spatial = Some(Box::new(crate::spatial::Spatial::build(self)?));
        }
        Ok(())
    }
}
