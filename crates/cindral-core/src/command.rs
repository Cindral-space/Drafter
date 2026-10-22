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
    PushPull {
        /// Target surface.
        face: FaceId,
        /// Signed normal distance in meters.
        distance: f64,
    },
    /// Transform selected vertices, or duplicate their induced geometry.
    Transform {
        /// Unique vertices to transform.
        vertices: Vec<VertexId>,
        /// Finite nonsingular affine transform.
        matrix: glam::DMat4,
        /// Duplicate induced edges and faces instead of moving.
        copy: bool,
    },
    /// Draw an inset (positive distance) or outset boundary on a face.
    Offset {
        /// Profile face.
        face: FaceId,
        /// Inset distance in meters, negative for outset.
        distance: f64,
    },
    /// Sweep a face along an ordered polyline path.
    FollowMe {
        /// Profile face.
        face: FaceId,
        /// Ordered sweep stations starting at the profile.
        path: Vec<DVec3>,
    },
    /// Apply a face color.
    PaintFace {
        /// Surfaces to color.
        faces: Vec<FaceId>,
        /// RGBA display color.
        color: [u8; 4],
    },
    /// Assign an optional document material without changing face topology.
    SetFaceMaterial {
        /// Target face IDs.
        faces: Vec<FaceId>,
        /// Optional document material identity.
        material: Option<crate::MaterialId>,
    },
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
            | Self::PaintFace { .. }
            | Self::SetFaceMaterial { .. } => Err(KernelError::InvalidTopology(
                "explicit surface edit requires a prepared transaction".into(),
            )),
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
    preview: Option<crate::transaction::Delta>,
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
            preview: None,
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
        self.cancel_preview()?;
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
    /// Atomically replace the latest completed geometry operation, preserving one undo entry.
    /// Commands refer to the model before that operation. Invalid commands restore it exactly.
    pub fn replace_last(&mut self, command: Command) -> Result<bool, KernelError> {
        self.cancel_preview()?;
        let Some(old) = self.undo.pop() else { return Ok(false); };
        old.delta.apply(&mut self.model, false)?;
        let prepared = crate::transaction::Delta::prepare(&mut self.model, &command, self.high_water);
        let delta = match prepared {
            Ok(delta) if !delta.is_empty() => delta,
            result => {
                old.delta.apply(&mut self.model, true)?;
                self.undo.push(old);
                return result.map(|_| false);
            }
        };
        if let Err(error) = delta.apply(&mut self.model, true) {
            old.delta.apply(&mut self.model, true)?;
            self.undo.push(old);
            return Err(error);
        }
        self.revision += 1;
        let mut dirty = old.delta.changes(self.revision);
        let mut new = delta.changes(self.revision);
        dirty.vertices.append(&mut new.vertices);
        dirty.edges.append(&mut new.edges);
        dirty.faces.append(&mut new.faces);
        dirty.rigid_translation = false;
        self.changes = dirty;
        self.high_water = self.high_water.max(delta.next_id());
        self.undo.push(Entry { delta });
        self.redo.clear();
        Ok(true)
    }
    /// Replace a provisional edit using the same local delta path as a committed command.
    /// No undo entry is recorded until `commit_preview`; cancellation restores the starting geometry.
    #[tracing::instrument(level = "info", skip_all, name = "history.preview")]
    pub fn preview(&mut self, command: Command) -> Result<bool, KernelError> {
        if let Some(delta) = &mut self.preview {
            if let Some(changed) = delta.replace_translation(&mut self.model, &command)? {
                self.revision = self.revision.checked_add(1).ok_or(KernelError::Capacity)?;
                self.changes.revision = self.revision;
                return Ok(changed);
            }
        }
        let mut dirty = crate::ChangeSet::default();
        if let Some(old) = self.preview.take() {
            old.apply(&mut self.model, false)?;
            dirty = old.changes(self.revision);
        }
        let prepared =
            crate::transaction::Delta::prepare(&mut self.model, &command, self.high_water);
        match prepared {
            Ok(delta) => {
                let changed = !delta.is_empty();
                if changed {
                    delta.apply(&mut self.model, true)?;
                }
                let mut new = delta.changes(self.revision);
                if dirty.vertices.is_empty() && dirty.edges.is_empty() && dirty.faces.is_empty() {
                    dirty = new;
                } else {
                    dirty.rigid_translation &= new.rigid_translation;
                    dirty.vertices.append(&mut new.vertices);
                    dirty.edges.append(&mut new.edges);
                    dirty.faces.append(&mut new.faces);
                }
                self.revision = self.revision.checked_add(1).ok_or(KernelError::Capacity)?;
                dirty.revision = self.revision;
                self.changes = dirty;
                self.preview = Some(delta);
                Ok(changed)
            }
            Err(error) => {
                self.revision = self.revision.checked_add(1).ok_or(KernelError::Capacity)?;
                dirty.revision = self.revision;
                self.changes = dirty;
                Err(error)
            }
        }
    }
    /// Typed preview: invalid geometry keeps the previous provisional model unchanged.
    pub fn preview_atomic(&mut self, command: Command) -> Result<bool, KernelError> {
        let previous = self.preview.clone();
        let result = self.preview(command);
        if result.is_err() {
            if let Some(delta) = previous {
                delta.apply(&mut self.model, true)?;
                self.preview = Some(delta);
            }
        }
        result
    }
    /// Cancel the active provisional delta without changing committed history.
    pub fn cancel_preview(&mut self) -> Result<bool, KernelError> {
        let Some(delta) = self.preview.take() else {
            return Ok(false);
        };
        delta.apply(&mut self.model, false)?;
        self.revision = self.revision.checked_add(1).ok_or(KernelError::Capacity)?;
        self.changes = delta.changes(self.revision);
        Ok(!delta.is_empty())
    }
    /// Record the current provisional delta as exactly one undo entry.
    #[tracing::instrument(level = "info", skip_all, name = "history.commit")]
    pub fn commit_preview(&mut self) -> Result<bool, KernelError> {
        let Some(delta) = self.preview.take() else {
            return Ok(false);
        };
        if delta.is_empty() {
            return Ok(false);
        }
        self.high_water = self.high_water.max(delta.next_id());
        self.undo.push(Entry { delta });
        self.redo.clear();
        Ok(true)
    }
    pub(crate) fn from_model(mut model: Model) -> Result<Self, KernelError> {
        model.ensure_spatial()?;
        Ok(Self {
            high_water: model.next_id,
            model,
            ..Self::default()
        })
    }
    pub(crate) fn allocation_floor(&self) -> u64 {
        self.high_water
    }
    pub(crate) fn record_import(&mut self) {
        let delta = crate::transaction::Delta::between(&Model::default(), &self.model, 1);
        self.revision = 1;
        self.changes = delta.changes(1);
        self.undo.push(Entry { delta });
    }
    pub(crate) fn replace_model(&mut self, mut model: Model) -> Result<bool, KernelError> {
        self.cancel_preview()?;
        model.spatial = None;
        model.next_id = model.next_id.max(self.high_water);
        model.validate()?;
        let delta = crate::transaction::Delta::between(&self.model, &model, self.model.next_id);
        if delta.is_empty() {
            return Ok(false);
        }
        delta.apply(&mut self.model, true)?;
        self.high_water = self.high_water.max(delta.next_id());
        self.revision += 1;
        self.changes = delta.changes(self.revision);
        self.undo.push(Entry { delta });
        self.redo.clear();
        Ok(true)
    }
    pub(crate) fn replace_import_model(&mut self, mut model: Model) -> Result<bool, KernelError> {
        self.cancel_preview()?;
        model.spatial = None;
        model.next_id = model.next_id.max(self.high_water);
        model.validate_mesh(false)?;
        let delta = crate::transaction::Delta::between(&self.model, &model, self.model.next_id);
        if delta.is_empty() {
            return Ok(false);
        }
        delta.apply(&mut self.model, true)?;
        self.high_water = self.high_water.max(delta.next_id());
        self.revision += 1;
        self.changes = delta.changes(self.revision);
        self.undo.push(Entry { delta });
        self.redo.clear();
        Ok(true)
    }
    /// Dirty entities from the most recent successful mutation, undo or redo.
    pub fn changes(&self) -> &crate::ChangeSet {
        &self.changes
    }
    /// Restore the state before the last successful command.
    pub fn undo(&mut self) -> Result<bool, KernelError> {
        self.cancel_preview()?;
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
        self.cancel_preview()?;
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
    /// Construct a known-topology benchmark model without quadratic full validation.
    /// Only deterministic benchmark fixtures should call this feature-gated entry point.
    pub fn benchmark_from_data(data: crate::GeometryData) -> Result<Self, KernelError> {
        Self::benchmark_seed(Model::from_values(data)?)
    }
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
