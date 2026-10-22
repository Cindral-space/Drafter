use cindral_core::{Face, Model, TOLERANCE};
use glam::{DVec2, DVec3};

use crate::{Axis, Query, Snap, SnapEntity, SnapKind, TangentHint, View};

/// Screen-space geometric inference engine.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Engine {
    /// Maximum screen-space snap distance in logical pixels.
    pub radius_pixels: f64,
}

impl Default for Engine {
    fn default() -> Self {
        Self {
            radius_pixels: 12.0,
        }
    }
}

impl Engine {
    /// Construct an inference engine with the default 12-pixel snap radius.
    pub const fn new() -> Self {
        Self {
            radius_pixels: 12.0,
        }
    }

    /// Infer the highest-priority geometric snap or work-plane fallback for `query`.
    #[tracing::instrument(level = "info", skip_all, name = "inference.scan")]
    pub fn infer(&self, model: &Model, query: &Query) -> Option<Snap> {
        self.infer_retained(model, query).filter(|snap| {
            query
                .section
                .as_ref()
                .is_none_or(|p| p.retains(snap.position))
        })
    }
    fn infer_retained(&self, model: &Model, query: &Query) -> Option<Snap> {
        if !self.radius_pixels.is_finite() || self.radius_pixels <= 0.0 {
            return None;
        }
        let ray_dir = validate_query(query)?;
        let plane_normal = normalize_vec(query.plane.normal)?;

        if let (Some(anchor), Some(locked_axis)) = (query.anchor, query.axis_lock) {
            return self.infer_axis_locked(model, query, anchor, locked_axis, ray_dir);
        }

        if let Some(best) = self.best_vertex_candidate(model, query, ray_dir, None) {
            return Some(best.snap);
        }
        if let Some(best) = self.best_midpoint_candidate(model, query, ray_dir, None) {
            return Some(best.snap);
        }
        if let Some(best) = self.best_on_edge_candidate(model, query, ray_dir, None) {
            return Some(best.snap);
        }
        if let Some(best) = self.best_guide_candidate(query, ray_dir, plane_normal) {
            return Some(best.snap);
        }
        if let Some(best) = self.best_face_candidate(model, query, ray_dir) {
            return Some(best.snap);
        }

        self.plane_fallback(query, plane_normal)
    }

    fn infer_axis_locked(
        &self,
        model: &Model,
        query: &Query,
        anchor: DVec3,
        locked_axis: Axis,
        ray_dir: DVec3,
    ) -> Option<Snap> {
        let lock_line = (anchor, locked_axis.direction());
        if let Some(best) = self.best_vertex_candidate(model, query, ray_dir, Some(lock_line)) {
            return Some(best.snap);
        }
        if let Some(best) = self.best_midpoint_candidate(model, query, ray_dir, Some(lock_line)) {
            return Some(best.snap);
        }
        if let Some(best) = self.best_on_edge_candidate(model, query, ray_dir, Some(lock_line)) {
            return Some(best.snap);
        }

        let position = closest_point_on_line_to_ray(
            anchor,
            locked_axis.direction(),
            query.ray.origin,
            ray_dir,
        )
        .or_else(|| {
            let (alt_origin, alt_dir) = fallback_cursor_ray(query)?;
            closest_point_on_line_to_ray(anchor, locked_axis.direction(), alt_origin, alt_dir)
        })?;
        let (screen_pos, _) = project_with_depth(&query.view, position)?;
        let screen_distance = screen_pos.distance(query.cursor);
        if !screen_distance.is_finite() {
            return None;
        }
        Some(Snap {
            position,
            kind: SnapKind::Axis(locked_axis),
            screen_distance,
            guide: Some([anchor, position]),
            entity: None,
        })
    }

    #[tracing::instrument(level = "info", skip_all, name = "inference.best_vertex_candidate")]
    fn best_vertex_candidate(
        &self,
        model: &Model,
        query: &Query,
        ray_dir: DVec3,
        axis_lock: Option<(DVec3, DVec3)>,
    ) -> Option<Candidate> {
        let mut best: Option<Candidate> = None;
        for (&vertex_id, vertex) in model.vertices() {
            let mut position = vertex.position;
            if !position.is_finite() {
                continue;
            }
            if query
                .anchor
                .is_some_and(|anchor| position.distance(anchor) <= TOLERANCE)
            {
                continue;
            }
            let guide = if let Some((lock_origin, lock_dir)) = axis_lock {
                let Some(projected) =
                    project_onto_line_within_tolerance(position, lock_origin, lock_dir)
                else {
                    continue;
                };
                position = projected;
                Some([lock_origin, position])
            } else {
                None
            };

            let Some((screen_pos, ndc_depth)) = project_with_depth(&query.view, position) else {
                continue;
            };
            let screen_distance = screen_pos.distance(query.cursor);
            if !screen_distance.is_finite() || screen_distance > self.radius_pixels {
                continue;
            }
            let degree = model.incident_edges(vertex_id).count();
            let (kind, sub_priority) = if degree >= 3 {
                (SnapKind::Intersection, 0)
            } else {
                (SnapKind::Endpoint, 1)
            };
            if query.section.as_ref().is_some_and(|p| !p.retains(position)) {
                continue;
            }
            let candidate = Candidate {
                snap: Snap {
                    position,
                    kind,
                    screen_distance,
                    guide,
                    entity: Some(SnapEntity::Vertex(vertex_id)),
                },
                sub_priority,
                ray_depth: (position - query.ray.origin).dot(ray_dir),
                ndc_depth,
                entity_order: vertex_id.value(),
            };
            update_best(&mut best, candidate);
        }
        best
    }

    #[tracing::instrument(level = "info", skip_all, name = "inference.best_midpoint_candidate")]
    fn best_midpoint_candidate(
        &self,
        model: &Model,
        query: &Query,
        ray_dir: DVec3,
        axis_lock: Option<(DVec3, DVec3)>,
    ) -> Option<Candidate> {
        let mut best: Option<Candidate> = None;
        for (&edge_id, edge) in model.edges() {
            let Some(va) = model.vertices().get(&edge.vertices[0]) else {
                continue;
            };
            let Some(vb) = model.vertices().get(&edge.vertices[1]) else {
                continue;
            };
            if !va.position.is_finite()
                || !vb.position.is_finite()
                || va.position.distance(vb.position) <= TOLERANCE
            {
                continue;
            }
            let mut position = (va.position + vb.position) * 0.5;
            if query
                .anchor
                .is_some_and(|anchor| position.distance(anchor) <= TOLERANCE)
            {
                continue;
            }
            let guide = if let Some((lock_origin, lock_dir)) = axis_lock {
                let Some(projected) =
                    project_onto_line_within_tolerance(position, lock_origin, lock_dir)
                else {
                    continue;
                };
                position = projected;
                Some([lock_origin, position])
            } else {
                None
            };

            let Some((screen_pos, ndc_depth)) = project_with_depth(&query.view, position) else {
                continue;
            };
            let screen_distance = screen_pos.distance(query.cursor);
            if !screen_distance.is_finite() || screen_distance > self.radius_pixels {
                continue;
            }
            if query.section.as_ref().is_some_and(|p| !p.retains(position)) {
                continue;
            }
            let candidate = Candidate {
                snap: Snap {
                    position,
                    kind: SnapKind::Midpoint,
                    screen_distance,
                    guide,
                    entity: Some(SnapEntity::Edge(edge_id)),
                },
                sub_priority: 0,
                ray_depth: (position - query.ray.origin).dot(ray_dir),
                ndc_depth,
                entity_order: edge_id.value(),
            };
            update_best(&mut best, candidate);
        }
        best
    }

    #[tracing::instrument(level = "info", skip_all, name = "inference.best_on_edge_candidate")]
    fn best_on_edge_candidate(
        &self,
        model: &Model,
        query: &Query,
        ray_dir: DVec3,
        axis_lock: Option<(DVec3, DVec3)>,
    ) -> Option<Candidate> {
        let mut best: Option<Candidate> = None;
        for (&edge_id, edge) in model.edges() {
            let Some(va) = model.vertices().get(&edge.vertices[0]) else {
                continue;
            };
            let Some(vb) = model.vertices().get(&edge.vertices[1]) else {
                continue;
            };
            let [a, b] = if let Some(p) = &query.section {
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

            let (position, guide) = if let Some((lock_origin, lock_dir)) = axis_lock {
                let Some(pos) = locked_edge_point(a, b, lock_origin, lock_dir, query) else {
                    continue;
                };
                (pos, Some([lock_origin, pos]))
            } else {
                let Some(pos) = closest_point_on_segment_screen(a, b, query.cursor, &query.view)
                else {
                    continue;
                };
                (pos, None)
            };

            if query
                .anchor
                .is_some_and(|anchor| position.distance(anchor) <= TOLERANCE)
            {
                continue;
            }
            let Some((screen_pos, ndc_depth)) = project_with_depth(&query.view, position) else {
                continue;
            };
            let screen_distance = screen_pos.distance(query.cursor);
            if !screen_distance.is_finite() || screen_distance > self.radius_pixels {
                continue;
            }
            if query.section.as_ref().is_some_and(|p| !p.retains(position)) {
                continue;
            }
            let candidate = Candidate {
                snap: Snap {
                    position,
                    kind: SnapKind::OnEdge,
                    screen_distance,
                    guide,
                    entity: Some(SnapEntity::Edge(edge_id)),
                },
                sub_priority: 0,
                ray_depth: (position - query.ray.origin).dot(ray_dir),
                ndc_depth,
                entity_order: edge_id.value(),
            };
            update_best(&mut best, candidate);
        }
        best
    }

    fn best_guide_candidate(
        &self,
        query: &Query,
        ray_dir: DVec3,
        plane_normal: DVec3,
    ) -> Option<Candidate> {
        let mut best: Option<Candidate> = None;

        if let Some(tangent) = query.tangent {
            if let Some(dir) = tangent_direction(tangent) {
                self.consider_guide_line(
                    &mut best,
                    query,
                    ray_dir,
                    tangent.reference,
                    dir,
                    SnapKind::Tangent,
                    0,
                );
            }
        }

        if let Some(anchor) = query.anchor {
            if let Some(prev) = query.previous_direction {
                if let Some(par_dir) = normalize_vec(prev) {
                    self.consider_guide_line(
                        &mut best,
                        query,
                        ray_dir,
                        anchor,
                        par_dir,
                        SnapKind::Parallel,
                        1,
                    );
                    if let Some(perp_dir) = normalize_vec(plane_normal.cross(par_dir)) {
                        self.consider_guide_line(
                            &mut best,
                            query,
                            ray_dir,
                            anchor,
                            perp_dir,
                            SnapKind::Perpendicular,
                            2,
                        );
                    }
                }
            }
            for (sub_priority, axis) in [(3, Axis::X), (4, Axis::Y), (5, Axis::Z)] {
                self.consider_guide_line(
                    &mut best,
                    query,
                    ray_dir,
                    anchor,
                    axis.direction(),
                    SnapKind::Axis(axis),
                    sub_priority,
                );
            }
        }

        best
    }

    #[allow(clippy::too_many_arguments)]
    fn consider_guide_line(
        &self,
        best: &mut Option<Candidate>,
        query: &Query,
        ray_dir: DVec3,
        line_origin: DVec3,
        line_dir: DVec3,
        kind: SnapKind,
        sub_priority: u8,
    ) {
        let primary =
            closest_point_on_line_to_ray(line_origin, line_dir, query.ray.origin, ray_dir)
                .filter(|pos| pos.distance(line_origin) > TOLERANCE)
                .and_then(|pos| {
                    let (screen_pos, ndc_depth) = project_with_depth(&query.view, pos)?;
                    let dist = screen_pos.distance(query.cursor);
                    (dist.is_finite() && dist <= self.radius_pixels)
                        .then_some((pos, dist, ndc_depth))
                });

        let resolved = primary.or_else(|| {
            let (alt_origin, alt_dir) = fallback_cursor_ray(query)?;
            let pos = closest_point_on_line_to_ray(line_origin, line_dir, alt_origin, alt_dir)?;
            if pos.distance(line_origin) <= TOLERANCE {
                return None;
            }
            let (screen_pos, ndc_depth) = project_with_depth(&query.view, pos)?;
            let dist = screen_pos.distance(query.cursor);
            (dist.is_finite() && dist <= self.radius_pixels).then_some((pos, dist, ndc_depth))
        });

        let Some((position, screen_distance, ndc_depth)) = resolved else {
            return;
        };
        let candidate = Candidate {
            snap: Snap {
                position,
                kind,
                screen_distance,
                guide: Some([line_origin, position]),
                entity: None,
            },
            sub_priority,
            ray_depth: (position - query.ray.origin).dot(ray_dir),
            ndc_depth,
            entity_order: u64::MAX,
        };
        update_best(best, candidate);
    }

    #[tracing::instrument(level = "info", skip_all, name = "inference.best_face_candidate")]
    fn best_face_candidate(
        &self,
        model: &Model,
        query: &Query,
        ray_dir: DVec3,
    ) -> Option<Candidate> {
        let mut best: Option<Candidate> = None;
        for face_id in model.face_candidates_for_ray(query.ray.origin, ray_dir, f64::INFINITY) {
            let face = &model.faces()[&face_id];
            let Some((position, ray_depth)) =
                ray_face_intersection(model, face, query.ray.origin, ray_dir)
            else {
                continue;
            };
            let Some((screen_pos, ndc_depth)) = project_with_depth(&query.view, position) else {
                continue;
            };
            let screen_distance = screen_pos.distance(query.cursor);
            if !screen_distance.is_finite() {
                continue;
            }
            if query.section.as_ref().is_some_and(|p| !p.retains(position)) {
                continue;
            }
            let candidate = Candidate {
                snap: Snap {
                    position,
                    kind: SnapKind::OnFace,
                    screen_distance,
                    guide: None,
                    entity: Some(SnapEntity::Face(face_id)),
                },
                sub_priority: 0,
                ray_depth,
                ndc_depth,
                entity_order: face_id.value(),
            };
            update_best_face(&mut best, candidate);
        }
        best
    }

    fn plane_fallback(&self, query: &Query, plane_normal: DVec3) -> Option<Snap> {
        let raw = query.plane.intersect(query.ray)?;
        let snapped = raw - plane_normal * (raw - query.plane.origin).dot(plane_normal);
        let (position, (screen_pos, _)) = if let Some(proj) = project_with_depth(&query.view, raw) {
            (raw, proj)
        } else {
            let proj = project_with_depth(&query.view, snapped)?;
            (snapped, proj)
        };
        let screen_distance = screen_pos.distance(query.cursor);
        screen_distance.is_finite().then_some(Snap {
            position,
            kind: SnapKind::Plane,
            screen_distance,
            guide: None,
            entity: None,
        })
    }
}

#[derive(Clone, Debug)]
struct Candidate {
    snap: Snap,
    sub_priority: u8,
    ray_depth: f64,
    ndc_depth: f64,
    entity_order: u64,
}

fn update_best(best: &mut Option<Candidate>, candidate: Candidate) {
    let replace = match best {
        None => true,
        Some(current) => compare_candidates(&candidate, current).is_lt(),
    };
    if replace {
        *best = Some(candidate);
    }
}

fn update_best_face(best: &mut Option<Candidate>, candidate: Candidate) {
    let replace = match best {
        None => true,
        Some(current) => {
            if (candidate.ray_depth - current.ray_depth).abs() > TOLERANCE {
                candidate.ray_depth.total_cmp(&current.ray_depth).is_lt()
            } else {
                candidate.entity_order < current.entity_order
            }
        }
    };
    if replace {
        *best = Some(candidate);
    }
}

fn compare_candidates(a: &Candidate, b: &Candidate) -> std::cmp::Ordering {
    if (a.snap.screen_distance - b.snap.screen_distance).abs() > TOLERANCE {
        return a.snap.screen_distance.total_cmp(&b.snap.screen_distance);
    }
    let sub = a.sub_priority.cmp(&b.sub_priority);
    if sub != std::cmp::Ordering::Equal {
        return sub;
    }
    if (a.ray_depth - b.ray_depth).abs() > TOLERANCE {
        return a.ray_depth.total_cmp(&b.ray_depth);
    }
    if (a.ndc_depth - b.ndc_depth).abs() > TOLERANCE {
        return a.ndc_depth.total_cmp(&b.ndc_depth);
    }
    a.entity_order
        .cmp(&b.entity_order)
        .then_with(|| a.snap.position.x.total_cmp(&b.snap.position.x))
        .then_with(|| a.snap.position.y.total_cmp(&b.snap.position.y))
        .then_with(|| a.snap.position.z.total_cmp(&b.snap.position.z))
}

fn validate_query(query: &Query) -> Option<DVec3> {
    if !query.cursor.is_finite()
        || !query.ray.origin.is_finite()
        || !query.ray.direction.is_finite()
        || !query.view.matrix.is_finite()
        || !query.view.size.is_finite()
        || query.view.size.min_element() <= 0.0
        || !query.plane.origin.is_finite()
        || !query.plane.normal.is_finite()
    {
        return None;
    }
    if query.anchor.is_some_and(|a| !a.is_finite()) {
        return None;
    }
    if query.previous_direction.is_some_and(|d| !d.is_finite()) {
        return None;
    }
    if query
        .tangent
        .is_some_and(|t| !t.center.is_finite() || !t.normal.is_finite() || !t.reference.is_finite())
    {
        return None;
    }
    normalize_vec(query.ray.direction)
}

fn normalize_vec(v: DVec3) -> Option<DVec3> {
    if !v.is_finite() || v.length() <= TOLERANCE {
        return None;
    }
    v.try_normalize()
}

fn project_with_depth(view: &View, point: DVec3) -> Option<(DVec2, f64)> {
    let screen = view.project(point)?;
    let clip = view.matrix * point.extend(1.0);
    if !clip.is_finite() || clip.w <= 0.0 {
        return None;
    }
    let ndc_z = clip.z / clip.w;
    ndc_z.is_finite().then_some((screen, ndc_z))
}

fn fallback_cursor_ray(query: &Query) -> Option<(DVec3, DVec3)> {
    let ray_proj_origin = query.view.project(query.ray.origin);
    let ray_proj_plane = query
        .plane
        .intersect(query.ray)
        .and_then(|p| query.view.project(p));
    if ray_proj_origin.is_some_and(|p| p.distance(query.cursor) <= 1.0)
        || ray_proj_plane.is_some_and(|p| p.distance(query.cursor) <= 1.0)
    {
        return None;
    }
    if query.view.matrix.determinant().abs() <= TOLERANCE {
        return None;
    }
    let inv = query.view.matrix.inverse();
    if !inv.is_finite() {
        return None;
    }
    let ndc_x = (query.cursor.x / query.view.size.x) * 2.0 - 1.0;
    let ndc_y = 1.0 - (query.cursor.y / query.view.size.y) * 2.0;
    let far = inv.project_point3(DVec3::new(ndc_x, ndc_y, 1.0));
    let near = inv.project_point3(DVec3::new(ndc_x, ndc_y, 0.0));
    let dir = normalize_vec(near - far)?;
    Some((far, dir))
}

fn project_onto_line_within_tolerance(
    point: DVec3,
    line_origin: DVec3,
    line_dir: DVec3,
) -> Option<DVec3> {
    let s = (point - line_origin).dot(line_dir);
    let projected = line_origin + line_dir * s;
    (projected.is_finite() && point.distance(projected) <= TOLERANCE).then_some(projected)
}

fn closest_point_on_line_to_ray(
    line_origin: DVec3,
    line_dir: DVec3,
    ray_origin: DVec3,
    ray_dir: DVec3,
) -> Option<DVec3> {
    let w0 = line_origin - ray_origin;
    let b = line_dir.dot(ray_dir);
    let denom = 1.0 - b * b;
    if denom <= TOLERANCE * TOLERANCE {
        return None;
    }
    let d_w = line_dir.dot(w0);
    let r_w = ray_dir.dot(w0);
    let s = (b * r_w - d_w) / denom;
    let t = (r_w - b * d_w) / denom;
    if !s.is_finite() || !t.is_finite() || t < -TOLERANCE {
        return None;
    }
    let point = line_origin + line_dir * s;
    point.is_finite().then_some(point)
}

fn tangent_direction(hint: TangentHint) -> Option<DVec3> {
    let normal = normalize_vec(hint.normal)?;
    let radial = hint.reference - hint.center;
    if radial.length() <= TOLERANCE {
        return None;
    }
    let tangent = normal.cross(radial);
    if tangent.length() <= TOLERANCE * radial.length().max(1.0) {
        return None;
    }
    tangent.try_normalize()
}

fn locked_edge_point(
    a: DVec3,
    b: DVec3,
    lock_origin: DVec3,
    lock_dir: DVec3,
    query: &Query,
) -> Option<DVec3> {
    let a_on = project_onto_line_within_tolerance(a, lock_origin, lock_dir);
    let b_on = project_onto_line_within_tolerance(b, lock_origin, lock_dir);
    if let (Some(pa), Some(pb)) = (a_on, b_on) {
        let pos = closest_point_on_segment_screen(pa, pb, query.cursor, &query.view)?;
        return project_onto_line_within_tolerance(pos, lock_origin, lock_dir);
    }

    let u = b - a;
    let lu = u.length();
    if lu <= TOLERANCE {
        return None;
    }
    let cross = u.cross(lock_dir);
    let cross_len_sq = cross.length_squared();
    if cross_len_sq <= (TOLERANCE * lu) * (TOLERANCE * lu) {
        return None;
    }
    let w = lock_origin - a;
    let t_edge = w.cross(lock_dir).dot(cross) / cross_len_sq;
    let s_lock = w.cross(u).dot(cross) / cross_len_sq;
    if !t_edge.is_finite()
        || !s_lock.is_finite()
        || t_edge < -TOLERANCE / lu
        || t_edge > 1.0 + TOLERANCE / lu
    {
        return None;
    }
    let p_edge = a + u * t_edge.clamp(0.0, 1.0);
    let p_lock = lock_origin + lock_dir * s_lock;
    (p_edge.distance(p_lock) <= TOLERANCE && p_lock.is_finite()).then_some(p_lock)
}

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

fn ray_face_intersection(
    model: &Model,
    face: &Face,
    ray_origin: DVec3,
    ray_dir: DVec3,
) -> Option<(DVec3, f64)> {
    if face.outer.len() < 3 {
        return None;
    }
    let normal = normalize_vec(face.plane.normal)?;
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

fn project_ring_2d(
    model: &Model,
    ring: &[cindral_core::VertexId],
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
    for (a, b) in ring.iter().zip(ring.iter().cycle().skip(1)) {
        let ab = *b - *a;
        let len_sq = ab.length_squared();
        let t = if len_sq <= TOLERANCE * TOLERANCE {
            0.0
        } else {
            ((point - *a).dot(ab) / len_sq).clamp(0.0, 1.0)
        };
        if point.distance(*a + ab * t) <= TOLERANCE {
            return true;
        }
    }
    false
}

fn polygon_contains_raycast(ring: &[DVec2], point: DVec2) -> bool {
    let mut inside = false;
    for (a, b) in ring.iter().zip(ring.iter().cycle().skip(1)) {
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

#[cfg(test)]
mod tests {
    use cindral_core::{Command, History, TOLERANCE};
    use glam::{DMat4, DVec2, DVec3};

    use super::Engine;
    use crate::{Axis, DrawingPlane, Query, Ray, SnapEntity, SnapKind, TangentHint, View};

    struct TestCamera {
        eye: DVec3,
        view: View,
        inv_vp: DMat4,
    }

    impl TestCamera {
        fn perspective(eye: DVec3, target: DVec3, up: DVec3, size: DVec2) -> Self {
            let view_mat = DMat4::look_at_rh(eye, target, up);
            let aspect = size.x / size.y;
            let proj_mat = DMat4::perspective_rh(std::f64::consts::FRAC_PI_3, aspect, 0.1, 1000.0);
            let vp = proj_mat * view_mat;
            Self {
                eye,
                view: View { matrix: vp, size },
                inv_vp: vp.inverse(),
            }
        }

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

        fn query_at_cursor(&self, cursor: DVec2) -> Query {
            let ndc_x = (cursor.x / self.view.size.x) * 2.0 - 1.0;
            let ndc_y = 1.0 - (cursor.y / self.view.size.y) * 2.0;
            let near = self.inv_vp.project_point3(DVec3::new(ndc_x, ndc_y, 0.0));
            let far = self.inv_vp.project_point3(DVec3::new(ndc_x, ndc_y, 1.0));
            let origin = if (near - self.eye).length() <= 1.0e-3 {
                self.eye
            } else {
                near
            };
            Query::new(
                cursor,
                Ray {
                    origin,
                    direction: far - near,
                },
                self.view,
            )
        }

        fn query_at_world(&self, world: DVec3) -> Query {
            let cursor = self
                .view
                .project(world)
                .expect("test world point must be visible");
            self.query_at_cursor(cursor)
        }
    }

    #[test]
    fn default_engine_and_plane_fallback_work() {
        let engine = Engine::default();
        assert_eq!(engine.radius_pixels, 12.0);
        assert_eq!(Engine::new(), engine);

        let history = History::new();
        let cam = TestCamera::perspective(
            DVec3::new(0.0, -6.0, 5.0),
            DVec3::ZERO,
            DVec3::Z,
            DVec2::new(800.0, 600.0),
        );
        let target = DVec3::new(1.25, -0.75, 0.0);
        let query = cam.query_at_world(target);
        let snap = engine
            .infer(history.model(), &query)
            .expect("should hit default XY plane");
        assert_eq!(snap.kind, SnapKind::Plane);
        assert!(snap.position.distance(target) <= 1.0e-6);
        assert!(snap.screen_distance <= 1.0e-6);
        assert!(snap.guide.is_none());
        assert!(snap.entity.is_none());

        // Custom tilted non-unit normal drawing plane
        let mut tilted_query = query.clone();
        tilted_query.plane = DrawingPlane {
            origin: DVec3::new(0.0, 0.0, 2.0),
            normal: DVec3::new(0.0, 0.0, 5.0),
        };
        let tilted_snap = engine
            .infer(history.model(), &tilted_query)
            .expect("should hit z=2 plane");
        assert_eq!(tilted_snap.kind, SnapKind::Plane);
        assert!((tilted_snap.position.z - 2.0).abs() <= 1.0e-6);
    }

    #[test]
    fn endpoints_and_grade_three_plus_intersections_are_distinguished() {
        let mut history = History::new();
        // Cross two segments at origin -> degree-4 vertex at (0,0,0)
        history
            .execute(Command::AddEdge {
                start: DVec3::new(-2.0, 0.0, 0.0),
                end: DVec3::new(2.0, 0.0, 0.0),
            })
            .expect("edge 1");
        history
            .execute(Command::AddEdge {
                start: DVec3::new(0.0, -2.0, 0.0),
                end: DVec3::new(0.0, 2.0, 0.0),
            })
            .expect("edge 2");
        // Add an edge from (2,0,0) to (2,2,0) -> (2,0,0) becomes degree 2, and from (2,0,0) to (2,-2,0) -> degree 3
        history
            .execute(Command::AddEdge {
                start: DVec3::new(2.0, 0.0, 0.0),
                end: DVec3::new(2.0, 2.0, 0.0),
            })
            .expect("edge 3");
        history
            .execute(Command::AddEdge {
                start: DVec3::new(2.0, 0.0, 0.0),
                end: DVec3::new(2.0, -2.0, 0.0),
            })
            .expect("edge 4");

        let cam = TestCamera::orthographic(
            DVec3::new(0.0, 0.0, 10.0),
            DVec3::ZERO,
            DVec3::Y,
            DVec2::new(5.0, 5.0),
            DVec2::new(1000.0, 1000.0),
        );
        let engine = Engine::default();

        // Degree-4 vertex at (0, 0, 0) -> Intersection
        let q_origin = cam.query_at_world(DVec3::ZERO);
        let snap_origin = engine
            .infer(history.model(), &q_origin)
            .expect("origin snap");
        assert_eq!(snap_origin.kind, SnapKind::Intersection);
        assert!(snap_origin.position.distance(DVec3::ZERO) <= TOLERANCE);
        assert!(matches!(snap_origin.entity, Some(SnapEntity::Vertex(_))));

        // Degree-3 vertex at (2, 0, 0) -> Intersection
        let q_t = cam.query_at_world(DVec3::new(2.0, 0.0, 0.0));
        let snap_t = engine
            .infer(history.model(), &q_t)
            .expect("T-junction snap");
        assert_eq!(snap_t.kind, SnapKind::Intersection);
        assert!(snap_t.position.distance(DVec3::new(2.0, 0.0, 0.0)) <= TOLERANCE);

        // Degree-1 vertex at (-2, 0, 0) -> Endpoint
        let end_screen = cam
            .view
            .project(DVec3::new(-2.0, 0.0, 0.0))
            .expect("visible");
        let q_end = cam.query_at_cursor(end_screen + DVec2::new(0.0, 8.0));
        let snap_end = engine
            .infer(history.model(), &q_end)
            .expect("endpoint snap");
        assert_eq!(snap_end.kind, SnapKind::Endpoint);
        assert!(snap_end.position.distance(DVec3::new(-2.0, 0.0, 0.0)) <= TOLERANCE);
        assert!((snap_end.screen_distance - 8.0).abs() <= 1.0e-6);

        // Outside 12px radius (15px away in Y from (-2, 0, 0), and (-2.15, -0.15) is off edge)
        let q_miss = cam.query_at_cursor(end_screen + DVec2::new(-15.0, 15.0));
        let snap_miss = engine
            .infer(history.model(), &q_miss)
            .expect("plane fallback");
        assert_eq!(snap_miss.kind, SnapKind::Plane);
    }

    #[test]
    fn midpoint_and_on_edge_snapping_respect_priority_and_perspective() {
        let mut history = History::new();
        let a = DVec3::new(-2.0, 1.0, 0.0);
        let b = DVec3::new(2.0, 1.0, 0.0);
        history
            .execute(Command::AddEdge { start: a, end: b })
            .expect("add edge");

        let cam = TestCamera::perspective(
            DVec3::new(0.0, -5.0, 4.0),
            DVec3::new(0.0, 1.0, 0.0),
            DVec3::Z,
            DVec2::new(1000.0, 800.0),
        );
        let engine = Engine::default();

        // Midpoint at (0, 1, 0): even if cursor is directly on the edge 6px from midpoint,
        // Midpoint must win over OnEdge!
        let mid_world = DVec3::new(0.0, 1.0, 0.0);
        let mid_screen = cam.view.project(mid_world).expect("mid visible");
        let q_mid = cam.query_at_cursor(mid_screen + DVec2::new(6.0, 0.0));
        let snap_mid = engine
            .infer(history.model(), &q_mid)
            .expect("midpoint snap");
        assert_eq!(snap_mid.kind, SnapKind::Midpoint);
        assert!(snap_mid.position.distance(mid_world) <= TOLERANCE);
        assert!(matches!(snap_mid.entity, Some(SnapEntity::Edge(_))));

        // OnEdge at (-1.0, 1.0, 0.0) (far from endpoints and midpoint)
        let edge_pt = DVec3::new(-1.0, 1.0, 0.0);
        let edge_screen = cam.view.project(edge_pt).expect("edge point visible");
        let q_edge = cam.query_at_cursor(edge_screen + DVec2::new(0.0, 5.0));
        let snap_edge = engine
            .infer(history.model(), &q_edge)
            .expect("on edge snap");
        assert_eq!(snap_edge.kind, SnapKind::OnEdge);
        assert!((snap_edge.position.y - 1.0).abs() <= 1.0e-6);
        assert!((snap_edge.position.z - 0.0).abs() <= 1.0e-6);
        assert!(snap_edge.position.x > -1.9 && snap_edge.position.x < -0.1);
        assert!((snap_edge.screen_distance - 5.0).abs() <= 1.0e-3);
        assert!(matches!(snap_edge.entity, Some(SnapEntity::Edge(_))));
    }

    #[test]
    fn on_face_respects_holes_and_picks_nearest_face_hit() {
        let mut history = History::new();
        // Outer square at z=0: (-4,-4,0)..(4,4,0)
        history
            .execute(Command::AddPolyline {
                points: vec![
                    DVec3::new(-4.0, -4.0, 0.0),
                    DVec3::new(4.0, -4.0, 0.0),
                    DVec3::new(4.0, 4.0, 0.0),
                    DVec3::new(-4.0, 4.0, 0.0),
                ],
                closed: true,
            })
            .expect("outer face");
        // Inner square at z=0: (-1,-1,0)..(1,1,0) creates a hole in the outer face and an inner face
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
        assert_eq!(history.model().faces().len(), 2);
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
        // Verify outer_face_id < inner_face_id so that if holes were ignored, outer_face_id would win tie-break!
        assert!(outer_face_id < inner_face_id);

        let cam = TestCamera::orthographic(
            DVec3::new(0.0, 0.0, 10.0),
            DVec3::ZERO,
            DVec3::Y,
            DVec2::new(6.0, 6.0),
            DVec2::new(1200.0, 1200.0),
        );
        let engine = Engine::default();

        // Point on the outer annulus (2.5, 0, 0) -> hits outer_face_id
        let q_annulus = cam.query_at_world(DVec3::new(2.5, 0.0, 0.0));
        let snap_annulus = engine
            .infer(history.model(), &q_annulus)
            .expect("annulus hit");
        assert_eq!(snap_annulus.kind, SnapKind::OnFace);
        assert_eq!(snap_annulus.entity, Some(SnapEntity::Face(outer_face_id)));

        // Point inside the hole (0, 0, 0) -> outer_face_id must reject it because it is in outer_face_id's hole!
        let q_hole = cam.query_at_world(DVec3::ZERO);
        let snap_hole = engine.infer(history.model(), &q_hole).expect("inner hit");
        assert_eq!(snap_hole.kind, SnapKind::OnFace);
        assert_eq!(snap_hole.entity, Some(SnapEntity::Face(inner_face_id)));

        // Add an elevated face at z=2.0 with a hole at (-1,-1,2)..(1,1,2)
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
            .expect("top outer face");

        // Ray at (2.0, 0.0, 2.0) hits both z=2.0 face and z=0.0 face -> nearest face (z=2.0) must win!
        let q_top = cam.query_at_world(DVec3::new(2.0, 0.0, 2.0));
        let snap_top = engine
            .infer(history.model(), &q_top)
            .expect("nearest face hit");
        assert_eq!(snap_top.kind, SnapKind::OnFace);
        assert!((snap_top.position.z - 2.0).abs() <= TOLERANCE);
    }

    #[test]
    fn automatic_world_axes_parallel_perpendicular_and_tangent_guides() {
        let history = History::new();
        let cam = TestCamera::perspective(
            DVec3::new(6.0, -6.0, 5.0),
            DVec3::new(0.0, 0.0, 1.0),
            DVec3::Z,
            DVec2::new(1000.0, 800.0),
        );
        let engine = Engine::default();
        let anchor = DVec3::ZERO;

        // World Axis X, Y, Z
        for (axis, target) in [
            (Axis::X, DVec3::new(2.5, 0.0, 0.0)),
            (Axis::Y, DVec3::new(0.0, 2.5, 0.0)),
            (Axis::Z, DVec3::new(0.0, 0.0, 2.5)),
        ] {
            let mut query = cam.query_at_world(target);
            query.anchor = Some(anchor);
            let snap = engine
                .infer(history.model(), &query)
                .expect("axis guide snap");
            assert_eq!(snap.kind, SnapKind::Axis(axis));
            assert!(snap.position.distance(target) <= 1.0e-5);
            assert_eq!(snap.guide, Some([anchor, snap.position]));
            assert!(snap.entity.is_none());
        }

        // Parallel and Perpendicular guides from previous_direction = (1, 1, 0)
        let prev_dir = DVec3::new(1.0, 1.0, 0.0);
        let par_target = DVec3::new(2.0, 2.0, 0.0);
        let mut q_par = cam.query_at_world(par_target);
        q_par.anchor = Some(anchor);
        q_par.previous_direction = Some(prev_dir);
        let snap_par = engine
            .infer(history.model(), &q_par)
            .expect("parallel snap");
        assert_eq!(snap_par.kind, SnapKind::Parallel);
        assert!(snap_par.position.distance(par_target) <= 1.0e-5);
        assert_eq!(snap_par.guide, Some([anchor, snap_par.position]));

        let perp_target = DVec3::new(-2.0, 2.0, 0.0);
        let mut q_perp = cam.query_at_world(perp_target);
        q_perp.anchor = Some(anchor);
        q_perp.previous_direction = Some(prev_dir);
        let snap_perp = engine
            .infer(history.model(), &q_perp)
            .expect("perpendicular snap");
        assert_eq!(snap_perp.kind, SnapKind::Perpendicular);
        assert!(snap_perp.position.distance(perp_target) <= 1.0e-5);
        assert_eq!(snap_perp.guide, Some([anchor, snap_perp.position]));

        // Tangent guide for in-progress arc: circle centered at (0, 0, 0), reference at (1, 1, 0), normal +Z
        // Radial is (1, 1, 0), so tangent direction is (-1, 1, 0).normalize()
        let reference = DVec3::new(1.0, 1.0, 0.0);
        let tan_target = DVec3::new(0.0, 2.0, 0.0);
        let mut q_tan = cam.query_at_world(tan_target);
        q_tan.anchor = Some(reference);
        q_tan.tangent = Some(TangentHint {
            center: DVec3::ZERO,
            normal: DVec3::Z,
            reference,
        });
        let snap_tan = engine
            .infer(history.model(), &q_tan)
            .expect("tangent guide snap");
        assert_eq!(snap_tan.kind, SnapKind::Tangent);
        assert!(snap_tan.position.distance(tan_target) <= 1.0e-5);
        assert_eq!(snap_tan.guide, Some([reference, snap_tan.position]));
    }

    #[test]
    fn hard_axis_lock_works_on_z_from_ground_plane_and_rejects_incompatible_geometry() {
        let mut history = History::new();
        // Add off-axis geometry at (1.5, 0.5, 0.0) and a ground face
        history
            .execute(Command::AddPolyline {
                points: vec![
                    DVec3::new(-2.0, -2.0, 0.0),
                    DVec3::new(2.0, -2.0, 0.0),
                    DVec3::new(2.0, 2.0, 0.0),
                    DVec3::new(-2.0, 2.0, 0.0),
                ],
                closed: true,
            })
            .expect("ground face");
        // Add an edge crossing the Z axis at (0, 0, 2.0) and having a vertex at (0, 0, 3.0)
        history
            .execute(Command::AddEdge {
                start: DVec3::new(-1.0, 0.0, 2.0),
                end: DVec3::new(1.0, 0.0, 2.0),
            })
            .expect("crossing edge at z=2");
        history
            .execute(Command::AddEdge {
                start: DVec3::new(0.0, 0.0, 3.0),
                end: DVec3::new(1.0, 0.0, 3.0),
            })
            .expect("vertex on z axis at z=3");

        let cam = TestCamera::perspective(
            DVec3::new(5.0, -5.0, 4.0),
            DVec3::new(0.0, 0.0, 1.5),
            DVec3::Z,
            DVec2::new(1000.0, 800.0),
        );
        let engine = Engine::default();
        let anchor = DVec3::ZERO;

        // 1. Hover directly over off-axis corner vertex (2.0, -2.0, 0.0) while Z-locked:
        // Without lock, it snaps to Endpoint (2.0, -2.0, 0.0).
        let q_corner_unlocked = cam.query_at_world(DVec3::new(2.0, -2.0, 0.0));
        let unlocked = engine
            .infer(history.model(), &q_corner_unlocked)
            .expect("unlocked corner");
        assert_eq!(unlocked.kind, SnapKind::Endpoint);

        // With Z lock, the off-axis vertex, edge, and ground face are REJECTED and Z axis wins!
        let mut q_corner_locked = q_corner_unlocked;
        q_corner_locked.anchor = Some(anchor);
        q_corner_locked.axis_lock = Some(Axis::Z);
        let locked = engine
            .infer(history.model(), &q_corner_locked)
            .expect("Z-locked snap");
        assert_eq!(locked.kind, SnapKind::Axis(Axis::Z));
        assert!(locked.position.x.abs() <= TOLERANCE);
        assert!(locked.position.y.abs() <= TOLERANCE);
        assert_eq!(locked.guide, Some([anchor, locked.position]));

        // 2. Hover near z=1.5 along Z axis (no vertex/edge there) -> SnapKind::Axis(Axis::Z) at (0, 0, 1.5)
        let mut q_z = cam.query_at_world(DVec3::new(0.0, 0.0, 1.5));
        q_z.anchor = Some(anchor);
        q_z.axis_lock = Some(Axis::Z);
        let snap_z = engine.infer(history.model(), &q_z).expect("z=1.5 lock");
        assert_eq!(snap_z.kind, SnapKind::Axis(Axis::Z));
        assert!(snap_z.position.distance(DVec3::new(0.0, 0.0, 1.5)) <= 1.0e-5);

        // 3. Hover near (0, 0, 2.0) where edge (-1,0,2)..(1,0,2) has its midpoint on the Z axis!
        let mut q_mid_z = cam.query_at_world(DVec3::new(0.0, 0.0, 2.0));
        q_mid_z.anchor = Some(anchor);
        q_mid_z.axis_lock = Some(Axis::Z);
        let snap_mid_z = engine
            .infer(history.model(), &q_mid_z)
            .expect("compatible midpoint on Z lock");
        assert_eq!(snap_mid_z.kind, SnapKind::Midpoint);
        assert!(snap_mid_z.position.distance(DVec3::new(0.0, 0.0, 2.0)) <= TOLERANCE);
        assert_eq!(snap_mid_z.guide, Some([anchor, snap_mid_z.position]));

        // 4. Hover near (0, 0, 3.0) where vertex (0, 0, 3.0) lies on the Z axis!
        let mut q_vert_z = cam.query_at_world(DVec3::new(0.0, 0.0, 3.0));
        q_vert_z.anchor = Some(anchor);
        q_vert_z.axis_lock = Some(Axis::Z);
        let snap_vert_z = engine
            .infer(history.model(), &q_vert_z)
            .expect("compatible vertex on Z lock");
        assert_eq!(snap_vert_z.kind, SnapKind::Endpoint);
        assert!(snap_vert_z.position.distance(DVec3::new(0.0, 0.0, 3.0)) <= TOLERANCE);
        assert_eq!(snap_vert_z.guide, Some([anchor, snap_vert_z.position]));
    }

    #[test]
    fn depth_ordering_and_stale_history_entities_behave_deterministically() {
        let mut history = History::new();
        // Two edges with vertices stacked along the Z viewing ray at (0, 0, 0.2) and (0, 0, 0.8)
        history
            .execute(Command::AddEdge {
                start: DVec3::new(0.0, 0.0, 0.2),
                end: DVec3::new(1.0, 0.0, 0.2),
            })
            .expect("lower edge");
        history
            .execute(Command::AddEdge {
                start: DVec3::new(0.0, 0.0, 0.8),
                end: DVec3::new(1.0, 0.0, 0.8),
            })
            .expect("upper edge");

        let view = View {
            matrix: DMat4::IDENTITY,
            size: DVec2::splat(100.0),
        };
        let query = Query::new(
            DVec2::splat(50.0),
            Ray {
                origin: DVec3::new(0.0, 0.0, 2.0),
                direction: -DVec3::Z,
            },
            view,
        );
        let engine = Engine::default();

        // Both (0,0,0.2) and (0,0,0.8) project to (50, 50); closer vertex to ray origin (z=0.8) wins!
        let snap = engine
            .infer(history.model(), &query)
            .expect("stacked vertex snap");
        assert_eq!(snap.kind, SnapKind::Endpoint);
        assert!(snap.position.distance(DVec3::new(0.0, 0.0, 0.8)) <= TOLERANCE);

        // Undo the upper edge -> now (0, 0, 0.8) is gone, and (0, 0, 0.2) wins!
        history.undo().expect("undo upper edge");
        let snap_after_undo = engine
            .infer(history.model(), &query)
            .expect("lower vertex snap after undo");
        assert_eq!(snap_after_undo.kind, SnapKind::Endpoint);
        assert!(snap_after_undo.position.distance(DVec3::new(0.0, 0.0, 0.2)) <= TOLERANCE);

        // Redo -> (0, 0, 0.8) wins again!
        history.redo().expect("redo upper edge");
        let snap_after_redo = engine
            .infer(history.model(), &query)
            .expect("upper vertex snap after redo");
        assert_eq!(snap_after_redo.position, snap.position);
        assert_eq!(snap_after_redo.entity, snap.entity);

        // Determinism across 100 consecutive runs
        for _ in 0..100 {
            let repeated = engine
                .infer(history.model(), &query)
                .expect("deterministic");
            assert_eq!(repeated.position, snap.position);
            assert_eq!(repeated.kind, snap.kind);
            assert_eq!(repeated.screen_distance, snap.screen_distance);
            assert_eq!(repeated.guide, snap.guide);
            assert_eq!(repeated.entity, snap.entity);
        }
    }

    #[test]
    fn invalid_and_nonfinite_inputs_are_rejected_safely() {
        let history = History::new();
        let valid_view = View {
            matrix: DMat4::IDENTITY,
            size: DVec2::splat(100.0),
        };
        let valid_ray = Ray {
            origin: DVec3::Z,
            direction: -DVec3::Z,
        };
        let valid_query = Query::new(DVec2::splat(50.0), valid_ray, valid_view);
        let engine = Engine::default();

        assert!(engine.infer(history.model(), &valid_query).is_some());

        // Invalid Engine radii
        for bad_radius in [0.0, -1.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let bad_engine = Engine {
                radius_pixels: bad_radius,
            };
            assert!(bad_engine.infer(history.model(), &valid_query).is_none());
        }

        // Non-finite cursor
        let mut q = valid_query.clone();
        q.cursor = DVec2::new(f64::NAN, 50.0);
        assert!(engine.infer(history.model(), &q).is_none());

        // Zero or non-finite ray
        let mut q = valid_query.clone();
        q.ray.direction = DVec3::ZERO;
        assert!(engine.infer(history.model(), &q).is_none());
        q.ray.origin = DVec3::splat(f64::INFINITY);
        assert!(engine.infer(history.model(), &q).is_none());

        // Invalid viewport size
        let mut q = valid_query.clone();
        q.view.size = DVec2::new(0.0, 100.0);
        assert!(engine.infer(history.model(), &q).is_none());

        // Zero or non-finite plane normal
        let mut q = valid_query.clone();
        q.plane.normal = DVec3::ZERO;
        assert!(engine.infer(history.model(), &q).is_none());

        // Non-finite optional fields
        let mut q = valid_query.clone();
        q.anchor = Some(DVec3::new(f64::NAN, 0.0, 0.0));
        assert!(engine.infer(history.model(), &q).is_none());

        let mut q = valid_query.clone();
        q.previous_direction = Some(DVec3::splat(f64::NEG_INFINITY));
        assert!(engine.infer(history.model(), &q).is_none());

        let mut q = valid_query.clone();
        q.tangent = Some(TangentHint {
            center: DVec3::ZERO,
            normal: DVec3::Z,
            reference: DVec3::splat(f64::NAN),
        });
        assert!(engine.infer(history.model(), &q).is_none());

        // Degenerate zero-length previous_direction / tangent hint are safely ignored and fall back to Plane
        let mut q = valid_query.clone();
        q.anchor = Some(DVec3::new(1.0, 1.0, 0.0));
        q.previous_direction = Some(DVec3::ZERO);
        q.tangent = Some(TangentHint {
            center: DVec3::X,
            normal: DVec3::Z,
            reference: DVec3::X,
        });
        let fallback = engine
            .infer(history.model(), &q)
            .expect("falls back to plane");
        assert_eq!(fallback.kind, SnapKind::Plane);
    }

    #[test]
    fn locked_axis_snaps_compatible_collinear_and_crossing_edges_and_partially_clipped_segments() {
        let mut history = History::new();
        // Edge crossing the X axis at (2.0, 0.0, 0.0) with asymmetric endpoints so (2.0, 0.0, 0.0) is NOT its midpoint
        history
            .execute(Command::AddEdge {
                start: DVec3::new(2.0, -1.0, 0.0),
                end: DVec3::new(2.0, 3.0, 0.0),
            })
            .expect("asymmetric crossing edge");
        // Edge collinear with the Y axis from (0.0, 1.0, 0.0) to (0.0, 4.0, 0.0)
        history
            .execute(Command::AddEdge {
                start: DVec3::new(0.0, 1.0, 0.0),
                end: DVec3::new(0.0, 4.0, 0.0),
            })
            .expect("collinear Y edge");
        // Edge partially outside [0, 1] NDC depth under IDENTITY matrix: from (-0.5, -0.5, -0.5) to (0.5, -0.5, 0.5)
        history
            .execute(Command::AddEdge {
                start: DVec3::new(-0.5, -0.5, -0.5),
                end: DVec3::new(0.5, -0.5, 0.5),
            })
            .expect("partially clipped edge");

        let cam = TestCamera::perspective(
            DVec3::new(4.0, -6.0, 5.0),
            DVec3::new(1.0, 1.0, 0.0),
            DVec3::Z,
            DVec2::new(1000.0, 800.0),
        );
        let engine = Engine::default();

        // 1. Hard X-axis lock from origin: hovering near (2.0, 0.0, 0.0) snaps OnEdge at the crossing point!
        let mut q_cross = cam.query_at_world(DVec3::new(2.0, 0.0, 0.0));
        q_cross.anchor = Some(DVec3::ZERO);
        q_cross.axis_lock = Some(Axis::X);
        let snap_cross = engine
            .infer(history.model(), &q_cross)
            .expect("locked crossing edge snap");
        assert_eq!(snap_cross.kind, SnapKind::OnEdge);
        assert!(snap_cross.position.distance(DVec3::new(2.0, 0.0, 0.0)) <= TOLERANCE);
        assert_eq!(snap_cross.guide, Some([DVec3::ZERO, snap_cross.position]));
        assert!(matches!(snap_cross.entity, Some(SnapEntity::Edge(_))));

        // 2. Hard Y-axis lock from origin: hovering near (0.0, 1.8, 0.0) snaps OnEdge along the collinear Y edge!
        let mut q_col = cam.query_at_world(DVec3::new(0.0, 1.8, 0.0));
        q_col.anchor = Some(DVec3::ZERO);
        q_col.axis_lock = Some(Axis::Y);
        let snap_col = engine
            .infer(history.model(), &q_col)
            .expect("locked collinear edge snap");
        assert_eq!(snap_col.kind, SnapKind::OnEdge);
        assert!(snap_col.position.distance(DVec3::new(0.0, 1.8, 0.0)) <= 1.0e-5);
        assert_eq!(snap_col.guide, Some([DVec3::ZERO, snap_col.position]));

        // 3. Partially clipped edge in IDENTITY view: (-0.5,-0.5,-0.5) has z < 0 (clipped),
        // while (0.2, -0.5, 0.2) has z = 0.2 in [0, 1] and is visible!
        let id_view = View {
            matrix: DMat4::IDENTITY,
            size: DVec2::splat(200.0),
        };
        let visible_pt = DVec3::new(0.2, -0.5, 0.2);
        let cursor = id_view
            .project(visible_pt)
            .expect("visible subsegment point");
        let q_clipped = Query::new(
            cursor,
            Ray {
                origin: DVec3::new(0.2, -0.5, 1.0),
                direction: -DVec3::Z,
            },
            id_view,
        );
        let snap_clipped = engine
            .infer(history.model(), &q_clipped)
            .expect("partially clipped edge snap");
        assert_eq!(snap_clipped.kind, SnapKind::OnEdge);
        assert!(snap_clipped.position.distance(visible_pt) <= 1.0e-5);
    }
}
