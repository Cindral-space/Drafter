//! Texture pixel definitions, preview downsampling, validation, and planar UV derivation.

/// Validated RGBA8 pixel buffer representing a 2D image.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TexturePixels {
    /// Pixel width of the image.
    pub width: u32,
    /// Pixel height of the image.
    pub height: u32,
    /// Raw uncompressed sRGB RGBA8 byte values (`width * height * 4` bytes).
    pub rgba: Vec<u8>,
}

/// Errors during texture dimensions or byte length validation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextureError {
    /// Dimensions cannot be 0.
    ZeroDimension,
    /// Dimensions exceed maximum supported GPU limit (16384).
    DimensionTooLarge(u32, u32),
    /// Byte length does not match `width * height * 4`.
    ByteLengthMismatch {
        /// Expected byte length based on dimensions (`width * height * 4`).
        expected: usize,
        /// Actual byte length of the provided `rgba` buffer.
        actual: usize,
    },
    /// Pixel count calculation overflows usize.
    Overflow,
}

impl std::fmt::Display for TextureError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ZeroDimension => write!(f, "texture dimension is zero"),
            Self::DimensionTooLarge(w, h) => {
                write!(f, "texture dimension ({w}x{h}) exceeds maximum limit 16384")
            }
            Self::ByteLengthMismatch { expected, actual } => write!(
                f,
                "texture byte length mismatch: expected {expected}, actual {actual}"
            ),
            Self::Overflow => write!(f, "texture pixel count calculation overflowed"),
        }
    }
}

impl std::error::Error for TextureError {}

/// Preview resolution in pixels (256x256) per M6 frozen interface.
pub const PREVIEW_SIZE: u32 = 256;

/// Maximum number of texture assets supported in the GPU texture array (layers 1..127).
pub const MAX_TEXTURE_LAYERS: usize = 127;

/// Layer index for the default solid white fallback texture.
pub const DEFAULT_WHITE_LAYER: u32 = 0;

impl TexturePixels {
    /// Creates a new texture pixel buffer.
    pub fn new(width: u32, height: u32, rgba: Vec<u8>) -> Self {
        Self {
            width,
            height,
            rgba,
        }
    }

    /// Validates the texture's dimensions and buffer byte length.
    pub fn validate(&self) -> Result<(), TextureError> {
        if self.width == 0 || self.height == 0 {
            return Err(TextureError::ZeroDimension);
        }
        if self.width > 16384 || self.height > 16384 {
            return Err(TextureError::DimensionTooLarge(self.width, self.height));
        }
        let expected = (self.width as usize)
            .checked_mul(self.height as usize)
            .and_then(|px| px.checked_mul(4))
            .ok_or(TextureError::Overflow)?;
        if self.rgba.len() != expected {
            return Err(TextureError::ByteLengthMismatch {
                expected,
                actual: self.rgba.len(),
            });
        }
        Ok(())
    }

    /// True if the dimensions are non-zero, within limits, and match `rgba.len()`.
    pub fn is_valid(&self) -> bool {
        self.validate().is_ok()
    }

    /// Creates a solid white RGBA8 texture of the given dimensions.
    pub fn white(width: u32, height: u32) -> Self {
        let count = (width as usize)
            .saturating_mul(height as usize)
            .saturating_mul(4);
        Self {
            width,
            height,
            rgba: vec![255u8; count],
        }
    }

    /// Creates a standard 256x256 solid white preview texture (layer 0).
    pub fn default_white_preview() -> Self {
        Self::white(PREVIEW_SIZE, PREVIEW_SIZE)
    }

    /// Resamples or downsamples this image to 256x256 RGBA8.
    ///
    /// Returns solid white on invalid dimensions or byte lengths.
    pub fn resize_preview_256(&self) -> Self {
        resample_bilinear(self, PREVIEW_SIZE, PREVIEW_SIZE)
    }

    /// Resamples this image to arbitrary dimensions using bilinear interpolation.
    pub fn resample(&self, target_w: u32, target_h: u32) -> Self {
        resample_bilinear(self, target_w, target_h)
    }

    /// Full power-of-two mip chain: `[self]`, then each level half the edge,
    /// down to 1x1. Premultiplied-free RGBA8; count is `log2(max_edge) + 1`.
    pub fn mip_chain(&self) -> Vec<Self> {
        let mut chain = vec![self.clone()];
        loop {
            let level = chain.last().unwrap();
            if level.width == 1 && level.height == 1 {
                return chain;
            }
            let w = (level.width / 2).max(1);
            let h = (level.height / 2).max(1);
            chain.push(resample_bilinear(level, w, h));
        }
    }

    /// GPU memory estimate in bytes for a `layers`-layer array of `edge`
    /// square sRGB layers with a full mip chain (×4/3).
    pub fn array_memory_bytes(layers: u32, edge: u32) -> u64 {
        let mip_factor = 4.0f64 / 3.0;
        (u64::from(layers)
            .saturating_mul(u64::from(edge))
            .saturating_mul(u64::from(edge))
            .saturating_mul(4) as f64
            * mip_factor) as u64
    }
}

/// Resamples an RGBA8 image to `(target_w, target_h)` using bilinear area interpolation.
///
/// If `src` is malformed, safely returns a solid white image.
pub fn resample_bilinear(src: &TexturePixels, target_w: u32, target_h: u32) -> TexturePixels {
    if target_w == 0 || target_h == 0 || target_w > 4096 || target_h > 4096 {
        return TexturePixels::white(PREVIEW_SIZE, PREVIEW_SIZE);
    }
    if src.validate().is_err() {
        return TexturePixels::white(target_w, target_h);
    }
    if src.width == target_w && src.height == target_h {
        return src.clone();
    }

    let mut out = vec![0u8; (target_w as usize) * (target_h as usize) * 4];
    let w_ratio = src.width as f32 / target_w as f32;
    let h_ratio = src.height as f32 / target_h as f32;

    for y in 0..target_h {
        let src_y = ((y as f32 + 0.5) * h_ratio - 0.5)
            .max(0.0)
            .min((src.height - 1) as f32);
        let y0 = src_y.floor() as u32;
        let y1 = (y0 + 1).min(src.height - 1);
        let wy = src_y - y0 as f32;

        for x in 0..target_w {
            let src_x = ((x as f32 + 0.5) * w_ratio - 0.5)
                .max(0.0)
                .min((src.width - 1) as f32);
            let x0 = src_x.floor() as u32;
            let x1 = (x0 + 1).min(src.width - 1);
            let wx = src_x - x0 as f32;

            let idx00 = ((y0 * src.width + x0) * 4) as usize;
            let idx10 = ((y0 * src.width + x1) * 4) as usize;
            let idx01 = ((y1 * src.width + x0) * 4) as usize;
            let idx11 = ((y1 * src.width + x1) * 4) as usize;

            let dst_idx = ((y * target_w + x) * 4) as usize;

            for c in 0..4 {
                let v00 = src.rgba[idx00 + c] as f32;
                let v10 = src.rgba[idx10 + c] as f32;
                let v01 = src.rgba[idx01 + c] as f32;
                let v11 = src.rgba[idx11 + c] as f32;

                let top = v00 * (1.0 - wx) + v10 * wx;
                let bot = v01 * (1.0 - wx) + v11 * wx;
                let val = top * (1.0 - wy) + bot * wy;

                out[dst_idx + c] = val.round().clamp(0.0, 255.0) as u8;
            }
        }
    }

    TexturePixels {
        width: target_w,
        height: target_h,
        rgba: out,
    }
}

/// Derives planar local UVs from local position and dominant normal axis.
///
/// - X dominant (|nx| >= |ny| and |nx| >= |nz|): UV = (pos.y, pos.z)
/// - Y dominant (|ny| >= |nx| and |ny| >= |nz|): UV = (pos.x, pos.z)
/// - Z dominant (|nz| >= |nx| and |nz| >= |ny|): UV = (pos.x, pos.y)
pub fn planar_uv(position: [f32; 3], normal: [f32; 3]) -> [f32; 2] {
    let nx = normal[0].abs();
    let ny = normal[1].abs();
    let nz = normal[2].abs();
    if nx >= ny && nx >= nz {
        [position[1], position[2]]
    } else if ny >= nx && ny >= nz {
        [position[0], position[2]]
    } else {
        [position[0], position[1]]
    }
}

/// Packs a material table index and explicit material flag into `FaceVertex.material`.
#[inline]
pub fn pack_face_material(material_index: u32, explicit_material: bool) -> f32 {
    let bit = if explicit_material { 1u32 } else { 0u32 };
    (material_index
        .min(crate::materials::MAX_MATERIALS as u32 - 1)
        .saturating_mul(2)
        + bit) as f32
}

/// Decodes `FaceVertex.material` into `(material_index, explicit_material)`.
#[inline]
pub fn unpack_face_material(material: f32) -> (u32, bool) {
    let val = (material + 0.5) as u32;
    let index = val / 2;
    let explicit = !val.is_multiple_of(2);
    (index, explicit)
}

/// Prepares normalized 256x256 preview layers for GPU texture array creation.
///
/// - Layer 0 is always solid white 256x256.
/// - Layers 1..N correspond to up to 127 validated assets downsampled to 256x256.
/// - Malformed asset entries safely fall back to solid white 256x256.
pub fn prepare_preview_layers(assets: &[TexturePixels]) -> Vec<TexturePixels> {
    let clamped_count = assets.len().min(MAX_TEXTURE_LAYERS);
    let mut layers = Vec::with_capacity(1 + clamped_count);
    layers.push(TexturePixels::default_white_preview());

    for asset in assets.iter().take(clamped_count) {
        if asset.is_valid() {
            layers.push(asset.resize_preview_256());
        } else {
            layers.push(TexturePixels::default_white_preview());
        }
    }

    layers
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn texture_pixels_validation_and_errors() {
        let valid = TexturePixels::new(2, 2, vec![255; 16]);
        assert!(valid.is_valid());
        assert_eq!(valid.validate(), Ok(()));

        let zero_w = TexturePixels::new(0, 2, vec![255; 0]);
        assert_eq!(zero_w.validate(), Err(TextureError::ZeroDimension));
        assert!(!zero_w.is_valid());

        let zero_h = TexturePixels::new(2, 0, vec![255; 0]);
        assert_eq!(zero_h.validate(), Err(TextureError::ZeroDimension));

        let mismatch = TexturePixels::new(2, 2, vec![255; 15]);
        assert_eq!(
            mismatch.validate(),
            Err(TextureError::ByteLengthMismatch {
                expected: 16,
                actual: 15
            })
        );

        let too_large = TexturePixels::new(20000, 20000, vec![]);
        assert_eq!(
            too_large.validate(),
            Err(TextureError::DimensionTooLarge(20000, 20000))
        );
    }

    #[test]
    fn white_and_default_previews() {
        let white_4x4 = TexturePixels::white(4, 4);
        assert_eq!(white_4x4.width, 4);
        assert_eq!(white_4x4.height, 4);
        assert_eq!(white_4x4.rgba.len(), 64);
        assert!(white_4x4.rgba.iter().all(|&b| b == 255));
        assert!(white_4x4.is_valid());

        let preview = TexturePixels::default_white_preview();
        assert_eq!(preview.width, 256);
        assert_eq!(preview.height, 256);
        assert_eq!(preview.rgba.len(), 256 * 256 * 4);
        assert!(preview.rgba.iter().all(|&b| b == 255));
    }

    #[test]
    fn resample_preserves_constant_color() {
        let blue = [20, 40, 180, 200];
        let mut pixels = Vec::with_capacity(4 * 4 * 4);
        for _ in 0..16 {
            pixels.extend_from_slice(&blue);
        }
        let src = TexturePixels::new(4, 4, pixels);
        let resized = src.resize_preview_256();
        assert_eq!(resized.width, 256);
        assert_eq!(resized.height, 256);
        assert_eq!(resized.rgba.len(), 256 * 256 * 4);
        for chunk in resized.rgba.as_chunks::<4>().0 {
            assert_eq!(chunk, &blue);
        }
    }

    #[test]
    fn resample_downsamples_large_image() {
        let mut pixels = Vec::with_capacity(512 * 512 * 4);
        for _ in 0..(512 * 512) {
            pixels.extend_from_slice(&[10, 20, 30, 255]);
        }
        let large = TexturePixels::new(512, 512, pixels);
        let preview = large.resize_preview_256();
        assert_eq!(preview.width, 256);
        assert_eq!(preview.height, 256);
        assert!(preview
            .rgba
            .as_chunks::<4>()
            .0
            .iter()
            .all(|c| *c == [10, 20, 30, 255]));
    }

    #[test]
    fn resample_malformed_falls_back_to_white() {
        let malformed = TexturePixels::new(100, 100, vec![0; 50]);
        let preview = malformed.resize_preview_256();
        assert_eq!(preview.width, 256);
        assert_eq!(preview.height, 256);
        assert!(preview.rgba.iter().all(|&b| b == 255));
    }

    #[test]
    fn planar_uv_derivation_dominant_axes() {
        let pos = [1.5, 2.5, 3.5];

        // Dominant X: (1, 0, 0) or (-0.9, 0.2, 0.1) -> [pos.y, pos.z]
        assert_eq!(planar_uv(pos, [1.0, 0.0, 0.0]), [2.5, 3.5]);
        assert_eq!(planar_uv(pos, [-0.9, 0.2, 0.1]), [2.5, 3.5]);

        // Dominant Y: (0, 1, 0) or (0.3, -0.8, 0.2) -> [pos.x, pos.z]
        assert_eq!(planar_uv(pos, [0.0, 1.0, 0.0]), [1.5, 3.5]);
        assert_eq!(planar_uv(pos, [0.3, -0.8, 0.2]), [1.5, 3.5]);

        // Dominant Z: (0, 0, 1) or (0.1, 0.1, -0.99) -> [pos.x, pos.y]
        assert_eq!(planar_uv(pos, [0.0, 0.0, 1.0]), [1.5, 2.5]);
        assert_eq!(planar_uv(pos, [0.1, 0.1, -0.99]), [1.5, 2.5]);
    }

    #[test]
    fn pack_and_unpack_material_float() {
        // Test layer 0 (default) with both explicit states
        let p0_f = pack_face_material(0, false);
        assert_eq!(p0_f, 0.0);
        assert_eq!(unpack_face_material(p0_f), (0, false));

        let p0_t = pack_face_material(0, true);
        assert_eq!(p0_t, 1.0);
        assert_eq!(unpack_face_material(p0_t), (0, true));

        // Test arbitrary layers up to MAX_TEXTURE_LAYERS
        for layer in 0..=127 {
            let pf = pack_face_material(layer, false);
            assert_eq!(pf, (layer * 2) as f32);
            assert_eq!(unpack_face_material(pf), (layer, false));

            let pt = pack_face_material(layer, true);
            assert_eq!(pt, (layer * 2 + 1) as f32);
            assert_eq!(unpack_face_material(pt), (layer, true));
        }
    }

    #[test]
    fn prepare_preview_layers_clamps_and_handles_invalid() {
        let valid = TexturePixels::white(10, 10);
        let invalid = TexturePixels::new(10, 10, vec![]);

        let assets = vec![valid, invalid];
        let layers = prepare_preview_layers(&assets);
        assert_eq!(layers.len(), 3);
        // Layer 0 is white
        assert!(layers[0].rgba.iter().all(|&b| b == 255));
        // Layer 1 is valid downsampled to 256x256
        assert_eq!(layers[1].width, 256);
        assert_eq!(layers[1].height, 256);
        // Layer 2 fell back to white
        assert!(layers[2].rgba.iter().all(|&b| b == 255));

        // Test clamping to 127
        let many: Vec<_> = (0..200).map(|_| TexturePixels::white(2, 2)).collect();
        let clamped = prepare_preview_layers(&many);
        assert_eq!(clamped.len(), 128); // 1 white + 127 assets
    }
}
