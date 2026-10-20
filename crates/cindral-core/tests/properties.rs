//! Property tests for the frozen Cindral Drafter M2 topology and history kernel.
//!
//! Covers:
//! - Rectangle chord split and erase healing preserving total area on randomized 3D planes.
//! - Hole triangulation area for nested randomized rectangles.
//! - Undo/redo exact model restoration across sequential edits.
//! - Redo clearing and monotonic fresh IDs after branching edits.
//! - Collinear overlapping segments without duplicate edges.
//! - Atomic invalid-command rollback preserving model state and redo history.

use std::collections::BTreeSet;

use cindral_core::{Command, DVec3, EdgeId, History, Model, MAX_COORDINATE, TOLERANCE};
use proptest::prelude::*;

/// Compute the sum of face areas for all faces currently in the model.
fn model_total_area(model: &Model) -> f64 {
    model
        .faces()
        .keys()
        .map(|id| model.face_area(*id).expect("valid face area"))
        .sum()
}

/// Assert that two floating-point values are equal within a relative tolerance.
fn assert_approx_eq(actual: f64, expected: f64, context: &str) {
    let scale = expected.abs().max(actual.abs()).max(1.0);
    let tolerance = TOLERANCE * scale;
    assert!(
        (actual - expected).abs() <= tolerance,
        "{context}: actual {actual} vs expected {expected} (difference {}) exceeds tolerance {tolerance}",
        (actual - expected).abs()
    );
}

/// Find an edge connecting two 3D positions within `TOLERANCE`.
fn find_edge_between(model: &Model, a: DVec3, b: DVec3) -> Option<EdgeId> {
    model.edges().iter().find_map(|(id, edge)| {
        let p = model.vertices().get(&edge.vertices[0])?.position;
        let q = model.vertices().get(&edge.vertices[1])?.position;
        if (p.distance(a) <= TOLERANCE && q.distance(b) <= TOLERANCE)
            || (p.distance(b) <= TOLERANCE && q.distance(a) <= TOLERANCE)
        {
            Some(*id)
        } else {
            None
        }
    })
}

/// Collect all entity IDs present in a model as raw `u64` values.
fn collect_model_ids(model: &Model) -> BTreeSet<u64> {
    let mut ids = BTreeSet::new();
    for v in model.vertices().keys() {
        ids.insert(v.value());
    }
    for e in model.edges().keys() {
        ids.insert(e.value());
    }
    for f in model.faces().keys() {
        ids.insert(f.value());
    }
    ids
}

/// Strategy for generating a randomized 3D plane frame: `(origin, u, v, normal)`.
///
/// Origin positions are kept in `[-200_000.0, 200_000.0]` to exercise large translations
/// while staying safely within `MAX_COORDINATE = 1e6` after adding local shapes.
fn plane_frame_strategy() -> impl Strategy<Value = (DVec3, DVec3, DVec3, DVec3)> {
    (
        (
            -200_000.0..200_000.0f64,
            -200_000.0..200_000.0f64,
            -200_000.0..200_000.0f64,
        ),
        (0.0..std::f64::consts::TAU, -1.4..1.4f64),
        0.0..std::f64::consts::TAU,
    )
        .prop_map(|((ox, oy, oz), (theta, phi), psi)| {
            let origin = DVec3::new(ox, oy, oz);
            let normal =
                DVec3::new(phi.cos() * theta.cos(), phi.cos() * theta.sin(), phi.sin()).normalize();
            let axis = if normal.x.abs() < 0.8 {
                DVec3::X
            } else {
                DVec3::Y
            };
            let u_base = normal.cross(axis).normalize();
            let v_base = normal.cross(u_base).normalize();
            let u = (u_base * psi.cos() + v_base * psi.sin()).normalize();
            let v = normal.cross(u).normalize();
            (origin, u, v, normal)
        })
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 128,
        ..ProptestConfig::default()
    })]

    /// Property 1: Rectangle chord split and erase healing preserves total area
    /// on randomized tilted/translated 3D planes.
    #[test]
    fn test_rectangle_chord_split_erase_healing(
        (origin, u, v, _) in plane_frame_strategy(),
        width in 10.0..100.0f64,
        height in 10.0..100.0f64,
        split_vertical in any::<bool>(),
        s in 0.2..0.8f64,
        t in 0.2..0.8f64,
    ) {
        let p0 = origin;
        let p1 = origin + u * width;
        let p2 = origin + u * width + v * height;
        let p3 = origin + v * height;

        let mut history = History::new();

        // 1. Create outer rectangle
        let initial_rect = Command::AddPolyline {
            points: vec![p0, p1, p2, p3],
            closed: true,
        };
        history.execute(initial_rect).expect("create rectangle");
        assert_eq!(history.model().faces().len(), 1, "rectangle must produce 1 face");
        assert_eq!(history.model().vertices().len(), 4, "rectangle has 4 vertices");
        assert_eq!(history.model().edges().len(), 4, "rectangle has 4 edges");
        let expected_total_area = width * height;
        assert_approx_eq(model_total_area(history.model()), expected_total_area, "initial area");

        // 2. Add chord splitting the rectangle
        let (chord_start, chord_end) = if split_vertical {
            // Chord from bottom edge (origin + u * s * width) to top edge (origin + u * t * width + v * height)
            (origin + u * (s * width), origin + u * (t * width) + v * height)
        } else {
            // Chord from left edge (origin + v * s * height) to right edge (origin + u * width + v * t * height)
            (origin + v * (s * height), origin + u * width + v * (t * height))
        };

        history.execute(Command::AddEdge {
            start: chord_start,
            end: chord_end,
        }).expect("add chord");

        assert_eq!(history.model().faces().len(), 2, "chord split must produce 2 faces");
        assert_eq!(history.model().vertices().len(), 6, "chord split produces 6 vertices");
        assert_eq!(history.model().edges().len(), 7, "chord split produces 7 edges");
        assert_approx_eq(model_total_area(history.model()), expected_total_area, "split area");
        history.model().validate().expect("split topology valid");

        // 3. Locate the chord edge and erase it
        let chord_id = find_edge_between(history.model(), chord_start, chord_end)
            .expect("chord edge must exist in model");
        history.execute(Command::EraseEdges(vec![chord_id])).expect("erase chord");

        // 4. Verify healing: collinear degree-two boundary vertices dissolved, face merged
        assert_eq!(history.model().faces().len(), 1, "erasing chord merges faces back to 1");
        assert_eq!(history.model().vertices().len(), 4, "healing dissolves midpoints back to 4 vertices");
        assert_eq!(history.model().edges().len(), 4, "healing restores 4 boundary edges");
        assert_approx_eq(model_total_area(history.model()), expected_total_area, "healed area");
        history.model().validate().expect("healed topology valid");
    }

    /// Property 2: Hole triangulation area for nested randomized rectangles on 3D planes.
    #[test]
    fn test_hole_triangulation_area(
        (origin, u, v, _) in plane_frame_strategy(),
        outer_w in 30.0..100.0f64,
        outer_h in 30.0..100.0f64,
        inner_x_frac in 0.15..0.40f64,
        inner_w_frac in 0.20..0.40f64,
        inner_y_frac in 0.15..0.40f64,
        inner_h_frac in 0.20..0.40f64,
    ) {
        let x0 = outer_w * inner_x_frac;
        let x1 = x0 + outer_w * inner_w_frac;
        let y0 = outer_h * inner_y_frac;
        let y1 = y0 + outer_h * inner_h_frac;

        let outer_points = vec![
            origin,
            origin + u * outer_w,
            origin + u * outer_w + v * outer_h,
            origin + v * outer_h,
        ];

        let inner_points = vec![
            origin + u * x0 + v * y0,
            origin + u * x1 + v * y0,
            origin + u * x1 + v * y1,
            origin + u * x0 + v * y1,
        ];

        let mut history = History::new();
        history.execute(Command::AddPolyline {
            points: outer_points,
            closed: true,
        }).expect("outer polyline");

        history.execute(Command::AddPolyline {
            points: inner_points,
            closed: true,
        }).expect("inner polyline");

        assert_eq!(history.model().faces().len(), 2, "nested rectangles produce 2 faces");
        let total_holes: usize = history.model().faces().values().map(|f| f.holes.len()).sum();
        assert_eq!(total_holes, 1, "exactly 1 hole assigned across faces");

        let outer_area = outer_w * outer_h;
        let inner_area = (x1 - x0) * (y1 - y0);
        let annulus_area = outer_area - inner_area;

        let (annulus_id, inner_id) = {
            let mut annulus = None;
            let mut inner = None;
            for (id, face) in history.model().faces() {
                if face.holes.is_empty() {
                    inner = Some(*id);
                } else {
                    annulus = Some(*id);
                }
            }
            (annulus.expect("annulus face"), inner.expect("inner face"))
        };

        // Check face areas
        assert_approx_eq(
            history.model().face_area(annulus_id).expect("annulus area"),
            annulus_area,
            "annulus face area"
        );
        assert_approx_eq(
            history.model().face_area(inner_id).expect("inner area"),
            inner_area,
            "inner face area"
        );
        assert_approx_eq(
            model_total_area(history.model()),
            outer_area,
            "total model area equals outer rectangle area"
        );

        // Check triangulation area matches face_area for all faces including holes
        for (id, face) in history.model().faces() {
            let triangles = history.model().triangulate(*id).expect("triangulate face");
            let tri_area_sum: f64 = triangles
                .iter()
                .map(|[a, b, c]| (*b - *a).cross(*c - *a).length() * 0.5)
                .sum();
            let face_area = history.model().face_area(*id).expect("face area");
            assert_approx_eq(tri_area_sum, face_area, "triangulated area matches face area");

            // For the face with the hole, assert triangle centroids do not lie inside hole interior
            if !face.holes.is_empty() {
                for [a, b, c] in &triangles {
                    let centroid = (*a + *b + *c) / 3.0;
                    let local_x = (centroid - origin).dot(u);
                    let local_y = (centroid - origin).dot(v);
                    let inside_hole = local_x > x0 + TOLERANCE
                        && local_x < x1 - TOLERANCE
                        && local_y > y0 + TOLERANCE
                        && local_y < y1 - TOLERANCE;
                    assert!(!inside_hole, "annulus triangle centroid lies inside hole interior");
                }
            }
        }

        history.model().validate().expect("nested rectangles topology valid");
    }

    /// Property 3: Undo/redo exact model restoration across sequential edits.
    #[test]
    fn test_undo_redo_exact_model_restoration(
        (origin, u, v, _) in plane_frame_strategy(),
        step_sizes in prop::collection::vec((10.0..50.0f64, 10.0..50.0f64), 3..6),
    ) {
        let mut history = History::new();
        let mut snapshots = vec![history.model().clone()];

        for (i, (w, h)) in step_sizes.iter().enumerate() {
            let offset = (i as f64) * 60.0;
            let p0 = origin + u * offset;
            let p1 = origin + u * (offset + w);
            let p2 = origin + u * (offset + w) + v * h;
            let p3 = origin + u * offset + v * h;

            let cmd = Command::AddPolyline {
                points: vec![p0, p1, p2, p3],
                closed: true,
            };
            let applied = history.execute(cmd).expect("execute polygon");
            assert!(applied, "sequential non-overlapping edits must mutate document");
            snapshots.push(history.model().clone());
        }

        let total_steps = snapshots.len();

        // 1. Undo sequentially all the way to empty document
        for expected in snapshots.iter().rev().skip(1) {
            let undone = history.undo().expect("undo");
            assert!(undone, "undo must succeed");
            assert_eq!(history.model(), expected, "model must match snapshot exactly on undo");
        }
        assert!(!history.undo().expect("undo at root is no-op"), "cannot undo past beginning");

        // 2. Redo sequentially all the way to final document
        for expected in snapshots.iter().skip(1) {
            let redone = history.redo().expect("redo");
            assert!(redone, "redo must succeed");
            assert_eq!(history.model(), expected, "model must match snapshot exactly on redo");
        }
        assert!(!history.redo().expect("redo at leaf is no-op"), "cannot redo past end");

        // 3. Partial undo / redo roundtrip
        let partial_undos = total_steps / 2;
        for _ in 0..partial_undos {
            history.undo().expect("partial undo");
        }
        assert_eq!(
            history.model(),
            &snapshots[total_steps - 1 - partial_undos],
            "model matches snapshot after partial undo"
        );
        for _ in 0..partial_undos {
            history.redo().expect("partial redo");
        }
        assert_eq!(
            history.model(),
            &snapshots[total_steps - 1],
            "model matches final snapshot after restoring redo"
        );
    }

    /// Property 4: Redo clearing and monotonic fresh IDs after branching edits.
    #[test]
    fn test_redo_clearing_and_monotonic_fresh_ids_after_branching(
        (origin, u, v, _) in plane_frame_strategy(),
        len_a in 10.0..50.0f64,
        len_b in 10.0..50.0f64,
        len_branch in 10.0..50.0f64,
    ) {
        let mut history = History::new();

        // Branch A: create initial geometry
        let cmd1 = Command::AddEdge {
            start: origin,
            end: origin + u * len_a,
        };
        let cmd2 = Command::AddEdge {
            start: origin + u * len_a,
            end: origin + u * len_a + v * len_b,
        };
        history.execute(cmd1).expect("cmd1");
        history.execute(cmd2).expect("cmd2");

        // Collect all IDs created on Branch A
        let branch_a_ids = collect_model_ids(history.model());
        let max_branch_a_id = *branch_a_ids.iter().max().expect("must have entity IDs");

        // Undo one step so redo stack is non-empty
        history.undo().expect("undo");
        assert!(history.redo_depth() > 0, "redo stack must be non-empty after undo");

        // Execute a new command to form Branch B
        let branch_cmd = Command::AddEdge {
            start: origin - v * 20.0,
            end: origin - v * 20.0 + u * len_branch,
        };
        let applied = history.execute(branch_cmd).expect("execute branching edit");
        assert!(applied, "branching edit must succeed and apply");

        // 1. Redo history must be cleared
        assert_eq!(history.redo_depth(), 0, "successful edit must clear redo history");

        // 2. Newly created entities in Branch B must have fresh, strictly monotonic IDs
        let current_ids = collect_model_ids(history.model());
        let mut new_entity_count = 0;
        for id in &current_ids {
            if !branch_a_ids.contains(id) {
                new_entity_count += 1;
                assert!(
                    *id > max_branch_a_id,
                    "new entity ID {id} must be strictly greater than max Branch A ID {max_branch_a_id}"
                );
            }
        }
        assert!(new_entity_count > 0, "branching edit must allocate fresh entities");
    }

    /// Property 5: Collinear overlaps partition cleanly with no duplicate edges.
    #[test]
    fn test_collinear_overlaps_no_duplicate_edges(
        (origin, u, _, _) in plane_frame_strategy(),
        offset in 0.0..100.0f64,
        d0 in 5.0..20.0f64,
        d1 in 5.0..20.0f64,
        d2 in 5.0..20.0f64,
    ) {
        let t0 = offset;
        let t1 = t0 + d0;
        let t2 = t1 + d1;
        let t3 = t2 + d2;

        let pt = |t: f64| origin + u * t;

        let mut history = History::new();

        // 1. Add Segment 1: [t0, t2]
        history.execute(Command::AddEdge {
            start: pt(t0),
            end: pt(t2),
        }).expect("add seg1");
        assert_eq!(history.model().edges().len(), 1);
        history.model().validate().expect("seg1 valid");

        // 2. Add overlapping Segment 2: [t1, t3]
        history.execute(Command::AddEdge {
            start: pt(t1),
            end: pt(t3),
        }).expect("add overlapping seg2");

        // The union [t0, t3] should be partitioned into exactly 3 edges: [t0, t1], [t1, t2], [t2, t3]
        assert_eq!(
            history.model().edges().len(),
            3,
            "collinear overlap must be partitioned into 3 distinct edges"
        );
        assert_eq!(
            history.model().vertices().len(),
            4,
            "collinear overlap must result in 4 distinct vertices"
        );
        history.model().validate().expect("overlap partition valid");

        // Verify all edge endpoint pairs are strictly unique and ordered
        let mut edge_vertex_pairs = BTreeSet::new();
        for edge in history.model().edges().values() {
            assert!(
                edge.vertices[0] < edge.vertices[1],
                "edge vertex IDs must be in ascending order"
            );
            assert!(
                edge_vertex_pairs.insert(edge.vertices),
                "must not have duplicate edges"
            );
        }

        // 3. Adding an enclosing segment [t0, t3] should be a no-op since it is already partitioned
        let rev_before = history.revision();
        let changed = history.execute(Command::AddEdge {
            start: pt(t0),
            end: pt(t3),
        }).expect("add enclosing segment");
        assert!(!changed, "adding fully covered segment must be a no-op");
        assert_eq!(history.revision(), rev_before, "no-op does not advance revision");
        assert_eq!(history.model().edges().len(), 3, "edge count unchanged");
        history.model().validate().expect("topology unchanged and valid");

        // 4. Adding a reversed subsegment [t2, t1] must also be a no-op
        let changed_sub = history.execute(Command::AddEdge {
            start: pt(t2),
            end: pt(t1),
        }).expect("add reversed subsegment");
        assert!(!changed_sub, "adding existing subsegment must be a no-op");
        assert_eq!(history.model().edges().len(), 3, "edge count unchanged");

        // 5. Extend the line with a partial overlap [t2, t3 + 10.0]
        let t4 = t3 + 10.0;
        history.execute(Command::AddEdge {
            start: pt(t2),
            end: pt(t4),
        }).expect("extend line");
        assert_eq!(history.model().edges().len(), 4, "extending line creates 4th edge");
        assert_eq!(history.model().vertices().len(), 5, "extending line creates 5th vertex");
        history.model().validate().expect("extended line valid");
    }

    /// Property 6: Atomic invalid-command rollback preserves model state, revision, and redo.
    #[test]
    fn test_atomic_invalid_command_rollback(
        (origin, u, v, _) in plane_frame_strategy(),
        error_choice in 0..7u8,
    ) {
        let mut history = History::new();

        // Set up history: apply step 1, apply step 2, undo step 2
        // Resulting state has undo_depth = 1, redo_depth = 1.
        history.execute(Command::AddPolyline {
            points: vec![
                origin,
                origin + u * 20.0,
                origin + u * 20.0 + v * 20.0,
                origin + v * 20.0,
            ],
            closed: true,
        }).expect("setup polyline 1");

        history.execute(Command::AddEdge {
            start: origin + u * 40.0,
            end: origin + u * 40.0 + v * 20.0,
        }).expect("setup edge 2");

        history.undo().expect("undo step 2");
        assert_eq!(history.undo_depth(), 1);
        assert_eq!(history.redo_depth(), 1);

        // Snapshot state before executing invalid command
        let model_before = history.model().clone();
        let revision_before = history.revision();
        let undo_depth_before = history.undo_depth();
        let redo_depth_before = history.redo_depth();

        // Construct invalid command based on error_choice
        let invalid_cmd = match error_choice {
            0 => Command::AddEdge {
                start: origin,
                end: origin, // Degenerate edge (zero length)
            },
            1 => Command::AddEdge {
                start: origin,
                end: origin + u * (TOLERANCE * 0.5), // Degenerate edge (< TOLERANCE)
            },
            2 => Command::AddEdge {
                start: origin,
                end: DVec3::splat(f64::NAN), // Non-finite coordinate
            },
            3 => Command::AddEdge {
                start: origin,
                end: DVec3::splat(MAX_COORDINATE + 100.0), // Out of bounds
            },
            4 => Command::AddPolyline {
                points: vec![origin, origin + u * 10.0],
                closed: true, // Closed polyline requires >= 3 points
            },
            5 => Command::SetEdgeStyle {
                edges: vec![EdgeId::from_raw(u64::MAX - 50)], // Stale / missing entity ID
                soft: true,
                smooth: true,
            },
            6 => {
                // Batch containing valid command followed by invalid command
                Command::Batch(vec![
                    Command::AddEdge {
                        start: origin + u * 100.0,
                        end: origin + u * 120.0,
                    },
                    Command::AddEdge {
                        start: origin,
                        end: origin,
                    },
                ])
            }
            _ => unreachable!(),
        };

        // Execution must return an Err
        let result = history.execute(invalid_cmd);
        assert!(result.is_err(), "invalid command must fail with Err, got: {result:?}");

        // Invariants: model, revision, undo depth, and redo depth are completely unchanged
        assert_eq!(history.model(), &model_before, "model must be untouched on rollback");
        assert_eq!(history.revision(), revision_before, "revision must not change");
        assert_eq!(history.undo_depth(), undo_depth_before, "undo depth must not change");
        assert_eq!(history.redo_depth(), redo_depth_before, "redo depth must be preserved");

        // Redo history must remain intact and functional
        assert!(history.redo().expect("redo after failed command"), "redo must still succeed");
        assert_eq!(history.redo_depth(), 0, "redo depth decremented");
        assert_eq!(history.undo_depth(), 2, "undo depth incremented");

        // Undoing restores back to the snapshot before redo
        assert!(history.undo().expect("undo after redo"), "undo must succeed");
        assert_eq!(history.model(), &model_before, "model restored after undoing redo");
        assert_eq!(history.redo_depth(), redo_depth_before, "redo depth restored");
        assert_eq!(history.undo_depth(), undo_depth_before, "undo depth restored");
    }
}
