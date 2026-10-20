//! Original robust BSP boundary CSG (Constructive Solid Geometry) helper.
//!
//! Provides boolean union and subtraction operations on closed solids represented
//! as consistently outward-wound convex planar polygons.
//!
//! Geometry computations use `f64` with a fixed linear tolerance of `1.0e-7` meters.
//! Splitting is performed using canonical Binary Space Partitioning (BSP) trees with
//! Sutherland-Hodgman convex polygon clipping, explicit coplanar classification,
//! and recursion/workload bounding to prevent stack explosion on malformed inputs.

#![forbid(unsafe_code)]
#![allow(dead_code)]

use glam::DVec3;

/// Single geometric tolerance, in meters, matching core crate conventions.
use crate::TOLERANCE;

/// Maximum recursion depth allowed during BSP tree construction and clipping.
const MAX_BSP_DEPTH: usize = 64;

/// A planar convex polygon with outward-oriented winding.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Polygon {
    /// Ordered sequence of convex boundary vertices with outward winding.
    pub vertices: Vec<DVec3>,
}

impl Polygon {
    /// Creates and validates a new convex polygon.
    ///
    /// Returns `None` if:
    /// - Coordinates contain non-finite values (`NaN` or infinity).
    /// - Consecutive duplicate vertices or collinear redundant vertices collapse the polygon.
    /// - The cleaned vertex count is less than 3.
    /// - The Newell area is degenerate (`<= TOLERANCE`).
    pub(crate) fn new(vertices: Vec<DVec3>) -> Option<Self> {
        let cleaned = clean_vertices(&vertices)?;
        Some(Self { vertices: cleaned })
    }

    /// Computes the normalized unit normal using Newell's method.
    pub(crate) fn normal(&self) -> DVec3 {
        let n = newell_normal(&self.vertices);
        let len = n.length();
        if len > TOLERANCE * TOLERANCE {
            n / len
        } else {
            DVec3::Z
        }
    }

    /// Inverts the polygon in place by reversing vertex order.
    pub(crate) fn flip(&mut self) {
        self.vertices.reverse();
    }

    /// Tests whether the polygon is strictly convex with respect to its normal.
    pub(crate) fn is_convex(&self) -> bool {
        let n = self.vertices.len();
        if n < 3 {
            return false;
        }
        let normal = self.normal();
        for i in 0..n {
            let v0 = self.vertices[i];
            let v1 = self.vertices[(i + 1) % n];
            let v2 = self.vertices[(i + 2) % n];
            let e1 = v1 - v0;
            let e2 = v2 - v1;
            let cross = e1.cross(e2);
            if cross.dot(normal) < -TOLERANCE {
                return false;
            }
        }
        true
    }

    /// Computes the signed tetrahedral volume contribution of this polygon relative to origin.
    #[cfg(test)]
    pub(crate) fn signed_volume_contribution(&self) -> f64 {
        if self.vertices.len() < 3 {
            return 0.0;
        }
        let v0 = self.vertices[0];
        let mut vol = 0.0;
        for i in 1..(self.vertices.len() - 1) {
            let v1 = self.vertices[i];
            let v2 = self.vertices[i + 1];
            vol += v0.dot(v1.cross(v2));
        }
        vol / 6.0
    }
}

/// Computes the total signed volume of a collection of closed boundary polygons.
pub(crate) fn total_signed_volume(polygons: &[Polygon]) -> f64 {
    let Some(origin) = polygons.first().and_then(|p| p.vertices.first()).copied() else {
        return 0.;
    };
    polygons
        .iter()
        .map(|p| {
            let a = p.vertices[0] - origin;
            p.vertices[1..]
                .windows(2)
                .map(|w| a.dot((w[0] - origin).cross(w[1] - origin)) / 6.)
                .sum::<f64>()
        })
        .sum()
}

/// Computes the unnormalized Newell normal vector for a 3D polygon.
///
/// The magnitude of the returned vector equals twice the polygon area.
fn newell_normal(vertices: &[DVec3]) -> DVec3 {
    let Some(origin) = vertices.first().copied() else {
        return DVec3::ZERO;
    };
    vertices
        .iter()
        .zip(vertices.iter().cycle().skip(1))
        .map(|(a, b)| (*a - origin).cross(*b - origin))
        .sum()
}

/// Sanitizes vertex lists by validating finiteness, welding consecutive duplicates,
/// eliminating collinear midpoints, and checking minimum area.
fn clean_vertices(vertices: &[DVec3]) -> Option<Vec<DVec3>> {
    for v in vertices {
        if !v.is_finite() {
            return None;
        }
    }

    let mut cleaned: Vec<DVec3> = Vec::with_capacity(vertices.len());
    for &v in vertices {
        if let Some(&last) = cleaned.last() {
            if last.distance(v) <= TOLERANCE {
                continue;
            }
        }
        cleaned.push(v);
    }
    while cleaned.len() > 1
        && cleaned
            .last()
            .is_some_and(|last| last.distance(cleaned[0]) <= TOLERANCE)
    {
        cleaned.pop();
    }

    if cleaned.len() < 3 {
        return None;
    }

    // Eliminate redundant collinear vertices
    let mut changed = true;
    while changed && cleaned.len() >= 3 {
        changed = false;
        let n = cleaned.len();
        let mut to_remove = None;
        for i in 0..n {
            let prev = cleaned[(i + n - 1) % n];
            let curr = cleaned[i];
            let next = cleaned[(i + 1) % n];

            let d1 = curr - prev;
            let d2 = next - curr;
            let l1 = d1.length();
            let l2 = d2.length();

            if l1 <= TOLERANCE || l2 <= TOLERANCE {
                to_remove = Some(i);
                break;
            }

            let chord = next - prev;
            let chord_len = chord.length();
            if chord_len <= TOLERANCE {
                to_remove = Some(i);
                break;
            }

            let cross = d1.cross(d2);
            let dist = cross.length() / chord_len;
            if dist <= TOLERANCE && d1.dot(d2) > 0.0 {
                to_remove = Some(i);
                break;
            }
        }

        if let Some(idx) = to_remove {
            cleaned.remove(idx);
            changed = true;
        }
    }

    if cleaned.len() < 3 {
        return None;
    }

    let normal = newell_normal(&cleaned);
    let area2 = normal.length();
    if area2 * 0.5 <= TOLERANCE * TOLERANCE {
        return None;
    }

    Some(cleaned)
}

/// An infinite geometric splitting plane `normal.dot(p) - distance = 0`.
#[derive(Clone, Copy, Debug)]
struct Plane {
    normal: DVec3,
    distance: f64,
}

/// Classification outcome of splitting a convex polygon against a plane.
enum SplitResult {
    /// Completely on the plane facing the same direction.
    CoplanarFront(Polygon),
    /// Completely on the plane facing the opposite direction.
    CoplanarBack(Polygon),
    /// Entirely in the positive half-space (front/outside).
    Front(Polygon),
    /// Entirely in the negative half-space (back/inside).
    Back(Polygon),
    /// Straddles the plane and is split into front and back convex polygons.
    Spanning { front: Polygon, back: Polygon },
    /// Collapsed to degenerate or invalid geometry.
    Degenerate,
}

impl Plane {
    /// Constructs a plane from an existing polygon using its Newell normal.
    fn from_polygon(poly: &Polygon) -> Option<Self> {
        let n = newell_normal(&poly.vertices);
        let len = n.length();
        if len <= TOLERANCE * TOLERANCE {
            return None;
        }
        let unit_normal = n / len;
        let mut sum_d = 0.0;
        for v in &poly.vertices {
            sum_d += unit_normal.dot(*v);
        }
        let distance = sum_d / (poly.vertices.len() as f64);
        Some(Self {
            normal: unit_normal,
            distance,
        })
    }

    /// Reverses the plane orientation.
    fn flip(&mut self) {
        self.normal = -self.normal;
        self.distance = -self.distance;
    }

    /// Computes the signed perpendicular distance from a point to this plane.
    fn signed_distance(&self, pt: DVec3) -> f64 {
        self.normal.dot(pt) - self.distance
    }

    /// Splits a convex polygon by this plane using Sutherland-Hodgman clipping.
    fn split_polygon(&self, poly: Polygon) -> SplitResult {
        let n = poly.vertices.len();
        if n < 3 {
            return SplitResult::Degenerate;
        }

        let mut types = Vec::with_capacity(n);
        let mut num_front = 0;
        let mut num_back = 0;

        for &v in &poly.vertices {
            let dist = self.signed_distance(v);
            if dist > TOLERANCE {
                types.push(1);
                num_front += 1;
            } else if dist < -TOLERANCE {
                types.push(-1);
                num_back += 1;
            } else {
                types.push(0);
            }
        }

        if num_front == 0 && num_back == 0 {
            let poly_n = poly.normal();
            if poly_n.dot(self.normal) > 0.0 {
                return SplitResult::CoplanarFront(poly);
            } else {
                return SplitResult::CoplanarBack(poly);
            }
        }

        if num_back == 0 {
            return SplitResult::Front(poly);
        }

        if num_front == 0 {
            return SplitResult::Back(poly);
        }

        let mut f_verts = Vec::with_capacity(n + 2);
        let mut b_verts = Vec::with_capacity(n + 2);

        for i in 0..n {
            let j = (i + 1) % n;
            let vi = poly.vertices[i];
            let vj = poly.vertices[j];
            let ti = types[i];
            let tj = types[j];

            if ti >= 0 {
                f_verts.push(vi);
            }
            if ti <= 0 {
                b_verts.push(vi);
            }

            if (ti == 1 && tj == -1) || (ti == -1 && tj == 1) {
                let di = self.signed_distance(vi);
                let dj = self.signed_distance(vj);
                let denom = di - dj;
                let t = if denom.abs() > 1e-15 {
                    (di / denom).clamp(0.0, 1.0)
                } else {
                    0.5
                };
                let mut pt = vi + t * (vj - vi);
                let err = self.signed_distance(pt);
                pt -= err * self.normal;

                f_verts.push(pt);
                b_verts.push(pt);
            }
        }

        let f_poly = Polygon::new(f_verts);
        let b_poly = Polygon::new(b_verts);

        match (f_poly, b_poly) {
            (Some(f), Some(b)) => SplitResult::Spanning { front: f, back: b },
            (Some(f), None) => SplitResult::Front(f),
            (None, Some(b)) => SplitResult::Back(b),
            (None, None) => SplitResult::Degenerate,
        }
    }
}

/// Binary Space Partitioning node for boundary solid evaluation.
#[derive(Clone, Debug)]
struct BspNode {
    truncated: std::cell::Cell<bool>,
    plane: Option<Plane>,
    polygons: Vec<Polygon>,
    front: Option<Box<BspNode>>,
    back: Option<Box<BspNode>>,
}

impl BspNode {
    /// Creates an empty BSP node.
    fn new() -> Self {
        Self {
            truncated: std::cell::Cell::new(false),
            plane: None,
            polygons: Vec::new(),
            front: None,
            back: None,
        }
    }

    /// Constructs a BSP tree from a list of polygons.
    fn from_polygons(polygons: Vec<Polygon>) -> Self {
        let mut node = Self::new();
        node.build(polygons, 0);
        node
    }

    fn check(&self) -> Result<(), crate::KernelError> {
        if self.truncated.get() {
            return Err(crate::KernelError::InvalidTopology(
                "CSG depth budget exceeded".into(),
            ));
        }
        if let Some(front) = &self.front {
            front.check()?;
        }
        if let Some(back) = &self.back {
            back.check()?;
        }
        Ok(())
    }
    /// Inverts the solid representation in place.
    fn invert(&mut self) {
        for poly in &mut self.polygons {
            poly.flip();
        }
        if let Some(plane) = &mut self.plane {
            plane.flip();
        }
        std::mem::swap(&mut self.front, &mut self.back);
        if let Some(front) = &mut self.front {
            front.invert();
        }
        if let Some(back) = &mut self.back {
            back.invert();
        }
    }

    /// Clips polygons against this solid, keeping portions outside the solid boundary.
    fn clip_polygons(&self, polygons: Vec<Polygon>, depth: usize) -> Vec<Polygon> {
        if polygons.len() > 100_000 {
            self.truncated.set(true);
            return polygons;
        }
        if polygons.is_empty() {
            return Vec::new();
        }
        let plane = match &self.plane {
            Some(p) => p,
            None => return polygons,
        };
        if depth >= MAX_BSP_DEPTH {
            return polygons;
        }

        let mut front_list = Vec::new();
        let mut back_list = Vec::new();

        for poly in polygons {
            match plane.split_polygon(poly) {
                SplitResult::CoplanarFront(p) => front_list.push(p),
                SplitResult::CoplanarBack(p) => back_list.push(p),
                SplitResult::Front(p) => front_list.push(p),
                SplitResult::Back(p) => back_list.push(p),
                SplitResult::Spanning { front, back } => {
                    front_list.push(front);
                    back_list.push(back);
                }
                SplitResult::Degenerate => {}
            }
        }

        let mut result_front = if let Some(ref front_node) = self.front {
            front_node.clip_polygons(front_list, depth + 1)
        } else {
            front_list
        };

        let result_back = if let Some(ref back_node) = self.back {
            back_node.clip_polygons(back_list, depth + 1)
        } else {
            Vec::new() // Interior of solid: clipped away
        };

        result_front.extend(result_back);
        result_front
    }

    /// Recursively clips all polygons of this tree against another BSP tree.
    fn clip_to(&mut self, bsp: &BspNode) {
        self.polygons = bsp.clip_polygons(self.polygons.clone(), 0);
        if let Some(front) = &mut self.front {
            front.clip_to(bsp);
        }
        if let Some(back) = &mut self.back {
            back.clip_to(bsp);
        }
    }

    /// Collects all polygons in this subtree.
    fn all_polygons(&self) -> Vec<Polygon> {
        let mut result = self.polygons.clone();
        if let Some(front) = &self.front {
            result.extend(front.all_polygons());
        }
        if let Some(back) = &self.back {
            result.extend(back.all_polygons());
        }
        result
    }

    /// Recursively builds the BSP tree with depth limiting.
    fn build(&mut self, mut polygons: Vec<Polygon>, depth: usize) {
        if polygons.is_empty() {
            return;
        }
        if depth >= MAX_BSP_DEPTH || polygons.len() > 100_000 {
            self.truncated.set(true);
            self.polygons.extend(polygons);
            return;
        }

        if self.plane.is_none() {
            let mut valid_idx = None;
            for (i, p) in polygons.iter().enumerate() {
                if Plane::from_polygon(p).is_some() {
                    valid_idx = Some(i);
                    break;
                }
            }
            if let Some(idx) = valid_idx {
                let first_poly = polygons.remove(idx);
                let Some(plane) = Plane::from_polygon(&first_poly) else {
                    return;
                };
                self.plane = Some(plane);
                self.polygons.push(first_poly);
            } else {
                return;
            }
        }

        let Some(plane) = self.plane else { return };

        let mut front_list = Vec::new();
        let mut back_list = Vec::new();

        for poly in polygons {
            match plane.split_polygon(poly) {
                SplitResult::CoplanarFront(p) => self.polygons.push(p),
                SplitResult::CoplanarBack(p) => self.polygons.push(p),
                SplitResult::Front(p) => front_list.push(p),
                SplitResult::Back(p) => back_list.push(p),
                SplitResult::Spanning { front, back } => {
                    front_list.push(front);
                    back_list.push(back);
                }
                SplitResult::Degenerate => {}
            }
        }

        if !front_list.is_empty() {
            let front_node = self.front.get_or_insert_with(|| Box::new(BspNode::new()));
            front_node.build(front_list, depth + 1);
        }
        if !back_list.is_empty() {
            let back_node = self.back.get_or_insert_with(|| Box::new(BspNode::new()));
            back_node.build(back_list, depth + 1);
        }
    }
}

/// Filters and sanitizes an initial list of polygons.
fn filter_valid(polygons: Vec<Polygon>) -> Vec<Polygon> {
    polygons
        .into_iter()
        .filter_map(|p| Polygon::new(p.vertices))
        .collect()
}

/// Triangulates a simple non-convex planar polygon via 2D ear clipping.
fn triangulate_polygon(poly: &Polygon) -> Vec<Polygon> {
    let n = poly.vertices.len();
    if n <= 3 {
        return vec![poly.clone()];
    }

    let normal = poly.normal();
    let abs_n = normal.abs();
    let (ax, ay) = if abs_n.x >= abs_n.y && abs_n.x >= abs_n.z {
        (1, 2)
    } else if abs_n.y >= abs_n.z {
        (0, 2)
    } else {
        (0, 1)
    };

    let to_2d = |v: DVec3| -> (f64, f64) {
        let coords = [v.x, v.y, v.z];
        (coords[ax], coords[ay])
    };

    let mut indices: Vec<usize> = (0..n).collect();
    let mut result = Vec::new();

    let mut area2d = 0.0;
    for i in 0..n {
        let (x0, y0) = to_2d(poly.vertices[i]);
        let (x1, y1) = to_2d(poly.vertices[(i + 1) % n]);
        area2d += x0 * y1 - x1 * y0;
    }
    let ccw = area2d > 0.0;

    let is_point_in_tri = |p: (f64, f64), a: (f64, f64), b: (f64, f64), c: (f64, f64)| -> bool {
        let cross = |p1: (f64, f64), p2: (f64, f64), p3: (f64, f64)| -> f64 {
            (p2.0 - p1.0) * (p3.1 - p1.1) - (p2.1 - p1.1) * (p3.0 - p1.0)
        };
        let c1 = cross(a, b, p);
        let c2 = cross(b, c, p);
        let c3 = cross(c, a, p);
        let has_neg = (c1 < -TOLERANCE) || (c2 < -TOLERANCE) || (c3 < -TOLERANCE);
        let has_pos = (c1 > TOLERANCE) || (c2 > TOLERANCE) || (c3 > TOLERANCE);
        !(has_neg && has_pos)
    };

    let mut iterations = 0;
    while indices.len() > 3 && iterations < 200 {
        iterations += 1;
        let m = indices.len();
        let mut ear_found = false;

        for i in 0..m {
            let prev = indices[(i + m - 1) % m];
            let curr = indices[i];
            let next = indices[(i + 1) % m];

            let p_prev = to_2d(poly.vertices[prev]);
            let p_curr = to_2d(poly.vertices[curr]);
            let p_next = to_2d(poly.vertices[next]);

            let cross = (p_curr.0 - p_prev.0) * (p_next.1 - p_curr.1)
                - (p_curr.1 - p_prev.1) * (p_next.0 - p_curr.0);
            let is_convex = if ccw {
                cross > TOLERANCE
            } else {
                cross < -TOLERANCE
            };

            if !is_convex {
                continue;
            }

            let mut has_inside = false;
            for &other in &indices {
                if other == prev || other == curr || other == next {
                    continue;
                }
                let pt = to_2d(poly.vertices[other]);
                if is_point_in_tri(pt, p_prev, p_curr, p_next) {
                    has_inside = true;
                    break;
                }
            }

            if !has_inside {
                if let Some(tri) = Polygon::new(vec![
                    poly.vertices[prev],
                    poly.vertices[curr],
                    poly.vertices[next],
                ]) {
                    result.push(tri);
                }
                indices.remove(i);
                ear_found = true;
                break;
            }
        }

        if !ear_found {
            break;
        }
    }

    if indices.len() == 3 {
        if let Some(tri) = Polygon::new(vec![
            poly.vertices[indices[0]],
            poly.vertices[indices[1]],
            poly.vertices[indices[2]],
        ]) {
            result.push(tri);
        }
    } else if result.is_empty() {
        for i in 1..(n - 1) {
            if let Some(tri) = Polygon::new(vec![
                poly.vertices[0],
                poly.vertices[i],
                poly.vertices[i + 1],
            ]) {
                result.push(tri);
            }
        }
    }

    result
}

/// Removes duplicate polygons and decomposes any non-convex fragments into convex pieces.
fn clean_and_filter_polygons(polygons: Vec<Polygon>) -> Vec<Polygon> {
    let mut cleaned = Vec::with_capacity(polygons.len());
    for p in polygons {
        if let Some(poly) = Polygon::new(p.vertices) {
            let convex_pieces = if poly.is_convex() {
                vec![poly]
            } else {
                triangulate_polygon(&poly)
            };

            for piece in convex_pieces {
                let mut is_dup = false;
                for existing in &cleaned {
                    if is_same_polygon(&piece, existing) {
                        is_dup = true;
                        break;
                    }
                }
                if !is_dup {
                    cleaned.push(piece);
                }
            }
        }
    }
    cleaned
}

/// Checks whether two polygons share identical vertices under cyclic rotation.
fn is_same_polygon(p1: &Polygon, p2: &Polygon) -> bool {
    if p1.vertices.len() != p2.vertices.len() {
        return false;
    }
    let n = p1.vertices.len();
    if n == 0 {
        return true;
    }
    for k in 0..n {
        let mut matched = true;
        for i in 0..n {
            if p1.vertices[i].distance(p2.vertices[(i + k) % n]) > TOLERANCE {
                matched = false;
                break;
            }
        }
        if matched {
            return true;
        }
    }
    false
}

/// Computes the boolean union of two solid polyhedra represented by convex polygons.
///
/// Both inputs must be consistently outward-wound closed solids. The output consists of
/// outward-wound convex polygons with no duplicate coincident caps.
pub(crate) fn union(a: Vec<Polygon>, b: Vec<Polygon>) -> Result<Vec<Polygon>, crate::KernelError> {
    let a = filter_valid(a);
    let b = filter_valid(b);
    if a.is_empty() {
        return Ok(b);
    }
    if b.is_empty() {
        return Ok(a);
    }

    let mut a_node = BspNode::from_polygons(a);
    let mut b_node = BspNode::from_polygons(b);

    a_node.check()?;
    b_node.check()?;
    a_node.clip_to(&b_node);
    b_node.clip_to(&a_node);
    b_node.invert();
    b_node.clip_to(&a_node);
    b_node.invert();
    a_node.build(b_node.all_polygons(), 0);

    a_node.check()?;
    b_node.check()?;
    Ok(clean_and_filter_polygons(a_node.all_polygons()))
}

/// Computes the boolean difference `a \ b` of two solid polyhedra.
///
/// Discards portions of solid `a` that lie inside solid `b`, and includes inverted interior
/// boundaries of `b` inside `a`. Produces outward-wound convex polygons without duplicate caps.
pub(crate) fn subtract(
    a: Vec<Polygon>,
    b: Vec<Polygon>,
) -> Result<Vec<Polygon>, crate::KernelError> {
    let a = filter_valid(a);
    let b = filter_valid(b);
    if a.is_empty() {
        return Ok(Vec::new());
    }
    if b.is_empty() {
        return Ok(a);
    }

    let mut a_node = BspNode::from_polygons(a);
    let mut b_node = BspNode::from_polygons(b);

    a_node.invert();
    a_node.check()?;
    b_node.check()?;
    a_node.clip_to(&b_node);
    b_node.clip_to(&a_node);
    b_node.invert();
    b_node.clip_to(&a_node);
    b_node.invert();
    a_node.build(b_node.all_polygons(), 0);
    a_node.invert();

    a_node.check()?;
    b_node.check()?;
    Ok(clean_and_filter_polygons(a_node.all_polygons()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_cuboid(min: DVec3, max: DVec3) -> Vec<Polygon> {
        vec![
            // Front (+Z)
            Polygon::new(vec![
                DVec3::new(min.x, min.y, max.z),
                DVec3::new(max.x, min.y, max.z),
                DVec3::new(max.x, max.y, max.z),
                DVec3::new(min.x, max.y, max.z),
            ])
            .unwrap(),
            // Back (-Z)
            Polygon::new(vec![
                DVec3::new(min.x, max.y, min.z),
                DVec3::new(max.x, max.y, min.z),
                DVec3::new(max.x, min.y, min.z),
                DVec3::new(min.x, min.y, min.z),
            ])
            .unwrap(),
            // Right (+X)
            Polygon::new(vec![
                DVec3::new(max.x, min.y, min.z),
                DVec3::new(max.x, max.y, min.z),
                DVec3::new(max.x, max.y, max.z),
                DVec3::new(max.x, min.y, max.z),
            ])
            .unwrap(),
            // Left (-X)
            Polygon::new(vec![
                DVec3::new(min.x, min.y, max.z),
                DVec3::new(min.x, max.y, max.z),
                DVec3::new(min.x, max.y, min.z),
                DVec3::new(min.x, min.y, min.z),
            ])
            .unwrap(),
            // Top (+Y)
            Polygon::new(vec![
                DVec3::new(min.x, max.y, min.z),
                DVec3::new(min.x, max.y, max.z),
                DVec3::new(max.x, max.y, max.z),
                DVec3::new(max.x, max.y, min.z),
            ])
            .unwrap(),
            // Bottom (-Y)
            Polygon::new(vec![
                DVec3::new(min.x, min.y, min.z),
                DVec3::new(max.x, min.y, min.z),
                DVec3::new(max.x, min.y, max.z),
                DVec3::new(min.x, min.y, max.z),
            ])
            .unwrap(),
        ]
    }

    fn assert_all_convex(polys: &[Polygon]) {
        for (i, p) in polys.iter().enumerate() {
            assert!(p.is_convex(), "Polygon {} is not convex: {:?}", i, p);
        }
    }

    #[test]
    fn test_polygon_new_validation() {
        // Valid triangle
        let tri = Polygon::new(vec![DVec3::ZERO, DVec3::X, DVec3::Y]);
        assert!(tri.is_some());

        // Less than 3 vertices
        let line = Polygon::new(vec![DVec3::ZERO, DVec3::X]);
        assert!(line.is_none());

        // Collinear triangle
        let col = Polygon::new(vec![DVec3::ZERO, DVec3::X, DVec3::X * 2.0]);
        assert!(col.is_none());

        // Duplicate consecutive vertices cleaned
        let dup = Polygon::new(vec![
            DVec3::ZERO,
            DVec3::ZERO,
            DVec3::X,
            DVec3::X + DVec3::Y,
            DVec3::Y,
            DVec3::ZERO,
        ]);
        assert!(dup.is_some());
        assert_eq!(dup.unwrap().vertices.len(), 4);

        // Non-finite vertices
        let nan_poly = Polygon::new(vec![DVec3::new(f64::NAN, 0.0, 0.0), DVec3::X, DVec3::Y]);
        assert!(nan_poly.is_none());

        // Degenerate sliver
        let sliver = Polygon::new(vec![DVec3::ZERO, DVec3::X, DVec3::X + DVec3::Y * 1e-10]);
        assert!(sliver.is_none());
    }

    #[test]
    fn test_signed_volume_cuboid() {
        let c1 = make_cuboid(DVec3::ZERO, DVec3::ONE);
        assert_all_convex(&c1);
        let v1 = total_signed_volume(&c1);
        assert!((v1 - 1.0).abs() < 1e-6);

        let c2 = make_cuboid(DVec3::new(2.0, 3.0, -1.0), DVec3::new(5.0, 7.0, 2.0));
        assert_all_convex(&c2);
        let v2 = total_signed_volume(&c2);
        // (5-2) * (7-3) * (2 - (-1)) = 3 * 4 * 3 = 36.0
        assert!((v2 - 36.0).abs() < 1e-6);
    }

    #[test]
    fn test_cuboid_union_disjoint() {
        let c1 = make_cuboid(DVec3::ZERO, DVec3::ONE);
        let c2 = make_cuboid(DVec3::new(3.0, 0.0, 0.0), DVec3::new(4.0, 1.0, 1.0));
        let res = union(c1, c2).unwrap();
        assert_all_convex(&res);
        let vol = total_signed_volume(&res);
        assert!((vol - 2.0).abs() < 1e-6);
    }

    #[test]
    fn test_cuboid_union_touching_face() {
        let c1 = make_cuboid(DVec3::ZERO, DVec3::ONE);
        let c2 = make_cuboid(DVec3::new(1.0, 0.0, 0.0), DVec3::new(2.0, 1.0, 1.0));
        let res = union(c1, c2).unwrap();
        assert_all_convex(&res);
        let vol = total_signed_volume(&res);
        assert!((vol - 2.0).abs() < 1e-6);

        // Verify shared face at x=1 has been eliminated
        for p in &res {
            let centroid = p.vertices.iter().copied().sum::<DVec3>() / (p.vertices.len() as f64);
            if (centroid.x - 1.0).abs() < 1e-5 && p.normal().x.abs() > 0.99 {
                panic!("Found internal touching face at x=1 that should have been eliminated!");
            }
        }
    }

    #[test]
    fn test_cuboid_union_overlap() {
        let c1 = make_cuboid(DVec3::ZERO, DVec3::new(2.0, 1.0, 1.0));
        let c2 = make_cuboid(DVec3::new(1.0, 0.0, 0.0), DVec3::new(3.0, 1.0, 1.0));
        let res = union(c1, c2).unwrap();
        assert_all_convex(&res);
        let vol = total_signed_volume(&res);
        // Union covers [0, 3] x [0, 1] x [0, 1] => vol 3.0
        assert!((vol - 3.0).abs() < 1e-6);
    }

    #[test]
    fn test_cuboid_subtract_tunnel() {
        let block = make_cuboid(DVec3::ZERO, DVec3::splat(4.0));
        let tunnel = make_cuboid(DVec3::new(0.0, 1.0, 1.0), DVec3::new(4.0, 3.0, 3.0));
        let res = subtract(block, tunnel).unwrap();
        assert_all_convex(&res);
        let vol = total_signed_volume(&res);
        // Block (64) - Tunnel (16) = 48.0
        assert!((vol - 48.0).abs() < 1e-6);

        // Verify tunnel aperture is open: no caps across y in [1,3], z in [1,3] at x=0 or x=4
        for p in &res {
            let n = p.normal();
            if (n.x.abs() - 1.0).abs() < 1e-4 {
                let centroid =
                    p.vertices.iter().copied().sum::<DVec3>() / (p.vertices.len() as f64);
                if centroid.y > 1.01 && centroid.y < 2.99 && centroid.z > 1.01 && centroid.z < 2.99
                {
                    panic!(
                        "Found unexpected cap covering tunnel hole! Centroid: {:?}",
                        centroid
                    );
                }
            }
        }
    }

    #[test]
    fn test_cuboid_subtract_protruding_tunnel() {
        let block = make_cuboid(DVec3::ZERO, DVec3::splat(4.0));
        let tunnel = make_cuboid(DVec3::new(-1.0, 1.0, 1.0), DVec3::new(5.0, 3.0, 3.0));
        let res = subtract(block, tunnel).unwrap();
        assert_all_convex(&res);
        let vol = total_signed_volume(&res);
        assert!((vol - 48.0).abs() < 1e-6);
    }

    #[test]
    fn test_cuboid_subtract_blind_pocket() {
        let block = make_cuboid(DVec3::ZERO, DVec3::splat(4.0));
        let pocket = make_cuboid(DVec3::new(0.0, 1.0, 1.0), DVec3::new(2.0, 3.0, 3.0));
        let res = subtract(block, pocket).unwrap();
        assert_all_convex(&res);
        let vol = total_signed_volume(&res);
        // 64 - 8 = 56.0
        assert!((vol - 56.0).abs() < 1e-6);
    }

    #[test]
    fn test_cuboid_subtract_disjoint() {
        let c1 = make_cuboid(DVec3::ZERO, DVec3::ONE);
        let c2 = make_cuboid(DVec3::new(10.0, 0.0, 0.0), DVec3::new(11.0, 1.0, 1.0));
        let res = subtract(c1.clone(), c2).unwrap();
        assert_all_convex(&res);
        let vol = total_signed_volume(&res);
        assert!((vol - 1.0).abs() < 1e-6);
    }

    #[test]
    fn test_cuboid_subtract_completely_inside() {
        let c1 = make_cuboid(DVec3::ZERO, DVec3::ONE);
        let large = make_cuboid(DVec3::splat(-1.0), DVec3::splat(2.0));
        let res = subtract(c1, large).unwrap();
        assert!(res.is_empty());
        assert_eq!(total_signed_volume(&res), 0.0);
    }

    #[test]
    fn test_l_shape_triangulation() {
        let l_shape = Polygon::new(vec![
            DVec3::new(0.0, 0.0, 0.0),
            DVec3::new(2.0, 0.0, 0.0),
            DVec3::new(2.0, 1.0, 0.0),
            DVec3::new(1.0, 1.0, 0.0),
            DVec3::new(1.0, 2.0, 0.0),
            DVec3::new(0.0, 2.0, 0.0),
        ])
        .unwrap();
        assert!(!l_shape.is_convex());
        let tris = triangulate_polygon(&l_shape);
        assert_all_convex(&tris);
        let area_sum: f64 = tris
            .iter()
            .map(|t| newell_normal(&t.vertices).length() * 0.5)
            .sum();
        assert!((area_sum - 3.0).abs() < 1e-6);
    }

    #[test]
    fn test_malformed_inputs_do_not_panic() {
        let empty_res = union(Vec::new(), Vec::new()).unwrap();
        assert!(empty_res.is_empty());

        let invalid_poly = Polygon {
            vertices: vec![DVec3::new(f64::NAN, 0.0, 0.0)],
        };
        let res = union(vec![invalid_poly.clone()], vec![invalid_poly]).unwrap();
        assert!(res.is_empty());
    }
}
