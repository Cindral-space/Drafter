//! M4 surface operations, provisional deltas and manifold property regressions.
use cindral_core::{Command, DVec3 as V, FaceId, History, Model, TOLERANCE};
use std::collections::BTreeMap;
fn rectangle(h: &mut History, z: f64, lo: f64, hi: f64) -> FaceId {
    h.execute(Command::AddPolyline {
        points: vec![
            V::new(lo, lo, z),
            V::new(hi, lo, z),
            V::new(hi, hi, z),
            V::new(lo, hi, z),
        ],
        closed: true,
    })
    .unwrap();
    face_at(h.model(), z, (hi - lo).powi(2))
}
fn face_at(m: &Model, z: f64, area: f64) -> FaceId {
    *m.faces()
        .iter()
        .find(|(id, f)| {
            f.plane.normal.z > 0.9
                && (f.plane.distance - z).abs() < TOLERANCE
                && (m.face_area(**id).unwrap() - area).abs() < 1e-5
        })
        .unwrap()
        .0
}
fn closed(m: &Model) {
    m.validate().unwrap();
    let mut usage = BTreeMap::new();
    for f in m.faces().values() {
        for ring in std::iter::once(&f.outer).chain(&f.holes) {
            for (a, b) in ring.iter().zip(ring.iter().cycle().skip(1)) {
                let mut pair = [*a, *b];
                pair.sort();
                *usage.entry(pair).or_insert(0) += 1;
            }
        }
    }
    for e in m.edges().values() {
        assert_eq!(
            usage.get(&e.vertices),
            Some(&2),
            "not closed: {:?} at {:?} {:?}; faces {:?}",
            e,
            m.vertices()[&e.vertices[0]].position,
            m.vertices()[&e.vertices[1]].position,
            m.faces()
                .iter()
                .filter(
                    |(_, f)| f.outer.contains(&e.vertices[0]) && f.outer.contains(&e.vertices[1])
                )
                .collect::<Vec<_>>()
        );
    }
    assert!(m
        .vertices()
        .keys()
        .all(|v| m.incident_edges(*v).next().is_some()));
}
fn box_model() -> History {
    let mut h = History::new();
    let f = rectangle(&mut h, 0., 0., 4.);
    h.execute(Command::PushPull {
        face: f,
        distance: 4.,
    })
    .unwrap();
    closed(h.model());
    h
}
#[test]
fn rectangle_push_pull_move_undo_redo_script() {
    let mut h = History::new();
    let f = rectangle(&mut h, 0., 0., 4.);
    let flat = h.model().clone();
    h.execute(Command::PushPull {
        face: f,
        distance: 3.,
    })
    .unwrap();
    closed(h.model());
    let solid = h.model().clone();
    let ids = h.model().vertices().keys().copied().collect();
    h.execute(Command::Transform {
        vertices: ids,
        matrix: glam::DMat4::from_translation(V::new(10., 0., 0.)),
        copy: false,
    })
    .unwrap();
    closed(h.model());
    assert_eq!(h.model().bounds().unwrap().0.x, 10.);
    h.undo().unwrap();
    assert_eq!(h.model(), &solid);
    h.undo().unwrap();
    assert_eq!(h.model(), &flat);
    h.redo().unwrap();
    assert_eq!(h.model(), &solid);
}
#[test]
fn pull_out_pocket_and_through_hole_keep_closed() {
    for distance in [1., -1., -4., -6.] {
        let mut h = box_model();
        rectangle(&mut h, 4., 1., 3.);
        let face = face_at(h.model(), 4., 4.);
        h.execute(Command::PushPull { face, distance }).unwrap();
        closed(h.model());
        let expected = 64. + 4. * distance.max(-4.);
        assert!((h.model().closed_solid_volume().unwrap().unwrap() - expected).abs() < 1e-5);
        if distance <= -4. {
            assert!(h.model().faces().values().any(|f| !f.holes.is_empty()));
        }
    }
}
#[test]
fn extrusion_of_face_with_hole_and_adjacent_faces() {
    let mut h = History::new();
    rectangle(&mut h, 0., 0., 4.);
    rectangle(&mut h, 0., 1., 3.);
    let annulus = *h
        .model()
        .faces()
        .iter()
        .find(|(_, f)| !f.holes.is_empty())
        .unwrap()
        .0;
    // Remove the inner face from extrusion by moving only the annular profile; adjacent inner sheet is preserved.
    h.execute(Command::PushPull {
        face: annulus,
        distance: 2.,
    })
    .unwrap();
    h.model().validate().unwrap();
    assert!(h
        .model()
        .faces()
        .values()
        .any(|f| !f.holes.is_empty() && (f.plane.distance - 2.).abs() < TOLERANCE));
}
#[test]
fn previews_replace_cancel_and_commit_one_entry() {
    let mut h = box_model();
    let before = h.model().clone();
    let depth = h.undo_depth();
    let face = face_at(h.model(), 4., 16.);
    for distance in [0.5, 1., 2.] {
        h.preview(Command::PushPull { face, distance }).unwrap();
        assert_eq!(h.undo_depth(), depth);
        closed(h.model());
    }
    h.cancel_preview().unwrap();
    assert_eq!(h.model(), &before);
    assert_eq!(h.undo_depth(), depth);
    for distance in [0.5, 1.] {
        h.preview(Command::PushPull { face, distance }).unwrap();
    }
    h.commit_preview().unwrap();
    assert_eq!(h.undo_depth(), depth + 1);
    h.undo().unwrap();
    assert_eq!(h.model(), &before);
}
proptest::proptest! {
    #![proptest_config(proptest::test_runner::Config::with_cases(64))]
    #[test] fn closed_solid_extrusion_invariants(width in 1f64..8., height in 1f64..5., depth in 0.05f64..4., inward in proptest::bool::ANY) {
        let mut h=History::new();let f=rectangle(&mut h,0.,0.,width);h.execute(Command::PushPull {face:f,distance:height}).unwrap();closed(h.model());
        let f=face_at(h.model(),height,width*width);h.execute(Command::PushPull {face:f,distance:if inward {-depth.min(height*0.9)} else {depth}}).unwrap();closed(h.model());
        let result=h.model().clone();h.undo().unwrap();closed(h.model());h.redo().unwrap();proptest::prop_assert_eq!(h.model(),&result);
    }
}

#[test]
fn annular_solid_face_can_be_pulled_again_without_capping_tunnel() {
    let mut h = box_model();
    rectangle(&mut h, 4., 1., 3.);
    let f = face_at(h.model(), 4., 4.);
    h.execute(Command::PushPull {
        face: f,
        distance: -5.,
    })
    .unwrap();
    closed(h.model());
    let annulus = face_at(h.model(), 4., 12.);
    h.execute(Command::PushPull {
        face: annulus,
        distance: 2.,
    })
    .unwrap();
    closed(h.model());
    assert!((h.model().closed_solid_volume().unwrap().unwrap() - 72.).abs() < 1e-5);
    let annulus = face_at(h.model(), 6., 12.);
    h.execute(Command::SetEdgeStyle {
        edges: vec![*h.model().edges().keys().next().unwrap()],
        soft: true,
        smooth: false,
    })
    .unwrap();
    h.execute(Command::Offset {
        face: annulus,
        distance: 0.2,
    })
    .unwrap();
    closed(h.model());
}
#[test]
fn transforms_copy_mirror_rotate_scale_and_spatial_cache_remain_valid() {
    let mut h = box_model();
    let before = h.model().clone();
    let vertices = h.model().vertices().keys().copied().collect::<Vec<_>>();
    for matrix in [
        glam::DMat4::from_rotation_z(0.3),
        glam::DMat4::from_scale(V::new(-1., 2., 0.5)),
    ] {
        h.execute(Command::Transform {
            vertices: vertices.clone(),
            matrix,
            copy: false,
        })
        .unwrap();
        closed(h.model());
        h.undo().unwrap();
        assert_eq!(h.model(), &before);
    }
    h.execute(Command::Transform {
        vertices,
        matrix: glam::DMat4::from_translation(V::new(10., 0., 0.)),
        copy: true,
    })
    .unwrap();
    closed(h.model());
    assert!((h.model().closed_solid_volume().unwrap().unwrap() - 128.).abs() < 1e-5);
    h.undo().unwrap();
    assert_eq!(h.model(), &before);
}
#[test]
fn sweep_straight_and_bent_profiles_and_paint_are_undoable() {
    for path in [
        vec![V::ZERO, V::Z * 3.],
        vec![V::ZERO, V::Z * 3., V::new(3., 0., 3.)],
    ] {
        let mut h = History::new();
        let face = rectangle(&mut h, 0., -0.25, 0.25);
        let before = h.model().clone();
        h.execute(Command::FollowMe { face, path }).unwrap();
        closed(h.model());
        h.undo().unwrap();
        assert_eq!(h.model(), &before);
        h.redo().unwrap();
        let face = *h.model().faces().keys().next().unwrap();
        h.execute(Command::PaintFace {
            faces: vec![face],
            color: [255, 0, 0, 255],
        })
        .unwrap();
        assert_eq!(h.model().faces()[&face].color, [255, 0, 0, 255]);
        h.undo().unwrap();
        assert_ne!(h.model().faces()[&face].color, [255, 0, 0, 255]);
    }
}
proptest::proptest! {
    #![proptest_config(proptest::test_runner::Config::with_cases(64))]
    #[test]
    fn pocket_or_tunnel_preserves_manifold_and_expected_volume(width in 3f64..10.,height in 1f64..6.,depth_ratio in 0.1f64..1.8) {
        let mut h=History::new();let face=rectangle(&mut h,0.,0.,width);h.execute(Command::PushPull{face,distance:height}).unwrap();
        let low=width/3.;let high=2.*width/3.;let face=rectangle(&mut h,height,low,high);let distance=height*depth_ratio;
        h.execute(Command::PushPull{face,distance:-distance}).unwrap();closed(h.model());
        let expected=width*width*height-(high-low).powi(2)*distance.min(height);
        proptest::prop_assert!((h.model().closed_solid_volume().unwrap().unwrap()-expected).abs()<TOLERANCE*expected.max(1.)*10.);
    }
}

proptest::proptest! {
    #![proptest_config(proptest::test_runner::Config::with_cases(32))]
    #[test]
    fn translated_tilted_solid_extrusion_stays_closed(angle in -1.5f64..1.5,translation in -200_000f64..200_000.,distance in 0.05f64..2.) {
        let mut h=box_model();let face=face_at(h.model(),4.,16.);let ids=h.model().vertices().keys().copied().collect();
        let matrix=glam::DMat4::from_translation(V::splat(translation))*glam::DMat4::from_rotation_x(angle);
        h.execute(Command::Transform{vertices:ids,matrix,copy:false}).unwrap();closed(h.model());
        let before=h.model().clone();h.execute(Command::PushPull{face,distance}).unwrap();closed(h.model());
        proptest::prop_assert!(h.model().closed_solid_volume().unwrap().unwrap()>0.);
        h.undo().unwrap();proptest::prop_assert_eq!(h.model(),&before);
    }
}

#[test]
fn outward_pull_keeps_untouched_bottom_ids_out_of_dirty_sets() {
    let mut h = box_model();
    let bottom = face_at(h.model(), 0., 16.);
    let top = face_at(h.model(), 4., 16.);
    h.execute(Command::PushPull {
        face: top,
        distance: 1.,
    })
    .unwrap();
    assert!(h.model().faces().contains_key(&bottom));
    assert!(!h.changes().faces.contains(&bottom));
    closed(h.model());
}

#[test]
fn section_cap_preserves_tunnel_void_and_translated_plane() {
    let mut h = box_model();
    rectangle(&mut h, 4., 1., 3.);
    let face = face_at(h.model(), 4., 4.);
    h.execute(Command::PushPull {
        face,
        distance: -4.,
    })
    .unwrap();
    let plane =
        cindral_core::SectionPlane::through("Tunnel".into(), V::new(0., 0., 2.), V::Z).unwrap();
    let cap = cindral_core::section_cap(h.model(), &plane).unwrap();
    let area: f64 = cap
        .iter()
        .map(|t| (t[1] - t[0]).cross(t[2] - t[0]).length() / 2.)
        .sum();
    assert!((area - 12.).abs() < 1e-7, "cap area {area}");
    for t in &cap {
        let center = (t[0] + t[1] + t[2]) / 3.;
        assert!(!(center.x > 1. && center.x < 3. && center.y > 1. && center.y < 3.));
    }
    let transform = glam::DMat4::from_scale_rotation_translation(
        V::new(-2., 3., 0.5),
        glam::DQuat::from_rotation_y(0.4),
        V::new(20000., -30000., 40000.),
    );
    let point = transform.transform_point3(V::new(0., 0., 2.));
    let normal = transform
        .inverse()
        .transpose()
        .transform_vector3(V::Z)
        .normalize();
    let world = cindral_core::SectionPlane::through("Transformed".into(), point, normal).unwrap();
    let local = world.in_frame(transform).unwrap();
    let transformed = cindral_core::section_cap(h.model(), &local).unwrap();
    let area: f64 = transformed
        .iter()
        .map(|t| (t[1] - t[0]).cross(t[2] - t[0]).length() / 2.)
        .sum();
    assert!((area - 12.).abs() < 1e-7);
}
