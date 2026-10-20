//! Derived indices. They are caches, never part of semantic model identity.
use crate::topology::{face_key, FaceKey};
use crate::{DVec3, Edge, EdgeId, Face, FaceId, KernelError, Model, VertexId, TOLERANCE};
use rstar::{RTree, RTreeObject, AABB};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, PartialEq)]
struct Item<I: Copy + PartialEq> {
    id: I,
    bounds: AABB<[f64; 3]>,
}
impl<I: Copy + PartialEq> RTreeObject for Item<I> {
    type Envelope = AABB<[f64; 3]>;
    fn envelope(&self) -> Self::Envelope {
        self.bounds
    }
}
#[derive(Debug, Clone, Default)]
pub(crate) struct Spatial {
    pub offset: DVec3,
    vertices: RTree<Item<VertexId>>,
    edges: RTree<Item<EdgeId>>,
    faces: RTree<Item<FaceId>>,
    pub adjacency: BTreeMap<VertexId, BTreeSet<EdgeId>>,
    pub edge_faces: BTreeMap<EdgeId, BTreeSet<FaceId>>,
    pub face_edges: BTreeMap<FaceId, BTreeSet<EdgeId>>,
    pub face_keys: BTreeMap<FaceKey, FaceId>,
    pairs: BTreeMap<[VertexId; 2], EdgeId>,
}

// Translation/reversal can change a reconstructed bound by an ULP. Find by stable
// identity in a conservative local neighborhood, then remove its exact stored item.
fn remove_item<I: Copy + PartialEq>(tree: &mut RTree<Item<I>>, id: I, bounds: AABB<[f64; 3]>) {
    let found = tree
        .locate_in_envelope_intersecting(&expanded(expanded(bounds)))
        .find(|item| item.id == id)
        .cloned();
    if let Some(found) = found {
        tree.remove(&found);
    }
}

pub(crate) fn envelope(points: impl IntoIterator<Item = DVec3>) -> Option<AABB<[f64; 3]>> {
    let mut bounds: Option<(DVec3, DVec3)> = None;
    for p in points {
        bounds = Some(bounds.map_or((p, p), |(a, b)| (a.min(p), b.max(p))));
    }
    bounds.map(|(a, b)| AABB::from_corners(a.to_array(), b.to_array()))
}
pub(crate) fn expanded(bounds: AABB<[f64; 3]>) -> AABB<[f64; 3]> {
    let low = DVec3::from_array(bounds.lower()) - DVec3::splat(TOLERANCE);
    let high = DVec3::from_array(bounds.upper()) + DVec3::splat(TOLERANCE);
    AABB::from_corners(low.to_array(), high.to_array())
}
impl Spatial {
    fn local_bounds(&self, bounds: AABB<[f64; 3]>) -> AABB<[f64; 3]> {
        AABB::from_corners(
            (DVec3::from_array(bounds.lower()) - self.offset).to_array(),
            (DVec3::from_array(bounds.upper()) - self.offset).to_array(),
        )
    }
    #[tracing::instrument(level = "info", skip_all, name = "spatial.build")]
    pub fn build(model: &Model) -> Result<Self, KernelError> {
        let mut result = Self {
            vertices: RTree::bulk_load(
                model
                    .vertices
                    .iter()
                    .map(|(id, v)| Item {
                        id: *id,
                        bounds: AABB::from_point(v.position.to_array()),
                    })
                    .collect(),
            ),
            ..Self::default()
        };
        for (id, edge) in &model.edges {
            result.add_edge(*id, edge, model)?;
        }
        for (id, face) in &model.faces {
            result.add_face(*id, face, model)?;
        }
        Ok(result)
    }
    pub fn query_edges(&self, bounds: AABB<[f64; 3]>) -> BTreeSet<EdgeId> {
        self.edges
            .locate_in_envelope_intersecting(&expanded(self.local_bounds(bounds)))
            .map(|i| i.id)
            .collect()
    }
    pub fn query_faces(&self, bounds: AABB<[f64; 3]>) -> BTreeSet<FaceId> {
        self.faces
            .locate_in_envelope_intersecting(&expanded(self.local_bounds(bounds)))
            .map(|i| i.id)
            .collect()
    }
    pub fn has_edge(&self, pair: [VertexId; 2]) -> bool {
        self.pairs.contains_key(&pair)
    }
    pub fn vertices_near(&self, p: DVec3) -> Vec<VertexId> {
        let local = p - self.offset;
        let bounds = expanded(expanded(AABB::from_point(local.to_array())));
        self.vertices
            .locate_in_envelope_intersecting(&bounds)
            .map(|item| item.id)
            .collect()
    }
    pub fn add_vertex(&mut self, id: VertexId, p: DVec3) {
        self.vertices.insert(Item {
            id,
            bounds: AABB::from_point((p - self.offset).to_array()),
        });
    }
    pub fn remove_vertex(&mut self, id: VertexId, p: DVec3) {
        remove_item(
            &mut self.vertices,
            id,
            AABB::from_point((p - self.offset).to_array()),
        );
        self.adjacency.remove(&id);
    }
    pub fn edge_bounds(edge: &Edge, model: &Model) -> Result<AABB<[f64; 3]>, KernelError> {
        envelope([
            model.position(edge.vertices[0])?,
            model.position(edge.vertices[1])?,
        ])
        .ok_or(KernelError::MissingEntity)
    }
    pub fn face_bounds(face: &Face, model: &Model) -> Result<AABB<[f64; 3]>, KernelError> {
        envelope(
            face.outer
                .iter()
                .map(|id| model.position(*id))
                .collect::<Result<Vec<_>, _>>()?,
        )
        .ok_or(KernelError::MissingEntity)
    }
    pub fn add_edge(&mut self, id: EdgeId, edge: &Edge, model: &Model) -> Result<(), KernelError> {
        self.edges.insert(Item {
            id,
            bounds: self.local_bounds(Self::edge_bounds(edge, model)?),
        });
        self.pairs.insert(edge.vertices, id);
        for vertex in edge.vertices {
            self.adjacency.entry(vertex).or_default().insert(id);
        }
        Ok(())
    }
    pub fn remove_edge(
        &mut self,
        id: EdgeId,
        edge: &Edge,
        model: &Model,
    ) -> Result<(), KernelError> {
        let bounds = self.local_bounds(Self::edge_bounds(edge, model)?);
        remove_item(&mut self.edges, id, bounds);
        self.pairs.remove(&edge.vertices);
        for vertex in edge.vertices {
            if let Some(edges) = self.adjacency.get_mut(&vertex) {
                edges.remove(&id);
            }
        }
        self.edge_faces.remove(&id);
        Ok(())
    }
    pub fn add_face(&mut self, id: FaceId, face: &Face, model: &Model) -> Result<(), KernelError> {
        self.faces.insert(Item {
            id,
            bounds: self.local_bounds(Self::face_bounds(face, model)?),
        });
        self.face_keys.insert(face_key(face), id);
        let mut edges = BTreeSet::new();
        for ring in std::iter::once(&face.outer).chain(face.holes.iter()) {
            for (a, b) in ring.iter().zip(ring.iter().cycle().skip(1)) {
                let mut pair = [*a, *b];
                pair.sort();
                let edge = *self.pairs.get(&pair).ok_or(KernelError::MissingEntity)?;
                edges.insert(edge);
                self.edge_faces.entry(edge).or_default().insert(id);
            }
        }
        self.face_edges.insert(id, edges);
        Ok(())
    }
    pub fn remove_face(
        &mut self,
        id: FaceId,
        face: &Face,
        model: &Model,
    ) -> Result<(), KernelError> {
        let bounds = self.local_bounds(Self::face_bounds(face, model)?);
        remove_item(&mut self.faces, id, bounds);
        self.face_keys.remove(&face_key(face));
        if let Some(edges) = self.face_edges.remove(&id) {
            for edge in edges {
                if let Some(faces) = self.edge_faces.get_mut(&edge) {
                    faces.remove(&id);
                }
            }
        }
        Ok(())
    }
}

impl Spatial {
    pub fn validate(&self, model: &Model) -> Result<(), KernelError> {
        let expected = Self::build(model)?;
        let vertices = |tree: &RTree<Item<VertexId>>| {
            tree.iter()
                .map(|i| (i.id, i.bounds))
                .collect::<BTreeMap<_, _>>()
        };
        let edges = |tree: &RTree<Item<EdgeId>>| {
            tree.iter()
                .map(|i| (i.id, i.bounds))
                .collect::<BTreeMap<_, _>>()
        };
        let faces = |tree: &RTree<Item<FaceId>>| {
            tree.iter()
                .map(|i| (i.id, i.bounds))
                .collect::<BTreeMap<_, _>>()
        };
        let adjacency = self
            .adjacency
            .iter()
            .filter(|(_, ids)| !ids.is_empty())
            .map(|(id, ids)| (*id, ids.clone()))
            .collect::<BTreeMap<_, _>>();
        let edge_faces = self
            .edge_faces
            .iter()
            .filter(|(_, ids)| !ids.is_empty())
            .map(|(id, ids)| (*id, ids.clone()))
            .collect::<BTreeMap<_, _>>();
        let mut expected = expected;
        expected.offset = self.offset;
        // Compare index envelopes in their shared local frame, not world space.
        if self.offset != DVec3::ZERO {
            let shift = |b: AABB<[f64; 3]>| self.local_bounds(b);
            expected.vertices = RTree::bulk_load(
                expected
                    .vertices
                    .iter()
                    .map(|v| Item {
                        id: v.id,
                        bounds: shift(v.bounds),
                    })
                    .collect(),
            );
            expected.edges = RTree::bulk_load(
                expected
                    .edges
                    .iter()
                    .map(|v| Item {
                        id: v.id,
                        bounds: shift(v.bounds),
                    })
                    .collect(),
            );
            expected.faces = RTree::bulk_load(
                expected
                    .faces
                    .iter()
                    .map(|v| Item {
                        id: v.id,
                        bounds: shift(v.bounds),
                    })
                    .collect(),
            );
        }
        fn same<I: Ord>(a: BTreeMap<I, AABB<[f64; 3]>>, b: BTreeMap<I, AABB<[f64; 3]>>) -> bool {
            a.len() == b.len()
                && a.into_iter().zip(b).all(|((ia, a), (ib, b))| {
                    ia == ib
                        && a.lower()
                            .iter()
                            .zip(b.lower())
                            .chain(a.upper().iter().zip(b.upper()))
                            .all(|(x, y)| (x - y).abs() <= TOLERANCE)
                })
        }
        if !same(vertices(&self.vertices), vertices(&expected.vertices))
            || !same(edges(&self.edges), edges(&expected.edges))
            || !same(faces(&self.faces), faces(&expected.faces))
            || adjacency != expected.adjacency
            || edge_faces != expected.edge_faces
            || self.face_edges != expected.face_edges
            || self.face_keys != expected.face_keys
            || self.pairs != expected.pairs
        {
            return Err(KernelError::InvalidTopology(
                "spatial/adjacency index mismatch".into(),
            ));
        }
        Ok(())
    }
}
