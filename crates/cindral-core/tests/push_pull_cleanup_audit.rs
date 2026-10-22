//! Cleanup audit: distinguish obsolete caps, wall seams and attached-sheet bases.
use cindral_core::{Command, DVec3 as V, FaceId, History, Model, TOLERANCE};

fn square() -> History {
    let mut h = History::new();
    h.execute(Command::AddPolyline {
        points: vec![V::ZERO, V::X * 2., (V::X + V::Y) * 2., V::Y * 2.],
        closed: true,
    })
    .unwrap();
    h
}

fn cap(m: &Model, z: f64) -> FaceId {
    *m.faces()
        .iter()
        .find(|(_, f)| f.plane.normal.z > 0.9 && (f.plane.distance - z).abs() < TOLERANCE)
        .unwrap()
        .0
}

fn pull(h: &mut History, from: f64, distance: f64) {
    let face = cap(h.model(), from);
    h.execute(Command::PushPull { face, distance }).unwrap();
    h.model().validate().unwrap();
}

fn ring_edges(m: &Model, z: f64) -> usize {
    m.edges()
        .values()
        .filter(|e| {
            e.vertices
                .iter()
                .all(|v| (m.vertices()[v].position.z - z).abs() < TOLERANCE)
        })
        .count()
}

fn attached_sheet() -> History {
    let mut h = square();
    h.execute(Command::AddPolyline {
        points: vec![
            V::new(2., 0., 0.),
            V::new(4., 0., 0.),
            V::new(4., 2., 0.),
            V::new(2., 2., 0.),
        ],
        closed: true,
    })
    .unwrap();
    let face = *h
        .model()
        .faces()
        .iter()
        .find(|(_, f)| {
            f.outer
                .iter()
                .all(|v| h.model().vertices()[v].position.x <= 2.)
        })
        .unwrap()
        .0;
    h.execute(Command::PushPull { face, distance: 1. }).unwrap();
    h
}

#[test]
fn repeated_outward_pull_heals_wall_seams_without_old_caps() {
    let mut h = square();
    pull(&mut h, 0., 1.);
    let before = h.model().clone();
    pull(&mut h, 1., 1.);
    let after = h.model().clone();
    assert_eq!(
        (
            after.vertices().len(),
            after.edges().len(),
            after.faces().len()
        ),
        (8, 12, 6)
    );
    assert_eq!(ring_edges(&after, 1.), 0);
    assert!(
        !after.faces().values().any(|f| {
            f.outer
                .iter()
                .all(|v| (after.vertices()[v].position.z - 1.).abs() < TOLERANCE)
        }),
        "old horizontal cap must be removed"
    );
    assert!((after.closed_solid_volume().unwrap().unwrap() - 8.).abs() < TOLERANCE);
    h.undo().unwrap();
    assert_eq!(h.model(), &before);
    h.redo().unwrap();
    assert_eq!(h.model(), &after);
}

#[test]
fn inward_and_through_push_remove_old_cap_and_ring() {
    for distance in [-0.5, -1., -2.] {
        let mut h = square();
        pull(&mut h, 0., 1.);
        pull(&mut h, 1., distance);
        assert_eq!(ring_edges(h.model(), 1.), 0);
        if distance > -1. {
            assert_eq!(
                (
                    h.model().vertices().len(),
                    h.model().edges().len(),
                    h.model().faces().len()
                ),
                (8, 12, 6)
            );
            assert!((h.model().closed_solid_volume().unwrap().unwrap() - 2.).abs() < TOLERANCE);
        } else {
            assert!(h.model().faces().is_empty());
            assert!(h.model().edges().is_empty());
            assert!(h.model().vertices().is_empty());
        }
    }
}

#[test]
fn replacing_preview_does_not_accumulate_intermediate_rings() {
    let mut h = square();
    pull(&mut h, 0., 1.);
    let original = h.model().clone();
    let depth = h.undo_depth();
    let face = cap(h.model(), 1.);
    for distance in [0.25, 0.5, 0.75] {
        h.preview(Command::PushPull { face, distance }).unwrap();
        h.model().validate().unwrap();
        assert_eq!(h.model().faces().len(), 6);
        assert_eq!(h.model().edges().len(), 12);
        assert_eq!(ring_edges(h.model(), 1.), 0);
        assert_eq!(h.undo_depth(), depth);
    }
    assert_eq!(ring_edges(h.model(), 1.25), 0);
    assert_eq!(ring_edges(h.model(), 1.5), 0);
    h.cancel_preview().unwrap();
    assert_eq!(h.model(), &original);
}

#[test]
fn attached_sheet_replaces_base_without_three_face_edge() {
    let h = attached_sheet();
    let m = h.model();
    m.validate().unwrap();
    assert_eq!(m.faces().len(), 6);
    let shared = m
        .edges()
        .values()
        .find(|e| {
            e.vertices.iter().all(|v| {
                let p = m.vertices()[v].position;
                (p.x - 2.).abs() < TOLERANCE && p.z.abs() < TOLERANCE
            })
        })
        .unwrap();
    let incident = m
        .faces()
        .values()
        .filter(|f| {
            std::iter::once(&f.outer).chain(&f.holes).any(|r| {
                r.iter().zip(r.iter().cycle().skip(1)).any(|(a, b)| {
                    let mut pair = [*a, *b];
                    pair.sort();
                    pair == shared.vertices
                })
            })
        })
        .count();
    assert_eq!(
        incident, 2,
        "only the neighbor sheet and new wall use the shared edge"
    );
}

#[test]
fn repeated_outward_pull_should_heal_uniform_box_walls() {
    let mut h = square();
    pull(&mut h, 0., 1.);
    pull(&mut h, 1., 1.);
    assert_eq!(
        ring_edges(h.model(), 1.),
        0,
        "obsolete cap perimeter remains"
    );
    assert_eq!(
        (
            h.model().vertices().len(),
            h.model().edges().len(),
            h.model().faces().len()
        ),
        (8, 12, 6)
    );
}

#[test]
fn wall_cleanup_preserves_a_painted_boundary() {
    let mut h = square();
    pull(&mut h, 0., 1.);
    let wall = *h
        .model()
        .faces()
        .iter()
        .find(|(_, f)| f.plane.normal.x.abs() > 0.9)
        .unwrap()
        .0;
    h.execute(Command::PaintFace {
        faces: vec![wall],
        color: [210, 20, 10, 255],
    })
    .unwrap();
    pull(&mut h, 1., 1.);
    assert_eq!(h.model().faces()[&wall].color, [210, 20, 10, 255]);
    assert_eq!(
        h.model().faces().len(),
        7,
        "only the material boundary should divide a wall"
    );
    assert!((h.model().closed_solid_volume().unwrap().unwrap() - 8.).abs() < TOLERANCE);
}

#[test]
fn attached_sheet_should_replace_starting_face() {
    let h = attached_sheet();
    let m = h.model();
    assert!(
        !m.faces().values().any(|f| f.outer.iter().all(|v| {
            let p = m.vertices()[v].position;
            p.x <= 2. && p.z.abs() < TOLERANCE
        })),
        "starting face remains beneath the extrusion"
    );
}
