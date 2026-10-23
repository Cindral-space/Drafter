//! Source-local SKP repair. Source positions remain inches; predicates use meters.
use crate::{ImportControl, ImportedScene, IoError, Result};
use cindral_core::{BulkFace, BulkGeometry, DVec3, TOLERANCE};
use std::collections::{BTreeMap, HashMap};

// 0.001 inch in meters. This compatibility allowance does not loosen the kernel.
const PROJECTION_TOLERANCE: f64 = 0.001 * 0.0254;
type Assignments = BTreeMap<usize, (Option<usize>, bool)>;

fn sample(geometry: &BulkGeometry, face: &BulkFace) -> Result<BulkGeometry> {
    let mut local = BulkGeometry::default();
    let mut face = face.clone();
    for ring in std::iter::once(&mut face.outer).chain(&mut face.holes) {
        for index in ring {
            let p = geometry.vertices.get(*index).ok_or_else(|| IoError::Invalid("face references missing vertex".into()))?;
            *index = local.vertices.len();
            local.vertices.push(*p * 0.0254);
        }
    }
    local.faces.push(face);
    Ok(local)
}
fn valid(geometry: &BulkGeometry, face: &BulkFace) -> bool {
    sample(geometry, face).and_then(|g| g.build(false, false).map_err(Into::into)).is_ok()
}

// Decompose a weakly simple closed walk at repeated welded vertices. Opposite
// traversals of a bridge form two-point walks; their source edges remain intact.
fn cycles(ring: &[usize]) -> Vec<Vec<usize>> {
    let mut path = Vec::new();
    let mut positions = HashMap::new();
    let mut out = Vec::new();
    for &v in ring.iter().chain(ring.first()) {
        if let Some(&start) = positions.get(&v) {
            let cycle = path[start..].to_vec();
            if cycle.len() >= 3 { out.push(cycle); }
            for old in path.drain(start+1..) { positions.remove(&old); }
        } else {
            positions.insert(v, path.len());
            path.push(v);
        }
    }
    out
}
fn signed_area(ring: &[usize], vertices: &[DVec3], center: DVec3, normal: DVec3) -> f64 {
    ring.iter().zip(ring.iter().cycle().skip(1)).take(ring.len()).map(|(&a,&b)| (vertices[a]*0.0254-center).cross(vertices[b]*0.0254-center).dot(normal)*0.5).sum()
}
fn degenerate(ring: &[usize], vertices: &[DVec3]) -> bool {
    let Some(&first) = ring.first() else { return true; };
    let a = vertices[first]*0.0254;
    let b = ring.iter().map(|&i| vertices[i]*0.0254).max_by(|b,c| b.distance_squared(a).total_cmp(&c.distance_squared(a))).unwrap_or(a);
    ring.iter().all(|&i| (b-a).cross(vertices[i]*0.0254-a).length() <= TOLERANCE*TOLERANCE)
}

pub(crate) fn recover(mut source: BulkGeometry, labels: &[String], assignments: &mut Assignments, scene: &mut ImportedScene, control: &ImportControl) -> Result<BulkGeometry> {
    let original_assignments = std::mem::take(assignments);
    let faces = std::mem::take(&mut source.faces);
    // Match the kernel's global insertion-order tolerance welding, including
    // orphan vertices, so repairs see exactly the same ring identities.
    let mut cells: HashMap<[i64;3], Vec<usize>> = HashMap::new();
    let mut canonical = Vec::with_capacity(source.vertices.len());
    for (i, p) in source.vertices.iter().enumerate() {
        if i % 1024 == 0 { control.check()?; }
        let p = *p * 0.0254;
        if !p.is_finite() || p.abs().max_element() > cindral_core::MAX_COORDINATE {
            return Err(IoError::Invalid(format!("SKP vertex {i}: invalid/out-of-range position; import stopped without dropping geometry")));
        }
        let c = (p/TOLERANCE).floor().as_i64vec3().to_array();
        let mut found = None;
        for x in -1..=1 { for y in -1..=1 { for z in -1..=1 {
            if let Some(bucket) = cells.get(&[c[0]+x,c[1]+y,c[2]+z]) {
                for &j in bucket { if (source.vertices[j]*0.0254).distance(p) <= TOLERANCE { found = Some(j); break; } }
            }
        }}}
        let representative = found.unwrap_or(i);
        canonical.push(representative);
        if found.is_none() { cells.entry(c).or_default().push(i); }
    }
    let mut work = 0usize;
    for (index, mut face) in faces.into_iter().enumerate() {
        control.check()?;
        let label = labels.get(index).cloned().unwrap_or_else(|| format!("face {index}"));
        let points: Vec<_> = std::iter::once(&face.outer).chain(&face.holes).flatten().map(|&i| source.vertices.get(i).copied().map(|p| p*0.0254).ok_or_else(|| IoError::Invalid(format!("{label}: missing vertex")))).collect::<Result<_>>()?;
        let center = points.iter().copied().sum::<DVec3>() / points.len().max(1) as f64;
        let location = format!("{label}, local position ({:.6}, {:.6}, {:.6}) m", center.x, center.y, center.z);
        let size = points.len();
        work = work.saturating_add(size.saturating_mul(size));
        if size > 4096 || work > 64_000_000 { return Err(IoError::Invalid(format!("{location}: repair work limit; import stopped without dropping geometry"))); }
        let start = source.faces.len();
        let mut actions = Vec::new();
        let original_error = sample(&source, &face)?.build(false, false).err();
        if original_error.is_none() {
            source.faces.push(face);
        } else {
            for ring in std::iter::once(&mut face.outer).chain(&mut face.holes) { for i in ring { *i = canonical[*i]; } }
            if degenerate(&face.outer, &source.vertices) {
                scene.skipped += 1;
                scene.warnings.push(format!("1 face skipped: {location}; degenerate outer loop (zero area or fewer than 3 distinct vertices)"));
                continue;
            }
            let (center, normal, deviation, _) = crate::skp_plane::fit(&points);
            let old_outer = face.outer.clone();
            let mut rings = cycles(&face.outer);
            if rings.is_empty() { return Err(IoError::Invalid(format!("{location}: nondegenerate ring could not be decomposed"))); }
            let largest = (0..rings.len()).max_by(|&a,&b| signed_area(&rings[a], &source.vertices, center, normal).abs().total_cmp(&signed_area(&rings[b], &source.vertices, center, normal).abs())).unwrap();
            face.outer = rings.remove(largest);
            let old_holes = std::mem::take(&mut face.holes);
            for hole in &old_holes { rings.extend(cycles(hole)); }
            face.holes = rings;
            if face.outer != old_outer || face.holes != old_holes { actions.push("decomposing retraced/welded loops into simple boundaries".to_string()); }
            if valid(&source, &face) {
                source.faces.push(face);
            } else {
                // Validate the projected footprint, including holes, before using
                // earcut. Invalid nondegenerate topology is an explicit error,
                // never silently dropped or filled with a fan.
                let mut projected = sample(&source, &face)?;
                for p in &mut projected.vertices { *p -= normal * normal.dot(*p-center); }
                let footprint = projected.clone().build(false, false).map_err(|e| IoError::Invalid(format!("{location}: projected footprint invalid ({e}); original check {original_error:?}; import stopped without dropping geometry")))?;
                if deviation <= PROJECTION_TOLERANCE {
                    let projected_face = &projected.faces[0];
                    for (ring, local_ring) in std::iter::once(&mut face.outer).chain(&mut face.holes).zip(std::iter::once(&projected_face.outer).chain(&projected_face.holes)) {
                        for (i, &j) in ring.iter_mut().zip(local_ring) {
                            *i = source.vertices.len();
                            source.vertices.push(projected.vertices[j] / 0.0254);
                        }
                    }
                    source.faces.push(face);
                    actions.push(format!("projecting to its best-fit plane (max deviation {:.6} mm, tolerance 0.0254 mm)", deviation*1000.));
                } else {
                    let axis = if normal.x.abs() < 0.9 { DVec3::X } else { DVec3::Y };
                    let u = normal.cross(axis).normalize();
                    let v = normal.cross(u);
                    let mut coordinates = Vec::new();
                    let mut indices = Vec::new();
                    let mut holes = Vec::new();
                    for (r, ring) in std::iter::once(&face.outer).chain(&face.holes).enumerate() {
                        if r>0 { holes.push(indices.len()); }
                        for &i in ring { let p = source.vertices[i]*0.0254-center; coordinates.extend([p.dot(u), p.dot(v)]); indices.push(i); }
                    }
                    let triangles = earcutr::earcut(&coordinates, &holes, 2).map_err(|e| IoError::Invalid(format!("{location}: triangulation failed: {e}")))?;
                    let area: f64 = triangles.chunks_exact(3).map(|t| {
                        let p = |i: usize| glam::DVec2::new(coordinates[2*t[i]], coordinates[2*t[i]+1]);
                        ((p(1)-p(0)).perp_dot(p(2)-p(0))).abs()*0.5
                    }).sum();
                    let expected = footprint.faces().keys().map(|&f| footprint.face_area(f).unwrap_or_default()).sum::<f64>();
                    if triangles.is_empty() || (area-expected).abs() > TOLERANCE*expected.max(1.) { return Err(IoError::Invalid(format!("{location}: triangulation coverage mismatch; import stopped"))); }
                    for triangle in triangles.chunks_exact(3) {
                        let triangle = BulkFace { outer: triangle.iter().map(|&i| indices[i]).collect(), holes: vec![], ..face.clone() };
                        if !valid(&source, &triangle) { return Err(IoError::Invalid(format!("{location}: degenerate triangulation; import stopped"))); }
                        source.faces.push(triangle);
                    }
                    actions.push(format!("triangulating into {} planar triangles (max deviation {:.6} mm exceeds 0.0254 mm)", source.faces.len()-start, deviation*1000.));
                }
            }
            scene.warnings.push(format!("1 face repaired by {}; {location}; rejected by {original_error:?}", actions.join(" and ")));
        }
        for i in start..source.faces.len() { if let Some(&assignment) = original_assignments.get(&index) { assignments.insert(i, assignment); } }
    }
    // Validate combined welding and source edge styles too. Nondegenerate
    // failures stop the import; recovery never discards an unexplained face.
    let mut meters = source.clone();
    for p in &mut meters.vertices { *p *= 0.0254; }
    meters.build_cancellable(false, false, || control.is_cancelled()).map_err(|e| IoError::Invalid(format!("SKP combined repair failed: {e}")))?;
    Ok(source)
}
