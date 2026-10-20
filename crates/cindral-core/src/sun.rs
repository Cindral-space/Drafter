//! Geographic sun position in the model's east/north/up coordinate frame.
//! NOAA solar equations using Julian centuries, without atmospheric refraction.
use crate::{DVec3, ShadowSettings};
/// Whether a Gregorian civil date is valid in the supported 1900..2100 range.
pub fn valid_date(year: i32, month: u32, day: u32) -> bool {
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let days = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if leap {
                29
            } else {
                28
            }
        }
        _ => return false,
    };
    (1900..=2100).contains(&year) && day >= 1 && day <= days
}
/// Unit vector toward the sun: +X east, +Y north, +Z up. Time is local civil
/// minutes with an explicit UTC offset (including DST). Negative Z means night.
/// Uses NOAA's Julian-century equations; refraction/parallax are excluded.
pub fn sun_direction(s: &ShadowSettings) -> DVec3 {
    let mut year = s.year;
    let mut month = s.month as i32;
    if month <= 2 {
        year -= 1;
        month += 12;
    }
    let a = (year as f64 / 100.).floor();
    let b = 2. - a + (a / 4.).floor();
    let jd = (365.25 * (year as f64 + 4716.)).floor()
        + (30.6001 * (month as f64 + 1.)).floor()
        + s.day as f64
        + b
        - 1524.5
        + (s.minutes - s.utc_offset * 60.) / 1440.;
    let t = (jd - 2451545.) / 36525.;
    let l = (280.46646 + t * (36000.76983 + t * 0.0003032)).rem_euclid(360.);
    let m = (357.52911 + t * (35999.05029 - 0.0001537 * t)).to_radians();
    let eccentricity = 0.016708634 - t * (0.000042037 + 0.0000001267 * t);
    let center = m.sin() * (1.914602 - t * (0.004817 + 0.000014 * t))
        + (2. * m).sin() * (0.019993 - 0.000101 * t)
        + (3. * m).sin() * 0.000289;
    let omega = (125.04 - 1934.136 * t).to_radians();
    let lambda = (l + center - 0.00569 - 0.00478 * omega.sin()).to_radians();
    let obliquity = (23.
        + (26. + (21.448 - t * (46.815 + t * (0.00059 - t * 0.001813))) / 60.) / 60.
        + 0.00256 * omega.cos())
    .to_radians();
    let declination = (obliquity.sin() * lambda.sin()).asin();
    let y = (obliquity / 2.).tan().powi(2);
    let lr = l.to_radians();
    let equation = 4.
        * (y * (2. * lr).sin() - 2. * eccentricity * m.sin()
            + 4. * eccentricity * y * m.sin() * (2. * lr).cos()
            - 0.5 * y * y * (4. * lr).sin()
            - 1.25 * eccentricity * eccentricity * (2. * m).sin())
        .to_degrees();
    let solar = (s.minutes + equation + 4. * s.longitude - 60. * s.utc_offset).rem_euclid(1440.);
    let hour = (solar / 4. - 180.).to_radians();
    let latitude = s.latitude.to_radians();
    DVec3::new(
        -declination.cos() * hour.sin(),
        latitude.cos() * declination.sin() - latitude.sin() * declination.cos() * hour.cos(),
        latitude.sin() * declination.sin() + latitude.cos() * declination.cos() * hour.cos(),
    )
    .normalize()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn nrel_boulder_reference() {
        // NREL SPA report (2003), Table A.1: azimuth 194.34024, zenith
        // 50.11162 (includes refraction); NOAA geometric approximation within 0.05°.
        let s = ShadowSettings {
            year: 2003,
            month: 10,
            day: 17,
            minutes: 750.5,
            utc_offset: -7.,
            latitude: 39.742476,
            longitude: -105.1786,
            ..Default::default()
        };
        let sun = sun_direction(&s);
        let azimuth = sun.x.atan2(sun.y).to_degrees().rem_euclid(360.);
        let zenith = sun.z.acos().to_degrees();
        assert!((azimuth - 194.34024).abs() < 0.05, "azimuth {azimuth}");
        assert!((zenith - 50.11162).abs() < 0.05, "zenith {zenith}");
    }
    #[test]
    fn equinox_noon_night_and_timezone() {
        let mut s = ShadowSettings {
            year: 2024,
            month: 3,
            day: 20,
            latitude: 0.,
            longitude: 0.,
            utc_offset: 0.,
            minutes: 720.,
            ..Default::default()
        };
        let noon = sun_direction(&s);
        assert!(noon.z > 0.999);
        s.minutes = 0.;
        assert!(sun_direction(&s).z < -0.999);
        s.minutes = 720.;
        s.utc_offset = 8.;
        let local = sun_direction(&s);
        s.minutes = 240.;
        s.utc_offset = 0.;
        assert!(local.distance(sun_direction(&s)) < 1e-12);
        s.month = 1;
        s.day = 1;
        s.minutes = 30.;
        s.utc_offset = 8.;
        let local = sun_direction(&s);
        s.year = 2023;
        s.month = 12;
        s.day = 31;
        s.minutes = 990.;
        s.utc_offset = 0.;
        assert!(local.distance(sun_direction(&s)) < 1e-12);
    }
    #[test]
    fn leap_dates_and_poles() {
        assert!(valid_date(2000, 2, 29));
        assert!(!valid_date(1900, 2, 29));
        assert!(valid_date(2024, 2, 29));
        assert!(!valid_date(2026, 2, 29));
        assert!(!valid_date(2026, 4, 31));
        for latitude in [-90., 90.] {
            let s = ShadowSettings {
                latitude,
                ..Default::default()
            };
            assert!(sun_direction(&s).is_finite());
            assert!((sun_direction(&s).length() - 1.).abs() < 1e-12);
        }
    }
}
