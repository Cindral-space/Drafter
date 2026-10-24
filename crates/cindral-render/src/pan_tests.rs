use super::*;
fn plane_point(camera: &Camera, pixel: Vec2, size: Vec2, depth: f32) -> Vec3 {
    let ndc = Vec2::new(pixel.x / size.x * 2. - 1., 1. - pixel.y / size.y * 2.);
    let (origin, direction) = camera.ray(ndc, size.x / size.y);
    let forward = camera.basis().forward();
    let plane = camera.eye() + forward * depth;
    origin + direction * ((plane - origin).dot(forward) / direction.dot(forward))
}
#[test]
fn picked_plane_stays_under_cursor_in_both_projections() {
    let size = Vec2::new(1200., 800.);
    let pixel = Vec2::new(730., 290.);
    let delta = Vec2::new(53., -31.);
    for projection in [Projection::Perspective, Projection::Orthographic] {
        for distance in [MIN_DISTANCE, 0.1, 10., MAX_DISTANCE] {
            for ratio in [0.1, 1., 5.] {
                let mut camera = Camera { distance, projection, ..Default::default() };
                let original = camera.clone();
                let depth = camera.pan_depth(Some(camera.eye() + camera.basis().forward() * distance * ratio));
                let point = plane_point(&camera, pixel, size, depth);
                camera.pan_at_depth(delta, size.y, depth);
                let after = plane_point(&camera, pixel + delta, size, depth);
                let tolerance = (distance * if projection == Projection::Orthographic { 3e-4 } else { 2e-5 }).max(3e-7);
                assert!((point - after).length() < tolerance, "{projection:?} {distance} {ratio}: {point:?} {after:?}");
                assert_eq!((camera.yaw, camera.pitch, camera.distance, camera.projection),
                    (original.yaw, original.pitch, original.distance, original.projection));
                let translation = camera.target - original.target;
                assert!((camera.eye() - original.eye() - translation).length() < tolerance);
                camera.pan_at_depth(-delta, size.y, depth);
                assert!((camera.target - original.target).length() < tolerance);
            }
        }
    }
}
#[test]
fn orthographic_units_per_pixel_ignore_hit_depth() {
    let mut a = Camera { projection: Projection::Orthographic, ..Default::default() };
    let mut b = a.clone();
    let expected = (a.basis().up * -17. - a.basis().right * 32.) * (2. * half_height(a.distance) / 700.);
    a.pan_at_depth(Vec2::new(32., -17.), 700., MIN_DISTANCE);
    b.pan_at_depth(Vec2::new(32., -17.), 700., MAX_DISTANCE);
    assert_eq!(a, b);
    assert!((a.target - expected).length() < 1e-6);
}
#[test]
fn missing_invalid_behind_and_extreme_depths_are_bounded() {
    for distance in [MIN_DISTANCE, 1., MAX_DISTANCE] {
        let c = Camera { distance, ..Default::default() };
        let forward = c.basis().forward();
        for point in [None, Some(Vec3::NAN), Some(Vec3::INFINITY), Some(c.eye() - forward)] {
            assert_eq!(c.pan_depth(point), distance);
        }
        assert_eq!(c.pan_depth(Some(c.eye() + forward * 1e15)), MAX_DISTANCE);
        // At the largest distances f32 cannot resolve tiny displacements from eye.
        let near = (distance * NEAR_RATIO).max(MIN_DISTANCE);
        let hit = c.eye() + forward * (near * 0.1);
        assert!((c.pan_depth(Some(hit)) - near).abs() <= near * 1e-4);
    }
}
#[test]
fn zero_viewport_extreme_scenes_and_deltas_cannot_corrupt_camera() {
    for distance in [MIN_DISTANCE, MAX_DISTANCE] {
        for target in [Vec3::ZERO, Vec3::splat(1e-8), Vec3::splat(MAX_TARGET_COORDINATE)] {
            let camera = Camera { distance, target, ..Default::default() };
            for height in [0., -1., f32::NAN, f32::INFINITY] {
                let mut c = camera.clone();
                c.pan_at_depth(Vec2::ONE, height, distance);
                assert_eq!(c, camera);
            }
            for delta in [Vec2::splat(f32::MAX), Vec2::NAN, Vec2::INFINITY, Vec2::ONE] {
                let mut c = camera.clone();
                c.pan_at_depth(delta, 1e-30, distance);
                assert!(c.target.is_finite() && c.eye().is_finite());
                assert!(c.view_projection(1.).is_finite());
                assert_eq!((c.yaw, c.pitch, c.distance, c.projection), (camera.yaw, camera.pitch, camera.distance, camera.projection));
            }
        }
    }
}
