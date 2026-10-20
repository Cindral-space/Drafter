use glam::{DVec2, DVec3};

use crate::KernelError;

/// Single geometric tolerance, in meters (also used for normalized predicates).
pub const TOLERANCE: f64 = 1.0e-7;
/// Supported absolute world coordinate in meters; computations use f64.
pub const MAX_COORDINATE: f64 = 1.0e6;

/// Canonically oriented plane, `normal.dot(position) == distance`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Plane {
    /// Unit normal. Its largest-magnitude component is positive.
    pub normal: DVec3,
    /// Signed distance from the world origin in meters.
    pub distance: f64,
}

impl Plane {
    pub(crate) fn through(origin: DVec3, a: DVec3, b: DVec3) -> Option<Self> {
        let length = a.length().min(b.length());
        let a = a.try_normalize()?;
        let b = b.try_normalize()?;
        let cross = a.cross(b);
        if cross.length() * length <= TOLERANCE {
            return None;
        }
        let mut normal = cross.normalize();
        let abs = normal.abs();
        let index = if abs.x >= abs.y && abs.x >= abs.z {
            0
        } else if abs.y >= abs.z {
            1
        } else {
            2
        };
        if normal[index] < 0.0 {
            normal = -normal;
        }
        Some(Self {
            normal,
            distance: normal.dot(origin),
        })
    }

    /// Signed perpendicular distance of a point from this plane.
    pub fn signed_distance(&self, point: DVec3) -> f64 {
        self.normal.dot(point) - self.distance
    }

    pub(crate) fn equivalent(&self, other: &Self) -> bool {
        self.normal.cross(other.normal).length() <= TOLERANCE * TOLERANCE
            && if self.normal.dot(other.normal) > 0.0 {
                (self.distance - other.distance).abs() <= TOLERANCE
            } else {
                (self.distance + other.distance).abs() <= TOLERANCE
            }
    }

    pub(crate) fn basis(&self) -> (DVec3, DVec3) {
        let axis = if self.normal.x.abs() < 0.9 {
            DVec3::X
        } else {
            DVec3::Y
        };
        let u = self.normal.cross(axis).normalize();
        (u, self.normal.cross(u))
    }
}

pub(crate) fn validate_position(point: DVec3) -> Result<(), KernelError> {
    if !point.is_finite() || point.abs().max_element() > MAX_COORDINATE {
        Err(KernelError::InvalidPosition)
    } else {
        Ok(())
    }
}

pub(crate) fn parameter(point: DVec3, start: DVec3, end: DVec3) -> f64 {
    let delta = end - start;
    (point - start).dot(delta) / delta.length_squared()
}

pub(crate) fn on_segment(point: DVec3, start: DVec3, end: DVec3) -> bool {
    let length = start.distance(end);
    if length <= TOLERANCE {
        return point.distance(start) <= TOLERANCE;
    }
    let t = parameter(point, start, end);
    t >= -TOLERANCE / length
        && t <= 1.0 + TOLERANCE / length
        && point.distance(start.lerp(end, t.clamp(0.0, 1.0))) <= TOLERANCE
}

/// Common intersection points, including overlap endpoints for collinear edges.
pub(crate) fn intersections(a: DVec3, b: DVec3, c: DVec3, d: DVec3) -> Vec<DVec3> {
    let u = b - a;
    let v = d - c;
    let lu = u.length();
    let lv = v.length();
    let cross = u.cross(v);
    if cross.length() <= TOLERANCE * TOLERANCE * lu * lv {
        if (c - a).cross(u / lu).length() > TOLERANCE {
            return Vec::new();
        }
        return [a, b, c, d]
            .into_iter()
            .filter(|p| on_segment(*p, a, b) && on_segment(*p, c, d))
            .collect();
    }
    let offset = c - a;
    let t = offset.cross(v).dot(cross) / cross.length_squared();
    let s = offset.cross(u).dot(cross) / cross.length_squared();
    if t < -TOLERANCE / lu
        || t > 1.0 + TOLERANCE / lu
        || s < -TOLERANCE / lv
        || s > 1.0 + TOLERANCE / lv
    {
        return Vec::new();
    }
    let p = a + u * t.clamp(0.0, 1.0);
    let q = c + v * s.clamp(0.0, 1.0);
    if p.distance(q) <= TOLERANCE {
        vec![(p + q) * 0.5]
    } else {
        Vec::new()
    }
}

pub(crate) fn signed_area(points: &[DVec2]) -> f64 {
    let Some(origin) = points.first() else {
        return 0.0;
    };
    points
        .iter()
        .zip(points.iter().cycle().skip(1))
        .map(|(a, b)| (*a - *origin).perp_dot(*b - *origin))
        .sum::<f64>()
        * 0.5
}

pub(crate) fn contains(points: &[DVec2], point: DVec2) -> bool {
    let mut inside = false;
    for (a, b) in points.iter().zip(points.iter().cycle().skip(1)) {
        if (a.y > point.y) != (b.y > point.y)
            && point.x < (b.x - a.x) * (point.y - a.y) / (b.y - a.y) + a.x
        {
            inside = !inside;
        }
    }
    inside
}
