// Legacy MFC archive layouts derived from OpenSKP (MIT).
// Copyright (c) 2024-2026 Ahsan Mehmood. See docs/LICENSES.md.
//! Bounded reader for classic (SketchUp 3–2020) MFC `CArchive` object streams.
//!
//! The record layouts have no length prefixes, so an unknown class or an
//! unreadable record cannot be skipped safely. The walk stops there and the
//! independent records completed so far are returned with a warning instead of
//! guessing a cursor realignment.
use crate::{Definition, Error, Instance, Result, Scene, MAX_BYTES, MAX_DEPTH, MAX_ITEMS};
use cindral_core::{BulkEdge, BulkFace, BulkGeometry, Material, MaterialId, Tag};
use glam::{DMat4, DVec3};
use std::collections::{BTreeMap, HashMap, HashSet};

const MARKER: &[u8] = b"\xff\xfe\xff";
const FIRST_V4: u32 = 4;
const FIRST_DIB_OBJECT: u32 = 6;
const FIRST_NAMED_INSTANCE: u32 = 5;
/// Store-map objects allowed per walk (geometry records plus bookkeeping objects).
const MAX_OBJECTS: usize = MAX_ITEMS * 4;
const MAX_BURNS: usize = 65_536;

#[derive(Debug)]
struct Fail {
    at: usize,
    msg: String,
}
type R<T> = std::result::Result<T, Fail>;
fn fail<T>(at: usize, msg: impl Into<String>) -> R<T> {
    Err(Fail {
        at,
        msg: msg.into(),
    })
}

fn u16_at(d: &[u8], at: usize) -> Option<u16> {
    let b = d.get(at..at.checked_add(2)?)?;
    Some(u16::from_le_bytes([b[0], b[1]]))
}
fn u32_at(d: &[u8], at: usize) -> Option<u32> {
    let b = d.get(at..at.checked_add(4)?)?;
    Some(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}
fn f64_at(d: &[u8], at: usize) -> Option<f64> {
    let b = d.get(at..at.checked_add(8)?)?;
    let mut a = [0u8; 8];
    a.copy_from_slice(b);
    Some(f64::from_le_bytes(a))
}
/// First match of `needle` lying entirely inside `d[start..end]`.
fn find(d: &[u8], needle: &[u8], start: usize, end: usize) -> Option<usize> {
    let end = end.min(d.len());
    if start >= end || end - start < needle.len() {
        return None;
    }
    d[start..end]
        .windows(needle.len())
        .position(|w| w == needle)
        .map(|i| i + start)
}

/// Bounded little-endian byte cursor.
struct Cur<'a> {
    d: &'a [u8],
    pos: usize,
}
impl<'a> Cur<'a> {
    fn take(&mut self, n: usize) -> R<&'a [u8]> {
        let Some(end) = self.pos.checked_add(n).filter(|&e| e <= self.d.len()) else {
            return fail(self.pos, format!("truncated read of {n} bytes"));
        };
        let s = &self.d[self.pos..end];
        self.pos = end;
        Ok(s)
    }
    fn u8(&mut self) -> R<u8> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> R<u16> {
        let b = self.take(2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }
    fn u32(&mut self) -> R<u32> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }
    fn f64(&mut self) -> R<f64> {
        let at = self.pos;
        self.take(8)?;
        f64_at(self.d, at).ok_or(Fail {
            at,
            msg: "truncated f64".into(),
        })
    }
    fn f64s<const N: usize>(&mut self) -> R<[f64; N]> {
        let mut o = [0.0; N];
        for v in &mut o {
            *v = self.f64()?;
        }
        Ok(o)
    }
    fn peek(&self, n: usize) -> Option<&'a [u8]> {
        self.d.get(self.pos..self.pos.checked_add(n)?)
    }
    fn peek_u16(&self) -> R<u16> {
        u16_at(self.d, self.pos).ok_or(Fail {
            at: self.pos,
            msg: "truncated tag".into(),
        })
    }
    fn seek(&mut self, pos: usize) -> R<()> {
        if pos > self.d.len() {
            return fail(self.pos, "seek beyond end of archive");
        }
        self.pos = pos;
        Ok(())
    }
    fn back(&mut self, n: usize) -> R<()> {
        self.pos = self.pos.checked_sub(n).ok_or(Fail {
            at: self.pos,
            msg: "seek before start".into(),
        })?;
        Ok(())
    }
    fn utf16(&mut self) -> R<String> {
        if self.peek(3) != Some(MARKER) {
            return fail(self.pos, "expected a string record");
        }
        self.pos += 3;
        let mut n = self.u8()? as usize;
        if n == 0xFF {
            n = self.u16()? as usize;
            if n == 0xFFFF {
                n = self.u32()? as usize;
            }
        }
        let bytes = n.checked_mul(2).ok_or(Fail {
            at: self.pos,
            msg: "string length overflow".into(),
        })?;
        let b = self.take(bytes)?;
        let units: Vec<u16> = b
            .as_chunks::<2>()
            .0
            .iter()
            .take(512)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        Ok(String::from_utf16_lossy(&units))
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum K {
    Vertex,
    Edge,
    Curve,
    ArcCurve,
    EdgeUse,
    Loop,
    Face,
    Layer,
    Material,
    Dib,
    AttrContainer,
    AttrNamed,
    Camera,
    Thumbnail,
    Relationship,
    Definition,
    Image,
    Instance,
    Ftc,
    CLine,
    CPoint,
    SectionPlane,
    Font,
    DimLinear,
    Text,
    Unknown,
}
fn kind(name: &str) -> K {
    match name {
        "CVertex" => K::Vertex,
        "CEdge" => K::Edge,
        "CCurve" => K::Curve,
        "CArcCurve" => K::ArcCurve,
        "CEdgeUse" => K::EdgeUse,
        "CLoop" => K::Loop,
        "CFace" => K::Face,
        "CLayer" => K::Layer,
        "CMaterial" => K::Material,
        "CDib" => K::Dib,
        "CAttributeContainer" => K::AttrContainer,
        "CAttributeNamed" => K::AttrNamed,
        "CCamera" => K::Camera,
        "CThumbnail" => K::Thumbnail,
        "CRelationship" => K::Relationship,
        "CComponentDefinition" => K::Definition,
        "CImage" => K::Image,
        "CComponentInstance" | "CGroup" => K::Instance,
        "CFaceTextureCoords" => K::Ftc,
        "CConstructionLine" => K::CLine,
        "CConstructionPoint" => K::CPoint,
        "CSectionPlane" => K::SectionPlane,
        "CSkFont" => K::Font,
        "CDimensionLinear" => K::DimLinear,
        "CText" => K::Text,
        _ => K::Unknown,
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct Db {
    mat: usize,
    hidden: u8,
    soft: u8,
    smooth: u8,
    layer: Option<usize>,
}

#[derive(Debug)]
struct Tex {
    dib: Option<usize>,
    inline: Option<(usize, usize)>,
    file: String,
}

#[derive(Debug)]
struct Mat {
    name: String,
    rgba: [u8; 4],
    opacity: f64,
    use_opacity: u8,
    tex: Option<Tex>,
}

#[derive(Debug)]
enum Val {
    Pending,
    None,
    Vertex(DVec3),
    Edge {
        v1: Option<usize>,
        v2: Option<usize>,
        db: Db,
    },
    EdgeUse {
        edge: Option<usize>,
        sense: u8,
    },
    Loop(Vec<(Option<usize>, u8)>),
    Face {
        db: Db,
        loops: Vec<Option<usize>>,
    },
    Material(Mat),
    Layer {
        name: String,
        hidden: bool,
    },
    Dib {
        start: usize,
        len: usize,
    },
    Definition {
        name: String,
        ents: Vec<Option<usize>>,
    },
    Instance {
        db: Db,
        def: Option<usize>,
        xf: [f64; 12],
        name: String,
    },
}

#[derive(Debug)]
enum Entry {
    Class(K, String),
    Obj(K, Val),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum RK {
    Null,
    Obj(K),
    Premodel,
    Reserved,
}
#[derive(Clone, Copy, Debug)]
struct Ref {
    slot: Option<usize>,
    kind: RK,
}
const NULL: Ref = Ref {
    slot: None,
    kind: RK::Null,
};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Owner {
    Root,
    Def,
}

/// Store-map bookkeeping and object-graph walk.
struct Ar<'a> {
    d: &'a [u8],
    ver: u32,
    has_pid: bool,
    slots: HashMap<usize, Entry>,
    class_slot: HashMap<K, usize>,
    cur_obj_slot: Option<usize>,
    next_slot: usize,
    walk_base: usize,
    current_loop: Option<usize>,
    burns: Vec<(usize, usize, usize)>,
    cum_delta: usize,
    annot_watermark: Option<usize>,
    burn_stack: Vec<usize>,
    cline_tail: Option<usize>,
    depth: usize,
    objects: usize,
    reads: usize,
    notes: Vec<String>,
}

impl<'a> Ar<'a> {
    fn new(d: &'a [u8], ver: u32, base: usize) -> Self {
        Self {
            d,
            ver,
            has_pid: ver >= 17,
            slots: HashMap::new(),
            class_slot: HashMap::new(),
            cur_obj_slot: None,
            next_slot: base,
            walk_base: base,
            current_loop: None,
            burns: Vec::new(),
            cum_delta: 0,
            annot_watermark: None,
            burn_stack: Vec::new(),
            cline_tail: None,
            depth: 0,
            objects: 0,
            reads: 0,
            notes: Vec::new(),
        }
    }

    fn alloc(&mut self, at: usize, e: Entry) -> R<usize> {
        self.objects += 1;
        if self.objects > MAX_OBJECTS {
            return fail(at, "archive object limit exceeded");
        }
        let s = self.next_slot;
        self.slots.insert(s, e);
        self.next_slot = s.checked_add(1).ok_or(Fail {
            at,
            msg: "slot overflow".into(),
        })?;
        Ok(s)
    }

    fn obj_kind(&self, slot: usize) -> Option<K> {
        match self.slots.get(&slot) {
            Some(Entry::Obj(k, _)) => Some(*k),
            _ => None,
        }
    }

    fn val(&self, slot: Option<usize>) -> Option<&Val> {
        match self.slots.get(&slot?) {
            Some(Entry::Obj(_, v)) => Some(v),
            _ => None,
        }
    }

    fn read_object(&mut self, r: &mut Cur, expect: Option<K>) -> R<Ref> {
        self.reads += 1;
        if self.reads > MAX_OBJECTS {
            return fail(r.pos, "archive read-work limit exceeded");
        }
        let at = r.pos;
        let tag = r.u16()?;
        if tag == 0 {
            return Ok(NULL);
        }
        if tag == 0x7FFF {
            let big = r.u32()?;
            if big & 0x8000_0000 != 0 {
                return self.new_of_class(r, (big & 0x7FFF_FFFF) as usize, expect);
            }
            return self.backref(big as usize, at);
        }
        if tag == 0xFFFF {
            let _schema = r.u16()?;
            let namelen = r.u16()? as usize;
            if namelen > 40 {
                return fail(r.pos, "implausible class name length");
            }
            let raw = r.take(namelen)?;
            if !raw.is_ascii() {
                return fail(r.pos, "non-ASCII class name");
            }
            let name = String::from_utf8_lossy(raw).into_owned();
            let k = kind(&name);
            let s = self.alloc(at, Entry::Class(k, name.clone()))?;
            if k != K::Unknown {
                self.class_slot.insert(k, s);
            }
            return self.new_obj(r, k, &name);
        }
        if tag & 0x8000 != 0 {
            return self.new_of_class(r, (tag & 0x7FFF) as usize, expect);
        }
        self.backref(tag as usize, at)
    }

    fn new_of_class(&mut self, r: &mut Cur, cslot: usize, expect: Option<K>) -> R<Ref> {
        let (k, name) = match self.slots.get(&cslot) {
            Some(Entry::Class(k, n)) => (*k, n.clone()),
            Some(Entry::Obj(..)) => {
                return fail(r.pos, format!("class-ref to non-class slot {cslot}"))
            }
            None => {
                let Some(e) = expect else {
                    return fail(r.pos, format!("class-ref to unknown slot {cslot}"));
                };
                // A class declared in the unwalked pre-model region: learn it from context.
                self.slots.insert(cslot, Entry::Class(e, format!("{e:?}")));
                self.class_slot.insert(e, cslot);
                (e, format!("{e:?}"))
            }
        };
        self.new_obj(r, k, &name)
    }

    fn new_obj(&mut self, r: &mut Cur, k: K, name: &str) -> R<Ref> {
        if k == K::Unknown {
            let n: String = name
                .chars()
                .filter(|c| c.is_ascii_graphic())
                .take(40)
                .collect();
            return fail(r.pos, format!("unsupported record class {n}"));
        }
        if self.depth >= MAX_DEPTH {
            return fail(r.pos, "object nesting depth limit exceeded");
        }
        let slot = self.alloc(r.pos, Entry::Obj(k, Val::Pending))?;
        let prev = self.cur_obj_slot.replace(slot);
        self.depth += 1;
        let res = self.dispatch(r, k);
        self.depth -= 1;
        self.cur_obj_slot = prev;
        let v = res?;
        self.slots.insert(slot, Entry::Obj(k, v));
        if matches!(k, K::DimLinear | K::Text) {
            self.annot_watermark = Some(self.next_slot);
        }
        Ok(Ref {
            slot: Some(slot),
            kind: RK::Obj(k),
        })
    }

    fn translate_ref(&self, slot: usize) -> Option<usize> {
        let index = self.burns.partition_point(|&(start, _, _)| start <= slot);
        if index == 0 {
            return Some(slot);
        }
        let (start, width, cumulative) = self.burns[index - 1];
        if slot < start.saturating_add(width) {
            None
        } else {
            slot.checked_sub(cumulative)
        }
    }

    fn backref(&self, mut slot: usize, at: usize) -> R<Ref> {
        if self.burns.first().is_some_and(|b| slot >= b.0) {
            match self.translate_ref(slot) {
                None => {
                    return Ok(Ref {
                        slot: Some(slot),
                        kind: RK::Reserved,
                    })
                }
                Some(w) => slot = w,
            }
        }
        match self.slots.get(&slot) {
            None if slot < self.walk_base => Ok(Ref {
                slot: Some(slot),
                kind: RK::Premodel,
            }),
            None => fail(at, format!("back-ref to unwalked slot {slot}")),
            Some(Entry::Class(..)) => fail(at, format!("back-ref to class slot {slot}")),
            Some(Entry::Obj(k, _)) => Ok(Ref {
                slot: Some(slot),
                kind: RK::Obj(*k),
            }),
        }
    }

    fn is_class_ref(&self, p: usize, slot: usize) -> bool {
        if slot < 0x7FFF {
            u16_at(self.d, p) == Some(0x8000 | slot as u16)
        } else {
            u16_at(self.d, p) == Some(0x7FFF)
                && u32::try_from(slot).is_ok_and(|s| u32_at(self.d, p + 2) == Some(0x8000_0000 | s))
        }
    }

    fn clayer_record_at(&self, at: usize, unmatched: bool) -> bool {
        let Some(tag) = u16_at(self.d, at) else {
            return false;
        };
        if tag == 0xFFFF
            && u16_at(self.d, at + 4) == Some(6)
            && self.d.get(at + 6..at + 12) == Some(b"CLayer")
        {
            return true;
        }
        if let Some(&c) = self.class_slot.get(&K::Layer) {
            if self.is_class_ref(at, c) {
                return true;
            }
        }
        unmatched && tag & 0x8000 != 0 && tag != 0xFFFF
    }

    fn skip_clayer_colour_ext(&self, r: &mut Cur) {
        if r.pos + 18 > self.d.len() || self.d[r.pos..r.pos + 12].iter().any(|&b| b != 0) {
            return;
        }
        if self.clayer_record_at(r.pos + 16, true) {
            r.pos += 16;
        }
    }

    fn skip_clayer_parent_ref(&self, r: &mut Cur, self_slot: Option<usize>) {
        let (Some(a), Some(b), Some(me)) =
            (u16_at(self.d, r.pos), u16_at(self.d, r.pos + 2), self_slot)
        else {
            return;
        };
        let bad = |t: u16| t == 0 || t & 0x8000 != 0 || t == 0x7FFF;
        if bad(a) || a as usize == me || bad(b) {
            return;
        }
        if self.obj_kind(a as usize) == Some(K::Layer)
            && self.obj_kind(b as usize) == Some(K::Layer)
        {
            r.pos += 2;
        }
    }

    fn collect_trailing_layers(
        &mut self,
        r: &mut Cur,
        out: &mut Vec<usize>,
        unmatched: bool,
    ) -> R<()> {
        while r.pos + 2 <= self.d.len() {
            if r.peek_u16()? == 0 {
                r.pos += 2;
                continue;
            }
            if !self.clayer_record_at(r.pos, unmatched) {
                break;
            }
            let x = self.read_object(r, Some(K::Layer))?;
            if let (RK::Obj(_), Some(s)) = (x.kind, x.slot) {
                out.push(s);
            }
        }
        Ok(())
    }

    fn plausible_list_tag(&self, at: usize) -> bool {
        let Some(t) = u16_at(self.d, at) else {
            return false;
        };
        if matches!(t, 0 | 0x7FFF | 0xFFFF) {
            return true;
        }
        if t & 0x8000 != 0 {
            return matches!(
                self.slots.get(&((t & 0x7FFF) as usize)),
                Some(Entry::Class(..))
            );
        }
        (t as usize) < self.next_slot
    }

    fn strict_next_tag(&self, at: usize, allow_null: bool) -> bool {
        let Some(t) = u16_at(self.d, at) else {
            return false;
        };
        match t {
            0 => allow_null,
            0x7FFF | 0xFFFF => true,
            t if t & 0x8000 != 0 => matches!(
                self.slots.get(&((t & 0x7FFF) as usize)),
                Some(Entry::Class(..))
            ),
            _ => false,
        }
    }

    /// SketchUp 2020 can place an empty string plus zero padding ahead of a count.
    fn retry_v20(&self, r: &mut Cur, count_pos: usize, limit: u32) -> Option<u32> {
        let d = self.d;
        let marker_at = find(
            d,
            MARKER,
            count_pos,
            (count_pos + 12).min(d.len().saturating_sub(3)) + 2,
        )?;
        if marker_at >= count_pos + 12 || *d.get(marker_at + 3)? != 0 {
            return None;
        }
        let mut at = marker_at + 4;
        while at < d.len() && d[at] == 0 {
            at += 1;
        }
        for back in 0..4 {
            let Some(at2) = at.checked_sub(back) else {
                continue;
            };
            if at2 < marker_at + 4 {
                continue;
            }
            let Some(count) = u32_at(d, at2) else {
                continue;
            };
            if count == 0 || count > limit || !self.plausible_list_tag(at2 + 4) {
                continue;
            }
            r.pos = at2 + 4;
            return Some(count);
        }
        None
    }

    fn preamble(&mut self, r: &mut Cur) -> R<Ref> {
        let attrs = self.read_object(r, Some(K::AttrContainer))?;
        if self.has_pid {
            let mask = r.u8()?;
            r.take(mask.count_ones() as usize)?;
        }
        Ok(attrs)
    }

    fn early_preamble(&mut self, r: &mut Cur) -> R<()> {
        if self.ver >= FIRST_V4 {
            self.preamble(r)?;
        }
        Ok(())
    }

    fn drawbase(&mut self, r: &mut Cur) -> R<Db> {
        let n = if self.ver < FIRST_V4 {
            5
        } else if self.ver < 5 {
            7
        } else {
            8
        };
        let b = r.take(n)?;
        let tag = r.peek_u16()?;
        let lay = self
            .class_slot
            .get(&K::Layer)
            .copied()
            .filter(|&c| c < 0x7FFF);
        let layer = if lay.is_some_and(|c| tag == 0x8000 | c as u16) {
            // By-object layer: an inline layer record on first use.
            self.read_object(r, Some(K::Layer))?.slot
        } else if tag == 0x7FFF {
            r.u16()?;
            let big = r.u32()?;
            if big & 0x8000_0000 != 0 {
                return fail(r.pos, "drawbase layer: unexpected class");
            }
            self.translate_ref(big as usize)
        } else {
            Some(r.u16()? as usize).filter(|&l| l != 0)
        };
        if self.ver < FIRST_V4 {
            return Ok(Db {
                layer,
                ..Db::default()
            });
        }
        Ok(Db {
            mat: u16::from_le_bytes([b[0], b[1]]) as usize,
            hidden: b[2],
            soft: b[5],
            smooth: b[6],
            layer,
        })
    }

    fn entity_ref(&self, r: &mut Cur) -> R<Option<usize>> {
        let tag = r.u16()?;
        if tag == 0 {
            return Ok(None);
        }
        if tag == 0x7FFF {
            let big = r.u32()?;
            if big & 0x8000_0000 != 0 {
                return fail(r.pos, "entity ref is a new object");
            }
            return Ok(Some(big as usize));
        }
        if tag & 0x8000 != 0 {
            return fail(r.pos, "entity ref is a new object");
        }
        Ok(Some(tag as usize))
    }

    fn dispatch(&mut self, r: &mut Cur, k: K) -> R<Val> {
        match k {
            K::Vertex => {
                self.early_preamble(r)?;
                let p = r.f64s::<3>()?;
                Ok(Val::Vertex(DVec3::from_array(p)))
            }
            K::Edge => {
                self.preamble(r)?;
                let db = self.drawbase(r)?;
                let v1 = self.read_object(r, Some(K::Vertex))?.slot;
                let v2 = self.read_object(r, Some(K::Vertex))?.slot;
                let c = self.read_object(r, None)?;
                if !matches!(c.kind, RK::Null | RK::Obj(K::Curve | K::ArcCurve)) {
                    return fail(r.pos, "edge curve pointer is not a curve");
                }
                Ok(Val::Edge { v1, v2, db })
            }
            K::Curve => {
                self.preamble(r)?;
                r.u8()?;
                r.u32()?;
                Ok(Val::None)
            }
            K::ArcCurve => {
                self.preamble(r)?;
                r.take(5 + 14 * 8)?;
                Ok(Val::None)
            }
            K::EdgeUse => self.read_edgeuse(r),
            K::Loop => {
                let my_slot = self.next_slot - 1;
                let prev = self.current_loop.replace(my_slot);
                let res = self.read_loop(r);
                self.current_loop = prev;
                res
            }
            K::Face => self.read_face(r),
            K::AttrContainer => {
                self.preamble(r)?;
                while r.peek_u16()? != 0 {
                    self.read_object(r, Some(K::AttrNamed))?;
                }
                r.pos += 2;
                Ok(Val::None)
            }
            K::AttrNamed => {
                self.preamble(r)?;
                r.take(4)?;
                r.utf16()?;
                loop {
                    if r.utf16()?.is_empty() {
                        break;
                    }
                    let t = r.u8()?;
                    read_typed(r, t, 0)?;
                }
                if self.ver >= 7 {
                    r.u32()?;
                }
                Ok(Val::None)
            }
            K::Layer => self.read_layer(r),
            K::Material => self.read_material(r),
            K::Dib => read_dib(r),
            K::Ftc => {
                self.preamble(r)?;
                r.u32()?;
                r.take(24 * 8)?;
                for _ in 0..2 {
                    let n = r.u32()? as usize;
                    r.take(n.saturating_mul(32))?;
                }
                r.take(8)?;
                Ok(Val::None)
            }
            K::Camera => {
                r.take(137)?;
                r.u16()?;
                r.utf16()?;
                r.take(if self.ver >= 6 { 33 } else { 10 })?;
                Ok(Val::None)
            }
            K::Thumbnail => {
                self.preamble(r)?;
                if self.ver < FIRST_V4 {
                    r.take(129)?;
                    return Ok(Val::None);
                }
                self.read_object(r, Some(K::Camera))?;
                if self.ver >= 6 {
                    self.read_object(r, Some(K::Dib))?;
                }
                Ok(Val::None)
            }
            K::Image => {
                self.preamble(r)?;
                let db = self.drawbase(r)?;
                let def = self.read_object(r, None)?.slot;
                let xf = r.f64s::<12>()?;
                r.f64()?;
                r.utf16()?;
                r.take(16)?;
                Ok(Val::Instance {
                    db,
                    def,
                    xf,
                    name: String::new(),
                })
            }
            K::Relationship => {
                self.preamble(r)?;
                self.entity_ref(r)?;
                self.entity_ref(r)?;
                Ok(Val::None)
            }
            K::CLine => {
                self.preamble(r)?;
                self.drawbase(r)?;
                r.take(8 * 8)?;
                let k = match self.cline_tail {
                    Some(k) => k,
                    None => {
                        let default = if self.ver == 17 { 7 } else { 4 };
                        let mut order = vec![default];
                        order.extend([0, 4, 7].into_iter().filter(|&c| c != default));
                        let k = order
                            .iter()
                            .copied()
                            .find(|&c| self.strict_next_tag(r.pos + c, false))
                            .or_else(|| {
                                [4, 0, 7]
                                    .into_iter()
                                    .find(|&c| self.strict_next_tag(r.pos + c, true))
                            })
                            .unwrap_or(4);
                        self.cline_tail = Some(k);
                        k
                    }
                };
                r.take(k)?;
                Ok(Val::None)
            }
            K::CPoint => {
                self.preamble(r)?;
                self.drawbase(r)?;
                r.take(48 + 1)?;
                Ok(Val::None)
            }
            K::SectionPlane => {
                self.preamble(r)?;
                self.drawbase(r)?;
                let first = f64_at(self.d, r.pos).ok_or(Fail {
                    at: r.pos,
                    msg: "truncated section plane".into(),
                })?;
                if !first.is_finite() || first.abs() > 1.0001 {
                    self.read_object(r, None)?;
                }
                r.take(32)?;
                if r.peek(3) == Some(MARKER) {
                    r.utf16()?;
                    r.utf16()?;
                }
                Ok(Val::None)
            }
            K::Font => {
                self.read_object(r, Some(K::AttrContainer))?;
                if self.has_pid {
                    let mask = r.u8()?;
                    r.take(mask.count_ones() as usize)?;
                }
                r.utf16()?;
                r.take(15)?;
                Ok(Val::None)
            }
            K::DimLinear => self.read_dimlinear(r),
            K::Text => self.read_text(r),
            K::Definition => self.read_definition(r),
            K::Instance => self.read_instance(r),
            K::Unknown => fail(r.pos, "unsupported record class"),
        }
    }

    fn register_burn(&mut self, delta: usize, at: usize) -> R<()> {
        let Some(w) = self.annot_watermark.take() else {
            return fail(at, "burn without annotation");
        };
        if self.burns.len() >= MAX_BURNS {
            return fail(at, "store-map burn limit exceeded");
        }
        let start = w + self.cum_delta;
        self.cum_delta += delta;
        self.burns.push((start, delta, self.cum_delta));
        if let Some(last) = self.burn_stack.last_mut() {
            *last += 1;
        }
        Ok(())
    }

    fn read_edgeuse(&mut self, r: &mut Cur) -> R<Val> {
        self.early_preamble(r)?;
        let edge = self.read_object(r, Some(K::Edge))?.slot;
        let sense = r.u8()?;
        let p0 = r.pos;
        let tag = r.u16()?;
        let ps = if tag == 0x7FFF {
            let ps = r.u32()?;
            if ps & 0x8000_0000 != 0 {
                return fail(r.pos, "edge-use parent is a new object");
            }
            Some(ps as usize)
        } else if tag & 0x8000 != 0 {
            return fail(r.pos, "edge-use parent is a new object");
        } else {
            Some(tag as usize).filter(|&t| t != 0)
        };
        let expected = self.current_loop.map(|c| c + self.cum_delta);
        if ps != expected {
            let delta = match (ps, expected) {
                (Some(p), Some(e)) if p > e => p - e,
                _ => 0,
            };
            if (1..=4096).contains(&delta) && self.annot_watermark.is_some() {
                self.register_burn(delta, p0)?;
            } else {
                r.pos = p0;
                return fail(
                    p0,
                    format!("edge-use parent slot {ps:?} != current loop {expected:?}"),
                );
            }
        }
        Ok(Val::EdgeUse { edge, sense })
    }

    fn read_loop(&mut self, r: &mut Cur) -> R<Val> {
        self.early_preamble(r)?;
        r.take(2)?;
        let mut uses = Vec::new();
        while r.peek_u16()? != 0 {
            if uses.len() >= 4096 {
                return fail(r.pos, "loop exceeds 4096 coedges");
            }
            let u = self.read_object(r, Some(K::EdgeUse))?;
            if let Some(Val::EdgeUse { edge, sense }) = self.val(u.slot) {
                uses.push((*edge, *sense));
            }
        }
        r.pos += 2;
        Ok(Val::Loop(uses))
    }

    fn read_face(&mut self, r: &mut Cur) -> R<Val> {
        let pre = self.preamble(r)?;
        let mut db = self.drawbase(r)?;
        if self.ver < FIRST_V4 && pre.kind == RK::Obj(K::Material) {
            // SketchUp 3: the opening pointer is the face's front material.
            db.mat = pre.slot.unwrap_or(0);
        }
        r.take(32)?;
        let nloops = r.u32()?;
        if nloops > 10_000 {
            return fail(r.pos, format!("implausible loop count {nloops}"));
        }
        let mut loops = Vec::with_capacity(nloops as usize);
        for _ in 0..nloops {
            loops.push(self.read_object(r, Some(K::Loop))?.slot);
        }
        let back_mat = r.u16()?;
        if db.mat == 0 {
            db.mat = back_mat as usize;
        }
        Ok(Val::Face { db, loops })
    }

    fn texture_block(&mut self, r: &mut Cur) -> R<([u8; 4], f64, u8, Tex)> {
        let (dib, inline) = if self.ver < FIRST_DIB_OBJECT {
            let Val::Dib { start, len } = read_dib(r)? else {
                return fail(r.pos, "texture dib");
            };
            (None, Some((start, len)))
        } else {
            r.back(1)?;
            self.preamble(r)?;
            let x = self.read_object(r, Some(K::Dib))?;
            if x.kind != RK::Obj(K::Dib) {
                return fail(r.pos, "texture object is not a dib");
            }
            (x.slot, None)
        };
        match find(self.d, MARKER, r.pos, r.pos + 28).map(|m| m - r.pos) {
            Some(20) => {
                r.u32()?;
            }
            Some(16) => {}
            _ => return fail(r.pos, "texture size block misaligned"),
        }
        r.take(16)?;
        let file = r.utf16()?;
        let avg = r.take(9)?;
        let rgba = [avg[0], avg[1], avg[2], avg[3]];
        r.utf16()?;
        r.take(8)?;
        let opacity = r.f64()?;
        let use_op = r.u8()?;
        Ok((rgba, opacity, use_op, Tex { dib, inline, file }))
    }

    fn read_material(&mut self, r: &mut Cur) -> R<Val> {
        self.early_preamble(r)?;
        let name = r.utf16()?;
        let texflag = r.u16()?;
        if texflag == 0 {
            let b = r.take(4)?;
            let rgba = [b[0], b[1], b[2], b[3]];
            r.utf16()?;
            r.take(8)?;
            let opacity = r.f64()?;
            let use_opacity = r.u8()?;
            return Ok(Val::Material(Mat {
                name,
                rgba,
                opacity,
                use_opacity,
                tex: None,
            }));
        }
        let (rgba, opacity, use_opacity, tex) = self.texture_block(r)?;
        Ok(Val::Material(Mat {
            name,
            rgba,
            opacity,
            use_opacity,
            tex: Some(tex),
        }))
    }

    fn read_layer(&mut self, r: &mut Cur) -> R<Val> {
        self.early_preamble(r)?;
        let name = r.utf16()?;
        let mut mid = Vec::new();
        while mid.len() < 8 && r.peek(3) != Some(MARKER) {
            mid.push(r.u8()?);
        }
        r.utf16()?;
        let hidden = mid.first().is_some_and(|&b| b != 0);
        let flags = r.u16()?;
        if flags & 0x00FF != 0 {
            self.texture_block(r)?;
            r.take(4)?;
            return Ok(Val::Layer { name, hidden });
        }
        r.take(4)?;
        r.utf16()?;
        r.take(if self.ver < FIRST_V4 { 17 } else { 21 })?;
        self.skip_clayer_colour_ext(r);
        let me = self.cur_obj_slot;
        self.skip_clayer_parent_ref(r, me);
        Ok(Val::Layer { name, hidden })
    }

    fn connection_paths(&mut self, r: &mut Cur) -> R<()> {
        self.entity_ref(r)?;
        for _ in 0..2 {
            let count = r.u32()?;
            if count > 1000 {
                return fail(r.pos, "implausible dimension path length");
            }
            for _ in 0..count {
                let tag = r.peek_u16()?;
                let new = if tag == 0x7FFF {
                    u32_at(self.d, r.pos + 2).ok_or(Fail {
                        at: r.pos,
                        msg: "truncated path ref".into(),
                    })? & 0x8000_0000
                        != 0
                } else {
                    tag == 0xFFFF || tag & 0x8000 != 0
                };
                if new {
                    self.read_object(r, None)?;
                } else {
                    self.entity_ref(r)?;
                }
            }
        }
        Ok(())
    }

    fn read_dimlinear(&mut self, r: &mut Cur) -> R<Val> {
        self.preamble(r)?;
        self.drawbase(r)?;
        r.utf16()?;
        if self.ver >= 6 {
            self.read_object(r, Some(K::Font))?;
        }
        r.take(if self.ver < 6 { 32 } else { 37 })?;
        self.entity_ref(r)?;
        self.connection_paths(r)?;
        r.take(32)?;
        self.entity_ref(r)?;
        self.connection_paths(r)?;
        r.take(if self.ver >= 6 { 72 } else { 68 })?;
        Ok(Val::None)
    }

    fn read_text(&mut self, r: &mut Cur) -> R<Val> {
        self.preamble(r)?;
        self.drawbase(r)?;
        self.read_object(r, Some(K::Font))?;
        let limit = r.pos + 512;
        let mut p = r.pos;
        let idx = loop {
            let Some(idx) = find(self.d, MARKER, p, limit) else {
                return fail(r.pos, "text delimiter not found");
            };
            if idx >= 11 {
                let blk = &self.d[idx - 11..idx];
                let arrow = u32::from_le_bytes([blk[6], blk[7], blk[8], blk[9]]);
                if blk[..4] == [1, 0, 0, 0] && arrow <= 8 && blk[10] == 1 {
                    break idx;
                }
            }
            p = idx + 3;
        };
        if idx < r.pos {
            return fail(r.pos, "text delimiter precedes cursor");
        }
        r.seek(idx)?;
        r.utf16()?;
        r.take(5)?;
        while r.peek(2) == Some(&[0xff, 0x7f][..]) {
            let v = u32_at(self.d, r.pos + 2).ok_or(Fail {
                at: r.pos,
                msg: "truncated text attachment".into(),
            })?;
            if v & 0x8000_0000 != 0 {
                break;
            }
            r.take(6)?;
        }
        Ok(Val::None)
    }

    fn entity_list(
        &mut self,
        r: &mut Cur,
        count: u32,
        owner: Owner,
        out: &mut Vec<Option<usize>>,
    ) -> R<()> {
        self.burn_stack.push(0);
        let res = self.entity_list_inner(r, count as usize, owner, out);
        self.burn_stack.pop();
        res
    }

    fn entity_list_inner(
        &mut self,
        r: &mut Cur,
        count: usize,
        owner: Owner,
        out: &mut Vec<Option<usize>>,
    ) -> R<()> {
        while out.len() < count {
            let p = r.pos;
            let burned = self.burn_stack.last().is_some_and(|&b| b > 0);
            if owner == Owner::Def
                && burned
                && u32_at(self.d, p) == Some(0)
                && self.d.get(p + 22..p + 25) == Some(MARKER)
            {
                break;
            }
            match self.read_object(r, None) {
                Ok(x) => out.push(x.slot),
                Err(e) if owner == Owner::Root => {
                    self.notes.truncate(255);
                    self.notes.push(format!(
                        "root entity list stopped after {} of {count} items at offset {:#x}: {}",
                        out.len(),
                        e.at,
                        e.msg
                    ));
                    r.pos = p;
                    break;
                }
                Err(_) if owner == Owner::Def && burned => {
                    r.pos = p;
                    break;
                }
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }

    fn read_definition(&mut self, r: &mut Cur) -> R<Val> {
        self.preamble(r)?;
        let v = self.ver;
        r.take(if v >= 17 {
            22
        } else if v >= 5 {
            20
        } else if v >= FIRST_V4 {
            19
        } else {
            13
        })?;
        let nlayers = r.u32()?;
        if nlayers > 10_000 {
            return fail(r.pos, "implausible definition layer count");
        }
        let mut got = 0;
        while got < nlayers {
            if r.peek_u16()? == 0 {
                r.pos += 2;
                continue;
            }
            self.read_object(r, Some(K::Layer))?;
            got += 1;
        }
        if v == 19 && nlayers > 0 && r.peek_u16()? == 0 {
            r.pos += 2;
        }
        if r.u16()? == 0x7FFF {
            r.u32()?;
        }
        let mut count = if v >= 20 {
            self.retry_v20(r, r.pos, 5_000_000)
        } else {
            None
        };
        let mut c = match count.take() {
            Some(c) => c,
            None => {
                r.u32()?;
                r.u32()?
            }
        };
        if c > 5_000_000 || c == 0 {
            if let Some(retry) = self.retry_v20(r, r.pos - 4, 5_000_000) {
                c = retry;
            }
        }
        if c > 5_000_000 {
            return fail(r.pos, "implausible definition entity count");
        }
        let mut ents = Vec::new();
        self.entity_list(r, c, Owner::Def, &mut ents)?;
        let mut nrel = r.u32()?;
        if nrel > 100_000 {
            if let Some(retry) = self.retry_v20(r, r.pos - 4, 100_000) {
                nrel = retry;
            }
        }
        if nrel > 100_000 {
            return fail(r.pos, "definition list misaligned");
        }
        for _ in 0..nrel {
            self.read_object(r, Some(K::Relationship))?;
        }
        r.u16()?;
        let anchored = reanchor(self.d, r.pos);
        r.seek(anchored)?;
        r.take(16)?;
        let name = r.utf16()?;
        r.utf16()?;
        r.utf16()?;
        r.u32()?;
        let thumb = self.class_slot.get(&K::Thumbnail).copied();
        let tpos = (0..96).map(|off| r.pos + off).find(|&p| {
            (self.d.get(p..p + 2) == Some(&[0xff, 0xff][..])
                && self.d.get(p + 4..p + 6) == Some(&[0x0a, 0x00][..])
                && self.d.get(p + 6..p + 16) == Some(b"CThumbnail"))
                || thumb.is_some_and(|t| self.is_class_ref(p, t))
        });
        let Some(tpos) = tpos else {
            return fail(r.pos, "definition tail: thumbnail not found");
        };
        r.seek(tpos)?;
        self.read_object(r, Some(K::Thumbnail))?;
        Ok(Val::Definition { name, ents })
    }

    fn read_instance(&mut self, r: &mut Cur) -> R<Val> {
        self.preamble(r)?;
        let db = self.drawbase(r)?;
        let d = self.read_object(r, Some(K::Definition))?;
        if d.kind != RK::Obj(K::Definition) {
            return fail(r.pos, "instance definition ref is not a definition");
        }
        let all = r.f64s::<13>()?;
        let mut xf = [0.0; 12];
        xf.copy_from_slice(&all[..12]);
        let name = if self.ver >= FIRST_NAMED_INSTANCE || r.peek(3) == Some(MARKER) {
            r.utf16()?
        } else {
            String::new()
        };
        if self.ver >= 14 {
            r.take(16)?;
        }
        Ok(Val::Instance {
            db,
            def: d.slot,
            xf,
            name,
        })
    }

    fn definition_starts_at(&self, at: usize) -> bool {
        if self.d.get(at..at + 2) == Some(&[0xff, 0xff][..])
            && self.d.get(at + 6..at + 26) == Some(b"CComponentDefinition")
        {
            return true;
        }
        self.class_slot
            .get(&K::Definition)
            .is_some_and(|&c| self.is_class_ref(at, c))
    }
}

fn read_typed(r: &mut Cur, t: u8, depth: usize) -> R<()> {
    if depth > MAX_DEPTH {
        return fail(r.pos, "attribute array nesting limit exceeded");
    }
    match t {
        0x00 => {}
        0x04 | 0x09 => {
            r.take(4)?;
        }
        0x06 | 0x0C => {
            r.take(8)?;
        }
        0x07 => {
            r.take(1)?;
        }
        0x0A => {
            r.utf16()?;
        }
        0x0B => {
            let n = r.u32()?;
            if n > 100_000 {
                return fail(r.pos, "implausible attribute array count");
            }
            for _ in 0..n {
                let t = r.u8()?;
                read_typed(r, t, depth + 1)?;
            }
        }
        0x11 | 0x12 => {
            r.take(24)?;
        }
        _ => return fail(r.pos, format!("unknown attribute value type {t:#x}")),
    }
    Ok(())
}

fn read_dib(r: &mut Cur) -> R<Val> {
    r.u32()?;
    let len = r.u32()? as usize;
    if len > r.d.len() {
        return fail(r.pos, format!("implausible dib length {len}"));
    }
    let start = r.pos;
    r.take(len)?;
    Ok(Val::Dib { start, len })
}

fn reanchor(d: &[u8], pos: usize) -> usize {
    let hit = |at: usize| d.get(at + 16..at + 19) == Some(MARKER);
    if hit(pos) {
        return pos;
    }
    for skip in [-1isize, 1, -2, 2, -3, 3, -4, 4] {
        if let Some(at) = pos.checked_add_signed(skip) {
            if hit(at) {
                return at;
            }
        }
    }
    pos
}

/// Locate `ff ff ?? ?? <len u16> <name>` class declarations.
fn find_class_decl(d: &[u8], name: &[u8]) -> Option<usize> {
    let mut needle = (name.len() as u16).to_le_bytes().to_vec();
    needle.extend_from_slice(name);
    let mut from = 4;
    while let Some(q) = find(d, &needle, from, d.len()) {
        if d[q - 4..q - 2] == [0xff, 0xff] {
            return Some(q - 4);
        }
        from = q + 1;
    }
    None
}

/// Records recovered by one walk attempt.
#[derive(Default)]
struct Progress {
    materials: Vec<usize>,
    layers: Vec<usize>,
    root: Vec<Option<usize>>,
    root_read: bool,
}

fn walk_model(ar: &mut Ar, st: &mut Progress, start: usize, mat_count: u32) -> R<()> {
    let d = ar.d;
    let ver = ar.ver;
    let mut r = Cur { d, pos: start };
    for _ in 0..mat_count {
        let x = ar.read_object(&mut r, Some(K::Material))?;
        if let (RK::Obj(_), Some(s)) = (x.kind, x.slot) {
            st.materials.push(s);
        }
    }
    r.u32()?;
    if ver >= 17 {
        r.u8()?;
    }
    let layer_count = if ver < FIRST_V4 { u32::MAX } else { r.u32()? };
    if layer_count > 100_000 && ver >= FIRST_V4 {
        return fail(r.pos, "implausible layer count");
    }
    while (st.layers.len() as u64) < u64::from(layer_count) {
        let tag = r.peek_u16()?;
        if tag == 0 {
            r.pos += 2;
            continue;
        }
        if tag != 0xFFFF && tag & 0x8000 == 0 {
            break;
        }
        let x = ar.read_object(&mut r, Some(K::Layer))?;
        if let (RK::Obj(_), Some(s)) = (x.kind, x.slot) {
            st.layers.push(s);
        }
    }
    let mut extra = Vec::new();
    ar.collect_trailing_layers(&mut r, &mut extra, false)?;
    st.layers.extend(extra);
    let anchor = ar.read_object(&mut r, None)?;
    if anchor.kind != RK::Obj(K::Layer) {
        return fail(r.pos, "definition-list anchor is not a layer");
    }
    let mut def_count = r.u32()?;
    let count_at = r.pos - 4;
    if def_count > 1_000_000 {
        if let Some(retry) = ar.retry_v20(&mut r, count_at, 1_000_000) {
            def_count = retry;
            if !ar.definition_starts_at(r.pos) {
                def_count = 0;
                r.back(4)?;
            }
        }
    }
    if def_count > 1_000_000 {
        return fail(r.pos, "implausible definition count");
    }
    for _ in 0..def_count {
        ar.read_object(&mut r, Some(K::Definition))?;
    }
    while ar.definition_starts_at(r.pos) {
        ar.read_object(&mut r, None)?;
    }
    let mut root_count = r.u32()?;
    let count_at = r.pos - 4;
    if root_count > 5_000_000 {
        if let Some(retry) = ar.retry_v20(&mut r, count_at, 5_000_000) {
            root_count = retry;
        }
    }
    if root_count > 5_000_000 {
        return fail(r.pos, "implausible root entity count");
    }
    let mut root = Vec::new();
    let res = ar.entity_list(&mut r, root_count, Owner::Root, &mut root);
    st.root = root;
    st.root_read = res.is_ok();
    res
}

fn bootstrap_two_materials(d: &[u8], ver: u32, start: usize) -> R<usize> {
    let mut boot = Ar::new(d, ver, 1 << 20);
    let mut r = Cur { d, pos: start };
    boot.read_object(&mut r, Some(K::Material))?;
    let tag = r.peek_u16()?;
    if tag == 0xFFFF || tag & 0x8000 == 0 {
        return fail(r.pos, "cannot bootstrap the slot base");
    }
    Ok((tag & 0x7FFF) as usize)
}

fn probe_layer_anchor_bases(d: &[u8], ver: u32, start: usize, mat_count: u32) -> R<Vec<usize>> {
    let b0 = 1usize << 20;
    let mut boot = Ar::new(d, ver, b0);
    let mut r = Cur { d, pos: start };
    for _ in 0..mat_count {
        boot.read_object(&mut r, Some(K::Material))?;
    }
    r.u32()?;
    if ver >= 17 {
        r.u8()?;
    }
    let layer_count = if ver < FIRST_V4 { 0 } else { r.u32()? };
    if ver >= FIRST_V4 && !(1..=100_000).contains(&layer_count) {
        return fail(r.pos, "implausible layer count in base probe");
    }
    let mut slots = Vec::new();
    for _ in 0..layer_count {
        if let Some(s) = boot.read_object(&mut r, Some(K::Layer))?.slot {
            slots.push(s);
        }
    }
    boot.collect_trailing_layers(&mut r, &mut slots, true)?;
    let anchor = boot.read_object(&mut r, None)?;
    if anchor.kind != RK::Premodel {
        return fail(r.pos, "base probe: anchor did not resolve before the model");
    }
    let s = anchor.slot.unwrap_or(0) as i64;
    Ok(slots
        .iter()
        .map(|&rel| s - (rel as i64 - b0 as i64))
        .filter(|&b| 0 < b && b < b0 as i64)
        .map(|b| b as usize)
        .collect())
}

/// Parse a classic MFC archive body. `major` is the header's major version.
pub fn parse(data: &[u8], major: u32, version: &str) -> Result<Scene> {
    let err = |m: String| Error(format!("SketchUp {version}: {m}"));
    if !(3..=20).contains(&major) {
        return Err(err(format!(
            "unsupported legacy archive version {major}; supported legacy versions are 3–20"
        )));
    }
    if data.len() > MAX_BYTES {
        return Err(err("legacy archive exceeds 64 MiB limit".into()));
    }
    let to_err = |f: Fail| {
        err(format!(
            "legacy archive error at offset {:#x}: {}",
            f.at, f.msg
        ))
    };
    let (start, mat_count) = if let Some(start) = find_class_decl(data, b"CMaterial") {
        let mc = start
            .checked_sub(4)
            .and_then(|p| u32_at(data, p))
            .ok_or_else(|| err("truncated material count".into()))?;
        if mc > 100_000 {
            return Err(err("implausible material count".into()));
        }
        (start, mc)
    } else {
        let lm = find_class_decl(data, b"CLayer")
            .ok_or_else(|| err("no CMaterial or CLayer class record found".into()))?;
        let start = lm
            .checked_sub(if major >= 17 { 9 } else { 8 })
            .ok_or_else(|| err("truncated layer list".into()))?;
        (start, 0)
    };
    let bases = if mat_count >= 2 {
        vec![bootstrap_two_materials(data, major, start).map_err(to_err)?]
    } else {
        probe_layer_anchor_bases(data, major, start, mat_count).map_err(to_err)?
    };
    let mut best: Option<(Ar, Progress, Fail)> = None;
    for base in bases.into_iter().take(32) {
        let mut ar = Ar::new(data, major, base);
        let mut st = Progress::default();
        match walk_model(&mut ar, &mut st, start, mat_count) {
            Ok(()) => return Ok(build_scene(&ar, &st, version, None)),
            Err(f) => {
                if best.as_ref().is_none_or(|b| f.at > b.2.at) {
                    best = Some((ar, st, f));
                }
            }
        }
    }
    let Some((ar, st, f)) = best else {
        return Err(err("no viable slot base candidate".into()));
    };
    let scene = build_scene(&ar, &st, version, Some(&f));
    if scene.definitions.is_empty() && scene.roots.is_empty() {
        return Err(to_err(f));
    }
    Ok(scene)
}

fn srgb_to_linear(c: u8) -> f32 {
    let c = f32::from(c) / 255.0;
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// Instance matrix: rows are `xf[0..3]`, `xf[3..6]`, `xf[6..9]`; translation `xf[9..12]` inches.
fn transform(xf: &[f64; 12]) -> Option<DMat4> {
    if xf.iter().any(|v| !v.is_finite()) {
        return None;
    }
    Some(DMat4::from_cols_array(&[
        xf[0], xf[3], xf[6], 0.0, xf[1], xf[4], xf[7], 0.0, xf[2], xf[5], xf[8], 0.0, xf[9],
        xf[10], xf[11], 1.0,
    ]))
}

struct Ctx<'s> {
    ar: &'s Ar<'s>,
    mats: HashMap<usize, usize>,
    colors: Vec<[u8; 4]>,
    tags: HashMap<usize, usize>,
    defs: HashMap<usize, usize>,
    items: usize,
    limit_hit: bool,
}

struct Build<'c, 's> {
    cx: &'c mut Ctx<'s>,
    g: BulkGeometry,
    vmap: HashMap<usize, usize>,
    edges: HashSet<usize>,
    instances: Vec<Instance>,
    warns: Vec<String>,
    face_assignments: BTreeMap<usize, (Option<usize>, bool)>,
    edge_tags: BTreeMap<usize, usize>,
}

impl Build<'_, '_> {
    fn budget(&mut self) -> bool {
        if self.cx.items >= MAX_ITEMS {
            if !self.cx.limit_hit {
                self.cx.limit_hit = true;
                self.warns.truncate(255);
                self.warns.push(
                    "legacy geometry record limit reached; remaining geometry skipped".into(),
                );
            }
            return false;
        }
        self.cx.items += 1;
        true
    }

    fn vertex(&mut self, slot: Option<usize>) -> Option<usize> {
        let slot = slot?;
        if let Some(&i) = self.vmap.get(&slot) {
            return Some(i);
        }
        let Some(Val::Vertex(p)) = self.cx.ar.val(Some(slot)) else {
            return None;
        };
        let p = *p;
        if !p.is_finite() || !self.budget() {
            return None;
        }
        let i = self.g.vertices.len();
        self.g.vertices.push(p);
        self.vmap.insert(slot, i);
        Some(i)
    }

    fn edge(&mut self, slot: usize) -> Option<(usize, usize)> {
        let Some(Val::Edge { v1, v2, db }) = self.cx.ar.val(Some(slot)) else {
            return None;
        };
        let (v1, v2, db) = (*v1, *v2, *db);
        let a = self.vertex(v1)?;
        let b = self.vertex(v2)?;
        if self.edges.insert(slot) && a != b && self.budget() {
            if let Some(tag) = db.layer.and_then(|id| self.cx.tags.get(&id)).copied() {
                self.edge_tags.insert(self.g.edges.len(), tag);
            }
            self.g.edges.push(BulkEdge {
                vertices: [a, b],
                soft: db.soft != 0,
                smooth: db.smooth != 0,
                hidden: db.hidden != 0,
            });
        }
        Some((a, b))
    }

    fn ring(&mut self, loop_slot: Option<usize>) -> Option<Vec<usize>> {
        let Some(Val::Loop(uses)) = self.cx.ar.val(loop_slot) else {
            return None;
        };
        let mut out: Vec<usize> = Vec::with_capacity(uses.len());
        let mut expected = None;
        for &(edge, sense) in uses {
            if !self.budget() {
                return None;
            }
            let (a, b) = self.edge(edge?)?;
            let start = if sense == 0 { a } else { b };
            if expected.is_some_and(|end| end != start) {
                return None;
            }
            expected = Some(if sense == 0 { b } else { a });
            if out.last() != Some(&start) {
                out.push(start);
            }
        }
        if expected != out.first().copied() || out.len() < 3 {
            return None;
        }
        if out.len() > 1 && out.first() == out.last() {
            out.pop();
        }
        Some(out)
    }

    fn entity(&mut self, slot: usize) {
        let ar = self.cx.ar;
        match ar.slots.get(&slot) {
            Some(Entry::Obj(K::Edge, Val::Edge { .. })) => {
                if self.edge(slot).is_none() {
                    self.warns.truncate(255);
                    self.warns
                        .push(format!("edge record {slot} has unresolved vertices"));
                }
            }
            Some(Entry::Obj(K::Face, Val::Face { db, loops })) => {
                let mut rings = Vec::with_capacity(loops.len());
                for &l in loops {
                    match self.ring(l) {
                        Some(r) => rings.push(r),
                        None => {
                            self.warns.truncate(255);
                            self.warns.push(format!(
                                "face record {slot} has an unresolved loop; face skipped"
                            ));
                            return;
                        }
                    }
                }
                let mut rings = rings.into_iter();
                let Some(outer) = rings.next().filter(|o| o.len() >= 3) else {
                    self.warns.truncate(255);
                    self.warns.push(format!(
                        "face record {slot} has a degenerate outer loop; face skipped"
                    ));
                    return;
                };
                let holes: Vec<Vec<usize>> = rings.collect();
                if !self.budget() {
                    return;
                }
                let mi = self.cx.mats.get(&db.mat).copied();
                self.face_assignments.insert(
                    self.g.faces.len(),
                    (
                        db.layer.and_then(|id| self.cx.tags.get(&id)).copied(),
                        db.hidden != 0,
                    ),
                );
                self.g.faces.push(BulkFace {
                    outer,
                    holes,
                    material: mi.map(|i| MaterialId(i as u64 + 1)),
                    color: mi
                        .and_then(|i| self.cx.colors.get(i).copied())
                        .unwrap_or([255; 4]),
                });
            }
            Some(Entry::Obj(K::Instance | K::Image, Val::Instance { db, def, xf, name })) => {
                let Some(&definition) = def.and_then(|d| self.cx.defs.get(&d)) else {
                    self.warns.truncate(255);
                    self.warns.push(format!(
                        "instance record {slot} references an unrecovered definition"
                    ));
                    return;
                };
                let Some(transform) = transform(xf) else {
                    self.warns.truncate(255);
                    self.warns
                        .push(format!("instance record {slot} has a non-finite transform"));
                    return;
                };
                let tag = db.layer.and_then(|l| self.cx.tags.get(&l).copied());
                self.instances.push(Instance {
                    name: name.clone(),
                    definition,
                    transform,
                    tag,
                    material: self.cx.mats.get(&db.mat).map(|i| MaterialId(*i as u64 + 1)),
                    hidden: db.hidden != 0,
                });
            }
            _ => {}
        }
    }
}

fn build(cx: &mut Ctx, ents: &[Option<usize>], warns: &mut Vec<String>) -> Definition {
    let mut b = Build {
        cx,
        g: BulkGeometry::default(),
        vmap: HashMap::new(),
        edges: HashSet::new(),
        instances: Vec::new(),
        warns: Vec::new(),
        face_assignments: BTreeMap::new(),
        edge_tags: BTreeMap::new(),
    };
    for &s in ents.iter().flatten() {
        b.entity(s);
    }
    warns.append(&mut b.warns);
    warns.truncate(256);
    Definition {
        geometry: b.g,
        instances: b.instances,
        face_assignments: b.face_assignments,
        edge_tags: b.edge_tags,
        ..Default::default()
    }
}

fn build_scene(ar: &Ar, st: &Progress, version: &str, failure: Option<&Fail>) -> Scene {
    let mut scene = Scene {
        version: version.to_string(),
        ..Scene::default()
    };
    let mut cx = Ctx {
        ar,
        mats: HashMap::new(),
        colors: Vec::new(),
        tags: HashMap::new(),
        defs: HashMap::new(),
        items: 0,
        limit_hit: false,
    };
    let mut warns: Vec<String> = Vec::new();
    let mut tex_bytes = 0usize;
    let mut dib_keys: HashMap<(usize, usize), String> = HashMap::new();
    for &s in &st.materials {
        if cx.mats.contains_key(&s) {
            continue;
        }
        let Some(Val::Material(m)) = ar.val(Some(s)) else {
            continue;
        };
        let mut opacity = f32::from(m.rgba[3]) / 255.0;
        if m.use_opacity != 0 && m.opacity.is_finite() {
            opacity *= (1.0 - m.opacity).clamp(0.0, 1.0) as f32;
        }
        let mut texture = None;
        if let Some(t) = &m.tex {
            let range = t.inline.or_else(|| match ar.val(t.dib) {
                Some(Val::Dib { start, len }) => Some((*start, *len)),
                _ => None,
            });
            if let Some((start, len)) = range.filter(|r| r.1 > 0) {
                if let Some(key) = dib_keys.get(&(start, len)) {
                    texture = Some(key.clone());
                } else if tex_bytes.saturating_add(len) > MAX_BYTES {
                    warns.push(format!(
                        "texture for material {} skipped: texture byte limit",
                        m.name
                    ));
                } else if let Some(bytes) = ar.d.get(start..start + len) {
                    let png = bytes.starts_with(b"\x89PNG");
                    let base = t
                        .file
                        .rsplit(['/', '\\'])
                        .next()
                        .unwrap_or("")
                        .trim()
                        .to_string();
                    let base = if base.is_empty() {
                        format!("{}.{}", m.name, if png { "png" } else { "jpg" })
                    } else {
                        base
                    };
                    let mut key = base.clone();
                    let mut n = 1;
                    while scene.textures.contains_key(&key) {
                        n += 1;
                        key = format!("{n}_{base}");
                    }
                    tex_bytes += len;
                    scene.textures.insert(key.clone(), bytes.to_vec());
                    dib_keys.insert((start, len), key.clone());
                    texture = Some(key);
                }
            }
        }
        cx.mats.insert(s, scene.materials.len());
        cx.colors.push(m.rgba);
        scene.materials.push(Material {
            name: m.name.clone(),
            color: [
                srgb_to_linear(m.rgba[0]),
                srgb_to_linear(m.rgba[1]),
                srgb_to_linear(m.rgba[2]),
            ],
            opacity,
            texture,
        });
    }
    for &s in &st.layers {
        if cx.tags.contains_key(&s) {
            continue;
        }
        if let Some(Val::Layer { name, hidden }) = ar.val(Some(s)) {
            cx.tags.insert(s, scene.tags.len());
            scene.tags.push(Tag {
                name: name.clone(),
                visible: !hidden,
            });
        }
    }
    let mut def_slots: Vec<usize> = ar
        .slots
        .iter()
        .filter(|(_, e)| matches!(e, Entry::Obj(K::Definition, Val::Definition { .. })))
        .map(|(s, _)| *s)
        .collect();
    def_slots.sort_unstable();
    for (i, &s) in def_slots.iter().enumerate() {
        cx.defs.insert(s, i);
    }
    for &s in &def_slots {
        let Some(Val::Definition { name, ents }) = ar.val(Some(s)) else {
            continue;
        };
        let mut definition = build(&mut cx, ents, &mut warns);
        definition.name = name.clone();
        scene.definitions.push(definition);
    }
    let mut root = build(&mut cx, &st.root, &mut warns);
    if failure.is_none()
        || !root.geometry.vertices.is_empty()
        || !root.instances.is_empty()
        || !scene.definitions.is_empty()
    {
        let definition = scene.definitions.len();
        root.name = "Model".into();
        scene.definitions.push(root);
        scene.roots.push(Instance {
            name: "Model".into(),
            definition,
            transform: DMat4::IDENTITY,
            tag: None,
            material: None,
            hidden: false,
        });
    }
    for n in &ar.notes {
        scene.warn(n.clone());
    }
    if let Some(f) = failure {
        scene.warn(format!(
            "legacy archive walk stopped at offset {:#x}: {}; returning {} recovered definitions{}",
            f.at,
            f.msg,
            def_slots.len(),
            if st.root_read || !st.root.is_empty() {
                ""
            } else {
                " without root placements"
            }
        ));
    }
    for w in warns {
        scene.warn(w);
    }
    scene
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s16(out: &mut Vec<u8>, s: &str) {
        out.extend_from_slice(MARKER);
        let u: Vec<u16> = s.encode_utf16().collect();
        out.push(u.len() as u8);
        for c in u {
            out.extend_from_slice(&c.to_le_bytes());
        }
    }
    fn class(out: &mut Vec<u8>, name: &str) {
        out.extend_from_slice(&[0xff, 0xff, 1, 0]);
        out.extend_from_slice(&(name.len() as u16).to_le_bytes());
        out.extend_from_slice(name.as_bytes());
    }
    fn f(out: &mut Vec<u8>, v: &[f64]) {
        for x in v {
            out.extend_from_slice(&x.to_le_bytes());
        }
    }

    #[test]
    fn rejects_unsupported_versions() {
        assert!(parse(&[], 2, "{2.0}").is_err());
        assert!(parse(&[], 21, "{21.0}").is_err());
    }

    #[test]
    fn rejects_missing_anchor_and_truncation() {
        assert!(parse(b"\xff\xfe\xff\x0e garbage", 16, "{16.0}").is_err());
        let mut d = vec![0u8; 8];
        d.extend_from_slice(&[0xff, 0xff, 1, 0, 6, 0]);
        d.extend_from_slice(b"CLayer");
        assert!(parse(&d, 16, "{16.0}").is_err());
    }

    #[test]
    fn cursor_bounds_and_strings() {
        let mut d = Vec::new();
        s16(&mut d, "Hi");
        let mut r = Cur { d: &d, pos: 0 };
        assert_eq!(r.utf16().unwrap(), "Hi");
        assert!(r.u8().is_err());
        let bad = [0xff, 0xfe, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff];
        assert!(Cur { d: &bad, pos: 0 }.utf16().is_err());
    }

    #[test]
    fn walks_vertex_edge_loop_face_with_hole() {
        // Synthetic v16 archive fragment: square face with a triangular hole.
        let mut d = Vec::new();
        let mut ar_bytes = Vec::new();
        let base = 1usize;
        let mut ar = Ar::new(&[], 16, base);
        let _ = &mut ar;
        // Build face record bytes directly.
        let pts = [
            [0.0, 0.0, 0.0],
            [10.0, 0.0, 0.0],
            [10.0, 10.0, 0.0],
            [0.0, 10.0, 0.0],
            [2.0, 2.0, 0.0],
            [5.0, 2.0, 0.0],
            [2.0, 5.0, 0.0],
        ];
        // slots: 1 CFace class, 2 face obj
        class(&mut ar_bytes, "CFace");
        ar_bytes.extend_from_slice(&[0, 0]); // attrs null (v16: no pid)
        ar_bytes.extend_from_slice(&[0; 8]); // drawbase
        ar_bytes.extend_from_slice(&[0, 0]); // layer id 0
        f(&mut ar_bytes, &[0.0, 0.0, 1.0, 0.0]);
        ar_bytes.extend_from_slice(&2u32.to_le_bytes());
        let mut next = 3u16;
        let mut cls: HashMap<&str, u16> = HashMap::new();
        let mut vslots: Vec<u16> = Vec::new();
        let put_class =
            |b: &mut Vec<u8>, name: &'static str, next: &mut u16, cls: &mut HashMap<&str, u16>| {
                if let Some(c) = cls.get(name) {
                    b.extend_from_slice(&(0x8000 | *c).to_le_bytes());
                } else {
                    class(b, name);
                    cls.insert(name, *next);
                    *next += 1;
                }
            };
        for (li, ring) in [vec![0usize, 1, 2, 3], vec![4, 5, 6]].iter().enumerate() {
            put_class(&mut ar_bytes, "CLoop", &mut next, &mut cls);
            let loop_slot = next;
            next += 1;
            ar_bytes.extend_from_slice(&[0, 0]); // attrs
            ar_bytes.extend_from_slice(&[0, 0]); // flags
            for k in 0..ring.len() {
                let (a, b) = (ring[k], ring[(k + 1) % ring.len()]);
                put_class(&mut ar_bytes, "CEdgeUse", &mut next, &mut cls);
                next += 1;
                ar_bytes.extend_from_slice(&[0, 0]);
                put_class(&mut ar_bytes, "CEdge", &mut next, &mut cls);
                next += 1;
                ar_bytes.extend_from_slice(&[0, 0]);
                ar_bytes.extend_from_slice(&[0; 8]);
                ar_bytes.extend_from_slice(&[0, 0]);
                for v in [a, b] {
                    if vslots.len() > v && vslots[v] != 0 {
                        ar_bytes.extend_from_slice(&vslots[v].to_le_bytes());
                    } else {
                        put_class(&mut ar_bytes, "CVertex", &mut next, &mut cls);
                        if vslots.len() <= v {
                            vslots.resize(v + 1, 0);
                        }
                        vslots[v] = next;
                        next += 1;
                        ar_bytes.extend_from_slice(&[0, 0]);
                        f(&mut ar_bytes, &pts[v]);
                    }
                }
                ar_bytes.extend_from_slice(&[0, 0]); // curve null
                ar_bytes.push(0); // sense forward
                ar_bytes.extend_from_slice(&loop_slot.to_le_bytes());
            }
            ar_bytes.extend_from_slice(&[0, 0]); // end of loop
            let _ = li;
        }
        ar_bytes.extend_from_slice(&[0, 0]); // back material
        d.extend_from_slice(&ar_bytes);
        let mut ar = Ar::new(&d, 16, base);
        let mut r = Cur { d: &d, pos: 0 };
        let face = ar.read_object(&mut r, None).unwrap();
        assert_eq!(r.pos, d.len());
        let st = Progress {
            root: vec![face.slot],
            root_read: true,
            ..Progress::default()
        };
        let scene = build_scene(&ar, &st, "{16.0}", None);
        let g = &scene.definitions[0].geometry;
        assert_eq!(g.faces.len(), 1);
        assert_eq!(g.faces[0].outer.len(), 4);
        assert_eq!(g.faces[0].holes.len(), 1);
        assert_eq!(g.faces[0].holes[0].len(), 3);
        assert_eq!(g.vertices.len(), 7);
        assert_eq!(g.edges.len(), 7);
        assert_eq!(g.vertices[g.faces[0].outer[2]], DVec3::new(10.0, 10.0, 0.0));
        assert_eq!(scene.roots.len(), 1);
        let mut back_only = d.clone();
        let end = back_only.len();
        back_only[end - 2..].copy_from_slice(&7u16.to_le_bytes());
        let mut archive = Ar::new(&back_only, 16, base);
        let mut cursor = Cur {
            d: &back_only,
            pos: 0,
        };
        let decoded = archive.read_object(&mut cursor, None).unwrap();
        assert!(matches!(archive.val(decoded.slot), Some(Val::Face {db, ..}) if db.mat == 7));
        for cut in (0..d.len()).step_by(17) {
            let mut archive = Ar::new(&d[..cut], 16, base);
            let mut cursor = Cur {
                d: &d[..cut],
                pos: 0,
            };
            let _ = archive.read_object(&mut cursor, None);
        }
        for i in (0..d.len()).step_by(7) {
            let mut mutated = d.clone();
            mutated[i] ^= 0xff;
            let mut archive = Ar::new(&mutated, 16, base);
            let mut cursor = Cur {
                d: &mutated,
                pos: 0,
            };
            let _ = archive.read_object(&mut cursor, None);
        }
    }

    #[test]
    fn unknown_class_stops_without_panic() {
        let mut d = Vec::new();
        class(&mut d, "CMystery");
        d.extend_from_slice(&[1, 2, 3]);
        let mut ar = Ar::new(&d, 17, 1);
        let mut r = Cur { d: &d, pos: 0 };
        let e = ar.read_object(&mut r, None).unwrap_err();
        assert!(e.msg.contains("unsupported record class"));
        let mut out = Vec::new();
        let mut r = Cur { d: &d, pos: 0 };
        ar.entity_list(&mut r, 3, Owner::Root, &mut out).unwrap();
        assert!(out.is_empty());
        assert_eq!(ar.notes.len(), 1);
    }

    #[test]
    fn row_major_transform() {
        let xf = [0.0, -1.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 5.0, 6.0, 7.0];
        let m = transform(&xf).unwrap();
        let p = m.transform_point3(DVec3::X);
        assert!((p - DVec3::new(5.0, 7.0, 7.0)).length() < 1e-12);
        assert!(transform(&[f64::NAN; 12]).is_none());
    }

    #[test]
    fn fuzz_prefixes_never_panic() {
        let mut d = vec![0u8; 4];
        d.extend_from_slice(&3u32.to_le_bytes());
        class(&mut d, "CMaterial");
        s16(&mut d, "Red");
        for i in 0..d.len() {
            let _ = parse(&d[..i], 17, "{17.0}");
            let mut m = d.clone();
            m[i] ^= 0xA5;
            let _ = parse(&m, 16, "{16.0}");
        }
    }
}
