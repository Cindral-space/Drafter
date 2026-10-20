//! GPU-independent inference inputs and screen-space geometric constraints.

mod engine;
pub use engine::Engine;

use cindral_core::{EdgeId, FaceId, VertexId, TOLERANCE};
pub use glam::{DMat4, DVec2, DVec3};

/// World axis used by explicit and inferred direction constraints.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Axis {
    /// Red X axis.
    X,
    /// Green Y axis.
    Y,
    /// Blue Z axis.
    Z,
}
impl Axis {
    /// Unit world direction.
    pub fn direction(self) -> DVec3 {
        match self {
            Self::X => DVec3::X,
            Self::Y => DVec3::Y,
            Self::Z => DVec3::Z,
        }
    }
}
/// World-space picking ray; direction need not be normalized.
#[derive(Clone, Copy, Debug)]
pub struct Ray {
    /// Start of the ray.
    pub origin: DVec3,
    /// Forward direction.
    pub direction: DVec3,
}
/// Work plane for planar drawing.
#[derive(Clone, Copy, Debug)]
pub struct DrawingPlane {
    /// A point in the plane.
    pub origin: DVec3,
    /// Plane normal, not necessarily normalized.
    pub normal: DVec3,
}
impl Default for DrawingPlane {
    fn default() -> Self {
        Self {
            origin: DVec3::ZERO,
            normal: DVec3::Z,
        }
    }
}
impl DrawingPlane {
    /// Forward ray intersection; invalid, parallel and behind-ray hits return None.
    pub fn intersect(&self, ray: Ray) -> Option<DVec3> {
        if !self.origin.is_finite()
            || !self.normal.is_finite()
            || !ray.origin.is_finite()
            || !ray.direction.is_finite()
        {
            return None;
        }
        let n = self.normal.try_normalize()?;
        let direction = ray.direction.try_normalize()?;
        let denominator = n.dot(direction);
        if denominator.abs() <= TOLERANCE {
            return None;
        }
        let t = (self.origin - ray.origin).dot(n) / denominator;
        let p = ray.origin + direction * t;
        (t >= 0.0 && p.is_finite()).then_some(p)
    }
}
/// Projection supplied by the caller; no renderer or camera dependency.
#[derive(Clone, Copy, Debug)]
pub struct View {
    /// Wgpu convention view-projection matrix, depth 0..1.
    pub matrix: DMat4,
    /// Viewport dimensions in logical pixels.
    pub size: DVec2,
}
impl View {
    /// Project a visible world point to local viewport pixels, Y down.
    pub fn project(&self, point: DVec3) -> Option<DVec2> {
        if !point.is_finite()
            || !self.matrix.is_finite()
            || !self.size.is_finite()
            || self.size.min_element() <= 0.0
        {
            return None;
        }
        let p = self.matrix * point.extend(1.0);
        if !p.is_finite() || p.w <= 0.0 {
            return None;
        }
        let ndc = p.truncate() / p.w;
        if !(0.0..=1.0).contains(&ndc.z) {
            return None;
        }
        Some(DVec2::new(
            (ndc.x + 1.0) * 0.5 * self.size.x,
            (1.0 - ndc.y) * 0.5 * self.size.y,
        ))
    }
}
/// Tangent guide at a known point of an in-progress circular arc.
#[derive(Clone, Copy, Debug)]
pub struct TangentHint {
    /// Circle center.
    pub center: DVec3,
    /// Arc plane normal.
    pub normal: DVec3,
    /// Point at which to offer the tangent.
    pub reference: DVec3,
}
/// All pure inputs for one inference request.
#[derive(Clone, Debug)]
pub struct Query {
    /// Local cursor position in logical pixels.
    pub cursor: DVec2,
    /// World-space picking ray.
    pub ray: Ray,
    /// Caller-supplied projection.
    pub view: View,
    /// Fallback drawing plane.
    pub plane: DrawingPlane,
    /// Active drawing anchor for directional inference.
    pub anchor: Option<DVec3>,
    /// Last committed line direction.
    pub previous_direction: Option<DVec3>,
    /// Hard world-axis constraint.
    pub axis_lock: Option<Axis>,
    /// Optional tangent guide.
    pub tangent: Option<TangentHint>,
}
impl Query {
    /// Construct an unconstrained query on the XY work plane.
    pub fn new(cursor: DVec2, ray: Ray, view: View) -> Self {
        Self {
            cursor,
            ray,
            view,
            plane: DrawingPlane::default(),
            anchor: None,
            previous_direction: None,
            axis_lock: None,
            tangent: None,
        }
    }
}
/// Inference classification shown by the UI.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SnapKind {
    /// Existing vertex.
    Endpoint,
    /// Edge midpoint.
    Midpoint,
    /// Point on a segment.
    OnEdge,
    /// Point inside a planar face, excluding holes.
    OnFace,
    /// Vertex with three or more incident edges.
    Intersection,
    /// World-axis guide.
    Axis(Axis),
    /// Parallel to the prior drawing direction.
    Parallel,
    /// Perpendicular to the prior direction within the work plane.
    Perpendicular,
    /// Tangent guide for the in-progress arc.
    Tangent,
    /// Work-plane fallback.
    Plane,
}
/// Optional entity supplying an inference candidate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SnapEntity {
    /// Model vertex.
    Vertex(VertexId),
    /// Model edge.
    Edge(EdgeId),
    /// Model face.
    Face(FaceId),
}
/// Winning candidate and optional visualization guide.
#[derive(Clone, Debug)]
pub struct Snap {
    /// Inferred world position.
    pub position: DVec3,
    /// Candidate classification.
    pub kind: SnapKind,
    /// Logical-pixel distance from the cursor.
    pub screen_distance: f64,
    /// Optional world-space guide segment.
    pub guide: Option<[DVec3; 2]>,
    /// Optional supplying model entity.
    pub entity: Option<SnapEntity>,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn projection_and_plane_inputs_are_pure_and_guarded() {
        let view = View {
            matrix: DMat4::IDENTITY,
            size: DVec2::splat(100.0),
        };
        assert_eq!(view.project(DVec3::ZERO), Some(DVec2::splat(50.0)));
        assert!(view.project(DVec3::new(0., 0., -1.)).is_none());
        let plane = DrawingPlane::default();
        assert_eq!(
            plane.intersect(Ray {
                origin: DVec3::Z,
                direction: -DVec3::Z
            }),
            Some(DVec3::ZERO)
        );
        assert!(plane
            .intersect(Ray {
                origin: DVec3::Z,
                direction: DVec3::X
            })
            .is_none());
    }
}
