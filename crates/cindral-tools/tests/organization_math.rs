//! Real property tests for composed, mirrored context-space query transforms.
use cindral_inference::{DrawingPlane, Query, Ray, View};
use cindral_tools::local_query;
use glam::{DMat4, DVec2, DVec3};
use proptest::prelude::*;
proptest! {
    #[test]
    fn nested_mirrored_query_preserves_ray_and_projection(
        tx in -100f64..100., ty in -100f64..100., angle in -3f64..3.,
        sx in 0.01f64..10., sy in 0.01f64..10., sz in 0.01f64..10., mirror in any::<bool>(),
        px in -2f64..2., py in -2f64..2., pz in -2f64..2.
    ) {
        let parent = DMat4::from_translation(DVec3::new(tx,ty,3.)) * DMat4::from_rotation_z(angle);
        let world = parent * DMat4::from_scale(DVec3::new(if mirror {-sx} else {sx}, sy, sz));
        let query = Query { ray: Ray{origin: DVec3::new(1.,2.,20.),direction:-DVec3::Z},
            view: View{matrix:DMat4::IDENTITY,size:DVec2::splat(800.)}, cursor:DVec2::splat(400.),
            plane:DrawingPlane::default(), anchor:None, previous_direction:None, axis_lock:None, tangent:None };
        let local = local_query(world,&query).unwrap();
        let p = DVec3::new(px,py,pz);
        prop_assert!((local.view.matrix.project_point3(p) - query.view.matrix.project_point3(world.transform_point3(p))).length() < 1e-8);
        prop_assert!((world.transform_point3(local.ray.origin) - query.ray.origin).length() < 1e-8);
        prop_assert!((world.transform_vector3(local.ray.direction) - query.ray.direction).length() < 1e-8);
        prop_assert_eq!(local.plane.origin,query.plane.origin);
        prop_assert_eq!(local.plane.normal,query.plane.normal);
    }
}

#[test]
fn tiny_nonsingular_context_scale_is_supported() {
    let world = DMat4::from_scale(DVec3::new(-1e-5, 2e-5, 3e-5));
    let q = Query::new(
        DVec2::ZERO,
        Ray {
            origin: DVec3::Z,
            direction: -DVec3::Z,
        },
        View {
            matrix: DMat4::IDENTITY,
            size: DVec2::splat(800.),
        },
    );
    let local = local_query(world, &q).unwrap();
    assert!((world.transform_vector3(local.ray.direction) - q.ray.direction).length() < 1e-12);
}
