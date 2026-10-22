//! Measurement and unit parsing.

use crate::ToolError;

/// Parse a length string into finite signed meters.
///
/// Supports plain numbers (meters) as well as units: `mm`, `cm`, `m`, `in`, `ft`.
/// Rejects unknown units, invalid numbers, non-finite values, and malformed or trailing input.
pub fn parse_length(input: &str) -> Result<f64, ToolError> {
    match crate::measurements::parse(input, Default::default())? {
        crate::measurements::Measurement::Values(values) if values.len() == 1 => {
            values[0].length(Default::default())
        }
        _ => Err(ToolError::InvalidMeasurement(input.into())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_units_and_scales_to_meters() {
        assert_eq!(parse_length("5").unwrap(), 5.0);
        assert_eq!(parse_length("5m").unwrap(), 5.0);
        assert_eq!(parse_length("5 m").unwrap(), 5.0);
        assert_eq!(parse_length("5000mm").unwrap(), 5.0);
        assert_eq!(parse_length("500cm").unwrap(), 5.0);
        assert!((parse_length("12in").unwrap() - 0.3048).abs() < 1e-9);
        assert!((parse_length("10ft").unwrap() - 3.048).abs() < 1e-9);
        assert_eq!(parse_length("-2.5m").unwrap(), -2.5);
        assert_eq!(parse_length("+100mm").unwrap(), 0.1);
    }

    #[test]
    fn rejects_invalid_and_unknown_units() {
        assert!(matches!(
            parse_length(""),
            Err(ToolError::InvalidMeasurement(_))
        ));
        assert!(matches!(
            parse_length("   "),
            Err(ToolError::InvalidMeasurement(_))
        ));
        assert!(matches!(
            parse_length("m"),
            Err(ToolError::InvalidMeasurement(_))
        ));
        assert!(matches!(
            parse_length("10yd"),
            Err(ToolError::InvalidMeasurement(_))
        ));
        assert!(matches!(
            parse_length("10km"),
            Err(ToolError::InvalidMeasurement(_))
        ));
        assert!(matches!(
            parse_length("10dm"),
            Err(ToolError::InvalidMeasurement(_))
        ));
        assert!(matches!(
            parse_length("10foo"),
            Err(ToolError::InvalidMeasurement(_))
        ));
        assert!(matches!(
            parse_length("10m5"),
            Err(ToolError::InvalidMeasurement(_))
        ));
        assert!(matches!(
            parse_length("10 m foo"),
            Err(ToolError::InvalidMeasurement(_))
        ));
        assert!(matches!(
            parse_length("NaN"),
            Err(ToolError::InvalidMeasurement(_))
        ));
        assert!(matches!(
            parse_length("inf"),
            Err(ToolError::InvalidMeasurement(_))
        ));
    }
}
