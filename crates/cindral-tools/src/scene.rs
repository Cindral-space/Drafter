//! Pure transformed instance picking and context-query math.
//!
//! Provides [`PickInstance`], [`InstanceHit`], [`pick_instances`], and [`local_query`].
//! Transforms world-space picking rays, views, and queries into instance-local space
//! while preserving world foreground depth, screen-space radius selection, face holes,
//! and handling mirrored, non-uniform, and nested transforms.

use cindral_core::{DefinitionId, Edge, Face, InstanceId, Model, VertexId, TOLERANCE};
use cindral_inference::{Query, Ray, View};
use glam::{DMat4, DVec2, DVec3};

use crate::selection::Hit;
use crate::ToolError;

/// An instance occurrence candidate for transformed ray and screen picking.
#[derive(Clone, Debug)]
pub struct PickInstance<'a> {
    /// Instance hierarchy path from root (root occurrence has an empty path).
    pub path: Vec<InstanceId>,
    /// Document definition identity of the model.
    pub definition: DefinitionId,
    /// Model geometry in definition-local space.
    pub model: &'a Model,
    /// Composed local-to-world affine transformation matrix.
    pub world: DMat4,
}

impl<'a> PickInstance<'a> {
    /// Construct a new pick instance.
    pub fn new(
        path: Vec<InstanceId>,
        definition: DefinitionId,
        model: &'a Model,
        world: DMat4,
    ) -> Self {
        Self {
            path,
            definition,
            model,
            world,
        }
    }
}

/// Result of picking an entity in a transformed scene instance.
#[derive(Clone, Debug, PartialEq)]
pub struct InstanceHit {
    /// Instance hierarchy path from root to the picked instance.
    pub path: Vec<InstanceId>,
    /// Definition identity of the hit model.
    pub definition: DefinitionId,
    /// Specific model entity picked.
    pub hit: Hit,
    /// Hit position in definition-local coordinates.
    pub local_position: DVec3,
    /// Hit position transformed to world coordinates.
    pub world_position: DVec3,
}

/// Check whether a 4x4 matrix is finite, invertible, and represents a valid 3D affine transform.
fn is_valid_affine(m: &DMat4) -> bool {
    if !m.is_finite() {
        return false;
    }
    // Affine transformation condition: bottom row must be [0, 0, 0, 1].
    // In glam's column-major matrix, the w component of each column vector is row 3:
    if m.x_axis.w.abs() > 1.0e-12
        || m.y_axis.w.abs() > 1.0e-12
        || m.z_axis.w.abs() > 1.0e-12
        || (m.w_axis.w - 1.0).abs() > 1.0e-12
    {
        return false;
    }
    let det = m.determinant();
    if !det.is_finite() || det == 0.0 {
        return false;
    }
    let inv = m.inverse();
    inv.is_finite()
}

/// Transform ONLY world ray and projection into definition-local coordinates.
///
/// Returns a new [`Query`] where:
/// - `query.ray` is transformed into local space: origin transformed by `world.inverse()`,
///   direction transformed by `world.inverse()` vector transformation.
/// - `query.view.matrix` is post-multiplied: `query.view.matrix * world`.
/// - Plane, anchor, axis lock, previous direction, tangent, and cursor remain unchanged.
///
/// Rejects singular, non-affine, or non-finite matrices with [`ToolError::Degenerate`].
pub fn local_query(world: DMat4, query: &Query) -> Result<Query, ToolError> {
    if !is_valid_affine(&world) {
        return Err(ToolError::Degenerate);
    }

    let inv = world.inverse();
    let local_origin = inv.transform_point3(query.ray.origin);
    let local_direction = inv.transform_vector3(query.ray.direction);

    if !local_origin.is_finite() || !local_direction.is_finite() || local_direction == DVec3::ZERO {
        return Err(ToolError::Degenerate);
    }

    let local_view_matrix = query.view.matrix * world;
    if !local_view_matrix.is_finite() {
        return Err(ToolError::Degenerate);
    }

    let mut local = query.clone();
    local.ray = Ray {
        origin: local_origin,
        direction: local_direction,
    };
    local.view = View {
        matrix: local_view_matrix,
        size: query.view.size,
    };

    Ok(local)
}

struct CandidateInstance<'a> {
    item_index: usize,
    item: &'a PickInstance<'a>,
    inv_world: DMat4,
    min_world_depth: f64,
    bounds: (DVec3, DVec3),
}

/// Pick nearest visible face along `ray` (holes excluded) or screen-near vertex/edge respecting occlusion
/// across multiple transformed instances.
///
/// Respects foreground world depth, nested, mirrored, and non-uniform transforms.
/// Rejects malformed or singular affine matrices and prefilters per-model bounding boxes
/// before detailed per-geometry tests.
pub fn pick_instances(
    items: &[PickInstance<'_>],
    ray: Ray,
    view: View,
    cursor: DVec2,
    radius: f64,
) -> Option<InstanceHit> {
    pick_instances_section(items,ray,view,cursor,radius,None)
}
/// Scene picking with a world-space section plane, including cut-aware occlusion.
pub fn pick_instances_section(items: &[PickInstance<'_>], ray: Ray, view: View, cursor: DVec2, radius: f64, section: Option<&cindral_core::SectionPlane>) -> Option<InstanceHit> {
    if items.is_empty()
        || !cursor.is_finite()
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

    // 1. Prefilter candidate instances using local bounding boxes and screen/ray rejection.
    let mut candidates = Vec::new();
    let mut bounds = std::collections::HashMap::new();

    for (item_index, item) in items.iter().enumerate() {
        if !is_valid_affine(&item.world) || item.model.vertices().is_empty() {
            continue;
        }

        let Some((local_min, local_max)) = *bounds
            .entry(item.model as *const Model as usize)
            .or_insert_with(|| item.model.bounds())
        else {
            continue;
        };

        let inv_world = item.world.inverse();

        // Check Ray - AABB intersection in local space
        let loc_origin = inv_world.transform_point3(ray.origin);
        let loc_dir = inv_world.transform_vector3(ray_dir);
        let ray_hits_box = ray_intersects_aabb(local_min, local_max, loc_origin, loc_dir);

        // Check Screen bounding box projection
        let corners_local = aabb_corners(local_min, local_max);
        let mut min_depth = f64::INFINITY;
        let mut corners_world = [DVec3::ZERO; 8];
        for (i, c) in corners_local.iter().enumerate() {
            let cw = item.world.transform_point3(*c);
            corners_world[i] = cw;
            let depth = (cw - ray.origin).dot(ray_dir);
            if depth < min_depth {
                min_depth = depth;
            }
        }

        let cursor_near_box = box_screen_near_cursor(&corners_world, &view, cursor, radius);

        if !ray_hits_box && !cursor_near_box {
            continue;
        }

        candidates.push(CandidateInstance {
            item_index,
            item,
            inv_world,
            min_world_depth: min_depth,
            bounds: (local_min, local_max),
        });
    }

    if candidates.is_empty() {
        return None;
    }

    // Sort candidate instances front-to-back by their minimum bounding-box world depth.
    candidates.sort_by(|a, b| {
        a.min_world_depth
            .partial_cmp(&b.min_world_depth)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    // 2. Nearest visible face along picking ray across instances
    // Stored as: (candidate_idx, face_id, local_pos, world_pos, world_t)
    let mut best_face: Option<(usize, cindral_core::FaceId, DVec3, DVec3, f64)> = None;

    for (cand_idx, cand) in candidates.iter().enumerate() {
        // If the candidate's closest box corner is already deeper than a confirmed face hit,
        // no face in this candidate can be in front of `best_face`.
        if let Some((_, _, _, _, best_t)) = best_face {
            if cand.min_world_depth > best_t + TOLERANCE {
                break;
            }
        }

        let loc_origin = cand.inv_world.transform_point3(ray.origin);
        let loc_dir = cand.inv_world.transform_vector3(ray_dir);

        for (&face_id, face) in cand.item.model.faces() {
            let Some((local_point, world_t)) =
                ray_face_intersection(cand.item.model, face, loc_origin, loc_dir)
            else {
                continue;
            };

            let world_point = cand.item.world.transform_point3(local_point);
            if section.is_some_and(|p| !p.retains(world_point)) { continue; }
            if view.project(world_point).is_none() {
                continue;
            }

            if let Some((best_cidx, best_id, _, _, best_t)) = best_face {
                if (world_t - best_t).abs() > TOLERANCE {
                    if world_t < best_t {
                        best_face = Some((cand_idx, face_id, local_point, world_point, world_t));
                    }
                } else {
                    let cand_item_idx = cand.item_index;
                    let best_item_idx = candidates[best_cidx].item_index;
                    if (cand_item_idx, face_id) < (best_item_idx, best_id) {
                        best_face = Some((cand_idx, face_id, local_point, world_point, world_t));
                    }
                }
            } else {
                best_face = Some((cand_idx, face_id, local_point, world_point, world_t));
            }
        }
    }

    let max_depth = best_face.map_or(f64::INFINITY, |(_, _, _, _, t)| t + TOLERANCE);

    // 3. Vertex candidates within screen radius
    // Stored as: (cand_idx, vertex_id, local_pos, world_pos, screen_dist, ray_depth)
    let mut best_vertex: Option<(usize, VertexId, DVec3, DVec3, f64, f64)> = None;

    for (cand_idx, cand) in candidates.iter().enumerate() {
        if cand.min_world_depth > max_depth {
            continue;
        }

        for (&vertex_id, vertex) in cand.item.model.vertices() {
            let v_local = vertex.position;
            if !v_local.is_finite() {
                continue;
            }
            let v_world = cand.item.world.transform_point3(v_local);
            if section.is_some_and(|p| !p.retains(v_world)) { continue; }
            let Some(screen_pos) = view.project(v_world) else {
                continue;
            };
            let screen_dist = screen_pos.distance(cursor);
            if !screen_dist.is_finite() || screen_dist > radius {
                continue;
            }
            let ray_depth = (v_world - ray.origin).dot(ray_dir);
            if ray_depth < 0.0 || ray_depth > max_depth {
                continue;
            }

            if is_point_occluded(
                &candidates,
                cand_idx,
                Some(vertex_id),
                None,
                v_world,
                &view,
                ray.origin,
                ray_dir,
                section,
            ) {
                continue;
            }

            let candidate = (
                cand_idx,
                vertex_id,
                v_local,
                v_world,
                screen_dist,
                ray_depth,
            );
            if let Some((best_cidx, best_id, _, _, best_sdist, best_rdepth)) = best_vertex {
                if (screen_dist - best_sdist).abs() > TOLERANCE {
                    if screen_dist < best_sdist {
                        best_vertex = Some(candidate);
                    }
                } else if (ray_depth - best_rdepth).abs() > TOLERANCE {
                    if ray_depth < best_rdepth {
                        best_vertex = Some(candidate);
                    }
                } else {
                    let cand_item_idx = cand.item_index;
                    let best_item_idx = candidates[best_cidx].item_index;
                    if (cand_item_idx, vertex_id) < (best_item_idx, best_id) {
                        best_vertex = Some(candidate);
                    }
                }
            } else {
                best_vertex = Some(candidate);
            }
        }
    }

    // 4. Edge candidates within screen radius
    // Stored as: (cand_idx, edge_id, local_pos, world_pos, screen_dist, ray_depth)
    let mut best_edge: Option<(usize, cindral_core::EdgeId, DVec3, DVec3, f64, f64)> = None;

    for (cand_idx, cand) in candidates.iter().enumerate() {
        if cand.min_world_depth > max_depth {
            continue;
        }

        for (&edge_id, edge) in cand.item.model.edges() {
            let Some(va) = cand.item.model.vertices().get(&edge.vertices[0]) else {
                continue;
            };
            let Some(vb) = cand.item.model.vertices().get(&edge.vertices[1]) else {
                continue;
            };
            let a_loc = va.position;
            let b_loc = vb.position;
            if !a_loc.is_finite() || !b_loc.is_finite() || a_loc.distance(b_loc) <= TOLERANCE {
                continue;
            }

            let original_a = cand.item.world.transform_point3(a_loc);
            let original_b = cand.item.world.transform_point3(b_loc);
            let [a_world,b_world] = if let Some(p) = section { let Some(segment)=p.clip_segment(original_a,original_b) else {continue;}; segment } else { [original_a,original_b] };
            if a_world.distance(b_world) <= TOLERANCE {
                continue;
            }

            let Some((p_world, u_param)) =
                closest_point_on_segment_screen(a_world, b_world, cursor, &view)
            else {
                continue;
            };

            let Some(screen_pos) = view.project(p_world) else {
                continue;
            };
            let screen_dist = screen_pos.distance(cursor);
            if !screen_dist.is_finite() || screen_dist > radius {
                continue;
            }
            let ray_depth = (p_world - ray.origin).dot(ray_dir);
            if ray_depth < 0.0 || ray_depth > max_depth {
                continue;
            }

            if is_point_occluded(
                &candidates,
                cand_idx,
                None,
                Some(edge),
                p_world,
                &view,
                ray.origin,
                ray_dir,
                section,
            ) {
                continue;
            }

            let p_local = cand.inv_world.transform_point3(p_world);
            let _ = u_param;
            let candidate = (cand_idx, edge_id, p_local, p_world, screen_dist, ray_depth);
            if let Some((best_cidx, best_id, _, _, best_sdist, best_rdepth)) = best_edge {
                if (screen_dist - best_sdist).abs() > TOLERANCE {
                    if screen_dist < best_sdist {
                        best_edge = Some(candidate);
                    }
                } else if (ray_depth - best_rdepth).abs() > TOLERANCE {
                    if ray_depth < best_rdepth {
                        best_edge = Some(candidate);
                    }
                } else {
                    let cand_item_idx = cand.item_index;
                    let best_item_idx = candidates[best_cidx].item_index;
                    if (cand_item_idx, edge_id) < (best_item_idx, best_id) {
                        best_edge = Some(candidate);
                    }
                }
            } else {
                best_edge = Some(candidate);
            }
        }
    }

    // 5. Resolve between vertex and edge hits (sub-entity resolution)
    let best_sub: Option<(usize, Hit, DVec3, DVec3)> = match (best_vertex, best_edge) {
        (
            Some((v_cidx, vid, v_loc, v_world, vsdist, vdepth)),
            Some((e_cidx, eid, e_loc, e_world, esdist, edepth)),
        ) => {
            if (vdepth - edepth).abs() > 1.0e-3 {
                if vdepth < edepth {
                    Some((v_cidx, Hit::Vertex(vid), v_loc, v_world))
                } else {
                    Some((e_cidx, Hit::Edge(eid), e_loc, e_world))
                }
            } else if (vsdist - esdist).abs() > TOLERANCE {
                if vsdist <= esdist {
                    Some((v_cidx, Hit::Vertex(vid), v_loc, v_world))
                } else {
                    Some((e_cidx, Hit::Edge(eid), e_loc, e_world))
                }
            } else {
                Some((v_cidx, Hit::Vertex(vid), v_loc, v_world))
            }
        }
        (Some((v_cidx, vid, v_loc, v_world, _, _)), None) => {
            Some((v_cidx, Hit::Vertex(vid), v_loc, v_world))
        }
        (None, Some((e_cidx, eid, e_loc, e_world, _, _))) => {
            Some((e_cidx, Hit::Edge(eid), e_loc, e_world))
        }
        (None, None) => None,
    };

    if let Some((cidx, hit, local_position, world_position)) = best_sub {
        let item = candidates[cidx].item;
        return Some(InstanceHit {
            path: item.path.clone(),
            definition: item.definition,
            hit,
            local_position,
            world_position,
        });
    }

    if let Some((cidx, fid, local_position, world_position, _)) = best_face {
        let item = candidates[cidx].item;
        return Some(InstanceHit {
            path: item.path.clone(),
            definition: item.definition,
            hit: Hit::Face(fid),
            local_position,
            world_position,
        });
    }

    None
}

// ---------------------------------------------------------------------------
// Pure internal geometric helper functions
// ---------------------------------------------------------------------------

/// Generate 8 corners of an AABB.
fn aabb_corners(min: DVec3, max: DVec3) -> [DVec3; 8] {
    [
        DVec3::new(min.x, min.y, min.z),
        DVec3::new(max.x, min.y, min.z),
        DVec3::new(min.x, max.y, min.z),
        DVec3::new(max.x, max.y, min.z),
        DVec3::new(min.x, min.y, max.z),
        DVec3::new(max.x, min.y, max.z),
        DVec3::new(min.x, max.y, max.z),
        DVec3::new(max.x, max.y, max.z),
    ]
}

/// Ray - AABB intersection test using slab method.
fn ray_intersects_aabb(min: DVec3, max: DVec3, origin: DVec3, dir: DVec3) -> bool {
    let mut tmin = -f64::INFINITY;
    let mut tmax = f64::INFINITY;

    for i in 0..3 {
        let (o, d, bmin, bmax) = match i {
            0 => (origin.x, dir.x, min.x, max.x),
            1 => (origin.y, dir.y, min.y, max.y),
            _ => (origin.z, dir.z, min.z, max.z),
        };

        if d.abs() > 1.0e-12 {
            let inv_d = 1.0 / d;
            let mut t1 = (bmin - o) * inv_d;
            let mut t2 = (bmax - o) * inv_d;
            if t1 > t2 {
                std::mem::swap(&mut t1, &mut t2);
            }
            tmin = tmin.max(t1);
            tmax = tmax.min(t2);
            if tmin > tmax {
                return false;
            }
        } else if o < bmin - TOLERANCE || o > bmax + TOLERANCE {
            return false;
        }
    }

    tmax >= 0.0 && tmin <= tmax
}

/// Test whether any part of a box's screen projection can be within `radius` pixels of `cursor`.
fn box_screen_near_cursor(
    corners_world: &[DVec3; 8],
    view: &View,
    cursor: DVec2,
    radius: f64,
) -> bool {
    let mut clip_corners = [glam::DVec4::ZERO; 8];
    let mut all_outside_left = true;
    let mut all_outside_right = true;
    let mut all_outside_bottom = true;
    let mut all_outside_top = true;
    let mut all_outside_near = true;
    let mut all_outside_far = true;
    let mut all_behind_w = true;

    for (i, c) in corners_world.iter().enumerate() {
        let clip = view.matrix * c.extend(1.0);
        clip_corners[i] = clip;

        if clip.w > 0.0 {
            all_behind_w = false;
        }
        if clip.x >= -clip.w {
            all_outside_left = false;
        }
        if clip.x <= clip.w {
            all_outside_right = false;
        }
        if clip.y >= -clip.w {
            all_outside_bottom = false;
        }
        if clip.y <= clip.w {
            all_outside_top = false;
        }
        if clip.z >= 0.0 {
            all_outside_near = false;
        }
        if clip.z <= clip.w {
            all_outside_far = false;
        }
    }

    if all_behind_w
        || all_outside_left
        || all_outside_right
        || all_outside_bottom
        || all_outside_top
        || all_outside_near
        || all_outside_far
    {
        return false;
    }

    // If all corners are strictly in front of the camera, test 2D screen bounding box
    let mut all_projected = true;
    let mut smin = DVec2::splat(f64::INFINITY);
    let mut smax = DVec2::splat(-f64::INFINITY);

    for clip in &clip_corners {
        if let Some(sp) = clip_to_screen(*clip, view.size) {
            smin = smin.min(sp);
            smax = smax.max(sp);
        } else {
            all_projected = false;
            break;
        }
    }

    if all_projected {
        cursor.x >= smin.x - radius - TOLERANCE
            && cursor.x <= smax.x + radius + TOLERANCE
            && cursor.y >= smin.y - radius - TOLERANCE
            && cursor.y <= smax.y + radius + TOLERANCE
    } else {
        // Crosses near plane; cannot reject conservatively based on screen projection
        true
    }
}

/// Ray - Face intersection in local coordinates.
/// Returns local intersection point and distance along the input `ray_dir`.
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

fn closest_point_on_segment_screen(
    a: DVec3,
    b: DVec3,
    cursor: DVec2,
    view: &View,
) -> Option<(DVec3, f64)> {
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
    let u_clip = ((s * ca_c.w) / denom).clamp(0.0, 1.0);
    let p_world = a_clipped.lerp(b_clipped, u_clip);
    if !p_world.is_finite() {
        return None;
    }

    let u_overall = t0 + u_clip * (t1 - t0);
    Some((p_world, u_overall.clamp(0.0, 1.0)))
}

/// Occlusion test for a world-space point against all candidate instances.
#[allow(clippy::too_many_arguments)]
fn is_point_occluded(
    candidates: &[CandidateInstance<'_>],
    current_cand_idx: usize,
    exclude_vertex: Option<VertexId>,
    exclude_edge: Option<&Edge>,
    target_pos: DVec3,
    view: &View,
    ray_origin: DVec3,
    ray_dir: DVec3,
    section: Option<&cindral_core::SectionPlane>,
) -> bool {
    let inv_vp = view.matrix.inverse();
    let (test_origin, test_dir, target_dist) = if inv_vp.is_finite() {
        if let Some(screen) = view.project(target_pos) {
            let ndc_x = (screen.x / view.size.x) * 2.0 - 1.0;
            let ndc_y = 1.0 - (screen.y / view.size.y) * 2.0;
            let near = inv_vp.project_point3(DVec3::new(ndc_x, ndc_y, 0.0));
            let far = inv_vp.project_point3(DVec3::new(ndc_x, ndc_y, 1.0));
            if let Some(dir) = (far - near).try_normalize() {
                let dist = (target_pos - near).dot(dir);
                (near, dir, dist)
            } else {
                let dist = (target_pos - ray_origin).dot(ray_dir);
                (ray_origin, ray_dir, dist)
            }
        } else {
            let dist = (target_pos - ray_origin).dot(ray_dir);
            (ray_origin, ray_dir, dist)
        }
    } else {
        let dist = (target_pos - ray_origin).dot(ray_dir);
        (ray_origin, ray_dir, dist)
    };

    if target_dist <= TOLERANCE {
        return false;
    }

    for (cand_idx, cand) in candidates.iter().enumerate() {
        let is_same_instance = cand_idx == current_cand_idx;

        // Check if camera ray intersects this instance's local bounding box closer than target_dist
        {
            let (min, max) = cand.bounds;
            let loc_test_origin = cand.inv_world.transform_point3(test_origin);
            let loc_test_dir = cand.inv_world.transform_vector3(test_dir);
            if !ray_intersects_aabb(min, max, loc_test_origin, loc_test_dir) {
                continue;
            }
        }

        let loc_test_origin = cand.inv_world.transform_point3(test_origin);
        let loc_test_dir = cand.inv_world.transform_vector3(test_dir);

        for face in cand.item.model.faces().values() {
            if is_same_instance {
                if let Some(vid) = exclude_vertex {
                    if face.outer.contains(&vid) || face.holes.iter().any(|h| h.contains(&vid)) {
                        continue;
                    }
                }
                if let Some(edge) = exclude_edge {
                    let [v0, v1] = edge.vertices;
                    let in_outer = face.outer.contains(&v0) && face.outer.contains(&v1);
                    let in_hole = face
                        .holes
                        .iter()
                        .any(|h| h.contains(&v0) && h.contains(&v1));
                    if in_outer || in_hole {
                        continue;
                    }
                }
            }

            let Some((point, t)) =
                ray_face_intersection(cand.item.model, face, loc_test_origin, loc_test_dir)
            else {
                continue;
            };

            if section.is_some_and(|p| !p.retains(cand.item.world.transform_point3(point))) { continue; }
            if t < target_dist - TOLERANCE {
                return true;
            }
        }
    }

    false
}

// ---------------------------------------------------------------------------
// Unit tests and property-based verification
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use cindral_core::{Command, History};

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

        fn perspective(eye: DVec3, target: DVec3, up: DVec3, fov_y: f64, size: DVec2) -> Self {
            let view_mat = DMat4::look_at_rh(eye, target, up);
            let aspect = size.x / size.y;
            let proj_mat = DMat4::perspective_rh(fov_y, aspect, 0.1, 1000.0);
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
            let direction = (far - near).normalize();
            Ray { origin, direction }
        }
    }

    fn unit_quad_model() -> History {
        let mut h = History::new();
        h.execute(Command::AddPolyline {
            points: vec![
                DVec3::new(-1.0, -1.0, 0.0),
                DVec3::new(1.0, -1.0, 0.0),
                DVec3::new(1.0, 1.0, 0.0),
                DVec3::new(-1.0, 1.0, 0.0),
            ],
            closed: true,
        })
        .unwrap();
        h
    }

    fn unit_cube_model() -> History {
        let mut h = History::new();
        h.execute(Command::AddPolyline {
            points: vec![
                DVec3::new(0.0, 0.0, 0.0),
                DVec3::new(1.0, 0.0, 0.0),
                DVec3::new(1.0, 1.0, 0.0),
                DVec3::new(0.0, 1.0, 0.0),
            ],
            closed: true,
        })
        .unwrap();
        let face = *h.model().faces().keys().next().unwrap();
        h.execute(Command::PushPull {
            face,
            distance: 1.0,
        })
        .unwrap();
        h
    }

    #[test]
    fn local_query_rejects_singular_and_non_affine() {
        let cam = TestCamera::orthographic(
            DVec3::new(0.0, 0.0, 10.0),
            DVec3::ZERO,
            DVec3::Y,
            DVec2::splat(5.0),
            DVec2::splat(500.0),
        );
        let q = Query::new(
            DVec2::new(250.0, 250.0),
            cam.ray_at(DVec2::new(250.0, 250.0)),
            cam.view,
        );

        // Singular matrix (scale 0 in one axis)
        let singular = DMat4::from_scale(DVec3::new(0.0, 1.0, 1.0));
        assert!(matches!(
            local_query(singular, &q),
            Err(ToolError::Degenerate)
        ));

        // Zero matrix
        assert!(matches!(
            local_query(DMat4::ZERO, &q),
            Err(ToolError::Degenerate)
        ));

        // Non-finite matrix
        let mut nan_mat = DMat4::IDENTITY;
        nan_mat.x_axis.x = f64::NAN;
        assert!(matches!(
            local_query(nan_mat, &q),
            Err(ToolError::Degenerate)
        ));

        // Non-affine matrix (projective bottom row)
        let mut proj_mat = DMat4::IDENTITY;
        proj_mat.z_axis.w = 0.5;
        assert!(matches!(
            local_query(proj_mat, &q),
            Err(ToolError::Degenerate)
        ));

        let mut bad_w = DMat4::IDENTITY;
        bad_w.w_axis.w = 0.0;
        assert!(matches!(local_query(bad_w, &q), Err(ToolError::Degenerate)));
    }

    #[test]
    fn local_query_transforms_ray_and_projection_preserving_context() {
        let cam = TestCamera::perspective(
            DVec3::new(0.0, -10.0, 5.0),
            DVec3::ZERO,
            DVec3::Z,
            std::f64::consts::FRAC_PI_4,
            DVec2::new(800.0, 600.0),
        );
        let cursor = DVec2::new(400.0, 300.0);
        let mut q = Query::new(cursor, cam.ray_at(cursor), cam.view);
        q.anchor = Some(DVec3::new(1.0, 2.0, 3.0));
        q.previous_direction = Some(DVec3::X);

        let transform = DMat4::from_translation(DVec3::new(5.0, -2.0, 1.0))
            * DMat4::from_rotation_z(std::f64::consts::FRAC_PI_3)
            * DMat4::from_scale(DVec3::new(2.0, 3.0, 0.5));

        let lq = local_query(transform, &q).unwrap();

        // Cursor and local tool context remain untouched
        assert_eq!(lq.cursor, q.cursor);
        assert_eq!(lq.anchor, q.anchor);
        assert_eq!(lq.previous_direction, q.previous_direction);
        assert_eq!(lq.plane.origin, q.plane.origin);
        assert_eq!(lq.plane.normal, q.plane.normal);

        // View projection matches: view.matrix * world
        assert_eq!(lq.view.matrix, q.view.matrix * transform);

        // Ray transformation preserves point correspondence: world(local_ray(t)) == world_ray(t)
        for t in [0.0, 1.5, 10.0] {
            let p_local = lq.ray.origin + lq.ray.direction * t;
            let p_world = transform.transform_point3(p_local);
            let expected_p_world = q.ray.origin + q.ray.direction * t;
            assert!(
                (p_world - expected_p_world).length() < 1.0e-8,
                "Point mismatch at t={t}: got {p_world:?}, expected {expected_p_world:?}"
            );
        }

        // Projecting local points with local view matches projecting world points with query view
        let test_local_points = [
            DVec3::ZERO,
            DVec3::new(1.0, 0.0, 0.0),
            DVec3::new(-0.5, 2.0, 1.0),
        ];
        for pt in test_local_points {
            let pt_world = transform.transform_point3(pt);
            let proj_local = lq.view.project(pt);
            let proj_world = q.view.project(pt_world);
            match (proj_local, proj_world) {
                (Some(pl), Some(pw)) => {
                    assert!(
                        (pl - pw).length() < 1.0e-6,
                        "Projection mismatch: local={pl:?}, world={pw:?}"
                    );
                }
                (None, None) => {}
                _ => panic!("Projection visibility mismatch for point {pt:?}"),
            }
        }
    }

    #[test]
    fn local_query_nesting_composition() {
        let cam = TestCamera::perspective(
            DVec3::new(5.0, 5.0, 5.0),
            DVec3::ZERO,
            DVec3::Z,
            std::f64::consts::FRAC_PI_4,
            DVec2::new(800.0, 600.0),
        );
        let q = Query::new(
            DVec2::new(400.0, 300.0),
            cam.ray_at(DVec2::new(400.0, 300.0)),
            cam.view,
        );

        let m_parent =
            DMat4::from_translation(DVec3::new(10.0, 0.0, 0.0)) * DMat4::from_rotation_z(0.4);
        let m_child = DMat4::from_translation(DVec3::new(0.0, 5.0, -1.0))
            * DMat4::from_scale(DVec3::new(2.0, 2.0, 2.0));
        let m_composed = m_parent * m_child;

        let q_step1 = local_query(m_parent, &q).unwrap();
        let q_step2 = local_query(m_child, &q_step1).unwrap();
        let q_direct = local_query(m_composed, &q).unwrap();

        assert!((q_step2.ray.origin - q_direct.ray.origin).length() < 1.0e-8);
        assert!((q_step2.ray.direction - q_direct.ray.direction).length() < 1.0e-8);
        for row in 0..4 {
            for col in 0..4 {
                assert!(
                    (q_step2.view.matrix.col(col)[row] - q_direct.view.matrix.col(col)[row]).abs()
                        < 1.0e-8
                );
            }
        }
    }

    #[test]
    fn pick_instances_foreground_depth_ordering() {
        let h = unit_quad_model();
        let cam = TestCamera::orthographic(
            DVec3::new(0.0, 0.0, 10.0),
            DVec3::ZERO,
            DVec3::Y,
            DVec2::splat(5.0),
            DVec2::splat(500.0),
        );
        let cursor = DVec2::new(250.0, 250.0);
        let ray = cam.ray_at(cursor);

        // Two quad instances:
        // Instance A at Z = 2.0 (closer to camera at Z = 10, world depth = 8.0)
        // Instance B at Z = -2.0 (further from camera at Z = 10, world depth = 12.0)
        let inst_a = PickInstance::new(
            vec![InstanceId(1)],
            DefinitionId(10),
            h.model(),
            DMat4::from_translation(DVec3::new(0.0, 0.0, 2.0)),
        );
        let inst_b = PickInstance::new(
            vec![InstanceId(2)],
            DefinitionId(20),
            h.model(),
            DMat4::from_translation(DVec3::new(0.0, 0.0, -2.0)),
        );

        // Order [B, A]: A must win because it is in front
        let hit1 = pick_instances(
            &[inst_b.clone(), inst_a.clone()],
            ray,
            cam.view,
            cursor,
            5.0,
        )
        .unwrap();
        assert_eq!(hit1.definition, DefinitionId(10));
        assert_eq!(hit1.path, vec![InstanceId(1)]);
        assert!((hit1.world_position.z - 2.0).abs() < 1.0e-6);

        // Order [A, B]: A must still win
        let hit2 = pick_instances(&[inst_a, inst_b], ray, cam.view, cursor, 5.0).unwrap();
        assert_eq!(hit2.definition, DefinitionId(10));
        assert_eq!(hit2.path, vec![InstanceId(1)]);
    }

    #[test]
    fn pick_instances_face_hole_passes_through_to_background() {
        // Front model: 4x4 box with a 1x1 through-hole down the center
        let mut h_front = History::new();
        h_front
            .execute(Command::AddPolyline {
                points: vec![
                    DVec3::new(-2.0, -2.0, 0.0),
                    DVec3::new(2.0, -2.0, 0.0),
                    DVec3::new(2.0, 2.0, 0.0),
                    DVec3::new(-2.0, 2.0, 0.0),
                ],
                closed: true,
            })
            .unwrap();
        let front_face = *h_front.model().faces().keys().next().unwrap();
        h_front
            .execute(Command::PushPull {
                face: front_face,
                distance: 1.0,
            })
            .unwrap();
        // Inner 1x1 rectangle on top face (z = 1.0)
        h_front
            .execute(Command::AddPolyline {
                points: vec![
                    DVec3::new(-0.5, -0.5, 1.0),
                    DVec3::new(0.5, -0.5, 1.0),
                    DVec3::new(0.5, 0.5, 1.0),
                    DVec3::new(-0.5, 0.5, 1.0),
                ],
                closed: true,
            })
            .unwrap();
        let inner_face = *h_front
            .model()
            .faces()
            .iter()
            .find(|(id, f)| {
                f.plane.normal.z > 0.9
                    && (h_front.model().face_area(**id).unwrap() - 1.0).abs() < 1.0e-4
            })
            .unwrap()
            .0;
        h_front
            .execute(Command::PushPull {
                face: inner_face,
                distance: -1.0,
            })
            .unwrap();
        assert!(h_front
            .model()
            .faces()
            .values()
            .any(|f| !f.holes.is_empty()));

        // Back model: solid 4x4 quad at Z = -5.0
        let mut h_back = History::new();
        h_back
            .execute(Command::AddPolyline {
                points: vec![
                    DVec3::new(-2.0, -2.0, 0.0),
                    DVec3::new(2.0, -2.0, 0.0),
                    DVec3::new(2.0, 2.0, 0.0),
                    DVec3::new(-2.0, 2.0, 0.0),
                ],
                closed: true,
            })
            .unwrap();

        let cam = TestCamera::orthographic(
            DVec3::new(0.0, 0.0, 10.0),
            DVec3::ZERO,
            DVec3::Y,
            DVec2::splat(5.0),
            DVec2::splat(500.0),
        );

        let inst_front = PickInstance::new(
            vec![InstanceId(1)],
            DefinitionId(1),
            h_front.model(),
            DMat4::from_translation(DVec3::new(0.0, 0.0, 2.0)),
        );
        let inst_back = PickInstance::new(
            vec![InstanceId(2)],
            DefinitionId(2),
            h_back.model(),
            DMat4::from_translation(DVec3::new(0.0, 0.0, -5.0)),
        );

        // Center ray (cursor 250, 250) passes through the hole -> must hit back instance at Z = -5.0
        let center_cursor = DVec2::new(250.0, 250.0);
        let ray_center = cam.ray_at(center_cursor);
        let hit_hole = pick_instances(
            &[inst_front.clone(), inst_back.clone()],
            ray_center,
            cam.view,
            center_cursor,
            0.0,
        )
        .unwrap();
        assert_eq!(hit_hole.definition, DefinitionId(2));
        assert!((hit_hole.world_position.z - (-5.0)).abs() < 1.0e-6);

        // Ray at x = 1.5, y = 1.5 in world (cursor = 325, 175) hits solid front instance top face at Z = 3.0
        let outer_cursor = DVec2::new(325.0, 175.0);
        let ray_outer = cam.ray_at(outer_cursor);
        let hit_solid = pick_instances(
            &[inst_front, inst_back],
            ray_outer,
            cam.view,
            outer_cursor,
            0.0,
        )
        .unwrap();
        assert_eq!(hit_solid.definition, DefinitionId(1));
        assert!((hit_solid.world_position.z - 3.0).abs() < 1.0e-6);
    }

    #[test]
    fn pick_instances_mirrored_and_nonuniform_transforms() {
        let h = unit_cube_model();
        let cam = TestCamera::perspective(
            DVec3::new(0.0, -6.0, 3.0),
            DVec3::ZERO,
            DVec3::Z,
            std::f64::consts::FRAC_PI_4,
            DVec2::new(600.0, 600.0),
        );

        // Mirrored instance: negative determinant
        let mirror_world = DMat4::from_scale(DVec3::new(-1.0, 1.0, 1.0))
            * DMat4::from_translation(DVec3::new(-0.5, -0.5, -0.5));
        assert!(mirror_world.determinant() < 0.0);

        let inst = PickInstance::new(vec![], DefinitionId(1), h.model(), mirror_world);
        let cursor = DVec2::new(300.0, 300.0);
        let ray = cam.ray_at(cursor);

        let hit = pick_instances(&[inst], ray, cam.view, cursor, 5.0).unwrap();
        assert_eq!(hit.definition, DefinitionId(1));
        assert!(hit.world_position.is_finite());
        assert!(
            (mirror_world.transform_point3(hit.local_position) - hit.world_position).length()
                < 1.0e-6
        );

        // Non-uniform scaling instance
        let nonunif_world = DMat4::from_scale(DVec3::new(5.0, 0.2, 1.0));
        let inst_nu = PickInstance::new(vec![], DefinitionId(2), h.model(), nonunif_world);
        let hit_nu = pick_instances(&[inst_nu], ray, cam.view, cursor, 10.0).unwrap();
        assert_eq!(hit_nu.definition, DefinitionId(2));
        assert!(
            (nonunif_world.transform_point3(hit_nu.local_position) - hit_nu.world_position)
                .length()
                < 1.0e-6
        );
    }

    #[test]
    fn pick_instances_screen_radius_vertex_and_edge() {
        let h = unit_quad_model();
        let cam = TestCamera::orthographic(
            DVec3::new(0.0, 0.0, 10.0),
            DVec3::ZERO,
            DVec3::Y,
            DVec2::splat(5.0),
            DVec2::splat(500.0),
        );

        let inst = PickInstance::new(
            vec![InstanceId(99)],
            DefinitionId(1),
            h.model(),
            DMat4::IDENTITY,
        );

        // Top-right vertex is at (1.0, 1.0, 0.0).
        // Projected: x = (1.0 / 5.0 + 1) * 0.5 * 500 = 300.0
        //            y = (1 - 1.0 / 5.0) * 0.5 * 500 = 200.0
        let vertex_screen = cam.view.project(DVec3::new(1.0, 1.0, 0.0)).unwrap();
        assert!((vertex_screen - DVec2::new(300.0, 200.0)).length() < 1.0e-6);

        // Cursor slightly offset from vertex by 3 pixels (outside ray intersection with face)
        let cursor_near_vertex = vertex_screen + DVec2::new(3.0, 0.0);
        let ray_near_vertex = cam.ray_at(cursor_near_vertex);

        let hit_v = pick_instances(
            std::slice::from_ref(&inst),
            ray_near_vertex,
            cam.view,
            cursor_near_vertex,
            5.0,
        )
        .unwrap();
        assert!(matches!(hit_v.hit, Hit::Vertex(_)));
        assert!((hit_v.world_position - DVec3::new(1.0, 1.0, 0.0)).length() < 1.0e-6);

        // Top edge connects (-1, 1, 0) and (1, 1, 0), which on screen is horizontal at y = 200.
        // Pick near midpoint (0, 1, 0) -> screen (250, 200). Cursor at (250, 197).
        let cursor_near_edge = DVec2::new(250.0, 197.0);
        let ray_near_edge = cam.ray_at(cursor_near_edge);

        let hit_e =
            pick_instances(&[inst], ray_near_edge, cam.view, cursor_near_edge, 5.0).unwrap();
        assert!(matches!(hit_e.hit, Hit::Edge(_)));
        assert!((hit_e.world_position.y - 1.0).abs() < 1.0e-3);
    }

    #[test]
    fn pick_instances_prefilter_5000_boxes_benchmark() {
        let h = unit_cube_model();
        let cam = TestCamera::perspective(
            DVec3::new(50.5, 100.5, 50.0),
            DVec3::new(50.5, 100.5, 0.0),
            DVec3::Y,
            std::f64::consts::FRAC_PI_4,
            DVec2::new(1024.0, 768.0),
        );

        // Create 5000 distinct box instances arranged in a 50x100 grid
        let mut instances = Vec::with_capacity(5000);
        for i in 0..50 {
            for j in 0..100 {
                let id = (i * 100 + j) as u64;
                let transform =
                    DMat4::from_translation(DVec3::new((i as f64) * 2.0, (j as f64) * 2.0, 0.0));
                instances.push(PickInstance::new(
                    vec![InstanceId(id)],
                    DefinitionId(id),
                    h.model(),
                    transform,
                ));
            }
        }
        assert_eq!(instances.len(), 5000);

        // Pick target box (25, 50) directly from above
        let target_cursor = DVec2::new(512.0, 384.0);
        let ray = cam.ray_at(target_cursor);

        let hit = pick_instances(&instances, ray, cam.view, target_cursor, 5.0).unwrap();

        // Must hit the expected target box
        assert_eq!(hit.definition, DefinitionId(25 * 100 + 50));
        assert!((hit.world_position.x - 50.5).abs() < 1.0);
        assert!((hit.world_position.y - 100.5).abs() < 1.0);
        assert!((hit.world_position.z - 1.0).abs() < 1.0e-5);
    }

    #[test]
    fn seeded_property_transform_invariants() {
        // Pseudo-random LCG for deterministic property testing without external test runner
        let mut rng_state: u64 = 0xdeadbeef_cafebabe;
        let mut next_f64 = |min: f64, max: f64| -> f64 {
            rng_state = rng_state.wrapping_mul(6364136223846793005).wrapping_add(1);
            let unit = (rng_state >> 11) as f64 / ((1u64 << 53) as f64);
            min + unit * (max - min)
        };

        let cam = TestCamera::perspective(
            DVec3::new(10.0, -15.0, 12.0),
            DVec3::ZERO,
            DVec3::Z,
            std::f64::consts::FRAC_PI_4,
            DVec2::new(800.0, 600.0),
        );

        for _ in 0..200 {
            let tx = next_f64(-50.0, 50.0);
            let ty = next_f64(-50.0, 50.0);
            let tz = next_f64(-50.0, 50.0);
            let rot_z = next_f64(-std::f64::consts::PI, std::f64::consts::PI);
            let sx = next_f64(-5.0, 5.0);
            let sy = next_f64(-5.0, 5.0);
            let sz = next_f64(-5.0, 5.0);

            if sx.abs() < 0.05 || sy.abs() < 0.05 || sz.abs() < 0.05 {
                continue;
            }

            let transform = DMat4::from_translation(DVec3::new(tx, ty, tz))
                * DMat4::from_rotation_z(rot_z)
                * DMat4::from_scale(DVec3::new(sx, sy, sz));

            let cursor = DVec2::new(next_f64(100.0, 700.0), next_f64(100.0, 500.0));
            let q = Query::new(cursor, cam.ray_at(cursor), cam.view);

            let lq = match local_query(transform, &q) {
                Ok(res) => res,
                Err(_) => continue,
            };

            // Invariant 1: Local ray parametrized point maps to world ray parametrized point
            for t in [0.0, 1.0, 5.0] {
                let p_loc = lq.ray.origin + lq.ray.direction * t;
                let p_world = transform.transform_point3(p_loc);
                let expected_world = q.ray.origin + q.ray.direction * t;
                assert!(
                    (p_world - expected_world).length() < 1.0e-6,
                    "Ray parameter invariant failed"
                );
            }

            // Invariant 2: Projecting local point equals projecting transformed world point
            let test_pt_loc = DVec3::new(
                next_f64(-1.0, 1.0),
                next_f64(-1.0, 1.0),
                next_f64(-1.0, 1.0),
            );
            let test_pt_world = transform.transform_point3(test_pt_loc);
            let pl = lq.view.project(test_pt_loc);
            let pw = q.view.project(test_pt_world);
            if let (Some(s1), Some(s2)) = (pl, pw) {
                assert!(
                    (s1 - s2).length() < 1.0e-5,
                    "Projected point invariant failed"
                );
            }
        }
    }
}
