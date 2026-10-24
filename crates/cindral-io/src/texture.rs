//! Bounded PNG/JPEG decoding; encoded images remain separate from runtime pixels.
use crate::{DecodedTexture, IoError, Result, TextureAsset};
use sha2::{Digest, Sha256};
use std::{
    io::{Cursor, Read},
    path::Path,
};
/// Maximum size of one encoded image (32 MiB).
pub const MAX_TEXTURE_BYTES: usize = 32 * 1024 * 1024;
/// Maximum decoded texture edge in pixels.
pub const MAX_TEXTURE_DIMENSION: u32 = 8192;
/// Maximum decoded bytes for one image (256 MiB).
pub const MAX_TEXTURE_DECODED: usize = 256 * 1024 * 1024;
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
    limits.max_image_width = Some(MAX_TEXTURE_DIMENSION);
    limits.max_image_height = Some(MAX_TEXTURE_DIMENSION);
    limits.max_alloc = Some(MAX_TEXTURE_DECODED as u64);
    reader.limits(limits);
    let image = reader
        .decode()
        .map_err(|e| IoError::Invalid(format!("texture decoding failed: {e}")))?;
    let (width, height) = (image.width(), image.height());
    if width == 0 || height == 0 || width > MAX_TEXTURE_DIMENSION || height > MAX_TEXTURE_DIMENSION
    {
        return Err(IoError::Invalid(format!(
            "texture dimensions {width}x{height} exceed the 8192x8192 limit"
        )));
    }
    let rgba = image.to_rgba8();
    let decoded = width as usize * height as usize * 4;
    if decoded > MAX_TEXTURE_DECODED {
        return Err(IoError::Invalid(format!(
            "texture decodes to {decoded} bytes, over the 256 MiB limit"
        )));
    }
    Ok(DecodedTexture {
        width,
        height,
        rgba: rgba.into_raw(),
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

#[cfg(test)]
mod phase2_tests {
    use super::*;
    use std::io::Cursor;

    fn png(width: u32, height: u32) -> Vec<u8> {
        let image = image::RgbaImage::from_pixel(width, height, image::Rgba([90, 120, 160, 255]));
        let mut bytes = Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(image)
            .write_to(&mut bytes, image::ImageFormat::Png)
            .unwrap();
        bytes.into_inner()
    }

    #[test]
    fn oversized_and_malformed_images_are_rejected_without_panicking() {
        // Truncated header of a huge declared PNG: the decoder rejects it.
        let mut header = b"\x89PNG\r\n\x1a\n".to_vec();
        header.extend_from_slice(&[0; 64]);
        header[16] = 0;
        header[20] = 0x20; // corrupt chunk length
        assert!(texture_from_bytes(header).is_err());

        // Truncated valid PNG bytes.
        let full = png(4, 4);
        for cut in [8usize, 20, 33] {
            let truncated = full[..full.len() - cut].to_vec();
            assert!(
                texture_from_bytes(truncated).is_err(),
                "truncated by {cut} bytes must be rejected"
            );
        }

        // Bit-flipped valid PNGs: either rejected or still decodable, never a panic.
        let full = png(8, 8);
        for i in (0..full.len()).step_by(7) {
            let mut mutated = full.clone();
            mutated[i] ^= 0x80;
            let _ = texture_from_bytes(mutated);
        }
        // Random junk.
        assert!(texture_from_bytes(vec![0u8; 1024]).is_err());
        assert!(texture_from_bytes(Vec::new()).is_err());
    }

    #[test]
    fn dimension_cap_rejects_declared_8193_pixel_images() {
        // A PNG header declaring 8193x2 pixels without pixel data is rejected
        // by the reader limits, not by a panic or an allocation.
        let mut header = b"\x89PNG\r\n\x1a\n".to_vec();
        header.extend_from_slice(&[0, 0, 0, 13, b'I', b'H', b'D', b'R']);
        header.extend_from_slice(&8193u32.to_be_bytes());
        header.extend_from_slice(&2u32.to_be_bytes());
        header.extend_from_slice(&[8, 6, 0, 0, 0]);
        assert!(texture_from_bytes(header.clone()).is_err());
        let mut within = header;
        within.clear();
        within.extend_from_slice(&png(2, 2));
        assert!(texture_from_bytes(within).is_ok());
    }

    #[test]
    fn identical_images_deduplicate_by_content_hash() {
        let bytes = png(3, 3);
        let (key_a, _) = texture_from_bytes(bytes.clone()).unwrap();
        let (key_b, _) = texture_from_bytes(bytes).unwrap();
        assert_eq!(key_a, key_b);
        let (key_c, _) = texture_from_bytes(png(3, 4)).unwrap();
        assert_ne!(key_a, key_c);
    }
}
