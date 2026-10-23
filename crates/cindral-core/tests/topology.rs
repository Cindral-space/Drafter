//! Topology and transactional-history regression coverage.
use cindral_core::{Command, DVec3 as V, EdgeId, History, KernelError, TOLERANCE};

fn line(a: V, b: V) -> Command {
    Command::AddEdge { start: a, end: b }
}
fn polygon(points: &[[f64; 3]]) -> Command {
    Command::AddPolyline {
        points: points.iter().map(|p| V::from_array(*p)).collect(),
        closed: true,
    }
}
fn square() -> Command {
    polygon(&[[0., 0., 0.], [4., 0., 0.], [4., 4., 0.], [0., 4., 0.]])
}
fn area(h: &History) -> f64 {
    h.model()
        .faces()
        .keys()
        .map(|id| h.model().face_area(*id).unwrap())
        .sum()
}
fn near(a: f64, b: f64) {
    assert!((a - b).abs() < TOLERANCE * a.abs().max(1.0), "{a} != {b}");
}
fn edge_between(h: &History, a: V, b: V) -> EdgeId {
    *h.model()
        .edges()
        .iter()
        .find(|(_, e)| {
            let p = h.model().vertices()[&e.vertices[0]].position;
            let q = h.model().vertices()[&e.vertices[1]].position;
            (p.distance(a) < TOLERANCE && q.distance(b) < TOLERANCE)
                || (p.distance(b) < TOLERANCE && q.distance(a) < TOLERANCE)
        })
        .unwrap()
        .0
}

#[test]
fn closing_loop_creates_face_and_undo_redo_restores_exact_state() {
    let mut h = History::new();
    for pair in [
        (V::ZERO, V::new(4., 0., 0.)),
        (V::new(4., 0., 0.), V::new(4., 4., 0.)),
        (V::new(4., 4., 0.), V::new(0., 4., 0.)),
    ] {
        h.execute(line(pair.0, pair.1)).unwrap();
    }
    assert!(h.model().faces().is_empty());
    let before = h.model().clone();
    h.execute(line(V::new(0., 4., 0.), V::ZERO)).unwrap();
    assert_eq!(h.model().faces().len(), 1);
    near(area(&h), 16.);
    let after = h.model().clone();
    h.undo().unwrap();
    assert_eq!(h.model(), &before);
    h.redo().unwrap();
    assert_eq!(h.model(), &after);
    assert_eq!(h.revision(), 6);
}

#[test]
fn diagonal_splits_and_erasing_it_merges() {
    let mut h = History::new();
    h.execute(square()).unwrap();
    h.execute(line(V::ZERO, V::new(4., 4., 0.))).unwrap();
    assert_eq!(h.model().faces().len(), 2);
    near(area(&h), 16.);
    let diagonal = edge_between(&h, V::ZERO, V::new(4., 4., 0.));
    h.execute(Command::EraseEdges(vec![diagonal])).unwrap();
    assert_eq!(h.model().faces().len(), 1);
    near(area(&h), 16.);
    assert_eq!(h.model().edges().len(), 4);
}

#[test]
fn chord_splits_edges_and_erasure_heals_midpoints() {
    let mut h = History::new();
    h.execute(square()).unwrap();
    h.execute(line(V::new(0., 2., 0.), V::new(4., 2., 0.)))
        .unwrap();
    assert_eq!(
        (
            h.model().vertices().len(),
            h.model().edges().len(),
            h.model().faces().len()
        ),
        (6, 7, 2)
    );
    let chord = edge_between(&h, V::new(0., 2., 0.), V::new(4., 2., 0.));
    h.execute(Command::EraseEdges(vec![chord])).unwrap();
    assert_eq!(
        (
            h.model().vertices().len(),
            h.model().edges().len(),
            h.model().faces().len()
        ),
        (4, 4, 1)
    );
    near(area(&h), 16.);
}

#[test]
fn crossing_diagonals_create_four_regions_and_share_one_vertex() {
    let mut h = History::new();
    h.execute(square()).unwrap();
    h.execute(Command::Batch(vec![
        line(V::ZERO, V::new(4., 4., 0.)),
        line(V::new(4., 0., 0.), V::new(0., 4., 0.)),
    ]))
    .unwrap();
    assert_eq!(h.model().vertices().len(), 5);
    assert_eq!(h.model().faces().len(), 4);
    near(area(&h), 16.);
    assert_eq!(h.undo_depth(), 2);
    h.model().validate().unwrap();
}

#[test]
fn crossing_wires_in_3d_split_only_if_they_meet() {
    let mut h = History::new();
    h.execute(line(V::new(-1., 0., 0.), V::new(1., 0., 0.)))
        .unwrap();
    h.execute(line(V::new(0., -1., 1.), V::new(0., 1., 1.)))
        .unwrap();
    assert_eq!(h.model().edges().len(), 2);
    h.execute(line(V::new(0., -1., 0.), V::new(0., 1., 0.)))
        .unwrap();
    assert_eq!(h.model().edges().len(), 5);
    assert!(h.model().faces().is_empty());
}

#[test]
fn collinear_overlap_is_partitioned_and_duplicate_is_a_noop() {
    let mut h = History::new();
    h.execute(line(V::ZERO, V::new(4., 0., 0.))).unwrap();
    h.execute(line(V::new(2., 0., 0.), V::new(6., 0., 0.)))
        .unwrap();
    assert_eq!(
        (h.model().vertices().len(), h.model().edges().len()),
        (4, 3)
    );
    let revision = h.revision();
    assert!(!h
        .execute(line(V::new(6., 0., 0.), V::new(2., 0., 0.)))
        .unwrap());
    assert_eq!(h.revision(), revision);
    h.model().validate().unwrap();
}

#[test]
fn nested_loops_make_holes_without_double_counting_area() {
    let mut h = History::new();
    h.execute(square()).unwrap();
    h.execute(polygon(&[
        [1., 1., 0.],
        [3., 1., 0.],
        [3., 3., 0.],
        [1., 3., 0.],
    ]))
    .unwrap();
    assert_eq!(h.model().faces().len(), 2);
    near(area(&h), 16.);
    assert_eq!(
        h.model()
            .faces()
            .values()
            .map(|f| f.holes.len())
            .sum::<usize>(),
        1
    );
    for (id, face) in h.model().faces() {
        let triangles = h.model().triangulate(*id).unwrap();
        let triangles_area: f64 = triangles
            .iter()
            .map(|[a, b, c]| (*b - *a).cross(*c - *a).length() * 0.5)
            .sum();
        near(triangles_area, h.model().face_area(*id).unwrap());
        if !face.holes.is_empty() {
            assert!(triangles.iter().all(|[a, b, c]| {
                let center = (*a + *b + *c) / 3.;
                !(center.x > 1. && center.x < 3. && center.y > 1. && center.y < 3.)
            }));
        }
    }
}

#[test]
fn three_nested_loops_and_connecting_bridge_preserve_regions() {
    let mut h = History::new();
    h.execute(square()).unwrap();
    h.execute(polygon(&[
        [1., 1., 0.],
        [3., 1., 0.],
        [3., 3., 0.],
        [1., 3., 0.],
    ]))
    .unwrap();
    h.execute(polygon(&[
        [1.5, 1.5, 0.],
        [2.5, 1.5, 0.],
        [2.5, 2.5, 0.],
        [1.5, 2.5, 0.],
    ]))
    .unwrap();
    h.execute(line(V::ZERO, V::new(1., 1., 0.))).unwrap();
    assert_eq!(h.model().faces().len(), 3);
    near(area(&h), 16.);
    h.model().validate().unwrap();
}

#[test]
fn dangling_wire_does_not_destroy_face_and_erase_boundary_does() {
    let mut h = History::new();
    h.execute(square()).unwrap();
    h.execute(line(V::ZERO, V::new(1., 1., 0.))).unwrap();
    assert_eq!(h.model().faces().len(), 1);
    near(area(&h), 16.);
    let boundary = edge_between(&h, V::ZERO, V::new(4., 0., 0.));
    h.execute(Command::EraseEdges(vec![boundary])).unwrap();
    assert!(h.model().faces().is_empty());
}

#[test]
fn concave_and_self_crossing_polygons_find_bounded_regions() {
    let mut h = History::new();
    h.execute(polygon(&[
        [0., 0., 0.],
        [4., 0., 0.],
        [4., 1., 0.],
        [1., 1., 0.],
        [1., 4., 0.],
        [0., 4., 0.],
    ]))
    .unwrap();
    assert_eq!(h.model().faces().len(), 1);
    near(area(&h), 7.);
    let mut h = History::new();
    h.execute(polygon(&[
        [0., 0., 0.],
        [4., 4., 0.],
        [0., 4., 0.],
        [4., 0., 0.],
    ]))
    .unwrap();
    assert_eq!(h.model().faces().len(), 2);
    near(area(&h), 8.);
}

#[test]
fn tilted_translated_plane_and_nonplanar_loop() {
    let base = V::new(900000., -800000., 500000.);
    let u = V::new(1., 2., 3.).normalize();
    let v = u.cross(V::Z).normalize();
    let mut h = History::new();
    h.execute(Command::AddPolyline {
        points: vec![base, base + u * 4., base + u * 4. + v * 3., base + v * 3.],
        closed: true,
    })
    .unwrap();
    assert_eq!(h.model().faces().len(), 1);
    near(area(&h), 12.);
    h.execute(line(base, base + u * 4. + v * 3.)).unwrap();
    assert_eq!(h.model().faces().len(), 2);
    near(area(&h), 12.);
    let mut h = History::new();
    h.execute(polygon(&[
        [0., 0., 0.],
        [4., 0., 0.],
        [4., 4., 1.],
        [0., 4., 0.],
    ]))
    .unwrap();
    assert!(h.model().faces().is_empty());
}

#[test]
fn cube_wireframe_finds_six_independent_planar_faces() {
    let mut h = History::new();
    let bottom = [[0., 0., 0.], [2., 0., 0.], [2., 2., 0.], [0., 2., 0.]];
    let top = bottom.map(|p| [p[0], p[1], 2.]);
    let mut commands = vec![polygon(&bottom), polygon(&top)];
    for i in 0..4 {
        commands.push(line(V::from_array(bottom[i]), V::from_array(top[i])));
    }
    h.execute(Command::Batch(commands)).unwrap();
    assert_eq!(h.model().faces().len(), 6);
    near(area(&h), 24.);
    h.model().validate().unwrap();
}

#[test]
fn style_changes_are_undoable_inherited_on_split_and_preserve_face_ids() {
    let mut h = History::new();
    h.execute(square()).unwrap();
    let original = h.model().clone();
    let faces: Vec<_> = h.model().faces().keys().copied().collect();
    let target = edge_between(&h, V::ZERO, V::new(4., 0., 0.));
    h.execute(Command::SetEdgeStyle {
        edges: vec![target],
        soft: true,
        smooth: true,
    })
    .unwrap();
    assert_eq!(h.model().faces().keys().copied().collect::<Vec<_>>(), faces);
    h.undo().unwrap();
    assert_eq!(h.model(), &original);
    h.redo().unwrap();
    h.execute(line(V::new(2., 0., 0.), V::new(2., 4., 0.)))
        .unwrap();
    assert_eq!(
        h.model()
            .edges()
            .values()
            .filter(|e| e.soft && e.smooth)
            .count(),
        2
    );
}

#[test]
fn errors_and_noops_preserve_state_revision_and_redo() {
    let mut h = History::new();
    h.execute(square()).unwrap();
    h.undo().unwrap();
    let original = h.model().clone();
    let revision = h.revision();
    let result = h.execute(Command::Batch(vec![
        line(V::ZERO, V::X),
        line(V::ZERO, V::ZERO),
    ]));
    assert_eq!(result, Err(KernelError::DegenerateEdge));
    assert_eq!(h.model(), &original);
    assert_eq!(h.revision(), revision);
    assert_eq!(h.redo_depth(), 1);
    assert!(!h.execute(Command::Batch(vec![])).unwrap());
    assert_eq!(h.redo_depth(), 1);
    h.redo().unwrap();
    let before = h.model().clone();
    let id = *h.model().edges().keys().next().unwrap();
    assert_eq!(
        h.execute(Command::SetEdgeStyle {
            edges: vec![id, EdgeId::from_raw(u64::MAX)],
            soft: true,
            smooth: false
        }),
        Err(KernelError::MissingEntity)
    );
    assert_eq!(h.model(), &before);
}

#[test]
fn ids_are_not_recycled_after_undo_and_new_edit() {
    let mut h = History::new();
    h.execute(line(V::ZERO, V::X)).unwrap();
    let ids: Vec<_> = h
        .model()
        .vertices()
        .keys()
        .map(|id| id.value())
        .chain(h.model().edges().keys().map(|id| id.value()))
        .collect();
    h.undo().unwrap();
    h.execute(line(V::Y, V::Z)).unwrap();
    assert!(h
        .model()
        .vertices()
        .keys()
        .map(|id| id.value())
        .chain(h.model().edges().keys().map(|id| id.value()))
        .all(|id| !ids.contains(&id)));
    assert_eq!(h.redo_depth(), 0);
}

#[test]
fn input_bounds_epsilon_welding_and_batch_depth_are_checked() {
    let mut h = History::new();
    for bad in [
        V::splat(f64::NAN),
        V::splat(f64::INFINITY),
        V::splat(1000001.),
    ] {
        assert_eq!(
            h.execute(line(V::ZERO, bad)),
            Err(KernelError::InvalidPosition)
        );
        assert!(h.model().vertices().is_empty());
    }
    assert_eq!(
        h.execute(line(V::ZERO, V::X * TOLERANCE * 0.5)),
        Err(KernelError::DegenerateEdge)
    );
    h.execute(line(V::ZERO, V::X)).unwrap();
    h.execute(line(V::X + V::Y * TOLERANCE * 0.5, V::Y))
        .unwrap();
    assert_eq!(h.model().vertices().len(), 3);
    let mut command = line(V::ZERO, V::Z);
    for _ in 0..66 {
        command = Command::Batch(vec![command]);
    }
    assert_eq!(h.execute(command), Err(KernelError::BatchDepth));
}

#[test]
fn shallow_angle_intersections_and_skinny_triangles_are_not_discarded() {
    let mut h = History::new();
    h.execute(line(V::new(-10000., 0., 0.), V::new(10000., 0., 0.)))
        .unwrap();
    h.execute(line(
        V::new(-10000., -0.00005, 0.),
        V::new(10000., 0.00005, 0.),
    ))
    .unwrap();
    assert_eq!(h.model().edges().len(), 4);
    assert_eq!(h.model().vertices().len(), 5);
    let mut h = History::new();
    h.execute(polygon(&[
        [0., 0., 0.],
        [10000., 0., 0.],
        [5000., 0.00005, 0.],
    ]))
    .unwrap();
    assert_eq!(h.model().faces().len(), 1);
    near(area(&h), 0.25);
}

#[test]
fn nearly_parallel_but_distinct_faces_are_both_created() {
    let mut h = History::new();
    h.execute(polygon(&[
        [0., 0., 0.],
        [10000., 0., 0.],
        [10000., 10000., 0.],
        [0., 10000., 0.],
    ]))
    .unwrap();
    h.execute(polygon(&[
        [0., 0., 0.],
        [10000., 0., 0.],
        [10000., 10000., 0.0001],
        [0., 10000., 0.0001],
    ]))
    .unwrap();
    assert_eq!(h.model().faces().len(), 2);
    h.model().validate().unwrap();
}

#[test]
fn erasing_face_preserves_shared_boundaries_neighbors_and_history() {
    let mut h = History::new();
    h.execute(square()).unwrap();
    let base = *h.model().faces().keys().next().unwrap();
    h.execute(Command::PushPull {
        face: base,
        distance: 4.,
    })
    .unwrap();
    let before = h.model().clone();
    let face = *before.faces().keys().next().unwrap();
    let depth = h.undo_depth();
    h.execute(Command::EraseFaces(vec![face])).unwrap();
    assert_eq!(h.undo_depth(), depth + 1);
    assert_eq!(h.model().vertices(), before.vertices());
    assert_eq!(h.model().edges(), before.edges());
    let mut expected = before.faces().clone();
    expected.remove(&face);
    assert_eq!(h.model().faces(), &expected);
    h.model().validate().unwrap();
    let after = h.model().clone();
    h.undo().unwrap();
    assert_eq!(h.model(), &before);
    h.redo().unwrap();
    assert_eq!(h.model(), &after);
    // A malformed multi-target request must not partially remove a valid face.
    let remaining = *after.faces().keys().next().unwrap();
    assert!(h
        .execute(Command::EraseFaces(vec![remaining, face]))
        .is_err());
    assert_eq!(h.model(), &after);
    assert_eq!(h.undo_depth(), depth + 1);
}

#[test]
fn erasing_annulus_leaves_inner_face_and_all_edges() {
    let mut h = History::new();
    h.execute(square()).unwrap();
    h.execute(polygon(&[
        [1., 1., 0.],
        [2., 1., 0.],
        [2., 2., 0.],
        [1., 2., 0.],
    ]))
    .unwrap();
    let before = h.model().clone();
    let outer = *before
        .faces()
        .iter()
        .find(|(_, f)| !f.holes.is_empty())
        .unwrap()
        .0;
    h.execute(Command::EraseFaces(vec![outer])).unwrap();
    assert_eq!(h.model().edges(), before.edges());
    assert_eq!(h.model().vertices(), before.vertices());
    assert_eq!(h.model().faces().len(), 1);
    near(area(&h), 1.);
    h.model().validate().unwrap();
}
