//! Bounded reader for the modern (2021+) SketchUp file format `model.dat` TLV records.
//!
//! Derived from the record layout logic of OpenSKP (MIT License,
//! Copyright (c) 2024-2026 Ahsan Mehmood); see `docs/LICENSES.md` for the full licence.
//! This is an original, safe Rust implementation; no SDK, binaries or assets are used.
//!
//! Records are `u16` little-endian tag, `u32` little-endian length, payload. Only a fixed set of
//! known container tags is recursed into. Payloads stay borrowed from the input buffer.
use crate::{Definition, Error, Instance, Result, Scene, MAX_BYTES, MAX_DEPTH, MAX_ITEMS};
use cindral_core::{BulkEdge, BulkFace, BulkGeometry, DVec3, Material, MaterialId, Tag};
use glam::DMat4;
use std::collections::{BTreeMap, HashMap};

// Tag values are the little-endian u16 of the two on-disk bytes (bytes `F4 01` -> 0x01F4).
const T_WRAPPER: u16 = 0x01F4;
const T_DEF_LIST: u16 = 0x01F9;
const T_DEF_CHAIN: [u16; 2] = [0x1770, 0x1771];
const T_ROOT: u16 = 0x01F6;
const T_DEFINITION: u16 = 0x157C;
const T_DEF_NAME: u16 = 0x157E;
const T_VERTEX: u16 = 0x09C4;
const T_POSITION: u16 = 0x09C5;
const T_EDGE: u16 = 0x0BB8;
const T_EDGE_START: u16 = 0x0BB9;
const T_EDGE_END: u16 = 0x0BBA;
const T_FACE: u16 = 0x0DAC;
const T_FACE_LOOPS: u16 = 0x0DAE;
const T_FACE_BACK_MAT: u16 = 0x0DAF;
const T_LOOP: u16 = 0x1194;
const T_COEDGE: u16 = 0x0FA0;
const T_COEDGE_EDGE: u16 = 0x0FA1;
const T_COEDGE_ORIENT: u16 = 0x0FA2;
const T_INSTANCE: u16 = 0x1964;
const T_INST_NAME: u16 = 0x1965;
const T_INST_MATRIX: u16 = 0x1966;
const T_INST_DEF: u16 = 0x1967;
const T_ENTITY_INFO: u16 = 0x07D0;
const T_INFO_MATERIAL: u16 = 0x07D1;
const T_INFO_LAYER: u16 = 0x07D2;
const T_INFO_FLAGS: u16 = 0x07D3;
const T_ID: u16 = 0x05DE;
const T_ID_WRAPPER: u16 = 0x05DC;
const T_MATERIAL: u16 = 0x32C8;
const T_MATERIAL_NAME: u16 = 0x32CC;
const T_LAYER_LIST: u16 = 0x3A99;
const T_LAYER: u16 = 0x3C8C;
const T_LAYER_NAME: u16 = 0x3C8D;
const T_LAYER_HIDDEN: u16 = 0x3C8E;

/// Known container tags; every other tag is an opaque leaf.
const CONTAINERS: &[u16] = &[
    0x01F4, 0x01F7, 0x30D4, 0x30D5, 0x32C8, 0x157C, 0x1388, 0x1389, 0x138A, 0x138B, 0x138C, 0x138D,
    0x1D4C, 0x1964, 0x01F9, 0x1770, 0x1771, 0x07D0, 0x09C4, 0x1194, 0x1195, 0x010F, 0x4A38, 0x0BB8,
    0x1397, 0x4C2C, 0x0DAC, 0x0DAE, 0x01F6, 0x01F8, 0x3A98, 0x3A99, 0x3C8C, 0x1390, 0x1F40, 0x1391,
    0x1392, 0x4269, 0x426C, 0x4268,
];
const MAT_NS: &str = "http://sketchup.google.com/schemas/sketchup/1.0/material";
/// TLV nodes materialized for one top-level record (or one definition).
const MAX_RECORD_NODES: usize = 4 * MAX_ITEMS;
/// Aggregate TLV nodes across the file.
const MAX_TOTAL_NODES: usize = 16 * MAX_ITEMS;
const MAX_LOOP: usize = 65_536;

/// Borrowed TLV node.
struct Node<'a> {
    tag: u16,
    payload: &'a [u8],
    children: Vec<Node<'a>>,
}

fn header(data: &[u8], pos: usize) -> Option<(u16, usize, usize)> {
    let h = data.get(pos..pos.checked_add(6)?)?;
    let tag = u16::from_le_bytes([h[0], h[1]]);
    let size = u32::from_le_bytes([h[2], h[3], h[4], h[5]]) as usize;
    let end = pos.checked_add(6)?.checked_add(size)?;
    (end <= data.len()).then_some((tag, pos + 6, end))
}

/// Flat run of `(tag, payload)` siblings; stops at the first record that does not fit.
fn flat(data: &[u8]) -> Vec<(u16, &[u8])> {
    let mut out = Vec::new();
    let mut pos = 0;
    while let Some((tag, start, end)) = header(data, pos) {
        if out.len() >= MAX_ITEMS {
            break;
        }
        out.push((tag, &data[start..end]));
        pos = end;
    }
    out
}

struct Budget {
    record: usize,
    total: usize,
    exhausted: bool,
    too_deep: bool,
    malformed: bool,
}

fn parse_tree<'a>(data: &'a [u8], depth: usize, budget: &mut Budget) -> Vec<Node<'a>> {
    let mut out = Vec::new();
    let mut pos = 0;
    while let Some((tag, start, end)) = header(data, pos) {
        if budget.record == 0 || budget.total == 0 {
            budget.exhausted = true;
            break;
        }
        budget.record -= 1;
        budget.total -= 1;
        let payload = &data[start..end];
        let children = if !payload.is_empty() && CONTAINERS.contains(&tag) {
            if depth + 1 >= MAX_DEPTH {
                budget.too_deep = true;
                Vec::new()
            } else {
                parse_tree(payload, depth + 1, budget)
            }
        } else {
            Vec::new()
        };
        out.push(Node {
            tag,
            payload,
            children,
        });
        pos = end;
    }
    if pos != data.len() {
        budget.malformed = true;
    }
    out
}

fn var_int(bytes: &[u8]) -> Option<u64> {
    if bytes.len() > 8 {
        return None;
    }
    let mut buf = [0u8; 8];
    buf[..bytes.len()].copy_from_slice(bytes);
    Some(u64::from_le_bytes(buf))
}

/// Entity key in an `DC05` entity-info wrapper: either a nested `DE05` record or a bare integer.
fn wrapped_id(payload: &[u8]) -> Option<u64> {
    match header(payload, 0) {
        Some((T_ID, start, end)) => var_int(&payload[start..end]),
        _ if payload.starts_with(&T_ID.to_le_bytes()) => None,
        _ => var_int(payload),
    }
}

fn entity_id(node: &Node) -> Option<u64> {
    for child in &node.children {
        match child.tag {
            T_ID => return var_int(child.payload),
            T_ID_WRAPPER if child.payload.starts_with(&T_ID.to_le_bytes()) => {
                return header(child.payload, 0).and_then(|(_, s, e)| var_int(&child.payload[s..e]))
            }
            _ => {}
        }
    }
    node.children.iter().find_map(entity_id)
}

fn find<'n, 'a>(nodes: &'n [Node<'a>], tag: u16) -> Option<&'n Node<'a>> {
    for n in nodes {
        if n.tag == tag {
            return Some(n);
        }
        if let Some(found) = find(&n.children, tag) {
            return Some(found);
        }
    }
    None
}

fn find_all<'n, 'a>(nodes: &'n [Node<'a>], tag: u16, out: &mut Vec<&'n Node<'a>>) {
    for n in nodes {
        if n.tag == tag {
            out.push(n);
        }
        find_all(&n.children, tag, out);
    }
}

fn direct<'n, 'a>(nodes: &'n [Node<'a>], tag: u16) -> Option<&'n Node<'a>> {
    nodes.iter().find(|n| n.tag == tag)
}

fn f64s<const N: usize>(bytes: &[u8]) -> Option<[f64; N]> {
    let mut out = [0.0; N];
    for (i, v) in out.iter_mut().enumerate() {
        let b = bytes.get(i * 8..i * 8 + 8)?;
        *v = f64::from_le_bytes(b.try_into().ok()?);
    }
    Some(out)
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(&bytes[..bytes.len().min(512)]).into_owned()
}

#[derive(Default)]
struct RawFace {
    id: u64,
    loops: Vec<Vec<(u64, bool)>>,
    material: Option<u64>,
    layer: Option<u64>,
    hidden: bool,
}

struct RawInstance {
    name: String,
    definition: u64,
    matrix: Option<[f64; 13]>,
    layer: Option<u64>,
    material: Option<u64>,
    hidden: bool,
}

#[derive(Default)]
struct Builder {
    vertices: Vec<(u64, DVec3)>,
    edges: Vec<(u64, u64, u64, u8, Option<u64>)>,
    faces: Vec<RawFace>,
    instances: Vec<RawInstance>,
}

struct RawDef {
    name: String,
    builder: Builder,
}

struct Reader {
    scene: Scene,
    defs: Vec<RawDef>,
    def_by_id: HashMap<u64, usize>,
    material_names: HashMap<u64, String>,
    layers: Vec<(u64, String, bool)>,
    root: Builder,
    items: usize,
    capped: bool,
    total_nodes: usize,
    work: usize,
    work_exhausted: bool,
}

impl Reader {
    /// Reserve aggregate work; returns false (once warning) when the item cap is reached.
    fn visit(&mut self) -> bool {
        if self.work == 0 {
            if !self.work_exhausted {
                self.work_exhausted = true;
                self.scene
                    .warn("TLV traversal work limit reached; remaining records skipped");
            }
            false
        } else {
            self.work -= 1;
            true
        }
    }
    fn take(&mut self, n: usize) -> bool {
        if self.items.saturating_add(n) > MAX_ITEMS {
            if !self.capped {
                self.capped = true;
                self.scene
                    .warn("item limit reached; remaining geometry skipped");
            }
            return false;
        }
        self.items += n;
        true
    }

    fn record(&mut self, data: &[u8], tag: u16) {
        let mut budget = Budget {
            record: MAX_RECORD_NODES,
            total: self.total_nodes,
            exhausted: false,
            too_deep: false,
            malformed: false,
        };
        let nodes = parse_tree(data, 0, &mut budget);
        self.total_nodes = budget.total;
        if budget.malformed {
            self.scene
                .warn(format!("record {tag:#06x}: truncated TLV siblings skipped"));
        }
        if budget.too_deep {
            self.scene.warn(format!(
                "record {tag:#06x}: nesting exceeds limit; truncated"
            ));
        }
        if budget.exhausted {
            self.scene
                .warn(format!("record {tag:#06x}: node limit reached; skipped"));
            return;
        }
        self.walk(&nodes);
        if let Some(root) = nodes.first().filter(|n| n.tag == T_ROOT) {
            let mut builder = std::mem::take(&mut self.root);
            self.extract(&root.children, &mut builder);
            self.root = builder;
        }
    }

    fn walk(&mut self, nodes: &[Node]) {
        for el in nodes {
            if !self.visit() {
                return;
            }
            match el.tag {
                T_LAYER_LIST => {
                    for layer in el.children.iter().filter(|c| c.tag == T_LAYER) {
                        let id =
                            find(&layer.children, T_ID_WRAPPER).and_then(|n| wrapped_id(n.payload));
                        let name = find(&layer.children, T_LAYER_NAME);
                        if let (Some(id), Some(name)) = (id, name) {
                            let hidden = find(&layer.children, T_LAYER_HIDDEN)
                                .and_then(|n| n.payload.first())
                                .is_some_and(|b| *b == 1);
                            if self.layers.len() < MAX_ITEMS {
                                self.layers.push((id, text(name.payload), hidden));
                            }
                        }
                    }
                }
                T_MATERIAL => {
                    let id = find(&el.children, T_ID_WRAPPER).and_then(|n| wrapped_id(n.payload));
                    if let (Some(id), Some(name)) = (id, find(&el.children, T_MATERIAL_NAME)) {
                        if self.material_names.len() < MAX_ITEMS {
                            self.material_names.insert(id, text(name.payload));
                        }
                    }
                }
                T_DEFINITION => self.definition(el),
                _ => {}
            }
            if !el.children.is_empty() {
                self.walk(&el.children);
            }
        }
    }

    fn definition(&mut self, el: &Node) {
        let Some(id) = entity_id(el) else {
            self.scene.warn("definition without entity id skipped");
            return;
        };
        if !self.take(1) {
            return;
        }
        let name = direct(&el.children, T_DEF_NAME)
            .map(|n| text(n.payload))
            .unwrap_or_default();
        let mut builder = Builder::default();
        self.extract(&el.children, &mut builder);
        if self.def_by_id.insert(id, self.defs.len()).is_some() {
            self.scene
                .warn(format!("duplicate definition id {id}; later record wins"));
        }
        self.defs.push(RawDef { name, builder });
    }

    fn extract(&mut self, elements: &[Node], b: &mut Builder) {
        for el in elements {
            if !self.visit() {
                return;
            }
            match el.tag {
                T_VERTEX => {
                    let pos = find(&el.children, T_POSITION).and_then(|n| f64s::<3>(n.payload));
                    match (entity_id(el), pos) {
                        (Some(id), Some(p)) if p.iter().all(|v| v.is_finite()) => {
                            if self.take(1) {
                                b.vertices.push((id, DVec3::from_array(p)));
                            }
                        }
                        _ => self.scene.warn("malformed vertex skipped"),
                    }
                }
                T_EDGE => {
                    let id = entity_id(el);
                    let v1 = find(&el.children, T_EDGE_START).and_then(|n| var_int(n.payload));
                    let v2 = find(&el.children, T_EDGE_END).and_then(|n| var_int(n.payload));
                    let flags = direct(&el.children, T_ENTITY_INFO)
                        .and_then(|i| direct(&i.children, T_INFO_FLAGS))
                        .and_then(|n| n.payload.first().copied())
                        .unwrap_or(0x06);
                    match (id, v1, v2) {
                        (Some(id), Some(v1), Some(v2)) => {
                            if self.take(1) {
                                let layer = direct(&el.children, T_ENTITY_INFO)
                                    .and_then(|i| direct(&i.children, T_INFO_LAYER))
                                    .and_then(|n| var_int(n.payload));
                                b.edges.push((id, v1, v2, flags, layer));
                            }
                        }
                        _ => self.scene.warn("malformed edge skipped"),
                    }
                }
                T_FACE => self.face(el, b),
                T_INSTANCE => self.instance(el, b),
                _ => {
                    if !el.children.is_empty() {
                        self.extract(&el.children, b);
                    }
                }
            }
        }
    }

    fn face(&mut self, el: &Node, b: &mut Builder) {
        if entity_id(el).is_none() {
            self.scene.warn("face without entity id skipped");
            return;
        }
        let mut face = RawFace {
            id: entity_id(el).unwrap_or_default(),
            ..Default::default()
        };
        if let Some(loops) = find(&el.children, T_FACE_LOOPS) {
            let mut loop_nodes = Vec::new();
            find_all(&loops.children, T_LOOP, &mut loop_nodes);
            for ln in loop_nodes {
                let mut co_nodes = Vec::new();
                find_all(&ln.children, T_COEDGE, &mut co_nodes);
                if co_nodes.len() > MAX_LOOP || !self.take(co_nodes.len()) {
                    self.scene.warn("face loop exceeds limits; face skipped");
                    return;
                }
                let mut coedges = Vec::with_capacity(co_nodes.len());
                for cn in co_nodes {
                    let (mut edge, mut orient) = (None, None);
                    for (tag, payload) in flat(cn.payload) {
                        match tag {
                            T_COEDGE_EDGE => edge = var_int(payload),
                            T_COEDGE_ORIENT => orient = var_int(payload),
                            _ => {}
                        }
                    }
                    // Source semantics: 0 = same direction as the edge, otherwise reversed.
                    if let (Some(edge), Some(orient)) = (edge, orient) {
                        coedges.push((edge, orient == 0));
                    } else {
                        self.scene.warn("malformed coedge; entire face skipped");
                        return;
                    }
                }
                if !coedges.is_empty() {
                    face.loops.push(coedges);
                }
            }
        }
        if face.loops.is_empty() {
            self.scene.warn("face without loops skipped");
            return;
        }
        if let Some(info) = direct(&el.children, T_ENTITY_INFO) {
            face.material =
                direct(&info.children, T_INFO_MATERIAL).and_then(|n| var_int(n.payload));
            face.layer = direct(&info.children, T_INFO_LAYER).and_then(|n| var_int(n.payload));
            face.hidden = direct(&info.children, T_INFO_FLAGS)
                .and_then(|n| n.payload.first())
                .is_some_and(|flag| flag & 1 != 0);
        }
        if face.material == Some(0) {
            face.material = None;
        }
        if face.material.is_none() {
            face.material = direct(&el.children, T_FACE_BACK_MAT)
                .filter(|n| !n.payload.is_empty())
                .and_then(|n| var_int(n.payload));
        }
        b.faces.push(face);
    }

    fn instance(&mut self, el: &Node, b: &mut Builder) {
        let search: &[Node] = &el.children;
        let Some(definition) = find(search, T_INST_DEF).and_then(|n| var_int(n.payload)) else {
            self.scene
                .warn("instance without definition reference skipped");
            return;
        };
        if !self.take(1) {
            return;
        }
        let name = find(search, T_INST_NAME)
            .map(|n| text(n.payload))
            .unwrap_or_default();
        let matrix = find(search, T_INST_MATRIX).and_then(|n| f64s::<13>(n.payload));
        let layer = direct(search, T_ENTITY_INFO)
            .and_then(|i| direct(&i.children, T_INFO_LAYER))
            .filter(|n| !n.payload.is_empty())
            .and_then(|n| var_int(n.payload));
        let info = direct(search, T_ENTITY_INFO);
        let material = info
            .and_then(|i| direct(&i.children, T_INFO_MATERIAL))
            .and_then(|n| var_int(n.payload));
        let hidden = info
            .and_then(|i| direct(&i.children, T_INFO_FLAGS))
            .and_then(|n| n.payload.first())
            .is_some_and(|flag| flag & 1 != 0);
        b.instances.push(RawInstance {
            name,
            definition,
            matrix,
            layer,
            material,
            hidden,
        });
    }

    fn top_level(&mut self, data: &[u8]) {
        let mut headers = flat(data);
        if headers.iter().map(|(_, p)| p.len() + 6).sum::<usize>() != data.len() {
            self.scene.warn("truncated top-level TLV records skipped");
        }
        if let [(T_WRAPPER, inner)] = headers.as_slice() {
            let inner = *inner;
            headers = flat(inner);
            if headers.iter().map(|(_, p)| p.len() + 6).sum::<usize>() != inner.len() {
                self.scene.warn("truncated model wrapper records skipped");
            }
        }
        for (tag, payload) in headers {
            if tag == T_DEF_LIST {
                if let Some(defs) = unwrap_definitions(payload) {
                    for def in defs {
                        self.record(def, T_DEFINITION);
                    }
                    continue;
                }
            }
            // Re-frame the record so that its own tag is part of the parsed tree.
            let start = payload.as_ptr() as usize - data.as_ptr() as usize;
            match data.get(start.saturating_sub(6)..start + payload.len()) {
                Some(full) if start >= 6 => self.record(full, tag),
                _ => self
                    .scene
                    .warn(format!("record {tag:#06x} unframed; skipped")),
            }
        }
    }
}

/// Unwrap `F901 -> 7017 -> 7117 -> [7C15...]` into one framed slice per definition.
fn unwrap_definitions(payload: &[u8]) -> Option<Vec<&[u8]>> {
    let mut level = payload;
    for want in T_DEF_CHAIN {
        level = flat(level).into_iter().find(|(t, _)| *t == want)?.1;
    }
    let mut defs = Vec::new();
    let mut pos = 0;
    while let Some((tag, _, end)) = header(level, pos) {
        if tag == T_DEFINITION {
            defs.push(&level[pos..end]);
        }
        pos = end;
    }
    (!defs.is_empty()).then_some(defs)
}

/// Parse `materials/*/material.xml`; returns material-name -> palette index plus display colors.
fn preflight_xml(xml: &str) -> Result<()> {
    let bytes = xml.as_bytes();
    let mut at = 0;
    let mut depth = 0usize;
    let mut count = 0usize;
    while at < bytes.len() {
        let Some(relative) = bytes[at..].iter().position(|b| *b == b'<') else {
            break;
        };
        at += relative;
        let tail = &bytes[at..];
        let terminator: Option<&[u8]> = if tail.starts_with(b"<!--") {
            Some(b"-->")
        } else if tail.starts_with(b"<![CDATA[") {
            Some(b"]]>")
        } else if tail.starts_with(b"<?") {
            Some(b"?>")
        } else {
            None
        };
        if let Some(end) = terminator {
            let relative = tail
                .windows(end.len())
                .position(|w| w == end)
                .ok_or_else(|| Error("unterminated XML markup".into()))?;
            at += relative + end.len();
            continue;
        }
        if tail.starts_with(b"<!") {
            return Err(Error("XML declarations are disabled".into()));
        }
        let closing = tail.starts_with(b"</");
        let mut end = at + 1;
        let mut quote = None;
        while end < bytes.len() {
            let b = bytes[end];
            if let Some(q) = quote {
                if b == q {
                    quote = None;
                }
            } else if b == b'\'' || b == b'"' {
                quote = Some(b);
            } else if b == b'>' {
                break;
            }
            end += 1;
        }
        if end == bytes.len() {
            return Err(Error("unterminated XML tag".into()));
        }
        if closing {
            depth = depth.saturating_sub(1);
        } else {
            count += 1;
            if count > 100_000 {
                return Err(Error("XML element count exceeds limit".into()));
            }
            let self_closing = bytes[at + 1..end]
                .iter()
                .rev()
                .find(|b| !b.is_ascii_whitespace())
                == Some(&b'/');
            if depth + 1 > MAX_DEPTH {
                return Err(Error("XML depth exceeds 64".into()));
            }
            if !self_closing {
                depth += 1;
            }
        }
        at = end + 1;
    }
    Ok(())
}

fn load_materials(
    files: &BTreeMap<String, Vec<u8>>,
    scene: &mut Scene,
) -> HashMap<String, (usize, [u8; 4])> {
    let mut by_name = HashMap::new();
    let mut texture_bytes = 0usize;
    for (path, bytes) in files {
        if !(path.starts_with("materials/") && path.ends_with("material.xml")) {
            continue;
        }
        let Some(xml) = std::str::from_utf8(bytes)
            .ok()
            .map(|s| s.trim_start_matches('\u{feff}'))
        else {
            scene.warn(format!("{path}: material XML is not UTF-8"));
            continue;
        };
        if bytes.len() > 1024 * 1024 || preflight_xml(xml).is_err() {
            scene.warn(format!(
                "{path}: XML size/depth/declaration limit; material skipped"
            ));
            continue;
        }
        let doc = match roxmltree::Document::parse(xml) {
            Ok(doc) => doc,
            Err(e) => {
                scene.warn(format!("{path}: {e}"));
                continue;
            }
        };
        let Some(mat) = doc
            .descendants()
            .find(|n| n.has_tag_name((MAT_NS, "material")))
            .or_else(|| {
                doc.descendants()
                    .find(|n| n.tag_name().name() == "material")
            })
        else {
            continue;
        };
        let channel = |key: &str| {
            mat.attribute(key)
                .and_then(|v| v.trim().parse::<i64>().ok())
                .map_or(128, |v| v.clamp(0, 255) as u8)
        };
        let rgb = [
            channel("colorRed"),
            channel("colorGreen"),
            channel("colorBlue"),
        ];
        let opacity = if mat.attribute("useTrans") == Some("1") {
            let t = mat
                .attribute("trans")
                .and_then(|v| v.trim().parse::<f32>().ok())
                .unwrap_or(0.0);
            if t.is_finite() {
                (1.0 - t).clamp(0.0, 1.0)
            } else {
                1.0
            }
        } else {
            1.0
        };
        let name = mat.attribute("name").unwrap_or("unknown").to_owned();
        let folder = path.rsplit_once('/').map_or("", |(f, _)| f);
        let texture = mat
            .children()
            .find(|n| n.tag_name().name() == "texture")
            .and_then(|tex| {
                let filename = tex.attribute("textureFilename").unwrap_or("");
                let candidate = format!("{folder}/{filename}");
                if !filename.is_empty() && files.contains_key(&candidate) {
                    return Some(candidate);
                }
                let prefix = format!("{folder}/");
                if let Some(entry) = files.keys().find(|k| {
                    k.starts_with(&prefix)
                        && *k != path
                        && !k.to_ascii_lowercase().ends_with(".xml")
                }) {
                    return Some(entry.clone());
                }
                let image = tex
                    .descendants()
                    .find(|n| n.tag_name().name() == "image")
                    .and_then(|n| n.attribute("path"))
                    .unwrap_or("")
                    .trim_start_matches(['.', '/']);
                [image.to_owned(), format!("{folder}/{image}")]
                    .into_iter()
                    .find(|c| !image.is_empty() && files.contains_key(c))
            });
        if let Some(reference) = &texture {
            if let Some(data) = files.get(reference) {
                if !scene.textures.contains_key(reference) {
                    if texture_bytes.saturating_add(data.len()) <= MAX_BYTES {
                        texture_bytes += data.len();
                        scene.textures.insert(reference.clone(), data.clone());
                    } else {
                        scene.warn(format!("{reference}: texture byte limit reached"));
                    }
                }
            }
        }
        if by_name.contains_key(&name) || scene.materials.len() >= MAX_ITEMS {
            continue;
        }
        let alpha = (opacity * 255.0).round() as u8;
        by_name.insert(
            name.clone(),
            (scene.materials.len(), [rgb[0], rgb[1], rgb[2], alpha]),
        );
        scene.materials.push(Material {
            name,
            color: rgb,
            opacity,
            texture,
        });
    }
    by_name
}

fn transform(m: &[f64; 13]) -> Option<DMat4> {
    // Reference convention: row-major 3x3 then translation; p' = R * p + t; m[12] is not applied.
    m.iter().all(|v| v.is_finite()).then(|| {
        DMat4::from_cols_array(&[
            m[0], m[3], m[6], 0.0, m[1], m[4], m[7], 0.0, m[2], m[5], m[8], 0.0, m[9], m[10],
            m[11], 1.0,
        ])
    })
}

fn polygon_area(points: &[DVec3]) -> f64 {
    let mut n = DVec3::ZERO;
    for (i, a) in points.iter().enumerate() {
        n += a.cross(points[(i + 1) % points.len()]);
    }
    n.length()
}

struct Resolver<'r> {
    def_by_id: &'r HashMap<u64, usize>,
    material_names: &'r HashMap<u64, String>,
    materials: &'r HashMap<String, (usize, [u8; 4])>,
    layer_index: &'r HashMap<u64, usize>,
}

fn build(b: Builder, name: String, r: &Resolver, scene: &mut Scene) -> Definition {
    let mut geometry = BulkGeometry::default();
    let mut face_assignments = BTreeMap::new();
    let mut face_ids = BTreeMap::new();
    let mut edge_tags = BTreeMap::new();
    let mut vertex_index = HashMap::with_capacity(b.vertices.len());
    for (id, p) in b.vertices {
        vertex_index.insert(id, geometry.vertices.len());
        geometry.vertices.push(p);
    }
    let mut edge_ends = HashMap::with_capacity(b.edges.len());
    for (id, v1, v2, flags, layer) in b.edges {
        match (vertex_index.get(&v1), vertex_index.get(&v2)) {
            (Some(&a), Some(&c)) => {
                edge_ends.insert(id, (a, c));
                if a != c {
                    // 0x01 hidden, 0x08 soft, 0x10 smooth.
                    if let Some(tag) = layer.and_then(|id| r.layer_index.get(&id)).copied() {
                        edge_tags.insert(geometry.edges.len(), tag);
                    }
                    geometry.edges.push(BulkEdge {
                        vertices: [a, c],
                        soft: flags & 0x08 != 0,
                        smooth: flags & 0x10 != 0,
                        hidden: flags & 0x01 != 0,
                    });
                }
            }
            _ => scene.warn(format!("edge {id}: unknown vertex; skipped")),
        }
    }
    'faces: for face in b.faces {
        let mut rings: Vec<Vec<usize>> = Vec::with_capacity(face.loops.len());
        for coedges in &face.loops {
            let mut ring = Vec::with_capacity(coedges.len());
            let mut expected = None;
            for &(edge, forward) in coedges {
                let Some(&(a, c)) = edge_ends.get(&edge) else {
                    scene.warn(format!(
                        "entity {}, definition {name}: face loop references unknown edge {edge}; face skipped", face.id
                    ));
                    continue 'faces;
                };
                let start = if forward { a } else { c };
                if expected.is_some_and(|end| end != start) {
                    scene.warn(format!(
                        "entity {}, definition {name}: disconnected coedge loop; face skipped",
                        face.id
                    ));
                    continue 'faces;
                }
                expected = Some(if forward { c } else { a });
                if ring.last() != Some(&start) {
                    ring.push(start);
                }
            }
            if expected != ring.first().copied() {
                scene.warn(format!(
                    "entity {}, definition {name}: unclosed coedge loop; face skipped",
                    face.id
                ));
                continue 'faces;
            }
            if ring.len() > 1 && ring.first() == ring.last() {
                ring.pop();
            }
            if ring.len() >= 3 {
                rings.push(ring);
            } else {
                scene.warn(format!(
                    "entity {}, definition {name}: degenerate face loop; entire face skipped",
                    face.id
                ));
                continue 'faces;
            }
        }
        if rings.is_empty() {
            scene.warn(format!(
                "entity {}, definition {name}: face without valid loops skipped",
                face.id
            ));
            continue;
        }
        let outer = rings
            .iter()
            .enumerate()
            .map(|(i, ring)| {
                let pts: Vec<DVec3> = ring.iter().map(|&v| geometry.vertices[v]).collect();
                (i, polygon_area(&pts))
            })
            .fold(
                (0, -1.0),
                |best, cur| if cur.1 > best.1 { cur } else { best },
            )
            .0;
        let outer = rings.remove(outer);
        let resolved = face
            .material
            .and_then(|id| r.material_names.get(&id))
            .and_then(|name| r.materials.get(name));
        face_assignments.insert(
            geometry.faces.len(),
            (
                face.layer.and_then(|id| r.layer_index.get(&id)).copied(),
                face.hidden,
            ),
        );
        face_ids.insert(geometry.faces.len(), face.id);
        geometry.faces.push(BulkFace {
            outer,
            holes: rings,
            material: resolved.map(|(i, _)| MaterialId(*i as u64 + 1)),
            color: resolved.map_or([255; 4], |(_, c)| *c),
        });
    }
    let mut instances = Vec::with_capacity(b.instances.len());
    for inst in b.instances {
        let Some(&definition) = r.def_by_id.get(&inst.definition) else {
            scene.warn(format!(
                "instance references unknown definition {}; skipped",
                inst.definition
            ));
            continue;
        };
        let transform = match inst.matrix.as_ref().map(transform) {
            Some(Some(m)) => m,
            Some(None) => {
                scene.warn("instance with non-finite transform skipped");
                continue;
            }
            None => DMat4::IDENTITY,
        };
        let tag = inst.layer.and_then(|l| r.layer_index.get(&l).copied());
        let material = inst
            .material
            .and_then(|id| r.material_names.get(&id))
            .and_then(|name| r.materials.get(name))
            .map(|(index, _)| MaterialId(*index as u64 + 1));
        instances.push(Instance {
            name: inst.name,
            definition,
            transform,
            tag,
            material,
            hidden: inst.hidden,
        });
    }
    Definition {
        name,
        geometry,
        instances,
        face_assignments,
        face_ids,
        edge_tags,
    }
}

/// Drop instance edges that close a definition cycle.
fn break_cycles(defs: &mut [Definition], scene: &mut Scene) {
    let n = defs.len();
    let mut state = vec![0u8; n];
    let mut removals = Vec::new();
    for start in 0..n {
        if state[start] != 0 {
            continue;
        }
        let mut stack = vec![(start, 0usize)];
        state[start] = 1;
        while let Some((d, i)) = stack.pop() {
            if let Some(child) = defs[d].instances.get(i).map(|x| x.definition) {
                stack.push((d, i + 1));
                match state.get(child) {
                    Some(0) => {
                        state[child] = 1;
                        stack.push((child, 0));
                    }
                    Some(1) | None => removals.push((d, i)),
                    _ => {}
                }
            } else {
                state[d] = 2;
            }
        }
    }
    for (d, i) in removals.into_iter().rev() {
        defs[d].instances.remove(i);
        scene.warn(format!("recursive placement in definition {d} removed"));
    }
}

/// Parse an already-extracted modern `model.dat` plus the other archive entries.
pub(crate) fn parse(
    model_data: &[u8],
    files: &BTreeMap<String, Vec<u8>>,
    version: &str,
) -> Result<Scene> {
    if model_data.len() > MAX_BYTES {
        return Err(Error(format!(
            "SketchUp file {version}: model.dat exceeds {MAX_BYTES} bytes"
        )));
    }
    if header(model_data, 0).is_none() {
        return Err(Error(format!(
            "SketchUp file {version}: model.dat has no readable records"
        )));
    }
    let mut scene = Scene {
        version: version.to_owned(),
        ..Scene::default()
    };
    let materials = load_materials(files, &mut scene);
    let mut reader = Reader {
        scene,
        defs: Vec::new(),
        def_by_id: HashMap::new(),
        material_names: HashMap::new(),
        layers: Vec::new(),
        root: Builder::default(),
        items: 0,
        capped: false,
        total_nodes: MAX_TOTAL_NODES,
        work: MAX_TOTAL_NODES,
        work_exhausted: false,
    };
    reader.top_level(model_data);
    let Reader {
        mut scene,
        defs,
        def_by_id,
        material_names,
        layers,
        root,
        ..
    } = reader;
    let mut layer_index = HashMap::new();
    for (id, name, hidden) in layers {
        if layer_index.contains_key(&id) {
            continue;
        }
        layer_index.insert(id, scene.tags.len());
        scene.tags.push(Tag {
            name,
            visible: !hidden,
        });
    }
    let resolver = Resolver {
        def_by_id: &def_by_id,
        material_names: &material_names,
        materials: &materials,
        layer_index: &layer_index,
    };
    let mut definitions = Vec::with_capacity(defs.len() + 1);
    for def in defs {
        definitions.push(build(def.builder, def.name, &resolver, &mut scene));
    }
    let root = build(root, "Model".to_owned(), &resolver, &mut scene);
    break_cycles(&mut definitions, &mut scene);
    scene.roots = vec![Instance {
        name: "Model".into(),
        definition: definitions.len(),
        transform: DMat4::IDENTITY,
        tag: None,
        material: None,
        hidden: false,
    }];
    definitions.push(root);
    scene.definitions = definitions;
    Ok(scene)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(tag: u16, payload: &[u8]) -> Vec<u8> {
        let mut out = tag.to_le_bytes().to_vec();
        out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        out.extend_from_slice(payload);
        out
    }
    fn cat(parts: &[Vec<u8>]) -> Vec<u8> {
        parts.concat()
    }
    fn id(v: u64) -> Vec<u8> {
        rec(T_ID, &(v as u32).to_le_bytes())
    }
    fn vertex(v: u64, p: [f64; 3]) -> Vec<u8> {
        let pos: Vec<u8> = p.iter().flat_map(|c| c.to_le_bytes()).collect();
        rec(T_VERTEX, &cat(&[id(v), rec(T_POSITION, &pos)]))
    }
    fn edge(e: u64, a: u64, b: u64, flags: u8) -> Vec<u8> {
        rec(
            T_EDGE,
            &cat(&[
                id(e),
                rec(T_EDGE_START, &(a as u32).to_le_bytes()),
                rec(T_EDGE_END, &(b as u32).to_le_bytes()),
                rec(T_ENTITY_INFO, &rec(T_INFO_FLAGS, &[flags])),
            ]),
        )
    }
    fn lp(coedges: &[(u64, u8)]) -> Vec<u8> {
        let parts: Vec<Vec<u8>> = coedges
            .iter()
            .map(|&(e, o)| {
                rec(
                    T_COEDGE,
                    &cat(&[
                        rec(T_COEDGE_EDGE, &(e as u32).to_le_bytes()),
                        rec(T_COEDGE_ORIENT, &[o]),
                    ]),
                )
            })
            .collect();
        rec(T_LOOP, &cat(&parts))
    }
    fn face(f: u64, loops: &[Vec<u8>], material: Option<u8>) -> Vec<u8> {
        let mut parts = vec![id(f), rec(T_FACE_LOOPS, &cat(loops))];
        if let Some(m) = material {
            parts.push(rec(T_ENTITY_INFO, &rec(T_INFO_MATERIAL, &[m])));
        }
        rec(T_FACE, &cat(&parts))
    }
    fn wrapped(v: u8) -> Vec<u8> {
        rec(T_ID_WRAPPER, &rec(T_ID, &[v]))
    }
    /// Square of side `s` at offset `o`, ids starting at `base`.
    fn square(base: u64, o: f64, s: f64, reversed: bool) -> (Vec<u8>, Vec<u8>) {
        let pts = [
            [o, o, 0.0],
            [o + s, o, 0.0],
            [o + s, o + s, 0.0],
            [o, o + s, 0.0],
        ];
        let mut geo = Vec::new();
        for (i, p) in pts.iter().enumerate() {
            geo.extend(vertex(base + i as u64, *p));
        }
        for i in 0..4u64 {
            geo.extend(edge(
                base + 10 + i,
                base + i,
                base + (i + 1) % 4,
                if i == 0 { 0x07 } else { 0x06 },
            ));
        }
        let coedges: Vec<(u64, u8)> = if reversed {
            (0..4u64).rev().map(|i| (base + 10 + i, 1)).collect()
        } else {
            (0..4u64).map(|i| (base + 10 + i, 0)).collect()
        };
        (geo, lp(&coedges))
    }

    fn sample() -> (Vec<u8>, BTreeMap<String, Vec<u8>>) {
        let (outer_geo, outer_loop) = square(100, 0.0, 10.0, false);
        let (inner_geo, inner_loop) = square(200, 3.0, 2.0, true);
        let (def_geo, def_loop) = square(300, 0.0, 1.0, false);
        let materials = rec(
            T_MATERIAL,
            &cat(&[wrapped(7), rec(T_MATERIAL_NAME, b"Brick")]),
        );
        let layers = rec(
            T_LAYER_LIST,
            &cat(&[rec(
                T_LAYER,
                &cat(&[
                    wrapped(9),
                    rec(T_LAYER_NAME, b"Walls"),
                    rec(T_LAYER_HIDDEN, &[1]),
                ]),
            )]),
        );
        let definition = rec(
            T_DEFINITION,
            &cat(&[
                id(50),
                rec(T_DEF_NAME, b"Box"),
                def_geo,
                face(51, &[def_loop], None),
            ]),
        );
        let def_list = rec(T_DEF_LIST, &rec(0x1770, &rec(0x1771, &definition)));
        let mut matrix = [0.0f64; 13];
        matrix[0] = 0.0;
        matrix[1] = -1.0;
        matrix[3] = 1.0;
        matrix[8] = 1.0;
        matrix[9] = 5.0;
        matrix[12] = 1.0;
        let m: Vec<u8> = matrix.iter().flat_map(|v| v.to_le_bytes()).collect();
        let instance = rec(
            T_INSTANCE,
            &cat(&[
                rec(T_INST_DEF, &[50]),
                rec(T_INST_NAME, b"Box#1"),
                rec(T_INST_MATRIX, &m),
                rec(T_ENTITY_INFO, &rec(T_INFO_LAYER, &[9])),
            ]),
        );
        let bad_face = face(99, &[lp(&[(999, 0), (998, 0), (997, 0)])], None);
        let root = rec(
            T_ROOT,
            &cat(&[
                outer_geo,
                inner_geo,
                face(1, &[inner_loop, outer_loop], Some(7)),
                bad_face,
                instance,
            ]),
        );
        let model = rec(T_WRAPPER, &cat(&[materials, layers, def_list, root]));
        let xml = format!(
            r#"<?xml version="1.0"?><m:materialDocument xmlns:m="{MAT_NS}"><m:material name="Brick" colorRed="255" colorGreen="0" colorBlue="0" useTrans="1" trans="0.25"><m:texture textureFilename="b.png"/></m:material></m:materialDocument>"#
        );
        let mut files = BTreeMap::new();
        files.insert("materials/Brick/material.xml".to_owned(), xml.into_bytes());
        files.insert("materials/Brick/b.png".to_owned(), b"PNGDATA".to_vec());
        (model, files)
    }

    #[test]
    fn synthetic_scene() {
        let (model, files) = sample();
        let scene = parse(&model, &files, "{21.0.0}").expect("parse");
        assert_eq!(scene.version, "{21.0.0}");
        assert_eq!(scene.definitions.len(), 2);
        assert_eq!(scene.definitions[0].name, "Box");
        assert_eq!(scene.definitions[0].geometry.faces.len(), 1);
        let root = &scene.definitions[1].geometry;
        assert_eq!(
            root.faces.len(),
            1,
            "malformed face skipped, valid face kept"
        );
        let f = &root.faces[0];
        assert_eq!(f.outer.len(), 4);
        assert_eq!(f.holes.len(), 1);
        assert_eq!(root.vertices[f.outer[1]], DVec3::new(10.0, 0.0, 0.0));
        assert_eq!(root.vertices[f.holes[0][0]], DVec3::new(3.0, 3.0, 0.0));
        assert_eq!(root.vertices[f.holes[0][1]], DVec3::new(3.0, 5.0, 0.0));
        assert_eq!(f.material, Some(MaterialId(1)));
        assert_eq!(f.color, [255, 0, 0, 191]);
        assert_eq!(root.edges.iter().filter(|e| e.hidden).count(), 2);
        assert!(scene.skipped >= 1 && !scene.warnings.is_empty());
        assert_eq!(
            scene.materials[0].texture.as_deref(),
            Some("materials/Brick/b.png")
        );
        assert!((scene.materials[0].opacity - 0.75).abs() < 1e-6);
        assert_eq!(scene.textures["materials/Brick/b.png"], b"PNGDATA");
        assert_eq!(
            scene.tags,
            vec![Tag {
                name: "Walls".into(),
                visible: false
            }]
        );
        assert_eq!(scene.roots.len(), 1);
        let inst = &scene.definitions[scene.roots[0].definition].instances[0];
        assert_eq!(
            (inst.definition, inst.tag, inst.name.as_str()),
            (0, Some(0), "Box#1")
        );
        let p = inst
            .transform
            .transform_point3(glam::DVec3::new(1.0, 0.0, 0.0));
        assert!((p - glam::DVec3::new(5.0, 1.0, 0.0)).length() < 1e-12);
        assert_eq!(scene.roots[0].definition, 1);
    }

    #[test]
    fn cycles_are_removed() {
        let inst = |d: u8| rec(T_INSTANCE, &rec(T_INST_DEF, &[d]));
        let a = rec(T_DEFINITION, &cat(&[id(1), inst(2)]));
        let b = rec(T_DEFINITION, &cat(&[id(2), inst(1)]));
        let scene = parse(&cat(&[a, b]), &BTreeMap::new(), "v").expect("parse");
        let total: usize = scene.definitions.iter().map(|d| d.instances.len()).sum();
        assert_eq!(total, 1);
        assert!(scene.skipped >= 1);
    }

    #[test]
    fn hostile_inputs_never_panic() {
        let (model, files) = sample();
        assert!(parse(&[], &files, "v").is_err());
        assert!(parse(&[1, 2, 3], &files, "v").is_err());
        let mut seed = 0x9E37_79B9_7F4A_7C15u64;
        for cut in (0..model.len()).step_by(7) {
            let _ = parse(&model[..cut], &files, "v");
        }
        for _ in 0..200 {
            let mut m = model.clone();
            for _ in 0..8 {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                let i = (seed as usize) % m.len();
                m[i] = (seed >> 32) as u8;
            }
            let _ = parse(&m, &files, "v");
        }
        // Deep nesting is truncated rather than recursing without bound.
        let mut deep = rec(T_DEFINITION, &id(1));
        for _ in 0..200 {
            deep = rec(T_WRAPPER, &deep);
        }
        assert!(parse(&deep, &BTreeMap::new(), "v").is_ok());
    }
}
