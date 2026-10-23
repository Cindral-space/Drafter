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
            let p = geometry
                .vertices
                .get(*index)
                .ok_or_else(|| IoError::Invalid("face references missing vertex".into()))?;
            *index = local.vertices.len();
            local.vertices.push(*p * 0.0254);
        }
    }
    local.faces.push(face);
    Ok(local)
}
fn valid(geometry: &BulkGeometry, face: &BulkFace) -> bool {
    sample(geometry, face)
        .and_then(|g| g.build(false, false).map_err(Into::into))
        .is_ok()
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
            if cycle.len() >= 3 {
                out.push(cycle);
            }
            for old in path.drain(start + 1..) {
                positions.remove(&old);
            }
        } else {
            positions.insert(v, path.len());
            path.push(v);
        }
    }
    out
}
fn signed_area(ring: &[usize], vertices: &[DVec3], center: DVec3, normal: DVec3) -> f64 {
    ring.iter()
        .zip(ring.iter().cycle().skip(1))
        .take(ring.len())
        .map(|(&a, &b)| {
            (vertices[a] * 0.0254 - center)
                .cross(vertices[b] * 0.0254 - center)
                .dot(normal)
                * 0.5
        })
        .sum()
}
fn degenerate(ring: &[usize], vertices: &[DVec3]) -> bool {
    let Some(&first) = ring.first() else {
        return true;
    };
    let a = vertices[first] * 0.0254;
    let b = ring
        .iter()
        .map(|&i| vertices[i] * 0.0254)
        .max_by(|b, c| b.distance_squared(a).total_cmp(&c.distance_squared(a)))
        .unwrap_or(a);
    ring.iter()
        .all(|&i| (b - a).cross(vertices[i] * 0.0254 - a).length() <= TOLERANCE * TOLERANCE)
}

pub(crate) fn recover(
    source: BulkGeometry,
    labels: &[String],
    assignments: &mut Assignments,
    scene: &mut ImportedScene,
    control: &ImportControl,
) -> Result<BulkGeometry> {
    recover_impl(source, labels, assignments, scene, control, true)
}
fn recover_impl(
    mut source: BulkGeometry,
    labels: &[String],
    assignments: &mut Assignments,
    scene: &mut ImportedScene,
    control: &ImportControl,
    allow_projection: bool,
) -> Result<BulkGeometry> {
    let mut output_labels = Vec::new();
    let original_assignments = std::mem::take(assignments);
    let faces = std::mem::take(&mut source.faces);
    // Match the kernel's global insertion-order tolerance welding, including
    // orphan vertices, so repairs see exactly the same ring identities.
    let mut cells: HashMap<[i64; 3], Vec<usize>> = HashMap::new();
    let mut canonical = Vec::with_capacity(source.vertices.len());
    for (i, p) in source.vertices.iter().enumerate() {
        if i % 1024 == 0 {
            control.check()?;
        }
        let p = *p * 0.0254;
        if !p.is_finite() || p.abs().max_element() > cindral_core::MAX_COORDINATE {
            return Err(IoError::Invalid(format!("SKP vertex {i}: invalid/out-of-range position; import stopped without dropping geometry")));
        }
        let c = (p / TOLERANCE).floor().as_i64vec3().to_array();
        let mut found = None;
        for x in -1..=1 {
            for y in -1..=1 {
                for z in -1..=1 {
                    if let Some(bucket) = cells.get(&[c[0] + x, c[1] + y, c[2] + z]) {
                        for &j in bucket {
                            if (source.vertices[j] * 0.0254).distance(p) <= TOLERANCE {
                                found = Some(j);
                                break;
                            }
                        }
                    }
                }
            }
        }
        let representative = found.unwrap_or(i);
        canonical.push(representative);
        if found.is_none() {
            cells.entry(c).or_default().push(i);
        }
    }
    let mut work = 0usize;
    for (index, mut face) in faces.into_iter().enumerate() {
        control.check()?;
        let label = labels
            .get(index)
            .cloned()
            .unwrap_or_else(|| format!("face {index}"));
        let points: Vec<_> = std::iter::once(&face.outer)
            .chain(&face.holes)
            .flatten()
            .map(|&i| {
                source
                    .vertices
                    .get(i)
                    .copied()
                    .map(|p| p * 0.0254)
                    .ok_or_else(|| IoError::Invalid(format!("{label}: missing vertex")))
            })
            .collect::<Result<_>>()?;
        let center = points.iter().copied().sum::<DVec3>() / points.len().max(1) as f64;
        let location = format!(
            "{label}, local position ({:.6}, {:.6}, {:.6}) m",
            center.x, center.y, center.z
        );
        let size = points.len();
        work = work.saturating_add(size.saturating_mul(size));
        if size > 4096 || work > 64_000_000 {
            return Err(IoError::Invalid(format!(
                "{location}: repair work limit; import stopped without dropping geometry"
            )));
        }
        let start = source.faces.len();
        let mut actions = Vec::new();
        let original_error = sample(&source, &face)?.build(false, false).err();
        let rejection = original_error
            .as_ref()
            .map(ToString::to_string)
            .unwrap_or_default();
        if original_error.is_none() {
            source.faces.push(face);
        } else {
            for ring in std::iter::once(&mut face.outer).chain(&mut face.holes) {
                for i in ring {
                    *i = canonical[*i];
                }
            }
            if degenerate(&face.outer, &source.vertices) {
                scene.skipped += 1;
                scene.warnings.push(format!("1 face skipped: {location}; degenerate outer loop (zero area or fewer than 3 distinct vertices)"));
                continue;
            }
            let (center, normal, deviation, _) = crate::skp_plane::fit(&points);
            let old_outer = face.outer.clone();
            let mut rings = cycles(&face.outer);
            if rings.is_empty() {
                return Err(IoError::Invalid(format!(
                    "{location}: nondegenerate ring could not be decomposed"
                )));
            }
            let largest = (0..rings.len())
                .max_by(|&a, &b| {
                    signed_area(&rings[a], &source.vertices, center, normal)
                        .abs()
                        .total_cmp(&signed_area(&rings[b], &source.vertices, center, normal).abs())
                })
                .unwrap();
            face.outer = rings.remove(largest);
            let old_holes = std::mem::take(&mut face.holes);
            for hole in &old_holes {
                rings.extend(cycles(hole));
            }
            face.holes = rings;
            if face.outer != old_outer || face.holes != old_holes {
                actions
                    .push("decomposing retraced/welded loops into simple boundaries".to_string());
            }
            if valid(&source, &face) {
                source.faces.push(face);
            } else {
                // Validate the projected footprint, including holes, before using
                // earcut. Invalid nondegenerate topology is an explicit error,
                // never silently dropped or filled with a fan.
                let mut projected = sample(&source, &face)?;
                for p in &mut projected.vertices {
                    *p -= normal * normal.dot(*p - center);
                }
                let footprint = projected.clone().build(false, false).map_err(|e| IoError::Invalid(format!("{location}: projected footprint invalid ({e}); original check {rejection}; import stopped without dropping geometry")))?;
                if allow_projection && deviation <= PROJECTION_TOLERANCE {
                    // Move shared source vertices and their explicit edges together.
                    // If this invalidates a previously accepted neighboring polygon,
                    // the final pass triangulates that neighbor without moving it.
                    for &i in std::iter::once(&face.outer).chain(&face.holes).flatten() {
                        let p = source.vertices[i] * 0.0254;
                        source.vertices[i] = (p - normal * normal.dot(p - center)) / 0.0254;
                    }
                    source.faces.push(face);
                    actions.push(format!("projecting to its best-fit plane (max deviation {:.6} mm, tolerance 0.0254 mm)", deviation*1000.));
                } else {
                    let axis = if normal.x.abs() < 0.9 {
                        DVec3::X
                    } else {
                        DVec3::Y
                    };
                    let u = normal.cross(axis).normalize();
                    let v = normal.cross(u);
                    let mut coordinates = Vec::new();
                    let mut indices = Vec::new();
                    let mut holes = Vec::new();
                    for (r, ring) in std::iter::once(&face.outer).chain(&face.holes).enumerate() {
                        if r > 0 {
                            holes.push(indices.len());
                        }
                        for &i in ring {
                            let p = source.vertices[i] * 0.0254 - center;
                            coordinates.extend([p.dot(u), p.dot(v)]);
                            indices.push(i);
                        }
                    }
                    let triangles = earcutr::earcut(&coordinates, &holes, 2).map_err(|e| {
                        IoError::Invalid(format!("{location}: triangulation failed: {e}"))
                    })?;
                    let area: f64 = triangles
                        .as_chunks::<3>()
                        .0
                        .iter()
                        .map(|t| {
                            let p = |i: usize| {
                                glam::DVec2::new(coordinates[2 * t[i]], coordinates[2 * t[i] + 1])
                            };
                            ((p(1) - p(0)).perp_dot(p(2) - p(0))).abs() * 0.5
                        })
                        .sum();
                    let expected = footprint
                        .faces()
                        .keys()
                        .map(|&f| footprint.face_area(f).unwrap_or_default())
                        .sum::<f64>();
                    if triangles.is_empty()
                        || (area - expected).abs() > TOLERANCE * expected.max(1.)
                    {
                        return Err(IoError::Invalid(format!(
                            "{location}: triangulation coverage mismatch; import stopped"
                        )));
                    }
                    for triangle in triangles.as_chunks::<3>().0 {
                        let triangle = BulkFace {
                            outer: triangle.iter().map(|&i| indices[i]).collect(),
                            holes: vec![],
                            ..face.clone()
                        };
                        if !valid(&source, &triangle) {
                            return Err(IoError::Invalid(format!(
                                "{location}: degenerate triangulation; import stopped"
                            )));
                        }
                        source.faces.push(triangle);
                    }
                    actions.push(format!("triangulating into {} planar triangles (max deviation {:.6} mm; projection tolerance 0.0254 mm or shared-boundary constraint)", source.faces.len()-start, deviation*1000.));
                }
            }
            scene.warnings.push(format!(
                "1 face repaired by {}; {location}; rejected by {rejection}",
                actions.join(" and ")
            ));
        }
        for i in start..source.faces.len() {
            output_labels.push(label.clone());
            if let Some(&assignment) = original_assignments.get(&index) {
                assignments.insert(i, assignment);
            }
        }
    }
    // Validate combined welding and source edge styles too. Nondegenerate
    // failures stop the import; recovery never discards an unexplained face.
    let mut meters = source.clone();
    for p in &mut meters.vertices {
        *p *= 0.0254;
    }
    if let Err(error) = meters.build_cancellable(false, false, || control.is_cancelled()) {
        control.check()?;
        if allow_projection {
            return recover_impl(source, &output_labels, assignments, scene, control, false);
        }
        return Err(IoError::Invalid(format!(
            "SKP combined repair failed: {error}"
        )));
    }
    Ok(source)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{convert_scene, ImportOptions, ImportedDefinition, ImportedInstance};
    use cindral_core::{BulkEdge, MaterialId};

    fn polygon(points: &[[f64; 3]], outer: Vec<usize>, holes: Vec<Vec<usize>>) -> BulkGeometry {
        let mut geometry = BulkGeometry {
            vertices: points
                .iter()
                .map(|p| DVec3::from_array(*p) / 0.0254)
                .collect(),
            faces: vec![BulkFace {
                outer,
                holes,
                material: Some(MaterialId(7)),
                color: [1, 2, 3, 255],
            }],
            edges: vec![],
        };
        for ring in std::iter::once(&geometry.faces[0].outer).chain(&geometry.faces[0].holes) {
            for (&a, &b) in ring
                .iter()
                .zip(ring.iter().cycle().skip(1))
                .take(ring.len())
            {
                geometry.edges.push(BulkEdge {
                    vertices: [a, b],
                    soft: true,
                    smooth: true,
                    hidden: true,
                });
            }
        }
        geometry
    }
    fn repair(g: BulkGeometry) -> (BulkGeometry, ImportedScene, Assignments) {
        let mut report = ImportedScene::default();
        let mut assignments = BTreeMap::from([(0, (Some(2), true))]);
        let result = recover(
            g,
            &["entity 123, definition Test, parent instance Example, tag Detail".into()],
            &mut assignments,
            &mut report,
            &ImportControl::default(),
        )
        .unwrap();
        (result, report, assignments)
    }
    fn model(mut g: BulkGeometry) -> cindral_core::Model {
        for p in &mut g.vertices {
            *p *= 0.0254;
        }
        g.build(false, false).unwrap()
    }
    fn square(z: f64) -> BulkGeometry {
        polygon(
            &[[0., 0., 0.], [2., 0., 0.], [2., 2., z], [0., 2., 0.]],
            vec![0, 1, 2, 3],
            vec![],
        )
    }
    #[test]
    fn slightly_off_plane_projects_and_moves_source_edges_together() {
        let (fixed, report, assignments) = repair(square(0.00002));
        assert_eq!(fixed.vertices.len(), 4);
        assert_eq!(fixed.edges.len(), 4);
        assert_eq!(fixed.faces.len(), 1);
        assert_eq!(assignments[&0], (Some(2), true));
        assert_eq!(fixed.faces[0].material, Some(MaterialId(7)));
        assert_eq!(fixed.faces[0].color, [1, 2, 3, 255]);
        let m = model(fixed);
        assert_eq!(m.edges().len(), 4);
        assert!(m.edges().values().all(|e| e.soft && e.smooth && e.hidden));
        assert_eq!(report.skipped, 0);
        assert!(report.warnings[0].contains("projecting to its best-fit plane"));
        assert!(report.warnings[0].contains("entity 123"));
        assert!(report.warnings[0].contains("local position"));
    }
    #[test]
    fn badly_nonplanar_triangulates_preserving_material_and_boundaries() {
        let original = square(0.2);
        let (fixed, report, assignments) = repair(original.clone());
        assert_eq!(fixed.vertices, original.vertices);
        assert_eq!(fixed.faces.len(), 2);
        assert_eq!(assignments.len(), 2);
        assert!(assignments.values().all(|&v| v == (Some(2), true)));
        assert!(fixed
            .faces
            .iter()
            .all(|f| f.material == Some(MaterialId(7)) && f.color == [1, 2, 3, 255]));
        let m = model(fixed);
        assert_eq!(m.edges().len(), 5);
        assert_eq!(
            m.edges()
                .values()
                .filter(|e| e.hidden && e.soft && e.smooth)
                .count(),
            4
        );
        assert!(report.warnings[0].contains("2 planar triangles"));
        assert_eq!(report.skipped, 0);
    }
    #[test]
    fn degenerate_is_skipped_and_identified() {
        let g = polygon(
            &[[0., 0., 0.], [1., 0., 0.], [2., 0., 0.]],
            vec![0, 1, 2],
            vec![],
        );
        let (fixed, report, assignments) = repair(g);
        assert!(fixed.faces.is_empty());
        assert!(assignments.is_empty());
        assert_eq!(model(fixed).edges().len(), 3);
        assert_eq!(report.skipped, 1);
        assert!(report.warnings[0].contains("entity 123"));
        assert!(report.warnings[0].contains("zero area"));
    }
    fn holed(z: f64, bridge: bool) -> BulkGeometry {
        polygon(
            &[
                [0., 0., 0.],
                [4., 0., 0.],
                [4., 4., z],
                [0., 4., 0.],
                [1., 1., 0.],
                [1., 3., 0.],
                [3., 3., 0.],
                [3., 1., 0.],
            ],
            if bridge {
                vec![0, 1, 2, 3, 0, 4, 5, 6, 7, 4]
            } else {
                vec![0, 1, 2, 3]
            },
            if bridge {
                vec![]
            } else {
                vec![vec![4, 5, 6, 7]]
            },
        )
    }
    #[test]
    fn retraced_bridge_restores_surface_and_preserves_authored_hole() {
        let (fixed, report, _) = repair(holed(0., true));
        assert_eq!(fixed.faces.len(), 1);
        assert_eq!(fixed.faces[0].holes.len(), 1);
        let m = model(fixed);
        let id = *m.faces().keys().next().unwrap();
        assert!((m.face_area(id).unwrap() - 12.).abs() < 1e-10);
        assert_eq!(m.edges().len(), 9); // eight boundary edges plus retained bridge
        assert!(report.warnings[0].contains("retraced/welded"));
        assert_eq!(report.skipped, 0);
    }
    #[test]
    fn hole_survives_projection_and_nonplanar_triangulation() {
        for z in [0.00002, 0.2] {
            let (fixed, report, _) = repair(holed(z, false));
            let m = model(fixed);
            if z < 0.001 {
                assert_eq!(m.faces().values().next().unwrap().holes.len(), 1);
            }
            // The original hole is empty in the XY footprint after both repairs.
            for &id in m.faces().keys() {
                for t in m.triangulate(id).unwrap() {
                    let c = t.into_iter().sum::<DVec3>() / 3.;
                    assert!(!(c.x > 1.0001 && c.x < 2.9999 && c.y > 1.0001 && c.y < 2.9999));
                }
            }
            assert_eq!(report.skipped, 0);
        }
    }
    #[test]
    fn invalid_nondegenerate_hole_fails_instead_of_disappearing() {
        let mut g = holed(0., false);
        for p in &mut g.vertices[4..] {
            p.x += 10. / 0.0254;
        }
        let result = recover(
            g,
            &["entity 456, Test".into()],
            &mut Assignments::new(),
            &mut ImportedScene::default(),
            &ImportControl::default(),
        );
        assert!(result.unwrap_err().to_string().contains("entity 456"));
    }
    #[test]
    fn summary_matches_model_with_shared_definition_instances() {
        let (mut g, _, _) = repair(square(0.2));
        for face in &mut g.faces {
            face.material = None;
        }
        let scene = ImportedScene {
            meters_per_unit: 0.0254,
            definitions: vec![ImportedDefinition {
                name: "Shared".into(),
                geometry: g,
                ..Default::default()
            }],
            roots: vec![ImportedInstance::default(), ImportedInstance::default()],
            ..Default::default()
        };
        let result = convert_scene(
            scene,
            &ImportOptions {
                merge_coplanar: false,
                soften_coplanar: false,
                as_component: true,
            },
            &ImportControl::default(),
        )
        .unwrap();
        let data = result.history.to_data();
        assert_eq!(
            result.report.vertices,
            data.definitions
                .values()
                .map(|d| d.geometry.vertices.len())
                .sum::<usize>()
        );
        assert_eq!(
            result.report.edges,
            data.definitions
                .values()
                .map(|d| d.geometry.edges.len())
                .sum::<usize>()
        );
        assert_eq!(
            result.report.faces,
            data.definitions
                .values()
                .map(|d| d.geometry.faces.len())
                .sum::<usize>()
        );
        assert_eq!(
            result.report.instances,
            data.definitions
                .values()
                .map(|d| d.instances.len())
                .sum::<usize>()
        );
        assert_eq!(
            (
                result.report.vertices,
                result.report.edges,
                result.report.faces,
                result.report.instances
            ),
            (4, 5, 2, 3)
        );
    }
    #[test]
    fn least_squares_fit_is_translation_and_rotation_invariant() {
        let rotation = glam::DQuat::from_rotation_x(0.7) * glam::DQuat::from_rotation_z(1.2);
        let points: Vec<_> = [[0., 0., 0.], [1., 0., 0.], [1., 1., 0.], [0., 1., 0.]]
            .map(|p| rotation * DVec3::from_array(p) + DVec3::splat(1234.))
            .into_iter()
            .collect();
        let (_, normal, deviation, _) = crate::skp_plane::fit(&points);
        assert!(normal.dot(rotation * DVec3::Z).abs() > 1. - 1e-12);
        assert!(deviation < 1e-10);
    }
    #[test]
    fn projection_keeps_shared_boundaries_and_triangulates_affected_neighbor() {
        let mut g = square(0.00002);
        g.vertices.extend([
            DVec3::new(3., 0., 0.) / 0.0254,
            DVec3::new(3., 2., 0.00002) / 0.0254,
        ]);
        g.faces.push(BulkFace {
            outer: vec![1, 4, 5, 2],
            ..g.faces[0].clone()
        });
        let mut assignments = Assignments::new();
        let mut report = ImportedScene::default();
        let fixed = recover(
            g,
            &["entity 1".into(), "entity 2".into()],
            &mut assignments,
            &mut report,
            &ImportControl::default(),
        )
        .unwrap();
        assert_eq!(fixed.vertices.len(), 6);
        assert_eq!(model(fixed).faces().len(), 3);
        assert!(report
            .warnings
            .iter()
            .any(|s| s.contains("entity 1") && s.contains("projecting")));
        assert!(report.warnings.iter().any(|s| s.contains("triangulating")));
        assert_eq!(report.skipped, 0);
    }
    #[test]
    fn tolerance_welded_consecutive_loop_vertex_is_reported_and_repaired() {
        let g = polygon(
            &[
                [0., 0., 0.],
                [0.00000005, 0., 0.],
                [1., 0., 0.],
                [1., 1., 0.],
                [0., 1., 0.],
            ],
            vec![0, 1, 2, 3, 4],
            vec![],
        );
        let (fixed, report, _) = repair(g);
        let m = model(fixed);
        assert_eq!(m.vertices().len(), 4);
        assert_eq!(m.faces().values().next().unwrap().outer.len(), 4);
        assert_eq!(report.skipped, 0);
        assert!(report.warnings[0].contains("welded"));
    }
    #[test]
    fn private_face_44340_has_full_surface_coverage_when_present() {
        let path = std::path::Path::new("/home/earl/Desktop/Cindral/Drafter/SKP/mybedroom.skp");
        if !path.exists() {
            eprintln!("SKIP: private SKP absent");
            return;
        }
        let bytes = std::fs::read(path).unwrap();
        let raw = cindral_skp::read(&bytes).unwrap();
        let def = &raw.definitions[0];
        let index = *def.face_ids.iter().find(|(_, &id)| id == 44340).unwrap().0;
        let original = &def.geometry.faces[index];
        let repaired = crate::import_skp_bytes(&bytes, &ImportControl::default()).unwrap();
        let fixed = &repaired.definitions[0].geometry;
        assert_eq!(fixed.vertices, def.geometry.vertices);
        assert_eq!(fixed.edges.len(), def.geometry.edges.len());
        for (a, b) in fixed.edges.iter().zip(&def.geometry.edges) {
            assert_eq!(
                (a.vertices, a.soft, a.smooth, a.hidden),
                (b.vertices, b.soft, b.smooth, b.hidden)
            );
        }
        assert_eq!(fixed.faces.len(), def.geometry.faces.len());
        let face = &fixed.faces[index];
        assert_eq!(face.material, original.material);
        assert_eq!(face.color, original.color);
        let m = model(BulkGeometry {
            vertices: fixed.vertices.clone(),
            faces: vec![face.clone()],
            edges: vec![],
        });
        let id = *m.faces().keys().next().unwrap();
        let triangles = m.triangulate(id).unwrap();
        let area: f64 = triangles
            .iter()
            .map(|[a, b, c]| (*b - *a).cross(*c - *a).length() * 0.5)
            .sum();
        assert!(
            (area - 0.0011646567495071517).abs() < 1e-12,
            "restored surface area {area}"
        );
        assert!(
            !triangles.is_empty(),
            "face must render and pick as a surface"
        );
        let result = crate::import_file(
            path,
            &ImportOptions {
                as_component: false,
                ..Default::default()
            },
            &ImportControl::default(),
        )
        .unwrap();
        assert_eq!(
            (
                result.report.vertices,
                result.report.edges,
                result.report.faces,
                result.report.instances,
                result.report.skipped
            ),
            (1760, 2021, 305, 1, 0)
        );
        assert_eq!(result.report.warnings.len(), 1);
        assert!(result.report.warnings[0].contains("44340"));
        assert!(!result.report.not_imported.is_empty());
        assert!(result.report.notes.iter().any(|s| s.contains("editable")));
        eprintln!(
            "OPEN BEFORE 1760/2021/304/1 skipped=1; AFTER {:?}",
            result.report
        );
    }
}
