//! Select contracts using public APIs and authored geometry; no GPU.
use cindral_core::{Command, DVec3, History};
use cindral_inference::{Ray, View};
use cindral_tools::{pick, Hit, Selection};
use glam::{DMat4, DVec2};

fn fixture() -> History {
    let mut history = History::new();
    for x in [0., 4.] {
        history
            .execute(Command::AddPolyline {
                points: vec![
                    DVec3::new(x, 0., 0.),
                    DVec3::new(x + 2., 0., 0.),
                    DVec3::new(x + 2., 2., 0.),
                    DVec3::new(x, 2., 0.),
                ],
                closed: true,
            })
            .unwrap();
    }
    history
}

fn view() -> View {
    View {
        matrix: DMat4::orthographic_rh(-4., 4., -3., 3., 0.1, 100.)
            * DMat4::look_at_rh(DVec3::new(3., 1., 10.), DVec3::new(3., 1., 0.), DVec3::Y),
        size: DVec2::new(800., 600.),
    }
}

#[test]
fn select_parity_single_replaces_add_preserves_without_history_changes() {
    let h = fixture();
    let before = h.model().clone();
    let depth = h.undo_depth();
    let faces: Vec<_> = h.model().faces().keys().copied().collect();
    let edge = *h.model().edges().keys().next().unwrap();
    let mut s = Selection::default();
    s.select(h.model(), Hit::Face(faces[0]), false);
    assert_eq!(s.faces.len(), 1);
    assert!(s.edges.is_empty());
    s.select(h.model(), Hit::Face(faces[1]), true);
    assert_eq!(s.faces.len(), 2);
    s.select(h.model(), Hit::Edge(edge), false);
    assert!(s.faces.is_empty());
    assert_eq!(s.edges, [edge].into());
    s.clear();
    assert_eq!(s, Selection::default());
    assert_eq!(h.model(), &before);
    assert_eq!(h.undo_depth(), depth);
    h.model().validate().unwrap();
}

#[test]
fn select_parity_connected_excludes_disconnected_island() {
    let h = fixture();
    let face = *h.model().faces().keys().next().unwrap();
    let mut s = Selection::default();
    s.connected(h.model(), Hit::Face(face), false);
    assert_eq!(s.faces, [face].into());
    assert_eq!(s.edges.len(), 4);
    assert_eq!(s.vertices.len(), 4);
    let other = *h.model().faces().keys().find(|&&f| f != face).unwrap();
    s.connected(h.model(), Hit::Face(other), true);
    assert_eq!(s.faces.len(), 2);
    assert_eq!(s.edges.len(), 8);
}

#[test]
fn select_parity_window_and_crossing_have_different_membership() {
    let h = fixture();
    let v = view();
    let a = v.project(DVec3::new(0.5, 2.5, 0.)).unwrap();
    let b = v.project(DVec3::new(1.5, -0.5, 0.)).unwrap();
    assert!(a.x < b.x);
    let mut s = Selection::default();
    s.region(h.model(), v, a, b, false);
    assert!(s.faces.is_empty());
    assert!(s.edges.is_empty());
    s.region(h.model(), v, b, a, false);
    assert_eq!(s.faces.len(), 1);
    assert_eq!(s.edges.len(), 2);
    // A containing window includes all boundaries of A, excluding B.
    s.region(
        h.model(),
        v,
        v.project(DVec3::new(-0.5, 2.5, 0.)).unwrap(),
        v.project(DVec3::new(2.5, -0.5, 0.)).unwrap(),
        false,
    );
    assert_eq!(s.faces.len(), 1);
    assert_eq!(s.edges.len(), 4);
}

#[test]
fn select_parity_pick_face_edge_and_empty_space() {
    let h = fixture();
    let v = view();
    for (world, expected) in [
        (DVec3::new(1., 1., 0.), 1),
        (DVec3::new(1., 0., 0.), 2),
        (DVec3::new(3., 1., 0.), 0),
    ] {
        let hit = pick(
            h.model(),
            Ray {
                origin: world + DVec3::Z * 10.,
                direction: -DVec3::Z,
            },
            v,
            v.project(world).unwrap(),
            4.,
        );
        match expected {
            1 => assert!(matches!(hit, Some(Hit::Face(_)))),
            2 => assert!(matches!(hit, Some(Hit::Edge(_)))),
            _ => assert_eq!(hit, None),
        }
    }
}

#[test]
fn select_parity_selection_does_not_consume_geometry_undo_redo() {
    let mut h = fixture();
    let before = h.model().clone();
    let depth = h.undo_depth();
    let face = *h.model().faces().keys().next().unwrap();
    let mut s = Selection::default();
    s.connected(h.model(), Hit::Face(face), false);
    assert_eq!(h.undo_depth(), depth);
    assert!(h.undo().unwrap());
    assert_eq!(h.model().faces().len(), 1);
    s.clear();
    assert!(h.redo().unwrap());
    assert_eq!(h.model(), &before);
    h.model().validate().unwrap();
}

#[test]
#[ignore = "corner picking can return independently selectable Hit::Vertex"]
fn select_parity_corner_does_not_select_standalone_vertex() {
    let h = fixture();
    let v = view();
    let world = DVec3::ZERO;
    let hit = pick(
        h.model(),
        Ray {
            origin: world + DVec3::Z * 10.,
            direction: -DVec3::Z,
        },
        v,
        v.project(world).unwrap(),
        4.,
    );
    assert!(hit.is_some());
    assert!(!matches!(hit, Some(Hit::Vertex(_))));
}
