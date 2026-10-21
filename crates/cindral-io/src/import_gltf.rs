//! Bounded glTF 2.0 (`.gltf` and `.glb`) parser producing [`ImportedScene`].

use crate::{
    import_scene::{
        read_asset, read_bounded, ImportControl, ImportedDefinition, ImportedInstance,
        ImportedScene, UpAxis,
    },
    texture_from_bytes, IoError, Result, TextureAsset,
};
use base64::prelude::*;
use cindral_core::{
    BulkFace, BulkGeometry, DVec3, Material, MaterialId, MAX_COORDINATE, TOLERANCE,
};
use glam::{DMat4, DQuat};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    path::Path,
};

macro_rules! import_warning {
    ($warnings:expr, $value:expr) => {
        if $warnings.len() < 1000 {
            $warnings.push($value);
        }
    };
}

const MAX_ASSET_BYTES: usize = 64 * 1024 * 1024;
const MAX_VERTICES: usize = 1_000_000;
const MAX_TRIANGLES: usize = 1_000_000;
const MAX_NODES: usize = 100_000;
const MAX_DEPTH: usize = 64;
const MAX_TEXTURES: usize = 127;

const GLB_MAGIC: u32 = 0x4654_6C67; // "glTF"
const GLB_CHUNK_JSON: u32 = 0x4E4F_534A; // "JSON"
const GLB_CHUNK_BIN: u32 = 0x004E_4942; // "BIN\0"

/// Parse a `.gltf` or `.glb` file from disk within bounded size and resource limits.
pub fn import_gltf(path: &Path, control: &ImportControl) -> Result<ImportedScene> {
    control.check()?;
    let bytes = read_bounded(path, MAX_ASSET_BYTES)?;
    let base_dir = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .or(Some(Path::new(".")));
    import_gltf_bytes(&bytes, base_dir, control)
}

/// Parse `.gltf` JSON or `.glb` binary container bytes into an [`ImportedScene`].
pub fn import_gltf_bytes(
    bytes: &[u8],
    base_dir: Option<&Path>,
    control: &ImportControl,
) -> Result<ImportedScene> {
    control.check()?;
    if bytes.is_empty() {
        return Err(IoError::Invalid("empty glTF input".into()));
    }
    if bytes.len() > MAX_ASSET_BYTES {
        return Err(IoError::Invalid("glTF exceeds 64 MiB limit".into()));
    }

    let mut total_bytes = bytes.len();
    let (json_slice, bin_chunk) = if bytes.len() >= 4
        && u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) == GLB_MAGIC
    {
        parse_glb_container(bytes)?
    } else {
        let stripped = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
        (stripped, None)
    };

    let root: Value = serde_json::from_slice(json_slice)
        .map_err(|e| IoError::Invalid(format!("invalid glTF JSON: {e}")))?;
    let root_obj = root
        .as_object()
        .ok_or_else(|| IoError::Invalid("glTF root must be a JSON object".into()))?;

    validate_gltf_asset(root_obj)?;

    let mut warnings = Vec::new();
    check_extensions(root_obj, &mut warnings)?;

    for unsupported_key in ["animations", "skins", "cameras", "morphTargets"] {
        if root_obj
            .get(unsupported_key)
            .and_then(Value::as_array)
            .is_some_and(|a| !a.is_empty())
        {
            import_warning!(
                warnings,
                format!("unsupported glTF feature ignored: {unsupported_key}")
            );
        }
    }

    control.check()?;
    control.set_progress(0.1);

    let buffers = load_buffers(root_obj, bin_chunk, base_dir, &mut total_bytes, control)?;
    let buffer_views = parse_buffer_views(root_obj, &buffers)?;
    let accessors = parse_accessors(root_obj, &buffer_views)?;

    control.check()?;
    control.set_progress(0.2);

    let mut textures: BTreeMap<String, TextureAsset> = BTreeMap::new();
    let materials = parse_materials_and_textures(
        root_obj,
        &buffers,
        &buffer_views,
        base_dir,
        &mut total_bytes,
        &mut textures,
        &mut warnings,
        control,
    )?;

    control.check()?;
    control.set_progress(0.3);

    let mut total_vertices = 0usize;
    let mut total_triangles = 0usize;
    let parsed_meshes = parse_meshes(
        root_obj,
        &buffers,
        &buffer_views,
        &accessors,
        &materials,
        &mut total_vertices,
        &mut total_triangles,
        &mut warnings,
        control,
    )?;

    control.check()?;
    control.set_progress(0.4);

    let mut scene = ImportedScene {
        definitions: Vec::new(),
        roots: Vec::new(),
        materials,
        textures,
        tags: Vec::new(),
        meters_per_unit: 1.0,
        up_axis: UpAxis::Y,
        skipped: parsed_meshes.iter().map(|m| m.skipped).sum(),
        warnings,
    };

    build_node_hierarchy(root_obj, &parsed_meshes, &mut scene, control)?;
    Ok(scene)
}

fn parse_glb_container(bytes: &[u8]) -> Result<(&[u8], Option<&[u8]>)> {
    if bytes.len() < 12 {
        return Err(IoError::Invalid("truncated GLB header".into()));
    }
    let version = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
    if version != 2 {
        return Err(IoError::Invalid(format!(
            "unsupported GLB version {version}"
        )));
    }
    let total_len = u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]) as usize;
    if total_len < 12 || total_len > bytes.len() || total_len > MAX_ASSET_BYTES {
        return Err(IoError::Invalid("invalid GLB byte length".into()));
    }

    let mut offset = 12usize;
    let mut json_chunk: Option<&[u8]> = None;
    let mut bin_chunk: Option<&[u8]> = None;
    let mut chunk_index = 0usize;

    while offset < total_len {
        let header_end = offset
            .checked_add(8)
            .ok_or_else(|| IoError::Invalid("GLB chunk offset overflow".into()))?;
        if header_end > total_len {
            return Err(IoError::Invalid("truncated GLB chunk header".into()));
        }
        let chunk_len = u32::from_le_bytes([
            bytes[offset],
            bytes[offset + 1],
            bytes[offset + 2],
            bytes[offset + 3],
        ]) as usize;
        let chunk_type = u32::from_le_bytes([
            bytes[offset + 4],
            bytes[offset + 5],
            bytes[offset + 6],
            bytes[offset + 7],
        ]);
        let chunk_end = header_end
            .checked_add(chunk_len)
            .ok_or_else(|| IoError::Invalid("GLB chunk length overflow".into()))?;
        if chunk_end > total_len {
            return Err(IoError::Invalid("GLB chunk exceeds container size".into()));
        }
        let payload = &bytes[header_end..chunk_end];
        if chunk_index == 0 {
            if chunk_type != GLB_CHUNK_JSON {
                return Err(IoError::Invalid("first GLB chunk must be JSON".into()));
            }
            json_chunk = Some(payload);
        } else if chunk_type == GLB_CHUNK_JSON {
            return Err(IoError::Invalid("duplicate GLB JSON chunk".into()));
        } else if chunk_type == GLB_CHUNK_BIN {
            if bin_chunk.is_some() {
                return Err(IoError::Invalid("duplicate GLB BIN chunk".into()));
            }
            bin_chunk = Some(payload);
        }
        offset = chunk_end;
        chunk_index += 1;
    }

    let json = json_chunk.ok_or_else(|| IoError::Invalid("GLB missing JSON chunk".into()))?;
    Ok((json, bin_chunk))
}

fn validate_gltf_asset(root: &serde_json::Map<String, Value>) -> Result<()> {
    let asset = root
        .get("asset")
        .and_then(Value::as_object)
        .ok_or_else(|| IoError::Invalid("missing glTF asset object".into()))?;
    let version = asset
        .get("version")
        .and_then(Value::as_str)
        .ok_or_else(|| IoError::Invalid("missing glTF asset.version".into()))?;
    if !version.starts_with("2.") {
        return Err(IoError::Invalid(format!(
            "unsupported glTF asset version: {version}"
        )));
    }
    if let Some(min_ver) = asset.get("minVersion").and_then(Value::as_str) {
        if min_ver != "2.0" {
            return Err(IoError::Invalid(format!(
                "unsupported glTF minVersion: {min_ver}"
            )));
        }
    }
    Ok(())
}

fn check_extensions(
    root: &serde_json::Map<String, Value>,
    warnings: &mut Vec<String>,
) -> Result<()> {
    if let Some(req) = root.get("extensionsRequired").and_then(Value::as_array) {
        if let Some(ext) = req.first() {
            let name = ext.as_str().unwrap_or("unknown");
            return Err(IoError::Invalid(format!(
                "unsupported required glTF extension: {name}"
            )));
        }
    }
    if let Some(used) = root.get("extensionsUsed").and_then(Value::as_array) {
        for ext in used {
            if let Some(name) = ext.as_str() {
                import_warning!(warnings, format!("glTF extension ignored: {name}"));
            }
        }
    }
    Ok(())
}

fn decode_data_uri(uri: &str, max_bytes: usize) -> Result<(Option<String>, Vec<u8>)> {
    let rest = uri
        .strip_prefix("data:")
        .ok_or_else(|| IoError::Invalid("not a data URI".into()))?;
    let (meta, encoded) = rest
        .split_once(',')
        .ok_or_else(|| IoError::Invalid("malformed data URI".into()))?;
    if !meta.to_ascii_lowercase().ends_with(";base64") {
        return Err(IoError::Invalid("data URI must use base64 encoding".into()));
    }
    let mime_part = &meta[..meta.len() - ";base64".len()];
    let mime = (!mime_part.is_empty()).then(|| mime_part.to_ascii_lowercase());
    let cleaned: Vec<u8> = encoded
        .bytes()
        .filter(|b| !b.is_ascii_whitespace())
        .collect();
    let max_encoded = max_bytes
        .saturating_div(3)
        .saturating_mul(4)
        .saturating_add(16);
    if cleaned.len() > max_encoded {
        return Err(IoError::Invalid("data URI exceeds size limit".into()));
    }
    let decoded = BASE64_STANDARD
        .decode(&cleaned)
        .or_else(|_| BASE64_STANDARD_NO_PAD.decode(&cleaned))
        .map_err(|e| IoError::Invalid(format!("invalid base64 in data URI: {e}")))?;
    if decoded.len() > max_bytes {
        return Err(IoError::Invalid(
            "decoded data URI exceeds size limit".into(),
        ));
    }
    Ok((mime, decoded))
}

fn load_uri_bytes(
    uri: &str,
    base_dir: Option<&Path>,
    total_bytes: &mut usize,
) -> Result<(Option<String>, Vec<u8>)> {
    let trimmed = uri.trim();
    if trimmed.is_empty() {
        return Err(IoError::Invalid("empty URI".into()));
    }
    let remaining = MAX_ASSET_BYTES
        .checked_sub(*total_bytes)
        .ok_or_else(|| IoError::Invalid("combined asset size exceeds 64 MiB".into()))?;
    if trimmed.starts_with("data:") {
        let (mime, data) = decode_data_uri(trimmed, remaining)?;
        *total_bytes = total_bytes
            .checked_add(data.len())
            .ok_or_else(|| IoError::Invalid("asset size overflow".into()))?;
        Ok((mime, data))
    } else {
        let lower = trimmed.to_ascii_lowercase();
        if lower.starts_with("http://")
            || lower.starts_with("https://")
            || lower.starts_with("ftp://")
            || lower.starts_with("file:")
            || trimmed.contains(':')
        {
            return Err(IoError::Invalid(
                "network or scheme URIs are forbidden".into(),
            ));
        }
        let base = base_dir
            .ok_or_else(|| IoError::Invalid("external URI requires a base directory".into()))?;
        let rel = trimmed.strip_prefix("./").unwrap_or(trimmed);
        let data = read_asset(base, rel, remaining)?;
        *total_bytes = total_bytes
            .checked_add(data.len())
            .ok_or_else(|| IoError::Invalid("asset size overflow".into()))?;
        Ok((None, data))
    }
}

fn load_buffers(
    root: &serde_json::Map<String, Value>,
    bin_chunk: Option<&[u8]>,
    base_dir: Option<&Path>,
    total_bytes: &mut usize,
    control: &ImportControl,
) -> Result<Vec<Vec<u8>>> {
    let Some(arr) = root.get("buffers").and_then(Value::as_array) else {
        return Ok(Vec::new());
    };
    if arr.len() > 1024 {
        return Err(IoError::Invalid("too many glTF buffers".into()));
    }
    let mut out = Vec::with_capacity(arr.len());
    for (idx, buf_val) in arr.iter().enumerate() {
        control.check()?;
        let obj = buf_val
            .as_object()
            .ok_or_else(|| IoError::Invalid("buffer must be an object".into()))?;
        let byte_length = obj
            .get("byteLength")
            .and_then(Value::as_u64)
            .and_then(|v| usize::try_from(v).ok())
            .ok_or_else(|| IoError::Invalid("buffer missing valid byteLength".into()))?;
        if byte_length > MAX_ASSET_BYTES {
            return Err(IoError::Invalid("buffer exceeds 64 MiB limit".into()));
        }
        if let Some(uri_val) = obj.get("uri") {
            let uri = uri_val
                .as_str()
                .ok_or_else(|| IoError::Invalid("buffer.uri must be a string".into()))?;
            let (_, mut data) = load_uri_bytes(uri, base_dir, total_bytes)?;
            if data.len() < byte_length {
                return Err(IoError::Invalid(
                    "buffer data shorter than byteLength".into(),
                ));
            }
            data.truncate(byte_length);
            out.push(data);
        } else if idx == 0 {
            let bin = bin_chunk.ok_or_else(|| {
                IoError::Invalid("buffer 0 has no URI and GLB BIN chunk is missing".into())
            })?;
            if bin.len() < byte_length {
                return Err(IoError::Invalid(
                    "GLB BIN chunk shorter than buffer 0 byteLength".into(),
                ));
            }
            out.push(bin[..byte_length].to_vec());
        } else {
            return Err(IoError::Invalid("non-zero buffer missing URI".into()));
        }
    }
    Ok(out)
}

#[derive(Clone, Copy, Debug)]
struct BufferViewDesc {
    buffer: usize,
    byte_offset: usize,
    byte_length: usize,
    byte_stride: Option<usize>,
}

fn parse_buffer_views(
    root: &serde_json::Map<String, Value>,
    buffers: &[Vec<u8>],
) -> Result<Vec<BufferViewDesc>> {
    let Some(arr) = root.get("bufferViews").and_then(Value::as_array) else {
        return Ok(Vec::new());
    };
    if arr.len() > 100_000 {
        return Err(IoError::Invalid("too many bufferViews".into()));
    }
    let mut out = Vec::with_capacity(arr.len());
    for bv in arr {
        let obj = bv
            .as_object()
            .ok_or_else(|| IoError::Invalid("bufferView must be an object".into()))?;
        let buffer = obj
            .get("buffer")
            .and_then(Value::as_u64)
            .and_then(|v| usize::try_from(v).ok())
            .ok_or_else(|| IoError::Invalid("bufferView missing buffer index".into()))?;
        let buf = buffers
            .get(buffer)
            .ok_or_else(|| IoError::Invalid("bufferView buffer index out of bounds".into()))?;
        let byte_offset = match obj.get("byteOffset") {
            Some(v) => v
                .as_u64()
                .and_then(|n| usize::try_from(n).ok())
                .ok_or_else(|| IoError::Invalid("invalid bufferView.byteOffset".into()))?,
            None => 0,
        };
        let byte_length = obj
            .get("byteLength")
            .and_then(Value::as_u64)
            .and_then(|n| usize::try_from(n).ok())
            .ok_or_else(|| IoError::Invalid("bufferView missing byteLength".into()))?;
        if byte_length == 0 {
            return Err(IoError::Invalid(
                "bufferView byteLength must be positive".into(),
            ));
        }
        let end = byte_offset
            .checked_add(byte_length)
            .ok_or_else(|| IoError::Invalid("bufferView range overflow".into()))?;
        if end > buf.len() {
            return Err(IoError::Invalid("bufferView exceeds buffer bounds".into()));
        }
        let byte_stride = match obj.get("byteStride") {
            Some(v) => {
                let s = v
                    .as_u64()
                    .and_then(|n| usize::try_from(n).ok())
                    .ok_or_else(|| IoError::Invalid("invalid bufferView.byteStride".into()))?;
                if !(4..=252).contains(&s) || !s.is_multiple_of(4) {
                    return Err(IoError::Invalid(
                        "bufferView.byteStride must be a multiple of 4 in 4..=252".into(),
                    ));
                }
                Some(s)
            }
            None => None,
        };
        out.push(BufferViewDesc {
            buffer,
            byte_offset,
            byte_length,
            byte_stride,
        });
    }
    Ok(out)
}

#[derive(Clone, Debug)]
struct AccessorDesc {
    buffer_view: Option<usize>,
    byte_offset: usize,
    component_type: u32,
    normalized: bool,
    count: usize,
    kind: String,
}

fn component_size(component_type: u32) -> Result<usize> {
    match component_type {
        5120 | 5121 => Ok(1),
        5122 | 5123 => Ok(2),
        5125 | 5126 => Ok(4),
        _ => Err(IoError::Invalid(format!(
            "unsupported glTF accessor componentType {component_type}"
        ))),
    }
}

fn type_components(kind: &str) -> Result<usize> {
    match kind {
        "SCALAR" => Ok(1),
        "VEC2" => Ok(2),
        "VEC3" => Ok(3),
        "VEC4" | "MAT2" => Ok(4),
        "MAT3" => Ok(9),
        "MAT4" => Ok(16),
        _ => Err(IoError::Invalid(format!(
            "unsupported glTF accessor type {kind}"
        ))),
    }
}

fn parse_accessors(
    root: &serde_json::Map<String, Value>,
    buffer_views: &[BufferViewDesc],
) -> Result<Vec<AccessorDesc>> {
    let Some(arr) = root.get("accessors").and_then(Value::as_array) else {
        return Ok(Vec::new());
    };
    if arr.len() > 100_000 {
        return Err(IoError::Invalid("too many accessors".into()));
    }
    let mut out = Vec::with_capacity(arr.len());
    for acc in arr {
        let obj = acc
            .as_object()
            .ok_or_else(|| IoError::Invalid("accessor must be an object".into()))?;
        if obj.get("sparse").is_some() {
            return Err(IoError::Invalid(
                "sparse glTF accessors are not supported".into(),
            ));
        }
        let buffer_view = match obj.get("bufferView") {
            Some(v) => Some(
                v.as_u64()
                    .and_then(|n| usize::try_from(n).ok())
                    .ok_or_else(|| IoError::Invalid("invalid accessor.bufferView".into()))?,
            ),
            None => None,
        };
        let byte_offset = match obj.get("byteOffset") {
            Some(v) => v
                .as_u64()
                .and_then(|n| usize::try_from(n).ok())
                .ok_or_else(|| IoError::Invalid("invalid accessor.byteOffset".into()))?,
            None => 0,
        };
        let component_type = obj
            .get("componentType")
            .and_then(Value::as_u64)
            .and_then(|n| u32::try_from(n).ok())
            .ok_or_else(|| IoError::Invalid("accessor missing componentType".into()))?;
        let comp_bytes = component_size(component_type)?;
        let normalized = obj
            .get("normalized")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let count = obj
            .get("count")
            .and_then(Value::as_u64)
            .and_then(|n| usize::try_from(n).ok())
            .ok_or_else(|| IoError::Invalid("accessor missing count".into()))?;
        if count == 0 || count > MAX_TRIANGLES * 3 {
            return Err(IoError::Invalid("accessor count out of bounds".into()));
        }
        let kind = obj
            .get("type")
            .and_then(Value::as_str)
            .ok_or_else(|| IoError::Invalid("accessor missing type".into()))?
            .to_string();
        let num_comp = type_components(&kind)?;
        let elem_size = comp_bytes
            .checked_mul(num_comp)
            .ok_or_else(|| IoError::Invalid("accessor element size overflow".into()))?;

        if let Some(bv_idx) = buffer_view {
            let bv = buffer_views
                .get(bv_idx)
                .ok_or_else(|| IoError::Invalid("accessor bufferView out of bounds".into()))?;
            let total_offset = bv
                .byte_offset
                .checked_add(byte_offset)
                .ok_or_else(|| IoError::Invalid("accessor offset overflow".into()))?;
            if !total_offset.is_multiple_of(comp_bytes) {
                return Err(IoError::Invalid(
                    "accessor offset is not aligned to componentType".into(),
                ));
            }
            let stride = bv.byte_stride.unwrap_or(elem_size);
            if stride < elem_size || !stride.is_multiple_of(comp_bytes) {
                return Err(IoError::Invalid(
                    "bufferView byteStride incompatible with accessor".into(),
                ));
            }
            let span = (count - 1)
                .checked_mul(stride)
                .and_then(|s| s.checked_add(elem_size))
                .and_then(|s| byte_offset.checked_add(s))
                .ok_or_else(|| IoError::Invalid("accessor byte span overflow".into()))?;
            if span > bv.byte_length {
                return Err(IoError::Invalid(
                    "accessor exceeds bufferView bounds".into(),
                ));
            }
        } else if byte_offset != 0 {
            return Err(IoError::Invalid(
                "accessor without bufferView cannot have non-zero byteOffset".into(),
            ));
        }

        out.push(AccessorDesc {
            buffer_view,
            byte_offset,
            component_type,
            normalized,
            count,
            kind,
        });
    }
    Ok(out)
}

fn read_component_f64(bytes: &[u8], component_type: u32, normalized: bool) -> Result<f64> {
    let val = match component_type {
        5120 => {
            let v = *bytes
                .first()
                .ok_or_else(|| IoError::Invalid("out of bounds i8 read".into()))?
                as i8;
            if normalized {
                (f64::from(v) / 127.0).max(-1.0)
            } else {
                f64::from(v)
            }
        }
        5121 => {
            let v = *bytes
                .first()
                .ok_or_else(|| IoError::Invalid("out of bounds u8 read".into()))?;
            if normalized {
                f64::from(v) / 255.0
            } else {
                f64::from(v)
            }
        }
        5122 => {
            let b: [u8; 2] = bytes
                .get(..2)
                .ok_or_else(|| IoError::Invalid("out of bounds i16 read".into()))?
                .try_into()
                .map_err(|_| IoError::Invalid("slice error".into()))?;
            let v = i16::from_le_bytes(b);
            if normalized {
                (f64::from(v) / 32767.0).max(-1.0)
            } else {
                f64::from(v)
            }
        }
        5123 => {
            let b: [u8; 2] = bytes
                .get(..2)
                .ok_or_else(|| IoError::Invalid("out of bounds u16 read".into()))?
                .try_into()
                .map_err(|_| IoError::Invalid("slice error".into()))?;
            let v = u16::from_le_bytes(b);
            if normalized {
                f64::from(v) / 65535.0
            } else {
                f64::from(v)
            }
        }
        5125 => {
            let b: [u8; 4] = bytes
                .get(..4)
                .ok_or_else(|| IoError::Invalid("out of bounds u32 read".into()))?
                .try_into()
                .map_err(|_| IoError::Invalid("slice error".into()))?;
            let v = u32::from_le_bytes(b);
            if normalized {
                f64::from(v) / f64::from(u32::MAX)
            } else {
                f64::from(v)
            }
        }
        5126 => {
            let b: [u8; 4] = bytes
                .get(..4)
                .ok_or_else(|| IoError::Invalid("out of bounds f32 read".into()))?
                .try_into()
                .map_err(|_| IoError::Invalid("slice error".into()))?;
            f64::from(f32::from_le_bytes(b))
        }
        _ => {
            return Err(IoError::Invalid("unsupported component type".into()));
        }
    };
    if !val.is_finite() {
        return Err(IoError::Invalid("non-finite float in accessor".into()));
    }
    Ok(val)
}

fn read_positions(
    acc: &AccessorDesc,
    buffers: &[Vec<u8>],
    buffer_views: &[BufferViewDesc],
) -> Result<Vec<DVec3>> {
    if acc.kind != "VEC3" {
        return Err(IoError::Invalid("POSITION accessor must be VEC3".into()));
    }
    if acc.count > MAX_VERTICES {
        return Err(IoError::Invalid(
            "POSITION count exceeds 1M vertex limit".into(),
        ));
    }
    let Some(bv_idx) = acc.buffer_view else {
        return Ok(vec![DVec3::ZERO; acc.count]);
    };
    let bv = &buffer_views[bv_idx];
    let buf = &buffers[bv.buffer];
    let comp_size = component_size(acc.component_type)?;
    let elem_size = comp_size * 3;
    let stride = bv.byte_stride.unwrap_or(elem_size);
    let base = bv
        .byte_offset
        .checked_add(acc.byte_offset)
        .ok_or_else(|| IoError::Invalid("position offset overflow".into()))?;

    let mut out = Vec::with_capacity(acc.count);
    for i in 0..acc.count {
        let offset = base
            .checked_add(
                i.checked_mul(stride)
                    .ok_or_else(|| IoError::Invalid("stride overflow".into()))?,
            )
            .ok_or_else(|| IoError::Invalid("offset overflow".into()))?;
        let x = read_component_f64(&buf[offset..], acc.component_type, acc.normalized)?;
        let y = read_component_f64(
            &buf[offset + comp_size..],
            acc.component_type,
            acc.normalized,
        )?;
        let z = read_component_f64(
            &buf[offset + comp_size * 2..],
            acc.component_type,
            acc.normalized,
        )?;
        if x.abs() > MAX_COORDINATE || y.abs() > MAX_COORDINATE || z.abs() > MAX_COORDINATE {
            return Err(IoError::Invalid("vertex coordinate exceeds bounds".into()));
        }
        out.push(DVec3::new(x, y, z));
    }
    Ok(out)
}

fn read_indices(
    acc: &AccessorDesc,
    buffers: &[Vec<u8>],
    buffer_views: &[BufferViewDesc],
) -> Result<Vec<usize>> {
    if acc.kind != "SCALAR" {
        return Err(IoError::Invalid("index accessor must be SCALAR".into()));
    }
    let bv_idx = acc
        .buffer_view
        .ok_or_else(|| IoError::Invalid("index accessor missing bufferView".into()))?;
    let bv = &buffer_views[bv_idx];
    let buf = &buffers[bv.buffer];
    let comp_size = component_size(acc.component_type)?;
    let stride = bv.byte_stride.unwrap_or(comp_size);
    let base = bv
        .byte_offset
        .checked_add(acc.byte_offset)
        .ok_or_else(|| IoError::Invalid("index offset overflow".into()))?;

    let mut out = Vec::with_capacity(acc.count);
    for i in 0..acc.count {
        let offset = base
            .checked_add(
                i.checked_mul(stride)
                    .ok_or_else(|| IoError::Invalid("stride overflow".into()))?,
            )
            .ok_or_else(|| IoError::Invalid("offset overflow".into()))?;
        let idx = match acc.component_type {
            5121 => usize::from(buf[offset]),
            5123 => {
                let b: [u8; 2] = buf[offset..offset + 2]
                    .try_into()
                    .map_err(|_| IoError::Invalid("u16 index slice error".into()))?;
                usize::from(u16::from_le_bytes(b))
            }
            5125 => {
                let b: [u8; 4] = buf[offset..offset + 4]
                    .try_into()
                    .map_err(|_| IoError::Invalid("u32 index slice error".into()))?;
                u32::from_le_bytes(b) as usize
            }
            _ => {
                return Err(IoError::Invalid(
                    "index accessor componentType must be UNSIGNED_BYTE, UNSIGNED_SHORT, or UNSIGNED_INT".into(),
                ));
            }
        };
        out.push(idx);
    }
    Ok(out)
}

#[allow(clippy::too_many_arguments)]
fn parse_materials_and_textures(
    root: &serde_json::Map<String, Value>,
    buffers: &[Vec<u8>],
    buffer_views: &[BufferViewDesc],
    base_dir: Option<&Path>,
    total_bytes: &mut usize,
    textures_out: &mut BTreeMap<String, TextureAsset>,
    warnings: &mut Vec<String>,
    control: &ImportControl,
) -> Result<Vec<Material>> {
    let images_json = root.get("images").and_then(Value::as_array);
    let textures_json = root.get("textures").and_then(Value::as_array);
    let Some(materials_json) = root.get("materials").and_then(Value::as_array) else {
        return Ok(Vec::new());
    };
    if materials_json.len() > 100_000 {
        return Err(IoError::Invalid("too many glTF materials".into()));
    }

    let mut loaded_images: HashMap<usize, String> = HashMap::new();

    let mut resolve_texture_slot = |tex_idx: usize| -> Result<Option<String>> {
        let tex_arr = textures_json
            .ok_or_else(|| IoError::Invalid("material references missing textures array".into()))?;
        let tex_obj = tex_arr
            .get(tex_idx)
            .and_then(Value::as_object)
            .ok_or_else(|| IoError::Invalid("texture index out of bounds".into()))?;
        let Some(source_idx) = tex_obj
            .get("source")
            .and_then(Value::as_u64)
            .and_then(|n| usize::try_from(n).ok())
        else {
            return Ok(None);
        };
        if let Some(slot) = loaded_images.get(&source_idx) {
            return Ok(Some(slot.clone()));
        }
        let img_arr = images_json
            .ok_or_else(|| IoError::Invalid("texture references missing images array".into()))?;
        let img_obj = img_arr
            .get(source_idx)
            .and_then(Value::as_object)
            .ok_or_else(|| IoError::Invalid("image source index out of bounds".into()))?;

        let img_bytes = if let Some(uri_val) = img_obj.get("uri") {
            let uri = uri_val
                .as_str()
                .ok_or_else(|| IoError::Invalid("image.uri must be a string".into()))?;
            let (declared_mime, bytes) = load_uri_bytes(uri, base_dir, total_bytes)?;
            if let Some(m) = img_obj.get("mimeType").and_then(Value::as_str) {
                if let Some(dm) = &declared_mime {
                    if !dm.eq_ignore_ascii_case(m) {
                        return Err(IoError::Invalid("image mimeType mismatch".into()));
                    }
                }
            }
            bytes
        } else if let Some(bv_val) = img_obj.get("bufferView") {
            let bv_idx = bv_val
                .as_u64()
                .and_then(|n| usize::try_from(n).ok())
                .ok_or_else(|| IoError::Invalid("invalid image.bufferView".into()))?;
            let bv = buffer_views
                .get(bv_idx)
                .ok_or_else(|| IoError::Invalid("image bufferView out of bounds".into()))?;
            let buf = &buffers[bv.buffer];
            buf[bv.byte_offset..bv.byte_offset + bv.byte_length].to_vec()
        } else {
            return Err(IoError::Invalid(
                "image must define uri or bufferView".into(),
            ));
        };

        let (key, asset) = texture_from_bytes(img_bytes)?;
        if let Some(mime_str) = img_obj.get("mimeType").and_then(Value::as_str) {
            if !mime_str.eq_ignore_ascii_case(&asset.mime) {
                return Err(IoError::Invalid(
                    "image mimeType disagrees with decoded image format".into(),
                ));
            }
        }
        if !textures_out.contains_key(&key) && textures_out.len() >= MAX_TEXTURES {
            return Err(IoError::Invalid("exceeded 127 texture limit".into()));
        }
        textures_out.insert(key.clone(), asset);
        loaded_images.insert(source_idx, key.clone());
        Ok(Some(key))
    };

    let mut materials = Vec::with_capacity(materials_json.len());
    for (idx, mat_val) in materials_json.iter().enumerate() {
        control.check()?;
        let obj = mat_val
            .as_object()
            .ok_or_else(|| IoError::Invalid("material must be an object".into()))?;
        let raw_name = obj
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim();
        let name = if raw_name.is_empty() {
            format!("Material_{idx}")
        } else {
            raw_name
                .chars()
                .filter(|c| !c.is_control())
                .take(512)
                .collect()
        };

        let mut color = [1.0f32, 1.0, 1.0];
        let mut factor_alpha = 1.0f32;
        let mut texture: Option<String> = None;

        if let Some(pbr) = obj.get("pbrMetallicRoughness").and_then(Value::as_object) {
            if let Some(bcf) = pbr.get("baseColorFactor").and_then(Value::as_array) {
                if bcf.len() != 4 {
                    return Err(IoError::Invalid(
                        "baseColorFactor must have 4 components".into(),
                    ));
                }
                let mut rgba = [0.0f32; 4];
                for (i, c) in bcf.iter().enumerate() {
                    let v = c
                        .as_f64()
                        .ok_or_else(|| IoError::Invalid("non-numeric baseColorFactor".into()))?
                        as f32;
                    if !v.is_finite() || !(0.0..=1.0).contains(&v) {
                        return Err(IoError::Invalid(
                            "baseColorFactor components must be finite in 0..=1".into(),
                        ));
                    }
                    rgba[i] = v;
                }
                color = [rgba[0], rgba[1], rgba[2]];
                factor_alpha = rgba[3];
            }
            if let Some(bct) = pbr.get("baseColorTexture").and_then(Value::as_object) {
                let tex_idx = bct
                    .get("index")
                    .and_then(Value::as_u64)
                    .and_then(|n| usize::try_from(n).ok())
                    .ok_or_else(|| IoError::Invalid("baseColorTexture missing index".into()))?;
                texture = resolve_texture_slot(tex_idx)?;
            }
        }

        let alpha_mode = obj.get("alphaMode").and_then(Value::as_str);
        let opacity = match alpha_mode {
            Some("OPAQUE") | None => 1.0,
            Some("BLEND") => factor_alpha,
            Some("MASK") => {
                import_warning!(
                    warnings,
                    "glTF alpha MASK is approximated with blending".into()
                );
                factor_alpha
            }
            Some(other) => {
                import_warning!(
                    warnings,
                    format!("unsupported glTF alphaMode {other}, treating as BLEND")
                );
                factor_alpha
            }
        };

        materials.push(Material {
            name,
            color,
            opacity,
            texture,
        });
    }

    // The renderer multiplies image alpha by material opacity. Normalize OPAQUE
    // textures so glTF's default alpha mode cannot accidentally become translucent.
    for (index, material) in materials.iter_mut().enumerate() {
        control.check()?;
        let mode = materials_json[index]
            .get("alphaMode")
            .and_then(Value::as_str)
            .unwrap_or("OPAQUE");
        if mode != "OPAQUE" {
            continue;
        }
        let Some(slot) = material.texture.clone() else {
            continue;
        };
        let asset = textures_out
            .get(&slot)
            .ok_or_else(|| IoError::Invalid("missing imported texture".into()))?;
        let mut pixels = crate::decode_texture(asset)?;
        if pixels.rgba.as_chunks::<4>().0.iter().all(|p| p[3] == 255) {
            continue;
        }
        for pixel in pixels.rgba.as_chunks_mut::<4>().0 {
            pixel[3] = 255;
        }
        let image = image::RgbaImage::from_raw(pixels.width, pixels.height, pixels.rgba)
            .ok_or_else(|| IoError::Invalid("invalid decoded image".into()))?;
        let mut encoded = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(image)
            .write_to(&mut encoded, image::ImageFormat::Png)
            .map_err(|e| IoError::Invalid(e.to_string()))?;
        let (key, asset) = texture_from_bytes(encoded.into_inner())?;
        material.texture = Some(key.clone());
        textures_out.insert(key, asset);
    }
    let used: BTreeSet<_> = materials
        .iter()
        .filter_map(|m| m.texture.as_ref())
        .collect();
    textures_out.retain(|key, _| used.contains(key));
    Ok(materials)
}

#[derive(Clone, Debug)]
struct ParsedMesh {
    skipped: usize,
    name: String,
    geometry: BulkGeometry,
}

#[allow(clippy::too_many_arguments)]
fn parse_meshes(
    root: &serde_json::Map<String, Value>,
    buffers: &[Vec<u8>],
    buffer_views: &[BufferViewDesc],
    accessors: &[AccessorDesc],
    materials: &[Material],
    total_vertices: &mut usize,
    total_triangles: &mut usize,
    warnings: &mut Vec<String>,
    control: &ImportControl,
) -> Result<Vec<ParsedMesh>> {
    let Some(meshes_arr) = root.get("meshes").and_then(Value::as_array) else {
        return Ok(Vec::new());
    };
    if meshes_arr.len() > MAX_NODES {
        return Err(IoError::Invalid("too many glTF meshes".into()));
    }

    let mut out = Vec::with_capacity(meshes_arr.len());
    for (mesh_idx, mesh_val) in meshes_arr.iter().enumerate() {
        control.check()?;
        let mesh_obj = mesh_val
            .as_object()
            .ok_or_else(|| IoError::Invalid("mesh must be an object".into()))?;
        let raw_name = mesh_obj
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim();
        let name = if raw_name.is_empty() {
            format!("Mesh_{mesh_idx}")
        } else {
            raw_name
                .chars()
                .filter(|c| !c.is_control())
                .take(512)
                .collect()
        };

        let prims = mesh_obj
            .get("primitives")
            .and_then(Value::as_array)
            .ok_or_else(|| IoError::Invalid("mesh missing primitives array".into()))?;
        if prims.is_empty() {
            return Err(IoError::Invalid("mesh primitives cannot be empty".into()));
        }

        let mut geom = BulkGeometry::default();
        let mut skipped_degenerate = 0usize;

        for prim_val in prims {
            control.check()?;
            let prim = prim_val
                .as_object()
                .ok_or_else(|| IoError::Invalid("primitive must be an object".into()))?;
            let mode = prim.get("mode").and_then(Value::as_u64).unwrap_or(4);
            if mode != 4 {
                skipped_degenerate += 1;
                import_warning!(
                    warnings,
                    format!("skipped non-triangle glTF primitive mode {mode} in {name}")
                );
                continue;
            }
            if prim.get("targets").is_some() {
                import_warning!(warnings, format!("ignored morph targets on mesh {name}"));
            }

            let attrs = prim
                .get("attributes")
                .and_then(Value::as_object)
                .ok_or_else(|| IoError::Invalid("primitive missing attributes".into()))?;
            let pos_acc_idx = attrs
                .get("POSITION")
                .and_then(Value::as_u64)
                .and_then(|n| usize::try_from(n).ok())
                .ok_or_else(|| IoError::Invalid("primitive missing POSITION attribute".into()))?;
            let pos_acc = accessors
                .get(pos_acc_idx)
                .ok_or_else(|| IoError::Invalid("POSITION accessor index out of bounds".into()))?;

            let positions = read_positions(pos_acc, buffers, buffer_views)?;
            *total_vertices = total_vertices
                .checked_add(positions.len())
                .ok_or_else(|| IoError::Invalid("vertex count overflow".into()))?;
            if *total_vertices > MAX_VERTICES {
                return Err(IoError::Invalid("total vertices exceed 1M limit".into()));
            }

            let (mat_id, face_color) = if let Some(mat_val) = prim.get("material") {
                let mat_idx = mat_val
                    .as_u64()
                    .and_then(|n| usize::try_from(n).ok())
                    .ok_or_else(|| IoError::Invalid("invalid primitive.material".into()))?;
                let mat = materials.get(mat_idx).ok_or_else(|| {
                    IoError::Invalid("primitive material index out of bounds".into())
                })?;
                let rgba = [
                    (mat.color[0] * 255.0).round() as u8,
                    (mat.color[1] * 255.0).round() as u8,
                    (mat.color[2] * 255.0).round() as u8,
                    (mat.opacity * 255.0).round() as u8,
                ];
                (Some(MaterialId((mat_idx + 1) as u64)), rgba)
            } else {
                (None, [160, 177, 193, 255])
            };

            let base_vertex = geom.vertices.len();
            geom.vertices.extend_from_slice(&positions);

            let tri_indices: Vec<usize> = if let Some(idx_val) = prim.get("indices") {
                let idx_acc_idx = idx_val
                    .as_u64()
                    .and_then(|n| usize::try_from(n).ok())
                    .ok_or_else(|| IoError::Invalid("invalid primitive.indices".into()))?;
                let idx_acc = accessors
                    .get(idx_acc_idx)
                    .ok_or_else(|| IoError::Invalid("indices accessor out of bounds".into()))?;
                read_indices(idx_acc, buffers, buffer_views)?
            } else {
                (0..positions.len()).collect()
            };

            if !tri_indices.len().is_multiple_of(3) {
                return Err(IoError::Invalid(
                    "triangle index count is not a multiple of 3".into(),
                ));
            }
            let num_tris = tri_indices.len() / 3;
            *total_triangles = total_triangles
                .checked_add(num_tris)
                .ok_or_else(|| IoError::Invalid("triangle count overflow".into()))?;
            if *total_triangles > MAX_TRIANGLES {
                return Err(IoError::Invalid("total triangles exceed 1M limit".into()));
            }

            for tri in tri_indices.as_chunks::<3>().0 {
                let (i0, i1, i2) = (tri[0], tri[1], tri[2]);
                if i0 >= positions.len() || i1 >= positions.len() || i2 >= positions.len() {
                    return Err(IoError::Invalid(
                        "triangle vertex index out of bounds".into(),
                    ));
                }
                let p0 = positions[i0];
                let p1 = positions[i1];
                let p2 = positions[i2];
                if p0.distance(p1) <= TOLERANCE
                    || p1.distance(p2) <= TOLERANCE
                    || p2.distance(p0) <= TOLERANCE
                    || (p1 - p0).cross(p2 - p0).length() <= TOLERANCE * TOLERANCE * 2.0
                {
                    skipped_degenerate += 1;
                    continue;
                }
                geom.faces.push(BulkFace {
                    outer: vec![base_vertex + i0, base_vertex + i1, base_vertex + i2],
                    holes: Vec::new(),
                    material: mat_id,
                    color: face_color,
                });
            }
        }

        if skipped_degenerate > 0 {
            import_warning!(
                warnings,
                format!("skipped {skipped_degenerate} degenerate triangles in {name}")
            );
        }

        out.push(ParsedMesh {
            skipped: skipped_degenerate,
            name,
            geometry: geom,
        });
    }
    Ok(out)
}

fn parse_node_transform(node_obj: &serde_json::Map<String, Value>) -> Result<DMat4> {
    let has_matrix = node_obj.contains_key("matrix");
    let has_trs = node_obj.contains_key("translation")
        || node_obj.contains_key("rotation")
        || node_obj.contains_key("scale");
    if has_matrix && has_trs {
        return Err(IoError::Invalid(
            "glTF node cannot specify both matrix and TRS properties".into(),
        ));
    }

    let mat = if let Some(mat_val) = node_obj.get("matrix") {
        let arr = mat_val
            .as_array()
            .ok_or_else(|| IoError::Invalid("node.matrix must be an array".into()))?;
        if arr.len() != 16 {
            return Err(IoError::Invalid("node.matrix must have 16 elements".into()));
        }
        let mut cols = [0.0f64; 16];
        for (i, v) in arr.iter().enumerate() {
            let num = v
                .as_f64()
                .ok_or_else(|| IoError::Invalid("non-numeric element in node.matrix".into()))?;
            if !num.is_finite() {
                return Err(IoError::Invalid("non-finite element in node.matrix".into()));
            }
            cols[i] = num;
        }
        DMat4::from_cols_array(&cols)
    } else {
        let translation = if let Some(t_val) = node_obj.get("translation") {
            let arr = t_val
                .as_array()
                .ok_or_else(|| IoError::Invalid("node.translation must be an array".into()))?;
            if arr.len() != 3 {
                return Err(IoError::Invalid(
                    "node.translation must have 3 elements".into(),
                ));
            }
            let x = arr[0]
                .as_f64()
                .ok_or_else(|| IoError::Invalid("invalid translation.x".into()))?;
            let y = arr[1]
                .as_f64()
                .ok_or_else(|| IoError::Invalid("invalid translation.y".into()))?;
            let z = arr[2]
                .as_f64()
                .ok_or_else(|| IoError::Invalid("invalid translation.z".into()))?;
            DVec3::new(x, y, z)
        } else {
            DVec3::ZERO
        };

        let rotation = if let Some(r_val) = node_obj.get("rotation") {
            let arr = r_val
                .as_array()
                .ok_or_else(|| IoError::Invalid("node.rotation must be an array".into()))?;
            if arr.len() != 4 {
                return Err(IoError::Invalid(
                    "node.rotation must have 4 elements".into(),
                ));
            }
            let x = arr[0]
                .as_f64()
                .ok_or_else(|| IoError::Invalid("invalid rotation.x".into()))?;
            let y = arr[1]
                .as_f64()
                .ok_or_else(|| IoError::Invalid("invalid rotation.y".into()))?;
            let z = arr[2]
                .as_f64()
                .ok_or_else(|| IoError::Invalid("invalid rotation.z".into()))?;
            let w = arr[3]
                .as_f64()
                .ok_or_else(|| IoError::Invalid("invalid rotation.w".into()))?;
            let q = DQuat::from_xyzw(x, y, z, w);
            if !q.is_finite() || q.length_squared() < 1e-12 {
                return Err(IoError::Invalid("invalid node rotation quaternion".into()));
            }
            q.normalize()
        } else {
            DQuat::IDENTITY
        };

        let scale = if let Some(s_val) = node_obj.get("scale") {
            let arr = s_val
                .as_array()
                .ok_or_else(|| IoError::Invalid("node.scale must be an array".into()))?;
            if arr.len() != 3 {
                return Err(IoError::Invalid("node.scale must have 3 elements".into()));
            }
            let x = arr[0]
                .as_f64()
                .ok_or_else(|| IoError::Invalid("invalid scale.x".into()))?;
            let y = arr[1]
                .as_f64()
                .ok_or_else(|| IoError::Invalid("invalid scale.y".into()))?;
            let z = arr[2]
                .as_f64()
                .ok_or_else(|| IoError::Invalid("invalid scale.z".into()))?;
            DVec3::new(x, y, z)
        } else {
            DVec3::ONE
        };

        DMat4::from_scale_rotation_translation(scale, rotation, translation)
    };

    cindral_core::validate_instance_transform(mat)?;
    Ok(mat)
}

fn build_node_hierarchy(
    root: &serde_json::Map<String, Value>,
    parsed_meshes: &[ParsedMesh],
    scene: &mut ImportedScene,
    control: &ImportControl,
) -> Result<()> {
    let Some(nodes_arr) = root.get("nodes").and_then(Value::as_array) else {
        return Ok(());
    };
    if nodes_arr.is_empty() {
        return Ok(());
    }
    if nodes_arr.len() > MAX_NODES {
        return Err(IoError::Invalid(
            "glTF node count exceeds 100k limit".into(),
        ));
    }

    let root_node_indices: Vec<usize> = if let Some(scenes_arr) =
        root.get("scenes").and_then(Value::as_array)
    {
        if scenes_arr.is_empty() {
            Vec::new()
        } else {
            let scene_idx = match root.get("scene") {
                Some(v) => v
                    .as_u64()
                    .and_then(|n| usize::try_from(n).ok())
                    .ok_or_else(|| IoError::Invalid("invalid glTF scene index".into()))?,
                None => 0,
            };
            let scene_obj = scenes_arr
                .get(scene_idx)
                .and_then(Value::as_object)
                .ok_or_else(|| IoError::Invalid("scene index out of bounds".into()))?;
            if let Some(n_arr) = scene_obj.get("nodes").and_then(Value::as_array) {
                let mut indices = Vec::with_capacity(n_arr.len());
                for v in n_arr {
                    let idx = v
                        .as_u64()
                        .and_then(|n| usize::try_from(n).ok())
                        .ok_or_else(|| IoError::Invalid("invalid scene root node index".into()))?;
                    if idx >= nodes_arr.len() {
                        return Err(IoError::Invalid(
                            "scene root node index out of bounds".into(),
                        ));
                    }
                    indices.push(idx);
                }
                indices
            } else {
                Vec::new()
            }
        }
    } else {
        let mut children = BTreeSet::new();
        for node in nodes_arr {
            if let Some(items) = node.get("children").and_then(Value::as_array) {
                for child in items {
                    let child = child
                        .as_u64()
                        .and_then(|v| usize::try_from(v).ok())
                        .filter(|i| *i < nodes_arr.len())
                        .ok_or_else(|| IoError::Invalid("invalid child node reference".into()))?;
                    children.insert(child);
                }
            }
        }
        let roots: Vec<_> = (0..nodes_arr.len())
            .filter(|i| !children.contains(i))
            .collect();
        if roots.is_empty() {
            return Err(IoError::Invalid(
                "cyclic glTF hierarchy has no roots".into(),
            ));
        }
        roots
    };

    let mut mesh_def_map: HashMap<usize, usize> = HashMap::new();
    let mut visited_count = 0usize;

    #[allow(clippy::too_many_arguments)]
    fn visit_node(
        node_idx: usize,
        depth: usize,
        active_path: &mut BTreeSet<usize>,
        nodes_arr: &[Value],
        parsed_meshes: &[ParsedMesh],
        mesh_def_map: &mut HashMap<usize, usize>,
        scene: &mut ImportedScene,
        visited_count: &mut usize,
        control: &ImportControl,
    ) -> Result<Option<ImportedInstance>> {
        if depth > MAX_DEPTH {
            return Err(IoError::Invalid(
                "glTF node hierarchy exceeds depth 64".into(),
            ));
        }
        if !active_path.insert(node_idx) {
            return Err(IoError::Invalid(
                "cycle detected in glTF node hierarchy".into(),
            ));
        }
        *visited_count += 1;
        if *visited_count > MAX_NODES {
            return Err(IoError::Invalid(
                "glTF node traversal exceeds 100k limit".into(),
            ));
        }
        if (*visited_count).is_multiple_of(512) {
            control.check()?;
        }

        let node_obj = nodes_arr
            .get(node_idx)
            .and_then(Value::as_object)
            .ok_or_else(|| IoError::Invalid("node index out of bounds".into()))?;

        let raw_name = node_obj
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim();
        let node_name = if raw_name.is_empty() {
            format!("Node_{node_idx}")
        } else {
            raw_name
                .chars()
                .filter(|c| !c.is_control())
                .take(512)
                .collect()
        };

        let transform = parse_node_transform(node_obj)?;

        let mesh_idx = match node_obj.get("mesh") {
            Some(v) => {
                let idx = v
                    .as_u64()
                    .and_then(|n| usize::try_from(n).ok())
                    .ok_or_else(|| IoError::Invalid("invalid node.mesh index".into()))?;
                if idx >= parsed_meshes.len() {
                    return Err(IoError::Invalid("node.mesh index out of bounds".into()));
                }
                Some(idx)
            }
            None => None,
        };

        let mut child_instances = Vec::new();
        if let Some(children_arr) = node_obj.get("children").and_then(Value::as_array) {
            for child_val in children_arr {
                let child_idx = child_val
                    .as_u64()
                    .and_then(|n| usize::try_from(n).ok())
                    .ok_or_else(|| IoError::Invalid("invalid child node index".into()))?;
                if child_idx >= nodes_arr.len() {
                    return Err(IoError::Invalid("child node index out of bounds".into()));
                }
                if let Some(inst) = visit_node(
                    child_idx,
                    depth + 1,
                    active_path,
                    nodes_arr,
                    parsed_meshes,
                    mesh_def_map,
                    scene,
                    visited_count,
                    control,
                )? {
                    child_instances.push(inst);
                }
            }
        }

        active_path.remove(&node_idx);

        let get_or_insert_mesh_def = |m_idx: usize,
                                      mesh_def_map: &mut HashMap<usize, usize>,
                                      scene: &mut ImportedScene|
         -> Option<usize> {
            if let Some(&def_idx) = mesh_def_map.get(&m_idx) {
                return Some(def_idx);
            }
            let pm = &parsed_meshes[m_idx];
            if pm.geometry.faces.is_empty() && pm.geometry.edges.is_empty() {
                return None;
            }
            let def_idx = scene.definitions.len();
            scene.definitions.push(ImportedDefinition {
                name: pm.name.clone(),
                geometry: pm.geometry.clone(),
                instances: Vec::new(),
            });
            mesh_def_map.insert(m_idx, def_idx);
            Some(def_idx)
        };

        match (mesh_idx, child_instances.is_empty()) {
            (Some(m_idx), true) => {
                let Some(def_idx) = get_or_insert_mesh_def(m_idx, mesh_def_map, scene) else {
                    return Ok(None);
                };
                Ok(Some(ImportedInstance {
                    name: node_name,
                    definition: def_idx,
                    transform,
                    tag: None,
            ..Default::default()
}))
            }
            (None, false) => {
                let def_idx = scene.definitions.len();
                scene.definitions.push(ImportedDefinition {
                    name: node_name.clone(),
                    geometry: BulkGeometry::default(),
                    instances: child_instances,
                });
                Ok(Some(ImportedInstance {
                    name: node_name,
                    definition: def_idx,
                    transform,
                    tag: None,
            ..Default::default()
}))
            }
            (Some(m_idx), false) => {
                let mut instances = child_instances;
                if let Some(mesh_def_idx) = get_or_insert_mesh_def(m_idx, mesh_def_map, scene) {
                    instances.insert(
                        0,
                        ImportedInstance {
                            name: parsed_meshes[m_idx].name.clone(),
                            definition: mesh_def_idx,
                            transform: DMat4::IDENTITY,
                            tag: None,
            ..Default::default()
},
                    );
                }
                let def_idx = scene.definitions.len();
                scene.definitions.push(ImportedDefinition {
                    name: node_name.clone(),
                    geometry: BulkGeometry::default(),
                    instances,
                });
                Ok(Some(ImportedInstance {
                    name: node_name,
                    definition: def_idx,
                    transform,
                    tag: None,
            ..Default::default()
}))
            }
            (None, true) => Ok(None),
        }
    }

    let mut active_path = BTreeSet::new();
    for root_idx in root_node_indices {
        if let Some(inst) = visit_node(
            root_idx,
            1,
            &mut active_path,
            nodes_arr,
            parsed_meshes,
            &mut mesh_def_map,
            scene,
            &mut visited_count,
            control,
        )? {
            scene.roots.push(inst);
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        export_gltf,
        import_scene::{convert_scene, ImportOptions},
        DraftBundle,
    };
    use cindral_core::{Command, DocumentHistory};
    use std::io::Cursor;

    fn sample_png_bytes() -> Vec<u8> {
        let img = image::RgbaImage::from_pixel(2, 2, image::Rgba([200, 100, 50, 255]));
        let mut buf = Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(img)
            .write_to(&mut buf, image::ImageFormat::Png)
            .unwrap();
        buf.into_inner()
    }

    #[test]
    fn gltf_shared_mesh_strides_materials_and_textures_work() {
        let png = sample_png_bytes();
        let png_b64 = BASE64_STANDARD.encode(&png);

        // Interleaved buffer: 3 vertices, stride 16 bytes (vec3 f32 + 4 bytes padding),
        // followed by 3 u16 indices (0, 1, 2) + 2 bytes padding.
        let mut bin = Vec::new();
        let verts = [[0.0f32, 0.0, 0.0], [2.0, 0.0, 0.0], [0.0, 3.0, 0.0]];
        for v in verts {
            for c in v {
                bin.extend_from_slice(&c.to_le_bytes());
            }
            bin.extend_from_slice(&0u32.to_le_bytes()); // stride padding
        }
        let idx_offset = bin.len();
        for idx in [0u16, 1, 2] {
            bin.extend_from_slice(&idx.to_le_bytes());
        }
        bin.extend_from_slice(&0u16.to_le_bytes());
        let bin_b64 = BASE64_STANDARD.encode(&bin);

        let gltf_json = serde_json::json!({
            "asset": { "version": "2.0" },
            "buffers": [{
                "byteLength": bin.len(),
                "uri": format!("data:application/octet-stream;base64,{bin_b64}")
            }],
            "bufferViews": [
                { "buffer": 0, "byteOffset": 0, "byteLength": 48, "byteStride": 16 },
                { "buffer": 0, "byteOffset": idx_offset, "byteLength": 6 }
            ],
            "accessors": [
                { "bufferView": 0, "byteOffset": 0, "componentType": 5126, "count": 3, "type": "VEC3" },
                { "bufferView": 1, "byteOffset": 0, "componentType": 5123, "count": 3, "type": "SCALAR" }
            ],
            "images": [{
                "mimeType": "image/png",
                "uri": format!("data:image/png;base64,{png_b64}")
            }],
            "textures": [{ "source": 0 }],
            "materials": [{
                "name": "TexturedAlpha",
                "pbrMetallicRoughness": {
                    "baseColorFactor": [0.5, 0.6, 0.7, 0.8],
                    "baseColorTexture": { "index": 0 }
                },
                "alphaMode": "BLEND"
            }],
            "meshes": [{
                "name": "SharedTri",
                "primitives": [{
                    "attributes": { "POSITION": 0 },
                    "indices": 1,
                    "material": 0
                }]
            }],
            "nodes": [
                { "name": "InstA", "mesh": 0, "translation": [1.0, 2.0, 3.0] },
                { "name": "InstB", "mesh": 0, "translation": [-1.0, 0.0, 0.0] }
            ],
            "scenes": [{ "nodes": [0, 1] }],
            "scene": 0
        });

        let bytes = serde_json::to_vec(&gltf_json).unwrap();
        let control = ImportControl::default();
        let scene = import_gltf_bytes(&bytes, None, &control).unwrap();
        assert_eq!(scene.definitions.len(), 1);
        assert_eq!(scene.roots.len(), 2);
        assert_eq!(scene.roots[0].definition, 0);
        assert_eq!(scene.roots[1].definition, 0);
        assert_eq!(scene.materials.len(), 1);
        assert!((scene.materials[0].opacity - 0.8).abs() < 1e-5);
        assert_eq!(scene.textures.len(), 1);
    }

    #[test]
    fn glb_binary_and_mutated_bytes_never_panic() {
        let mut bin = Vec::new();
        for v in [[0.0f32, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]] {
            for c in v {
                bin.extend_from_slice(&c.to_le_bytes());
            }
        }
        let mut json_bytes = serde_json::to_vec(&serde_json::json!({
            "asset": { "version": "2.0" },
            "buffers": [{ "byteLength": bin.len() }],
            "bufferViews": [{ "buffer": 0, "byteOffset": 0, "byteLength": bin.len() }],
            "accessors": [{ "bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3" }],
            "meshes": [{ "primitives": [{ "attributes": { "POSITION": 0 } }] }],
            "nodes": [{ "mesh": 0 }],
            "scenes": [{ "nodes": [0] }]
        }))
        .unwrap();
        while !json_bytes.len().is_multiple_of(4) {
            json_bytes.push(b' ');
        }

        let total_len = 12 + 8 + json_bytes.len() + 8 + bin.len();
        let mut glb = Vec::with_capacity(total_len);
        glb.extend_from_slice(&GLB_MAGIC.to_le_bytes());
        glb.extend_from_slice(&2u32.to_le_bytes());
        glb.extend_from_slice(&(total_len as u32).to_le_bytes());
        glb.extend_from_slice(&(json_bytes.len() as u32).to_le_bytes());
        glb.extend_from_slice(&GLB_CHUNK_JSON.to_le_bytes());
        glb.extend_from_slice(&json_bytes);
        glb.extend_from_slice(&(bin.len() as u32).to_le_bytes());
        glb.extend_from_slice(&GLB_CHUNK_BIN.to_le_bytes());
        glb.extend_from_slice(&bin);

        let control = ImportControl::default();
        let scene = import_gltf_bytes(&glb, None, &control).unwrap();
        assert_eq!(scene.definitions.len(), 1);

        // Mutated / truncated hostile bytes must never panic.
        for cut in (0..glb.len()).step_by(7) {
            let _ = import_gltf_bytes(&glb[..cut], None, &control);
        }
        for i in (0..glb.len()).step_by(5) {
            let mut mutated = glb.clone();
            mutated[i] ^= 0xFF;
            let _ = import_gltf_bytes(&mutated, None, &control);
        }
    }

    #[test]
    fn gltf_export_import_roundtrip_preserves_geometry() {
        let mut history = DocumentHistory::new();
        history
            .execute(Command::AddPolyline {
                points: vec![
                    DVec3::new(0.0, 0.0, 0.0),
                    DVec3::new(2.0, 0.0, 0.0),
                    DVec3::new(2.0, 3.0, 0.0),
                    DVec3::new(0.0, 3.0, 0.0),
                ],
                closed: true,
            })
            .unwrap();
        let bundle = DraftBundle {
            document: history.to_data(),
            ..Default::default()
        };
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("roundtrip.gltf");
        let summary = export_gltf(&path, &bundle).unwrap();
        assert_eq!(summary.triangles, 2);

        let control = ImportControl::default();
        let scene = import_gltf(&path, &control).unwrap();
        let result = convert_scene(
            scene,
            &ImportOptions {
                merge_coplanar: true,
                soften_coplanar: true,
                as_component: false,
            },
            &control,
        )
        .unwrap();
        assert_eq!(result.report.faces, 1);
        assert_eq!(result.report.vertices, 4);
    }
}
