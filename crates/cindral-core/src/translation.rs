//! Compact whole-definition transforms, shared by Move and Rotate previews.
use crate::{Command, DVec3, FaceId, KernelError, Model, Plane, VertexId};
use glam::DMat4;
#[derive(Clone, Debug)]
pub(crate) struct Translation {
    pub vertices: Vec<(VertexId, DVec3)>,
    pub planes: Vec<(FaceId, Plane)>,
    pub edges: Vec<crate::EdgeId>,
    pub matrix: DMat4,
    pub index_frame: DMat4,
}
impl Translation {
    fn transform(command: &Command) -> Option<(&[VertexId], DMat4)> {
        if let Command::Transform {
            vertices,
            matrix,
            copy: false,
        } = command
        {
            // Whole-definition rigid transforms preserve all distances and topology.
            // Nonuniform context conjugations retain the established validation path.
            let linear = glam::DMat3::from_mat4(*matrix);
            if matrix.is_finite()
                && (linear.transpose() * linear).abs_diff_eq(glam::DMat3::IDENTITY, 1e-10)
                && (matrix.determinant() - 1.).abs() < 1e-9
                && matrix.x_axis.w == 0.
                && matrix.y_axis.w == 0.
                && matrix.z_axis.w == 0.
                && matrix.w_axis.w == 1.
            {
                return Some((vertices, *matrix));
            }
        }
        None
    }
    pub fn prepare(model: &Model, command: &Command) -> Result<Option<Self>, KernelError> {
        let Some((vertices, matrix)) = Self::transform(command) else {
            return Ok(None);
        };
        if vertices.is_empty()
            || vertices.len() != model.vertices.len()
            || !vertices.iter().eq(model.vertices.keys())
        {
            return Ok(None);
        }
        for v in model.vertices.values() {
            crate::geometry::validate_position(matrix.transform_point3(v.position))?;
        }
        Ok(Some(Self {
            vertices: model
                .vertices
                .iter()
                .map(|(id, v)| (*id, v.position))
                .collect(),
            planes: model.faces.iter().map(|(id, f)| (*id, f.plane)).collect(),
            edges: model.edges.keys().copied().collect(),
            matrix,
            index_frame: model.geometry_frame(),
        }))
    }
    pub fn replace(
        &mut self,
        model: &mut Model,
        command: &Command,
    ) -> Result<Option<bool>, KernelError> {
        let Some((vertices, matrix)) = Self::transform(command) else {
            return Ok(None);
        };
        if vertices.len() != self.vertices.len()
            || !vertices
                .iter()
                .zip(&self.vertices)
                .all(|(id, (old, _))| id == old)
        {
            return Ok(None);
        }
        for (_, p) in &self.vertices {
            crate::geometry::validate_position(matrix.transform_point3(*p))?;
        }
        if matrix == self.matrix {
            return Ok(Some(false));
        }
        self.matrix = matrix;
        self.apply(model, true)?;
        Ok(Some(matrix != DMat4::IDENTITY))
    }
    pub fn apply(&self, model: &mut Model, forward: bool) -> Result<(), KernelError> {
        if model.vertices.len() != self.vertices.len() || model.faces.len() != self.planes.len() {
            return Err(KernelError::InvalidTopology(
                "transform topology changed".into(),
            ));
        }
        let matrix = if forward {
            self.matrix
        } else {
            DMat4::IDENTITY
        };
        let normals = matrix.inverse().transpose();
        for ((id, v), (expected, p)) in model.vertices.iter_mut().zip(&self.vertices) {
            if id != expected {
                return Err(KernelError::MissingEntity);
            }
            v.position = if forward {
                matrix.transform_point3(*p)
            } else {
                *p
            };
        }
        for ((id, f), (expected, p)) in model.faces.iter_mut().zip(&self.planes) {
            if id != expected {
                return Err(KernelError::MissingEntity);
            }
            f.plane = *p;
            if forward {
                let normal = normals.transform_vector3(p.normal).normalize();
                f.plane.normal = normal;
                f.plane.distance = normal.dot(matrix.transform_point3(p.normal * p.distance));
            }
        }
        if let Some(index) = model.spatial.as_mut() {
            index.offset = DVec3::ZERO;
            index.basis = matrix * self.index_frame;
        }
        Ok(())
    }
}
