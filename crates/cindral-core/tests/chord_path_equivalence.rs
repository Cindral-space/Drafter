//! Equivalence between the confined-chord cavity fast path and the exterior-walk
//! cavity path, across ring sizes spanning the merged-ring cap (256 corners).
//! The fast path was widened from 64 to 256 ring corners; every case where the
//! fast path engages must produce identical geometry to the walk path.
use cindral_core::*;
use proptest::prelude::*;
use std::collections::BTreeSet;
use std::sync::atomic::Ordering;

/// Deterministic LCG so property cases build reproducible rings.
struct Lcg(u64);
impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 >> 11
    }
    fn unit(&mut self) -> f64 {
        self.next() as f64 / (1u64 << 53) as f64
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum RingKind {
    Convex,
    /// Alternating radii: simple but non-convex (needs at least 5 corners).
    Star,
    /// Convex ring with extra collinear midpoints on three edges.
    Collinear,
    /// Convex ring with two corners 2e-6 radians apart (near-degenerate).
    NearDegenerate,
}

/// Ring of `n` corners on a 10-unit circle, z = 0.
fn ring(n: usize, kind: RingKind, rng: &mut Lcg) -> Vec<DVec3> {
    let mut radius = vec![10.0; n];
    if kind == RingKind::Star && n >= 5 {
        for (i, r) in radius.iter_mut().enumerate() {
            if i % 2 == 1 {
                *r = 4.5;
            }
        }
    }
    let spread = std::f64::consts::TAU / n as f64;
    let mut angles: Vec<f64> = (0..n)
        .map(|i| i as f64 * spread + (rng.unit() - 0.5) * 0.8 * spread)
        .collect();
    angles.sort_by(|a, b| a.total_cmp(b));
    if kind == RingKind::NearDegenerate {
        angles[1] = angles[0] + 2e-6;
        angles.sort_by(|a, b| a.total_cmp(b));
    }
    let mut vertices: Vec<DVec3> = angles
        .iter()
        .zip(&radius)
        .map(|(a, r)| DVec3::new(r * a.cos(), r * a.sin(), 0.))
        .collect();
    if kind == RingKind::Collinear {
        // Split three edges in ring order; the ring stays convex with straight runs.
        let split = [1usize, 1 + n / 3, 1 + 2 * n / 3];
        let mut ordered = Vec::with_capacity(n + 3);
        for i in 0..n {
            ordered.push(vertices[i]);
            if split.contains(&i) {
                let b = vertices[(i + 1) % n];
                let a = vertices[i];
                ordered.push(DVec3::new((a.x + b.x) / 2., (a.y + b.y) / 2., 0.));
            }
        }
        vertices = ordered;
    }
    vertices
}

fn geometry(ring: &[DVec3]) -> BulkGeometry {
    let centroid = ring.iter().sum::<DVec3>() / ring.len() as f64;
    let mut g = BulkGeometry {
        vertices: ring.to_vec(),
        ..Default::default()
    };
    g.faces.push(BulkFace {
        outer: (0..ring.len()).collect(),
        ..Default::default()
    });
    // Sentinel face sharing ring vertex 0 through one vertex only: the slow
    // exterior walk collects it into the cavity, the confined fast path does not.
    let out = (ring[0] - centroid).normalize();
    let tangent = DVec3::new(-out.y, out.x, 0.);
    g.vertices.extend([
        ring[0] + out * 1.2 + tangent * 0.4,
        ring[0] + out * 1.2 - tangent * 0.4,
    ]);
    let n = ring.len();
    g.faces.push(BulkFace {
        outer: vec![0, n, n + 1],
        ..Default::default()
    });
    // A loose wire, followed by both paths, must stay identical too.
    let out2 = (ring[2] - centroid).normalize();
    g.vertices.push(ring[2] + out2 * 0.8);
    g.edges.push(BulkEdge {
        vertices: [2, n + 2],
        ..Default::default()
    });
    g
}

/// Chord between two boundary-edge midpoints far apart; inside a convex ring.
fn chord_for(ring: &[DVec3]) -> (DVec3, DVec3) {
    let n = ring.len();
    let mid = |i: usize| {
        let (a, b) = (ring[i % n], ring[(i + 1) % n]);
        DVec3::new((a.x + b.x) / 2., (a.y + b.y) / 2., 0.)
    };
    let step = (n / 2).max(2);
    (mid(1), mid(1 + step))
}

fn run(ring: &[DVec3], force_slow: bool) -> Result<Model, KernelError> {
    let chord = chord_for(ring);
    let model = geometry(ring).build(false, false).unwrap();
    let mut data = DocumentHistory::new().to_data();
    data.definitions.get_mut(&DefinitionId(0)).unwrap().geometry = model.to_data();
    let mut history = DocumentHistory::from_import_data(data).unwrap();
    FORCE_SLOW_CAVITY.store(force_slow, Ordering::Relaxed);
    let result = history.execute(Command::AddEdge {
        start: chord.0,
        end: chord.1,
    });
    FORCE_SLOW_CAVITY.store(false, Ordering::Relaxed);
    result?;
    let model = history.model().clone();
    model.validate()?;
    Ok(model)
}

fn key(p: DVec3) -> [i64; 3] {
    [
        (p.x * 1e9).round() as i64,
        (p.y * 1e9).round() as i64,
        (p.z * 1e9).round() as i64,
    ]
}

/// Rounded geometry fingerprint: vertex positions, edges with styles, face areas.
fn fingerprint(model: &Model) -> (BTreeSet<[i64; 3]>, BTreeSet<String>, Vec<i64>) {
    let vertices: BTreeSet<_> = model.vertices().values().map(|v| key(v.position)).collect();
    let edges: BTreeSet<_> = model
        .edges()
        .values()
        .map(|e| {
            let a = model.vertices()[&e.vertices[0]].position;
            let b = model.vertices()[&e.vertices[1]].position;
            format!(
                "{:?}{:?}/{}/{}/{}",
                key(a),
                key(b),
                e.soft,
                e.smooth,
                e.hidden
            )
        })
        .collect();
    let mut areas: Vec<i64> = model
        .faces()
        .keys()
        .map(|id| (model.face_area(*id).unwrap() * 1e9).round() as i64)
        .collect();
    areas.sort_unstable();
    (vertices, edges, areas)
}

fn compare_paths(ring: Vec<DVec3>, must_succeed: bool, context: &str) {
    let fast = run(&ring, false);
    let fast_succeeded = fast.is_ok();
    let slow = run(&ring, true);
    match (fast, slow) {
        (Ok(a), Ok(b)) => {
            let (fast_geometry, slow_geometry) = (&fingerprint(&a), &fingerprint(&b));
            assert_eq!(
                fast_geometry.0, slow_geometry.0,
                "{context}: vertex sets differ"
            );
            assert_eq!(
                fast_geometry.1, slow_geometry.1,
                "{context}: edge sets differ"
            );
            assert_eq!(
                fast_geometry.2, slow_geometry.2,
                "{context}: face areas differ"
            );
        }
        (Err(_), Err(_)) => {}
        (fast, slow) => panic!("{context}: path outcome mismatch: fast={fast:?} slow={slow:?}"),
    }
    if must_succeed {
        assert!(
            fast_succeeded,
            "{context}: chord edit unexpectedly rejected"
        );
    }
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 200,
        rng_seed: proptest::test_runner::RngSeed::Fixed(0xC0FFEE),
        ..ProptestConfig::default()
    })]
    #[test]
    fn fast_and_slow_cavity_paths_agree(
        kind_flag in 0u8..2,
        n in 3usize..=257,
        seed in any::<u64>(),
    ) {
        let kind = if kind_flag == 0 || n < 5 {
            RingKind::Convex
        } else {
            RingKind::Star
        };
        let ring = ring(n, kind, &mut Lcg(seed | 1));
        compare_paths(ring, kind == RingKind::Convex, &format!("{kind:?} n={n} seed={seed}"));
    }
}

#[test]
fn boundary_ring_sizes_match_slow_path() {
    let mut seed = 1_000u64;
    for n in [64usize, 65, 255, 256, 257] {
        for kind in [
            RingKind::Convex,
            RingKind::Star,
            RingKind::Collinear,
            RingKind::NearDegenerate,
        ] {
            if kind == RingKind::Star && n < 5 {
                continue;
            }
            let ring = ring(n, kind, &mut Lcg(seed));
            compare_paths(ring, true, &format!("{kind:?} n={n}"));
            seed += 17;
        }
    }
}

#[test]
fn collinear_and_tolerance_reflex_corners_match_slow_path() {
    // Pentagon with a dent of controlled depth into the top edge: d = 0 is exactly
    // collinear, d = 2e-8 is a reflex corner inside the convexity tolerance
    // (|cross| = 2e-7 < TOLERANCE * 5), d = -2 is a strictly convex bump.
    let pentagon = |d: f64| {
        vec![
            DVec3::new(0., 0., 0.),
            DVec3::new(10., 0., 0.),
            DVec3::new(10., 10., 0.),
            DVec3::new(5., 10. - d, 0.),
            DVec3::new(0., 10., 0.),
        ]
    };
    for d in [0., 2e-8, -2.] {
        compare_paths(pentagon(d), true, &format!("pentagon d={d}"));
    }
}
