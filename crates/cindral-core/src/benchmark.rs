//! Known-topology fixtures, outside user document construction and timing.
//!
//! Each separated unit square has a diagonal: 4 vertices, 5 edges, 2 faces.
//! Direct construction avoids quadratic seed insertion and is feature-gated.

use crate::{
    Command, DVec3, Edge, EdgeId, Face, FaceId, History, KernelError, Model, Plane, Vertex,
    VertexId,
};

/// Large seeded document and the single-face edit to measure.
pub struct Fixture {
    /// Empty-history seeded model.
    pub history: History,
    /// Segment splitting the first triangle at the midpoint of its outer edge.
    pub edit: Command,
}

/// Construct `columns * rows` separated triangulated squares in linear time.
/// 100×100 yields exactly 50,000 edges and 20,000 faces.
pub fn tiled_squares(columns: usize, rows: usize) -> Result<Fixture, KernelError> {
    if columns == 0 || rows == 0 || columns > 1000 || rows > 1000 {
        return Err(KernelError::InvalidPosition);
    }
    let mut model = Model::default();
    for y in 0..rows {
        for x in 0..columns {
            let origin = DVec3::new(x as f64 * 2.0, y as f64 * 2.0, 0.0);
            let points = [
                origin,
                origin + DVec3::X,
                origin + DVec3::X + DVec3::Y,
                origin + DVec3::Y,
            ];
            let mut ids = Vec::new();
            for position in points {
                let id = VertexId::from_raw(model.allocate()?);
                model.vertices.insert(id, Vertex { position });
                ids.push(id);
            }
            for (a, b) in [(0, 1), (1, 2), (2, 3), (3, 0), (0, 2)] {
                let mut vertices = [ids[a], ids[b]];
                vertices.sort();
                let id = EdgeId::from_raw(model.allocate()?);
                model.edges.insert(
                    id,
                    Edge {
                        vertices,
                        soft: false,
                        smooth: false,
                        hidden: false,
                    },
                );
            }
            for indices in [[0, 1, 2], [0, 2, 3]] {
                let id = FaceId::from_raw(model.allocate()?);
                model.faces.insert(
                    id,
                    Face {
                        color: [160, 177, 193, 255],
                        material: None,
                        outer: indices.map(|i| ids[i]).to_vec(),
                        holes: Vec::new(),
                        plane: Plane {
                            normal: DVec3::Z,
                            distance: 0.0,
                        },
                    },
                );
            }
        }
    }
    Ok(Fixture {
        history: History::benchmark_seed(model)?,
        edit: Command::AddEdge {
            start: DVec3::ZERO,
            end: DVec3::new(1.0, 0.5, 0.0),
        },
    })
}

/// Add an isolated 2×2×1 m closed solid outside the tiled fixture, returning its top face.
/// Setup is excluded from interactive benchmark timing.
pub fn add_test_solid(history: &mut History) -> Result<crate::FaceId, KernelError> {
    history.execute(crate::Command::AddPolyline {
        points: vec![
            DVec3::new(-4., -4., 0.),
            DVec3::new(-2., -4., 0.),
            DVec3::new(-2., -2., 0.),
            DVec3::new(-4., -2., 0.),
        ],
        closed: true,
    })?;
    let face = *history
        .model()
        .faces()
        .iter()
        .find(|(id, f)| {
            f.plane.normal.z > 0.9
                && f.plane.distance.abs() < crate::TOLERANCE
                && history
                    .model()
                    .face_area(**id)
                    .is_ok_and(|a| (a - 4.).abs() < crate::TOLERANCE)
        })
        .ok_or(KernelError::MissingEntity)?
        .0;
    history.execute(crate::Command::PushPull { face, distance: 1. })?;
    history
        .model()
        .faces()
        .iter()
        .find(|(_, f)| f.plane.normal.z > 0.9 && (f.plane.distance - 1.).abs() < crate::TOLERANCE)
        .map(|(id, _)| *id)
        .ok_or(KernelError::MissingEntity)
}

/// Execute the retained reference algorithm for small-model equivalence tests.
/// This benchmark-only API must never be used for interactive large models.
/// Execute a closure with reference (slow CSG) path forced.
pub fn with_reference<R>(f: impl FnOnce() -> R) -> R {
    struct Reset(bool);
    impl Drop for Reset {
        fn drop(&mut self) {
            REFERENCE.with(|r| r.set(self.0));
        }
    }
    let previous = REFERENCE.with(|r| r.replace(true));
    let _reset = Reset(previous);
    f()
}

/// Execute a command using the reference (slow CSG) fallback path.
pub fn reference_execute(history: &mut History, command: Command) -> Result<bool, KernelError> {
    with_reference(|| history.execute(command))
}
thread_local! {static REFERENCE: std::cell::Cell<bool> = const {std::cell::Cell::new(false)};}
pub(crate) fn reference_enabled() -> bool {
    REFERENCE.with(|r| r.get())
}

#[cfg(test)]
mod tests {
    #[test]
    fn seed_is_structurally_valid_and_edit_has_expected_effect() {
        let mut f = super::tiled_squares(2, 2).unwrap();
        assert_eq!(f.history.model().edges().len(), 20);
        assert_eq!(f.history.model().faces().len(), 8);
        f.history.model().validate().unwrap();
        f.history.execute(f.edit).unwrap();
        assert_eq!(f.history.model().faces().len(), 9);
        f.history.model().validate().unwrap();
    }
}
