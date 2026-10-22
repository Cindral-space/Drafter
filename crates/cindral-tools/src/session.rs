//! State machine session for interactive drawing tools.

use cindral_core::{Command, DVec3, MAX_COORDINATE, TOLERANCE};
use cindral_inference::{DrawingPlane, TangentHint};

use crate::measurements::{self, Measurement, MeasurementSettings, MeasurementTool, Quantity};
use crate::ToolError;

/// Supported interactive drawing tools.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ToolKind {
    /// Draw free 3D connected line segments.
    Line,
    /// Draw planar rectangles via first and opposite corners.
    Rectangle,
    /// Draw planar circles via center and radius point.
    Circle,
    /// Draw planar circular arcs via center, start, and end direction.
    Arc,
    /// Draw planar regular polygons via center and first vertex.
    Polygon,
}

impl ToolKind {
    /// Default segment count for this tool kind.
    pub fn default_segments(self) -> usize {
        match self {
            Self::Circle => 48,
            Self::Polygon => 6,
            Self::Arc => 24,
            Self::Line | Self::Rectangle => 48,
        }
    }
}

/// Validate that a position is finite and within the supported coordinate range.
fn validate_point(point: DVec3) -> Result<(), ToolError> {
    if !point.is_finite() || point.abs().max_element() > MAX_COORDINATE {
        Err(ToolError::InvalidPoint)
    } else {
        Ok(())
    }
}

/// Compute a deterministic right-handed orthonormal basis `(u, v)` for a plane normal `n`.
///
/// For the XY plane (normal +Z), this returns `(+X, +Y)`.
pub(crate) fn plane_basis_for(normal: DVec3) -> (DVec3, DVec3) {
    let n = normal.try_normalize().unwrap_or(DVec3::Z);
    let x_proj = DVec3::X - n * n.dot(DVec3::X);
    let u = if x_proj.length_squared() >= 0.01 {
        x_proj.normalize()
    } else {
        (DVec3::Y - n * n.dot(DVec3::Y)).normalize()
    };
    let v = n.cross(u);
    (u, v)
}

/// Interactive session managing tool states and command emission without model mutation.
#[derive(Debug, Clone)]
pub struct ToolSession {
    kind: ToolKind,
    completed: Option<Box<ToolSession>>,
    plane: DrawingPlane,
    anchors: Vec<DVec3>,
    current_pointer: Option<DVec3>,
    previous_direction: Option<DVec3>,
    segments: usize,
    typed_sweep: Option<f64>,
    typed_direction: Option<DVec3>,
}

impl ToolSession {
    /// Construct a new drawing session for the specified tool kind.
    pub fn new(kind: ToolKind) -> Self {
        Self {
            kind,
            completed: None,
            plane: DrawingPlane::default(),
            anchors: Vec::new(),
            current_pointer: None,
            previous_direction: None,
            segments: kind.default_segments(),
            typed_sweep: None,
            typed_direction: None,
        }
    }

    /// The currently active tool kind.
    pub fn kind(&self) -> ToolKind {
        self.kind
    }

    /// Change the tool kind, resetting any uncommitted interaction.
    pub fn set_kind(&mut self, kind: ToolKind) {
        self.completed = None;
        self.typed_sweep = None;
        self.typed_direction = None;
        self.kind = kind;
        self.anchors.clear();
        self.current_pointer = None;
        self.previous_direction = None;
        self.segments = kind.default_segments();
    }

    /// Cancel the current uncommitted interaction, resetting to idle.
    pub fn cancel(&mut self) {
        self.completed = None;
        self.typed_sweep = None;
        self.typed_direction = None;
        self.anchors.clear();
        self.current_pointer = None;
        self.previous_direction = None;
    }

    /// Set the drawing plane for subsequent operations.
    ///
    /// The plane normal must be finite and non-zero.
    pub fn set_plane(&mut self, plane: DrawingPlane) -> Result<(), ToolError> {
        validate_point(plane.origin)?;
        if !plane.normal.is_finite() {
            return Err(ToolError::InvalidPoint);
        }
        if plane.normal.try_normalize().is_none()
            || plane.normal.length_squared() <= TOLERANCE * TOLERANCE
        {
            return Err(ToolError::Degenerate);
        }
        self.plane = plane;
        Ok(())
    }

    /// The currently active drawing plane.
    pub fn plane(&self) -> DrawingPlane {
        self.plane
    }

    /// The active anchor point, if any.
    pub fn anchor(&self) -> Option<DVec3> {
        self.anchors.last().copied()
    }

    /// All anchor points for the in-progress operation.
    pub fn anchors(&self) -> &[DVec3] {
        &self.anchors
    }

    /// The direction of the most recently committed line segment, if any.
    pub fn previous_direction(&self) -> Option<DVec3> {
        self.previous_direction
    }

    /// Tangent guide hint exposed when Arc center and start points are established.
    pub fn tangent_hint(&self) -> Option<TangentHint> {
        if self.kind == ToolKind::Arc && self.anchors.len() == 2 {
            let center = self.anchors[0];
            let start = self.anchors[1];
            let normal = self.unit_normal();
            Some(TangentHint {
                center,
                normal,
                reference: start,
            })
        } else {
            None
        }
    }

    /// Set the tessellation segment count for curve tools (range `3..=256`).
    pub fn set_segments(&mut self, segments: usize) -> Result<(), ToolError> {
        if !(3..=256).contains(&segments) {
            return Err(ToolError::InvalidSegments);
        }
        self.segments = segments;
        Ok(())
    }

    /// The current segment count.
    pub fn segments(&self) -> usize {
        self.segments
    }

    /// Update the prospective pointer position.
    pub fn pointer_move(&mut self, point: DVec3) -> Result<(), ToolError> {
        validate_point(point)?;
        if self.completed.is_none() {
            self.typed_sweep = None;
        }
        self.current_pointer = Some(point);
        Ok(())
    }

    /// Process a click at the specified world coordinate.
    pub fn click(&mut self, point: DVec3) -> Result<Option<Command>, ToolError> {
        validate_point(point)?;
        self.completed = None;
        self.typed_sweep = None;
        self.typed_direction = None;
        let mut source = self.clone();
        source.current_pointer = Some(point);
        let result = match self.kind {
            ToolKind::Line => self.click_line(point),
            ToolKind::Rectangle => self.click_rectangle(point),
            ToolKind::Circle => self.click_circle(point),
            ToolKind::Arc => self.click_arc(point),
            ToolKind::Polygon => self.click_polygon(point),
        };
        if matches!(result, Ok(Some(_))) {
            self.completed = Some(Box::new(source));
        }
        result
    }

    /// Whether the latest completed operation remains editable until another click/cancel.
    pub fn editing_completed(&self) -> bool {
        self.completed.is_some()
    }

    /// End remeasurement eligibility when a new placement begins; preserve Line continuation.
    pub fn end_remeasurement(&mut self) {
        self.completed = None;
        self.typed_direction = None;
        self.typed_sweep = None;
    }

    /// Parse with default meter/dot preferences for standalone clients.
    pub fn measure(&mut self, text: &str) -> Result<Option<Command>, ToolError> {
        self.measure_with_settings(text, MeasurementSettings::default())
    }

    /// Parse once in the shared parser, then apply the structured input atomically.
    pub fn measure_with_settings(
        &mut self,
        text: &str,
        settings: MeasurementSettings,
    ) -> Result<Option<Command>, ToolError> {
        let input = measurements::parse(text, settings)?;
        self.apply_input(&input, settings)
    }

    /// Return the prospective polyline in world coordinates.
    ///
    /// For closed shapes, the first point is repeated at the end.
    pub fn preview(&self) -> Vec<DVec3> {
        if self.anchors.is_empty() {
            return Vec::new();
        }
        let Some(pointer) = self.current_pointer else {
            return Vec::new();
        };
        match self.kind {
            ToolKind::Line => self.preview_line(pointer),
            ToolKind::Rectangle => self.preview_rectangle(pointer),
            ToolKind::Circle => self.preview_circle(pointer),
            ToolKind::Arc => self.preview_arc(pointer),
            ToolKind::Polygon => self.preview_polygon(pointer),
        }
    }

    /// Numerical status readout without formatting or allocation.
    pub fn measurement_readout(&self) -> crate::measurements::Readout {
        if let Some(source) = &self.completed {
            return source.measurement_readout();
        }
        use crate::measurements::{Readout, ValueKind};
        let label = match self.kind {
            ToolKind::Line => "Length",
            ToolKind::Rectangle => "Dimensions",
            _ => "Radius",
        };
        let mut out = Readout {
            label,
            values: [0.; 3],
            count: 0,
            kind: ValueKind::Length,
        };
        if let (Some(start), Some(pointer)) = (self.anchors.first(), self.current_pointer) {
            let delta = pointer - *start;
            out.count = 1;
            if self.kind == ToolKind::Arc && self.anchors.len() == 2 {
                let r = self.anchors[1] - *start;
                let u = r.normalize_or_zero();
                let v = self.unit_normal().cross(u);
                let mut angle = delta.dot(v).atan2(delta.dot(u)).to_degrees();
                if angle <= 0. {
                    angle += 360.;
                }
                out.values[0] = self.typed_sweep.unwrap_or(angle);
                out.label = "Angle";
                out.kind = ValueKind::Angle;
            } else if self.kind == ToolKind::Rectangle {
                let (u, v) = self.plane_basis();
                out.values[0] = delta.dot(u);
                out.values[1] = delta.dot(v);
                out.count = 2;
            } else {
                out.values[0] = delta.length();
            }
        }
        out
    }

    // --- Internal Helpers ---

    fn unit_normal(&self) -> DVec3 {
        self.plane.normal.try_normalize().unwrap_or(DVec3::Z)
    }

    fn plane_basis(&self) -> (DVec3, DVec3) {
        plane_basis_for(self.plane.normal)
    }

    fn project_to_plane(&self, point: DVec3) -> DVec3 {
        let n = self.unit_normal();
        let diff = point - self.plane.origin;
        point - n * diff.dot(n)
    }

    // --- Line Tool ---

    fn click_line(&mut self, point: DVec3) -> Result<Option<Command>, ToolError> {
        if self.anchors.is_empty() {
            self.plane.origin = point;
            self.anchors.push(point);
            self.current_pointer = None;
            Ok(None)
        } else {
            let start = self.anchors[0];
            let end = point;
            if start.distance(end) <= TOLERANCE {
                return Err(ToolError::Degenerate);
            }
            let dir = (end - start).normalize();
            self.previous_direction = Some(dir);
            self.plane.origin = end;
            self.anchors[0] = end;
            self.current_pointer = None;
            Ok(Some(Command::AddEdge { start, end }))
        }
    }

    fn measure_line(&mut self, len: f64) -> Result<Option<Command>, ToolError> {
        if self.anchors.is_empty() {
            return Err(ToolError::NotReady);
        }
        if len.abs() <= TOLERANCE {
            return Err(ToolError::InvalidMeasurement(
                "Invalid or degenerate dimension".into(),
            ));
        }
        let start = self.anchors[0];
        let dir = if let Some(dir) = self.typed_direction {
            dir
        } else if let Some(p) = self.current_pointer {
            let diff = p - start;
            if diff.length() > TOLERANCE {
                diff.normalize()
            } else if let Some(prev) = self.previous_direction {
                prev
            } else {
                self.plane_basis().0
            }
        } else if let Some(prev) = self.previous_direction {
            prev
        } else {
            self.plane_basis().0
        };
        self.typed_direction = Some(dir);
        let end = start + dir * len;
        validate_point(end)?;
        self.previous_direction = Some(dir);
        self.plane.origin = end;
        self.anchors[0] = end;
        self.current_pointer = None;
        Ok(Some(Command::AddEdge { start, end }))
    }

    fn preview_line(&self, pointer: DVec3) -> Vec<DVec3> {
        vec![self.anchors[0], pointer]
    }

    // --- Rectangle Tool ---

    fn click_rectangle(&mut self, point: DVec3) -> Result<Option<Command>, ToolError> {
        if self.anchors.is_empty() {
            self.plane.origin = point;
            self.anchors.push(point);
            self.current_pointer = None;
            Ok(None)
        } else {
            let p0 = self.anchors[0];
            let (u, v) = self.plane_basis();
            let delta = self.project_to_plane(point) - p0;
            let w = delta.dot(u);
            let h = delta.dot(v);
            if w.abs() <= TOLERANCE || h.abs() <= TOLERANCE {
                return Err(ToolError::Degenerate);
            }
            let c0 = p0;
            let c1 = p0 + w * u;
            let c2 = p0 + w * u + h * v;
            let c3 = p0 + h * v;
            validate_point(c1)?;
            validate_point(c2)?;
            validate_point(c3)?;
            self.anchors.clear();
            self.current_pointer = None;
            Ok(Some(Command::AddPolyline {
                points: vec![c0, c1, c2, c3],
                closed: true,
            }))
        }
    }

    fn measure_rectangle(&mut self, w: f64, h: f64) -> Result<Option<Command>, ToolError> {
        if self.anchors.is_empty() {
            return Err(ToolError::NotReady);
        }
        if w.abs() <= TOLERANCE || h.abs() <= TOLERANCE {
            return Err(ToolError::InvalidMeasurement(
                "Invalid or degenerate dimension".into(),
            ));
        }
        let p0 = self.anchors[0];
        let (u, v) = self.plane_basis();
        let c0 = p0;
        let c1 = p0 + w * u;
        let c2 = p0 + w * u + h * v;
        let c3 = p0 + h * v;
        validate_point(c1)?;
        validate_point(c2)?;
        validate_point(c3)?;
        self.anchors.clear();
        self.current_pointer = None;
        Ok(Some(Command::AddPolyline {
            points: vec![c0, c1, c2, c3],
            closed: true,
        }))
    }

    fn preview_rectangle(&self, pointer: DVec3) -> Vec<DVec3> {
        let p0 = self.anchors[0];
        let (u, v) = self.plane_basis();
        let delta = self.project_to_plane(pointer) - p0;
        let w = delta.dot(u);
        let h = delta.dot(v);
        let c0 = p0;
        let c1 = p0 + w * u;
        let c2 = p0 + w * u + h * v;
        let c3 = p0 + h * v;
        vec![c0, c1, c2, c3, c0]
    }

    // --- Circle Tool ---

    fn click_circle(&mut self, point: DVec3) -> Result<Option<Command>, ToolError> {
        if self.anchors.is_empty() {
            self.plane.origin = point;
            self.anchors.push(point);
            self.current_pointer = None;
            Ok(None)
        } else {
            let center = self.anchors[0];
            let delta = self.project_to_plane(point) - center;
            let r = delta.length();
            if r <= TOLERANCE {
                return Err(ToolError::Degenerate);
            }
            let u_circ = delta / r;
            let v_circ = self.unit_normal().cross(u_circ);
            let n = self.segments;
            let mut points = Vec::with_capacity(n);
            for i in 0..n {
                let angle = 2.0 * std::f64::consts::PI * (i as f64) / (n as f64);
                let pt = center + r * (angle.cos() * u_circ + angle.sin() * v_circ);
                validate_point(pt)?;
                points.push(pt);
            }
            validate_path(&points, true)?;
            self.anchors.clear();
            self.current_pointer = None;
            Ok(Some(Command::AddPolyline {
                points,
                closed: true,
            }))
        }
    }

    fn measure_circle(&mut self, r: f64) -> Result<Option<Command>, ToolError> {
        if self.anchors.is_empty() {
            return Err(ToolError::NotReady);
        }
        if r <= TOLERANCE {
            return Err(ToolError::InvalidMeasurement(
                "Invalid or degenerate dimension".into(),
            ));
        }
        let center = self.anchors[0];
        let u_circ = if let Some(p) = self.current_pointer {
            let delta = self.project_to_plane(p) - center;
            if delta.length() > TOLERANCE {
                delta.normalize()
            } else {
                self.plane_basis().0
            }
        } else {
            self.plane_basis().0
        };
        let v_circ = self.unit_normal().cross(u_circ);
        let n = self.segments;
        let mut points = Vec::with_capacity(n);
        for i in 0..n {
            let angle = 2.0 * std::f64::consts::PI * (i as f64) / (n as f64);
            let pt = center + r * (angle.cos() * u_circ + angle.sin() * v_circ);
            validate_point(pt)?;
            points.push(pt);
        }
        validate_path(&points, true)?;
        self.anchors.clear();
        self.current_pointer = None;
        Ok(Some(Command::AddPolyline {
            points,
            closed: true,
        }))
    }

    fn preview_circle(&self, pointer: DVec3) -> Vec<DVec3> {
        let center = self.anchors[0];
        let delta = self.project_to_plane(pointer) - center;
        let r = delta.length();
        let u_circ = if r > TOLERANCE {
            delta / r
        } else {
            self.plane_basis().0
        };
        let v_circ = self.unit_normal().cross(u_circ);
        let n = self.segments;
        let mut points = Vec::with_capacity(n + 1);
        for i in 0..n {
            let angle = 2.0 * std::f64::consts::PI * (i as f64) / (n as f64);
            points.push(center + r * (angle.cos() * u_circ + angle.sin() * v_circ));
        }
        if let Some(&first) = points.first() {
            points.push(first);
        }
        points
    }

    // --- Polygon Tool ---

    fn click_polygon(&mut self, point: DVec3) -> Result<Option<Command>, ToolError> {
        if self.anchors.is_empty() {
            self.plane.origin = point;
            self.anchors.push(point);
            self.current_pointer = None;
            Ok(None)
        } else {
            let center = self.anchors[0];
            let delta = self.project_to_plane(point) - center;
            let r = delta.length();
            if r <= TOLERANCE {
                return Err(ToolError::Degenerate);
            }
            let u_poly = delta / r;
            let v_poly = self.unit_normal().cross(u_poly);
            let n = self.segments;
            let mut points = Vec::with_capacity(n);
            for i in 0..n {
                let angle = 2.0 * std::f64::consts::PI * (i as f64) / (n as f64);
                let pt = center + r * (angle.cos() * u_poly + angle.sin() * v_poly);
                validate_point(pt)?;
                points.push(pt);
            }
            validate_path(&points, true)?;
            self.anchors.clear();
            self.current_pointer = None;
            Ok(Some(Command::AddPolyline {
                points,
                closed: true,
            }))
        }
    }

    fn measure_polygon(&mut self, r: f64) -> Result<Option<Command>, ToolError> {
        if self.anchors.is_empty() {
            return Err(ToolError::NotReady);
        }
        if r <= TOLERANCE {
            return Err(ToolError::InvalidMeasurement(
                "Invalid or degenerate dimension".into(),
            ));
        }
        let center = self.anchors[0];
        let u_poly = if let Some(p) = self.current_pointer {
            let delta = self.project_to_plane(p) - center;
            if delta.length() > TOLERANCE {
                delta.normalize()
            } else {
                self.plane_basis().0
            }
        } else {
            self.plane_basis().0
        };
        let v_poly = self.unit_normal().cross(u_poly);
        let n = self.segments;
        let mut points = Vec::with_capacity(n);
        for i in 0..n {
            let angle = 2.0 * std::f64::consts::PI * (i as f64) / (n as f64);
            let pt = center + r * (angle.cos() * u_poly + angle.sin() * v_poly);
            validate_point(pt)?;
            points.push(pt);
        }
        validate_path(&points, true)?;
        self.anchors.clear();
        self.current_pointer = None;
        Ok(Some(Command::AddPolyline {
            points,
            closed: true,
        }))
    }

    fn preview_polygon(&self, pointer: DVec3) -> Vec<DVec3> {
        let center = self.anchors[0];
        let delta = self.project_to_plane(pointer) - center;
        let r = delta.length();
        let u_poly = if r > TOLERANCE {
            delta / r
        } else {
            self.plane_basis().0
        };
        let v_poly = self.unit_normal().cross(u_poly);
        let n = self.segments;
        let mut points = Vec::with_capacity(n + 1);
        for i in 0..n {
            let angle = 2.0 * std::f64::consts::PI * (i as f64) / (n as f64);
            points.push(center + r * (angle.cos() * u_poly + angle.sin() * v_poly));
        }
        if let Some(&first) = points.first() {
            points.push(first);
        }
        points
    }

    // --- Arc Tool ---

    fn click_arc(&mut self, point: DVec3) -> Result<Option<Command>, ToolError> {
        if self.anchors.is_empty() {
            // Click 1: center
            self.plane.origin = point;
            self.anchors.push(point);
            self.current_pointer = None;
            Ok(None)
        } else if self.anchors.len() == 1 {
            // Click 2: start defines radius
            let center = self.anchors[0];
            let delta = self.project_to_plane(point) - center;
            let r = delta.length();
            if r <= TOLERANCE {
                return Err(ToolError::Degenerate);
            }
            let projected = self.project_to_plane(point);
            validate_point(projected)?;
            self.anchors.push(projected);
            self.current_pointer = None;
            Ok(None)
        } else {
            // Click 3: end direction projected onto arc radius in the plane
            let center = self.anchors[0];
            let start = self.anchors[1];
            let r_vec = self.project_to_plane(start) - center;
            let r = r_vec.length();
            if r <= TOLERANCE {
                return Err(ToolError::Degenerate);
            }
            let u_arc = r_vec / r;
            let v_arc = self.unit_normal().cross(u_arc);

            let d_vec = self.project_to_plane(point) - center;
            if d_vec.length() <= TOLERANCE {
                return Err(ToolError::Degenerate);
            }
            let x = d_vec.dot(u_arc);
            let y = d_vec.dot(v_arc);
            let mut theta = f64::atan2(y, x);
            if theta <= 0.0 {
                theta += 2.0 * std::f64::consts::PI;
            }
            if theta <= TOLERANCE || theta >= 2.0 * std::f64::consts::PI - TOLERANCE {
                return Err(ToolError::Degenerate);
            }

            let n = self.segments;
            let mut points = Vec::with_capacity(n + 1);
            for i in 0..=n {
                let angle = theta * (i as f64) / (n as f64);
                let pt = center + r * (angle.cos() * u_arc + angle.sin() * v_arc);
                validate_point(pt)?;
                points.push(pt);
            }

            validate_path(&points, false)?;
            self.anchors.clear();
            self.current_pointer = None;
            Ok(Some(Command::AddPolyline {
                points,
                closed: false,
            }))
        }
    }

    fn measure_arc(&mut self, deg: f64) -> Result<Option<Command>, ToolError> {
        if self.anchors.len() < 2 {
            return Err(ToolError::NotReady);
        }
        let deg_abs = deg.abs();
        if deg_abs <= TOLERANCE || deg_abs >= 360.0 - TOLERANCE {
            return Err(ToolError::InvalidMeasurement(
                "Invalid or degenerate dimension".into(),
            ));
        }
        let theta = deg.to_radians();

        let center = self.anchors[0];
        let start = self.anchors[1];
        let r_vec = self.project_to_plane(start) - center;
        let r = r_vec.length();
        if r <= TOLERANCE {
            return Err(ToolError::Degenerate);
        }
        let u_arc = r_vec / r;
        let v_arc = self.unit_normal().cross(u_arc);

        let n = self.segments;
        let mut points = Vec::with_capacity(n + 1);
        for i in 0..=n {
            let angle = theta * (i as f64) / (n as f64);
            let pt = center + r * (angle.cos() * u_arc + angle.sin() * v_arc);
            validate_point(pt)?;
            points.push(pt);
        }

        validate_path(&points, false)?;
        self.anchors.clear();
        self.current_pointer = None;
        Ok(Some(Command::AddPolyline {
            points,
            closed: false,
        }))
    }

    fn preview_arc(&self, pointer: DVec3) -> Vec<DVec3> {
        if self.anchors.len() < 2 {
            return Vec::new();
        }
        let center = self.anchors[0];
        let start = self.anchors[1];
        let r_vec = self.project_to_plane(start) - center;
        let r = r_vec.length();
        if r <= TOLERANCE {
            return Vec::new();
        }
        let u_arc = r_vec / r;
        let v_arc = self.unit_normal().cross(u_arc);

        let d_vec = self.project_to_plane(pointer) - center;
        if d_vec.length() <= TOLERANCE {
            return Vec::new();
        }
        let x = d_vec.dot(u_arc);
        let y = d_vec.dot(v_arc);
        let mut theta = f64::atan2(y, x);
        if theta <= 0.0 {
            theta += 2.0 * std::f64::consts::PI;
        }
        if theta <= TOLERANCE || theta >= 2.0 * std::f64::consts::PI - TOLERANCE {
            return Vec::new();
        }

        let n = self.segments;
        let mut points = Vec::with_capacity(n + 1);
        for i in 0..=n {
            let angle = theta * (i as f64) / (n as f64);
            points.push(center + r * (angle.cos() * u_arc + angle.sin() * v_arc));
        }
        points
    }
}

fn validate_path(points: &[DVec3], closed: bool) -> Result<(), ToolError> {
    for point in points {
        validate_point(*point)?;
    }
    if points
        .windows(2)
        .any(|pair| pair[0].distance(pair[1]) <= TOLERANCE)
        || (closed
            && points
                .first()
                .zip(points.last())
                .is_some_and(|(a, b)| a.distance(*b) <= TOLERANCE))
    {
        return Err(ToolError::Degenerate);
    }
    Ok(())
}

impl MeasurementTool for ToolSession {
    fn readout(&self) -> crate::measurements::Readout {
        self.measurement_readout()
    }
    fn apply_input(
        &mut self,
        input: &Measurement,
        settings: MeasurementSettings,
    ) -> Result<Option<Command>, ToolError> {
        // Work on a bounded interaction snapshot; unsupported syntax never changes the session.
        let mut source = self.completed.as_deref().unwrap_or(self).clone();
        source.completed = None;
        let mut candidate = source.clone();
        let bad =
            || ToolError::InvalidMeasurement("Input is not supported by this drawing tool".into());
        let result = match input {
            Measurement::Sides(n)
                if matches!(
                    source.kind,
                    ToolKind::Circle | ToolKind::Polygon | ToolKind::Arc
                ) =>
            {
                candidate.set_segments(*n)?;
                source.segments = *n;
                if self.completed.is_some() {
                    let p = source.current_pointer.ok_or(ToolError::NotReady)?;
                    if source.kind == ToolKind::Arc {
                        candidate.measure_arc(source.measurement_readout().values[0])?
                    } else {
                        candidate.click(p)?
                    }
                } else {
                    None
                }
            }
            Measurement::Coordinates { relative, values }
                if source.kind == ToolKind::Line && values.len() == 3 =>
            {
                let a = source.anchors.first().copied().ok_or(ToolError::NotReady)?;
                let p = DVec3::new(
                    values[0].length(settings)?,
                    values[1].length(settings)?,
                    values[2].length(settings)?,
                );
                candidate.click_line(if *relative { a + p } else { p })?
            }
            Measurement::Values(values) => {
                let first = *values.first().ok_or_else(bad)?;
                match source.kind {
                    ToolKind::Rectangle if values.len() <= 2 => {
                        let h = if values.len() == 2 {
                            values[1].length(settings)?
                        } else {
                            let a = source.anchors.first().copied().ok_or(ToolError::NotReady)?;
                            let p = source.current_pointer.ok_or(ToolError::NotReady)?;
                            (p - a).dot(source.plane_basis().1)
                        };
                        candidate.measure_rectangle(first.length(settings)?, h)?
                    }
                    ToolKind::Line if values.len() == 1 => {
                        candidate.measure_line(first.length(settings)?)?
                    }
                    ToolKind::Circle | ToolKind::Polygon if values.len() == 1 => {
                        if source.anchors.is_empty() {
                            let n = first.number()?;
                            if n.fract() != 0. || !(3. ..=256.).contains(&n) {
                                return Err(bad());
                            }
                            candidate.set_segments(n as usize)?;
                            None
                        } else if source.kind == ToolKind::Circle {
                            candidate.measure_circle(first.length(settings)?)?
                        } else {
                            candidate.measure_polygon(first.length(settings)?)?
                        }
                    }
                    ToolKind::Arc if values.len() == 1 => {
                        if source.anchors.is_empty() {
                            return Err(ToolError::NotReady);
                        }
                        if source.anchors.len() == 1 || matches!(first, Quantity::Length(_)) {
                            if source.anchors.len() == 1 && matches!(first, Quantity::Degrees(_)) {
                                return Err(ToolError::NotReady);
                            }
                            let radius = first.length(settings)?;
                            if radius <= TOLERANCE {
                                return Err(bad());
                            }
                            let center = source.anchors[0];
                            let direction = source
                                .anchors
                                .get(1)
                                .copied()
                                .or(source.current_pointer)
                                .map(|p| (p - center).normalize_or_zero())
                                .filter(|d| d.length_squared() > 0.)
                                .unwrap_or(source.plane_basis().0);
                            let start = center + direction * radius;
                            validate_point(start)?;
                            if source.anchors.len() == 1 {
                                candidate.anchors.push(start);
                                None
                            } else {
                                candidate.anchors[1] = start;
                                let angle = source.measurement_readout().values[0];
                                candidate.measure_arc(angle)?
                            }
                        } else {
                            source.typed_sweep = Some(first.number()?);
                            candidate.measure_arc(first.number()?)?
                        }
                    }
                    _ => return Err(bad()),
                }
            }
            _ => return Err(bad()),
        };
        if let Some(command) = &result {
            match command {
                Command::AddEdge { end, .. } => {
                    source.current_pointer = Some(*end);
                    source.typed_direction = candidate.typed_direction;
                }
                Command::AddPolyline { points, .. } => {
                    source.current_pointer = points
                        .get(if source.kind == ToolKind::Rectangle {
                            2
                        } else if source.kind == ToolKind::Arc {
                            points.len() - 1
                        } else {
                            0
                        })
                        .copied();
                    if source.kind == ToolKind::Arc {
                        source.anchors[1] = points[0];
                    }
                }
                _ => {}
            }
            candidate.completed = Some(Box::new(source));
        }
        *self = candidate;
        Ok(result)
    }
}
