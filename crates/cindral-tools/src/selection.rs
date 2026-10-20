//! Rendering-independent selection queries.
use cindral_core::{EdgeId, FaceId, VertexId};
use std::collections::BTreeSet;
/// Picked model entity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    /// Vertex hit.
    Vertex(VertexId),
    /// Edge hit.
    Edge(EdgeId),
    /// Face hit.
    Face(FaceId),
}
/// Selected entities in the root model.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Selection {
    /// Explicit vertices.
    pub vertices: BTreeSet<VertexId>,
    /// Selected edges.
    pub edges: BTreeSet<EdgeId>,
    /// Selected faces.
    pub faces: BTreeSet<FaceId>,
}
impl Selection {
    /// Resolve selected face/edge boundaries to unique vertex IDs.
    pub fn vertex_ids(&self, model: &cindral_core::Model) -> Vec<VertexId> {
        let mut ids = self.vertices.clone();
        for id in &self.edges {
            if let Some(edge) = model.edges().get(id) {
                ids.extend(edge.vertices);
            }
        }
        for id in &self.faces {
            if let Some(face) = model.faces().get(id) {
                ids.extend(&face.outer);
                for hole in &face.holes {
                    ids.extend(hole);
                }
            }
        }
        ids.into_iter().collect()
    }
}
