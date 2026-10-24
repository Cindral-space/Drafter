//! sRGB color conversions and hex parsing shared by UI, renderer and IO.
//!
//! Materials store display sRGB values (what a color picker shows). Lighting
//! happens in linear space, so uploads convert with [`srgb_to_linear`] and the
//! final framebuffer encode converts back with [`linear_to_srgb`].

/// Converts one sRGB channel in 0..=1 to linear light (IEC 61966-2-1).
pub fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// Converts one linear-light channel in 0..=1 to display sRGB.
pub fn linear_to_srgb(c: f32) -> f32 {
    if c <= 0.0031308 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    }
}

/// Converts an sRGB triple to linear light.
pub fn srgb_to_linear_rgb(srgb: [u8; 3]) -> [f32; 3] {
    [
        srgb_to_linear(srgb[0] as f32 / 255.),
        srgb_to_linear(srgb[1] as f32 / 255.),
        srgb_to_linear(srgb[2] as f32 / 255.),
    ]
}

/// Converts a linear-light triple to sRGB bytes, clamping out-of-range input.
pub fn linear_rgb_to_srgb(linear: [f32; 3]) -> [u8; 3] {
    let mut out = [0u8; 3];
    for (slot, v) in out.iter_mut().zip(linear) {
        *slot = (linear_to_srgb(v.clamp(0., 1.)) * 255.)
            .round()
            .clamp(0., 255.) as u8;
    }
    out
}

/// Parses `#RRGGBB`, `RRGGBB`, `#RGB` and `RGB` hex color text into sRGB bytes.
///
/// `#` is optional, letters are case-insensitive, surrounding whitespace is
/// tolerated; anything else (wrong length, non-hex digits) is rejected.
pub fn parse_hex_rgb(text: &str) -> Option<[u8; 3]> {
    let trimmed = text.trim();
    let digits = trimmed.strip_prefix('#').unwrap_or(trimmed);
    if digits.len() != 6 && digits.len() != 3 {
        return None;
    }
    if !digits.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let byte = |hi: u8, lo: u8| {
        let hex = |b: u8| (b as char).to_digit(16).unwrap() as u8;
        hex(hi) * 16 + hex(lo)
    };
    if digits.len() == 6 {
        let d = digits.as_bytes();
        Some([byte(d[0], d[1]), byte(d[2], d[3]), byte(d[4], d[5])])
    } else {
        let d = digits.as_bytes();
        Some([byte(d[0], d[0]), byte(d[1], d[1]), byte(d[2], d[2])])
    }
}

/// Formats sRGB bytes as `#RRGGBB` (lowercase).
pub fn format_hex_rgb(srgb: [u8; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", srgb[0], srgb[1], srgb[2])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Shader/framebuffer round-trips must agree within one 8-bit step.
    fn assert_round_trip(srgb: [u8; 3]) {
        let linear = srgb_to_linear_rgb(srgb);
        let back = linear_rgb_to_srgb(linear);
        for (a, b) in srgb.iter().zip(back) {
            assert!(
                (i16::from(*a) - i16::from(b)).abs() <= 1,
                "round trip {srgb:?} -> {back:?}"
            );
        }
    }

    #[test]
    fn srgb_linear_round_trip_at_representative_values() {
        for v in [0u8, 1, 16, 64, 128, 191, 224, 254, 255] {
            assert_round_trip([v, 255 - v / 2, v / 3]);
        }
    }

    #[test]
    fn known_conversion_values() {
        assert!((srgb_to_linear(0.0) - 0.0).abs() < 1e-7);
        assert!((srgb_to_linear(1.0) - 1.0).abs() < 1e-7);
        assert!((linear_to_srgb(0.0) - 0.0).abs() < 1e-7);
        assert!((linear_to_srgb(1.0) - 1.0).abs() < 1e-7);
        // Mid gray 128/255 sRGB is about 0.2158 linear light.
        let mid = srgb_to_linear(128. / 255.);
        assert!((mid - 0.2159).abs() < 5e-4);
        let gray = linear_rgb_to_srgb([mid, mid, mid]);
        assert_eq!(gray, [128, 128, 128]);
    }

    #[test]
    fn linear_rgb_to_srgb_clamps() {
        assert_eq!(linear_rgb_to_srgb([-1., 0.5, 2.])[0], 0);
        assert_eq!(linear_rgb_to_srgb([-1., 0.5, 2.])[2], 255);
    }

    #[test]
    fn hex_parsing_accepts_supported_forms() {
        assert_eq!(parse_hex_rgb("#ff8000"), Some([255, 128, 0]));
        assert_eq!(parse_hex_rgb("FF8000"), Some([255, 128, 0]));
        assert_eq!(parse_hex_rgb("#F08"), Some([255, 0, 136]));
        assert_eq!(parse_hex_rgb("abc"), Some([170, 187, 204]));
        assert_eq!(parse_hex_rgb("  #1a2b3c "), Some([26, 43, 60]));
    }

    #[test]
    fn hex_parsing_rejects_invalid_input() {
        for bad in [
            "",
            "#",
            "#ff80",
            "#ff80000",
            "ff800",
            "gg8000",
            "#gg0000",
            "ff 800",
            "#ff80 0",
            "0x1a2b3c",
            "#1a2b3g",
            "\u{ff0f}ff0000",
        ] {
            assert_eq!(parse_hex_rgb(bad), None, "unexpectedly parsed {bad:?}");
        }
    }

    #[test]
    fn hex_formatting_matches_parsing() {
        let rgb = [26, 43, 60];
        let text = format_hex_rgb(rgb);
        assert_eq!(text, "#1a2b3c");
        assert_eq!(parse_hex_rgb(&text), Some(rgb));
    }
}
