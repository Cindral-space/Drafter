//! Rendering-independent edge/face topology and transactional command history.
//!
//! All positions are meters in a right-handed Z-up coordinate system. Model
//! maps are read-only; [`History::execute`] is the mutation entry point.

//! # Example
//!
//! ```
//! use cindral_core::{Command, DVec3, History, KernelError};
//! let mut history = History::new();
//! history.execute(Command::AddPolyline {
//!     points: vec![DVec3::ZERO, DVec3::X, DVec3::X + DVec3::Y, DVec3::Y],
//!     closed: true,
//! })?;
//! assert_eq!(history.model().faces().len(), 1);
//! history.execute(Command::AddEdge { start: DVec3::ZERO, end: DVec3::X + DVec3::Y })?;
//! assert_eq!(history.model().faces().len(), 2);
//! history.undo()?;
//! assert_eq!(history.model().faces().len(), 1);
//! # Ok::<(), KernelError>(())
//! ```

mod command;
mod csg;
mod geometry;
mod model;
mod modification;
mod spatial;
mod topology;
mod transaction;

pub use command::{Command, History};
pub use geometry::{Plane, MAX_COORDINATE, TOLERANCE};
pub use glam::DVec3;
pub use model::{Edge, EdgeId, Face, FaceId, Model, Vertex, VertexId};
pub use transaction::ChangeSet;

/// Failures leave the model and its command history unchanged.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum KernelError {
    /// Positions must be finite and inside the supported coordinate range.
    #[error("position is nonfinite or outside ±{MAX_COORDINATE} meters")]
    InvalidPosition,
    /// An input segment is too short to form an edge.
    #[error("edge endpoints must be more than {TOLERANCE} meters apart")]
    DegenerateEdge,
    /// A polyline does not have enough points.
    #[error("a polyline needs at least two points (three when closed)")]
    InvalidPolyline,
    /// An entity ID is stale or does not belong to this model.
    #[error("entity does not exist")]
    MissingEntity,
    /// Entity or revision counters cannot advance.
    #[error("entity or revision counter exhausted")]
    Capacity,
    /// Command batch nesting exceeds the supported depth.
    #[error("command batches may nest at most 64 levels")]
    BatchDepth,
    /// A derived topology invariant failed.
    #[error("invalid topology: {0}")]
    InvalidTopology(String),
    /// A bounded region could not be triangulated.
    #[error("face triangulation failed")]
    Triangulation,
}

/// Deterministic model construction for performance experiments only.
#[cfg(feature = "bench-fixtures")]
pub mod benchmark;
