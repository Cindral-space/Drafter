//! Rotate contracts through the existing transform and history paths.
use cindral_core::{Command, DVec3, History};
use cindral_tools::{rotation, ModifyKind, ModifySession};
use proptest::prelude::*;
fn fixture() -> History {
    let mut h = History::new();
    h.execute(Command::AddPolyline {
        points: vec![DVec3::ZERO, DVec3::X, DVec3::X + DVec3::Y, DVec3::Y],
        closed: true,
    })
    .unwrap();
    h
}
fn session(h: &History, center: DVec3, normal: DVec3) -> ModifySession {
    let mut s = ModifySession::new(ModifyKind::Rotate);
    s.begin_rotation(
        h.model().vertices().keys().copied().collect(),
        center,
        normal,
    )
    .unwrap();
    s
}
#[test]
fn center_only_typed_slopes_reentry_invalid_exact_history() {
    let mut h = fixture();
    let before = h.model().clone();
    let depth = h.undo_depth();
    let mut s = session(
        &h,
        DVec3::new(2., 3., 4.),
        DVec3::new(1., 2., 3.).normalize(),
    );
    for text in ["1:2", "8:12", "-1:2", "1:-2", "45", "-45", "22.5"] {
        let replacing = s.editing_completed();
        let c = s.measure(text).unwrap().unwrap();
        if replacing {
            h.replace_last(c).unwrap();
        } else {
            h.execute(c).unwrap();
        }
        s.finish();
        h.model().validate().unwrap();
        assert_eq!(h.undo_depth(), depth + 1);
        let angle = match text {
            "1:2" => 0.5_f64.atan().to_degrees(),
            "8:12" => (8_f64 / 12.).atan().to_degrees(),
            "-1:2" | "1:-2" => (-0.5_f64).atan().to_degrees(),
            _ => text.parse().unwrap(),
        };
        let matrix = rotation::matrix(
            DVec3::new(2., 3., 4.),
            DVec3::new(1., 2., 3.).normalize(),
            angle.to_radians(),
        );
        for (id, v) in before.vertices() {
            assert!(
                h.model().vertices()[id]
                    .position
                    .distance(matrix.transform_point3(v.position))
                    < 1e-9
            );
        }
    }
    let completed = h.model().clone();
    for bad in ["1:0", "foo", "NaN", "1:2:3", "45m"] {
        assert!(s.measure(bad).is_err(), "{bad}");
        assert_eq!(h.model(), &completed);
    }
    h.undo().unwrap();
    assert_eq!(h.model(), &before);
    h.redo().unwrap();
    assert_eq!(h.model(), &completed);
}
#[test]
fn signed_rotation_and_ring_threshold() {
    let p = rotation::matrix(DVec3::ZERO, DVec3::Z, std::f64::consts::FRAC_PI_2)
        .transform_point3(DVec3::X);
    assert!(p.distance(DVec3::Y) < 1e-12);
    assert!((rotation::angle(DVec3::Z, DVec3::X, DVec3::Y).to_degrees() - 90.).abs() < 1e-12);
    let a = 22_f64.to_radians();
    for d in [0.75, 1., 1.25] {
        assert!((rotation::snap(a, d, 1., 15).to_degrees() - 15.).abs() < 1e-10);
    }
    for d in [0.749, 1.251, 4.] {
        assert_eq!(rotation::snap(a, d, 1., 15), a);
    }
    assert!((rotation::snap(-a, 1., 1., 15).to_degrees() + 15.).abs() < 1e-10);
    assert!((rotation::snap(a, 1., 1., 10).to_degrees() - 20.).abs() < 1e-10);
}
#[test]
fn preview_cancel_is_exact_and_has_no_history() {
    for stage in 0..3 {
        let mut h = fixture();
        let before = h.model().clone();
        let depth = h.undo_depth();
        let mut s = session(&h, DVec3::new(3., 2., 1.), DVec3::Z);
        if stage > 0 {
            s.set_reference(DVec3::new(4., 2., 1.)).unwrap();
        }
        if stage > 1 {
            h.preview(s.update(DVec3::new(3., 3., 1.)).unwrap().unwrap())
                .unwrap();
        }
        assert_eq!(h.undo_depth(), depth);
        h.cancel_preview().unwrap();
        s.cancel();
        assert_eq!(h.model(), &before);
        s.begin_rotation(
            before.vertices().keys().copied().collect(),
            DVec3::ZERO,
            DVec3::Y,
        )
        .unwrap();
        assert!(s.measure("-45").unwrap().is_some());
    }
}
proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]
    #[test]
    fn rotation_then_opposite_returns_geometry(angle in -179_f64..179.,x in -10_f64..10.,y in -10_f64..10.,z in -10_f64..10.) {
        let mut h=fixture();let before=h.model().clone();let center=DVec3::new(x,y,z);let normal=DVec3::new(1.,2.,3.).normalize();
        for a in [angle,-angle] {
            let mut s=session(&h,center,normal);
            h.execute(s.measure(&a.to_string()).unwrap().unwrap()).unwrap();
        }
        h.model().validate().unwrap();
        for (id,v) in before.vertices() { prop_assert!(h.model().vertices()[id].position.distance(v.position)<1e-8); }
    }
}

#[test]
fn radial_arrays_replace_in_one_undo_slot() {
    let mut h = fixture();
    let before = h.model().clone();
    let depth = h.undo_depth();
    let center = DVec3::new(-10., -10., 0.);
    let mut s = session(&h, center, DVec3::Z);
    s.set_rotation_copy(true);
    h.execute(s.measure("60").unwrap().unwrap()).unwrap();
    s.finish();
    for (text, angles) in [
        ("3x", vec![60_f64, 120., 180.]),
        ("*2", vec![60., 120.]),
        ("/3", vec![20., 40., 60.]),
        ("3x", vec![60., 120., 180.]),
    ] {
        h.replace_last(s.measure(text).unwrap().unwrap()).unwrap();
        s.finish();
        assert_eq!(h.undo_depth(), depth + 1);
        assert_eq!(h.model().faces().len(), angles.len() + 1);
        for a in angles {
            for v in before.vertices().values() {
                let p =
                    rotation::matrix(center, DVec3::Z, a.to_radians()).transform_point3(v.position);
                assert!(h
                    .model()
                    .vertices()
                    .values()
                    .any(|v| v.position.distance(p) < 1e-8));
            }
        }
        h.model().validate().unwrap();
    }
    let copied = h.model().clone();
    h.undo().unwrap();
    assert_eq!(h.model(), &before);
    h.redo().unwrap();
    assert_eq!(h.model(), &copied);
}

#[test]
fn partial_selection_uses_shared_connectivity_and_rejects_nonplanar() {
    let mut h = fixture();
    let face = *h.model().faces().keys().next().unwrap();
    h.execute(Command::PushPull { face, distance: 1. }).unwrap();
    let before = h.model().clone();
    let depth = h.undo_depth();
    let corner = *before
        .vertices()
        .iter()
        .find(|(_, v)| v.position.distance(DVec3::new(1., 1., 1.)) < 1e-9)
        .unwrap()
        .0;
    let mut s = ModifySession::new(ModifyKind::Rotate);
    s.begin_rotation(vec![corner], DVec3::ZERO, DVec3::Z)
        .unwrap();
    assert!(h.preview(s.measure("37").unwrap().unwrap()).is_err());
    h.cancel_preview().unwrap();
    assert_eq!(h.model(), &before);
    assert_eq!(h.undo_depth(), depth);
    h.model().validate().unwrap();
    // An entire connected edge can rotate within its adjacent planes or be rejected atomically.
    let edge = *before.edges().keys().next().unwrap();
    let selected = before.edges()[&edge].vertices.to_vec();
    s.begin_rotation(selected, DVec3::ZERO, DVec3::Z).unwrap();
    let result = h.preview(s.measure("90").unwrap().unwrap());
    if result.is_ok() {
        h.model().validate().unwrap();
    }
    h.cancel_preview().unwrap();
    assert_eq!(h.model(), &before);
}
