//! Linearithmic mesh construction, independent of the editing arrangement algorithm.
use crate::*;
use std::collections::{BTreeMap, HashMap};
/// Indexed polygons and explicit edge styles in one local coordinate frame.
#[derive(Clone, Debug, Default)]
pub struct BulkGeometry {
    /// Positions before tolerance welding.
    pub vertices: Vec<DVec3>,
    /// Planar polygons, optionally with holes.
    pub faces: Vec<BulkFace>,
    /// Additional edges or style overrides.
    pub edges: Vec<BulkEdge>,
}
/// One planar polygon.
#[derive(Clone, Debug)]
pub struct BulkFace {
    /// Outer vertex indices.
    pub outer: Vec<usize>,
    /// Inner vertex indices.
    pub holes: Vec<Vec<usize>>,
    /// Document material identity.
    pub material: Option<MaterialId>,
    /// Display RGBA.
    pub color: [u8; 4],
}
impl Default for BulkFace {
    fn default() -> Self {
        Self {
            outer: Vec::new(),
            holes: Vec::new(),
            material: None,
            color: [255; 4],
        }
    }
}
/// An indexed edge and its presentation flags.
#[derive(Clone, Debug, Default)]
pub struct BulkEdge {
    /// Endpoint indices.
    pub vertices: [usize; 2],
    /// Soft seam.
    pub soft: bool,
    /// Interpolate normals independently of visibility.
    pub smooth: bool,
    /// Explicitly hidden edge.
    pub hidden: bool,
}
impl BulkGeometry {
    /// Weld positions, build boundaries directly, validate polygons, and build runtime indices.
    /// Surface intersections are preserved, without arrangement splitting. Counts and ring
    /// size are bounded so malformed meshes cannot trigger unbounded polygon validation.
    pub fn build(self, merge: bool, soften: bool) -> Result<Model, KernelError> {
        self.build_cancellable(merge, soften, || false)
    }
    /// Build with cooperative cancellation at bounded geometry intervals.
    pub fn build_cancellable(
        self,
        merge: bool,
        soften: bool,
        cancelled: impl Fn() -> bool,
    ) -> Result<Model, KernelError> {
        let check = || {
            if cancelled() {
                Err(KernelError::InvalidTopology("import cancelled".into()))
            } else {
                Ok(())
            }
        };
        if self.vertices.len() > 1_000_000
            || self.faces.len() > 1_000_000
            || self.edges.len() > 3_000_000
        {
            return Err(KernelError::Capacity);
        }
        let mut ring_work = 0usize;
        for face in &self.faces {
            for ring in std::iter::once(&face.outer).chain(&face.holes) {
                ring_work = ring_work
                    .checked_add(
                        ring.len()
                            .checked_mul(ring.len())
                            .ok_or(KernelError::Capacity)?,
                    )
                    .ok_or(KernelError::Capacity)?;
                if ring_work > 64_000_000 {
                    return Err(KernelError::Capacity);
                }
            }
        }
        let mut model = Model::default();
        let mut cells = HashMap::<[i64; 3], Vec<VertexId>>::new();
        let mut ids = Vec::with_capacity(self.vertices.len());
        for (i, p) in self.vertices.into_iter().enumerate() {
            if i % 1024 == 0 {
                check()?;
            }
            crate::geometry::validate_position(p)?;
            let c = (p / TOLERANCE).floor().as_i64vec3().to_array();
            let mut found = None;
            for x in -1..=1 {
                for y in -1..=1 {
                    for z in -1..=1 {
                        if let Some(bucket) = cells.get(&[c[0] + x, c[1] + y, c[2] + z]) {
                            for id in bucket {
                                if model.vertices[id].position.distance(p) <= TOLERANCE {
                                    found = Some(*id);
                                    break;
                                }
                            }
                        }
                    }
                }
            }
            let id = if let Some(id) = found {
                id
            } else {
                let id = VertexId::from_raw(model.allocate()?);
                model.vertices.insert(id, Vertex { position: p });
                cells.entry(c).or_default().push(id);
                id
            };
            ids.push(id);
        }
        let mut pairs = BTreeMap::<[VertexId; 2], EdgeId>::new();
        let mut adjacency = BTreeMap::<[VertexId; 2], Vec<FaceId>>::new();
        for (i, f) in self.faces.into_iter().enumerate() {
            if i % 1024 == 0 {
                check()?;
            }
            let ring = |r: Vec<usize>| -> Result<Vec<VertexId>, KernelError> {
                if r.len() < 3 || r.len() > 4096 {
                    return Err(KernelError::InvalidPolyline);
                }
                r.into_iter()
                    .map(|i| ids.get(i).copied().ok_or(KernelError::MissingEntity))
                    .collect()
            };
            let mut outer = ring(f.outer)?;
            let mut holes = f
                .holes
                .into_iter()
                .map(ring)
                .collect::<Result<Vec<_>, _>>()?;
            let a = model.position(outer[0])?;
            let mut normal = DVec3::ZERO;
            for w in outer[1..].windows(2) {
                normal += (model.position(w[0])? - a).cross(model.position(w[1])? - a);
            }
            if normal.length() <= TOLERANCE * TOLERANCE {
                return Err(KernelError::InvalidPolyline);
            }
            let plane = Plane {
                normal: normal.normalize(),
                distance: normal.normalize().dot(a),
            };
            if crate::geometry::signed_area(&model.project_loop(&outer, plane)?) < 0. {
                outer.reverse();
            }
            for h in &mut holes {
                if crate::geometry::signed_area(&model.project_loop(h, plane)?) > 0. {
                    h.reverse();
                }
            }
            let fid = FaceId::from_raw(model.allocate()?);
            for r in std::iter::once(&outer).chain(&holes) {
                for (a, b) in r.iter().zip(r.iter().cycle().skip(1)).take(r.len()) {
                    let mut pair = [*a, *b];
                    pair.sort();
                    if let std::collections::btree_map::Entry::Vacant(entry) = pairs.entry(pair) {
                        let eid = EdgeId::from_raw(model.allocate()?);
                        model.edges.insert(
                            eid,
                            Edge {
                                vertices: pair,
                                soft: false,
                                smooth: false,
                                hidden: false,
                            },
                        );
                        entry.insert(eid);
                    }
                    adjacency.entry(pair).or_default().push(fid);
                }
            }
            model.faces.insert(
                fid,
                Face {
                    outer,
                    holes,
                    plane,
                    color: f.color,
                    material: f.material,
                },
            );
        }
        // Pairwise disjoint triangle joins avoid repeatedly growing/rechecking large rings.
        let mut merged = std::collections::BTreeSet::new();
        for (pair, faces) in adjacency {
            if faces.len() != 2 {
                continue;
            }
            let (a, b) = (faces[0], faces[1]);
            let (Some(f), Some(g)) = (model.faces.get(&a), model.faces.get(&b)) else {
                continue;
            };
            if f.material != g.material
                || f.color != g.color
                || f.plane.normal.dot(g.plane.normal).abs() < 1. - 1e-10
                || g.outer
                    .iter()
                    .any(|v| f.plane.signed_distance(model.vertices[v].position).abs() > TOLERANCE)
            {
                continue;
            }
            let eid = pairs[&pair];
            if soften {
                let e = model
                    .edges
                    .get_mut(&eid)
                    .ok_or(KernelError::MissingEntity)?;
                e.soft = true;
                e.smooth = true;
            }
            if !merge
                || merged.contains(&a)
                || merged.contains(&b)
                || f.outer.len() != 3
                || g.outer.len() != 3
                || !f.holes.is_empty()
                || !g.holes.is_empty()
            {
                continue;
            }
            let mut next = HashMap::new();
            let mut other = g.outer.clone();
            if f.plane.normal.dot(g.plane.normal) < 0. {
                other.reverse();
            }
            for r in [&f.outer, &other] {
                for (x, y) in r.iter().zip(r.iter().cycle().skip(1)).take(r.len()) {
                    let mut p = [*x, *y];
                    p.sort();
                    if p != pair {
                        next.insert(*x, *y);
                    }
                }
            }
            let start = f
                .outer
                .iter()
                .copied()
                .find(|v| next.contains_key(v))
                .ok_or(KernelError::InvalidPolyline)?;
            let mut r = vec![start];
            for _ in 0..3 {
                let n = *next
                    .get(r.last().ok_or(KernelError::InvalidPolyline)?)
                    .ok_or(KernelError::InvalidPolyline)?;
                r.push(n);
            }
            if next.get(&r[3]) != Some(&start) {
                continue;
            }
            model
                .faces
                .get_mut(&a)
                .ok_or(KernelError::MissingEntity)?
                .outer = r;
            model.faces.remove(&b);
            model.edges.remove(&eid);
            merged.insert(a);
            merged.insert(b);
        }
        for e in self.edges {
            let mut pair = [
                *ids.get(e.vertices[0]).ok_or(KernelError::MissingEntity)?,
                *ids.get(e.vertices[1]).ok_or(KernelError::MissingEntity)?,
            ];
            pair.sort();
            if pair[0] == pair[1] {
                continue;
            }
            if let Some(edge) = pairs.get(&pair).and_then(|id| model.edges.get_mut(id)) {
                edge.soft |= e.soft;
                edge.smooth |= e.smooth;
                edge.hidden |= e.hidden;
            } else if let std::collections::btree_map::Entry::Vacant(entry) = pairs.entry(pair) {
                let id = EdgeId::from_raw(model.allocate()?);
                model.edges.insert(
                    id,
                    Edge {
                        vertices: pair,
                        soft: e.soft,
                        smooth: e.smooth,
                        hidden: e.hidden,
                    },
                );
                entry.insert(id);
            }
        }
        check()?;
        model.validate_mesh(false)?;
        model.ensure_spatial()?;
        // Validate the newly built adjacency and spatial indices too, as edits do.
        #[cfg(debug_assertions)]
        model.validate_mesh(false)?;
        Ok(model)
    }
}
