//! Compact, topology-preserving whole-model translations.
use crate::{Command, DVec3, FaceId, KernelError, Model, Plane, VertexId};
#[derive(Clone, Debug)]
pub(crate) struct Translation {
    pub vertices: Vec<(VertexId, DVec3)>,
    pub planes: Vec<(FaceId, Plane)>,
    pub edges: Vec<crate::EdgeId>,
    pub offset: DVec3,
    pub index_offset: DVec3,
}
impl Translation {
    pub fn offset(command: &Command) -> Option<(&[VertexId], DVec3)> {
        if let Command::Transform {
            vertices,
            matrix,
            copy: false,
        } = command
        {
            if matrix.x_axis == glam::DVec4::X
                && matrix.y_axis == glam::DVec4::Y
                && matrix.z_axis == glam::DVec4::Z
                && matrix.w_axis.w == 1.
            {
                return Some((vertices, matrix.w_axis.truncate()));
            }
        }
        None
    }
    pub fn prepare(model: &Model, command: &Command) -> Result<Option<Self>, KernelError> {
        let Some((vertices, offset)) = Self::offset(command) else {
            return Ok(None);
        };
        if vertices.is_empty()
            || vertices.len() != model.vertices.len()
            || !vertices.iter().eq(model.vertices.keys())
        {
            return Ok(None);
        }
        for vertex in model.vertices.values() {
            crate::geometry::validate_position(vertex.position + offset)?;
        }
        Ok(Some(Self {
            vertices: model
                .vertices
                .iter()
                .map(|(id, v)| (*id, v.position))
                .collect(),
            edges: model.edges.keys().copied().collect(),
            planes: model.faces.iter().map(|(id, f)| (*id, f.plane)).collect(),
            offset,
            index_offset: model.translation_offset(),
        }))
    }
    pub fn replace(
        &mut self,
        model: &mut Model,
        command: &Command,
    ) -> Result<Option<bool>, KernelError> {
        let Some((vertices, offset)) = Self::offset(command) else {
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
            crate::geometry::validate_position(*p + offset)?;
        }
        self.offset = offset;
        self.apply(model, true)?;
        Ok(Some(offset != DVec3::ZERO))
    }
    pub fn apply(&self, model: &mut Model, forward: bool) -> Result<(), KernelError> {
        let offset = if forward { self.offset } else { DVec3::ZERO };
        if model.vertices.len() != self.vertices.len() || model.faces.len() != self.planes.len() {
            return Err(KernelError::InvalidTopology(
                "translation topology changed".into(),
            ));
        }
        for ((id, v), (expected, p)) in model.vertices.iter_mut().zip(&self.vertices) {
            if id != expected {
                return Err(KernelError::MissingEntity);
            }
            v.position = *p + offset;
        }
        for ((id, f), (expected, p)) in model.faces.iter_mut().zip(&self.planes) {
            if id != expected {
                return Err(KernelError::MissingEntity);
            }
            f.plane = *p;
            f.plane.distance = p.distance + p.normal.dot(offset);
        }
        if let Some(index) = model.spatial.as_mut() {
            index.offset = self.index_offset + offset;
        }
        Ok(())
    }
}
