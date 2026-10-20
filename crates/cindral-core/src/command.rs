use glam::DVec3;

use crate::{EdgeId, FaceId, KernelError, Model, VertexId};

/// The supported document mutations. A batch is one atomic undo step.
#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    /// Draw an edge, splitting existing edges at intersections and overlaps.
    AddEdge {
        /// Start position in meters.
        start: DVec3,
        /// End position in meters.
        end: DVec3,
    },
    /// Draw an ordered chain; a closed chain automatically creates planar regions.
    AddPolyline {
        /// Positions in drawing order (do not repeat the first point when closed).
        points: Vec<DVec3>,
        /// Join the last point to the first.
        closed: bool,
    },
    /// Erase edges and rebuild/heal the remaining topology.
    EraseEdges(Vec<EdgeId>),
    /// Set independent soft and smooth attributes on existing edges.
    SetEdgeStyle {
        /// Target edges.
        edges: Vec<EdgeId>,
        /// Hide the edge in ordinary shaded views.
        soft: bool,
        /// Interpolate shading across the edge.
        smooth: bool,
    },
    /// Extrude a planar face along its canonical normal by signed meters.
    PushPull { face: FaceId, distance: f64 },
    /// Transform selected vertices, or duplicate their induced geometry.
    Transform {
        vertices: Vec<VertexId>,
        matrix: glam::DMat4,
        copy: bool,
    },
    /// Draw an inset (positive distance) or outset boundary on a face.
    Offset { face: FaceId, distance: f64 },
    /// Sweep a face along an ordered polyline path.
    FollowMe { face: FaceId, path: Vec<DVec3> },
    /// Apply a face color.
    PaintFace { faces: Vec<FaceId>, color: [u8; 4] },
    /// Execute several mutations as one transaction and one history entry.
    Batch(Vec<Command>),
}

impl Command {
    pub(crate) fn apply(&self, model: &mut Model, depth: usize) -> Result<bool, KernelError> {
        if depth > 64 {
            return Err(KernelError::BatchDepth);
        }
        match self {
            Self::PushPull { .. }
            | Self::Transform { .. }
            | Self::Offset { .. }
            | Self::FollowMe { .. }
            | Self::PaintFace { .. } => {
                Err(KernelError::InvalidTopology("M4 operation pending".into()))
            }
            Self::AddEdge { start, end } => {
                model.insert_segment(*start, *end)?;
                Ok(true)
            }
            Self::AddPolyline { points, closed } => {
                if points.len() < if *closed { 3 } else { 2 } {
                    return Err(KernelError::InvalidPolyline);
                }
                for pair in points.windows(2) {
                    model.insert_segment(pair[0], pair[1])?;
                }
                if *closed {
                    model.insert_segment(points[points.len() - 1], points[0])?;
                }
                Ok(true)
            }
            Self::EraseEdges(edges) => {
                for id in edges {
                    if !model.edges.contains_key(id) {
                        return Err(KernelError::MissingEntity);
                    }
                }
                for id in edges {
                    model.edges.remove(id);
                }
                if !edges.is_empty() {
                    model.heal()?;
                }
                Ok(!edges.is_empty())
            }
            Self::SetEdgeStyle {
                edges,
                soft,
                smooth,
            } => {
                for id in edges {
                    let edge = model.edges.get_mut(id).ok_or(KernelError::MissingEntity)?;
                    edge.soft = *soft;
                    edge.smooth = *smooth;
                }
                Ok(false)
            }
            Self::Batch(commands) => {
                let mut dirty = false;
                for command in commands {
                    dirty |= command.apply(model, depth + 1)?;
                }
                Ok(dirty)
            }
        }
    }
}

#[derive(Debug, Clone)]
struct Entry {
    delta: crate::transaction::Delta,
}

/// Transactional model owner and undo/redo history.
///
/// Edits use locally validated cavities and reversible entity deltas. IDs are never reused within
/// this history, including after undo followed by a new branch of edits. Revision
/// increases on successful changes, undo and redo, so render caches can invalidate.
#[derive(Debug, Clone)]
pub struct History {
    model: Model,
    undo: Vec<Entry>,
    redo: Vec<Entry>,
    high_water: u64,
    revision: u64,
    changes: crate::ChangeSet,
}

impl Default for History {
    fn default() -> Self {
        Self {
            model: Model::default(),
            undo: Vec::new(),
            redo: Vec::new(),
            high_water: 1,
            revision: 0,
            changes: crate::ChangeSet::default(),
        }
    }
}

impl History {
    /// Construct an empty document and history.
    pub fn new() -> Self {
        Self::default()
    }
    /// Current read-only document.
    pub fn model(&self) -> &Model {
        &self.model
    }
    /// Monotonic cache invalidation revision.
    pub fn revision(&self) -> u64 {
        self.revision
    }
    /// Available undo steps.
    pub fn undo_depth(&self) -> usize {
        self.undo.len()
    }
    /// Available redo steps.
    pub fn redo_depth(&self) -> usize {
        self.redo.len()
    }
    /// Validate a candidate mutation and commit it atomically.
    /// Returns `false` for a no-op, which preserves redo history and revision.
    pub fn execute(&mut self, command: Command) -> Result<bool, KernelError> {
        let delta = crate::transaction::Delta::prepare(&mut self.model, &command, self.high_water)?;
        if delta.is_empty() {
            return Ok(false);
        }
        let revision = self.revision.checked_add(1).ok_or(KernelError::Capacity)?;
        delta.apply(&mut self.model, true)?;
        self.high_water = self.high_water.max(delta.next_id());
        self.changes = delta.changes(revision);
        self.undo.push(Entry { delta });
        self.redo.clear();
        self.revision = revision;
        Ok(true)
    }
    /// Dirty entities from the most recent successful mutation, undo or redo.
    pub fn changes(&self) -> &crate::ChangeSet {
        &self.changes
    }
    /// Restore the state before the last successful command.
    pub fn undo(&mut self) -> Result<bool, KernelError> {
        if self.undo.is_empty() {
            return Ok(false);
        }
        let revision = self.revision.checked_add(1).ok_or(KernelError::Capacity)?;
        if let Some(entry) = self.undo.pop() {
            entry.delta.apply(&mut self.model, false)?;
            self.changes = entry.delta.changes(revision);
            self.redo.push(entry);
            self.revision = revision;
        }
        Ok(true)
    }
    /// Reapply the most recently undone command.
    pub fn redo(&mut self) -> Result<bool, KernelError> {
        if self.redo.is_empty() {
            return Ok(false);
        }
        let revision = self.revision.checked_add(1).ok_or(KernelError::Capacity)?;
        if let Some(entry) = self.redo.pop() {
            entry.delta.apply(&mut self.model, true)?;
            self.changes = entry.delta.changes(revision);
            self.undo.push(entry);
            self.revision = revision;
        }
        Ok(true)
    }
}

#[cfg(feature = "bench-fixtures")]
impl History {
    pub(crate) fn benchmark_seed(mut model: Model) -> Result<Self, KernelError> {
        model.ensure_spatial()?;
        Ok(Self {
            high_water: model.next_id,
            model,
            ..Self::default()
        })
    }
}

#[cfg(test)]
impl History {
    pub(crate) fn highest_test_id(&self) -> u64 {
        self.high_water
    }
}
