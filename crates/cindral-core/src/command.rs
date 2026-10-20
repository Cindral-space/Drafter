use glam::DVec3;

use crate::{EdgeId, KernelError, Model};

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
    /// Execute several mutations as one transaction and one history entry.
    Batch(Vec<Command>),
}

impl Command {
    fn apply(&self, model: &mut Model, depth: usize) -> Result<bool, KernelError> {
        if depth > 64 {
            return Err(KernelError::BatchDepth);
        }
        match self {
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
    before: Model,
    after: Model,
}

/// Transactional model owner and undo/redo history.
///
/// M2 uses validated snapshots for reliable inverses. IDs are never reused within
/// this history, including after undo followed by a new branch of edits. Revision
/// increases on successful changes, undo and redo, so render caches can invalidate.
#[derive(Debug, Clone)]
pub struct History {
    model: Model,
    undo: Vec<Entry>,
    redo: Vec<Entry>,
    high_water: u64,
    revision: u64,
}

impl Default for History {
    fn default() -> Self {
        Self {
            model: Model::default(),
            undo: Vec::new(),
            redo: Vec::new(),
            high_water: 1,
            revision: 0,
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
        let mut candidate = self.model.clone();
        candidate.next_id = self.high_water;
        if command.apply(&mut candidate, 0)? {
            candidate.rebuild_faces()?;
        }
        candidate.validate()?;
        if candidate.same_geometry(&self.model) {
            return Ok(false);
        }
        let revision = self.revision.checked_add(1).ok_or(KernelError::Capacity)?;
        self.high_water = self.high_water.max(candidate.next_id);
        let before = std::mem::replace(&mut self.model, candidate);
        self.undo.push(Entry {
            before,
            after: self.model.clone(),
        });
        self.redo.clear();
        self.revision = revision;
        Ok(true)
    }
    /// Restore the state before the last successful command.
    pub fn undo(&mut self) -> Result<bool, KernelError> {
        if self.undo.is_empty() {
            return Ok(false);
        }
        let revision = self.revision.checked_add(1).ok_or(KernelError::Capacity)?;
        if let Some(entry) = self.undo.pop() {
            self.model = entry.before.clone();
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
            self.model = entry.after.clone();
            self.undo.push(entry);
            self.revision = revision;
        }
        Ok(true)
    }
}
