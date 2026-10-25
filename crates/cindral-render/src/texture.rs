//! Texture pixel definitions, array sizing, quality resampling, validation, and planar UV derivation.

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

/// Maximum number of texture assets supported in the GPU texture array (layers 1..127).
pub const MAX_TEXTURE_LAYERS: usize = 127;

/// Layer index for the default solid white fallback texture.
pub const DEFAULT_WHITE_LAYER: u32 = 0;

/// 192 MiB working target for the whole array including mips (TEXTURE_DESIGN.md).
const TEXTURE_ARRAY_TARGET_BYTES: u64 = 192 * 1024 * 1024;

/// Candidate layer edges, largest first. One uniform edge is chosen so that
/// `(assets + 1)` layers with a full mip chain stay inside the target.
const TEXTURE_ARRAY_EDGES: [u32; 3] = [1024, 512, 256];

/// Layer edge length for `n` assets under the texture budget
/// (docs/TEXTURE_DESIGN.md). Falls back to the smallest edge if even that
/// exceeds the target.
pub fn texture_array_edge(asset_count: usize) -> u32 {
    let layers = (asset_count as u64).saturating_add(1); // + white fallback layer 0
    TEXTURE_ARRAY_EDGES
        .iter()
        .copied()
        .find(|&edge| {
            TexturePixels::array_memory_bytes(layers as u32, edge) <= TEXTURE_ARRAY_TARGET_BYTES
        })
        .unwrap_or(256)
}

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

/// One texture-array layer: the base image at the array edge plus its full
/// mip chain down to 1×1. Levels are straight-alpha RGBA8; every level below
/// the base is generated in premultiplied space and un-premultiplied
/// afterwards, so transparent texels cannot bleed dark colors into edges.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextureMips {
    /// Mip levels, base first, 1×1 last.
    pub levels: Vec<TexturePixels>,
}

impl TextureMips {
    /// Solid white layer with its (constant) mip chain.
    pub fn white(edge: u32) -> Self {
        Self {
            levels: mip_levels(&TexturePixels::white(edge, edge)),
        }
    }

    /// Resamples a decoded source directly to the array edge — never through
    /// a smaller preview — and generates the mip chain. Malformed sources
    /// fall back to solid white.
    pub fn from_source(edge: u32, source: &TexturePixels) -> Self {
        if source.validate().is_err() {
            return Self::white(edge);
        }
        let base = if source.width == edge && source.height == edge {
            source.clone()
        } else {
            resample_to_edge(source, edge)
        };
        Self {
            levels: mip_levels(&base),
        }
    }
}

/// Decoded-source adapter so callers can stream assets one at a time (and
/// tests can track buffer lifetimes) without retaining every decoded image.
pub trait TextureSource {
    /// The decoded pixels of one asset.
    fn pixels(&self) -> &TexturePixels;
}

impl TextureSource for TexturePixels {
    fn pixels(&self) -> &TexturePixels {
        self
    }
}

/// Upload payload for the GPU materials texture array: layer 0 is the solid
/// white fallback and layers 1..N are the decoded assets at the array edge.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextureArrayData {
    /// Square edge length of every layer and mip level.
    pub edge: u32,
    /// One entry per layer, each with a full mip chain (base first, 1×1 last).
    pub layers: Vec<TextureMips>,
}

/// Builds layer 0 (solid white) plus one mip-chain layer per asset.
///
/// `decoded` is consumed one asset at a time: each asset is resampled to
/// `edge` and reduced to a mip chain before the next is taken, so the peak
/// number of simultaneously-live decoded buffers is one. Decoding stops at
/// [`MAX_TEXTURE_LAYERS`] assets; the first decode error aborts the build.
pub fn build_texture_layers<S, E, I>(edge: u32, decoded: I) -> Result<Vec<TextureMips>, E>
where
    S: TextureSource,
    I: IntoIterator<Item = Result<S, E>>,
{
    let mut layers = vec![TextureMips::white(edge)];
    for asset in decoded {
        if layers.len() > MAX_TEXTURE_LAYERS {
            break;
        }
        layers.push(TextureMips::from_source(edge, asset?.pixels()));
    }
    Ok(layers)
}

/// Floating-point RGBA image in premultiplied alpha. All resampling happens
/// here so fully or partially transparent texels contribute no color.
struct Premultiplied {
    width: u32,
    height: u32,
    /// `width * height * 4` floats; RGB is already multiplied by alpha.
    texels: Vec<f32>,
}

fn to_premultiplied(src: &TexturePixels) -> Premultiplied {
    let mut texels = Vec::with_capacity(src.rgba.len() * 2);
    for chunk in src.rgba.as_chunks::<4>().0 {
        let alpha = chunk[3] as f32;
        let coverage = alpha / 255.0;
        texels.extend_from_slice(&[
            chunk[0] as f32 * coverage,
            chunk[1] as f32 * coverage,
            chunk[2] as f32 * coverage,
            alpha,
        ]);
    }
    Premultiplied {
        width: src.width,
        height: src.height,
        texels,
    }
}

fn to_straight(pm: &Premultiplied) -> TexturePixels {
    let mut rgba = Vec::with_capacity(pm.texels.len());
    for texel in pm.texels.as_chunks::<4>().0 {
        let alpha = texel[3];
        // Fully transparent texels keep black RGB (no color to recover).
        let scale = if alpha > 0.0 { 255.0 / alpha } else { 0.0 };
        for channel in &texel[..3] {
            rgba.push((channel * scale).round().clamp(0.0, 255.0) as u8);
        }
        rgba.push(alpha.round().clamp(0.0, 255.0) as u8);
    }
    TexturePixels {
        width: pm.width,
        height: pm.height,
        rgba,
    }
}

impl Premultiplied {
    fn sample(&self, x: u32, y: u32) -> [f32; 4] {
        let idx = ((y as usize * self.width as usize) + x as usize) * 4;
        [
            self.texels[idx],
            self.texels[idx + 1],
            self.texels[idx + 2],
            self.texels[idx + 3],
        ]
    }

    /// Box-filter resize: each destination texel averages exactly the source
    /// window it covers (weights per overlapping source texel). Upscaling
    /// windows narrower than one texel fall back to the nearest texel.
    /// Destination rows are independent, so large resamples spread rows
    /// across scoped threads.
    fn box_resize(&self, tw: u32, th: u32) -> Premultiplied {
        let mut out = vec![0.0f32; (tw as usize) * (th as usize) * 4];
        let row = tw as usize * 4;
        let workers = row_workers(th, row);
        std::thread::scope(|scope| {
            let mut cursor = &mut out[..];
            let mut y = 0u32;
            for _ in 0..workers {
                let rows = row_chunk(th, workers);
                let rows = rows.min(th - y);
                let take = rows as usize * row;
                let (head, tail) = cursor.split_at_mut(take);
                cursor = tail;
                let (y0, y1) = (y, y + rows);
                y = y1;
                scope.spawn(move || {
                    for yy in y0..y1 {
                        self.box_row(yy, tw, th, &mut head[((yy - y0) as usize) * row..][..row]);
                    }
                });
            }
        });
        Premultiplied {
            width: tw,
            height: th,
            texels: out,
        }
    }

    /// One destination row of [`Premultiplied::box_resize`].
    fn box_row(&self, y: u32, tw: u32, th: u32, out_row: &mut [f32]) {
        let sw = self.width as f32;
        let sh = self.height as f32;
        let ya = y as f32 * sh / th as f32;
        let yb = (y + 1) as f32 * sh / th as f32;
        for x in 0..tw {
            let xa = x as f32 * sw / tw as f32;
            let xb = (x + 1) as f32 * sw / tw as f32;
            let x0 = xa.floor() as u32;
            let x1 = xb.ceil().min(sw) as u32;
            let y0 = ya.floor() as u32;
            let y1 = yb.ceil().min(sh) as u32;
            let mut acc = [0.0f32; 4];
            let mut weight = 0.0f32;
            for sy in y0..y1 {
                let wy = (yb - sy as f32).clamp(0.0, 1.0) * (sy as f32 + 1.0 - ya).clamp(0.0, 1.0);
                if wy <= 0.0 {
                    continue;
                }
                for sx in x0..x1 {
                    let wx =
                        (xb - sx as f32).clamp(0.0, 1.0) * (sx as f32 + 1.0 - xa).clamp(0.0, 1.0);
                    if wx <= 0.0 {
                        continue;
                    }
                    let sample = self.sample(sx.min(self.width - 1), sy.min(self.height - 1));
                    let w = wx * wy;
                    for c in 0..4 {
                        acc[c] += sample[c] * w;
                    }
                    weight += w;
                }
            }
            let idx = (x as usize) * 4;
            if weight > 0.0 {
                for c in 0..4 {
                    out_row[idx + c] = acc[c] / weight;
                }
            } else {
                // Upscale gap: nearest source texel at the window center.
                let sx = ((x as f32 + 0.5) * sw / tw as f32) as u32;
                let sy = ((y as f32 + 0.5) * sh / th as f32) as u32;
                let sample = self.sample(sx.min(self.width - 1), sy.min(self.height - 1));
                out_row[idx..idx + 4].copy_from_slice(&sample);
            }
        }
    }

    /// Catmull-Rom bicubic resample (separable, clamped at the borders).
    fn catmull_rom(&self, tw: u32, th: u32) -> Premultiplied {
        let horizontal = cr_pass(self, tw, self.height, true);
        cr_pass(&horizontal, tw, th, false)
    }

    /// Halves both dimensions (minimum 1) with exact box averaging.
    fn halved(&self) -> Premultiplied {
        self.box_resize((self.width / 2).max(1), (self.height / 2).max(1))
    }
}

/// One axis of the Catmull-Rom filter; `along_x` selects the source axis.
/// Destination rows are independent, so large passes spread rows across
/// scoped threads.
fn cr_pass(src: &Premultiplied, tw: u32, th: u32, along_x: bool) -> Premultiplied {
    let n_src = if along_x { src.width } else { src.height };
    let n_dst = if along_x { tw } else { th };
    let ratio = n_src as f32 / n_dst as f32;
    let row = tw as usize * 4;
    let mut out = vec![0.0f32; th as usize * row];
    let workers = row_workers(th, row);
    std::thread::scope(|scope| {
        let mut cursor = &mut out[..];
        let mut y = 0u32;
        for _ in 0..workers {
            let chunk = row_chunk(th, workers).min(th - y);
            let take = chunk as usize * row;
            let (head, tail) = cursor.split_at_mut(take);
            cursor = tail;
            let (y0, y1) = (y, y + chunk);
            y = y1;
            scope.spawn(move || {
                let pass = CrPass {
                    src,
                    along_x,
                    ratio,
                    n_src,
                };
                for yy in y0..y1 {
                    pass.row(yy, tw, &mut head[((yy - y0) as usize) * row..][..row]);
                }
            });
        }
    });
    Premultiplied {
        width: tw,
        height: th,
        texels: out,
    }
}

/// Catmull-Rom pass parameters plus the destination-row kernel.
struct CrPass<'a> {
    src: &'a Premultiplied,
    along_x: bool,
    /// Filter position along the source axis: output X when `along_x`,
    /// output Y otherwise.
    ratio: f32,
    n_src: u32,
}

impl CrPass<'_> {
    /// Computes one destination row of the pass. The vertical pass filters
    /// each texel independently in Y, so destination rows stay independent
    /// and contiguous in both orientations.
    fn row(&self, y: u32, tw: u32, out_row: &mut [f32]) {
        for x in 0..tw {
            let center = ((if self.along_x { x } else { y }) as f32 + 0.5) * self.ratio - 0.5;
            let base = center.floor();
            let frac = center - base;
            let base = base as i32;
            let w = cr_weights(frac);
            let mut acc = [0.0f32; 4];
            for (k, weight) in w.iter().enumerate() {
                let i = (base + k as i32 - 1).clamp(0, self.n_src as i32 - 1) as u32;
                let sample = if self.along_x {
                    self.src.sample(i, y)
                } else {
                    self.src.sample(x, i)
                };
                for c in 0..4 {
                    acc[c] += sample[c] * weight;
                }
            }
            let idx = (x as usize) * 4;
            out_row[idx..idx + 4].copy_from_slice(&acc);
        }
    }
}

/// Catmull-Rom kernel weights for the four surrounding texels.
fn cr_weights(f: f32) -> [f32; 4] {
    let f2 = f * f;
    let f3 = f2 * f;
    [
        -0.5 * f3 + f2 - 0.5 * f,
        1.5 * f3 - 2.5 * f2 + 1.0,
        -1.5 * f3 + 2.0 * f2 + 0.5 * f,
        0.5 * f3 - 0.5 * f2,
    ]
}

/// Number of worker threads for a row-parallel pass; single-threaded below a
/// useful work threshold (rows or bytes too small to amortize spawning).
fn row_workers(rows: u32, row_bytes: usize) -> u32 {
    let available = std::thread::available_parallelism()
        .map(|n| n.get() as u32)
        .unwrap_or(1);
    let useful = (rows as usize * row_bytes) / (64 * 1024);
    available.min(useful.max(1) as u32).clamp(1, available)
}

/// Rows per worker, roughly even; callers clamp to the remaining rows.
fn row_chunk(rows: u32, workers: u32) -> u32 {
    rows.div_ceil(workers.max(1))
}

/// Resamples a decoded source directly to the square array edge.
///
/// Large downscales are prefiltered with a box down to at most 2× the target
/// so the Catmull-Rom kernel cannot alias fine detail; the final step is
/// bicubic. All math runs in premultiplied space, then the result is
/// un-premultiplied so straight-alpha texels keep their hue.
fn resample_to_edge(src: &TexturePixels, edge: u32) -> TexturePixels {
    let mut pm = to_premultiplied(src);
    if pm.width > 2 * edge || pm.height > 2 * edge {
        let pw = (2 * edge).min(pm.width);
        let ph = (2 * edge).min(pm.height);
        pm = pm.box_resize(pw, ph);
    }
    pm = pm.catmull_rom(edge, edge);
    to_straight(&pm)
}

/// Full mip chain of a straight-alpha base: `[base]`, then each level half
/// the edge, down to 1×1. Every level below the base is averaged in
/// premultiplied space and un-premultiplied afterwards.
fn mip_levels(base: &TexturePixels) -> Vec<TexturePixels> {
    let mut levels = vec![base.clone()];
    let mut pm = to_premultiplied(base);
    while pm.width > 1 || pm.height > 1 {
        pm = pm.halved();
        levels.push(to_straight(&pm));
    }
    levels
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::rc::Rc;

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
    fn white_layers_and_defaults() {
        let white_4x4 = TexturePixels::white(4, 4);
        assert_eq!(white_4x4.width, 4);
        assert_eq!(white_4x4.height, 4);
        assert_eq!(white_4x4.rgba.len(), 64);
        assert!(white_4x4.rgba.iter().all(|&b| b == 255));
        assert!(white_4x4.is_valid());

        let layer = TextureMips::white(256);
        assert_eq!(layer.levels.len(), 9); // 256, 128, ..., 1
        assert_eq!(layer.levels[0].width, 256);
        assert!(layer
            .levels
            .iter()
            .all(|l| l.rgba.iter().all(|&b| b == 255)));
    }

    #[test]
    fn resample_preserves_constant_color() {
        let blue = [20, 40, 180, 200];
        let mut pixels = Vec::with_capacity(4 * 4 * 4);
        for _ in 0..16 {
            pixels.extend_from_slice(&blue);
        }
        let src = TexturePixels::new(4, 4, pixels);
        let layer = TextureMips::from_source(256, &src);
        assert_eq!(layer.levels[0].width, 256);
        assert_eq!(layer.levels[0].height, 256);
        assert_eq!(layer.levels[0].rgba.len(), 256 * 256 * 4);
        for chunk in layer.levels[0].rgba.as_chunks::<4>().0 {
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
        let layer = TextureMips::from_source(256, &large);
        assert_eq!(layer.levels[0].width, 256);
        assert_eq!(layer.levels[0].height, 256);
        assert!(layer.levels[0]
            .rgba
            .as_chunks::<4>()
            .0
            .iter()
            .all(|c| *c == [10, 20, 30, 255]));
    }

    #[test]
    fn resample_malformed_falls_back_to_white() {
        let malformed = TexturePixels::new(100, 100, vec![0; 50]);
        let layer = TextureMips::from_source(256, &malformed);
        assert_eq!(layer.levels.len(), 9);
        assert!(layer
            .levels
            .iter()
            .all(|l| l.rgba.iter().all(|&b| b == 255)));
    }

    /// A 1-pixel-stripe texture keeps alternating detail when resampled
    /// directly to the 1024 array edge, while the old path (downsample to a
    /// 256 preview first, then upscale) washes the stripes into flat gray.
    #[test]
    fn stripes_keep_more_detail_at_1024_than_the_old_256_path() {
        // 1024x1024 alternating black/white 1px columns: exactly the array
        // edge, so the direct path can keep every stripe.
        let (w, h) = (1024u32, 64u32);
        let mut rgba = vec![0u8; (w * h * 4) as usize];
        for y in 0..h {
            for x in 0..w {
                let v = if x % 2 == 0 { 255u8 } else { 0u8 };
                let idx = ((y * w + x) * 4) as usize;
                rgba[idx..idx + 4].copy_from_slice(&[v, v, v, 255]);
            }
        }
        let src = TexturePixels::new(w, h, rgba);

        let new_base = TextureMips::from_source(1024, &src).levels.remove(0);
        // Old path: bilinear-free preview downsample would first shrink to
        // 256; approximate its information loss by box-filtering to 256 and
        // resampling back up to 1024 through the same quality pipeline.
        let old_base = {
            let mut pm = to_premultiplied(&src);
            // The old path downsampled every texture to a 256 preview before
            // the array-edge resample; four stripe columns merge per pixel.
            pm = pm.box_resize(256, 256);
            pm = pm.catmull_rom(1024, 1024);
            to_straight(&pm)
        };

        let contrast = |tex: &TexturePixels, y: usize| {
            let row = &tex.rgba[y * tex.width as usize * 4..][..tex.width as usize * 4];
            let lum = |x: usize| row[x * 4] as usize;
            (0..tex.width as usize - 1)
                .map(|x| lum(x).abs_diff(lum(x + 1)))
                .sum::<usize>()
        };
        // Direct resample keeps strongly alternating stripes on many columns.
        assert!(
            contrast(&new_base, 0) > 100_000,
            "1px stripes must survive direct resampling, got {}",
            contrast(&new_base, 0)
        );
        // The 256-preview path washes them into near-flat gray.
        assert!(
            contrast(&old_base, 0) < 10_000,
            "256-preview path must lose the stripes, got {}",
            contrast(&old_base, 0)
        );
    }

    /// A tracking source whose drop marks the slot free; the builder must
    /// release each decoded buffer before pulling the next asset.
    struct Tracked {
        tex: TexturePixels,
        live: Rc<Cell<usize>>,
    }

    impl TextureSource for Tracked {
        fn pixels(&self) -> &TexturePixels {
            &self.tex
        }
    }

    impl Drop for Tracked {
        fn drop(&mut self) {
            self.live.set(self.live.get() - 1);
        }
    }

    #[test]
    fn peak_simultaneously_live_decoded_buffers_is_one_for_127_assets() {
        let live = Rc::new(Cell::new(0usize));
        let peak = Rc::new(Cell::new(0usize));
        // Lazy map: each Tracked is constructed only when the builder pulls
        // it, so `live` reflects exactly how many decoded buffers the
        // builder holds at once; `peak` records the maximum.
        let peak_cell = Rc::clone(&peak);
        let live_cell = Rc::clone(&live);
        let decoded = (0..127).map(move |_| {
            let tex = TexturePixels::white(512, 512);
            live_cell.set(live_cell.get() + 1);
            peak_cell.set(peak_cell.get().max(live_cell.get()));
            Ok::<Tracked, std::convert::Infallible>(Tracked {
                tex,
                live: Rc::clone(&live_cell),
            })
        });
        let layers = build_texture_layers(1024, decoded).expect("no decode errors");
        assert_eq!(layers.len(), 128); // white + 127 assets
        assert_eq!(live.get(), 0, "every decoded buffer must be dropped");
        assert_eq!(peak.get(), 1, "at most one decoded buffer may be alive");
    }

    /// Fully transparent texels must not darken semi-transparent edges after
    /// mip generation: a half-covered red pixel stays red (255, 0, 0), not a
    /// dark blend with the black transparent neighbor.
    #[test]
    fn transparent_edges_keep_hue_after_mips() {
        // 8x8: left half opaque red, right half fully transparent black.
        let (w, h) = (8u32, 8u32);
        let mut rgba = vec![0u8; (w * h * 4) as usize];
        for y in 0..h {
            for x in 0..w {
                let idx = ((y * w + x) * 4) as usize;
                if x < 4 {
                    rgba[idx..idx + 4].copy_from_slice(&[255, 0, 0, 255]);
                }
            }
        }
        let src = TexturePixels::new(w, h, rgba);
        let levels = TextureMips::from_source(8, &src).levels;
        // Mip 1 (4x4): column 1 is the 50% coverage boundary texel.
        let mip1 = &levels[1];
        let boundary = &mip1.rgba[4..8];
        assert_eq!(boundary[0], 255, "red channel stays saturated");
        assert!(
            boundary[1] <= 64 && boundary[2] <= 64,
            "no dark halo: got rgb {:?}",
            &boundary[..3]
        );
    }

    /// Odd and non-power-of-two sources still produce a complete chain to 1×1.
    #[test]
    fn odd_and_non_power_of_two_sizes_mip_to_1x1() {
        let src = TexturePixels::new(3, 5, vec![90; 3 * 5 * 4]);
        let levels = TextureMips::from_source(8, &src).levels;
        let last = levels.last().unwrap();
        assert_eq!((last.width, last.height), (1, 1));
        assert_eq!(levels.len(), 4); // 8, 4, 2, 1
        let src = TexturePixels::new(6, 6, vec![90; 6 * 6 * 4]);
        let levels = TextureMips::from_source(6, &src).levels;
        assert_eq!((levels[0].width, levels[0].height), (6, 6));
        assert_eq!(
            (levels.last().unwrap().width, levels.last().unwrap().height),
            (1, 1)
        );
    }

    #[test]
    fn texture_array_edge_fits_the_budget() {
        // One 1024² layer with mips = 16/3 MiB = 5.33 MiB: 33 assets + white
        // = 34 layers = 181.3 MiB, safely inside the 192 MiB target.
        assert_eq!(texture_array_edge(33), 1024);
        // 38 layers (37 assets) = 202.7 MiB > 192 MiB → 512.
        assert_eq!(texture_array_edge(37), 512);
        // 128 layers at 512² = 170.67 MiB ≤ 192 MiB.
        assert_eq!(texture_array_edge(127), 512);
        assert_eq!(texture_array_edge(0), 1024);
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
}
