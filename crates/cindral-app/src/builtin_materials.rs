//! Original builtin color and procedural pattern collection for the material picker.
//!
//! Every color value and name in this module was invented for this project; nothing
//! is copied from any vendor palette and no names are trademarked. Patterns are
//! generated procedurally in code (no file reads, no external crates) and are
//! seamlessly tileable: all pattern periods divide the requested size.

use cindral_render::texture::TexturePixels;

/// A named builtin material color belonging to one display group.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BuiltinColor {
    /// Unique human-readable color name.
    pub name: &'static str,
    /// Display group the color belongs to.
    pub group: BuiltinGroup,
    /// sRGB color as three 8-bit channels.
    pub srgb: [u8; 3],
}

/// Display group of a [`BuiltinColor`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BuiltinGroup {
    /// Whites, greys, and blacks.
    Neutral,
    /// Wood tones from light maple to dark walnut.
    Wood,
    /// Muted interior wall paints.
    Wall,
    /// Saturated accent colors.
    Accent,
}

impl std::fmt::Display for BuiltinGroup {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name = match self {
            Self::Neutral => "neutrals",
            Self::Wood => "wood",
            Self::Wall => "walls",
            Self::Accent => "accents",
        };
        f.write_str(name)
    }
}

/// All builtin colors, ordered by group: neutrals, wood, walls, accents.
pub const BUILTIN_COLORS: &[BuiltinColor] = &[
    // Neutrals (10)
    BuiltinColor {
        name: "Chalk Veil",
        group: BuiltinGroup::Neutral,
        srgb: [246, 244, 239],
    },
    BuiltinColor {
        name: "Ashen Mist",
        group: BuiltinGroup::Neutral,
        srgb: [214, 213, 208],
    },
    BuiltinColor {
        name: "Pebble Grey",
        group: BuiltinGroup::Neutral,
        srgb: [188, 187, 182],
    },
    BuiltinColor {
        name: "Cement Pale",
        group: BuiltinGroup::Neutral,
        srgb: [166, 165, 160],
    },
    BuiltinColor {
        name: "Steel Hush",
        group: BuiltinGroup::Neutral,
        srgb: [142, 142, 140],
    },
    BuiltinColor {
        name: "Graphite Soft",
        group: BuiltinGroup::Neutral,
        srgb: [110, 110, 108],
    },
    BuiltinColor {
        name: "Storm Slate",
        group: BuiltinGroup::Neutral,
        srgb: [84, 86, 88],
    },
    BuiltinColor {
        name: "Iron Shadow",
        group: BuiltinGroup::Neutral,
        srgb: [62, 63, 65],
    },
    BuiltinColor {
        name: "Charcoal Night",
        group: BuiltinGroup::Neutral,
        srgb: [40, 41, 43],
    },
    BuiltinColor {
        name: "Ink Well",
        group: BuiltinGroup::Neutral,
        srgb: [24, 25, 27],
    },
    // Wood (10)
    BuiltinColor {
        name: "Birch Pale",
        group: BuiltinGroup::Wood,
        srgb: [231, 214, 185],
    },
    BuiltinColor {
        name: "Maple Froth",
        group: BuiltinGroup::Wood,
        srgb: [222, 199, 163],
    },
    BuiltinColor {
        name: "Ash Sunlit",
        group: BuiltinGroup::Wood,
        srgb: [213, 187, 148],
    },
    BuiltinColor {
        name: "Pine Honey",
        group: BuiltinGroup::Wood,
        srgb: [203, 172, 128],
    },
    BuiltinColor {
        name: "Oak Golden",
        group: BuiltinGroup::Wood,
        srgb: [189, 154, 108],
    },
    BuiltinColor {
        name: "Chestnut Warm",
        group: BuiltinGroup::Wood,
        srgb: [172, 134, 92],
    },
    BuiltinColor {
        name: "Teak Amber",
        group: BuiltinGroup::Wood,
        srgb: [154, 116, 78],
    },
    BuiltinColor {
        name: "Cedar Rust",
        group: BuiltinGroup::Wood,
        srgb: [136, 99, 66],
    },
    BuiltinColor {
        name: "Rosewood Ember",
        group: BuiltinGroup::Wood,
        srgb: [117, 82, 54],
    },
    BuiltinColor {
        name: "Walnut Deep",
        group: BuiltinGroup::Wood,
        srgb: [94, 66, 42],
    },
    // Walls (12)
    BuiltinColor {
        name: "Linen Hush",
        group: BuiltinGroup::Wall,
        srgb: [233, 226, 214],
    },
    BuiltinColor {
        name: "Oat Field",
        group: BuiltinGroup::Wall,
        srgb: [224, 214, 196],
    },
    BuiltinColor {
        name: "Sand Drift",
        group: BuiltinGroup::Wall,
        srgb: [215, 202, 180],
    },
    BuiltinColor {
        name: "Clay Court",
        group: BuiltinGroup::Wall,
        srgb: [199, 182, 158],
    },
    BuiltinColor {
        name: "Kiln Clay",
        group: BuiltinGroup::Wall,
        srgb: [188, 142, 112],
    },
    BuiltinColor {
        name: "Sage Whisper",
        group: BuiltinGroup::Wall,
        srgb: [178, 188, 168],
    },
    BuiltinColor {
        name: "Fern Murmur",
        group: BuiltinGroup::Wall,
        srgb: [150, 165, 140],
    },
    BuiltinColor {
        name: "Sea Glass Calm",
        group: BuiltinGroup::Wall,
        srgb: [168, 192, 186],
    },
    BuiltinColor {
        name: "Harbour Mist",
        group: BuiltinGroup::Wall,
        srgb: [172, 184, 190],
    },
    BuiltinColor {
        name: "Faded Denim",
        group: BuiltinGroup::Wall,
        srgb: [120, 140, 158],
    },
    BuiltinColor {
        name: "Lilac Ash",
        group: BuiltinGroup::Wall,
        srgb: [196, 186, 194],
    },
    BuiltinColor {
        name: "Rose Putty",
        group: BuiltinGroup::Wall,
        srgb: [208, 188, 178],
    },
    // Accents (8)
    BuiltinColor {
        name: "Coral Spark",
        group: BuiltinGroup::Accent,
        srgb: [224, 108, 86],
    },
    BuiltinColor {
        name: "Poppy Ember",
        group: BuiltinGroup::Accent,
        srgb: [204, 62, 58],
    },
    BuiltinColor {
        name: "Marigold Bold",
        group: BuiltinGroup::Accent,
        srgb: [232, 168, 52],
    },
    BuiltinColor {
        name: "Moss Verge",
        group: BuiltinGroup::Accent,
        srgb: [96, 122, 70],
    },
    BuiltinColor {
        name: "Teal Anchor",
        group: BuiltinGroup::Accent,
        srgb: [48, 116, 118],
    },
    BuiltinColor {
        name: "Marine Dive",
        group: BuiltinGroup::Accent,
        srgb: [38, 74, 110],
    },
    BuiltinColor {
        name: "Plum Cellar",
        group: BuiltinGroup::Accent,
        srgb: [110, 66, 104],
    },
    BuiltinColor {
        name: "Berry Stain",
        group: BuiltinGroup::Accent,
        srgb: [146, 52, 74],
    },
];

/// Default generation size in pixels for builtin patterns (square previews).
pub const PATTERN_SIZE: u32 = 256;

/// Period in pixels of the fine checker pattern (divides 256).
pub const FINE_CHECKER_PERIOD: u32 = 32;

/// Period in pixels of the coarse checker pattern (divides 256).
pub const COARSE_CHECKER_PERIOD: u32 = 64;

/// Lattice cell size in pixels of the fine plaster noise (divides 256).
pub const FINE_PLASTER_CELL: u32 = 8;

/// Lattice cell size in pixels of the coarse plaster noise (divides 256).
pub const COARSE_PLASTER_CELL: u32 = 32;

/// Stripe period in pixels of the wood grain patterns (divides 256).
pub const WOOD_GRAIN_PERIOD: u32 = 32;

/// A builtin procedurally generated tileable pattern.
#[derive(Clone, Copy)]
pub struct BuiltinPattern {
    /// Unique human-readable pattern name.
    pub name: &'static str,
    /// Base sRGB color suggestion for the material using this pattern.
    pub srgb: [u8; 3],
    /// Pure generator producing the pattern at the requested square size.
    pub generator: fn(u32) -> TexturePixels,
}

/// All builtin patterns: fine/coarse checkers, fine/coarse plaster noise, and
/// horizontal/vertical wood grain, in that order.
pub const BUILTIN_PATTERNS: &[BuiltinPattern; 6] = &[
    BuiltinPattern {
        name: "Checker Fine",
        srgb: [208, 206, 200],
        generator: checker_fine,
    },
    BuiltinPattern {
        name: "Checker Coarse",
        srgb: [192, 190, 184],
        generator: checker_coarse,
    },
    BuiltinPattern {
        name: "Plaster Fine",
        srgb: [228, 222, 212],
        generator: plaster_fine,
    },
    BuiltinPattern {
        name: "Plaster Coarse",
        srgb: [216, 208, 196],
        generator: plaster_coarse,
    },
    BuiltinPattern {
        name: "Grain Horizontal",
        srgb: [184, 148, 106],
        generator: wood_grain_horizontal,
    },
    BuiltinPattern {
        name: "Grain Vertical",
        srgb: [170, 132, 92],
        generator: wood_grain_vertical,
    },
];

/// Generates the builtin pattern at `index` as a 256x256 sRGB texture.
pub fn pattern_texture(index: usize) -> Option<TexturePixels> {
    let pattern = BUILTIN_PATTERNS.get(index)?;
    Some((pattern.generator)(PATTERN_SIZE))
}

/// Fine checker: base squares alternating with a 20% darker variant, 32 px period.
pub fn checker_fine(size: u32) -> TexturePixels {
    checker(size, FINE_CHECKER_PERIOD, BUILTIN_PATTERNS[0].srgb)
}

/// Coarse checker: base squares alternating with a 20% darker variant, 64 px period.
pub fn checker_coarse(size: u32) -> TexturePixels {
    checker(size, COARSE_CHECKER_PERIOD, BUILTIN_PATTERNS[1].srgb)
}

/// Fine plaster noise: hashed value noise jittering luminance by +/-6%, 8 px cells.
pub fn plaster_fine(size: u32) -> TexturePixels {
    plaster(
        size,
        FINE_PLASTER_CELL,
        BUILTIN_PATTERNS[2].srgb,
        0x5EED_0001,
    )
}

/// Coarse plaster noise: hashed value noise jittering luminance by +/-6%, 32 px cells.
pub fn plaster_coarse(size: u32) -> TexturePixels {
    plaster(
        size,
        COARSE_PLASTER_CELL,
        BUILTIN_PATTERNS[3].srgb,
        0x5EED_0002,
    )
}

/// Horizontal wood grain: sine stripes of +/-12% luminance running along x, 32 px period.
pub fn wood_grain_horizontal(size: u32) -> TexturePixels {
    wood_grain(size, WOOD_GRAIN_PERIOD, BUILTIN_PATTERNS[4].srgb, false)
}

/// Vertical wood grain: sine stripes of +/-12% luminance running along y, 32 px period.
pub fn wood_grain_vertical(size: u32) -> TexturePixels {
    wood_grain(size, WOOD_GRAIN_PERIOD, BUILTIN_PATTERNS[5].srgb, true)
}

/// Integer lattice hash (multiply-xorshift), deterministic and dependency-free.
fn hash_lattice(x: u32, y: u32, salt: u32) -> u32 {
    let mut h =
        x.wrapping_mul(0x9E37_79B9) ^ y.wrapping_mul(0x85EB_CA6B) ^ salt.wrapping_mul(0xC2B2_AE3D);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2C1B_3C6D);
    h ^= h >> 12;
    h = h.wrapping_mul(0x2977_44C1);
    h ^= h >> 15;
    h
}

/// Maps a hash to a jitter value in `[-1.0, 1.0]`.
fn hash_jitter(x: u32, y: u32, salt: u32) -> f32 {
    ((hash_lattice(x, y, salt) & 0xFFFF) as f32 / 32_768.0) - 1.0
}

/// Scales an sRGB triple by `factor`, clamping to the byte range.
fn scale_srgb(base: [u8; 3], factor: f32) -> [u8; 3] {
    [
        ((base[0] as f32 * factor).round().clamp(0.0, 255.0)) as u8,
        ((base[1] as f32 * factor).round().clamp(0.0, 255.0)) as u8,
        ((base[2] as f32 * factor).round().clamp(0.0, 255.0)) as u8,
    ]
}

/// Builds a square opaque sRGB texture from a per-pixel color closure.
fn build_texture(size: u32, color_at: impl Fn(u32, u32) -> [u8; 3]) -> TexturePixels {
    let mut rgba = Vec::with_capacity(size as usize * size as usize * 4);
    for y in 0..size {
        for x in 0..size {
            rgba.extend_from_slice(&color_at(x, y));
            rgba.push(255);
        }
    }
    TexturePixels::new(size, size, rgba)
}

/// Two-tone checkerboard: `base` alternating with a 20% darker variant.
///
/// `period` is the full pattern cycle (two cells of `period / 2`), so the color
/// at offset `period / 2` differs from the origin and the color at `period`
/// matches it again. `period` must be even and divide `size` for seamless tiling.
fn checker(size: u32, period: u32, base: [u8; 3]) -> TexturePixels {
    let half = (period / 2).max(1);
    let dark = scale_srgb(base, 0.8);
    build_texture(size, |x, y| {
        if ((x / half) + (y / half)).is_multiple_of(2) {
            base
        } else {
            dark
        }
    })
}

/// Value noise plaster: luminance jitter of +/-6% from a wrapped lattice.
///
/// Lattice indices wrap modulo `size / cell`, so the noise tiles seamlessly
/// when `cell` divides `size`.
fn plaster(size: u32, cell: u32, base: [u8; 3], salt: u32) -> TexturePixels {
    let grid = (size / cell).max(1);
    build_texture(size, |x, y| {
        let gx = x / cell;
        let gy = y / cell;
        let fx = smooth((x % cell) as f32 / cell as f32);
        let fy = smooth((y % cell) as f32 / cell as f32);
        let v00 = hash_jitter(gx % grid, gy % grid, salt);
        let v10 = hash_jitter((gx + 1) % grid, gy % grid, salt);
        let v01 = hash_jitter(gx % grid, (gy + 1) % grid, salt);
        let v11 = hash_jitter((gx + 1) % grid, (gy + 1) % grid, salt);
        let top = v00 + (v10 - v00) * fx;
        let bottom = v01 + (v11 - v01) * fx;
        let jitter = top + (bottom - top) * fy;
        scale_srgb(base, 1.0 + 0.06 * jitter)
    })
}

/// Smoothstep easing for value-noise interpolation.
fn smooth(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}

/// Wood grain: sine stripes of up to +/-12% luminance plus a third harmonic.
///
/// `period` must divide `size` for seamless tiling. With `vertical` the stripes
/// run along x (varying with x); otherwise they run along y.
fn wood_grain(size: u32, period: u32, base: [u8; 3], vertical: bool) -> TexturePixels {
    let tau = std::f32::consts::TAU;
    build_texture(size, |x, y| {
        let t = if vertical { x } else { y } as f32;
        let phase = tau * t / period as f32;
        let factor = 1.0 + 0.10 * phase.sin() + 0.02 * (3.0 * phase).sin();
        scale_srgb(base, factor)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    /// Iterates a group membership count from `BUILTIN_COLORS`.
    fn count_group(group: BuiltinGroup) -> usize {
        BUILTIN_COLORS.iter().filter(|c| c.group == group).count()
    }

    /// Mean per-pixel luminance of a texture.
    fn mean_luminance(tex: &TexturePixels) -> f32 {
        let pixels = tex.rgba.as_chunks::<4>().0;
        let mut total = 0u64;
        for px in pixels {
            total += (u32::from(px[0]) + u32::from(px[1]) + u32::from(px[2])) as u64;
        }
        total as f32 / pixels.len() as f32 / 3.0
    }

    /// Base sRGB luminance of a pattern.
    fn base_luminance(srgb: [u8; 3]) -> f32 {
        (u32::from(srgb[0]) + u32::from(srgb[1]) + u32::from(srgb[2])) as f32 / 3.0
    }

    #[test]
    fn builtin_colors_count_names_and_groups() {
        assert!(
            (38..=44).contains(&BUILTIN_COLORS.len()),
            "expected 38..=44 builtin colors, got {}",
            BUILTIN_COLORS.len()
        );
        let names: HashSet<_> = BUILTIN_COLORS.iter().map(|c| c.name).collect();
        assert_eq!(
            names.len(),
            BUILTIN_COLORS.len(),
            "color names must be unique"
        );
        assert!(count_group(BuiltinGroup::Neutral) >= 6);
        assert!(count_group(BuiltinGroup::Wood) >= 8);
        assert!(count_group(BuiltinGroup::Wall) >= 8);
        assert!(count_group(BuiltinGroup::Accent) >= 6);
    }

    #[test]
    fn builtin_colors_unique_srgb_within_group() {
        for group in [
            BuiltinGroup::Neutral,
            BuiltinGroup::Wood,
            BuiltinGroup::Wall,
            BuiltinGroup::Accent,
        ] {
            let values: HashSet<_> = BUILTIN_COLORS
                .iter()
                .filter(|c| c.group == group)
                .map(|c| c.srgb)
                .collect();
            assert_eq!(
                values.len(),
                count_group(group),
                "duplicate srgb within group {group}"
            );
        }
    }

    #[test]
    fn group_display_uses_lowercase_plural_names() {
        assert_eq!(BuiltinGroup::Neutral.to_string(), "neutrals");
        assert_eq!(BuiltinGroup::Wood.to_string(), "wood");
        assert_eq!(BuiltinGroup::Wall.to_string(), "walls");
        assert_eq!(BuiltinGroup::Accent.to_string(), "accents");
    }

    #[test]
    fn checkers_alternate_base_and_dark_with_divisible_period() {
        for index in [0usize, 1] {
            let period = if index == 0 {
                FINE_CHECKER_PERIOD
            } else {
                COARSE_CHECKER_PERIOD
            };
            let base = BUILTIN_PATTERNS[index].srgb;
            let gen = BUILTIN_PATTERNS[index].generator;
            assert_eq!(PATTERN_SIZE % period, 0, "checker period must divide size");
            let tex = gen(PATTERN_SIZE);
            let px = |x: u32, y: u32| {
                let i = ((y * tex.width + x) * 4) as usize;
                [tex.rgba[i], tex.rgba[i + 1], tex.rgba[i + 2]]
            };
            let c00 = px(0, 0);
            let chalf = px(period / 2, 0);
            let cfull = px(period, 0);
            assert_ne!(c00, chalf, "adjacent half-period cells must differ");
            assert_eq!(c00, cfull, "full period must return to the base cell");
            assert_eq!(c00, base, "cell (0,0) must be the base color");
            assert_eq!(chalf, scale_srgb(base, 0.8), "variant must be 20% darker");
        }
    }

    #[test]
    fn all_patterns_are_pure_and_dimensionally_consistent() {
        for pattern in BUILTIN_PATTERNS {
            let a = (pattern.generator)(PATTERN_SIZE);
            let b = (pattern.generator)(PATTERN_SIZE);
            assert_eq!(a, b, "generator {} must be pure", pattern.name);
            assert_eq!(a.width, PATTERN_SIZE);
            assert_eq!(a.height, PATTERN_SIZE);
            assert_eq!(a.rgba.len(), (PATTERN_SIZE * PATTERN_SIZE * 4) as usize);
            assert!(
                a.rgba.iter().skip(3).step_by(4).all(|&alpha| alpha == 255),
                "pattern {} must be fully opaque",
                pattern.name
            );
            assert!(a.is_valid());
        }
    }

    #[test]
    fn noise_and_wood_mean_luminance_within_ten_percent_of_base() {
        for pattern in BUILTIN_PATTERNS {
            let is_jittered =
                pattern.name.starts_with("Plaster") || pattern.name.starts_with("Grain");
            if !is_jittered {
                continue;
            }
            let tex = (pattern.generator)(PATTERN_SIZE);
            let mean = mean_luminance(&tex);
            let base = base_luminance(pattern.srgb);
            let ratio = mean / base;
            assert!(
                (0.9..=1.1).contains(&ratio),
                "pattern {} mean luminance {mean:.1} deviates from base {base:.1}",
                pattern.name
            );
        }
    }

    #[test]
    fn wood_grain_respects_twelve_percent_amplitude() {
        for vertical in [false, true] {
            let tex = wood_grain(PATTERN_SIZE, WOOD_GRAIN_PERIOD, [128, 128, 128], vertical);
            let mut min = 255.0_f32;
            let mut max = 0.0_f32;
            for px in tex.rgba.as_chunks::<4>().0 {
                let lum = (u32::from(px[0]) + u32::from(px[1]) + u32::from(px[2])) as f32 / 3.0;
                min = min.min(lum);
                max = max.max(lum);
            }
            assert!(min >= 128.0 * 0.88 - 0.5, "undershoot {min}");
            assert!(max <= 128.0 * 1.12 + 0.5, "overshoot {max}");
        }
    }

    #[test]
    fn pattern_texture_returns_generators_at_256_or_none() {
        for (index, pattern) in BUILTIN_PATTERNS.iter().enumerate() {
            let tex = pattern_texture(index).expect("valid index must yield a texture");
            assert_eq!(tex, (pattern.generator)(PATTERN_SIZE));
            assert_eq!(tex.width, 256);
        }
        assert!(pattern_texture(BUILTIN_PATTERNS.len()).is_none());
        assert!(pattern_texture(usize::MAX).is_none());
    }

    #[test]
    fn hash_lattice_is_deterministic_and_spread() {
        let a = hash_lattice(7, 11, 0x5EED_0001);
        assert_eq!(a, hash_lattice(7, 11, 0x5EED_0001));
        assert_ne!(a, hash_lattice(8, 11, 0x5EED_0001));
        assert_ne!(a, hash_lattice(7, 11, 0x5EED_0002));
        let spread = (0..64)
            .map(|i| hash_jitter(i, i * 3, 0x5EED_0001))
            .collect::<Vec<_>>();
        assert!(spread.iter().any(|&v| v > 0.5));
        assert!(spread.iter().any(|&v| v < -0.5));
    }
}
