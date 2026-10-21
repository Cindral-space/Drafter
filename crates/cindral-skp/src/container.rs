// Container layouts derived from OpenSKP (MIT).
// Copyright (c) 2024-2026 Ahsan Mehmood. See docs/LICENSES.md.
use crate::{legacy, modern, Error, Result, Scene, MAX_BYTES};
use std::{collections::BTreeMap, io::{Cursor, Read}};

/// Read a bounded SketchUp file buffer without filesystem or native runtime access.
pub fn read(bytes: &[u8]) -> Result<Scene> {
    if bytes.len() > MAX_BYTES { return Err(Error("SketchUp input exceeds 64 MiB limit".into())); }
    if !bytes.starts_with(&[0xff, 0xfe, 0xff, 0x0e]) {
        return Err(Error("Not a SketchUp file (invalid header)".into()));
    }
    let header = &bytes[..bytes.len().min(4096)];
    let version = version(header).ok_or_else(|| Error("Unsupported SketchUp version: unknown (missing version header)".into()))?;
    let major = version.trim_matches(['{', '}']).split('.').next().and_then(|s| s.parse::<u32>().ok())
        .ok_or_else(|| Error(format!("Unsupported SketchUp version {version}")))?;
    if !((3..=8).contains(&major) || (13..=26).contains(&major)) {
        return Err(Error(format!("Unsupported SketchUp version {version}; supported families are 3–8 / 13–20 (MFC) and 21–26 (VFF)")));
    }
    if major < 21 { return legacy::parse(bytes, major, &version); }
    let offset = header.windows(4).position(|w| w == b"PK\x03\x04")
        .ok_or_else(|| Error(format!("SketchUp {version}: missing VFF ZIP container")))?;
    let mut archive = zip::ZipArchive::new(Cursor::new(&bytes[offset..]))
        .map_err(|e| Error(format!("SketchUp {version}: invalid ZIP: {e}")))?;
    if archive.len() > 4096 { return Err(Error(format!("SketchUp {version}: ZIP entry count exceeds 4096"))); }
    let mut total = 0u64;
    let mut files = BTreeMap::new();
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).map_err(|e| Error(format!("Invalid ZIP entry: {e}")))?;
        total = total.checked_add(entry.size()).ok_or_else(|| Error("ZIP size overflow".into()))?;
        if total > MAX_BYTES as u64 || entry.name().len() > 4096 {
            return Err(Error("SketchUp ZIP expanded size/name limit exceeded (64 MiB)".into()));
        }
        // No extraction to disk, path resolution or resource fetching. Account for every
        // entry, even unused thumbnails, before accepting any payload.
        let name = entry.name().to_string();
        if files.contains_key(&name) { return Err(Error("Duplicate SketchUp ZIP entry".into())); }
        let mut payload = Vec::new();
        (&mut entry).take(MAX_BYTES as u64 + 1).read_to_end(&mut payload)
            .map_err(|e| Error(format!("Invalid SketchUp ZIP payload: {e}")))?;
        if payload.len() as u64 != entry.size() || payload.len() > MAX_BYTES {
            return Err(Error("SketchUp ZIP payload size mismatch/limit exceeded".into()));
        }
        files.insert(name, payload);
    }
    let key = files.keys().find(|name| name.eq_ignore_ascii_case("model.dat") || name.to_ascii_lowercase().ends_with("/model.dat"))
        .cloned().ok_or_else(|| Error(format!("SketchUp {version}: ZIP is missing model.dat")))?;
    let model = files.remove(&key).ok_or_else(|| Error("Missing model.dat".into()))?;
    modern::parse(&model, &files, &version)
}

fn version(header: &[u8]) -> Option<String> {
    let second = header.get(4..)?.windows(3).position(|w| w == [0xff, 0xfe, 0xff])? + 4;
    let start = second.checked_add(4)?;
    let text: Vec<u16> = header.get(start..header.len().min(start + 200))?.as_chunks::<2>().0.iter()
        .map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
    let text = String::from_utf16_lossy(&text);
    let begin = text.find('{')?;
    let end = text.get(begin..)?.find('}')? + begin;
    Some(text.get(begin..=end)?.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    fn header(version: &str) -> Vec<u8> {
        let mut bytes = vec![0xff,0xfe,0xff,0x0e,0,0,0,0,0,0,0xff,0xfe,0xff,version.len() as u8];
        bytes.extend(version.encode_utf16().flat_map(u16::to_le_bytes));
        bytes
    }
    #[test]
    fn named_version_failures() {
        for ver in ["{2.0.0}", "{27.0.1}", "{999999999999.0}"] {
            let error = read(&header(ver)).unwrap_err().to_string();
            assert!(error.contains(ver), "{error}");
        }
        assert!(read(b"PK\x03\x04").is_err());
    }
    #[test]
    fn missing_model_and_expansion_limit() {
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        writer.start_file("thumbnail", zip::write::SimpleFileOptions::default()).unwrap();
        writer.write_all(b"unused").unwrap();
        let mut bytes = header("{25.0.575}");
        bytes.extend(writer.finish().unwrap().into_inner());
        assert!(read(&bytes).unwrap_err().to_string().contains("model.dat"));
        // A hostile central-directory expansion claim is rejected before reading.
        let mut bomb = bytes.clone();
        let central = bomb.windows(4).position(|w| w == b"PK\x01\x02").unwrap();
        bomb[central+24..central+28].copy_from_slice(&(MAX_BYTES as u32 + 1).to_le_bytes());
        assert!(read(&bomb).is_err());
    }
    proptest::proptest! {
        #![proptest_config(proptest::test_runner::Config::with_cases(128))]
        #[test]
        fn arbitrary_input_is_bounded(bytes in proptest::collection::vec(proptest::prelude::any::<u8>(), 0..8192)) {
            let _ = read(&bytes);
            let mut candidate = header("{25.0.575}");
            candidate.extend(bytes);
            let _ = read(&candidate);
        }
    }
}
