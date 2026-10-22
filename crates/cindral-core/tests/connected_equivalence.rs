#![cfg(feature = "bench-fixtures")]

//! Equivalence and regression test scaffolding for connected-solid optimization.
//!
//! Compares the optimized `History::execute` / `History::preview` path against the
//! frozen reference algorithm in `cindral_core::benchmark::reference_execute`.
//!
//! Subdivided connected cube solids (n = 1..=3) are verified for:
//! - Exact vertex/edge identities on translation, equivalent planes/surfaces and exact undo.
//! - Geometric boundary equivalence on outward and pocket extrusions:
//!   closed-edge incidence, volume, total face area, and symmetric triangle sample coverage
//!   (without demanding identical tessellation due to coplanar seam differences).
//! - Incremental preview cancellation and single-commit undo/redo cycles.

use std::collections::BTreeMap;

use glam::{DMat4, DVec3};

use cindral_core::benchmark::reference_execute;
use cindral_core::connected_fixture::connected_solid;
use cindral_core::{Command, FaceId, History, Model, VertexId, TOLERANCE};

// ---------------------------------------------------------------------------
// Geometry and boundary verification helpers
// ---------------------------------------------------------------------------

/// Squared distance from point `p` to 3D segment `[a, b]`.
fn point_to_segment_sq_dist(p: DVec3, a: DVec3, b: DVec3) -> f64 {
    let ab = b - a;
    let len_sq = ab.length_squared();
    if len_sq <= TOLERANCE * TOLERANCE {
        return (p - a).length_squared();
    }
    let t = ((p - a).dot(ab) / len_sq).clamp(0.0, 1.0);
    (a + ab * t - p).length_squared()
}

/// Squared distance from point `p` to 3D triangle `[a, b, c]`.
fn point_to_triangle_sq_dist(p: DVec3, a: DVec3, b: DVec3, c: DVec3) -> f64 {
    let ab = b - a;
    let ac = c - a;
    let ap = p - a;

    let d1 = ab.dot(ap);
    let d2 = ac.dot(ap);
    if d1 <= 0.0 && d2 <= 0.0 {
        return ap.length_squared();
    }

    let bp = p - b;
    let d3 = ab.dot(bp);
    let d4 = ac.dot(bp);
    if d3 >= 0.0 && d4 <= d3 {
        return bp.length_squared();
    }

    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        let v = d1 / (d1 - d3);
        return (a + ab * v - p).length_squared();
    }

    let cp = p - c;
    let d5 = ab.dot(cp);
    let d6 = ac.dot(cp);
    if d6 >= 0.0 && d5 <= d6 {
        return cp.length_squared();
    }

    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        let w = d2 / (d2 - d6);
        return (a + ac * w - p).length_squared();
    }

    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 {
        let w = (d4 - d3) / ((d4 - d3) + (d5 - d6));
        return (b + (c - b) * w - p).length_squared();
    }

    let denom = va + vb + vc;
    if denom <= TOLERANCE * TOLERANCE {
        return point_to_segment_sq_dist(p, a, b)
            .min(point_to_segment_sq_dist(p, b, c))
            .min(point_to_segment_sq_dist(p, c, a));
    }
    let v = vb / denom;
    let w = vc / denom;
    (a + ab * v + ac * w - p).length_squared()
}

/// Collect all triangles from all triangulated faces in the model.
fn model_triangles(model: &Model) -> Vec<[DVec3; 3]> {
    let mut triangles = Vec::new();
    for &face_id in model.faces().keys() {
        triangles.extend(
            model
                .triangulate(face_id)
                .expect("triangulation must succeed"),
        );
    }
    triangles
}

/// Assert that the model is a valid, closed 2-manifold where every undirected edge
/// has an incidence count of exactly 2 across outer boundaries and holes.
fn assert_closed_manifold(model: &Model) {
    model.validate().expect("model validation must pass");
    let mut edge_usage = BTreeMap::new();
    for face in model.faces().values() {
        for ring in std::iter::once(&face.outer).chain(&face.holes) {
            for (a, b) in ring.iter().zip(ring.iter().cycle().skip(1)) {
                let mut pair = [*a, *b];
                pair.sort();
                *edge_usage.entry(pair).or_insert(0usize) += 1;
            }
        }
    }
    for edge in model.edges().values() {
        let count = edge_usage.get(&edge.vertices).copied().unwrap_or(0);
        assert_eq!(
            count, 2,
            "edge {:?} has incidence {} instead of 2",
            edge.vertices, count
        );
    }
    assert_eq!(edge_usage.len(), model.edges().len());
}

/// Compute total surface area across all faces.
fn total_surface_area(model: &Model) -> f64 {
    model
        .faces()
        .keys()
        .map(|&id| model.face_area(id).expect("valid face area"))
        .sum()
}

/// Assert symmetric triangle sample coverage (Hausdorff-like boundary surface equivalence).
///
/// Every sample point on model A's triangulated surface must lie within `tol` of
/// model B's surface, and vice versa.
fn assert_symmetric_triangle_sample_coverage(model_a: &Model, model_b: &Model, tol: f64) {
    let tris_a = model_triangles(model_a);
    let tris_b = model_triangles(model_b);
    assert!(!tris_a.is_empty(), "model A has no triangles");
    assert!(!tris_b.is_empty(), "model B has no triangles");

    let min_dist_to_mesh = |point: DVec3, triangles: &[[DVec3; 3]]| -> f64 {
        triangles
            .iter()
            .map(|&[a, b, c]| point_to_triangle_sq_dist(point, a, b, c))
            .fold(f64::INFINITY, f64::min)
            .sqrt()
    };

    for &[a, b, c] in &tris_a {
        let samples = [
            a,
            b,
            c,
            (a + b + c) / 3.0,
            (a + b) * 0.5,
            (b + c) * 0.5,
            (c + a) * 0.5,
            a * 0.6 + b * 0.2 + c * 0.2,
        ];
        for p in samples {
            let dist = min_dist_to_mesh(p, &tris_b);
            assert!(
                dist <= tol,
                "sample point {:?} of model A is not on model B surface (dist={})",
                p,
                dist
            );
        }
    }

    for &[a, b, c] in &tris_b {
        let samples = [
            a,
            b,
            c,
            (a + b + c) / 3.0,
            (a + b) * 0.5,
            (b + c) * 0.5,
            (c + a) * 0.5,
            a * 0.6 + b * 0.2 + c * 0.2,
        ];
        for p in samples {
            let dist = min_dist_to_mesh(p, &tris_a);
            assert!(
                dist <= tol,
                "sample point {:?} of model B is not on model A surface (dist={})",
                p,
                dist
            );
        }
    }
}

/// Comprehensive geometric boundary equivalence check between optimized and reference models.
fn assert_boundary_equivalence(opt_model: &Model, ref_model: &Model) {
    // 1. Closed-edge incidence and 2-manifold validity
    assert_closed_manifold(opt_model);
    assert_closed_manifold(ref_model);

    // 2. Closed solid volume equivalence
    let vol_opt = opt_model
        .closed_solid_volume()
        .unwrap()
        .expect("opt model must have closed volume");
    let vol_ref = ref_model
        .closed_solid_volume()
        .unwrap()
        .expect("ref model must have closed volume");
    assert!(
        (vol_opt - vol_ref).abs() < TOLERANCE * 100.0,
        "volumes differ between opt ({vol_opt}) and ref ({vol_ref})"
    );

    // 3. Total surface area equivalence
    let area_opt = total_surface_area(opt_model);
    let area_ref = total_surface_area(ref_model);
    assert!(
        (area_opt - area_ref).abs() < TOLERANCE * 100.0,
        "surface areas differ between opt ({area_opt}) and ref ({area_ref})"
    );

    // 4. Symmetric triangle sample coverage (surface point membership)
    assert_symmetric_triangle_sample_coverage(opt_model, ref_model, TOLERANCE * 8.0);
}

/// Translation verification helper: exact model equality and undo/redo restoration.
fn verify_translation_equivalence(segments: usize, translation: DVec3) {
    let (mut history_opt, _) = connected_solid(segments).expect("connected_solid");
    let (mut history_ref, _) = connected_solid(segments).expect("connected_solid");

    let initial_model = history_opt.model().clone();
    assert_eq!(history_opt.model(), history_ref.model());

    let vertices: Vec<VertexId> = history_opt.model().vertices().keys().copied().collect();
    let matrix = DMat4::from_translation(translation);
    let command = Command::Transform {
        vertices,
        matrix,
        copy: false,
    };

    let opt_changed = history_opt.execute(command.clone()).expect("opt execute");
    let ref_changed = reference_execute(&mut history_ref, command).expect("ref execute");
    assert_eq!(opt_changed, ref_changed);

    // Exact model equality (IDs, planes, vertices, edges, faces)
    assert_eq!(
        history_opt.model().vertices(),
        history_ref.model().vertices()
    );
    assert_eq!(history_opt.model().edges(), history_ref.model().edges());
    assert_eq!(
        history_opt.model().faces().keys().collect::<Vec<_>>(),
        history_ref.model().faces().keys().collect::<Vec<_>>()
    );
    // The fixture stores outward planes; legacy update_plane canonicalizes them.
    // Compare geometric surfaces, while retaining exact vertex/edge/ID checks above.
    assert_boundary_equivalence(history_opt.model(), history_ref.model());

    assert_closed_manifold(history_opt.model());

    // Undo restoration
    history_opt.undo().expect("opt undo");
    history_ref.undo().expect("ref undo");
    assert_eq!(history_opt.model(), &initial_model);
    assert_eq!(history_ref.model(), &initial_model);

    // Redo restoration
    history_opt.redo().expect("opt redo");
    history_ref.redo().expect("ref redo");
    assert_boundary_equivalence(history_opt.model(), history_ref.model());
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[test]
fn test_subdivided_cube_translation_exact_equality_and_undo() {
    for n in 1..=3 {
        // Positive translation
        verify_translation_equivalence(n, DVec3::new(1.5, 2.0, 3.5));
        // Negative / mirrored translation
        verify_translation_equivalence(n, DVec3::new(-2.0, -1.0, -0.5));
    }
}

#[test]
fn test_subdivided_cube_extrusion_outward_and_pocket_boundary_equivalence() {
    for n in 1..=3 {
        // Outward extrusion
        let (mut history_opt, top_face_opt) = connected_solid(n).expect("connected_solid");
        let (mut history_ref, top_face_ref) = connected_solid(n).expect("connected_solid");
        let initial_model = history_opt.model().clone();

        let outward_distance = 0.2;
        let cmd_opt = Command::PushPull {
            face: top_face_opt,
            distance: outward_distance,
        };
        let cmd_ref = Command::PushPull {
            face: top_face_ref,
            distance: outward_distance,
        };

        history_opt.execute(cmd_opt).expect("opt outward execute");
        reference_execute(&mut history_ref, cmd_ref).expect("ref outward execute");

        assert_boundary_equivalence(history_opt.model(), history_ref.model());

        history_opt.undo().expect("opt undo");
        history_ref.undo().expect("ref undo");
        assert_eq!(history_opt.model(), &initial_model);
        assert_eq!(history_ref.model(), &initial_model);

        history_opt.redo().expect("opt redo");
        history_ref.redo().expect("ref redo");
        assert_boundary_equivalence(history_opt.model(), history_ref.model());

        // Inward pocket extrusion (below full depth of 2.0)
        let (mut history_opt, top_face_opt) = connected_solid(n).expect("connected_solid");
        let (mut history_ref, top_face_ref) = connected_solid(n).expect("connected_solid");
        let initial_model = history_opt.model().clone();

        let pocket_distance = -0.3;
        let cmd_opt = Command::PushPull {
            face: top_face_opt,
            distance: pocket_distance,
        };
        let cmd_ref = Command::PushPull {
            face: top_face_ref,
            distance: pocket_distance,
        };

        history_opt.execute(cmd_opt).expect("opt pocket execute");
        reference_execute(&mut history_ref, cmd_ref).expect("ref pocket execute");

        assert_boundary_equivalence(history_opt.model(), history_ref.model());

        history_opt.undo().expect("opt undo");
        history_ref.undo().expect("ref undo");
        assert_eq!(history_opt.model(), &initial_model);
        assert_eq!(history_ref.model(), &initial_model);
    }
}

#[test]
fn test_preview_cancel_and_commit_undo_redo() {
    for n in 1..=3 {
        let (mut history_opt, top_face_opt) = connected_solid(n).expect("connected_solid");
        let (mut history_ref, top_face_ref) = connected_solid(n).expect("connected_solid");

        let initial_model = history_opt.model().clone();
        let depth_opt = history_opt.undo_depth();
        let depth_ref = history_ref.undo_depth();

        // 1. Incremental previews with changing distances
        for &distance in &[0.05, 0.15, 0.25] {
            let cmd = Command::PushPull {
                face: top_face_opt,
                distance,
            };
            history_opt.preview(cmd).expect("opt preview");
            assert_eq!(history_opt.undo_depth(), depth_opt);
            assert_closed_manifold(history_opt.model());
        }

        // 2. Cancel preview restores initial model exactly
        history_opt.cancel_preview().expect("opt cancel_preview");
        assert_eq!(history_opt.model(), &initial_model);
        assert_eq!(history_opt.undo_depth(), depth_opt);

        // 3. Incremental preview sequence followed by single commit
        for &distance in &[0.1, 0.2] {
            let cmd = Command::PushPull {
                face: top_face_opt,
                distance,
            };
            history_opt.preview(cmd).expect("opt preview");
            assert_eq!(history_opt.undo_depth(), depth_opt);
        }
        history_opt.commit_preview().expect("opt commit_preview");
        assert_eq!(history_opt.undo_depth(), depth_opt + 1);

        // Reference execution of the committed edit
        let ref_cmd = Command::PushPull {
            face: top_face_ref,
            distance: 0.2,
        };
        reference_execute(&mut history_ref, ref_cmd).expect("ref execute");
        assert_eq!(history_ref.undo_depth(), depth_ref + 1);

        // Geometric boundary equivalence
        assert_boundary_equivalence(history_opt.model(), history_ref.model());

        // 4. Single undo restores initial model
        history_opt.undo().expect("opt undo");
        history_ref.undo().expect("ref undo");
        assert_eq!(history_opt.model(), &initial_model);
        assert_eq!(history_ref.model(), &initial_model);
        assert_eq!(history_opt.undo_depth(), depth_opt);

        // 5. Redo restores the committed state
        history_opt.redo().expect("opt redo");
        history_ref.redo().expect("ref redo");
        assert_eq!(history_opt.undo_depth(), depth_opt + 1);
        assert_boundary_equivalence(history_opt.model(), history_ref.model());
    }
}

#[test]
fn test_face_with_hole_extrusion_equivalence() {
    fn create_annulus_history() -> (History, FaceId) {
        let mut h = History::new();
        h.execute(Command::AddPolyline {
            points: vec![
                DVec3::new(0.0, 0.0, 0.0),
                DVec3::new(4.0, 0.0, 0.0),
                DVec3::new(4.0, 4.0, 0.0),
                DVec3::new(0.0, 4.0, 0.0),
            ],
            closed: true,
        })
        .expect("add outer polyline");
        h.execute(Command::AddPolyline {
            points: vec![
                DVec3::new(1.0, 1.0, 0.0),
                DVec3::new(3.0, 1.0, 0.0),
                DVec3::new(3.0, 3.0, 0.0),
                DVec3::new(1.0, 3.0, 0.0),
            ],
            closed: true,
        })
        .expect("add inner polyline");
        let annulus_id = *h
            .model()
            .faces()
            .iter()
            .find(|(_, f)| !f.holes.is_empty())
            .expect("annulus face")
            .0;
        (h, annulus_id)
    }

    let (mut h_opt, face_opt) = create_annulus_history();
    let (mut h_ref, face_ref) = create_annulus_history();

    let cmd_opt = Command::PushPull {
        face: face_opt,
        distance: 1.0,
    };
    let cmd_ref = Command::PushPull {
        face: face_ref,
        distance: 1.0,
    };

    let opt_res = h_opt.execute(cmd_opt);
    let ref_res = reference_execute(&mut h_ref, cmd_ref);
    assert_eq!(opt_res.is_ok(), ref_res.is_ok());

    h_opt.model().validate().expect("opt validate");
    h_ref.model().validate().expect("ref validate");

    let area_opt = total_surface_area(h_opt.model());
    let area_ref = total_surface_area(h_ref.model());
    assert!(
        (area_opt - area_ref).abs() < TOLERANCE * 100.0,
        "annulus extrusion surface areas differ: opt={area_opt}, ref={area_ref}"
    );

    assert_symmetric_triangle_sample_coverage(h_opt.model(), h_ref.model(), TOLERANCE * 8.0);

    h_opt.undo().expect("opt undo");
    h_ref.undo().expect("ref undo");
    assert_eq!(h_opt.model(), h_ref.model());

    h_opt.redo().expect("opt redo");
    h_ref.redo().expect("ref redo");
    assert_symmetric_triangle_sample_coverage(h_opt.model(), h_ref.model(), TOLERANCE * 8.0);
}

// ---------------------------------------------------------------------------
// Proptests: bounded runtime (cases = 16 or 32, n <= 3)
// ---------------------------------------------------------------------------

proptest::proptest! {
    #![proptest_config(proptest::test_runner::Config {
        cases: 16,
        failure_persistence: None,
        ..proptest::test_runner::Config::default()
    })]

    #[test]
    fn prop_connected_solid_translation_equivalence(
        n in 1usize..=3,
        dx in -4.0f64..4.0,
        dy in -4.0f64..4.0,
        dz in -4.0f64..4.0,
    ) {
        verify_translation_equivalence(n, DVec3::new(dx, dy, dz));
    }

    #[test]
    fn prop_connected_solid_extrusion_equivalence(
        n in 1usize..=3,
        distance in proptest::prop_oneof![
            0.05f64..1.0,
            -1.0f64..-0.05,
        ],
    ) {
        let (mut history_opt, top_face_opt) = connected_solid(n).expect("connected_solid");
        let (mut history_ref, top_face_ref) = connected_solid(n).expect("connected_solid");

        let cmd_opt = Command::PushPull {
            face: top_face_opt,
            distance,
        };
        let cmd_ref = Command::PushPull {
            face: top_face_ref,
            distance,
        };

        let opt_changed = history_opt.execute(cmd_opt).expect("opt execute");
        let ref_changed = reference_execute(&mut history_ref, cmd_ref).expect("ref execute");
        proptest::prop_assert_eq!(opt_changed, ref_changed);

        assert_boundary_equivalence(history_opt.model(), history_ref.model());

        let initial_opt = connected_solid(n).expect("connected_solid").0;
        history_opt.undo().expect("opt undo");
        history_ref.undo().expect("ref undo");
        proptest::prop_assert_eq!(history_opt.model(), initial_opt.model());
        proptest::prop_assert_eq!(history_ref.model(), initial_opt.model());

        history_opt.redo().expect("opt redo");
        history_ref.redo().expect("ref redo");
        assert_boundary_equivalence(history_opt.model(), history_ref.model());
    }
}

#[test]
fn interior_pocket_fast_path_matches_reference_and_translation_index_supports_later_edits() {
    let (mut fast, _) = connected_solid(3).unwrap();
    let mut reference = fast.clone();
    let face = *fast
        .model()
        .faces()
        .iter()
        .find(|(_, f)| {
            f.plane.normal.z > 0.9
                && (f.plane.distance - 2.).abs() < TOLERANCE
                && f.outer.iter().all(|id| {
                    let p = fast.model().vertices()[id].position;
                    p.x > 0.1 && p.x < 1.9 && p.y > 0.1 && p.y < 1.9
                })
        })
        .unwrap()
        .0;
    fast.execute(Command::PushPull {
        face,
        distance: -0.2,
    })
    .unwrap();
    reference_execute(
        &mut reference,
        Command::PushPull {
            face,
            distance: -0.2,
        },
    )
    .unwrap();
    assert_boundary_equivalence(fast.model(), reference.model());
    let vertices = fast.model().vertices().keys().copied().collect();
    fast.execute(Command::Transform {
        vertices,
        matrix: DMat4::from_translation(DVec3::new(1234.567, -89.123, 0.1)),
        copy: false,
    })
    .unwrap();
    fast.model().validate().unwrap();
    let before = fast.model().clone();
    fast.execute(Command::AddEdge {
        start: DVec3::new(1234.567, -89.123, 2.1),
        end: DVec3::new(1236.567, -87.123, 2.1),
    })
    .unwrap();
    fast.model().validate().unwrap();
    fast.undo().unwrap();
    assert_eq!(fast.model(), &before);
    let mut empty = History::new();
    assert!(!empty
        .execute(Command::Transform {
            vertices: Vec::new(),
            matrix: DMat4::from_translation(DVec3::X),
            copy: false
        })
        .unwrap());
}

fn open_grid(transform: DMat4) -> (History, FaceId) {
    let mut geometry = cindral_core::BulkGeometry::default();
    for y in 0..=3 {
        for x in 0..=3 {
            geometry
                .vertices
                .push(transform.transform_point3(DVec3::new(x as f64, y as f64, 0.)));
        }
    }
    for y in 0..3 {
        for x in 0..3 {
            let a = y * 4 + x;
            geometry.faces.push(cindral_core::BulkFace {
                outer: vec![a, a + 1, a + 5, a + 4],
                color: [50, 100, 150, 255],
                ..Default::default()
            });
        }
    }
    let model = geometry.build(false, false).unwrap();
    let face = *model.faces().keys().nth(4).unwrap();
    (History::benchmark_from_data(model.to_data()).unwrap(), face)
}
fn assert_open_equivalence(a: &Model, b: &Model) {
    a.validate().unwrap();
    b.validate().unwrap();
    let area = total_surface_area(a);
    assert!((area - total_surface_area(b)).abs() < TOLERANCE * area.max(1.));
    let (amin, amax) = a.bounds().unwrap();
    let (bmin, bmax) = b.bounds().unwrap();
    assert!(amin.distance(bmin) < TOLERANCE && amax.distance(bmax) < TOLERANCE);
    assert_symmetric_triangle_sample_coverage(a, b, TOLERANCE);
}
#[test]
fn planar_open_extrusion_matches_general_path_in_both_directions() {
    for transform in [
        DMat4::IDENTITY,
        DMat4::from_translation(DVec3::new(1000., -2000., 3000.))
            * DMat4::from_rotation_x(0.37)
            * DMat4::from_scale(DVec3::new(-1., 1., 1.)),
    ] {
        for distance in [-0.2, 0.2] {
            let (mut optimized, face) = open_grid(transform);
            let mut reference = optimized.clone();
            let before = optimized.model().clone();
            let command = Command::PushPull { face, distance };
            optimized.execute(command.clone()).unwrap();
            reference_execute(&mut reference, command).unwrap();
            assert_open_equivalence(optimized.model(), reference.model());
            let after = optimized.model().clone();
            // Neighbors and source attributes survive without face reconstruction.
            for (id, old) in before.faces().iter().filter(|(id, _)| **id != face) {
                assert_eq!(optimized.model().faces().get(id), Some(old));
            }
            optimized.undo().unwrap();
            assert_eq!(optimized.model(), &before);
            optimized.redo().unwrap();
            assert_eq!(optimized.model(), &after);
        }
    }
}
#[test]
fn planar_open_drag_replaces_original_and_commits_one_reversible_step() {
    let (mut h, face) = open_grid(DMat4::from_rotation_y(0.2));
    let before = h.model().clone();
    for distance in [0.1, 0.25, -0.2, 0.3] {
        let mut reference = History::benchmark_from_data(before.to_data()).unwrap();
        let command = Command::PushPull { face, distance };
        h.preview(command.clone()).unwrap();
        reference_execute(&mut reference, command).unwrap();
        assert_open_equivalence(h.model(), reference.model());
        assert_eq!(h.undo_depth(), 0);
    }
    h.cancel_preview().unwrap();
    assert_eq!(h.model(), &before);
    h.preview(Command::PushPull {
        face,
        distance: 0.4,
    })
    .unwrap();
    let after = h.model().clone();
    h.commit_preview().unwrap();
    assert_eq!(h.undo_depth(), 1);
    h.undo().unwrap();
    assert_eq!(h.model(), &before);
    h.redo().unwrap();
    assert_eq!(h.model(), &after);
}
#[test]
fn concave_open_face_matches_general_extrusion() {
    let geometry = cindral_core::BulkGeometry {
        vertices: vec![
            DVec3::ZERO,
            DVec3::X * 2.,
            DVec3::new(2., 1., 0.),
            DVec3::new(1., 1., 0.),
            DVec3::new(1., 2., 0.),
            DVec3::Y * 2.,
        ],
        faces: vec![cindral_core::BulkFace {
            outer: (0..6).collect(),
            ..Default::default()
        }],
        ..Default::default()
    };
    let model = geometry.build(false, false).unwrap();
    let face = *model.faces().keys().next().unwrap();
    for distance in [-0.2, 0.2] {
        let mut h = History::benchmark_from_data(model.to_data()).unwrap();
        let mut reference = h.clone();
        let command = Command::PushPull { face, distance };
        h.execute(command.clone()).unwrap();
        reference_execute(&mut reference, command).unwrap();
        assert_open_equivalence(h.model(), reference.model());
    }
}
