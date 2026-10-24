//! GPU material parameter table shared by every shaded face.
//!
//! Materials live in one small `Rgba32Float` texture (three texels per
//! material) instead of being baked into vertex bytes, so editing a material
//! rewrites one record and never touches geometry buffers. `FaceVertex`
//! carries a packed material index; index 0 means "no material record" and
//! keeps the legacy vertex-baked color path readable.

/// Upper bound on material table records; the table texture is 3 texels high
/// per material and GPU texture dimensions cap at 16384.
pub const MAX_MATERIALS: usize = 16384;

/// Texel columns occupied by one material record.
pub const MATERIAL_TEXELS: u32 = 3;

/// One material record uploaded to the GPU table.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MaterialParams {
    /// Linear-light RGBA tint and opacity.
    pub color: [f32; 4],
    /// Texture array layer (0 = white fallback).
    pub texture_layer: u32,
    /// Texture repeat scale: 1/width_m and 1/height_m (default 1,1 = one
    /// repeat per meter, the pre-table planar mapping).
    pub uv_scale: [f32; 2],
    /// Texture rotation in radians about the mapping origin.
    pub rotation: f32,
    /// Horizontal/vertical flip multipliers (±1).
    pub flip: [f32; 2],
}

impl Default for MaterialParams {
    fn default() -> Self {
        Self {
            color: [1., 1., 1., 1.],
            texture_layer: 0,
            uv_scale: [1., 1.],
            rotation: 0.,
            flip: [1., 1.],
        }
    }
}

/// CPU-side material table; entry 0 is the shared default record.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MaterialTable {
    entries: Vec<MaterialParams>,
}

impl MaterialTable {
    /// New table containing only the default entry at index 0.
    pub fn new() -> Self {
        Self {
            entries: vec![MaterialParams::default()],
        }
    }

    /// Number of records including the default entry.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True while only the implicit default entry exists.
    pub fn is_empty(&self) -> bool {
        self.entries.len() <= 1
    }

    /// Appends a record and returns its 1-based GPU index.
    ///
    /// Extra records beyond [`MAX_MATERIALS`] reuse the final index so the
    /// packed vertex index always resolves to a valid texel row.
    pub fn push(&mut self, params: MaterialParams) -> u32 {
        if self.entries.len() >= MAX_MATERIALS {
            return MAX_MATERIALS as u32 - 1;
        }
        self.entries.push(params);
        (self.entries.len() - 1) as u32
    }

    /// Record at a GPU index (0 = default).
    pub fn get(&self, index: u32) -> MaterialParams {
        self.entries
            .get(index as usize)
            .copied()
            .unwrap_or_default()
    }

    /// Minimum texture height that fits the current records.
    pub fn required_height(&self) -> u32 {
        self.entries.len().max(1) as u32
    }

    /// Encodes the table as `Rgba32Float` bytes: `3 x len` texels, row-major.
    ///
    /// Texel 0 is the linear RGBA color, texel 1 holds the texture layer,
    /// UV scale and rotation, texel 2 the flip multipliers.
    pub fn upload_bytes(&self) -> Vec<u8> {
        let height = self.required_height();
        let mut bytes = Vec::with_capacity(height as usize * MATERIAL_TEXELS as usize * 16);
        for y in 0..height {
            let entry = self.get(y);
            let mut push = |v: [f32; 4]| {
                for channel in v {
                    bytes.extend_from_slice(&channel.to_ne_bytes());
                }
            };
            push(entry.color);
            push([
                entry.texture_layer as f32,
                entry.uv_scale[0],
                entry.uv_scale[1],
                entry.rotation,
            ]);
            push([entry.flip[0], entry.flip[1], 0., 0.]);
        }
        bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_table_has_one_default_record() {
        let table = MaterialTable::new();
        assert_eq!(table.len(), 1);
        assert_eq!(table.get(0), MaterialParams::default());
        assert_eq!(table.get(99), MaterialParams::default());
        assert!(table.is_empty());
    }

    #[test]
    fn push_returns_one_based_indices() {
        let mut table = MaterialTable::new();
        let a = MaterialParams {
            color: [0.1, 0.2, 0.3, 0.4],
            texture_layer: 5,
            uv_scale: [2., 0.5],
            rotation: 0.7,
            flip: [-1., 1.],
        };
        assert_eq!(table.push(a), 1);
        assert_eq!(table.push(MaterialParams::default()), 2);
        assert_eq!(table.get(1), a);
        assert_eq!(table.len(), 3);
        assert!(!table.is_empty());
    }

    #[test]
    fn upload_bytes_layout_round_trips() {
        let mut table = MaterialTable::new();
        table.push(MaterialParams {
            color: [0.25, 0.5, 0.75, 0.5],
            texture_layer: 7,
            uv_scale: [4., 0.25],
            rotation: -1.5,
            flip: [1., -1.],
        });
        let bytes = table.upload_bytes();
        assert_eq!(bytes.len(), 2 * MATERIAL_TEXELS as usize * 16);
        let f = |x: u32, y: u32, c: usize| {
            let offset = (y as usize * MATERIAL_TEXELS as usize + x as usize) * 16 + c * 4;
            f32::from_ne_bytes(bytes[offset..offset + 4].try_into().unwrap())
        };
        assert_eq!((f(0, 0, 0), f(0, 0, 3)), (1., 1.));
        assert_eq!((f(0, 1, 0), f(0, 1, 1), f(0, 1, 3)), (0.25, 0.5, 0.5));
        assert_eq!((f(1, 1, 0), f(1, 1, 1), f(1, 1, 2)), (7., 4., 0.25));
        assert_eq!(f(1, 1, 3), -1.5);
        assert_eq!((f(2, 1, 0), f(2, 1, 1)), (1., -1.));
    }

    #[test]
    fn oversized_tables_clamp_to_the_final_index() {
        let mut table = MaterialTable::new();
        table.entries = vec![MaterialParams::default(); MAX_MATERIALS];
        let index = table.push(MaterialParams::default());
        assert_eq!(index, MAX_MATERIALS as u32 - 1);
        assert_eq!(table.len(), MAX_MATERIALS);
        assert_eq!(table.required_height() as usize, MAX_MATERIALS);
    }
}
