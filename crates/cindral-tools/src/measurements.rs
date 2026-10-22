//! Shared syntax, unit conversion and display for every measurements consumer.
use crate::ToolError;
pub use cindral_core::{LengthUnit, MeasurementSettings};
#[derive(Clone, Copy, Debug, PartialEq)]
/// One parsed value, retaining whether its unit was explicit.
pub enum Quantity {
    /// Bare number or percentage converted to a factor.
    Number(f64),
    /// Explicit length converted to meters.
    Length(f64),
    /// Explicit angle converted to degrees.
    Degrees(f64),
}
impl Quantity {
    /// Interpret a bare number in document units; reject angles.
    pub fn length(self, settings: MeasurementSettings) -> Result<f64, ToolError> { match self { Self::Number(n) => Ok(n * settings.unit.meters()), Self::Length(n) => Ok(n), _ => Err(invalid("Expected a length")) } }
    /// Read a unitless number or degrees; reject lengths.
    pub fn number(self) -> Result<f64, ToolError> { match self { Self::Number(n) | Self::Degrees(n) => Ok(n), _ => Err(invalid("Expected a number")) } }
}
#[derive(Clone, Debug, PartialEq)]
/// Shared syntactic forms. Tools validate which forms they accept.
pub enum Measurement {
    /// One to three quantities.
    Values(Vec<Quantity>),
    /// Exactly three absolute or relative coordinates.
    Coordinates {
        /// Angle brackets indicate a displacement.
        relative: bool,
        /// X, Y, Z quantities.
        values: Vec<Quantity>,
    },
    /// Curve tessellation count.
    Sides(usize),
    /// External or internal copy array.
    Copies {
        /// Count, bounded to 256.
        count: usize,
        /// Divide the original distance instead of multiplying it.
        divide: bool,
    },
    /// Slope ratio converted to degrees.
    Slope(f64),
}
fn invalid(s: &str) -> ToolError { ToolError::InvalidMeasurement(s.into()) }
fn number(s: &str, settings: MeasurementSettings) -> Result<f64, ToolError> {
    let s = s.trim();
    if s.is_empty() || (settings.decimal_comma && s.contains('.')) || (!settings.decimal_comma && s.contains(',')) { return Err(invalid(s)); }
    let n = if settings.decimal_comma { s.replace(',', ".").parse::<f64>() } else { s.parse::<f64>() }.map_err(|_| invalid(s))?;
    if n.is_finite() { Ok(n) } else { Err(invalid(s)) }
}
fn fraction(s: &str, settings: MeasurementSettings) -> Result<f64, ToolError> {
    if let Some((a,b)) = s.trim().split_once('/') {
        let b = number(b, settings)?;
        if b <= 0. { return Err(invalid(s)); }
        let pieces: Vec<_> = a.split_whitespace().collect();
        let n = match pieces.as_slice() { [a] => number(a, settings)? / b, [whole, numerator] => { let w = number(whole, settings)?; let n = number(numerator, settings)?; if n < 0. { return Err(invalid(s)); } w.signum() * (w.abs() + n / b) }, _ => return Err(invalid(s)) };
        if n.is_finite() { Ok(n) } else { Err(invalid(s)) }
    } else { number(s, settings) }
}
fn quantity(s: &str, settings: MeasurementSettings) -> Result<Quantity, ToolError> {
    let s = s.trim();
    let result = if let Some((feet, inches)) = s.split_once('\'') {
        let f = fraction(feet, settings)?;
        let remainder = inches.trim().strip_prefix('-').unwrap_or(inches.trim());
        let i = if remainder.is_empty() { 0. } else { fraction(remainder.strip_suffix('"').ok_or_else(|| invalid(s))?, settings)? };
        if i < 0. { return Err(invalid(s)); }
        Quantity::Length((if feet.trim().starts_with('-') { -1. } else { 1. }) * (f.abs()*0.3048 + i*0.0254))
    } else {
        let mut found = None;
        for (suffix, factor) in [("mm",0.001),("cm",0.01),("ft",0.3048),("in",0.0254),("m",1.),("\"",0.0254)] {
            if let Some(n) = s.strip_suffix(suffix) { found = Some(Quantity::Length(fraction(n, settings)? * factor)); break; }
        }
        if let Some(q) = found { q } else if let Some(n) = s.strip_suffix("deg").or_else(|| s.strip_suffix('°')) { Quantity::Degrees(number(n, settings)?) }
        else if let Some(n) = s.strip_suffix("rad") { Quantity::Degrees(number(n, settings)?.to_degrees()) }
        else if let Some(n) = s.strip_suffix('%') { Quantity::Number(number(n, settings)? / 100.) }
        else { Quantity::Number(number(s, settings)?) }
    };
    let n = match result { Quantity::Length(n) | Quantity::Number(n) | Quantity::Degrees(n) => n };
    if n.is_finite() { Ok(result) } else { Err(invalid(s)) }
}
/// Parse bounded input. No tool-specific interpretation or document mutations.
pub fn parse(input: &str, settings: MeasurementSettings) -> Result<Measurement, ToolError> {
    if input.len() > 4096 { return Err(invalid("Input too long")); }
    let s = input.trim();
    let count = |s: &str| -> Result<usize, ToolError> { let n = s.trim().parse::<usize>().map_err(|_| invalid(s))?; if (1..=256).contains(&n) { Ok(n) } else { Err(invalid(s)) } };
    if let Some(n) = s.strip_suffix('s') { return Ok(Measurement::Sides(count(n)?)); }
    if let Some(n) = s.strip_suffix('x').or_else(|| s.strip_suffix('X')).or_else(|| s.strip_prefix('*')) { return Ok(Measurement::Copies { count: count(n)?, divide: false }); }
    if let Some(n) = s.strip_prefix('/') { return Ok(Measurement::Copies { count: count(n)?, divide: true }); }
    if let Some((a,b)) = s.split_once(':') { let a = number(a, settings)?; let b = number(b, settings)?; if b == 0. { return Err(invalid(s)); } return Ok(Measurement::Slope(a.atan2(b).to_degrees())); }
    let coordinates = s.starts_with('[') || s.starts_with('<');
    let body = if coordinates { s.get(1..).and_then(|v| v.strip_suffix(if s.starts_with('[') { ']' } else { '>' })).ok_or_else(|| invalid(s))? } else { s };
    let values: Vec<_> = body.split(if settings.decimal_comma { ';' } else { ',' }).map(|s| quantity(s, settings)).collect::<Result<_,_>>()?;
    if values.is_empty() || values.len() > 3 || (coordinates && values.len() != 3) { return Err(invalid(s)); }
    if coordinates { Ok(Measurement::Coordinates { relative: s.starts_with('<'), values }) } else { Ok(Measurement::Values(values)) }
}
/// Formatting occurs on value/settings changes, never in an idle frame.
pub fn format_length(meters: f64, settings: MeasurementSettings) -> String {
    let n = meters / settings.unit.meters();
    let precision = settings.precision.min(9) as usize;
    let factor = 10_f64.powi(precision as i32);
    let rounded = (n * factor).round() / factor;
    let approximate = (rounded-n).abs() > f64::EPSILON * n.abs().max(1.) * 4.;
    let mut s = format!("{}{n:.precision$}{}", if approximate { "~" } else { "" }, settings.unit.suffix());
    if settings.decimal_comma { s = s.replace('.', ","); }
    s
}
