//! Linearithmic mesh construction, independent of the editing arrangement algorithm.
use crate::topology::canonical_ring;
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
/// Cleanup counts reported through the import summary.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BulkStats {
    /// Exact duplicate faces removed (same oriented boundary and material).
    pub duplicates_removed: usize,
    /// Coplanar regions that fell back to pairwise joins.
    pub region_fallbacks: usize,
}
/// Merged region rings may not exceed the accepted input ring length.
const MAX_MERGED_RING: usize = 4096;
/// Bounded cleanup work per build; exhausted regions fall back to pairwise joins.
/// Every region step is linear in region size, so this only guards hostile inputs
/// whose aggregated validation cost (ring work) would otherwise exceed the input budget.
const REGION_WORK_BUDGET: usize = 64_000_000;
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
        self.build_cancellable_with_stats(merge, soften, cancelled)
            .map(|(model, _)| model)
    }
    /// Build with cooperative cancellation and report cleanup statistics.
    pub fn build_cancellable_with_stats(
        self,
        merge: bool,
        soften: bool,
        cancelled: impl Fn() -> bool,
    ) -> Result<(Model, BulkStats), KernelError> {
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
        let mut stats = BulkStats::default();
        // Exact duplicates (same oriented boundary and material) keep one representative;
        // opposite-winding copies are distinct on purpose (double-sided surfaces).
        let mut seen = std::collections::BTreeSet::new();
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
            if merge {
                let mut hole_keys: Vec<_> = holes.iter().map(|r| canonical_ring(r)).collect();
                hole_keys.sort();
                let key = (canonical_ring(&outer), hole_keys, f.material);
                if !seen.insert(key) {
                    stats.duplicates_removed += 1;
                    continue;
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
        // Coplanar seam qualification: matching seams soften, same-orientation pairs
        // become region merge candidates. Opposite-winding pairs stay unmerged.
        let mut merge_pairs = Vec::new();
        for (pair, faces) in &adjacency {
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
                    .chain(g.holes.iter().flatten())
                    .any(|v| f.plane.signed_distance(model.vertices[v].position).abs() > TOLERANCE)
            {
                continue;
            }
            if soften {
                let e = model
                    .edges
                    .get_mut(&pairs[pair])
                    .ok_or(KernelError::MissingEntity)?;
                e.soft = true;
                e.smooth = true;
            }
            if merge && f.plane.normal.dot(g.plane.normal) > 1. - 1e-10 {
                merge_pairs.push((a, b, *pair));
            }
        }
        if merge && !merge_pairs.is_empty() {
            region_merge(&mut model, &pairs, &merge_pairs, &mut stats, &check)?;
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
        Ok((model, stats))
    }
}
type Check<'a> = &'a (dyn Fn() -> Result<(), KernelError> + 'a);
/// Union coplanar same-orientation edge-connected face groups into single faces.
/// Deterministic throughout: pairs arrive sorted by edge key and members by face id.
fn region_merge(
    model: &mut Model,
    edges: &BTreeMap<[VertexId; 2], EdgeId>,
    merge_pairs: &[(FaceId, FaceId, [VertexId; 2])],
    stats: &mut BulkStats,
    check: Check,
) -> Result<(), KernelError> {
    let mut participating = std::collections::BTreeSet::new();
    for (a, b, _) in merge_pairs {
        participating.insert(*a);
        participating.insert(*b);
    }
    let ids: Vec<FaceId> = participating.into_iter().collect();
    let index: HashMap<FaceId, usize> = ids.iter().enumerate().map(|(i, id)| (*id, i)).collect();
    let mut parent: Vec<usize> = (0..ids.len()).collect();
    let mut work = 0usize;
    fn root(parent: &mut [usize], mut i: usize) -> usize {
        while parent[i] != i {
            parent[i] = parent[parent[i]];
            i = parent[i];
        }
        i
    }
    for (a, b, _) in merge_pairs {
        work += 1;
        let (ra, rb) = (root(&mut parent, index[a]), root(&mut parent, index[b]));
        if ra != rb {
            parent[rb] = ra;
        }
    }
    let mut groups = BTreeMap::<usize, Vec<FaceId>>::new();
    let mut group_pairs = BTreeMap::<usize, Vec<(FaceId, FaceId, [VertexId; 2])>>::new();
    for (i, id) in ids.iter().enumerate() {
        groups.entry(root(&mut parent, i)).or_default().push(*id);
    }
    for (a, b, pair) in merge_pairs {
        let r = root(&mut parent, index[a]);
        group_pairs.entry(r).or_default().push((*a, *b, *pair));
    }
    for (r, members) in groups {
        if members.len() < 2 {
            continue;
        }
        let pairs = group_pairs.remove(&r).unwrap_or_default();
        merge_region(model, edges, &members, &pairs, stats, check, &mut work)?;
    }
    Ok(())
}
/// Merge one connected coplanar group, splitting oversized regions deterministically
/// and falling back to pairwise joins when a region cannot produce a valid planar face.
fn merge_region(
    model: &mut Model,
    edges: &BTreeMap<[VertexId; 2], EdgeId>,
    members: &[FaceId],
    pairs: &[(FaceId, FaceId, [VertexId; 2])],
    stats: &mut BulkStats,
    check: Check,
    work: &mut usize,
) -> Result<(), KernelError> {
    check()?;
    if members.len() < 2 {
        return Ok(());
    }
    if *work > REGION_WORK_BUDGET {
        stats.region_fallbacks += 1;
        legacy_pairwise(model, edges, pairs);
        return Ok(());
    }
    *work += members.len() + pairs.len();
    match reconstruct_region(model, members)? {
        RegionOutcome::Merged(candidate) => {
            if candidate.ring_work > REGION_WORK_BUDGET {
                split_region(model, edges, members, pairs, stats, check, work)
            } else {
                commit_region(model, edges, members, candidate);
                Ok(())
            }
        }
        RegionOutcome::TooLarge => {
            split_region(model, edges, members, pairs, stats, check, work)
        }
        RegionOutcome::Fallback => {
            stats.region_fallbacks += 1;
            legacy_pairwise(model, edges, pairs);
            Ok(())
        }
    }
}
/// Split a region into a connected BFS prefix half and the remaining components,
/// then merge each part. Halving terminates; cut edges stay as softened seams.
fn split_region(
    model: &mut Model,
    edges: &BTreeMap<[VertexId; 2], EdgeId>,
    members: &[FaceId],
    pairs: &[(FaceId, FaceId, [VertexId; 2])],
    stats: &mut BulkStats,
    check: Check,
    work: &mut usize,
) -> Result<(), KernelError> {
    for component in bfs_components(members, pairs) {
        if component.len() < 2 {
            continue;
        }
        let half = component.len() / 2;
        let left: std::collections::BTreeSet<FaceId> = component[..half].iter().copied().collect();
        let left_pairs: Vec<_> = pairs
            .iter()
            .filter(|(a, b, _)| left.contains(a) && left.contains(b))
            .copied()
            .collect();
        merge_region(model, edges, &component[..half], &left_pairs, stats, check, work)?;
        let rest: Vec<FaceId> = component[half..].to_vec();
        let rest_pairs: Vec<_> = pairs
            .iter()
            .filter(|(a, b, _)| {
                !left.contains(a) && !left.contains(b)
            })
            .copied()
            .collect();
        for sub in bfs_components(&rest, &rest_pairs) {
            let set: std::collections::BTreeSet<_> = sub.iter().copied().collect();
            let sub_pairs: Vec<_> = rest_pairs
                .iter()
                .filter(|(a, b, _)| set.contains(a) && set.contains(b))
                .copied()
                .collect();
            merge_region(model, edges, &sub, &sub_pairs, stats, check, work)?;
        }
    }
    Ok(())
}
/// Connected components in deterministic BFS order (roots in member order).
fn bfs_components(
    members: &[FaceId],
    pairs: &[(FaceId, FaceId, [VertexId; 2])],
) -> Vec<Vec<FaceId>> {
    let mut adjacent: BTreeMap<FaceId, Vec<FaceId>> = BTreeMap::new();
    for (a, b, _) in pairs {
        adjacent.entry(*a).or_default().push(*b);
        adjacent.entry(*b).or_default().push(*a);
    }
    let mut remaining: std::collections::BTreeSet<_> = members.iter().copied().collect();
    let mut components = Vec::new();
    for start in members {
        if !remaining.remove(start) {
            continue;
        }
        let mut component = vec![*start];
        let mut queue = std::collections::VecDeque::from([*start]);
        while let Some(id) = queue.pop_front() {
            for next in adjacent.get(&id).into_iter().flatten() {
                if remaining.remove(next) {
                    component.push(*next);
                    queue.push_back(*next);
                }
            }
        }
        components.push(component);
    }
    components
}
enum RegionOutcome {
    Merged(RegionCandidate),
    /// The merged boundary cannot fit one bounded face; split the region instead.
    TooLarge,
    /// Self-touching boundary or unusable holes; fall back to pairwise joins.
    Fallback,
}
struct RegionCandidate {
    outer: Vec<VertexId>,
    holes: Vec<Vec<VertexId>>,
    plane: Plane,
    interior: Vec<[VertexId; 2]>,
    ring_work: usize,
}
/// Reconstruct one outer boundary plus inner hole loops for a coplanar region
/// without mutating the model, so failures can fall back cleanly.
fn reconstruct_region(model: &Model, members: &[FaceId]) -> Result<RegionOutcome, KernelError> {
    let mut boundary: BTreeMap<[VertexId; 2], (VertexId, VertexId, u32)> = BTreeMap::new();
    let mut corners: std::collections::BTreeSet<VertexId> = std::collections::BTreeSet::new();
    for id in members {
        let Some(face) = model.faces.get(id) else {
            return Err(KernelError::MissingEntity);
        };
        for ring in std::iter::once(&face.outer).chain(&face.holes) {
            for (a, b) in ring.iter().zip(ring.iter().cycle().skip(1)).take(ring.len()) {
                let mut pair = [*a, *b];
                pair.sort();
                let entry = boundary.entry(pair).or_insert((*a, *b, 0));
                entry.2 += 1;
            }
            corners.extend(ring.iter().copied());
        }
    }
    if boundary.values().any(|(_, _, n)| *n > 2) {
        return Ok(RegionOutcome::Fallback);
    }
    let mut next: BTreeMap<VertexId, VertexId> = BTreeMap::new();
    for (_, (a, b, n)) in &boundary {
        if *n == 1 && next.insert(*a, *b).is_some() {
            return Ok(RegionOutcome::Fallback);
        }
    }
    let mut rings: Vec<Vec<VertexId>> = Vec::new();
    let mut ring_work = 0usize;
    while let Some(start) = next.keys().next().copied() {
        let mut ring = vec![start];
        let mut current = start;
        loop {
            let Some(following) = next.remove(&current) else {
                return Ok(RegionOutcome::Fallback);
            };
            if following == start {
                break;
            }
            ring.push(following);
            if ring.len() > MAX_MERGED_RING {
                return Ok(RegionOutcome::TooLarge);
            }
            current = following;
        }
        if ring.len() < 3 {
            return Ok(RegionOutcome::Fallback);
        }
        ring_work = ring_work
            .checked_add(ring.len().checked_mul(ring.len()).ok_or(KernelError::Capacity)?)
            .ok_or(KernelError::Capacity)?;
        rings.push(ring);
    }
    // Classification against a member plane; all member planes agree within tolerance.
    let Some(first) = members.first() else {
        return Ok(RegionOutcome::Fallback);
    };
    let Some(face) = model.faces.get(first) else {
        return Err(KernelError::MissingEntity);
    };
    let (u, v) = face.plane.basis();
    let project = |ring: &[VertexId]| -> Result<Vec<glam::DVec2>, KernelError> {
        ring.iter()
            .map(|id| {
                let p = model.vertices[id].position;
                Ok(glam::DVec2::new(p.dot(u), p.dot(v)))
            })
            .collect()
    };
    let mut areas = Vec::with_capacity(rings.len());
    for ring in &rings {
        areas.push(crate::geometry::signed_area(&project(ring)?));
    }
    let outer_positions: Vec<usize> = areas
        .iter()
        .enumerate()
        .filter(|(_, area)| **area > 0.)
        .map(|(i, _)| i)
        .collect();
    if outer_positions.len() != 1 || areas[outer_positions[0]] <= TOLERANCE * TOLERANCE {
        return Ok(RegionOutcome::Fallback);
    }
    let outer = rings.remove(outer_positions[0]);
    let holes = rings;
    // Final plane: Newell normal of the outer boundary, offset by the mean corner
    // distance so the merged face passes the same planarity check as any other face.
    let origin = model.vertices[&outer[0]].position;
    let mut normal = DVec3::ZERO;
    for w in outer[1..].windows(2) {
        normal += (model.vertices[&w[0]].position - origin)
            .cross(model.vertices[&w[1]].position - origin);
    }
    if normal.length() <= TOLERANCE * TOLERANCE {
        return Ok(RegionOutcome::Fallback);
    }
    let normal = normal.normalize();
    let mut distance = 0.;
    for id in &corners {
        distance += normal.dot(model.vertices[id].position);
    }
    distance /= corners.len() as f64;
    let plane = Plane { normal, distance };
    for id in &corners {
        if plane.signed_distance(model.vertices[id].position).abs() > TOLERANCE {
            return Ok(RegionOutcome::Fallback);
        }
    }
    for ring in std::iter::once(&outer).chain(&holes) {
        if ring.iter().collect::<std::collections::BTreeSet<_>>().len() != ring.len() {
            return Ok(RegionOutcome::Fallback);
        }
    }
    let interior = boundary
        .iter()
        .filter(|(_, (_, _, n))| *n == 2)
        .map(|(pair, _)| *pair)
        .collect();
    Ok(RegionOutcome::Merged(RegionCandidate {
        outer,
        holes,
        plane,
        interior,
        ring_work,
    }))
}
/// Replace a region's faces with the merged face and drop its interior edges.
/// Removed edges keep their key mappings, so explicit source edges still cannot
/// restore a cleaned diagonal.
fn commit_region(
    model: &mut Model,
    edges: &BTreeMap<[VertexId; 2], EdgeId>,
    members: &[FaceId],
    candidate: RegionCandidate,
) {
    let Some(first) = members.first().copied() else {
        return;
    };
    let Some(face) = model.faces.get(&first) else {
        return;
    };
    let (color, material) = (face.color, face.material);
    for id in members {
        model.faces.remove(id);
    }
    model.faces.insert(
        first,
        Face {
            outer: candidate.outer,
            holes: candidate.holes,
            plane: candidate.plane,
            color,
            material,
        },
    );
    for pair in candidate.interior {
        if let Some(eid) = edges.get(&pair) {
            model.edges.remove(eid);
        }
    }
}
/// Pairwise triangle-to-quad join for one region: the pre-region-merge cleanup,
/// used when the region cannot produce a valid planar face. Infallible; pairs that
/// cannot join stay as softened seams.
fn legacy_pairwise(
    model: &mut Model,
    edges: &BTreeMap<[VertexId; 2], EdgeId>,
    pairs: &[(FaceId, FaceId, [VertexId; 2])],
) {
    let mut merged = std::collections::BTreeSet::new();
    for (a, b, pair) in pairs {
        let (Some(f), Some(g)) = (model.faces.get(a), model.faces.get(b)) else {
            continue;
        };
        if merged.contains(a)
            || merged.contains(b)
            || f.material != g.material
            || f.color != g.color
            || f.outer.len() != 3
            || g.outer.len() != 3
            || !f.holes.is_empty()
            || !g.holes.is_empty()
            || f.plane.normal.dot(g.plane.normal).abs() < 1. - 1e-10
            || g.outer
                .iter()
                .any(|v| f.plane.signed_distance(model.vertices[v].position).abs() > TOLERANCE)
        {
            continue;
        }
        let (f_outer, other) = {
            let mut other = g.outer.clone();
            if f.plane.normal.dot(g.plane.normal) < 0. {
                other.reverse();
            }
            (f.outer.clone(), other)
        };
        let Some(eid) = edges.get(pair).copied() else {
            continue;
        };
        let mut next = HashMap::new();
        for r in [&f_outer, &other] {
            for (x, y) in r.iter().zip(r.iter().cycle().skip(1)).take(r.len()) {
                let mut p = [*x, *y];
                p.sort();
                if p != *pair {
                    next.insert(*x, *y);
                }
            }
        }
        let Some(start) = f_outer.iter().copied().find(|v| next.contains_key(v)) else {
            continue;
        };
        let mut r = vec![start];
        for _ in 0..3 {
            match next.get(r.last().unwrap_or(&start)) {
                Some(n) => r.push(*n),
                None => break,
            }
        }
        if r.len() != 4 || next.get(&r[3]) != Some(&start) {
            continue;
        }
        if let Some(face) = model.faces.get_mut(a) {
            face.outer = r;
        }
        model.faces.remove(b);
        model.edges.remove(&eid);
        merged.insert(*a);
        merged.insert(*b);
    }
}
