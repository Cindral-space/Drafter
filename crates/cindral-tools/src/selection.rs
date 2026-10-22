//! Rendering-independent selection queries.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use cindral_core::{Edge, EdgeId, Face, FaceId, Model, VertexId, TOLERANCE};
use cindral_inference::{Ray, View};
use glam::{DVec2, DVec3};

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
    /// Clear all selected entities.
    pub fn clear(&mut self) {
        self.vertices.clear();
        self.edges.clear();
        self.faces.clear();
    }

    /// Select a single entity, optionally adding to the current selection.
    pub fn select(&mut self, model: &Model, hit: Hit, add: bool) {
        let valid = match hit {
            Hit::Vertex(v) => model.vertices().contains_key(&v),
            Hit::Edge(e) => model.edges().contains_key(&e),
            Hit::Face(f) => model.faces().contains_key(&f),
        };
        if !valid {
            return;
        }
        if !add {
            self.clear();
        }
        match hit {
            Hit::Vertex(v) => {
                self.vertices.insert(v);
            }
            Hit::Edge(e) => {
                self.edges.insert(e);
            }
            Hit::Face(f) => {
                self.faces.insert(f);
            }
        }
    }

    /// Select all entities connected topologically to `hit`, optionally adding to selection.
    pub fn connected(&mut self, model: &Model, hit: Hit, add: bool) {
        let valid = match hit {
            Hit::Vertex(v) => model.vertices().contains_key(&v),
            Hit::Edge(e) => model.edges().contains_key(&e),
            Hit::Face(f) => model.faces().contains_key(&f),
        };
        if !valid {
            return;
        }
        if !add {
            self.clear();
        }

        let mut vertex_to_edges: BTreeMap<VertexId, Vec<EdgeId>> = BTreeMap::new();
        for (&edge_id, edge) in model.edges() {
            vertex_to_edges
                .entry(edge.vertices[0])
                .or_default()
                .push(edge_id);
            vertex_to_edges
                .entry(edge.vertices[1])
                .or_default()
                .push(edge_id);
        }

        let mut vertex_to_faces: BTreeMap<VertexId, Vec<FaceId>> = BTreeMap::new();
        for (&face_id, face) in model.faces() {
            for vid in &face.outer {
                vertex_to_faces.entry(*vid).or_default().push(face_id);
            }
            for hole in &face.holes {
                for vid in hole {
                    vertex_to_faces.entry(*vid).or_default().push(face_id);
                }
            }
        }

        let mut visited_vertices = BTreeSet::new();
        let mut queue = VecDeque::new();

        match hit {
            Hit::Vertex(v) => {
                visited_vertices.insert(v);
                queue.push_back(v);
            }
            Hit::Edge(e) => {
                if let Some(edge) = model.edges().get(&e) {
                    for &v in &edge.vertices {
                        if visited_vertices.insert(v) {
                            queue.push_back(v);
                        }
                    }
                }
            }
            Hit::Face(f) => {
                if let Some(face) = model.faces().get(&f) {
                    for &v in &face.outer {
                        if visited_vertices.insert(v) {
                            queue.push_back(v);
                        }
                    }
                    for hole in &face.holes {
                        for &v in hole {
                            if visited_vertices.insert(v) {
                                queue.push_back(v);
                            }
                        }
                    }
                }
            }
        }

        let mut visited_faces = BTreeSet::new();
        let mut visited_edges = BTreeSet::new();

        while let Some(v) = queue.pop_front() {
            if let Some(edges) = vertex_to_edges.get(&v) {
                for &eid in edges {
                    if visited_edges.insert(eid) {
                        if let Some(edge) = model.edges().get(&eid) {
                            for &endpoint in &edge.vertices {
                                if visited_vertices.insert(endpoint) {
                                    queue.push_back(endpoint);
                                }
                            }
                        }
                    }
                }
            }
            if let Some(faces) = vertex_to_faces.get(&v) {
                for &fid in faces {
                    if visited_faces.insert(fid) {
                        if let Some(face) = model.faces().get(&fid) {
                            for &v_out in &face.outer {
                                if visited_vertices.insert(v_out) {
                                    queue.push_back(v_out);
                                }
                            }
                            for hole in &face.holes {
                                for &v_hole in hole {
                                    if visited_vertices.insert(v_hole) {
                                        queue.push_back(v_hole);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        self.vertices.extend(visited_vertices);
        self.edges.extend(visited_edges);
        self.faces.extend(visited_faces);
    }

    /// Select entities in a screen rectangle: left-to-right for window (enclosed), right-to-left for crossing.
    pub fn region(&mut self, model: &Model, view: View, start: DVec2, end: DVec2, add: bool) {
        if !start.is_finite()
            || !end.is_finite()
            || !view.matrix.is_finite()
            || !view.size.is_finite()
            || view.size.min_element() <= 0.0
        {
            return;
        }

        let is_crossing = start.x > end.x;
        let rect_min = DVec2::new(start.x.min(end.x), start.y.min(end.y));
        let rect_max = DVec2::new(start.x.max(end.x), start.y.max(end.y));

        if !add {
            self.clear();
        }

        // 1. Vertices
        for (&vertex_id, vertex) in model.vertices() {
            let Some(screen_pos) = view.project(vertex.position) else {
                continue;
            };
            if point_in_rect(screen_pos, rect_min, rect_max) {
                self.vertices.insert(vertex_id);
            }
        }

        // 2. Edges
        for (&edge_id, edge) in model.edges() {
            let Some(va) = model.vertices().get(&edge.vertices[0]) else {
                continue;
            };
            let Some(vb) = model.vertices().get(&edge.vertices[1]) else {
                continue;
            };
            if !is_crossing {
                let Some(sa) = view.project(va.position) else {
                    continue;
                };
                let Some(sb) = view.project(vb.position) else {
                    continue;
                };
                if point_in_rect(sa, rect_min, rect_max) && point_in_rect(sb, rect_min, rect_max) {
                    self.edges.insert(edge_id);
                }
            } else if let (Some(sa), Some(sb)) =
                (view.project(va.position), view.project(vb.position))
            {
                if segment_intersects_rect(sa, sb, rect_min, rect_max) {
                    self.edges.insert(edge_id);
                }
            } else if let Some((sa, sb)) = clip_segment_to_screen(&view, va.position, vb.position) {
                if segment_intersects_rect(sa, sb, rect_min, rect_max) {
                    self.edges.insert(edge_id);
                }
            }
        }

        // 3. Faces
        let corners = [
            rect_min,
            DVec2::new(rect_max.x, rect_min.y),
            rect_max,
            DVec2::new(rect_min.x, rect_max.y),
            (rect_min + rect_max) * 0.5,
        ];

        for (&face_id, face) in model.faces() {
            let mut outer_screen = Vec::with_capacity(face.outer.len());
            let mut all_outer_projected = true;
            for vid in &face.outer {
                if let Some(v) = model.vertices().get(vid) {
                    if let Some(sp) = view.project(v.position) {
                        outer_screen.push(sp);
                    } else {
                        all_outer_projected = false;
                        break;
                    }
                } else {
                    all_outer_projected = false;
                    break;
                }
            }
            if !all_outer_projected || outer_screen.len() < 3 {
                continue;
            }

            let mut holes_screen = Vec::with_capacity(face.holes.len());
            let mut all_holes_projected = true;
            for hole in &face.holes {
                let mut h_screen = Vec::with_capacity(hole.len());
                for vid in hole {
                    if let Some(v) = model.vertices().get(vid) {
                        if let Some(sp) = view.project(v.position) {
                            h_screen.push(sp);
                        } else {
                            all_holes_projected = false;
                            break;
                        }
                    } else {
                        all_holes_projected = false;
                        break;
                    }
                }
                if !all_holes_projected || h_screen.len() < 3 {
                    all_holes_projected = false;
                    break;
                }
                holes_screen.push(h_screen);
            }
            if !all_holes_projected {
                continue;
            }

            if !is_crossing {
                let all_outer_inside = outer_screen
                    .iter()
                    .all(|p| point_in_rect(*p, rect_min, rect_max));
                let all_holes_inside = holes_screen
                    .iter()
                    .all(|h| h.iter().all(|p| point_in_rect(*p, rect_min, rect_max)));
                if all_outer_inside && all_holes_inside {
                    self.faces.insert(face_id);
                }
            } else {
                let any_vertex_in_rect = outer_screen
                    .iter()
                    .any(|p| point_in_rect(*p, rect_min, rect_max))
                    || holes_screen
                        .iter()
                        .any(|h| h.iter().any(|p| point_in_rect(*p, rect_min, rect_max)));

                let mut edge_crosses_rect = false;
                for i in 0..outer_screen.len() {
                    let p0 = outer_screen[i];
                    let p1 = outer_screen[(i + 1) % outer_screen.len()];
                    if segment_intersects_rect(p0, p1, rect_min, rect_max) {
                        edge_crosses_rect = true;
                        break;
                    }
                }
                if !edge_crosses_rect {
                    for h in &holes_screen {
                        for i in 0..h.len() {
                            let p0 = h[i];
                            let p1 = h[(i + 1) % h.len()];
                            if segment_intersects_rect(p0, p1, rect_min, rect_max) {
                                edge_crosses_rect = true;
                                break;
                            }
                        }
                        if edge_crosses_rect {
                            break;
                        }
                    }
                }

                let rect_inside_face = corners.iter().any(|&c| {
                    polygon_contains_inclusive(&outer_screen, c)
                        && !holes_screen.iter().any(|h| polygon_contains_strict(h, c))
                });

                if any_vertex_in_rect || edge_crosses_rect || rect_inside_face {
                    self.faces.insert(face_id);
                }
            }
        }
    }

    /// Resolve selected face/edge boundaries to unique vertex IDs.
    pub fn vertex_ids(&self, model: &Model) -> Vec<VertexId> {
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

/// Pick nearest visible face along `ray` (holes excluded) or screen-near vertex/edge respecting occlusion.
#[allow(dead_code)]
pub fn pick(model: &Model, ray: Ray, view: View, cursor: DVec2, radius: f64) -> Option<Hit> {
    pick_section(model, ray, view, cursor, radius, None)
}
/// Pick only geometry retained by the local section plane.
#[tracing::instrument(level = "info", skip_all, name = "pick.total")]
pub fn pick_section(
    model: &Model,
    ray: Ray,
    view: View,
    cursor: DVec2,
    radius: f64,
    section: Option<&cindral_core::SectionPlane>,
) -> Option<Hit> {
    if !cursor.is_finite()
        || !radius.is_finite()
        || radius < 0.0
        || !view.matrix.is_finite()
        || !view.size.is_finite()
        || view.size.min_element() <= 0.0
        || !ray.origin.is_finite()
        || !ray.direction.is_finite()
    {
        return None;
    }
    let ray_dir = ray.direction.try_normalize()?;

    // 1. Nearest visible face along picking ray
    let mut best_face: Option<(FaceId, f64)> = None;
    for (&face_id, face) in model.faces() {
        let Some((point, t)) = ray_face_intersection(model, face, ray.origin, ray_dir) else {
            continue;
        };
        if section.is_some_and(|p| !p.retains(point)) {
            continue;
        }
        if view.project(point).is_none() {
            continue;
        }
        if let Some((best_id, best_t)) = best_face {
            if (t - best_t).abs() > TOLERANCE {
                if t < best_t {
                    best_face = Some((face_id, t));
                }
            } else if face_id < best_id {
                best_face = Some((face_id, t));
            }
        } else {
            best_face = Some((face_id, t));
        }
    }

    let max_depth = best_face.map_or(f64::INFINITY, |(_, t)| t + TOLERANCE);

    // 2. Vertex candidates within screen radius
    let mut best_vertex: Option<(VertexId, f64, f64)> = None;
    for (&vertex_id, vertex) in model.vertices() {
        let pos = vertex.position;
        if section.is_some_and(|p| !p.retains(pos)) {
            continue;
        }
        if !pos.is_finite() {
            continue;
        }
        let Some(screen_pos) = view.project(pos) else {
            continue;
        };
        let screen_dist = screen_pos.distance(cursor);
        if !screen_dist.is_finite() || screen_dist > radius {
            continue;
        }
        let ray_depth = (pos - ray.origin).dot(ray_dir);
        if ray_depth < 0.0 || ray_depth > max_depth {
            continue;
        }
        if is_vertex_occluded(model, vertex_id, pos, &view, ray.origin, ray_dir, section) {
            continue;
        }
        let candidate = (vertex_id, screen_dist, ray_depth);
        if let Some((best_id, best_sdist, best_rdepth)) = best_vertex {
            if (screen_dist - best_sdist).abs() > TOLERANCE {
                if screen_dist < best_sdist {
                    best_vertex = Some(candidate);
                }
            } else if (ray_depth - best_rdepth).abs() > TOLERANCE {
                if ray_depth < best_rdepth {
                    best_vertex = Some(candidate);
                }
            } else if vertex_id < best_id {
                best_vertex = Some(candidate);
            }
        } else {
            best_vertex = Some(candidate);
        }
    }

    // 3. Edge candidates within screen radius
    let mut best_edge: Option<(EdgeId, f64, f64)> = None;
    for (&edge_id, edge) in model.edges() {
        let Some(va) = model.vertices().get(&edge.vertices[0]) else {
            continue;
        };
        let Some(vb) = model.vertices().get(&edge.vertices[1]) else {
            continue;
        };
        let [a, b] = if let Some(p) = section {
            let Some(segment) = p.clip_segment(va.position, vb.position) else {
                continue;
            };
            segment
        } else {
            [va.position, vb.position]
        };
        if !a.is_finite() || !b.is_finite() || a.distance(b) <= TOLERANCE {
            continue;
        }
        let Some(p_edge) = closest_point_on_segment_screen(a, b, cursor, &view) else {
            continue;
        };
        let Some(screen_pos) = view.project(p_edge) else {
            continue;
        };
        let screen_dist = screen_pos.distance(cursor);
        if !screen_dist.is_finite() || screen_dist > radius {
            continue;
        }
        let ray_depth = (p_edge - ray.origin).dot(ray_dir);
        if ray_depth < 0.0 || ray_depth > max_depth {
            continue;
        }
        if is_edge_occluded(model, edge, p_edge, &view, ray.origin, ray_dir, section) {
            continue;
        }
        let candidate = (edge_id, screen_dist, ray_depth);
        if let Some((best_id, best_sdist, best_rdepth)) = best_edge {
            if (screen_dist - best_sdist).abs() > TOLERANCE {
                if screen_dist < best_sdist {
                    best_edge = Some(candidate);
                }
            } else if (ray_depth - best_rdepth).abs() > TOLERANCE {
                if ray_depth < best_rdepth {
                    best_edge = Some(candidate);
                }
            } else if edge_id < best_id {
                best_edge = Some(candidate);
            }
        } else {
            best_edge = Some(candidate);
        }
    }

    // 4. Resolve between vertex and edge
    let best_sub: Option<Hit> = match (best_vertex, best_edge) {
        (Some((vid, vsdist, vdepth)), Some((eid, esdist, edepth))) => {
            if (vdepth - edepth).abs() > 1.0e-3 {
                if vdepth < edepth {
                    Some(Hit::Vertex(vid))
                } else {
                    Some(Hit::Edge(eid))
                }
            } else if (vsdist - esdist).abs() > TOLERANCE {
                if vsdist <= esdist {
                    Some(Hit::Vertex(vid))
                } else {
                    Some(Hit::Edge(eid))
                }
            } else {
                Some(Hit::Vertex(vid))
            }
        }
        (Some((vid, _, _)), None) => Some(Hit::Vertex(vid)),
        (None, Some((eid, _, _))) => Some(Hit::Edge(eid)),
        (None, None) => None,
    };

    if let Some(hit) = best_sub {
        return Some(hit);
    }

    if let Some((fid, _)) = best_face {
        return Some(Hit::Face(fid));
    }

    None
}

fn point_in_rect(p: DVec2, min: DVec2, max: DVec2) -> bool {
    p.x >= min.x - TOLERANCE
        && p.x <= max.x + TOLERANCE
        && p.y >= min.y - TOLERANCE
        && p.y <= max.y + TOLERANCE
}

fn segment_intersects_rect(p0: DVec2, p1: DVec2, min: DVec2, max: DVec2) -> bool {
    let dx = p1.x - p0.x;
    let dy = p1.y - p0.y;
    let mut t0 = 0.0_f64;
    let mut t1 = 1.0_f64;

    let p = [-dx, dx, -dy, dy];
    let q = [p0.x - min.x, max.x - p0.x, p0.y - min.y, max.y - p0.y];

    for i in 0..4 {
        if p[i].abs() <= TOLERANCE {
            if q[i] < -TOLERANCE {
                return false;
            }
        } else {
            let t = q[i] / p[i];
            if p[i] < 0.0 {
                if t > t1 {
                    return false;
                }
                if t > t0 {
                    t0 = t;
                }
            } else {
                if t < t0 {
                    return false;
                }
                if t < t1 {
                    t1 = t;
                }
            }
        }
    }
    t0 <= t1 + TOLERANCE
}

#[allow(dead_code)]
fn ray_face_intersection(
    model: &Model,
    face: &Face,
    ray_origin: DVec3,
    ray_dir: DVec3,
) -> Option<(DVec3, f64)> {
    if face.outer.len() < 3 {
        return None;
    }
    let normal = face.plane.normal.try_normalize()?;
    let denom = normal.dot(ray_dir);
    if denom.abs() <= TOLERANCE {
        return None;
    }
    let first_id = *face.outer.first()?;
    let origin = model.vertices().get(&first_id)?.position;
    if !origin.is_finite() {
        return None;
    }
    let t = (origin - ray_origin).dot(normal) / denom;
    if !t.is_finite() || t < 0.0 {
        return None;
    }
    let raw = ray_origin + ray_dir * t;
    let point = raw - normal * (raw - origin).dot(normal);
    if !point.is_finite() {
        return None;
    }

    let axis = if normal.x.abs() < 0.9 {
        DVec3::X
    } else {
        DVec3::Y
    };
    let u = normal.cross(axis).try_normalize()?;
    let v = normal.cross(u).try_normalize()?;

    let p2 = DVec2::new((point - origin).dot(u), (point - origin).dot(v));
    let outer_2d = project_ring_2d(model, &face.outer, origin, u, v)?;
    if !polygon_contains_inclusive(&outer_2d, p2) {
        return None;
    }
    for hole in &face.holes {
        let hole_2d = project_ring_2d(model, hole, origin, u, v)?;
        if polygon_contains_strict(&hole_2d, p2) {
            return None;
        }
    }
    Some((point, t))
}

#[allow(dead_code)]
fn project_ring_2d(
    model: &Model,
    ring: &[VertexId],
    origin: DVec3,
    u: DVec3,
    v: DVec3,
) -> Option<Vec<DVec2>> {
    if ring.len() < 3 {
        return None;
    }
    let mut projected = Vec::with_capacity(ring.len());
    for id in ring {
        let pos = model.vertices().get(id)?.position;
        if !pos.is_finite() {
            return None;
        }
        projected.push(DVec2::new((pos - origin).dot(u), (pos - origin).dot(v)));
    }
    Some(projected)
}

fn point_on_polygon_boundary(ring: &[DVec2], point: DVec2) -> bool {
    let n = ring.len();
    for i in 0..n {
        let a = ring[i];
        let b = ring[(i + 1) % n];
        let ab = b - a;
        let len_sq = ab.length_squared();
        let t = if len_sq <= TOLERANCE * TOLERANCE {
            0.0
        } else {
            ((point - a).dot(ab) / len_sq).clamp(0.0, 1.0)
        };
        if point.distance(a + ab * t) <= TOLERANCE {
            return true;
        }
    }
    false
}

fn polygon_contains_raycast(ring: &[DVec2], point: DVec2) -> bool {
    let mut inside = false;
    let n = ring.len();
    for i in 0..n {
        let a = ring[i];
        let b = ring[(i + 1) % n];
        if (a.y > point.y) != (b.y > point.y)
            && point.x < (b.x - a.x) * (point.y - a.y) / (b.y - a.y) + a.x
        {
            inside = !inside;
        }
    }
    inside
}

fn polygon_contains_inclusive(ring: &[DVec2], point: DVec2) -> bool {
    point_on_polygon_boundary(ring, point) || polygon_contains_raycast(ring, point)
}

fn polygon_contains_strict(ring: &[DVec2], point: DVec2) -> bool {
    !point_on_polygon_boundary(ring, point) && polygon_contains_raycast(ring, point)
}

fn clip_linear(t0: &mut f64, t1: &mut f64, v0: f64, v1: f64) -> bool {
    if v0 >= 0.0 && v1 >= 0.0 {
        return true;
    }
    if v0 < 0.0 && v1 < 0.0 {
        return false;
    }
    let dv = v1 - v0;
    if dv.abs() <= f64::EPSILON {
        return v0 >= 0.0 && v1 >= 0.0;
    }
    let root = -v0 / dv;
    if dv > 0.0 {
        *t0 = (*t0).max(root);
    } else {
        *t1 = (*t1).min(root);
    }
    *t0 <= *t1
}

fn clip_to_screen(clip: glam::DVec4, size: DVec2) -> Option<DVec2> {
    if !clip.is_finite() || clip.w <= 0.0 {
        return None;
    }
    let ndc = clip.truncate() / clip.w;
    if !ndc.is_finite() {
        return None;
    }
    Some(DVec2::new(
        (ndc.x + 1.0) * 0.5 * size.x,
        (1.0 - ndc.y) * 0.5 * size.y,
    ))
}

fn clip_segment_to_screen(view: &View, a: DVec3, b: DVec3) -> Option<(DVec2, DVec2)> {
    let ca = view.matrix * a.extend(1.0);
    let cb = view.matrix * b.extend(1.0);
    if !ca.is_finite() || !cb.is_finite() {
        return None;
    }
    let mut t0 = 0.0_f64;
    let mut t1 = 1.0_f64;
    if !clip_linear(&mut t0, &mut t1, ca.z, cb.z)
        || !clip_linear(&mut t0, &mut t1, ca.w - ca.z, cb.w - cb.z)
        || !clip_linear(&mut t0, &mut t1, ca.w - TOLERANCE, cb.w - TOLERANCE)
    {
        return None;
    }
    let a_clipped = a.lerp(b, t0);
    let b_clipped = a.lerp(b, t1);
    let ca_c = view.matrix * a_clipped.extend(1.0);
    let cb_c = view.matrix * b_clipped.extend(1.0);
    if ca_c.w <= 0.0 || cb_c.w <= 0.0 {
        return None;
    }
    let sa = clip_to_screen(ca_c, view.size)?;
    let sb = clip_to_screen(cb_c, view.size)?;
    Some((sa, sb))
}

#[allow(dead_code)]
fn closest_point_on_segment_screen(
    a: DVec3,
    b: DVec3,
    cursor: DVec2,
    view: &View,
) -> Option<DVec3> {
    let ca = view.matrix * a.extend(1.0);
    let cb = view.matrix * b.extend(1.0);
    if !ca.is_finite() || !cb.is_finite() {
        return None;
    }
    let mut t0 = 0.0_f64;
    let mut t1 = 1.0_f64;
    if !clip_linear(&mut t0, &mut t1, ca.z, cb.z)
        || !clip_linear(&mut t0, &mut t1, ca.w - ca.z, cb.w - cb.z)
        || !clip_linear(&mut t0, &mut t1, ca.w - TOLERANCE, cb.w - TOLERANCE)
    {
        return None;
    }
    let a_clipped = a.lerp(b, t0);
    let b_clipped = a.lerp(b, t1);
    let ca_c = view.matrix * a_clipped.extend(1.0);
    let cb_c = view.matrix * b_clipped.extend(1.0);
    if ca_c.w <= 0.0 || cb_c.w <= 0.0 {
        return None;
    }
    let sa = clip_to_screen(ca_c, view.size)?;
    let sb = clip_to_screen(cb_c, view.size)?;
    let delta = sb - sa;
    let len_sq = delta.length_squared();
    let s = if len_sq <= TOLERANCE * TOLERANCE {
        0.0
    } else {
        ((cursor - sa).dot(delta) / len_sq).clamp(0.0, 1.0)
    };
    let denom = (1.0 - s) * cb_c.w + s * ca_c.w;
    if !denom.is_finite() || denom <= 0.0 {
        return None;
    }
    let u = ((s * ca_c.w) / denom).clamp(0.0, 1.0);
    let point = a_clipped.lerp(b_clipped, u);
    point.is_finite().then_some(point)
}

#[allow(dead_code)]
#[tracing::instrument(level = "info", skip_all, name = "pick.vertex_occlusion")]
fn is_vertex_occluded(
    model: &Model,
    vertex_id: VertexId,
    pos: DVec3,
    view: &View,
    ray_origin: DVec3,
    ray_dir: DVec3,
    section: Option<&cindral_core::SectionPlane>,
) -> bool {
    let inv_vp = view.matrix.inverse();
    let (test_origin, test_dir, target_dist) = if inv_vp.is_finite() {
        if let Some(screen) = view.project(pos) {
            let ndc_x = (screen.x / view.size.x) * 2.0 - 1.0;
            let ndc_y = 1.0 - (screen.y / view.size.y) * 2.0;
            let near = inv_vp.project_point3(DVec3::new(ndc_x, ndc_y, 0.0));
            let far = inv_vp.project_point3(DVec3::new(ndc_x, ndc_y, 1.0));
            if let Some(dir) = (far - near).try_normalize() {
                let dist = (pos - near).dot(dir);
                (near, dir, dist)
            } else {
                let dist = (pos - ray_origin).dot(ray_dir);
                (ray_origin, ray_dir, dist)
            }
        } else {
            let dist = (pos - ray_origin).dot(ray_dir);
            (ray_origin, ray_dir, dist)
        }
    } else {
        let dist = (pos - ray_origin).dot(ray_dir);
        (ray_origin, ray_dir, dist)
    };

    if target_dist <= TOLERANCE {
        return false;
    }

    for face in model.faces().values() {
        if face.outer.contains(&vertex_id) || face.holes.iter().any(|h| h.contains(&vertex_id)) {
            continue;
        }
        let Some((point, t)) = ray_face_intersection(model, face, test_origin, test_dir) else {
            continue;
        };
        if section.is_some_and(|p| !p.retains(point)) {
            continue;
        }
        if t < target_dist - TOLERANCE {
            return true;
        }
    }
    false
}

#[allow(dead_code)]
#[tracing::instrument(level = "info", skip_all, name = "pick.edge_occlusion")]
fn is_edge_occluded(
    model: &Model,
    edge: &Edge,
    point: DVec3,
    view: &View,
    ray_origin: DVec3,
    ray_dir: DVec3,
    section: Option<&cindral_core::SectionPlane>,
) -> bool {
    let inv_vp = view.matrix.inverse();
    let (test_origin, test_dir, target_dist) = if inv_vp.is_finite() {
        if let Some(screen) = view.project(point) {
            let ndc_x = (screen.x / view.size.x) * 2.0 - 1.0;
            let ndc_y = 1.0 - (screen.y / view.size.y) * 2.0;
            let near = inv_vp.project_point3(DVec3::new(ndc_x, ndc_y, 0.0));
            let far = inv_vp.project_point3(DVec3::new(ndc_x, ndc_y, 1.0));
            if let Some(dir) = (far - near).try_normalize() {
                let dist = (point - near).dot(dir);
                (near, dir, dist)
            } else {
                let dist = (point - ray_origin).dot(ray_dir);
                (ray_origin, ray_dir, dist)
            }
        } else {
            let dist = (point - ray_origin).dot(ray_dir);
            (ray_origin, ray_dir, dist)
        }
    } else {
        let dist = (point - ray_origin).dot(ray_dir);
        (ray_origin, ray_dir, dist)
    };

    if target_dist <= TOLERANCE {
        return false;
    }

    let [v0, v1] = edge.vertices;
    for face in model.faces().values() {
        let in_outer = face.outer.contains(&v0) && face.outer.contains(&v1);
        let in_hole = face
            .holes
            .iter()
            .any(|h| h.contains(&v0) && h.contains(&v1));
        if in_outer || in_hole {
            continue;
        }
        let Some((point, t)) = ray_face_intersection(model, face, test_origin, test_dir) else {
            continue;
        };
        if section.is_some_and(|p| !p.retains(point)) {
            continue;
        }
        if t < target_dist - TOLERANCE {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use cindral_core::{Command, History};
    use glam::DMat4;

    struct TestCamera {
        eye: DVec3,
        view: View,
        inv_vp: DMat4,
    }

    impl TestCamera {
        fn orthographic(
            eye: DVec3,
            target: DVec3,
            up: DVec3,
            half_extents: DVec2,
            size: DVec2,
        ) -> Self {
            let view_mat = DMat4::look_at_rh(eye, target, up);
            let proj_mat = DMat4::orthographic_rh(
                -half_extents.x,
                half_extents.x,
                -half_extents.y,
                half_extents.y,
                0.1,
                1000.0,
            );
            let vp = proj_mat * view_mat;
            Self {
                eye,
                view: View { matrix: vp, size },
                inv_vp: vp.inverse(),
            }
        }

        fn ray_at(&self, cursor: DVec2) -> Ray {
            let ndc_x = (cursor.x / self.view.size.x) * 2.0 - 1.0;
            let ndc_y = 1.0 - (cursor.y / self.view.size.y) * 2.0;
            let near = self.inv_vp.project_point3(DVec3::new(ndc_x, ndc_y, 0.0));
            let far = self.inv_vp.project_point3(DVec3::new(ndc_x, ndc_y, 1.0));
            let origin = if (near - self.eye).length() <= 1.0e-3 {
                self.eye
            } else {
                near
            };
            Ray {
                origin,
                direction: far - near,
            }
        }

        fn pick_world(&self, model: &Model, world: DVec3, radius: f64) -> Option<Hit> {
            let cursor = self.view.project(world)?;
            let ray = self.ray_at(cursor);
            pick(model, ray, self.view, cursor, radius)
        }
    }

    #[test]
    fn test_face_occlusion_and_holes() {
        let mut history = History::new();

        // 1. Bottom face at z=0: (-5, -5, 0) to (5, 5, 0)
        history
            .execute(Command::AddPolyline {
                points: vec![
                    DVec3::new(-5.0, -5.0, 0.0),
                    DVec3::new(5.0, -5.0, 0.0),
                    DVec3::new(5.0, 5.0, 0.0),
                    DVec3::new(-5.0, 5.0, 0.0),
                ],
                closed: true,
            })
            .expect("bottom face");

        // 2. Face with hole at z=0:
        // Inner loop creating a hole at z=0: (-1, -1, 0) to (1, 1, 0)
        history
            .execute(Command::AddPolyline {
                points: vec![
                    DVec3::new(-1.0, -1.0, 0.0),
                    DVec3::new(1.0, -1.0, 0.0),
                    DVec3::new(1.0, 1.0, 0.0),
                    DVec3::new(-1.0, 1.0, 0.0),
                ],
                closed: true,
            })
            .expect("inner loop");

        let outer_face_id = *history
            .model()
            .faces()
            .iter()
            .find(|(_, f)| !f.holes.is_empty())
            .map(|(id, _)| id)
            .expect("outer face with hole exists");
        let inner_face_id = *history
            .model()
            .faces()
            .keys()
            .find(|id| **id != outer_face_id)
            .expect("inner face exists");

        // Verify ID ordering: outer_face_id < inner_face_id
        assert!(outer_face_id < inner_face_id);

        let cam = TestCamera::orthographic(
            DVec3::new(0.0, 0.0, 10.0),
            DVec3::ZERO,
            DVec3::Y,
            DVec2::new(6.0, 6.0),
            DVec2::new(600.0, 600.0),
        );

        // A. Point on the outer annulus (2.5, 0, 0) -> hits outer_face_id
        let hit_annulus = cam
            .pick_world(history.model(), DVec3::new(2.5, 0.0, 0.0), 5.0)
            .expect("annulus hit");
        assert_eq!(hit_annulus, Hit::Face(outer_face_id));

        // B. Point inside the hole (0, 0, 0):
        // outer_face_id must reject it because it is in outer_face_id's hole!
        // Because outer_face_id < inner_face_id, if hole was ignored, outer_face_id would win.
        // Respecting holes ensures inner_face_id is picked!
        let hit_hole = cam
            .pick_world(history.model(), DVec3::new(0.0, 0.0, 0.0), 5.0)
            .expect("inner face hit");
        assert_eq!(hit_hole, Hit::Face(inner_face_id));

        // 3. Elevated face at z=2: (-3, -3, 2) to (3, 3, 2)
        history
            .execute(Command::AddPolyline {
                points: vec![
                    DVec3::new(-3.0, -3.0, 2.0),
                    DVec3::new(3.0, -3.0, 2.0),
                    DVec3::new(3.0, 3.0, 2.0),
                    DVec3::new(-3.0, 3.0, 2.0),
                ],
                closed: true,
            })
            .expect("elevated face outer");

        let elevated_face_id = *history
            .model()
            .faces()
            .iter()
            .find(|(_, f)| (f.plane.distance - 2.0).abs() <= TOLERANCE)
            .map(|(id, _)| id)
            .expect("elevated face exists");

        // C. Face occlusion:
        // Ray at (2.0, 0.0) hits both elevated face at z=2 and bottom face at z=0.
        // Nearest visible ray face (z=2) must win!
        let hit_elevated = cam
            .pick_world(history.model(), DVec3::new(2.0, 0.0, 2.0), 5.0)
            .expect("elevated face hit");
        assert_eq!(hit_elevated, Hit::Face(elevated_face_id));

        // Ray at (4.0, 0.0) misses elevated face (which ends at x=3) and hits bottom face at z=0!
        let hit_bottom = cam
            .pick_world(history.model(), DVec3::new(4.0, 0.0, 0.0), 5.0)
            .expect("bottom face hit");
        assert_eq!(hit_bottom, Hit::Face(outer_face_id));

        // D. Vertex/Edge occlusion:
        // An edge on the bottom face at (2.0, 0.0, 0.0) is under the solid elevated face at z=2.
        history
            .execute(Command::AddEdge {
                start: DVec3::new(1.9, 0.0, 0.0),
                end: DVec3::new(2.1, 0.0, 0.0),
            })
            .expect("bottom edge under elevated face");

        // Cursor at (2.0, 0.0) with large radius (15px):
        // The bottom edge is directly under cursor, but is occluded by elevated face at z=2!
        let hit_occluded = cam
            .pick_world(history.model(), DVec3::new(2.0, 0.0, 2.0), 15.0)
            .expect("hit foreground");
        // Must pick the foreground face, NOT the occluded bottom edge!
        assert_eq!(
            hit_occluded,
            Hit::Face(elevated_face_id),
            "occluded entity at z=0 must not be picked over foreground face at z=2"
        );

        // A vertex on the elevated face itself (e.g. at (3.0, 3.0, 2.0)) is in front and NOT occluded!
        let hit_front_vertex = cam
            .pick_world(history.model(), DVec3::new(3.0, 3.0, 2.0), 5.0)
            .expect("hit front vertex");
        match hit_front_vertex {
            Hit::Vertex(vid) => {
                let v = history.model().vertices().get(&vid).unwrap();
                assert!((v.position.z - 2.0).abs() <= TOLERANCE);
            }
            Hit::Edge(eid) => {
                let e = history.model().edges().get(&eid).unwrap();
                let v = history.model().vertices().get(&e.vertices[0]).unwrap();
                assert!((v.position.z - 2.0).abs() <= TOLERANCE);
            }
            _ => panic!("expected front vertex/edge hit"),
        }
    }

    #[test]
    fn test_click_add() {
        let mut history = History::new();
        history
            .execute(Command::AddPolyline {
                points: vec![
                    DVec3::new(0.0, 0.0, 0.0),
                    DVec3::new(2.0, 0.0, 0.0),
                    DVec3::new(2.0, 2.0, 0.0),
                    DVec3::new(0.0, 2.0, 0.0),
                ],
                closed: true,
            })
            .unwrap();

        let fid = *history.model().faces().keys().next().unwrap();
        let eid = *history.model().edges().keys().next().unwrap();
        let vid = *history.model().vertices().keys().next().unwrap();

        let mut sel = Selection::default();

        // 1. Click without add: selects single entity
        sel.select(history.model(), Hit::Face(fid), false);
        assert_eq!(sel.faces.len(), 1);
        assert!(sel.faces.contains(&fid));
        assert!(sel.edges.is_empty());
        assert!(sel.vertices.is_empty());

        // 2. Click with add: adds second entity
        sel.select(history.model(), Hit::Edge(eid), true);
        assert_eq!(sel.faces.len(), 1);
        assert_eq!(sel.edges.len(), 1);
        assert!(sel.faces.contains(&fid));
        assert!(sel.edges.contains(&eid));

        // 3. Click with add: adds third entity
        sel.select(history.model(), Hit::Vertex(vid), true);
        assert_eq!(sel.faces.len(), 1);
        assert_eq!(sel.edges.len(), 1);
        assert_eq!(sel.vertices.len(), 1);

        // 4. Click without add: replaces entire selection
        sel.select(history.model(), Hit::Face(fid), false);
        assert_eq!(sel.faces.len(), 1);
        assert!(sel.edges.is_empty());
        assert!(sel.vertices.is_empty());

        // 5. Clear
        sel.clear();
        assert!(sel.faces.is_empty());
        assert!(sel.edges.is_empty());
        assert!(sel.vertices.is_empty());
    }

    #[test]
    fn test_connected_disconnected() {
        let mut history = History::new();

        // Component A: square at (0,0,0)..(2,2,0)
        history
            .execute(Command::AddPolyline {
                points: vec![
                    DVec3::new(0.0, 0.0, 0.0),
                    DVec3::new(2.0, 0.0, 0.0),
                    DVec3::new(2.0, 2.0, 0.0),
                    DVec3::new(0.0, 2.0, 0.0),
                ],
                closed: true,
            })
            .unwrap();

        // Attached wire edge to Component A from (2,2,0) to (3,3,0)
        history
            .execute(Command::AddEdge {
                start: DVec3::new(2.0, 2.0, 0.0),
                end: DVec3::new(3.0, 3.0, 0.0),
            })
            .unwrap();

        // Component B (disconnected): square at (10,0,0)..(12,2,0)
        history
            .execute(Command::AddPolyline {
                points: vec![
                    DVec3::new(10.0, 0.0, 0.0),
                    DVec3::new(12.0, 0.0, 0.0),
                    DVec3::new(12.0, 2.0, 0.0),
                    DVec3::new(10.0, 2.0, 0.0),
                ],
                closed: true,
            })
            .unwrap();

        let model = history.model();
        assert_eq!(model.faces().len(), 2);

        let face_a = *model
            .faces()
            .iter()
            .find(|(_, f)| {
                let pos = model.vertices().get(&f.outer[0]).unwrap().position;
                pos.x <= 2.0
            })
            .map(|(id, _)| id)
            .unwrap();

        let face_b = *model
            .faces()
            .iter()
            .find(|(_, f)| {
                let pos = model.vertices().get(&f.outer[0]).unwrap().position;
                pos.x >= 10.0
            })
            .map(|(id, _)| id)
            .unwrap();

        let mut sel = Selection::default();

        // 1. Select connected on Component A (face A)
        sel.connected(model, Hit::Face(face_a), false);

        assert!(sel.faces.contains(&face_a));
        assert!(!sel.faces.contains(&face_b));
        // Verify all selected entities belong to Component A (x <= 4)
        for &vid in &sel.vertices {
            assert!(model.vertices().get(&vid).unwrap().position.x <= 4.0);
        }
        for &eid in &sel.edges {
            let edge = model.edges().get(&eid).unwrap();
            let p0 = model.vertices().get(&edge.vertices[0]).unwrap().position;
            assert!(p0.x <= 4.0);
        }

        // 2. Add connected on Component B
        sel.connected(model, Hit::Face(face_b), true);
        assert!(sel.faces.contains(&face_a));
        assert!(sel.faces.contains(&face_b));

        // 3. Select connected without add on Component B: clears Component A
        sel.connected(model, Hit::Face(face_b), false);
        assert!(!sel.faces.contains(&face_a));
        assert!(sel.faces.contains(&face_b));
        for &vid in &sel.vertices {
            assert!(model.vertices().get(&vid).unwrap().position.x >= 9.0);
        }
    }

    #[test]
    fn test_left_to_right_window_vs_right_to_left_crossing() {
        let mut history = History::new();

        // Large face F from (-10,-10,0) to (10,10,0)
        history
            .execute(Command::AddPolyline {
                points: vec![
                    DVec3::new(-10.0, -10.0, 0.0),
                    DVec3::new(10.0, -10.0, 0.0),
                    DVec3::new(10.0, 10.0, 0.0),
                    DVec3::new(-10.0, 10.0, 0.0),
                ],
                closed: true,
            })
            .unwrap();

        // Edge crossing rectangle from (-2, 0, 0) to (2, 0, 0)
        history
            .execute(Command::AddEdge {
                start: DVec3::new(-2.0, 0.0, 0.0),
                end: DVec3::new(2.0, 0.0, 0.0),
            })
            .unwrap();

        let model = history.model();
        let cam = TestCamera::orthographic(
            DVec3::new(0.0, 0.0, 10.0),
            DVec3::ZERO,
            DVec3::Y,
            DVec2::new(10.0, 10.0),
            DVec2::new(1000.0, 1000.0),
        );

        // Center on screen is (500, 500) corresponding to (0, 0) in world.
        // Scale is 1000px / 20m = 50px per meter.

        // Part 1: Face containing rectangle:
        // A rectangle wholly inside the face at (2, 2)..(4, 4) in world
        // Screen coords: (600, 300) to (700, 400).
        let p_start_l2r = cam.view.project(DVec3::new(2.0, 4.0, 0.0)).unwrap();
        let p_end_l2r = cam.view.project(DVec3::new(4.0, 2.0, 0.0)).unwrap();
        assert!(p_start_l2r.x < p_end_l2r.x);

        // Left-to-right (Window): face contains rectangle, so face is NOT fully enclosed!
        let mut sel_window = Selection::default();
        sel_window.region(model, cam.view, p_start_l2r, p_end_l2r, false);
        assert!(
            sel_window.faces.is_empty(),
            "face must not be enclosed in rectangle"
        );

        // Right-to-left (Crossing): rectangle wholly inside face -> face MUST be selected!
        let mut sel_crossing = Selection::default();
        sel_crossing.region(model, cam.view, p_end_l2r, p_start_l2r, false);
        assert!(
            !sel_crossing.faces.is_empty(),
            "crossing selection must select face containing rectangle"
        );

        // Part 2: Edge crossing rectangle:
        // Edge is from (-2, 0, 0) to (2, 0, 0).
        // Rectangle from (-1, -1, 0) to (1, 1, 0).
        // Endpoints of edge are outside the rectangle (-2 and +2), but edge crosses the rectangle!
        let rect_cross_start_l2r = cam.view.project(DVec3::new(-1.0, 1.0, 0.0)).unwrap();
        let rect_cross_end_l2r = cam.view.project(DVec3::new(1.0, -1.0, 0.0)).unwrap();
        assert!(rect_cross_start_l2r.x < rect_cross_end_l2r.x);

        let cross_edge_id = *model
            .edges()
            .iter()
            .find(|(_, e)| {
                let v0 = model.vertices().get(&e.vertices[0]).unwrap().position;
                let v1 = model.vertices().get(&e.vertices[1]).unwrap().position;
                (v0.y.abs() <= TOLERANCE && v1.y.abs() <= TOLERANCE)
                    && (v0.x.abs() >= 1.5 || v1.x.abs() >= 1.5)
            })
            .map(|(id, _)| id)
            .unwrap();

        // Left-to-right (Window): edge endpoints are outside rectangle -> edge NOT selected
        let mut edge_window = Selection::default();
        edge_window.region(
            model,
            cam.view,
            rect_cross_start_l2r,
            rect_cross_end_l2r,
            false,
        );
        assert!(
            !edge_window.edges.contains(&cross_edge_id),
            "crossing edge must not be selected by window"
        );

        // Right-to-left (Crossing): edge crosses rectangle -> edge IS selected
        let mut edge_crossing = Selection::default();
        edge_crossing.region(
            model,
            cam.view,
            rect_cross_end_l2r,
            rect_cross_start_l2r,
            false,
        );
        assert!(
            edge_crossing.edges.contains(&cross_edge_id),
            "crossing edge must be selected by crossing"
        );
    }
}
