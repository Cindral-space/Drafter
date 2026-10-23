//! Persisted document measurement preferences. Geometry always uses meters.
/// Display and implicit-input length unit.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum LengthUnit {
    /// Meters.
    #[default]
    Meter,
    /// Centimeters.
    Centimeter,
    /// Millimeters.
    Millimeter,
    /// Decimal inches.
    Inch,
    /// Decimal feet.
    Foot,
}
impl LengthUnit {
    /// Meters per display unit.
    pub fn meters(self) -> f64 {
        match self {
            Self::Meter => 1.,
            Self::Centimeter => 0.01,
            Self::Millimeter => 0.001,
            Self::Inch => 0.0254,
            Self::Foot => 0.3048,
        }
    }
    /// Unambiguous suffix.
    pub fn suffix(self) -> &'static str {
        match self {
            Self::Meter => "m",
            Self::Centimeter => "cm",
            Self::Millimeter => "mm",
            Self::Inch => "in",
            Self::Foot => "ft",
        }
    }
}
/// Units, precision and numeric locale shared by all tools.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct MeasurementSettings {
    /// Unit used for bare lengths and displayed lengths.
    pub unit: LengthUnit,
    /// Decimal places (display safely clamps to nine).
    pub precision: u8,
    /// Use comma decimal mark and semicolon list separator.
    pub decimal_comma: bool,
    /// Rotate ring snap increment in degrees (1–90, default 15).
    pub angle_snap_degrees: u16,
}
impl Default for MeasurementSettings {
    fn default() -> Self {
        Self {
            unit: LengthUnit::Meter,
            precision: 3,
            decimal_comma: false,
            angle_snap_degrees: 15,
        }
    }
}
