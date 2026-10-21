//! Local edit cavities, reversible entity deltas and derived dirty sets.
use crate::spatial::envelope;
use crate::topology::face_key;
use crate::{Command, DVec3, Edge, EdgeId, Face, FaceId, KernelError, Model, Vertex, VertexId};
use std::collections::{BTreeMap, BTreeSet};

/// Entities changed by the last edit/undo/redo, including removed IDs.
/// Current geometry belongs to the root container; M5 can partition this per group.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ChangeSet {
    /// Monotonic document revision of this change.
    pub revision: u64,
    /// Only a topology-preserving whole-definition translation changed positions.
    pub rigid_translation: bool,
    /// Added, changed or removed vertices.
    pub vertices: BTreeSet<VertexId>,
    /// Added, changed or removed edges.
    pub edges: BTreeSet<EdgeId>,
    /// Added, changed or removed faces.
    pub faces: BTreeSet<FaceId>,
}
#[derive(Debug, Clone)]
pub(crate) struct ValueChange<T> {
    before: Option<T>,
    after: Option<T>,
}
#[derive(Debug, Clone)]
pub(crate) struct Delta {
    translation: Option<crate::translation::Translation>,
    vertices: BTreeMap<VertexId, ValueChange<Vertex>>,
    edges: BTreeMap<EdgeId, ValueChange<Edge>>,
    faces: BTreeMap<FaceId, ValueChange<Face>>,
    before_next: u64,
    after_next: u64,
}
fn difference<I: Ord + Copy, T: Clone + PartialEq>(
    before: &BTreeMap<I, T>,
    after: &BTreeMap<I, T>,
) -> BTreeMap<I, ValueChange<T>> {
    before
        .keys()
        .chain(after.keys())
        .copied()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .filter_map(|id| {
            let a = before.get(&id);
            let b = after.get(&id);
            (a != b).then(|| {
                (
                    id,
                    ValueChange {
                        before: a.cloned(),
                        after: b.cloned(),
                    },
                )
            })
        })
        .collect()
}
impl<T> ValueChange<T> {
    fn old(&self, forward: bool) -> Option<&T> {
        if forward {
            self.before.as_ref()
        } else {
            self.after.as_ref()
        }
    }
    fn new_value(&self, forward: bool) -> Option<&T> {
        if forward {
            self.after.as_ref()
        } else {
            self.before.as_ref()
        }
    }
}
#[derive(Default)]
struct Selection {
    points: Vec<DVec3>,
    edges: BTreeSet<EdgeId>,
    erase_vertices: BTreeSet<VertexId>,
    geometry: bool,
}
impl Selection {
    fn scan(&mut self, model: &Model, command: &Command, depth: usize) -> Result<(), KernelError> {
        if depth > 64 {
            return Err(KernelError::BatchDepth);
        }
        match command {
            Command::PushPull { .. }
            | Command::Transform { .. }
            | Command::Offset { .. }
            | Command::FollowMe { .. }
            | Command::PaintFace { .. }
            | Command::SetFaceMaterial { .. } => {
                return Err(KernelError::InvalidTopology(
                    "explicit surface edit requires a prepared transaction".into(),
                ))
            }
            Command::AddEdge { start, end } => {
                self.geometry = true;
                self.points.extend([*start, *end]);
            }
            Command::AddPolyline { points, .. } => {
                self.geometry = true;
                self.points.extend(points);
            }
            Command::EraseEdges(ids) => {
                self.geometry |= !ids.is_empty();
                for id in ids {
                    let edge = model.edges.get(id).ok_or(KernelError::MissingEntity)?;
                    self.edges.insert(*id);
                    self.erase_vertices.extend(edge.vertices);
                    for vertex in edge.vertices {
                        self.points.push(model.position(vertex)?);
                    }
                }
            }
            Command::SetEdgeStyle { edges, .. } => {
                for id in edges {
                    if !model.edges.contains_key(id) {
                        return Err(KernelError::MissingEntity);
                    }
                    self.edges.insert(*id);
                }
            }
            Command::Batch(commands) => {
                for command in commands {
                    self.scan(model, command, depth + 1)?;
                }
            }
        }
        Ok(())
    }
}
impl Delta {
    pub fn prepare(
        model: &mut Model,
        command: &Command,
        high_water: u64,
    ) -> Result<Self, KernelError> {
        if fast_paths_enabled() {
            model.ensure_spatial()?;
            if let Some((before, after)) =
                crate::local_extrusion::prepare(model, command, high_water)?
            {
                return Ok(Self::between(&before, &after, model.next_id));
            }
            if let Some(translation) = crate::translation::Translation::prepare(model, command)? {
                return Ok(Self {
                    translation: Some(translation),
                    vertices: BTreeMap::new(),
                    edges: BTreeMap::new(),
                    faces: BTreeMap::new(),
                    before_next: model.next_id,
                    after_next: high_water.max(model.next_id),
                });
            }
        }
        Self::prepare_entities(model, command, high_water)
    }
    pub fn replace_translation(
        &mut self,
        model: &mut Model,
        command: &Command,
    ) -> Result<Option<bool>, KernelError> {
        if let Some(translation) = &mut self.translation {
            translation.replace(model, command)
        } else {
            Ok(None)
        }
    }
    fn prepare_entities(
        model: &mut Model,
        command: &Command,
        high_water: u64,
    ) -> Result<Self, KernelError> {
        if let Command::Batch(commands) = command {
            fn depth(command: &Command, level: usize) -> Result<(), KernelError> {
                if level > 64 {
                    return Err(KernelError::BatchDepth);
                }
                if let Command::Batch(commands) = command {
                    for c in commands {
                        depth(c, level + 1)?;
                    }
                }
                Ok(())
            }
            depth(command, 0)?;
            let initial_next = model.next_id;
            let mut deltas: Vec<Self> = Vec::new();
            let mut next = high_water;
            for c in commands {
                match Self::prepare_entities(model, c, next) {
                    Ok(delta) => {
                        delta.apply(model, true)?;
                        next = delta.next_id();
                        deltas.push(delta);
                    }
                    Err(error) => {
                        for d in deltas.iter().rev() {
                            d.apply(model, false)?;
                        }
                        return Err(error);
                    }
                }
            }
            for d in deltas.iter().rev() {
                d.apply(model, false)?;
            }
            let mut combined = Self {
                translation: None,
                vertices: BTreeMap::new(),
                edges: BTreeMap::new(),
                faces: BTreeMap::new(),
                before_next: initial_next,
                after_next: next,
            };
            fn append<I: Ord, T>(
                target: &mut BTreeMap<I, ValueChange<T>>,
                source: BTreeMap<I, ValueChange<T>>,
            ) {
                for (id, change) in source {
                    match target.entry(id) {
                        std::collections::btree_map::Entry::Vacant(e) => {
                            e.insert(change);
                        }
                        std::collections::btree_map::Entry::Occupied(mut e) => {
                            e.get_mut().after = change.after;
                        }
                    }
                }
            }
            for d in deltas {
                append(&mut combined.vertices, d.vertices);
                append(&mut combined.edges, d.edges);
                append(&mut combined.faces, d.faces);
            }
            combined.vertices.retain(|_, v| v.before != v.after);
            combined.edges.retain(|_, v| {
                v.before != v.after
                    || v.after.as_ref().is_some_and(|e| {
                        e.vertices.iter().any(|v| combined.vertices.contains_key(v))
                    })
            });
            combined.faces.retain(|_, v| {
                v.before != v.after
                    || v.after.as_ref().is_some_and(|f| {
                        f.outer
                            .iter()
                            .chain(f.holes.iter().flatten())
                            .any(|v| combined.vertices.contains_key(v))
                    })
            });
            return Ok(combined);
        }
        if let Command::Offset { face, distance } = command {
            return Self::prepare(
                model,
                &crate::modification::offset_command(model, *face, *distance)?,
                high_water,
            );
        }
        if matches!(
            command,
            Command::PushPull { .. }
                | Command::Transform { .. }
                | Command::FollowMe { .. }
                | Command::PaintFace { .. }
                | Command::SetFaceMaterial { .. }
        ) {
            model.ensure_spatial()?;
            let (before, after) = crate::modification::prepare(model, command, high_water)?;
            return Ok(Self::between(&before, &after, model.next_id));
        }
        let mut selection = Selection::default();
        selection.scan(model, command, 0)?;
        for point in &selection.points {
            crate::geometry::validate_position(*point)?;
        }
        model.ensure_spatial()?;
        let spatial = model.spatial.as_ref().ok_or(KernelError::MissingEntity)?;
        let mut faces = BTreeSet::new();
        if selection.geometry {
            if let Some(bounds) = envelope(selection.points) {
                selection.edges.extend(spatial.query_edges(bounds));
                faces.extend(spatial.query_faces(bounds));
            }
            // A chord wholly inside one convex face cannot close a remote wire loop.
            // Other edits still need the exterior-boundary traversal for Euler correctness.
            let confined = if let Command::AddEdge { start, end } = command {
                faces.iter().any(|id| {
                    model
                        .faces
                        .get(id)
                        .is_some_and(|f| convex_chord(model, f, *start, *end))
                })
            } else {
                false
            };
            // Follow loose wire components, not entire connected face meshes.
            // A closing segment may be far from the other sides of an open loop.
            let mut queue: Vec<_> = selection
                .edges
                .iter()
                .filter(|id| {
                    spatial
                        .edge_faces
                        .get(id)
                        .is_none_or(|faces| faces.is_empty() || (!confined && faces.len() == 1))
                })
                .copied()
                .collect();
            let mut visited = BTreeSet::new();
            while let Some(id) = queue.pop() {
                if !visited.insert(id) {
                    continue;
                }
                selection.edges.insert(id);
                let edge = model.edges.get(&id).ok_or(KernelError::MissingEntity)?;
                for vertex in edge.vertices {
                    if let Some(neighbors) = spatial.adjacency.get(&vertex) {
                        for neighbor in neighbors {
                            if let Some(boundary_faces) = spatial
                                .edge_faces
                                .get(neighbor)
                                .filter(|set| !set.is_empty())
                            {
                                faces.extend(boundary_faces);
                                if !confined && boundary_faces.len() == 1 {
                                    queue.push(*neighbor);
                                }
                            } else {
                                queue.push(*neighbor);
                            }
                        }
                    }
                }
            }
            for edge in &selection.edges {
                if let Some(incident) = spatial.edge_faces.get(edge) {
                    faces.extend(incident);
                }
            }
            for face in &faces {
                if let Some(boundary) = spatial.face_edges.get(face) {
                    selection.edges.extend(boundary);
                }
            }
        }
        let mut before = Model {
            next_id: model.next_id,
            heal_scope: Some(selection.erase_vertices),
            ..Model::default()
        };
        for id in &selection.edges {
            let edge = model.edges.get(id).ok_or(KernelError::MissingEntity)?;
            before.edges.insert(*id, edge.clone());
            for vertex in edge.vertices {
                before.vertices.insert(
                    vertex,
                    model
                        .vertices
                        .get(&vertex)
                        .ok_or(KernelError::MissingEntity)?
                        .clone(),
                );
            }
        }
        for id in &faces {
            before.faces.insert(
                *id,
                model
                    .faces
                    .get(id)
                    .ok_or(KernelError::MissingEntity)?
                    .clone(),
            );
        }
        for vertex in before.vertices.keys() {
            if spatial
                .adjacency
                .get(vertex)
                .is_some_and(|edges| edges.iter().any(|id| !selection.edges.contains(id)))
            {
                before.protected.insert(*vertex);
            }
        }
        let mut after = before.clone();
        after.next_id = high_water;
        if command.apply(&mut after, 0)? {
            after.rebuild_faces()?;
        }
        // An explicit solid operation may leave bounded openings. A later draw on
        // another surface must not automatically cap an already-existing void.
        let old_faces: BTreeSet<_> = before.faces.values().map(face_key).collect();
        let context = crate::spatial::envelope(before.vertices.values().map(|v| v.position))
            .map(|bounds| spatial.query_faces(bounds))
            .unwrap_or_default();
        let context_faces: Vec<_> = context
            .iter()
            .filter_map(|id| model.faces.get(id))
            .collect();
        let existing_outers: BTreeSet<BTreeSet<VertexId>> = context_faces
            .iter()
            .map(|f| f.outer.iter().copied().collect())
            .collect();
        let openings: Vec<BTreeSet<VertexId>> = context_faces
            .iter()
            .flat_map(|f| f.holes.iter())
            .map(|ring| ring.iter().copied().collect())
            .filter(|ring| !existing_outers.contains(ring))
            .collect();
        after.faces.retain(|_, f| {
            old_faces.contains(&face_key(f))
                || !openings
                    .iter()
                    .any(|hole| *hole == f.outer.iter().copied().collect())
        });
        // Context edges may bound an unchanged face outside this cavity. Do not
        // create a duplicate of that globally indexed region.
        after.faces.retain(|_, face| {
            spatial
                .face_keys
                .get(&face_key(face))
                .is_none_or(|id| faces.contains(id))
        });
        // A cavity's context boundary can enclose untouched regions whose
        // interior edges were deliberately not copied. Suppress those artificial
        // regions; the globally indexed faces already represent them.
        let mut artificial = Vec::new();
        for (id, face) in &after.faces {
            let triangles = after.triangulate(*id)?;
            if let Some([a, b, c]) = triangles.first() {
                let sample = (*a + *b + *c) / 3.0;
                if let Some(bounds) = envelope([sample]) {
                    for other in spatial.query_faces(bounds) {
                        if !faces.contains(&other) {
                            let old = model.faces.get(&other).ok_or(KernelError::MissingEntity)?;
                            if old.plane.normal.cross(face.plane.normal).length()
                                <= crate::TOLERANCE
                                && old.plane.signed_distance(sample).abs() <= crate::TOLERANCE
                                && contains_face(model, old, sample)?
                            {
                                artificial.push(*id);
                                break;
                            }
                        }
                    }
                }
            }
        }
        for id in artificial {
            after.faces.remove(&id);
        }
        // New split/healed regions inherit the material/color of their old containing
        // surface. Use a triangulated interior sample so concave rings and holes work.
        let mut styles = Vec::new();
        for (id, face) in &after.faces {
            if before.faces.contains_key(id) {
                continue;
            }
            if let Some([a, b, c]) = after.triangulate(*id)?.first() {
                let sample = (*a + *b + *c) / 3.;
                for previous in before.faces.values() {
                    if previous.plane.normal.cross(face.plane.normal).length() <= crate::TOLERANCE
                        && previous.plane.signed_distance(sample).abs() <= crate::TOLERANCE
                        && contains_face(&before, previous, sample)?
                    {
                        styles.push((*id, previous.color, previous.material));
                        break;
                    }
                }
            }
        }
        for (id, color, material) in styles {
            if let Some(face) = after.faces.get_mut(&id) {
                face.color = color;
                face.material = material;
            }
        }
        after.validate()?;
        Ok(Self::between(&before, &after, model.next_id))
    }
    pub(crate) fn between(before: &Model, after: &Model, before_next: u64) -> Self {
        let vertices = difference(&before.vertices, &after.vertices);
        let moved: BTreeSet<_> = vertices.keys().copied().collect();
        let mut edges = difference(&before.edges, &after.edges);
        let mut faces = difference(&before.faces, &after.faces);
        for (id, edge) in &after.edges {
            if edge.vertices.iter().any(|v| moved.contains(v)) {
                edges.insert(
                    *id,
                    ValueChange {
                        before: before.edges.get(id).cloned(),
                        after: Some(edge.clone()),
                    },
                );
            }
        }
        for (id, face) in &after.faces {
            if face
                .outer
                .iter()
                .chain(face.holes.iter().flatten())
                .any(|v| moved.contains(v))
            {
                faces.insert(
                    *id,
                    ValueChange {
                        before: before.faces.get(id).cloned(),
                        after: Some(face.clone()),
                    },
                );
            }
        }
        Self {
            translation: None,
            vertices,
            edges,
            faces,
            before_next,
            after_next: after.next_id,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.translation
            .as_ref()
            .is_none_or(|t| t.offset == DVec3::ZERO)
            && self.vertices.is_empty()
            && self.edges.is_empty()
            && self.faces.is_empty()
    }
    pub fn next_id(&self) -> u64 {
        self.after_next
    }
    pub fn changes(&self, revision: u64) -> ChangeSet {
        if let Some(t) = &self.translation {
            return ChangeSet {
                revision,
                rigid_translation: true,
                vertices: t.vertices.iter().map(|(id, _)| *id).collect(),
                edges: t.edges.iter().copied().collect(),
                faces: t.planes.iter().map(|(id, _)| *id).collect(),
            };
        }
        ChangeSet {
            revision,
            rigid_translation: false,
            vertices: self.vertices.keys().copied().collect(),
            edges: self.edges.keys().copied().collect(),
            faces: self.faces.keys().copied().collect(),
        }
    }
    pub fn apply(&self, model: &mut Model, forward: bool) -> Result<(), KernelError> {
        if let Some(t) = &self.translation {
            t.apply(model, forward)?;
            model.next_id = if forward {
                self.after_next
            } else {
                self.before_next
            };
            return Ok(());
        }
        model.ensure_spatial()?;
        let mut spatial = model.spatial.take().ok_or(KernelError::MissingEntity)?;
        // Remove index entries while all old endpoints still exist.
        for (id, value) in &self.faces {
            if let Some(old) = value.old(forward) {
                spatial.remove_face(*id, old, model)?;
            }
        }
        for (id, value) in &self.edges {
            if let Some(old) = value.old(forward) {
                if value.new_value(forward).is_none_or(|new| {
                    new.vertices != old.vertices
                        || old.vertices.iter().any(|v| self.vertices.contains_key(v))
                }) {
                    spatial.remove_edge(*id, old, model)?;
                }
            }
        }
        for (id, value) in &self.vertices {
            if let Some(old) = value.old(forward) {
                spatial.remove_vertex(*id, old.position);
            }
        }
        for id in self.faces.keys() {
            model.faces.remove(id);
        }
        for id in self.edges.keys() {
            model.edges.remove(id);
        }
        for id in self.vertices.keys() {
            model.vertices.remove(id);
        }
        for (id, value) in &self.vertices {
            if let Some(new) = value.new_value(forward) {
                model.vertices.insert(*id, new.clone());
                spatial.add_vertex(*id, new.position);
            }
        }
        for (id, value) in &self.edges {
            if let Some(new) = value.new_value(forward) {
                model.edges.insert(*id, new.clone());
                if value.old(forward).is_none_or(|old| {
                    old.vertices != new.vertices
                        || new.vertices.iter().any(|v| self.vertices.contains_key(v))
                }) {
                    spatial.add_edge(*id, new, model)?;
                }
            }
        }
        for (id, value) in &self.faces {
            if let Some(new) = value.new_value(forward) {
                model.faces.insert(*id, new.clone());
                spatial.add_face(*id, new, model)?;
            }
        }
        model.next_id = if forward {
            self.after_next
        } else {
            self.before_next
        };
        model.spatial = Some(spatial);
        Ok(())
    }
}

pub(crate) fn contains_face(model: &Model, face: &Face, point: DVec3) -> Result<bool, KernelError> {
    let origin = model.position(*face.outer.first().ok_or(KernelError::Triangulation)?)?;
    let (u, v) = face.plane.basis();
    let p = glam::DVec2::new((point - origin).dot(u), (point - origin).dot(v));
    let outer = model.project_at(&face.outer, face.plane, origin)?;
    if !crate::geometry::contains(&outer, p) {
        return Ok(false);
    }
    for hole in &face.holes {
        if crate::geometry::contains(&model.project_at(hole, face.plane, origin)?, p) {
            return Ok(false);
        }
    }
    Ok(true)
}

fn convex_chord(model: &Model, face: &Face, start: DVec3, end: DVec3) -> bool {
    if !face.holes.is_empty()
        || !(3..=64).contains(&face.outer.len())
        || face.plane.signed_distance(start).abs() > crate::TOLERANCE
        || face.plane.signed_distance(end).abs() > crate::TOLERANCE
    {
        return false;
    }
    let points: Vec<_> = face
        .outer
        .iter()
        .filter_map(|id| model.vertices.get(id).map(|v| v.position))
        .collect();
    if points.len() != face.outer.len() {
        return false;
    }
    for i in 0..points.len() {
        let a = points[i];
        let b = points[(i + 1) % points.len()];
        let c = points[(i + 2) % points.len()];
        let edge = b - a;
        let tolerance = crate::TOLERANCE * edge.length();
        if edge.cross(c - b).dot(face.plane.normal) < -tolerance
            || edge.cross(start - a).dot(face.plane.normal) < -tolerance
            || edge.cross(end - a).dot(face.plane.normal) < -tolerance
        {
            return false;
        }
    }
    true
}

fn fast_paths_enabled() -> bool {
    #[cfg(feature = "bench-fixtures")]
    {
        !crate::benchmark::reference_enabled()
    }
    #[cfg(not(feature = "bench-fixtures"))]
    {
        true
    }
}

#[cfg(test)]
mod tests {
    use crate::{Command, DVec3 as V, History};
    fn square(x: f64, y: f64) -> Command {
        Command::AddPolyline {
            points: vec![
                V::new(x, y, 0.),
                V::new(x + 1., y, 0.),
                V::new(x + 1., y + 1., 0.),
                V::new(x, y + 1., 0.),
            ],
            closed: true,
        }
    }
    #[test]
    fn local_edits_match_full_rebuild_on_connected_grid_and_styles_keep_adjacency() {
        let mut h = History::new();
        for y in 0..4 {
            for x in 0..4 {
                h.execute(square(x as f64, y as f64)).unwrap();
            }
        }
        for (start, end) in [
            (V::new(1., 1., 0.), V::new(2., 2., 0.)),
            (V::new(0., 0.5, 0.), V::new(4., 0.5, 0.)),
            (V::new(2., 0., 0.), V::new(2., 4., 0.)),
        ] {
            let command = Command::AddEdge { start, end };
            let mut reference = h.model().clone();
            reference.spatial = None;
            reference.next_id = h.highest_test_id();
            if command.apply(&mut reference, 0).unwrap() {
                reference.rebuild_faces().unwrap();
            }
            reference.validate().unwrap();
            h.execute(command).unwrap();
            assert_eq!(h.model().edges(), reference.edges());
            let total = |m: &crate::Model| {
                m.faces()
                    .keys()
                    .map(|id| m.face_area(*id).unwrap())
                    .sum::<f64>()
            };
            assert_eq!(h.model().faces().len(), reference.faces().len());
            assert!((total(h.model()) - total(&reference)).abs() < crate::TOLERANCE);
            h.model().validate().unwrap();
        }
        let edge = *h.model().edges().keys().next().unwrap();
        h.execute(Command::SetEdgeStyle {
            edges: vec![edge],
            soft: true,
            smooth: true,
        })
        .unwrap();
        let spatial = h.model().spatial.as_ref().unwrap();
        assert!(!spatial.edge_faces[&edge].is_empty());
        h.execute(Command::EraseEdges(vec![edge])).unwrap();
        h.model().validate().unwrap();
    }
    #[test]
    fn dirty_sets_are_local_and_reversible_without_history_model_copies() {
        let mut h = History::new();
        for i in 0..30 {
            h.execute(square(i as f64 * 3., 0.)).unwrap();
        }
        let unchanged: Vec<_> = h.model().faces().keys().skip(1).copied().collect();
        let before = h.model().clone();
        h.execute(Command::AddEdge {
            start: V::ZERO,
            end: V::new(1., 1., 0.),
        })
        .unwrap();
        assert!(h.changes().faces.len() <= 3);
        assert!(h.changes().edges.len() <= 1);
        assert!(unchanged.iter().all(|id| !h.changes().faces.contains(id)));
        let changed = h.changes().clone();
        h.undo().unwrap();
        assert_eq!(h.model(), &before);
        assert_eq!(h.changes().faces, changed.faces);
        h.redo().unwrap();
        h.model().validate().unwrap();
    }
}
