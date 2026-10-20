//! Core boundary for Cindral Drafter.

/// The crate identifier for diagnostics.
pub const CRATE_NAME: &str = "cindral-core";

#[cfg(test)]
mod tests {
    #[test]
    fn identifies_crate() {
        assert_eq!(super::CRATE_NAME, "cindral-core");
    }
}
