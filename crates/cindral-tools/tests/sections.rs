//! Section-aware inference, foreground picking and mirrored occurrence regression.
use cindral_core::{Command, DVec3, DefinitionId, History, SectionPlane};
use cindral_inference::{Engine, Query, Ray, View};
use cindral_tools::{pick_instances_section, pick_section, Hit, PickInstance};
#[test]
fn cut_removes_front_face_from_picking_inference_and_occlusion() {
    let mut h = History::new();
    for z in [0., 1.] {
        h.execute(Command::AddPolyline {
            points: vec![
                DVec3::new(-2., -2., z),
                DVec3::new(2., -2., z),
                DVec3::new(2., 2., z),
                DVec3::new(-2., 2., z),
            ],
            closed: true,
        })
        .unwrap();
    }
    let view = View {
        matrix: glam::DMat4::orthographic_rh(-3., 3., -3., 3., 0.1, 20.)
            * glam::DMat4::look_at_rh(DVec3::new(0., 0., 10.), DVec3::ZERO, DVec3::Y),
        size: glam::DVec2::splat(600.),
    };
    let ray = Ray {
        origin: DVec3::new(0., 0., 10.),
        direction: -DVec3::Z,
    };
    let cursor = glam::DVec2::splat(300.);
    let plane = SectionPlane::through("cut".into(), DVec3::Z * 0.5, -DVec3::Z).unwrap();
    let Some(Hit::Face(front)) = pick_section(h.model(), ray, view, cursor, 10., None) else {
        panic!("front face")
    };
    let Some(Hit::Face(back)) = pick_section(h.model(), ray, view, cursor, 10., Some(&plane))
    else {
        panic!("back face")
    };
    assert_ne!(front, back);
    assert_eq!(h.model().faces()[&back].plane.distance, 0.);
    let mut query = Query::new(cursor, ray, view);
    query.section = Some(plane.clone());
    let snap = Engine::new().infer(h.model(), &query).unwrap();
    assert!(snap.position.z.abs() < 1e-9);
    let instance = PickInstance::new(
        vec![],
        DefinitionId(0),
        h.model(),
        glam::DMat4::from_scale(DVec3::new(-1., 2., 1.)),
    );
    let hit = pick_instances_section(&[instance], ray, view, cursor, 10., Some(&plane)).unwrap();
    assert_eq!(hit.hit, Hit::Face(back));
}
