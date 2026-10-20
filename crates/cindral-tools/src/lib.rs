//! Tools boundary for Cindral Drafter.
//!
//! Provides the interactive [`ToolSession`] state machine, [`ToolKind`],
//! [`ToolError`], and length parser [`parse_length`].

mod error;
mod selection;
mod session;
pub use selection::{Hit, Selection};
mod units;

pub use cindral_core::{Command, DVec3};
pub use cindral_inference::{DrawingPlane, TangentHint};
pub use error::ToolError;
pub use session::{ToolKind, ToolSession};
pub use units::parse_length;

/// The crate identifier for diagnostics.
pub const CRATE_NAME: &str = "cindral-tools";

#[cfg(test)]
mod tests {
    #[test]
    fn identifies_crate() {
        assert_eq!(super::CRATE_NAME, "cindral-tools");
    }
}
