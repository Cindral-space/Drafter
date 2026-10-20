//! Bounded PNG/JPEG decoding; encoded images remain separate from runtime pixels.
use crate::{DecodedTexture, IoError, Result, TextureAsset};
use sha2::{Digest, Sha256};
use std::{
    io::{Cursor, Read},
    path::Path,
};
/// Maximum size of one encoded image (32 MiB).
pub const MAX_TEXTURE_BYTES: usize = 32 * 1024 * 1024;
/// Decode a validated original image into bounded runtime sRGB RGBA pixels.
pub fn decode_texture(asset: &TextureAsset) -> Result<DecodedTexture> {
    if asset.bytes.len() > MAX_TEXTURE_BYTES {
        return Err(IoError::Invalid("texture exceeds 32 MiB".into()));
    }
    let format = image::guess_format(&asset.bytes).map_err(|e| IoError::Invalid(e.to_string()))?;
    let mime = match format {
        image::ImageFormat::Png => "image/png",
        image::ImageFormat::Jpeg => "image/jpeg",
        _ => {
            return Err(IoError::Invalid(
                "only PNG and JPEG textures are supported".into(),
            ))
        }
    };
    if asset.mime != mime {
        return Err(IoError::Invalid(
            "texture MIME type disagrees with its bytes".into(),
        ));
    }
    let mut reader = image::ImageReader::with_format(Cursor::new(&asset.bytes), format);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(4096);
    limits.max_image_height = Some(4096);
    limits.max_alloc = Some(128 * 1024 * 1024);
    reader.limits(limits);
    let image = reader
        .decode()
        .map_err(|e| IoError::Invalid(e.to_string()))?
        .to_rgba8();
    Ok(DecodedTexture {
        width: image.width(),
        height: image.height(),
        rgba: image.into_raw(),
    })
}
/// Validate original PNG/JPEG bytes and return a stable content-addressed material slot.
pub fn texture_from_bytes(bytes: Vec<u8>) -> Result<(String, TextureAsset)> {
    let format = image::guess_format(&bytes).map_err(|e| IoError::Invalid(e.to_string()))?;
    let mime = match format {
        image::ImageFormat::Png => "image/png",
        image::ImageFormat::Jpeg => "image/jpeg",
        _ => {
            return Err(IoError::Invalid(
                "only PNG and JPEG textures are supported".into(),
            ))
        }
    };
    let asset = TextureAsset {
        mime: mime.into(),
        bytes,
    };
    decode_texture(&asset)?;
    let key = Sha256::digest(&asset.bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    Ok((key, asset))
}
/// Load an image explicitly chosen by the user, with a bounded file read.
pub fn load_texture(path: &Path) -> Result<(String, TextureAsset)> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take(MAX_TEXTURE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_TEXTURE_BYTES {
        return Err(IoError::Invalid("texture exceeds 32 MiB".into()));
    }
    texture_from_bytes(bytes)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn malformed_and_mime_mismatch_are_rejected() {
        assert!(texture_from_bytes(vec![1, 2, 3]).is_err());
        let image = image::RgbaImage::from_pixel(2, 2, image::Rgba([30, 60, 90, 128]));
        let mut bytes = Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(image)
            .write_to(&mut bytes, image::ImageFormat::Png)
            .unwrap();
        let (key, mut asset) = texture_from_bytes(bytes.into_inner()).unwrap();
        assert_eq!(key.len(), 64);
        let decoded = decode_texture(&asset).unwrap();
        assert_eq!((decoded.width, decoded.height), (2, 2));
        assert_eq!(decoded.rgba[3], 128);
        asset.mime = "image/jpeg".into();
        assert!(decode_texture(&asset).is_err());
    }
}
