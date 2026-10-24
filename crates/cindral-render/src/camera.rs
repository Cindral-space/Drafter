//! Viewport camera math for the M1 frozen camera interface.
//!
//! The camera orbits a `target` point at `distance`, oriented by `yaw` and
//! `pitch` (radians). All math uses glam `f32`; it is independent of any
//! windowing or UI toolkit.
//!
//! # Conventions
//!
//! * World axes: Z is up; X is red, Y is green, Z is blue. Right-handed.
//! * The eye sits at `target + distance * (cos(pitch)·cos(yaw), cos(pitch)·sin(yaw), sin(pitch))`.
//!   Yaw `0` puts the eye on +X, yaw `π/2` on +Y; positive pitch puts the eye
//!   above the target.
//! * The camera basis is derived analytically from yaw and pitch rather than
//!   from a world-up vector, so it is orthonormal at every pitch including
//!   straight down (Top) and straight up (Bottom). Screen-right is always the
//!   horizontal vector `(-sin(yaw), cos(yaw), 0)`.
//! * Pitch is clamped to `[-PITCH_LIMIT, PITCH_LIMIT]`, distance to
//!   `[MIN_DISTANCE, MAX_DISTANCE]` and yaw is wrapped to `[-π, π]`.
//! * [`Camera::view_projection`] produces clip space for wgpu: right-handed
//!   view space, NDC X right / Y up, depth `0` (near) to `1` (far).
//!   Perspective uses an infinite far plane with the near plane at
//!   `distance / 100`. Orthographic spans a depth slab of `±1000·distance`
//!   around the eye.
//! * Both projections share the same visible half-height at the target depth,
//!   `distance · tan(FOV_Y / 2)`, so toggling projection keeps the target plane
//!   framed identically and `distance` acts as the zoom level in either mode.
//! * Orbit and pan deltas are logical pixels with screen Y pointing down.
//!   Scroll is in points; positive zooms in. Cursor NDC has Y up.
//! * Every method ignores nonfinite input. Mutating methods leave the camera
//!   untouched when given invalid input; otherwise they first sanitize the
//!   public fields (nonfinite values are replaced by defaults, distance and
//!   pitch are clamped). Read-only methods operate on a sanitized copy, so they
//!   always return finite values.

use std::f32::consts::{FRAC_PI_2, FRAC_PI_4, PI, TAU};

use glam::{Mat4, Vec2, Vec3, Vec4};

/// Vertical field of view of the perspective projection, in radians (45°).
pub const FOV_Y: f32 = FRAC_PI_4;

/// Smallest permitted camera distance from the target.
pub const MIN_DISTANCE: f32 = 1.0e-3;

/// Largest permitted camera distance from the target.
pub const MAX_DISTANCE: f32 = 1.0e6;

/// Largest target coordinate supported before a floating origin is required.
/// Out-of-range public targets are reset to the origin when sanitized.
pub const MAX_TARGET_COORDINATE: f32 = 1.0e9;

/// Largest permitted absolute pitch, in radians.
///
/// Straight down/up is allowed because the camera basis is computed
/// analytically and never degenerates.
pub const PITCH_LIMIT: f32 = FRAC_PI_2;

/// Distance used by [`Camera::default`] and when a nonfinite distance is
/// sanitized.
pub const DEFAULT_DISTANCE: f32 = 10.0;

/// Orbit rotation per logical pixel of drag, in radians.
const ORBIT_RADIANS_PER_PIXEL: f32 = 0.005;

/// Exponential zoom rate per scroll point (one 50-point wheel notch ≈ 12%).
const ZOOM_RATE_PER_POINT: f32 = 0.0025;

/// Clamp on the zoom exponent so a single huge scroll cannot overflow.
const MAX_ZOOM_EXPONENT: f32 = 30.0;

/// Perspective near plane as a fraction of `distance`.
const NEAR_RATIO: f32 = 0.01;

/// Orthographic half depth of the clip slab as a multiple of `distance`.
const ORTHO_DEPTH_RATIO: f32 = 1000.0;

/// Smallest orthographic zoom-extents distance as a fraction of the bounds
/// radius, keeping the whole box inside the orthographic depth slab.
const ORTHO_MIN_FIT_RATIO: f32 = 2.0 / ORTHO_DEPTH_RATIO;

/// Fraction of the half-viewport that zoom extents fills.
const EXTENTS_FILL: f32 = 0.9;

/// Accepted aspect ratio range; finite positive aspects are clamped to it.
const MIN_ASPECT: f32 = 1.0e-3;
const MAX_ASPECT: f32 = 1.0e3;

/// Projection model of the viewport.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Projection {
    /// Perspective projection with vertical field of view [`FOV_Y`].
    #[default]
    Perspective,
    /// Parallel projection whose visible half-height is
    /// `distance · tan(FOV_Y / 2)`.
    Orthographic,
}

/// Standard view orientations. Selecting a view keeps the target and
/// distance and only changes yaw and pitch.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StandardView {
    /// True isometric view from the front-right-top (eye toward +X, −Y, +Z).
    Iso,
    /// Looking down −Z; X points right and Y points up on screen.
    Top,
    /// Looking up +Z; X points right and −Y points up on screen.
    Bottom,
    /// Eye on −Y looking toward +Y; X right, Z up.
    Front,
    /// Eye on +Y looking toward −Y; −X right, Z up.
    Back,
    /// Eye on −X looking toward +X; −Y right, Z up.
    Left,
    /// Eye on +X looking toward −X; Y right, Z up.
    Right,
}

impl StandardView {
    /// Returns `(yaw, pitch)` for the view.
    fn angles(self) -> (f32, f32) {
        match self {
            StandardView::Iso => (-FRAC_PI_4, iso_pitch()),
            StandardView::Top => (-FRAC_PI_2, PITCH_LIMIT),
            StandardView::Bottom => (-FRAC_PI_2, -PITCH_LIMIT),
            StandardView::Front => (-FRAC_PI_2, 0.0),
            StandardView::Back => (FRAC_PI_2, 0.0),
            StandardView::Left => (PI, 0.0),
            StandardView::Right => (0.0, 0.0),
        }
    }
}

/// Elevation of the true isometric view: `atan(1/√2)` ≈ 35.26°.
fn iso_pitch() -> f32 {
    std::f32::consts::FRAC_1_SQRT_2.atan()
}

/// Orbit camera for the drafting viewport.
#[derive(Clone, Debug, PartialEq)]
pub struct Camera {
    /// Orbit pivot and centre of the view.
    pub target: Vec3,
    /// Distance from the eye to the target; also the zoom level.
    pub distance: f32,
    /// Azimuth of the eye around the target, in radians, measured from +X
    /// toward +Y.
    pub yaw: f32,
    /// Elevation of the eye above the target's horizontal plane, in radians.
    pub pitch: f32,
    /// Projection model.
    pub projection: Projection,
}

impl Default for Camera {
    /// Perspective isometric view of the origin at [`DEFAULT_DISTANCE`].
    fn default() -> Self {
        let (yaw, pitch) = StandardView::Iso.angles();
        Self {
            target: Vec3::ZERO,
            distance: DEFAULT_DISTANCE,
            yaw,
            pitch,
            projection: Projection::Perspective,
        }
    }
}

/// Orthonormal camera frame.
#[derive(Clone, Copy, Debug)]
struct Basis {
    /// Unit vector from the target toward the eye (view-space +Z).
    back: Vec3,
    /// Screen right (view-space +X); always horizontal.
    right: Vec3,
    /// Screen up (view-space +Y).
    up: Vec3,
}

impl Basis {
    fn new(yaw: f32, pitch: f32) -> Self {
        let (sy, cy) = yaw.sin_cos();
        let (sp, cp) = pitch.sin_cos();
        // `FRAC_PI_2` rounds slightly above π/2 in f32, which would make the
        // cosine a tiny negative number; never flip past the pole.
        let cp = cp.max(0.0);
        Self {
            back: Vec3::new(cp * cy, cp * sy, sp),
            right: Vec3::new(-sy, cy, 0.0),
            // back × right, expanded analytically.
            up: Vec3::new(-sp * cy, -sp * sy, cp),
        }
    }

    fn forward(&self) -> Vec3 {
        -self.back
    }
}

/// Visible half-height of the view at the target depth.
fn half_height(distance: f32) -> f32 {
    distance * (0.5 * FOV_Y).tan()
}

fn clamp_distance(distance: f32) -> f32 {
    distance.clamp(MIN_DISTANCE, MAX_DISTANCE)
}

fn clamp_pitch(pitch: f32) -> f32 {
    pitch.clamp(-PITCH_LIMIT, PITCH_LIMIT)
}

/// Wraps an angle into `[-π, π]`.
///
/// Angles already in range are returned unchanged so repeated sanitizing is
/// exact and never drifts.
fn wrap_angle(angle: f32) -> f32 {
    if (-PI..=PI).contains(&angle) {
        angle
    } else {
        (angle + PI).rem_euclid(TAU) - PI
    }
}

/// Validates and clamps an aspect ratio, rejecting nonfinite or non-positive
/// values.
fn sanitize_aspect(aspect: f32) -> Option<f32> {
    (aspect.is_finite() && aspect > 0.0).then(|| aspect.clamp(MIN_ASPECT, MAX_ASPECT))
}

impl Camera {
    /// Returns a copy with every public field forced into its valid range.
    fn sanitized(&self) -> Self {
        let defaults = Self::default();
        Self {
            target: if self.target.is_finite()
                && self.target.abs().max_element() <= MAX_TARGET_COORDINATE
            {
                self.target
            } else {
                defaults.target
            },
            distance: if self.distance.is_finite() {
                clamp_distance(self.distance)
            } else {
                defaults.distance
            },
            yaw: if self.yaw.is_finite() {
                wrap_angle(self.yaw)
            } else {
                defaults.yaw
            },
            pitch: if self.pitch.is_finite() {
                clamp_pitch(self.pitch)
            } else {
                defaults.pitch
            },
            projection: self.projection,
        }
    }

    fn basis(&self) -> Basis {
        Basis::new(self.yaw, self.pitch)
    }

    /// World-space eye position.
    pub fn eye(&self) -> Vec3 {
        let cam = self.sanitized();
        cam.target + cam.basis().back * cam.distance
    }

    /// Combined projection × view matrix for wgpu clip space (depth `0..1`).
    ///
    /// A nonfinite or non-positive `aspect` (width / height) falls back to
    /// `1.0`.
    pub fn view_projection(&self, aspect: f32) -> Mat4 {
        let cam = self.sanitized();
        let aspect = sanitize_aspect(aspect).unwrap_or(1.0);
        cam.projection_matrix(aspect) * cam.view_matrix()
    }

    /// Orbits the eye around the target by a drag of `delta` logical pixels.
    ///
    /// The scene follows the pointer: dragging right turns the model to the
    /// right (yaw decreases) and dragging down tilts its top toward the viewer
    /// (pitch increases). Pitch is clamped at straight down/up.
    pub fn orbit(&mut self, delta: Vec2) {
        if !delta.is_finite() {
            return;
        }
        let mut cam = self.sanitized();
        let yaw = wrap_angle(cam.yaw - delta.x * ORBIT_RADIANS_PER_PIXEL);
        let pitch = clamp_pitch(cam.pitch + delta.y * ORBIT_RADIANS_PER_PIXEL);
        if !(yaw.is_finite() && pitch.is_finite()) {
            return;
        }
        cam.yaw = yaw;
        cam.pitch = pitch;
        *self = cam;
    }

    /// Pans by a drag of `delta` logical pixels in a viewport that is
    /// `viewport_height` logical pixels tall.
    ///
    /// Points on the target-depth view plane stay under the pointer.
    pub fn pan(&mut self, delta: Vec2, viewport_height: f32) {
        self.pan_at_depth(delta, viewport_height, self.pan_depth(None));
    }

    /// View-axis depth of a picked world point; missing/behind/invalid hits use
    /// the target plane. Clamp to the visible near plane and supported range.
    pub fn pan_depth(&self, point: Option<Vec3>) -> f32 {
        let cam = self.sanitized();
        let depth = point.map(|p| (p - cam.eye()).dot(cam.basis().forward()));
        depth
            .filter(|d| d.is_finite() && *d > 0.0)
            .unwrap_or(cam.distance)
            .clamp((cam.distance * NEAR_RATIO).max(MIN_DISTANCE), MAX_DISTANCE)
    }

    /// Pan at a fixed press-time view-axis depth. Orthographic scale depends
    /// only on view size. This allocates nothing and changes only translation.
    pub fn pan_at_depth(&mut self, delta: Vec2, viewport_height: f32, depth: f32) {
        if !depth.is_finite()
            || !delta.is_finite()
            || !viewport_height.is_finite()
            || viewport_height <= 0.0
        {
            return;
        }
        let mut cam = self.sanitized();
        let basis = cam.basis();
        let scale = match cam.projection {
            Projection::Perspective => depth.clamp(MIN_DISTANCE, MAX_DISTANCE),
            Projection::Orthographic => cam.distance,
        };
        let units_per_pixel = 2.0 * half_height(scale) / viewport_height;
        let target = cam.target + (basis.up * delta.y - basis.right * delta.x) * units_per_pixel;
        if !target.is_finite() || target.abs().max_element() > MAX_TARGET_COORDINATE {
            return;
        }
        cam.target = target;
        *self = cam;
    }

    /// Zooms by `scroll` points toward the cursor; positive values zoom in.
    ///
    /// The world point where the cursor ray meets the target-depth view plane
    /// keeps its screen position. `cursor_ndc` (Y up) is clamped to the
    /// viewport `[-1, 1]²`. When the distance is already at a limit the camera
    /// does not drift.
    pub fn zoom(&mut self, scroll: f32, cursor_ndc: Vec2, aspect: f32) {
        if !scroll.is_finite() || !cursor_ndc.is_finite() {
            return;
        }
        let Some(aspect) = sanitize_aspect(aspect) else {
            return;
        };
        let mut cam = self.sanitized();
        let exponent = (-scroll * ZOOM_RATE_PER_POINT).clamp(-MAX_ZOOM_EXPONENT, MAX_ZOOM_EXPONENT);
        let distance = clamp_distance(cam.distance * exponent.exp());
        // Use the post-clamp ratio so the anchor holds even at the limits.
        let ratio = distance / cam.distance;

        let cursor = cursor_ndc.clamp(Vec2::NEG_ONE, Vec2::ONE);
        let basis = cam.basis();
        let half_h = half_height(cam.distance);
        let anchor_offset =
            basis.right * (cursor.x * half_h * aspect) + basis.up * (cursor.y * half_h);
        // Scaling the view about the anchor: the anchor stays on the
        // (parallel) target plane and its offset scales with the view size.
        let target = cam.target + anchor_offset * (1.0 - ratio);
        if !target.is_finite()
            || target.abs().max_element() > MAX_TARGET_COORDINATE
            || !distance.is_finite()
        {
            return;
        }
        cam.target = target;
        cam.distance = distance;
        *self = cam;
    }

    /// Frames the axis-aligned box `min..max` in the current orientation and
    /// projection.
    ///
    /// The box corners are fitted exactly into the central 90% of the
    /// viewport (per axis); the target is placed on the
    /// view plane through the box centre. Swapped corners are accepted. A
    /// zero-size box recentres on the point without changing the distance.
    /// Boxes too large for [`MAX_DISTANCE`] are framed as closely as allowed.
    pub fn zoom_extents(&mut self, min: Vec3, max: Vec3, aspect: f32) {
        if !min.is_finite() || !max.is_finite() {
            return;
        }
        let Some(aspect) = sanitize_aspect(aspect) else {
            return;
        };
        let mut cam = self.sanitized();
        let lo = min.min(max);
        let hi = min.max(max);
        // Halving first avoids overflow for boxes near f32::MAX.
        let center = lo * 0.5 + hi * 0.5;
        let half = hi * 0.5 - lo * 0.5;
        let radius = half.length();
        if !center.is_finite() || center.abs().max_element() > MAX_TARGET_COORDINATE {
            return;
        }
        let point_tolerance = 4.0 * f32::EPSILON * (1.0 + center.abs().max_element());
        if radius <= point_tolerance {
            cam.target = center;
            *self = cam;
            return;
        }

        let basis = cam.basis();
        let tan_half = (0.5 * FOV_Y).tan();
        let tx = tan_half * aspect * EXTENTS_FILL;
        let ty = tan_half * EXTENTS_FILL;
        // Corner coordinates relative to the centre in the camera frame.
        let corners = (0..8).map(|i| {
            let offset = Vec3::new(
                if i & 1 == 0 { -half.x } else { half.x },
                if i & 2 == 0 { -half.y } else { half.y },
                if i & 4 == 0 { -half.z } else { half.z },
            );
            Vec3::new(
                offset.dot(basis.right),
                offset.dot(basis.up),
                offset.dot(basis.back),
            )
        });

        let (shift_x, shift_y, distance) = match cam.projection {
            Projection::Perspective => {
                // With the eye at (ex, ey, ez) in the frame, a corner (x, y, z)
                // is visible when |x - ex| <= (ez - z)·tx, i.e.
                //   ex + ez·tx >= x + z·tx   and   -ex + ez·tx >= -x + z·tx.
                // The tightest eye satisfies both maxima with equality, and the
                // same for y. Taking the larger ez keeps the other axis valid
                // because its centred ex/ey sits mid-way in its feasible range.
                let mut sum_pos_x = f32::NEG_INFINITY;
                let mut sum_neg_x = f32::NEG_INFINITY;
                let mut sum_pos_y = f32::NEG_INFINITY;
                let mut sum_neg_y = f32::NEG_INFINITY;
                let mut z_max = f32::NEG_INFINITY;
                for c in corners {
                    sum_pos_x = sum_pos_x.max(c.x + c.z * tx);
                    sum_neg_x = sum_neg_x.max(-c.x + c.z * tx);
                    sum_pos_y = sum_pos_y.max(c.y + c.z * ty);
                    sum_neg_y = sum_neg_y.max(-c.y + c.z * ty);
                    z_max = z_max.max(c.z);
                }
                let ez_x = (sum_pos_x + sum_neg_x) / (2.0 * tx);
                let ez_y = (sum_pos_y + sum_neg_y) / (2.0 * ty);
                // Keep every corner at least twice the near distance in front
                // of the eye (relevant when looking along a thin box).
                let ez_near = z_max / (1.0 - 2.0 * NEAR_RATIO);
                (
                    0.5 * (sum_pos_x - sum_neg_x),
                    0.5 * (sum_pos_y - sum_neg_y),
                    ez_x.max(ez_y).max(ez_near),
                )
            }
            Projection::Orthographic => {
                let mut lo2 = Vec2::splat(f32::INFINITY);
                let mut hi2 = Vec2::splat(f32::NEG_INFINITY);
                for c in corners {
                    lo2 = lo2.min(c.truncate());
                    hi2 = hi2.max(c.truncate());
                }
                let mid = lo2 * 0.5 + hi2 * 0.5;
                let half2 = hi2 * 0.5 - lo2 * 0.5;
                let distance = (half2.x / tx)
                    .max(half2.y / ty)
                    .max(radius * ORTHO_MIN_FIT_RATIO);
                (mid.x, mid.y, distance)
            }
        };

        // Distance is measured to the plane through the box centre, so the
        // orbit pivot stays inside the model.
        let distance = clamp_distance(distance);
        let target = center + basis.right * shift_x + basis.up * shift_y;
        if !target.is_finite()
            || target.abs().max_element() > MAX_TARGET_COORDINATE
            || !distance.is_finite()
        {
            return;
        }
        cam.target = target;
        cam.distance = distance;
        *self = cam;
    }

    /// Switches to a standard view, keeping target, distance and projection.
    pub fn set_view(&mut self, view: StandardView) {
        let mut cam = self.sanitized();
        let (yaw, pitch) = view.angles();
        cam.yaw = yaw;
        cam.pitch = pitch;
        *self = cam;
    }

    /// World-space picking ray through `ndc` (Y up) for a viewport with the
    /// given `aspect`.
    ///
    /// Returns `(origin, direction)`: the origin lies on the near clip plane
    /// and the direction is unit length, pointing into the scene, so ray
    /// parameters increase with depth exactly as rendered. Nonfinite `ndc`
    /// yields the centre ray; an invalid `aspect` falls back to `1.0`.
    pub fn ray(&self, ndc: Vec2, aspect: f32) -> (Vec3, Vec3) {
        let cam = self.sanitized();
        let aspect = sanitize_aspect(aspect).unwrap_or(1.0);
        let ndc = if ndc.is_finite() { ndc } else { Vec2::ZERO };
        let ray = cam.ray_unchecked(ndc, aspect);
        if ray.0.is_finite() && ray.1.is_finite() {
            ray
        } else {
            cam.ray_unchecked(Vec2::ZERO, aspect)
        }
    }

    /// Ray computation on an already sanitized camera.
    fn ray_unchecked(&self, ndc: Vec2, aspect: f32) -> (Vec3, Vec3) {
        let basis = self.basis();
        let eye = self.target + basis.back * self.distance;
        let tan_half = (0.5 * FOV_Y).tan();
        match self.projection {
            Projection::Perspective => {
                // Unnormalised direction with unit forward component.
                let through = basis.forward()
                    + basis.right * (ndc.x * tan_half * aspect)
                    + basis.up * (ndc.y * tan_half);
                let origin = eye + through * (self.distance * NEAR_RATIO);
                (origin, through.normalize_or(basis.forward()))
            }
            Projection::Orthographic => {
                let half_h = self.distance * tan_half;
                let origin = eye
                    + basis.right * (ndc.x * half_h * aspect)
                    + basis.up * (ndc.y * half_h)
                    + basis.back * (self.distance * ORTHO_DEPTH_RATIO);
                (origin, basis.forward())
            }
        }
    }

    /// Right-handed view matrix (view-space −Z is forward).
    fn view_matrix(&self) -> Mat4 {
        let basis = self.basis();
        let eye = self.target + basis.back * self.distance;
        let (r, u, b) = (basis.right, basis.up, basis.back);
        Mat4::from_cols(
            Vec4::new(r.x, u.x, b.x, 0.0),
            Vec4::new(r.y, u.y, b.y, 0.0),
            Vec4::new(r.z, u.z, b.z, 0.0),
            Vec4::new(-eye.dot(r), -eye.dot(u), -eye.dot(b), 1.0),
        )
    }

    /// Projection matrix with wgpu depth `0..1`.
    fn projection_matrix(&self, aspect: f32) -> Mat4 {
        match self.projection {
            Projection::Perspective => {
                Mat4::perspective_infinite_rh(FOV_Y, aspect, self.distance * NEAR_RATIO)
            }
            Projection::Orthographic => {
                let half_h = half_height(self.distance);
                let half_w = half_h * aspect;
                let depth = self.distance * ORTHO_DEPTH_RATIO;
                Mat4::orthographic_rh(-half_w, half_w, -half_h, half_h, -depth, depth)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL_VIEWS: [StandardView; 7] = [
        StandardView::Iso,
        StandardView::Top,
        StandardView::Bottom,
        StandardView::Front,
        StandardView::Back,
        StandardView::Left,
        StandardView::Right,
    ];
    const PROJECTIONS: [Projection; 2] = [Projection::Perspective, Projection::Orthographic];

    /// Projects a world point to NDC (x, y, depth).
    fn project(cam: &Camera, aspect: f32, p: Vec3) -> Vec3 {
        let clip = cam.view_projection(aspect) * p.extend(1.0);
        assert!(clip.w > 0.0, "point behind camera: {p:?}");
        clip.truncate() / clip.w
    }

    fn assert_close(a: f32, b: f32, tol: f32, what: &str) {
        assert!((a - b).abs() <= tol, "{what}: {a} vs {b} (tol {tol})");
    }

    fn assert_vec_close(a: Vec3, b: Vec3, tol: f32, what: &str) {
        assert!((a - b).abs().max_element() <= tol, "{what}: {a:?} vs {b:?}");
    }

    fn camera(projection: Projection, view: StandardView) -> Camera {
        let mut cam = Camera {
            target: Vec3::new(1.5, -2.0, 0.75),
            distance: 12.0,
            projection,
            ..Camera::default()
        };
        cam.set_view(view);
        cam
    }

    /// World point on the target-depth view plane under `ndc`.
    ///
    /// Computed analytically so anchor tests are not limited by the
    /// precision of the far-away orthographic ray origin; `ray` consistency is
    /// covered separately by `ray_matches_projection`.
    fn target_plane_hit(cam: &Camera, ndc: Vec2, aspect: f32) -> Vec3 {
        let b = cam.basis();
        let h = half_height(cam.distance);
        cam.target + b.right * (ndc.x * h * aspect) + b.up * (ndc.y * h)
    }

    #[test]
    fn default_is_perspective_iso_at_origin() {
        let cam = Camera::default();
        assert_eq!(cam.target, Vec3::ZERO);
        assert_eq!(cam.distance, DEFAULT_DISTANCE);
        assert_eq!(cam.projection, Projection::Perspective);
        let eye_dir = cam.eye().normalize();
        // True isometric: eye direction is (1, -1, 1)/√3.
        assert_vec_close(
            eye_dir,
            Vec3::new(1.0, -1.0, 1.0).normalize(),
            1e-5,
            "iso eye",
        );
    }

    #[test]
    fn eye_follows_yaw_and_pitch() {
        let mut cam = Camera {
            target: Vec3::new(1.0, 2.0, 3.0),
            distance: 5.0,
            yaw: 0.0,
            pitch: 0.0,
            projection: Projection::Perspective,
        };
        assert_vec_close(cam.eye(), Vec3::new(6.0, 2.0, 3.0), 1e-5, "yaw 0");
        cam.yaw = FRAC_PI_2;
        assert_vec_close(cam.eye(), Vec3::new(1.0, 7.0, 3.0), 1e-5, "yaw 90");
        cam.pitch = FRAC_PI_2;
        assert_vec_close(cam.eye(), Vec3::new(1.0, 2.0, 8.0), 1e-5, "pitch 90");
    }

    #[test]
    fn projection_maps_target_plane_to_viewport_edges() {
        for projection in PROJECTIONS {
            for aspect in [0.5, 1.0, 2.0] {
                let cam = camera(projection, StandardView::Iso);
                let b = cam.basis();
                let h = half_height(cam.distance);
                let center = project(&cam, aspect, cam.target);
                assert_close(center.x, 0.0, 1e-5, "center x");
                assert_close(center.y, 0.0, 1e-5, "center y");
                assert!(center.z > 0.0 && center.z < 1.0, "depth {}", center.z);

                let top = project(&cam, aspect, cam.target + b.up * h);
                assert_close(top.y, 1.0, 1e-4, "top edge");
                assert_close(top.x, 0.0, 1e-4, "top edge x");
                let right = project(&cam, aspect, cam.target + b.right * (h * aspect));
                assert_close(right.x, 1.0, 1e-4, "right edge");
                assert_close(right.y, 0.0, 1e-4, "right edge y");
            }
        }
    }

    #[test]
    fn perspective_depth_range_and_foreshortening() {
        let cam = camera(Projection::Perspective, StandardView::Front);
        let f = cam.basis().forward();
        let near = project(&cam, 1.0, cam.eye() + f * (cam.distance * NEAR_RATIO));
        assert_close(near.z, 0.0, 1e-5, "near plane depth");
        let mid = project(&cam, 1.0, cam.target);
        let far = project(&cam, 1.0, cam.target + f * 1.0e4);
        assert!(near.z < mid.z && mid.z < far.z && far.z < 1.0);

        // An offset point appears smaller when farther away.
        let offset = cam.basis().right * 2.0;
        let close = project(&cam, 1.0, cam.target + offset - f * 5.0);
        let distant = project(&cam, 1.0, cam.target + offset + f * 5.0);
        assert!(close.x > distant.x && distant.x > 0.0);
    }

    #[test]
    fn orthographic_screen_position_ignores_depth() {
        let cam = camera(Projection::Orthographic, StandardView::Iso);
        let b = cam.basis();
        let h = half_height(cam.distance);
        let lateral = b.right * (0.5 * h * 1.5) + b.up * (-0.25 * h);
        let mut last_depth = -1.0;
        for depth in [-5.0e3, -10.0, 0.0, 10.0, 5.0e3] {
            let p = project(&cam, 1.5, cam.target + lateral + b.forward() * depth);
            assert_close(p.x, 0.5, 1e-4, "ortho x");
            assert_close(p.y, -0.25, 1e-4, "ortho y");
            assert!(
                p.z > last_depth && (0.0..=1.0).contains(&p.z),
                "depth {}",
                p.z
            );
            last_depth = p.z;
        }
    }

    #[test]
    fn standard_views_orient_axes() {
        // (view, screen direction of +X, +Y, +Z) as rounded NDC deltas.
        let cases = [
            (StandardView::Top, Vec2::X, Vec2::Y, Vec2::ZERO),
            (StandardView::Bottom, Vec2::X, -Vec2::Y, Vec2::ZERO),
            (StandardView::Front, Vec2::X, Vec2::ZERO, Vec2::Y),
            (StandardView::Back, -Vec2::X, Vec2::ZERO, Vec2::Y),
            (StandardView::Left, Vec2::ZERO, -Vec2::X, Vec2::Y),
            (StandardView::Right, Vec2::ZERO, Vec2::X, Vec2::Y),
        ];
        for (view, ex, ey, ez) in cases {
            let cam = camera(Projection::Orthographic, view);
            let o = project(&cam, 1.0, cam.target).truncate();
            for (axis, expected) in [(Vec3::X, ex), (Vec3::Y, ey), (Vec3::Z, ez)] {
                let d = project(&cam, 1.0, cam.target + axis).truncate() - o;
                let d = d * half_height(cam.distance);
                assert!(
                    (d - expected).abs().max_element() < 1e-4,
                    "{view:?} axis {axis:?}: {d:?} expected {expected:?}"
                );
            }
        }

        // Iso: X toward lower right, Y toward upper right, Z straight up.
        let cam = camera(Projection::Orthographic, StandardView::Iso);
        let o = project(&cam, 1.0, cam.target).truncate();
        let x = project(&cam, 1.0, cam.target + Vec3::X).truncate() - o;
        let y = project(&cam, 1.0, cam.target + Vec3::Y).truncate() - o;
        let z = project(&cam, 1.0, cam.target + Vec3::Z).truncate() - o;
        assert!(x.x > 0.0 && x.y < 0.0, "iso x {x:?}");
        assert!(y.x > 0.0 && y.y > 0.0, "iso y {y:?}");
        assert!(z.x.abs() < 1e-5 && z.y > 0.0, "iso z {z:?}");
    }

    #[test]
    fn top_and_bottom_are_nonsingular() {
        for projection in PROJECTIONS {
            for view in [StandardView::Top, StandardView::Bottom] {
                let mut cam = camera(projection, view);
                for _ in 0..4 {
                    let vp = cam.view_projection(1.3);
                    assert!(vp.is_finite(), "{view:?} matrix not finite");
                    assert!(vp.determinant().abs() > 1e-12, "{view:?} singular");
                    let p = project(&cam, 1.3, cam.target + Vec3::new(0.3, 0.2, 0.0));
                    assert!(p.is_finite());
                    let (o, d) = cam.ray(Vec2::new(0.4, -0.2), 1.3);
                    assert!(o.is_finite() && d.is_finite());
                    // Spinning about the pole must stay well-defined.
                    cam.orbit(Vec2::new(157.0, 0.0));
                }
                // Pushing past the pole clamps instead of flipping.
                cam.orbit(Vec2::new(
                    0.0,
                    if view == StandardView::Top {
                        500.0
                    } else {
                        -500.0
                    },
                ));
                assert_close(cam.pitch.abs(), PITCH_LIMIT, 1e-6, "pole clamp");
            }
        }
    }

    #[test]
    fn orbit_rotates_about_target() {
        let mut cam = camera(Projection::Perspective, StandardView::Front);
        let target = cam.target;
        let (yaw0, pitch0) = (cam.yaw, cam.pitch);

        cam.orbit(Vec2::new(100.0, 0.0));
        assert!(cam.yaw < yaw0, "drag right decreases yaw");
        assert_close(
            cam.yaw,
            yaw0 - 100.0 * ORBIT_RADIANS_PER_PIXEL,
            1e-5,
            "yaw step",
        );
        cam.orbit(Vec2::new(0.0, 40.0));
        assert!(cam.pitch > pitch0, "drag down raises the eye");
        assert_eq!(cam.target, target);
        assert_close(
            cam.eye().distance(target),
            cam.distance,
            1e-4,
            "orbit radius",
        );

        // Dragging right moves a point right of the target toward the viewer
        // side, i.e. the scene turns with the pointer.
        let mut cam = camera(Projection::Orthographic, StandardView::Front);
        let probe = cam.target + Vec3::new(0.0, -1.0, 0.0); // in front of target
        let before = project(&cam, 1.0, probe);
        cam.orbit(Vec2::new(50.0, 0.0));
        let after = project(&cam, 1.0, probe);
        assert!(after.x > before.x, "front point moves right with pointer");

        // Pitch clamps and yaw wraps under extreme drags.
        cam.orbit(Vec2::new(1.0e6, -1.0e6));
        assert_close(cam.pitch, -PITCH_LIMIT, 1e-6, "pitch clamp");
        assert!(cam.yaw.abs() <= PI + 1e-6, "yaw wrapped: {}", cam.yaw);
    }

    #[test]
    fn pan_keeps_target_plane_under_pointer() {
        for projection in PROJECTIONS {
            for view in [StandardView::Iso, StandardView::Top, StandardView::Left] {
                let aspect = 1.6;
                let height = 800.0;
                let mut cam = camera(projection, view);
                let before = cam.clone();
                let anchor = target_plane_hit(&cam, Vec2::new(0.25, -0.5), aspect);
                let delta = Vec2::new(40.0, -24.0);
                cam.pan(delta, height);

                assert_eq!(cam.distance, before.distance);
                assert_eq!(cam.yaw, before.yaw);
                assert_eq!(cam.pitch, before.pitch);
                // Old target moves by the pixel delta (screen Y down → NDC Y up).
                let p = project(&cam, aspect, before.target);
                assert_close(p.x, 2.0 * delta.x / (height * aspect), 1e-4, "pan x");
                assert_close(p.y, -2.0 * delta.y / height, 1e-4, "pan y");
                // Any target-plane point follows the pointer by the same amount.
                let q = project(&cam, aspect, anchor);
                assert_close(
                    q.x,
                    0.25 + 2.0 * delta.x / (height * aspect),
                    1e-4,
                    "anchor x",
                );
                assert_close(q.y, -0.5 - 2.0 * delta.y / height, 1e-4, "anchor y");
            }
        }
    }

    #[test]
    fn zoom_preserves_cursor_anchor() {
        for projection in PROJECTIONS {
            for view in [StandardView::Iso, StandardView::Top, StandardView::Back] {
                for aspect in [0.4, 1.0, 2.5] {
                    for cursor in [Vec2::ZERO, Vec2::new(0.7, -0.4), Vec2::new(-1.0, 1.0)] {
                        for scroll in [50.0, -50.0, 600.0, -600.0] {
                            let mut cam = camera(projection, view);
                            let anchor = target_plane_hit(&cam, cursor, aspect);
                            let before = project(&cam, aspect, anchor);
                            assert_close(before.x, cursor.x, 1e-4, "anchor setup x");
                            assert_close(before.y, cursor.y, 1e-4, "anchor setup y");
                            let distance = cam.distance;
                            cam.zoom(scroll, cursor, aspect);
                            if scroll > 0.0 {
                                assert!(cam.distance < distance, "positive scroll zooms in");
                            } else {
                                assert!(cam.distance > distance, "negative scroll zooms out");
                            }
                            let after = project(&cam, aspect, anchor);
                            let what = format!(
                                "{projection:?} {view:?} a={aspect} c={cursor:?} s={scroll}"
                            );
                            assert_close(after.x, cursor.x, 1e-4, &what);
                            assert_close(after.y, cursor.y, 1e-4, &what);
                            // Anchor remains on the new target-depth plane.
                            let normal = (cam.eye() - cam.target).normalize();
                            assert_close((anchor - cam.target).dot(normal), 0.0, 1e-3, &what);
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn zoom_clamps_distance_without_drifting() {
        let cursor = Vec2::new(0.6, 0.3);
        let mut cam = camera(Projection::Perspective, StandardView::Iso);
        cam.zoom(1.0e9, cursor, 1.0);
        assert_eq!(cam.distance, MIN_DISTANCE);
        let pinned = cam.clone();
        cam.zoom(500.0, cursor, 1.0);
        assert_eq!(cam, pinned, "no drift at the near limit");

        cam.zoom(-1.0e9, cursor, 1.0);
        assert_eq!(cam.distance, MAX_DISTANCE);
        assert!(cam.target.is_finite());
        let pinned = cam.clone();
        cam.zoom(-500.0, cursor, 1.0);
        assert_eq!(cam, pinned, "no drift at the far limit");

        // Zooming at the centre never moves the target.
        let mut cam = camera(Projection::Orthographic, StandardView::Top);
        let target = cam.target;
        cam.zoom(120.0, Vec2::ZERO, 1.7);
        assert_vec_close(cam.target, target, 1e-6, "centre zoom");

        // An off-viewport cursor is clamped to the viewport edge.
        let mut a = camera(Projection::Perspective, StandardView::Iso);
        let mut b = a.clone();
        a.zoom(100.0, Vec2::new(25.0, -3.0), 1.0);
        b.zoom(100.0, Vec2::new(1.0, -1.0), 1.0);
        assert_eq!(a, b);
    }

    /// Checks that every corner of the box is visible; returns NDC min/max.
    fn assert_box_visible(cam: &Camera, aspect: f32, min: Vec3, max: Vec3) -> (Vec2, Vec2) {
        let mut lo = Vec2::splat(f32::INFINITY);
        let mut hi = Vec2::splat(f32::NEG_INFINITY);
        for i in 0..8 {
            let corner = Vec3::new(
                if i & 1 == 0 { min.x } else { max.x },
                if i & 2 == 0 { min.y } else { max.y },
                if i & 4 == 0 { min.z } else { max.z },
            );
            let p = project(cam, aspect, corner);
            assert!(
                p.x.abs() <= 1.0 + 1e-4 && p.y.abs() <= 1.0 + 1e-4,
                "corner {corner:?} outside viewport: {p:?} ({cam:?}, aspect {aspect})"
            );
            assert!(
                (0.0..=1.0).contains(&p.z),
                "corner {corner:?} depth clipped: {} ({cam:?})",
                p.z
            );
            lo = lo.min(p.truncate());
            hi = hi.max(p.truncate());
        }
        (lo, hi)
    }

    #[test]
    fn zoom_extents_contains_box_at_narrow_and_wide_aspects() {
        let boxes = [
            (Vec3::new(-1.0, -1.0, -1.0), Vec3::new(1.0, 1.0, 1.0)),
            (Vec3::new(10.0, 20.0, 0.0), Vec3::new(14.0, 21.0, 3.0)),
            (Vec3::new(-50.0, -0.5, 0.0), Vec3::new(50.0, 0.5, 0.2)),
            (Vec3::new(0.0, 0.0, 0.0), Vec3::new(3.0, 2.0, 0.0)), // flat plate
            (
                Vec3::new(-200.0, 300.0, -5.0),
                Vec3::new(400.0, 900.0, 80.0),
            ),
        ];
        let mut orientations: Vec<(f32, f32)> = ALL_VIEWS.iter().map(|v| v.angles()).collect();
        orientations.extend([(0.3, 0.2), (2.1, -0.9), (-2.8, 1.2)]);
        for projection in PROJECTIONS {
            for aspect in [0.2, 0.75, 1.0, 2.4, 5.0] {
                for &(yaw, pitch) in &orientations {
                    for &(min, max) in &boxes {
                        let mut cam = Camera {
                            target: Vec3::new(-7.0, 3.0, 2.0),
                            distance: 0.5,
                            yaw,
                            pitch,
                            projection,
                        };
                        cam.zoom_extents(min, max, aspect);
                        assert_eq!(cam.yaw, yaw);
                        assert_eq!(cam.pitch, pitch);
                        let (lo, hi) = assert_box_visible(&cam, aspect, min, max);
                        // The fit is tight: the binding axis reaches the fill
                        // fraction (a looser fit happens only when the near
                        // plane constrains an end-on perspective view).
                        let reach = hi.max(-lo).max_element();
                        assert!(reach <= EXTENTS_FILL + 1e-3, "overfilled: {reach}");
                        if projection == Projection::Orthographic {
                            assert_close(reach, EXTENTS_FILL, 1e-3, "ortho fill");
                            assert_vec_close(
                                (lo + hi).extend(0.0),
                                Vec3::ZERO,
                                1e-3,
                                "ortho centred",
                            );
                        } else {
                            // Depth half-extent of the box along the view axis.
                            let back = cam.basis().back;
                            let depth_half = ((max - min) * 0.5 * back).abs().element_sum();
                            let near_bound = depth_half / (1.0 - 2.0 * NEAR_RATIO);
                            let near_binds =
                                (cam.distance - near_bound).abs() <= 1e-4 * cam.distance;
                            assert!(
                                near_binds || reach > EXTENTS_FILL - 1e-3,
                                "perspective fit too loose: {reach} {cam:?} {aspect} {min:?} {max:?}"
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn zoom_extents_perspective_is_tight_and_centred() {
        for aspect in [0.25, 1.0, 4.0] {
            let mut cam = camera(Projection::Perspective, StandardView::Iso);
            let (min, max) = (Vec3::new(-2.0, -1.0, 0.0), Vec3::new(3.0, 4.0, 2.5));
            cam.zoom_extents(min, max, aspect);
            let (lo, hi) = assert_box_visible(&cam, aspect, min, max);
            // On the binding axis both sides touch the fill boundary.
            let touches =
                |l: f32, h: f32| (h - EXTENTS_FILL).abs() < 1e-3 && (l + EXTENTS_FILL).abs() < 1e-3;
            assert!(
                touches(lo.x, hi.x) || touches(lo.y, hi.y),
                "aspect {aspect}: not tight {lo:?} {hi:?}"
            );
            // The pivot lies on the view plane through the box centre.
            let center = (min + max) * 0.5;
            let normal = (cam.eye() - cam.target).normalize();
            assert_close((center - cam.target).dot(normal), 0.0, 1e-3, "pivot depth");
        }
    }

    #[test]
    fn zoom_extents_handles_degenerate_bounds() {
        for projection in PROJECTIONS {
            // Single point: recentre, keep distance and orientation.
            let mut cam = camera(projection, StandardView::Iso);
            let before = cam.clone();
            let point = Vec3::new(4.0, -3.0, 2.0);
            cam.zoom_extents(point, point, 1.5);
            assert_eq!(cam.target, point);
            assert_eq!(cam.distance, before.distance);
            assert_eq!((cam.yaw, cam.pitch), (before.yaw, before.pitch));

            // Swapped corners behave like sorted ones.
            let mut a = camera(projection, StandardView::Iso);
            let mut b = a.clone();
            a.zoom_extents(Vec3::new(5.0, -1.0, 3.0), Vec3::new(-2.0, 4.0, 0.0), 1.2);
            b.zoom_extents(Vec3::new(-2.0, -1.0, 0.0), Vec3::new(5.0, 4.0, 3.0), 1.2);
            assert_eq!(a, b);

            // Flat plate seen edge-on, and a segment seen end-on.
            let cases = [
                (
                    StandardView::Front,
                    Vec3::new(-1.0, -1.0, 0.0),
                    Vec3::new(1.0, 1.0, 0.0),
                ),
                (
                    StandardView::Front,
                    Vec3::new(0.0, -10.0, 0.0),
                    Vec3::new(0.0, 10.0, 0.0),
                ),
                (
                    StandardView::Top,
                    Vec3::new(2.0, 2.0, -3.0),
                    Vec3::new(2.0, 2.0, 3.0),
                ),
            ];
            for (view, min, max) in cases {
                for aspect in [0.2, 5.0] {
                    let mut cam = camera(projection, view);
                    cam.zoom_extents(min, max, aspect);
                    assert!(cam.target.is_finite() && cam.distance.is_finite());
                    assert!((MIN_DISTANCE..=MAX_DISTANCE).contains(&cam.distance));
                    assert_box_visible(&cam, aspect, min, max);
                }
            }

            // Tiny boxes clamp to the minimum distance but stay framed.
            let mut cam = camera(projection, StandardView::Iso);
            let (min, max) = (Vec3::splat(1.0), Vec3::splat(1.0 + 1.0e-5));
            cam.zoom_extents(min, max, 1.0);
            assert_eq!(cam.distance, MIN_DISTANCE);
            assert_box_visible(&cam, 1.0, min, max);
        }
    }

    #[test]
    fn invalid_inputs_are_ignored() {
        let bad = [f32::NAN, f32::INFINITY, f32::NEG_INFINITY];
        for projection in PROJECTIONS {
            let original = camera(projection, StandardView::Iso);
            let check = |f: &dyn Fn(&mut Camera), what: &str| {
                let mut cam = original.clone();
                f(&mut cam);
                assert_eq!(cam, original, "{what} should be ignored");
            };
            for v in bad {
                check(&|c| c.orbit(Vec2::new(v, 1.0)), "orbit");
                check(&|c| c.orbit(Vec2::new(1.0, v)), "orbit");
                check(&|c| c.pan(Vec2::new(v, 1.0), 600.0), "pan delta");
                check(&|c| c.pan(Vec2::new(1.0, 1.0), v), "pan height");
                check(&|c| c.zoom(v, Vec2::ZERO, 1.0), "zoom scroll");
                check(&|c| c.zoom(10.0, Vec2::new(v, 0.0), 1.0), "zoom cursor");
                check(&|c| c.zoom(10.0, Vec2::ZERO, v), "zoom aspect");
                check(
                    &|c| c.zoom_extents(Vec3::new(v, 0.0, 0.0), Vec3::ONE, 1.0),
                    "extents min",
                );
                check(
                    &|c| c.zoom_extents(Vec3::ZERO, Vec3::new(1.0, v, 1.0), 1.0),
                    "extents max",
                );
                check(
                    &|c| c.zoom_extents(Vec3::ZERO, Vec3::ONE, v),
                    "extents aspect",
                );
            }
            for non_positive in [0.0, -1.0] {
                check(&|c| c.pan(Vec2::ONE, non_positive), "pan height <= 0");
                check(
                    &|c| c.zoom(10.0, Vec2::ZERO, non_positive),
                    "zoom aspect <= 0",
                );
                check(
                    &|c| c.zoom_extents(Vec3::ZERO, Vec3::ONE, non_positive),
                    "extents aspect <= 0",
                );
            }
            // Overflowing extents are rejected rather than producing NaN.
            check(
                &|c| c.zoom_extents(Vec3::splat(-f32::MAX), Vec3::splat(f32::MAX), 1.0),
                "overflowing extents",
            );

            let cam = original.clone();
            for v in bad.into_iter().chain([0.0, -2.0]) {
                assert_eq!(cam.view_projection(v), cam.view_projection(1.0));
                assert_eq!(
                    cam.ray(Vec2::new(0.2, 0.1), v),
                    cam.ray(Vec2::new(0.2, 0.1), 1.0)
                );
            }
            for v in bad {
                assert_eq!(cam.ray(Vec2::new(v, 0.3), 1.4), cam.ray(Vec2::ZERO, 1.4));
            }
        }
    }

    #[test]
    fn extreme_finite_targets_cannot_overflow_derived_transforms() {
        for projection in PROJECTIONS {
            let cam = Camera {
                target: Vec3::splat(f32::MAX),
                distance: MIN_DISTANCE,
                projection,
                ..Camera::default()
            };
            assert!(cam.eye().is_finite());
            assert!(cam.view_projection(0.001).is_finite());
            let (origin, direction) = cam.ray(Vec2::ONE, 1.0);
            assert!(origin.is_finite() && direction.is_finite());
        }
    }

    #[test]
    fn corrupted_fields_are_sanitized() {
        let corrupt = Camera {
            target: Vec3::new(f32::NAN, 1.0, 2.0),
            distance: f32::INFINITY,
            yaw: f32::NAN,
            pitch: f32::NEG_INFINITY,
            projection: Projection::Orthographic,
        };
        assert!(corrupt.eye().is_finite());
        assert!(corrupt.view_projection(1.0).is_finite());
        let (o, d) = corrupt.ray(Vec2::new(0.5, 0.5), 1.0);
        assert!(o.is_finite() && d.is_finite());

        let mut cam = corrupt.clone();
        cam.orbit(Vec2::ZERO);
        let defaults = Camera::default();
        assert_eq!(cam.target, defaults.target);
        assert_eq!(cam.distance, defaults.distance);
        assert_eq!((cam.yaw, cam.pitch), (defaults.yaw, defaults.pitch));
        assert_eq!(cam.projection, Projection::Orthographic);

        let mut cam = Camera {
            distance: -4.0,
            pitch: 3.0,
            yaw: 10.0 * TAU + 0.5,
            ..Camera::default()
        };
        cam.set_view(StandardView::Front);
        assert_eq!(cam.distance, MIN_DISTANCE);
        let mut cam2 = Camera {
            distance: 1.0e12,
            pitch: 3.0,
            yaw: 10.0 * TAU + 0.5,
            ..Camera::default()
        };
        cam2.orbit(Vec2::ZERO);
        assert_eq!(cam2.distance, MAX_DISTANCE);
        assert_eq!(cam2.pitch, PITCH_LIMIT);
        assert_close(cam2.yaw, 0.5, 1e-4, "yaw wrapped");
    }

    #[test]
    fn ray_matches_projection() {
        for projection in PROJECTIONS {
            for view in ALL_VIEWS {
                for aspect in [0.3, 1.0, 3.0] {
                    let cam = camera(projection, view);
                    for ndc in [Vec2::ZERO, Vec2::new(0.8, -0.6), Vec2::new(-1.0, 1.0)] {
                        let (origin, dir) = cam.ray(ndc, aspect);
                        assert_close(dir.length(), 1.0, 1e-5, "unit direction");
                        let at_origin = project(&cam, aspect, origin);
                        assert_close(at_origin.z, 0.0, 1e-4, "origin on near plane");
                        let mut last_depth = -1.0;
                        for s in [0.5, 5.0, 50.0, 500.0] {
                            let p = project(&cam, aspect, origin + dir * s);
                            let what = format!("{projection:?} {view:?} {ndc:?} s={s}");
                            assert_close(p.x, ndc.x, 1e-3, &what);
                            assert_close(p.y, ndc.y, 1e-3, &what);
                            assert!(p.z > last_depth, "{what}: depth must increase");
                            last_depth = p.z;
                        }
                    }
                    // The centre ray passes through the target.
                    let (origin, dir) = cam.ray(Vec2::ZERO, aspect);
                    let to_target = cam.target - origin;
                    let miss = (to_target - dir * to_target.dot(dir)).length();
                    assert!(miss < 1e-3, "centre ray misses target by {miss}");
                }
            }
        }
    }

    #[test]
    fn switching_projection_keeps_target_plane_framing() {
        let mut cam = camera(Projection::Perspective, StandardView::Iso);
        let b = cam.basis();
        let h = half_height(cam.distance);
        let p = cam.target + b.right * (0.3 * h * 2.0) + b.up * (-0.7 * h);
        let persp = project(&cam, 2.0, p);
        cam.projection = Projection::Orthographic;
        let ortho = project(&cam, 2.0, p);
        assert_vec_close(
            persp.truncate().extend(0.0),
            ortho.truncate().extend(0.0),
            1e-4,
            "framing",
        );
        assert_close(ortho.x, 0.3, 1e-4, "x");
        assert_close(ortho.y, -0.7, 1e-4, "y");
    }
}

#[cfg(test)]
#[path = "pan_tests.rs"]
mod pan_tests;
