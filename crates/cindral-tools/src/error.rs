//! Error types for drawing tools.

/// Errors produced by drawing tools and measurement parsing.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ToolError {
    /// A coordinate or point is non-finite or outside supported bounds.
    #[error("point is nonfinite or outside bounds")]
    InvalidPoint,
    /// The geometric input forms a zero length, zero area, or otherwise degenerate shape.
    #[error("geometry is degenerate")]
    Degenerate,
    /// The measurement string could not be parsed, has unknown units, or is invalid.
    #[error("invalid measurement: {0}")]
    InvalidMeasurement(String),
    /// The tool is not in a state to commit a command.
    #[error("tool is not ready to commit")]
    NotReady,
    /// The segment count is outside the valid range 3..=256.
    #[error("segment count must be between 3 and 256")]
    InvalidSegments,
}
