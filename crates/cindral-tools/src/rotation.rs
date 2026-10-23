//! Shared right-handed Rotate math. Positive is counterclockwise from the normal side.
use glam::{DMat4, DVec3};
/// Rigid rotation about an arbitrary center and normalized world normal.
pub fn matrix(center: DVec3, normal: DVec3, radians: f64) -> DMat4 {
    DMat4::from_translation(center)
        * DMat4::from_axis_angle(normal, radians)
        * DMat4::from_translation(-center)
}
/// Signed angle from a planar reference, positive toward normal cross reference.
pub fn angle(normal: DVec3, reference: DVec3, offset: DVec3) -> f64 {
    offset
        .dot(normal.cross(reference))
        .atan2(offset.dot(reference))
}
/// Snap only in the ring annulus [0.75, 1.25] times its world radius.
/// Outside this documented threshold the sweep is free, including far-pointer fine control.
pub fn snap(radians: f64, distance: f64, radius: f64, increment_degrees: u16) -> f64 {
    if radius > 0.
        && (0.75..=1.25).contains(&(distance / radius))
        && (1..=90).contains(&increment_degrees)
    {
        let increment = f64::from(increment_degrees).to_radians();
        (radians / increment).round() * increment
    } else {
        radians
    }
}
