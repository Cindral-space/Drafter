//! Difficult arrangement cases and randomized crossing-line coverage.

use cindral_core::{Command, DVec3 as V, History, TOLERANCE};
use proptest::prelude::*;

fn ring(low: V, high: V) -> Command {
    Command::AddPolyline {
        points: vec![
            low,
            V::new(high.x, low.y, low.z),
            high,
            V::new(low.x, high.y, high.z),
        ],
        closed: true,
    }
}
fn line(start: V, end: V) -> Command {
    Command::AddEdge { start, end }
}
fn area(h: &History) -> f64 {
    h.model()
        .faces()
        .keys()
        .map(|id| h.model().face_area(*id).unwrap())
        .sum()
}

#[test]
fn hole_touching_outer_at_a_vertex_has_no_overlapping_face_area() {
    let mut h = History::new();
    h.execute(ring(V::ZERO, V::new(4., 4., 0.))).unwrap();
    h.execute(Command::AddPolyline {
        points: vec![V::ZERO, V::new(2., 1., 0.), V::new(1., 2., 0.)],
        closed: true,
    })
    .unwrap();
    assert_eq!(h.model().faces().len(), 2);
    assert!((area(&h) - 16.).abs() < TOLERANCE);
    h.model().validate().unwrap();
}

#[test]
fn hole_sharing_a_boundary_edge_becomes_an_indented_region() {
    let mut h = History::new();
    h.execute(ring(V::ZERO, V::new(4., 4., 0.))).unwrap();
    h.execute(ring(V::new(0., 1., 0.), V::new(2., 3., 0.)))
        .unwrap();
    assert_eq!(h.model().faces().len(), 2);
    assert!((area(&h) - 16.).abs() < TOLERANCE);
    assert!(h.model().faces().values().all(|face| face.holes.is_empty()));
}

#[test]
fn chord_crossing_an_annulus_and_its_inner_region_partitions_all_area() {
    let mut h = History::new();
    h.execute(ring(V::ZERO, V::new(4., 4., 0.))).unwrap();
    h.execute(ring(V::new(1., 1., 0.), V::new(3., 3., 0.)))
        .unwrap();
    h.execute(line(V::new(-1., 2., 0.), V::new(5., 2., 0.)))
        .unwrap();
    assert_eq!(h.model().faces().len(), 4);
    assert!((area(&h) - 16.).abs() < TOLERANCE);
    assert!(h.model().faces().values().all(|face| face.holes.is_empty()));
}

#[test]
fn deleting_inner_loop_removes_the_hole_and_unused_vertices() {
    let mut h = History::new();
    h.execute(ring(V::ZERO, V::new(4., 4., 0.))).unwrap();
    let outside: Vec<_> = h.model().edges().keys().copied().collect();
    h.execute(ring(V::new(1., 1., 0.), V::new(3., 3., 0.)))
        .unwrap();
    let inside: Vec<_> = h
        .model()
        .edges()
        .keys()
        .filter(|id| !outside.contains(id))
        .copied()
        .collect();
    h.execute(Command::EraseEdges(inside)).unwrap();
    assert_eq!(h.model().faces().len(), 1);
    assert_eq!(h.model().vertices().len(), 4);
    assert!((area(&h) - 16.).abs() < TOLERANCE);
}

#[test]
fn equal_magnitude_mixed_sign_normal_does_not_duplicate_faces() {
    let base = V::new(500000., -500000., 200000.);
    let u = V::new(1., 1., 0.).normalize() * 0.1;
    let v = V::Z * 0.15;
    let mut h = History::new();
    h.execute(Command::AddPolyline {
        points: vec![base, base + u, base + u + v, base + v],
        closed: true,
    })
    .unwrap();
    assert_eq!(h.model().faces().len(), 1);
    assert!((area(&h) - 0.015).abs() < TOLERANCE);
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]
    #[test]
    fn arbitrary_crossing_chords_conserve_bounded_area(cuts in prop::collection::vec((0.05f64..3.95,0.05f64..3.95),1..9)) {
        let mut h=History::new();h.execute(ring(V::ZERO,V::new(4.,4.,0.))).unwrap();
        for (a,b) in cuts {
            h.execute(line(V::new(0.,a,0.),V::new(4.,b,0.))).unwrap();
            prop_assert!((area(&h)-16.).abs()<TOLERANCE*16.);
            h.model().validate().unwrap();
        }
    }
}

fn components(h: &History) -> usize {
    let mut unseen: std::collections::BTreeSet<_> = h.model().vertices().keys().copied().collect();
    let mut count = 0;
    while let Some(start) = unseen.pop_first() {
        count += 1;
        let mut stack = vec![start];
        while let Some(vertex) = stack.pop() {
            for (_, edge) in h.model().incident_edges(vertex) {
                for neighbor in edge.vertices {
                    if unseen.remove(&neighbor) {
                        stack.push(neighbor);
                    }
                }
            }
        }
    }
    count
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]
    #[test]
    fn planar_arrangements_satisfy_euler_and_validate_after_each_insert(
        segments in prop::collection::vec(((-4i32..=4,-4i32..=4),(-4i32..=4,-4i32..=4)),1..12)
    ) {
        let mut h=History::new();
        for ((ax,ay),(bx,by)) in segments {
            if (ax,ay)==(bx,by) {continue;}
            h.execute(line(V::new(ax as f64,ay as f64,0.),V::new(bx as f64,by as f64,0.))).unwrap();
            let model=h.model();
            let expected=model.edges().len() as isize-model.vertices().len() as isize+components(&h) as isize;
            prop_assert_eq!(model.faces().len() as isize,expected);
            model.validate().unwrap();
        }
    }
}

#[test]
fn translated_collinear_overlaps_handle_float_roundoff() {
    let origin = V::new(123456.7, -34567.8, 200000.);
    let direction = V::new(1., 2., 3.).normalize();
    let point = |t| origin + direction * t;
    let mut h = History::new();
    h.execute(line(point(0.), point(30.))).unwrap();
    h.execute(line(point(10.), point(50.))).unwrap();
    assert_eq!(h.model().edges().len(), 3);
    assert_eq!(h.model().vertices().len(), 4);
    h.model().validate().unwrap();
}

#[test]
fn exterior_boundary_traversal_closes_remote_arrangement_regions() {
    let mut h = History::new();
    for ((ax, ay), (bx, by)) in [
        ((-3, 0), (2, 3)),
        ((0, 0), (-3, 0)),
        ((1, 0), (0, 2)),
        ((-3, -2), (0, 2)),
        ((0, -3), (1, 3)),
        ((-3, 0), (1, 0)),
    ] {
        h.execute(line(
            V::new(ax as f64, ay as f64, 0.),
            V::new(bx as f64, by as f64, 0.),
        ))
        .unwrap();
        let m = h.model();
        let expected =
            m.edges().len() as isize - m.vertices().len() as isize + components(&h) as isize;
        assert_eq!(m.faces().len() as isize, expected);
        m.validate().unwrap();
    }
}
