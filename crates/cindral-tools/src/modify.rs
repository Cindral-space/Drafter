//! State machine session for interactive modify tools.
//!
//! Provides the bounded [`ModifySession`] state machine, [`ModifyKind`],
//! and construction [`Guide`] representation.

use std::collections::{BTreeMap, BTreeSet};

use cindral_core::{
    Command, Edge, EdgeId, Face, FaceId, Model, VertexId, MAX_COORDINATE, TOLERANCE,
};
use glam::{DMat4, DVec3};

use crate::measurements::{self, Measurement, MeasurementSettings, MeasurementTool, Quantity};
use crate::selection::{Hit, Selection};
use crate::session::plane_basis_for;
use crate::ToolError;

/// Supported interactive modify tools.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ModifyKind {
    /// Select tool (idle selection mode, produces no geometry mutations).
    Select,
    /// Extrude/recess a planar face along its canonical normal.
    PushPull,
    /// Translate selected entities in world space.
    Move,
    /// Duplicate and translate selected entities.
    Copy,
    /// Rotate selected entities about an axis through an anchor point.
    Rotate,
    /// Uniformly scale selected entities centered at an anchor point.
    Scale,
    /// Inset or outset a planar face boundary.
    Offset,
    /// Sweep a face profile along an ordered path.
    FollowMe,
    /// Erase the clicked face or edge.
    Eraser,
    /// Measure distances and place infinite construction guide lines without mutating model geometry.
    TapeMeasure,
    /// Apply colors to planar faces.
    PaintBucket,
}

/// An infinite construction guide line defined by an origin point and direction vector.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Guide {
    /// An arbitrary point lying on the guide line.
    pub origin: DVec3,
    /// Normalized unit direction vector of the guide line.
    pub direction: DVec3,
}

impl Guide {
    /// Construct a new guide line with the given origin and direction.
    pub fn new(origin: DVec3, direction: DVec3) -> Self {
        Self { origin, direction }
    }
}

/// Validate that a position or vector is finite and within the supported coordinate range.
fn validate_point(point: DVec3) -> Result<(), ToolError> {
    if !point.is_finite() || point.abs().max_element() > MAX_COORDINATE {
        Err(ToolError::InvalidPoint)
    } else {
        Ok(())
    }
}

/// Check if a face outer loop or holes contain a given vertex.
fn face_contains_vertex(face: &Face, vid: VertexId) -> bool {
    face.outer.contains(&vid) || face.holes.iter().any(|hole| hole.contains(&vid))
}

/// Check if a face outer loop or holes contain a given topological edge.
fn face_contains_edge(face: &Face, edge: &Edge) -> bool {
    let [v0, v1] = edge.vertices;
    let has_edge_in_ring = |ring: &[VertexId]| {
        if ring.len() < 2 {
            return false;
        }
        ring.iter()
            .zip(ring.iter().cycle().skip(1))
            .take(ring.len())
            .any(|(&a, &b)| (a == v0 && b == v1) || (a == v1 && b == v0))
    };
    has_edge_in_ring(&face.outer) || face.holes.iter().any(|hole| has_edge_in_ring(hole))
}

/// Resolve target face for PushPull or Offset from selection and hit.
fn resolve_face_for_pushpull_or_offset(
    model: &Model,
    selection: &Selection,
    hit: Option<Hit>,
) -> Option<FaceId> {
    // 1. Direct face hit
    if let Some(Hit::Face(fid)) = hit {
        if model.faces().contains_key(&fid) {
            return Some(fid);
        }
    }

    // 2. Edge hit -> selected face containing edge, then any incident face
    if let Some(Hit::Edge(eid)) = hit {
        if let Some(edge) = model.edges().get(&eid) {
            if let Some(&fid) = selection.faces.iter().find(|&&fid| {
                model
                    .faces()
                    .get(&fid)
                    .is_some_and(|f| face_contains_edge(f, edge))
            }) {
                return Some(fid);
            }
            if let Some((&fid, _)) = model
                .faces()
                .iter()
                .find(|(_, f)| face_contains_edge(f, edge))
            {
                return Some(fid);
            }
        }
    }

    // 3. Vertex hit -> selected face containing vertex, then any incident face
    if let Some(Hit::Vertex(vid)) = hit {
        if let Some(&fid) = selection.faces.iter().find(|&&fid| {
            model
                .faces()
                .get(&fid)
                .is_some_and(|f| face_contains_vertex(f, vid))
        }) {
            return Some(fid);
        }
        if let Some((&fid, _)) = model
            .faces()
            .iter()
            .find(|(_, f)| face_contains_vertex(f, vid))
        {
            return Some(fid);
        }
    }

    // 4. Fallback to first selected face
    if let Some(&fid) = selection.faces.iter().next() {
        if model.faces().contains_key(&fid) {
            return Some(fid);
        }
    }

    None
}

/// Resolve unique vertex IDs for affine transformations from selection and hit.
fn resolve_vertices_for_transform(
    model: &Model,
    selection: &Selection,
    hit: Option<Hit>,
) -> Vec<VertexId> {
    let mut vertices = BTreeSet::new();
    vertices.extend(selection.vertex_ids(model));

    match hit {
        Some(Hit::Vertex(vid)) => {
            if model.vertices().contains_key(&vid) {
                vertices.insert(vid);
            }
        }
        Some(Hit::Edge(eid)) => {
            if let Some(edge) = model.edges().get(&eid) {
                vertices.extend(edge.vertices);
            }
        }
        Some(Hit::Face(fid)) => {
            if let Some(face) = model.faces().get(&fid) {
                vertices.extend(&face.outer);
                for hole in &face.holes {
                    vertices.extend(hole);
                }
            }
        }
        None => {}
    }

    vertices.into_iter().collect()
}

/// Resolve face IDs for the PaintBucket tool from selection and hit.
fn resolve_faces_for_paint(model: &Model, selection: &Selection, hit: Option<Hit>) -> Vec<FaceId> {
    let mut faces = BTreeSet::new();
    faces.extend(&selection.faces);

    match hit {
        Some(Hit::Face(fid)) => {
            if model.faces().contains_key(&fid) {
                faces.insert(fid);
            }
        }
        Some(Hit::Edge(eid)) => {
            if let Some(edge) = model.edges().get(&eid) {
                for (&fid, face) in model.faces() {
                    if face_contains_edge(face, edge) {
                        faces.insert(fid);
                    }
                }
            }
        }
        Some(Hit::Vertex(vid)) => {
            for (&fid, face) in model.faces() {
                if face_contains_vertex(face, vid) {
                    faces.insert(fid);
                }
            }
        }
        None => {}
    }

    faces.into_iter().collect()
}

/// Try to order selected edges into a single connected polyline path.
fn resolve_ordered_path_from_edges(model: &Model, edges: &BTreeSet<EdgeId>) -> Option<Vec<DVec3>> {
    if edges.is_empty() {
        return None;
    }

    let mut adj: BTreeMap<VertexId, Vec<(EdgeId, VertexId)>> = BTreeMap::new();
    for &eid in edges {
        let edge = model.edges().get(&eid)?;
        adj.entry(edge.vertices[0])
            .or_default()
            .push((eid, edge.vertices[1]));
        adj.entry(edge.vertices[1])
            .or_default()
            .push((eid, edge.vertices[0]));
    }

    // Branching paths (vertex degree > 2) cannot form a single ordered path
    if adj.values().any(|neighbors| neighbors.len() > 2) {
        return None;
    }

    // Prefer starting at a degree 1 endpoint; if closed loop (all degree 2), pick first
    let start_vid = adj
        .iter()
        .find(|(_, neighbors)| neighbors.len() == 1)
        .map(|(&vid, _)| vid)
        .or_else(|| adj.keys().next().copied())?;

    let mut visited_edges = BTreeSet::new();
    let mut ordered_vertices = vec![start_vid];
    let mut current_vid = start_vid;

    while visited_edges.len() < edges.len() {
        let Some(neighbors) = adj.get(&current_vid) else {
            break;
        };
        if let Some(&(eid, next_vid)) = neighbors
            .iter()
            .find(|(eid, _)| !visited_edges.contains(eid))
        {
            visited_edges.insert(eid);
            ordered_vertices.push(next_vid);
            current_vid = next_vid;
        } else {
            break;
        }
    }

    if visited_edges.len() != edges.len() || ordered_vertices.len() < 2 {
        return None;
    }

    let mut positions = Vec::with_capacity(ordered_vertices.len());
    for vid in ordered_vertices {
        let pos = model.vertices().get(&vid)?.position;
        positions.push(pos);
    }
    Some(positions)
}

/// Bounded interactive state machine managing modify operations and absolute command previews.
///
/// Integrators own document history, preview committing, and rendering; `ModifySession`
/// holds no clones of `Model` or `History`.
///
/// # Rotation Angle Convention
/// - At `begin(model, selection, hit, point, axis)`:
///   - `point` is established as the rotation center (anchor $A$).
///   - `axis` is validated and normalized to unit length normal $N$.
///   - The initial reference direction $U$ is established as the canonical unit basis vector
///     orthogonal to $N$ (`plane_basis_for(N).0`).
/// - At `update(point)`:
///   - Vector $(P - A)$ is projected onto the plane perpendicular to $N$: $V = (P - A) - N ((P - A) \cdot N)$.
///   - The angle $\theta$ is measured counterclockwise around $N$ from $U$:
///     $\theta = \text{atan2}(V \cdot (N \times U), V \cdot U)$.
///   - If $|V| \le \text{TOLERANCE}$, $\theta = 0.0$.
///   - The resulting affine transform is centered around anchor $A$:
///     $M = T(A) \cdot R(N, \theta) \cdot T(-A)$.
///   - Repeated updates are always computed directly from the original anchor $A$ and initial reference $U$.
/// - At `measure(text)`:
///   - Parses typed degrees (e.g. `"45"`, `"45 deg"`, `"45°"`, `"-90"`).
///   - Produces the absolute affine transform around $A$ by the parsed angle.
#[derive(Debug, Clone)]
pub struct ModifySession {
    kind: ModifyKind,
    active: bool,
    rotation_copy: bool,
    completed: Option<Box<ModifySession>>,
    anchor: Option<DVec3>,
    axis: DVec3,
    reference_dir: Option<DVec3>,
    reference_radius: f64,
    scale_extents: DVec3,
    scale_factors: Option<(DVec3, usize)>,
    tape_baseline: Option<f64>,
    resize_factor: Option<f64>,
    current_pointer: Option<DVec3>,
    measurement: Option<f64>,
    measurement_direction: Option<DVec3>,
    last_command: Option<Command>,
    color: [u8; 4],
    guides: Vec<Guide>,

    target_vertices: Vec<VertexId>,
    target_face: Option<FaceId>,
    target_edges: Vec<EdgeId>,
    target_faces: Vec<FaceId>,
    preselected_path: Vec<DVec3>,
    face_centroid: Option<DVec3>,
}

impl ModifySession {
    /// Construct a new modify session for the specified tool kind.
    pub fn new(kind: ModifyKind) -> Self {
        Self {
            kind,
            active: false,
            rotation_copy: false,
            completed: None,
            anchor: None,
            axis: DVec3::Z,
            reference_dir: None,
            reference_radius: 1.,
            scale_extents: DVec3::ONE,
            scale_factors: None,
            tape_baseline: None,
            resize_factor: None,
            current_pointer: None,
            measurement: None,
            measurement_direction: None,
            last_command: None,
            color: [255, 255, 255, 255],
            guides: Vec::new(),
            target_vertices: Vec::new(),
            target_face: None,
            target_edges: Vec::new(),
            target_faces: Vec::new(),
            preselected_path: Vec::new(),
            face_centroid: None,
        }
    }

    /// The currently configured tool kind.
    pub fn kind(&self) -> ModifyKind {
        self.kind
    }

    /// Change the tool kind, resetting any active interaction while preserving guide lines.
    pub fn set_kind(&mut self, kind: ModifyKind) {
        self.cancel();
        self.kind = kind;
    }

    /// True if an interaction operation is currently underway.
    pub fn active(&self) -> bool {
        self.active
    }

    /// Begin Rotate with the already collected transform targets, including container-only selections.
    pub fn begin_rotation(&mut self, vertices: Vec<VertexId>, center: DVec3, normal: DVec3) -> Result<(), ToolError> {
        validate_point(center)?;
        let normal = normal.try_normalize().ok_or(ToolError::Degenerate)?;
        self.cancel();
        self.kind = ModifyKind::Rotate;
        self.active = true;
        self.anchor = Some(center);
        self.axis = normal;
        self.reference_dir = Some(plane_basis_for(normal).0);
        self.target_vertices = vertices;
        Ok(())
    }

    /// Set copy mode without altering Rotate anchors or targets.
    pub fn set_rotation_copy(&mut self, copy: bool) { self.rotation_copy = copy; }
    /// Rotation baseline and radius retained for protractor feedback.
    pub fn rotation_reference(&self) -> Option<(DVec3, f64)> {
        self.reference_dir.map(|d| (d, self.reference_radius))
    }
    /// Normalized interaction axis fixed at begin.
    pub fn axis(&self) -> DVec3 {
        self.axis
    }
    /// The active anchor point, if established.
    pub fn anchor(&self) -> Option<DVec3> {
        self.anchor
    }

    /// Cancel the current in-progress interaction, resetting interaction state while preserving guide lines.
    pub fn cancel(&mut self) {
        self.completed = None;
        self.active = false;
        self.rotation_copy = false;
        self.anchor = None;
        self.axis = DVec3::Z;
        self.reference_dir = None;
        self.reference_radius = 1.;
        self.tape_baseline = None;
        self.resize_factor = None;
        self.scale_factors = None;
        self.current_pointer = None;
        self.measurement = None;
        self.measurement_direction = None;
        self.last_command = None;
        self.target_vertices.clear();
        self.target_face = None;
        self.target_edges.clear();
        self.target_faces.clear();
        self.preselected_path.clear();
        self.face_centroid = None;
    }

    /// All persistent construction guide lines.
    pub fn guides(&self) -> &[Guide] {
        &self.guides
    }

    /// Exchange local construction guides when the editor changes definition context.
    pub fn replace_guides(&mut self, guides: Vec<Guide>) -> Vec<Guide> {
        std::mem::replace(&mut self.guides, guides)
    }

    /// The most recent numerical measurement (distance in meters, angle in degrees, or scale factor).
    pub fn measurement(&self) -> Option<f64> {
        self.completed
            .as_deref()
            .and_then(|s| s.measurement)
            .or(self.measurement)
    }

    /// Set the color for face painting.
    pub fn set_color(&mut self, color: [u8; 4]) {
        self.color = color;
    }

    /// The current paint color.
    pub fn color(&self) -> [u8; 4] {
        self.color
    }

    /// Add a construction guide line through the anchor along the direction to the prospective pointer.
    pub fn add_guide(&mut self) -> Result<(), ToolError> {
        let anchor = self.anchor.ok_or(ToolError::NotReady)?;
        let pointer = self.current_pointer.ok_or(ToolError::NotReady)?;
        let diff = pointer - anchor;
        if diff.length_squared() <= TOLERANCE * TOLERANCE {
            return Err(ToolError::Degenerate);
        }
        let direction = diff.try_normalize().ok_or(ToolError::Degenerate)?;
        let guide = Guide {
            origin: anchor,
            direction,
        };
        if self.guides.last() != Some(&guide) {
            self.guides.push(guide);
        }
        Ok(())
    }

    /// Begin a modify operation at the given anchor point and axis.
    ///
    /// Resolves target entities from selection and hit without taking clones of `Model`.
    #[tracing::instrument(level = "info", skip_all, name = "tool.begin")]
    pub fn begin(
        &mut self,
        model: &Model,
        selection: &Selection,
        hit: Option<Hit>,
        point: DVec3,
        axis: DVec3,
    ) -> Result<(), ToolError> {
        validate_point(point)?;
        if !axis.is_finite() {
            return Err(ToolError::InvalidPoint);
        }

        let mut target_vertices = Vec::new();
        let mut target_face = None;
        let mut target_edges = Vec::new();
        let mut target_faces = Vec::new();
        let mut preselected_path = Vec::new();
        let mut face_centroid = None;

        let norm_axis = match self.kind {
            ModifyKind::Select | ModifyKind::TapeMeasure => {
                axis.try_normalize().unwrap_or(DVec3::Z)
            }
            ModifyKind::PushPull => {
                let face_id = resolve_face_for_pushpull_or_offset(model, selection, hit)
                    .ok_or(ToolError::NoEditableFace)?;
                target_face = Some(face_id);
                let face = model.faces().get(&face_id).ok_or(ToolError::NotReady)?;
                face.plane.normal
            }
            ModifyKind::Offset => {
                let face_id = resolve_face_for_pushpull_or_offset(model, selection, hit)
                    .ok_or(ToolError::NoEditableFace)?;
                let face = model.faces().get(&face_id).ok_or(ToolError::NotReady)?;
                let mut sum = DVec3::ZERO;
                let mut count = 0usize;
                for &vid in &face.outer {
                    if let Some(v) = model.vertices().get(&vid) {
                        sum += v.position;
                        count += 1;
                    }
                }
                if count > 0 {
                    face_centroid = Some(sum / (count as f64));
                }
                target_face = Some(face_id);
                axis.try_normalize().unwrap_or(face.plane.normal)
            }
            ModifyKind::Move | ModifyKind::Copy | ModifyKind::Scale => {
                let vertices = resolve_vertices_for_transform(model, selection, hit);
                if vertices.is_empty() {
                    return Err(ToolError::NotReady);
                }
                target_vertices = vertices;
                axis.try_normalize().unwrap_or(DVec3::Z)
            }
            ModifyKind::Rotate => {
                let vertices = resolve_vertices_for_transform(model, selection, hit);
                if vertices.is_empty() {
                    return Err(ToolError::NotReady);
                }
                let norm = axis.try_normalize().ok_or(ToolError::Degenerate)?;
                target_vertices = vertices;
                norm
            }
            ModifyKind::FollowMe => {
                let face_id = resolve_face_for_pushpull_or_offset(model, selection, hit)
                    .ok_or(ToolError::NoEditableFace)?;
                target_face = Some(face_id);
                preselected_path =
                    resolve_ordered_path_from_edges(model, &selection.edges).unwrap_or_default();
                axis.try_normalize().unwrap_or(DVec3::Z)
            }
            ModifyKind::Eraser => {
                // Eraser is a pointer action: selection must never broaden the target.
                match hit {
                    Some(Hit::Edge(id)) if model.edges().contains_key(&id) => {
                        target_edges.push(id);
                    }
                    Some(Hit::Face(id)) if model.faces().contains_key(&id) => {
                        target_faces.push(id);
                    }
                    _ => return Err(ToolError::NotReady),
                }
                axis.try_normalize().unwrap_or(DVec3::Z)
            }
            ModifyKind::PaintBucket => {
                let faces = resolve_faces_for_paint(model, selection, hit);
                if faces.is_empty() {
                    return Err(ToolError::NotReady);
                }
                target_faces = faces;
                axis.try_normalize().unwrap_or(DVec3::Z)
            }
        };

        let reference_dir = if self.kind == ModifyKind::Rotate {
            Some(plane_basis_for(norm_axis).0)
        } else {
            None
        };

        // Reset previous interaction state
        self.cancel();

        self.active = true;
        self.anchor = Some(point);
        self.axis = norm_axis;
        self.reference_dir = reference_dir;
        if self.kind == ModifyKind::TapeMeasure {
            target_vertices = model.vertices().keys().copied().collect();
        }
        let mut min = DVec3::splat(f64::INFINITY);
        let mut max = DVec3::splat(f64::NEG_INFINITY);
        for id in &target_vertices {
            if let Some(v) = model.vertices().get(id) {
                min = min.min(v.position);
                max = max.max(v.position);
            }
        }
        self.scale_extents = max - min;
        self.target_vertices = target_vertices;
        self.target_face = target_face;
        self.target_edges = target_edges;
        self.target_faces = target_faces;
        self.preselected_path = preselected_path;
        self.face_centroid = face_centroid;

        Ok(())
    }

    /// Set a rotation/scale handle relative to the initial pivot without changing selected entities.
    pub fn set_reference(&mut self, point: DVec3) -> Result<(), ToolError> {
        validate_point(point)?;
        let anchor = self.anchor.ok_or(ToolError::NotReady)?;
        let delta = point - anchor;
        let in_plane = delta - self.axis * delta.dot(self.axis);
        let radius = in_plane.length();
        if radius <= TOLERANCE {
            return Err(ToolError::Degenerate);
        }
        self.reference_dir = Some(in_plane / radius);
        self.reference_radius = radius;
        Ok(())
    }
    /// Sweep Rotate with the document increment and a world-space protractor radius.
    pub fn update_rotation(&mut self, point: DVec3, radius: f64, increment: u16) -> Result<Option<Command>, ToolError> {
        let center = self.anchor.ok_or(ToolError::NotReady)?;
        let reference = self.reference_dir.ok_or(ToolError::NotReady)?;
        let offset = point-center;
        let planar = offset-self.axis*offset.dot(self.axis);
        let angle = crate::rotation::snap(crate::rotation::angle(self.axis,reference,planar),planar.length(),radius,increment);
        let point = center + (reference*angle.cos()+self.axis.cross(reference)*angle.sin())*planar.length();
        self.update(point)
    }
    /// Produce an absolute preview mutation command from the initial anchor to the prospective point.
    #[tracing::instrument(level = "info", skip_all, name = "tool.update")]
    pub fn update(&mut self, point: DVec3) -> Result<Option<Command>, ToolError> {
        let _phase = cindral_core::profile::Scope::new(2);
        if !self.active {
            return Err(ToolError::NotReady);
        }
        validate_point(point)?;
        let anchor = self.anchor.ok_or(ToolError::NotReady)?;

        match self.kind {
            ModifyKind::Select => {
                self.current_pointer = Some(point);
                Ok(None)
            }
            ModifyKind::TapeMeasure => {
                self.current_pointer = Some(point);
                let dist = (point - anchor).length();
                self.measurement = Some(dist);
                Ok(None)
            }
            ModifyKind::PushPull => {
                let face = self.target_face.ok_or(ToolError::NotReady)?;
                let distance = (point - anchor).dot(self.axis);
                self.current_pointer = Some(point);
                self.measurement = Some(distance);
                let cmd = Command::PushPull { face, distance };
                if self.last_command.as_ref() == Some(&cmd) {
                    return Ok(None);
                }
                self.last_command = Some(cmd.clone());
                Ok(Some(cmd))
            }
            ModifyKind::Move => {
                self.current_pointer = Some(point);
                let translation = point - anchor;
                self.measurement = Some(translation.length());
                let matrix = DMat4::from_translation(translation);
                let cmd = Command::Transform {
                    vertices: self.target_vertices.clone(),
                    matrix,
                    copy: false,
                };
                self.last_command = Some(cmd.clone());
                Ok(Some(cmd))
            }
            ModifyKind::Copy => {
                self.current_pointer = Some(point);
                let translation = point - anchor;
                self.measurement = Some(translation.length());
                let matrix = DMat4::from_translation(translation);
                let cmd = Command::Transform {
                    vertices: self.target_vertices.clone(),
                    matrix,
                    copy: true,
                };
                self.last_command = Some(cmd.clone());
                Ok(Some(cmd))
            }
            ModifyKind::Rotate => {
                self.current_pointer = Some(point);
                let axis = self.axis;
                let u_ref = self
                    .reference_dir
                    .unwrap_or_else(|| plane_basis_for(axis).0);
                let diff = point - anchor;
                let in_plane = diff - axis * diff.dot(axis);
                let angle_rad = if in_plane.length() > TOLERANCE {
                    crate::rotation::angle(axis, u_ref, in_plane)
                } else {
                    0.0
                };
                let angle_deg = angle_rad.to_degrees();
                self.measurement = Some(angle_deg);
                let matrix = crate::rotation::matrix(anchor, axis, angle_rad);
                let cmd = Command::Transform {
                    vertices: self.target_vertices.clone(),
                    matrix,
                    copy: self.rotation_copy,
                };
                self.last_command = Some(cmd.clone());
                Ok(Some(cmd))
            }
            ModifyKind::Scale => {
                self.current_pointer = Some(point);
                let delta = point - anchor;
                let dist = (delta - self.axis * delta.dot(self.axis)).length();
                if dist <= TOLERANCE {
                    return Err(ToolError::Degenerate);
                }
                let s = dist / self.reference_radius;
                self.measurement = Some(s);
                let scale_mat = DMat4::from_scale(DVec3::splat(s));
                let matrix =
                    DMat4::from_translation(anchor) * scale_mat * DMat4::from_translation(-anchor);
                let cmd = Command::Transform {
                    vertices: self.target_vertices.clone(),
                    matrix,
                    copy: false,
                };
                self.last_command = Some(cmd.clone());
                Ok(Some(cmd))
            }
            ModifyKind::Offset => {
                let face = self.target_face.ok_or(ToolError::NotReady)?;
                self.current_pointer = Some(point);
                let diff = point - anchor;
                let raw_dist = diff.length();
                let distance = if raw_dist <= TOLERANCE {
                    0.0
                } else if let Some(centroid) = self.face_centroid {
                    let v_in = centroid - anchor;
                    if v_in.length() > TOLERANCE {
                        if diff.dot(v_in) >= 0.0 {
                            raw_dist
                        } else {
                            -raw_dist
                        }
                    } else {
                        raw_dist
                    }
                } else {
                    raw_dist
                };
                self.measurement = Some(distance);
                let cmd = Command::Offset { face, distance };
                self.last_command = Some(cmd.clone());
                Ok(Some(cmd))
            }
            ModifyKind::FollowMe => {
                let face = self.target_face.ok_or(ToolError::NotReady)?;
                self.current_pointer = Some(point);
                let path = if self.preselected_path.len() >= 2 {
                    self.preselected_path.clone()
                } else {
                    vec![anchor, point]
                };
                self.measurement = Some((point - anchor).length());
                let cmd = Command::FollowMe { face, path };
                self.last_command = Some(cmd.clone());
                Ok(Some(cmd))
            }
            ModifyKind::Eraser => {
                self.current_pointer = Some(point);
                let cmd = if !self.target_faces.is_empty() {
                    Command::EraseFaces(self.target_faces.clone())
                } else if !self.target_edges.is_empty() {
                    Command::EraseEdges(self.target_edges.clone())
                } else {
                    return Ok(None);
                };
                self.last_command = Some(cmd.clone());
                Ok(Some(cmd))
            }
            ModifyKind::PaintBucket => {
                self.current_pointer = Some(point);
                if self.target_faces.is_empty() {
                    return Ok(None);
                }
                let cmd = Command::PaintFace {
                    faces: self.target_faces.clone(),
                    color: self.color,
                };
                self.last_command = Some(cmd.clone());
                Ok(Some(cmd))
            }
        }
    }

    /// Produce a command preview by parsing a typed measurement string.
    ///
    /// Preserves exact session state atomically if measurement parsing fails.
    pub fn measure(&mut self, text: &str) -> Result<Option<Command>, ToolError> {
        self.measure_with_settings(text, MeasurementSettings::default())
    }
    /// Shared parser entry point with document units and locale.
    pub fn measure_with_settings(
        &mut self,
        text: &str,
        settings: MeasurementSettings,
    ) -> Result<Option<Command>, ToolError> {
        self.apply_input(&measurements::parse(text, settings)?, settings)
    }
    /// True until the next begin/cancel/tool change after a completed operation.
    pub fn editing_completed(&self) -> bool {
        self.completed.is_some()
    }

    /// Absolute model resize multiplier requested by the most recent Tape entry.
    pub fn resize_factor(&self) -> Option<f64> {
        self.resize_factor
    }
    /// Anchor retained through completion for confirmed Tape resizing.
    pub fn measurement_anchor(&self) -> Option<DVec3> {
        self.completed
            .as_deref()
            .and_then(|s| s.anchor)
            .or(self.anchor)
    }
    fn measure_value(&mut self, value: f64) -> Result<Option<Command>, ToolError> {
        if !self.active {
            return Err(ToolError::NotReady);
        }
        let anchor = self.anchor.ok_or(ToolError::NotReady)?;

        match self.kind {
            ModifyKind::Select | ModifyKind::Eraser | ModifyKind::PaintBucket => Ok(None),
            ModifyKind::PushPull => {
                let face = self.target_face.ok_or(ToolError::NotReady)?;
                let distance = value;
                self.measurement = Some(distance);
                self.current_pointer = Some(anchor + self.axis * distance);
                let cmd = Command::PushPull { face, distance };
                self.last_command = Some(cmd.clone());
                Ok(Some(cmd))
            }
            ModifyKind::Offset => {
                let face = self.target_face.ok_or(ToolError::NotReady)?;
                let distance = value;
                self.measurement = Some(distance);
                let cmd = Command::Offset { face, distance };
                self.last_command = Some(cmd.clone());
                Ok(Some(cmd))
            }
            ModifyKind::Move | ModifyKind::Copy => {
                let is_copy = self.kind == ModifyKind::Copy;
                let dist = value;
                let dir = if let Some(dir) = self.measurement_direction {
                    dir
                } else if let Some(p) = self.current_pointer {
                    let d = p - anchor;
                    if d.length() > TOLERANCE {
                        d.normalize()
                    } else {
                        DVec3::X
                    }
                } else {
                    DVec3::X
                };
                self.measurement_direction = Some(dir);
                let translation = dir * dist;
                self.current_pointer = Some(anchor + translation);
                self.measurement = Some(dist);
                let matrix = DMat4::from_translation(translation);
                let cmd = Command::Transform {
                    vertices: self.target_vertices.clone(),
                    matrix,
                    copy: is_copy,
                };
                self.last_command = Some(cmd.clone());
                Ok(Some(cmd))
            }
            ModifyKind::Rotate => {
                let deg = value;
                let rad = deg.to_radians();
                let matrix = crate::rotation::matrix(anchor, self.axis, rad);
                let u_ref = self
                    .reference_dir
                    .unwrap_or_else(|| plane_basis_for(self.axis).0);
                let v_basis = self.axis.cross(u_ref);
                let rotated_dir = u_ref * rad.cos() + v_basis * rad.sin();
                self.current_pointer = Some(anchor + rotated_dir);
                self.measurement = Some(deg);
                let cmd = Command::Transform {
                    vertices: self.target_vertices.clone(),
                    matrix,
                    copy: self.rotation_copy,
                };
                self.last_command = Some(cmd.clone());
                Ok(Some(cmd))
            }
            ModifyKind::Scale => {
                let s = value;
                let scale_mat = DMat4::from_scale(DVec3::splat(s));
                let matrix =
                    DMat4::from_translation(anchor) * scale_mat * DMat4::from_translation(-anchor);
                self.measurement = Some(s);
                let cmd = Command::Transform {
                    vertices: self.target_vertices.clone(),
                    matrix,
                    copy: false,
                };
                self.last_command = Some(cmd.clone());
                Ok(Some(cmd))
            }
            ModifyKind::FollowMe => {
                let face = self.target_face.ok_or(ToolError::NotReady)?;
                let dist = value;
                let dir = if let Some(dir) = self.measurement_direction {
                    dir
                } else if let Some(p) = self.current_pointer {
                    let d = p - anchor;
                    if d.length() > TOLERANCE {
                        d.normalize()
                    } else {
                        DVec3::X
                    }
                } else {
                    DVec3::X
                };
                self.measurement_direction = Some(dir);
                let p = anchor + dir * dist;
                let path = if self.preselected_path.len() >= 2 {
                    self.preselected_path.clone()
                } else {
                    vec![anchor, p]
                };
                self.current_pointer = Some(p);
                self.measurement = Some(dist);
                let cmd = Command::FollowMe { face, path };
                self.last_command = Some(cmd.clone());
                Ok(Some(cmd))
            }
            ModifyKind::TapeMeasure => {
                let dist = value;
                let dir = if let Some(dir) = self.measurement_direction {
                    dir
                } else if let Some(p) = self.current_pointer {
                    let d = p - anchor;
                    if d.length() > TOLERANCE {
                        d.normalize()
                    } else {
                        DVec3::X
                    }
                } else {
                    DVec3::X
                };
                self.measurement_direction = Some(dir);
                self.current_pointer = Some(anchor + dir * dist);
                self.measurement = Some(dist);
                Ok(None)
            }
        }
    }

    /// End the current modify session and return the final command to commit.
    ///
    /// For [`ModifyKind::TapeMeasure`] and [`ModifyKind::Select`], returns `None`.
    /// When `TapeMeasure` ends with an active anchor and pointer, a construction
    /// guide line is automatically registered if valid.
    pub fn finish(&mut self) -> Option<Command> {
        if !self.active {
            return None;
        }

        let cmd = match self.kind {
            ModifyKind::TapeMeasure => {
                if self.resize_factor.is_none() {
                    let _ = self.add_guide();
                }
                None
            }
            ModifyKind::Select => None,
            _ => self.last_command.take(),
        };

        let source = self.clone();
        self.cancel();
        self.completed = Some(Box::new(source));
        cmd
    }
}

impl MeasurementTool for ModifySession {
    fn readout(&self) -> measurements::Readout {
        if let Some(source) = &self.completed {
            return source.readout();
        }
        use measurements::{Readout, ValueKind};
        if let Some((factors, count)) = self.scale_factors {
            return Readout {
                label: "Scale",
                values: factors.to_array(),
                count,
                kind: ValueKind::Factor,
            };
        }
        let label = match self.kind {
            ModifyKind::Rotate => "Angle",
            ModifyKind::Scale => "Scale",
            ModifyKind::Select | ModifyKind::Eraser | ModifyKind::PaintBucket => "",
            _ => "Distance",
        };
        Readout {
            label,
            values: [self.measurement().unwrap_or(0.), 0., 0.],
            count: usize::from(!label.is_empty() && self.measurement().is_some()),
            kind: match self.kind {
                ModifyKind::Rotate => ValueKind::Angle,
                ModifyKind::Scale => ValueKind::Factor,
                _ => ValueKind::Length,
            },
        }
    }
    fn apply_input(
        &mut self,
        input: &Measurement,
        settings: MeasurementSettings,
    ) -> Result<Option<Command>, ToolError> {
        let mut candidate = self.completed.as_deref().unwrap_or(self).clone();
        candidate.completed = None;
        if !candidate.active
            || matches!(
                candidate.kind,
                ModifyKind::Select | ModifyKind::Eraser | ModifyKind::PaintBucket
            )
        {
            return Err(ToolError::NotReady);
        }
        let bad =
            || ToolError::InvalidMeasurement("Input is not supported by this modify tool".into());
        let command = match input {
            Measurement::Copies { count, divide }
                if matches!(candidate.kind, ModifyKind::Move | ModifyKind::Copy) =>
            {
                if !(1..=256).contains(count) {
                    return Err(bad());
                }
                let anchor = candidate.anchor.ok_or(ToolError::NotReady)?;
                let delta = candidate.current_pointer.ok_or(ToolError::NotReady)? - anchor;
                if delta.length() <= TOLERANCE {
                    return Err(bad());
                }
                let commands = (1..=*count)
                    .map(|i| Command::Transform {
                        vertices: candidate.target_vertices.clone(),
                        matrix: DMat4::from_translation(
                            delta * (i as f64) / if *divide { *count as f64 } else { 1. },
                        ),
                        copy: true,
                    })
                    .collect();
                let command = Command::Batch(commands);
                candidate.last_command = Some(command.clone());
                Some(command)
            }
            Measurement::Copies {
                count,
                divide: false,
            } if candidate.kind == ModifyKind::Scale => candidate.measure_value(*count as f64)?,
            Measurement::Values(values)
                if candidate.kind == ModifyKind::Scale
                    && !values.is_empty()
                    && values.len() <= 3 =>
            {
                let mut factors = DVec3::ONE;
                for (axis, q) in values.iter().enumerate() {
                    let extent = if values.len() == 1 {
                        candidate.scale_extents.max_element()
                    } else {
                        candidate.scale_extents[axis]
                    };
                    let factor = match q {
                        Quantity::Length(length) if extent > TOLERANCE => length / extent,
                        Quantity::Number(n) => *n,
                        _ => return Err(bad()),
                    };
                    if !factor.is_finite() || factor.abs() <= TOLERANCE {
                        return Err(bad());
                    }
                    factors[axis] = factor;
                }
                if values.len() == 1 {
                    factors = DVec3::splat(factors.x);
                }
                let anchor = candidate.anchor.ok_or(ToolError::NotReady)?;
                let command = Command::Transform {
                    vertices: candidate.target_vertices.clone(),
                    matrix: DMat4::from_translation(anchor)
                        * DMat4::from_scale(factors)
                        * DMat4::from_translation(-anchor),
                    copy: false,
                };
                candidate.scale_factors = Some((factors, values.len()));
                candidate.measurement = Some(factors.x);
                candidate.last_command = Some(command.clone());
                Some(command)
            }
            Measurement::Values(values)
                if candidate.kind == ModifyKind::TapeMeasure && values.len() == 1 =>
            {
                let desired = values[0].length(settings)?;
                let baseline = candidate
                    .tape_baseline
                    .or(candidate.measurement)
                    .ok_or(ToolError::NotReady)?;
                if baseline <= TOLERANCE || desired <= TOLERANCE {
                    return Err(bad());
                }
                let factor = desired / baseline;
                if !factor.is_finite() {
                    return Err(bad());
                }
                candidate.tape_baseline = Some(baseline);
                candidate.resize_factor = Some(factor);
                candidate.measurement = Some(desired);
                let anchor = candidate.anchor.ok_or(ToolError::NotReady)?;
                let command = Command::Transform {
                    vertices: candidate.target_vertices.clone(),
                    matrix: DMat4::from_translation(anchor)
                        * DMat4::from_scale(DVec3::splat(factor))
                        * DMat4::from_translation(-anchor),
                    copy: false,
                };
                candidate.last_command = Some(command.clone());
                Some(command)
            }
            Measurement::Values(values) if values.len() == 1 => {
                let value = match candidate.kind {
                    ModifyKind::Rotate | ModifyKind::Scale => values[0].number()?,
                    _ => values[0].length(settings)?,
                };
                if !value.is_finite() || (candidate.kind == ModifyKind::Scale && value <= TOLERANCE)
                {
                    return Err(bad());
                }
                candidate.measure_value(value)?
            }
            Measurement::Slope(deg) if candidate.kind == ModifyKind::Rotate => {
                candidate.measure_value(*deg)?
            }
            _ => return Err(bad()),
        };
        *self = candidate;
        Ok(command)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cindral_core::History;

    /// Helper to construct a single planar square face in XY plane at z=0.
    fn make_test_square() -> (History, FaceId, Vec<VertexId>, Vec<EdgeId>) {
        let mut history = History::new();
        let p0 = DVec3::new(0.0, 0.0, 0.0);
        let p1 = DVec3::new(2.0, 0.0, 0.0);
        let p2 = DVec3::new(2.0, 2.0, 0.0);
        let p3 = DVec3::new(0.0, 2.0, 0.0);

        let cmd = Command::AddPolyline {
            points: vec![p0, p1, p2, p3],
            closed: true,
        };
        history.execute(cmd).expect("face creation");

        let face_id = *history.model().faces().keys().next().expect("face");
        let vertices: Vec<VertexId> = history.model().vertices().keys().copied().collect();
        let edges: Vec<EdgeId> = history.model().edges().keys().copied().collect();
        (history, face_id, vertices, edges)
    }

    #[test]
    fn stationary_pushpull_does_not_emit_repeated_preview() {
        let mut h = cindral_core::History::new();
        h.execute(Command::AddPolyline {
            points: vec![DVec3::ZERO, DVec3::X, DVec3::X + DVec3::Y, DVec3::Y],
            closed: true,
        })
        .unwrap();
        let face = *h.model().faces().keys().next().unwrap();
        let mut session = ModifySession::new(ModifyKind::PushPull);
        session
            .begin(
                h.model(),
                &Selection::default(),
                Some(Hit::Face(face)),
                DVec3::ZERO,
                DVec3::Z,
            )
            .unwrap();
        assert!(session.update(DVec3::Z).unwrap().is_some());
        assert_eq!(session.update(DVec3::Z).unwrap(), None);
        assert_eq!(session.update(DVec3::Z + DVec3::X).unwrap(), None);
        assert!(session.update(DVec3::Z * 2.).unwrap().is_some());
        session.cancel();
        session
            .begin(
                h.model(),
                &Selection::default(),
                Some(Hit::Face(face)),
                DVec3::ZERO,
                DVec3::Z,
            )
            .unwrap();
        assert!(session.update(DVec3::Z * 2.).unwrap().is_some());
    }
    #[test]
    fn face_tools_explain_missing_editable_face() {
        for kind in [
            ModifyKind::PushPull,
            ModifyKind::Offset,
            ModifyKind::FollowMe,
        ] {
            let mut session = ModifySession::new(kind);
            assert_eq!(
                session.begin(
                    &Model::default(),
                    &Selection::default(),
                    None,
                    DVec3::ZERO,
                    DVec3::Z
                ),
                Err(ToolError::NoEditableFace)
            );
            assert!(!session.active());
        }
    }
    #[test]
    fn test_pushpull_hit_resolution_and_distance_update_and_measure() {
        let (history, face_id, vertices, edges) = make_test_square();

        // 1. Hit on edge resolves face even with empty selection
        let mut session = ModifySession::new(ModifyKind::PushPull);
        let empty_sel = Selection::default();
        let edge_hit = Some(Hit::Edge(edges[0]));
        session
            .begin(history.model(), &empty_sel, edge_hit, DVec3::ZERO, DVec3::Z)
            .expect("begin PushPull on edge hit");
        assert!(session.active());
        assert_eq!(session.anchor(), Some(DVec3::ZERO));

        // update produces PushPull command with distance along normal Z
        let cmd = session
            .update(DVec3::new(0.0, 0.0, 2.5))
            .expect("update")
            .expect("command");
        assert_eq!(
            cmd,
            Command::PushPull {
                face: face_id,
                distance: 2.5
            }
        );
        assert_eq!(session.measurement(), Some(2.5));

        // repeated update produces absolute distance from original anchor
        let cmd2 = session
            .update(DVec3::new(0.0, 0.0, 5.0))
            .expect("update 2")
            .expect("command 2");
        assert_eq!(
            cmd2,
            Command::PushPull {
                face: face_id,
                distance: 5.0
            }
        );

        // typed measurement with units
        let cmd_typed = session.measure("1500mm").expect("measure").expect("cmd");
        assert_eq!(
            cmd_typed,
            Command::PushPull {
                face: face_id,
                distance: 1.5
            }
        );
        assert_eq!(session.measurement(), Some(1.5));

        // typed negative measurement
        let cmd_neg = session.measure("-2.5m").expect("measure neg").expect("cmd");
        assert_eq!(
            cmd_neg,
            Command::PushPull {
                face: face_id,
                distance: -2.5
            }
        );

        // finish returns latest command and resets active state
        let finished = session.finish();
        assert_eq!(finished, Some(cmd_neg));
        assert!(!session.active());
        assert_eq!(session.anchor(), None);

        // 2. Hit on vertex resolves face
        let mut session_v = ModifySession::new(ModifyKind::PushPull);
        session_v
            .begin(
                history.model(),
                &empty_sel,
                Some(Hit::Vertex(vertices[0])),
                DVec3::ZERO,
                DVec3::ZERO, // Fallback to face normal
            )
            .expect("begin PushPull on vertex hit");
        let cmd_v = session_v
            .update(DVec3::new(0.0, 0.0, 1.0))
            .expect("update")
            .expect("cmd");
        assert_eq!(
            cmd_v,
            Command::PushPull {
                face: face_id,
                distance: 1.0
            }
        );
    }

    #[test]
    fn test_move_and_copy_variants_and_repeated_absolute_updates() {
        let (history, _, vertices, _) = make_test_square();

        // 1. Move
        let mut session = ModifySession::new(ModifyKind::Move);
        let mut sel = Selection::default();
        sel.vertices.insert(vertices[0]);
        sel.vertices.insert(vertices[1]);

        let anchor = DVec3::new(0.0, 0.0, 0.0);
        session
            .begin(history.model(), &sel, None, anchor, DVec3::Z)
            .expect("begin Move");

        // First update
        let p1 = DVec3::new(3.0, 0.0, 0.0);
        let cmd1 = session.update(p1).expect("update 1").expect("cmd 1");
        let expected_matrix1 = DMat4::from_translation(DVec3::new(3.0, 0.0, 0.0));
        assert_eq!(
            cmd1,
            Command::Transform {
                vertices: vec![vertices[0], vertices[1]],
                matrix: expected_matrix1,
                copy: false,
            }
        );
        assert_eq!(session.measurement(), Some(3.0));

        // Repeated update: absolute from original anchor, not incremental
        let p2 = DVec3::new(0.0, 4.0, 0.0);
        let cmd2 = session.update(p2).expect("update 2").expect("cmd 2");
        let expected_matrix2 = DMat4::from_translation(DVec3::new(0.0, 4.0, 0.0));
        assert_eq!(
            cmd2,
            Command::Transform {
                vertices: vec![vertices[0], vertices[1]],
                matrix: expected_matrix2,
                copy: false,
            }
        );
        assert_eq!(session.measurement(), Some(4.0));

        // Typed measurement reuses last direction (which was +Y)
        let cmd_typed = session.measure("10m").expect("measure").expect("cmd");
        let expected_typed_matrix = DMat4::from_translation(DVec3::new(0.0, 10.0, 0.0));
        assert_eq!(
            cmd_typed,
            Command::Transform {
                vertices: vec![vertices[0], vertices[1]],
                matrix: expected_typed_matrix,
                copy: false,
            }
        );

        // 2. Copy (produces copy: true)
        let mut copy_session = ModifySession::new(ModifyKind::Copy);
        copy_session
            .begin(history.model(), &sel, None, anchor, DVec3::Z)
            .expect("begin Copy");
        let copy_cmd = copy_session.update(p1).expect("update").expect("cmd");
        assert_eq!(
            copy_cmd,
            Command::Transform {
                vertices: vec![vertices[0], vertices[1]],
                matrix: expected_matrix1,
                copy: true,
            }
        );

        // Measure without prior update defaults to +X
        let mut copy_fresh = ModifySession::new(ModifyKind::Copy);
        copy_fresh
            .begin(history.model(), &sel, None, anchor, DVec3::Z)
            .expect("begin");
        let def_x_cmd = copy_fresh.measure("5m").expect("measure").expect("cmd");
        let expected_x = DMat4::from_translation(DVec3::new(5.0, 0.0, 0.0));
        assert_eq!(
            def_x_cmd,
            Command::Transform {
                vertices: vec![vertices[0], vertices[1]],
                matrix: expected_x,
                copy: true,
            }
        );
    }

    #[test]
    fn test_rotate_convention_affine_around_anchor_and_measure() {
        let (history, _, vertices, _) = make_test_square();

        let mut session = ModifySession::new(ModifyKind::Rotate);
        let mut sel = Selection::default();
        sel.vertices.extend(&vertices);

        let anchor = DVec3::new(1.0, 1.0, 0.0);
        let axis = DVec3::Z;
        session
            .begin(history.model(), &sel, None, anchor, axis)
            .expect("begin Rotate");

        // Cursor at anchor + X: angle should be 0 degrees
        let p_x = anchor + DVec3::new(2.0, 0.0, 0.0);
        let cmd_0 = session.update(p_x).expect("update").expect("cmd");
        if let Command::Transform { matrix, copy, .. } = cmd_0 {
            assert!(!copy);
            // Anchor point must be invariant under transformation
            let transformed_anchor = matrix.transform_point3(anchor);
            assert!(transformed_anchor.distance(anchor) < 1e-9);
        } else {
            panic!("expected Transform");
        }
        assert!((session.measurement().unwrap() - 0.0).abs() < 1e-9);

        // Cursor at anchor + Y: angle should be 90 degrees
        let p_y = anchor + DVec3::new(0.0, 2.0, 0.0);
        let _ = session.update(p_y).expect("update");
        assert!((session.measurement().unwrap() - 90.0).abs() < 1e-9);

        // Repeated update is absolute: moving back to +X gives 0 degrees
        let _ = session.update(p_x).expect("update");
        assert!((session.measurement().unwrap() - 0.0).abs() < 1e-9);

        // Typed measurements: 45 degrees, 90 deg, -90°
        session.measure("45 deg").expect("measure 45");
        assert!((session.measurement().unwrap() - 45.0).abs() < 1e-9);

        session.measure("-90°").expect("measure -90");
        assert!((session.measurement().unwrap() - (-90.0)).abs() < 1e-9);

        session.measure("180").expect("measure 180");
        assert!((session.measurement().unwrap() - 180.0).abs() < 1e-9);
    }

    #[test]
    fn test_scale_affine_around_anchor_and_typed_scale() {
        let (history, _, vertices, _) = make_test_square();

        let mut session = ModifySession::new(ModifyKind::Scale);
        let mut sel = Selection::default();
        sel.vertices.extend(&vertices);

        let anchor = DVec3::new(1.0, 1.0, 0.0);
        session
            .begin(history.model(), &sel, None, anchor, DVec3::Z)
            .expect("begin Scale");

        // Update with distance 2.0 -> scale factor 2.0
        let p2 = anchor + DVec3::new(2.0, 0.0, 0.0);
        let cmd2 = session.update(p2).expect("update").expect("cmd");
        if let Command::Transform { matrix, .. } = cmd2 {
            // Anchor is invariant
            assert!(matrix.transform_point3(anchor).distance(anchor) < 1e-9);
            // Point at anchor + (1, 0, 0) scales to anchor + (2, 0, 0)
            let test_pt = anchor + DVec3::new(1.0, 0.0, 0.0);
            let expected_pt = anchor + DVec3::new(2.0, 0.0, 0.0);
            assert!(matrix.transform_point3(test_pt).distance(expected_pt) < 1e-9);
        } else {
            panic!("expected Transform");
        }
        assert_eq!(session.measurement(), Some(2.0));

        // Repeated update is absolute: distance 0.5 -> scale factor 0.5
        let p_half = anchor + DVec3::new(0.5, 0.0, 0.0);
        let _ = session.update(p_half).expect("update");
        assert_eq!(session.measurement(), Some(0.5));

        // Typed scale factors
        session.measure("3x").expect("3x");
        assert_eq!(session.measurement(), Some(3.0));

        session.measure("150%").expect("150%");
        assert_eq!(session.measurement(), Some(1.5));

        session.measure("0.5").expect("0.5");
        assert_eq!(session.measurement(), Some(0.5));

        // Zero scales are rejected; negative factors reflect.
        assert!(matches!(
            session.measure("0"),
            Err(ToolError::InvalidMeasurement(_))
        ));
        assert!(session.measure("-1").unwrap().is_some());
    }

    #[test]
    fn test_offset_face_resolution_and_typed_measure() {
        let (history, face_id, _, _) = make_test_square();

        let mut session = ModifySession::new(ModifyKind::Offset);
        let mut sel = Selection::default();
        sel.faces.insert(face_id);

        let anchor = DVec3::new(0.0, 1.0, 0.0);
        session
            .begin(history.model(), &sel, None, anchor, DVec3::Z)
            .expect("begin Offset");

        // Move inward towards centroid (1, 1, 0)
        let p_in = DVec3::new(0.2, 1.0, 0.0);
        let cmd_in = session.update(p_in).expect("update").expect("cmd");
        assert_eq!(
            cmd_in,
            Command::Offset {
                face: face_id,
                distance: 0.2
            }
        );

        // Move outward away from centroid
        let p_out = DVec3::new(-0.2, 1.0, 0.0);
        let cmd_out = session.update(p_out).expect("update").expect("cmd");
        assert_eq!(
            cmd_out,
            Command::Offset {
                face: face_id,
                distance: -0.2
            }
        );

        // Typed measurement
        let cmd_typed = session.measure("100mm").expect("measure").expect("cmd");
        assert_eq!(
            cmd_typed,
            Command::Offset {
                face: face_id,
                distance: 0.1
            }
        );
    }

    #[test]
    fn test_followme_with_connected_ordered_edges_and_anchor_fallback() {
        let (history, face_id, _, edges) = make_test_square();

        // 1. FollowMe with pre-selected connected edges:
        // edges[0] and edges[1] form a connected chain
        let mut session = ModifySession::new(ModifyKind::FollowMe);
        let mut sel = Selection::default();
        sel.faces.insert(face_id);
        sel.edges.insert(edges[0]);
        sel.edges.insert(edges[1]);

        session
            .begin(history.model(), &sel, None, DVec3::ZERO, DVec3::Z)
            .expect("begin FollowMe with edges");
        let cmd = session
            .update(DVec3::new(5.0, 5.0, 0.0))
            .expect("update")
            .expect("cmd");

        if let Command::FollowMe { face, path } = cmd {
            assert_eq!(face, face_id);
            assert_eq!(path.len(), 3); // 2 connected edges yield 3 points
        } else {
            panic!("expected FollowMe");
        }

        // 2. FollowMe without selected edges: falls back to [anchor, point]
        let mut session_fallback = ModifySession::new(ModifyKind::FollowMe);
        let mut sel_no_edges = Selection::default();
        sel_no_edges.faces.insert(face_id);

        let anchor = DVec3::new(1.0, 1.0, 0.0);
        session_fallback
            .begin(history.model(), &sel_no_edges, None, anchor, DVec3::Z)
            .expect("begin fallback");

        let p = DVec3::new(1.0, 1.0, 3.0);
        let cmd_fallback = session_fallback.update(p).expect("update").expect("cmd");
        assert_eq!(
            cmd_fallback,
            Command::FollowMe {
                face: face_id,
                path: vec![anchor, p]
            }
        );

        // Typed length with fallback
        let cmd_typed = session_fallback
            .measure("2m")
            .expect("measure")
            .expect("cmd");
        assert_eq!(
            cmd_typed,
            Command::FollowMe {
                face: face_id,
                path: vec![anchor, anchor + DVec3::new(0.0, 0.0, 2.0)],
            }
        );
    }

    #[test]
    fn eraser_targets_only_hit_and_never_falls_back_to_selection() {
        let (history, face, vertices, edges) = make_test_square();
        let mut selection = Selection::default();
        selection.edges.extend(&edges);
        selection.faces.insert(face);
        let mut eraser = ModifySession::new(ModifyKind::Eraser);
        for (hit, expected) in [
            (Hit::Edge(edges[0]), Command::EraseEdges(vec![edges[0]])),
            (Hit::Face(face), Command::EraseFaces(vec![face])),
        ] {
            eraser
                .begin(
                    history.model(),
                    &selection,
                    Some(hit),
                    DVec3::ZERO,
                    DVec3::Z,
                )
                .unwrap();
            assert_eq!(eraser.update(DVec3::ZERO).unwrap(), Some(expected));
            eraser.finish();
        }
        for hit in [None, Some(Hit::Vertex(vertices[0]))] {
            assert!(eraser
                .begin(history.model(), &selection, hit, DVec3::ZERO, DVec3::Z)
                .is_err());
            assert!(!eraser.active());
        }
    }

    #[test]
    fn test_eraser_and_paint_bucket_commands() {
        let (history, face_id, _, edges) = make_test_square();

        // 1. Eraser
        let mut eraser = ModifySession::new(ModifyKind::Eraser);
        let mut sel_edge = Selection::default();
        sel_edge.edges.insert(edges[0]);

        eraser
            .begin(
                history.model(),
                &sel_edge,
                Some(Hit::Edge(edges[0])),
                DVec3::ZERO,
                DVec3::Z,
            )
            .expect("begin Eraser");
        let erase_cmd = eraser
            .update(DVec3::new(1.0, 0.0, 0.0))
            .expect("update")
            .expect("cmd");
        assert_eq!(erase_cmd, Command::EraseEdges(vec![edges[0]]));
        let finished_erase = eraser.finish();
        assert_eq!(finished_erase, Some(Command::EraseEdges(vec![edges[0]])));

        // 2. PaintBucket
        let mut paint = ModifySession::new(ModifyKind::PaintBucket);
        paint.set_color([12, 34, 56, 78]);
        assert_eq!(paint.color(), [12, 34, 56, 78]);

        let mut sel_face = Selection::default();
        sel_face.faces.insert(face_id);
        paint
            .begin(history.model(), &sel_face, None, DVec3::ZERO, DVec3::Z)
            .expect("begin Paint");

        let paint_cmd = paint
            .update(DVec3::new(1.0, 1.0, 0.0))
            .expect("update")
            .expect("cmd");
        assert_eq!(
            paint_cmd,
            Command::PaintFace {
                faces: vec![face_id],
                color: [12, 34, 56, 78],
            }
        );
    }

    #[test]
    fn test_tape_measure_no_real_edits_and_guides() {
        let (history, _, _, _) = make_test_square();
        let initial_faces_count = history.model().faces().len();
        let initial_edges_count = history.model().edges().len();

        let mut session = ModifySession::new(ModifyKind::TapeMeasure);
        let anchor = DVec3::new(0.0, 0.0, 0.0);
        let sel = Selection::default();
        session
            .begin(history.model(), &sel, None, anchor, DVec3::Z)
            .expect("begin Tape");

        // update produces NO command
        let update_res = session.update(DVec3::new(4.0, 0.0, 0.0)).expect("update");
        assert_eq!(update_res, None);
        assert_eq!(session.measurement(), Some(4.0));

        // Typed measurement proposes a resize; the app requires confirmation.
        let measure_res = session.measure("5m").expect("measure");
        assert!(measure_res.is_some());
        assert_eq!(session.measurement(), Some(5.0));

        // add_guide persists a construction line
        session.add_guide().expect("add guide");
        assert_eq!(session.guides().len(), 1);
        assert_eq!(
            session.guides()[0],
            Guide {
                origin: anchor,
                direction: DVec3::X,
            }
        );

        // finish produces NO command
        let finish_res = session.finish();
        assert_eq!(finish_res, None);

        // Verify model had zero mutations
        assert_eq!(history.model().faces().len(), initial_faces_count);
        assert_eq!(history.model().edges().len(), initial_edges_count);
    }

    #[test]
    fn test_cancel_resets_interaction_preserving_guides() {
        let (history, face_id, _, _) = make_test_square();

        let mut session = ModifySession::new(ModifyKind::TapeMeasure);
        session
            .begin(
                history.model(),
                &Selection::default(),
                None,
                DVec3::ZERO,
                DVec3::Z,
            )
            .expect("begin");
        session.update(DVec3::new(2.0, 0.0, 0.0)).expect("update");
        session.add_guide().expect("add guide");
        assert_eq!(session.guides().len(), 1);

        // Start a Move tool
        session.set_kind(ModifyKind::Move);
        let mut sel = Selection::default();
        sel.faces.insert(face_id);
        session
            .begin(history.model(), &sel, None, DVec3::ZERO, DVec3::Z)
            .expect("begin move");
        assert!(session.active());
        session.update(DVec3::new(1.0, 0.0, 0.0)).expect("update");
        assert!(session.measurement().is_some());

        // Cancel
        session.cancel();
        assert!(!session.active());
        assert_eq!(session.anchor(), None);
        assert_eq!(session.measurement(), None);

        // Guide lines are preserved!
        assert_eq!(session.guides().len(), 1);
        assert_eq!(session.guides()[0].origin, DVec3::ZERO);
    }

    #[test]
    fn test_measurement_invalid_atomic_state() {
        let (history, _, vertices, _) = make_test_square();

        let mut session = ModifySession::new(ModifyKind::Move);
        let mut sel = Selection::default();
        sel.vertices.insert(vertices[0]);

        session
            .begin(history.model(), &sel, None, DVec3::ZERO, DVec3::Z)
            .expect("begin");
        let valid_cmd = session
            .update(DVec3::new(2.0, 0.0, 0.0))
            .expect("update")
            .expect("cmd");
        assert_eq!(session.measurement(), Some(2.0));

        // Invalid measurement error must leave state completely unchanged
        let err = session.measure("not_a_valid_dimension_xyz");
        assert!(matches!(err, Err(ToolError::InvalidMeasurement(_))));

        assert!(session.active());
        assert_eq!(session.anchor(), Some(DVec3::ZERO));
        assert_eq!(session.measurement(), Some(2.0));

        // finish still produces the prior valid command
        let final_cmd = session.finish();
        assert_eq!(final_cmd, Some(valid_cmd));
    }

    #[test]
    fn test_axis_and_point_validation() {
        let (history, _, vertices, _) = make_test_square();
        let mut sel = Selection::default();
        sel.vertices.insert(vertices[0]);

        let mut session = ModifySession::new(ModifyKind::Rotate);

        // Non-finite point is rejected
        let nan_point = DVec3::new(f64::NAN, 0.0, 0.0);
        assert!(matches!(
            session.begin(history.model(), &sel, None, nan_point, DVec3::Z),
            Err(ToolError::InvalidPoint)
        ));

        // Non-finite axis is rejected
        let nan_axis = DVec3::new(0.0, 0.0, f64::INFINITY);
        assert!(matches!(
            session.begin(history.model(), &sel, None, DVec3::ZERO, nan_axis),
            Err(ToolError::InvalidPoint)
        ));

        // Degenerate zero axis for Rotate is rejected
        assert!(matches!(
            session.begin(history.model(), &sel, None, DVec3::ZERO, DVec3::ZERO),
            Err(ToolError::Degenerate)
        ));

        // Non-unit axis is normalized properly
        session
            .begin(
                history.model(),
                &sel,
                None,
                DVec3::ZERO,
                DVec3::new(0.0, 0.0, 5.0),
            )
            .expect("scaled axis normalized");
        assert_eq!(session.axis, DVec3::Z);
    }
}
