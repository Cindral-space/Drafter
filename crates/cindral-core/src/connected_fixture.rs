//! Subdivided connected solid fixture for performance benchmarking.
//!
//! Provides a closed cube boundary subdivided into an `n × n` grid of quad
//! faces on each of its six sides, with all shared corner/side vertices and
//! undirected edges welded.

use std::collections::BTreeMap;

use glam::DVec3;

use crate::{
    Edge, EdgeId, Face, FaceId, GeometryData, History, KernelError, Plane, Vertex, VertexId,
};

/// Construct a closed solid cube with each of its 6 sides subdivided into an
/// `n × n` grid of quad faces (`6 * n * n` faces in total).
///
/// Corner and side vertices and undirected edges are welded across all faces.
/// Returns the initialized [`History`] and the [`FaceId`] of one top-grid face.
///
/// Geometry is built directly into [`GeometryData`] and ingested via
/// [`History::benchmark_from_data`] in linear time without quadratic validation.
pub fn connected_solid(segments: usize) -> Result<(History, FaceId), KernelError> {
    if segments == 0 || segments > 128 {
        return Err(KernelError::InvalidPosition);
    }
    let n = segments;
    let size = n + 1;
    let step = 2.0 / n as f64;

    let mut next_id = 1u64;
    let mut vertices = BTreeMap::new();
    let mut vertex_grid = vec![VertexId::from_raw(0); size * size * size];

    // Allocate all boundary vertices on the [0, 2]^3 cube grid.
    for i in 0..=n {
        for j in 0..=n {
            for k in 0..=n {
                if i == 0 || i == n || j == 0 || j == n || k == 0 || k == n {
                    let id = VertexId::from_raw(next_id);
                    next_id = next_id.checked_add(1).ok_or(KernelError::Capacity)?;
                    let position = DVec3::new(i as f64 * step, j as f64 * step, k as f64 * step);
                    vertices.insert(id, Vertex { position });
                    vertex_grid[(i * size + j) * size + k] = id;
                }
            }
        }
    }

    let mut edges = BTreeMap::new();
    let mut edge_map = BTreeMap::<[VertexId; 2], EdgeId>::new();
    let mut faces = BTreeMap::new();
    let mut top_face_id = None;

    let v = |i: usize, j: usize, k: usize| vertex_grid[(i * size + j) * size + k];

    // Helper to get or insert an undirected edge with sorted endpoints.
    let get_or_insert_edge = |v0: VertexId,
                              v1: VertexId,
                              next_id: &mut u64,
                              edges: &mut BTreeMap<EdgeId, Edge>,
                              edge_map: &mut BTreeMap<[VertexId; 2], EdgeId>|
     -> Result<EdgeId, KernelError> {
        let mut pair = [v0, v1];
        pair.sort();
        if let Some(&id) = edge_map.get(&pair) {
            Ok(id)
        } else {
            let id = EdgeId::from_raw(*next_id);
            *next_id = next_id.checked_add(1).ok_or(KernelError::Capacity)?;
            edge_map.insert(pair, id);
            edges.insert(
                id,
                Edge {
                    vertices: pair,
                    soft: false,
                    smooth: false,
                },
            );
            Ok(id)
        }
    };

    // Helper to register a face with its outer boundary and outward plane.
    let add_quad_face = |ring: [VertexId; 4],
                         plane: Plane,
                         next_id: &mut u64,
                         edges: &mut BTreeMap<EdgeId, Edge>,
                         edge_map: &mut BTreeMap<[VertexId; 2], EdgeId>,
                         faces: &mut BTreeMap<FaceId, Face>|
     -> Result<FaceId, KernelError> {
        for i in 0..4 {
            let a = ring[i];
            let b = ring[(i + 1) % 4];
            get_or_insert_edge(a, b, next_id, edges, edge_map)?;
        }
        let fid = FaceId::from_raw(*next_id);
        *next_id = next_id.checked_add(1).ok_or(KernelError::Capacity)?;
        faces.insert(
            fid,
            Face {
                color: [160, 177, 193, 255],
                material: None,
                outer: ring.to_vec(),
                holes: Vec::new(),
                plane,
            },
        );
        Ok(fid)
    };

    // 1. Top face (+Z): k = n, normal = (0, 0, 1), distance = 2.0
    let top_plane = Plane {
        normal: DVec3::Z,
        distance: 2.0,
    };
    for i in 0..n {
        for j in 0..n {
            let ring = [
                v(i, j, n),
                v(i + 1, j, n),
                v(i + 1, j + 1, n),
                v(i, j + 1, n),
            ];
            let fid = add_quad_face(
                ring,
                top_plane,
                &mut next_id,
                &mut edges,
                &mut edge_map,
                &mut faces,
            )?;
            if top_face_id.is_none() {
                top_face_id = Some(fid);
            }
        }
    }

    // 2. Bottom face (-Z): k = 0, normal = (0, 0, -1), distance = 0.0
    let bottom_plane = Plane {
        normal: -DVec3::Z,
        distance: 0.0,
    };
    for i in 0..n {
        for j in 0..n {
            let ring = [
                v(i, j, 0),
                v(i, j + 1, 0),
                v(i + 1, j + 1, 0),
                v(i + 1, j, 0),
            ];
            add_quad_face(
                ring,
                bottom_plane,
                &mut next_id,
                &mut edges,
                &mut edge_map,
                &mut faces,
            )?;
        }
    }

    // 3. Front face (+Y): j = n, normal = (0, 1, 0), distance = 2.0
    let front_plane = Plane {
        normal: DVec3::Y,
        distance: 2.0,
    };
    for i in 0..n {
        for k in 0..n {
            let ring = [
                v(i, n, k),
                v(i, n, k + 1),
                v(i + 1, n, k + 1),
                v(i + 1, n, k),
            ];
            add_quad_face(
                ring,
                front_plane,
                &mut next_id,
                &mut edges,
                &mut edge_map,
                &mut faces,
            )?;
        }
    }

    // 4. Back face (-Y): j = 0, normal = (0, -1, 0), distance = 0.0
    let back_plane = Plane {
        normal: -DVec3::Y,
        distance: 0.0,
    };
    for i in 0..n {
        for k in 0..n {
            let ring = [
                v(i, 0, k),
                v(i + 1, 0, k),
                v(i + 1, 0, k + 1),
                v(i, 0, k + 1),
            ];
            add_quad_face(
                ring,
                back_plane,
                &mut next_id,
                &mut edges,
                &mut edge_map,
                &mut faces,
            )?;
        }
    }

    // 5. Right face (+X): i = n, normal = (1, 0, 0), distance = 2.0
    let right_plane = Plane {
        normal: DVec3::X,
        distance: 2.0,
    };
    for j in 0..n {
        for k in 0..n {
            let ring = [
                v(n, j, k),
                v(n, j + 1, k),
                v(n, j + 1, k + 1),
                v(n, j, k + 1),
            ];
            add_quad_face(
                ring,
                right_plane,
                &mut next_id,
                &mut edges,
                &mut edge_map,
                &mut faces,
            )?;
        }
    }

    // 6. Left face (-X): i = 0, normal = (-1, 0, 0), distance = 0.0
    let left_plane = Plane {
        normal: -DVec3::X,
        distance: 0.0,
    };
    for j in 0..n {
        for k in 0..n {
            let ring = [
                v(0, j, k),
                v(0, j, k + 1),
                v(0, j + 1, k + 1),
                v(0, j + 1, k),
            ];
            add_quad_face(
                ring,
                left_plane,
                &mut next_id,
                &mut edges,
                &mut edge_map,
                &mut faces,
            )?;
        }
    }

    let top_face = top_face_id.ok_or(KernelError::MissingEntity)?;

    let data = GeometryData {
        vertices,
        edges,
        faces,
        next_id,
    };

    let history = History::benchmark_from_data(data)?;
    Ok((history, top_face))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_fixture_validates_and_has_exact_euler_and_incidence() {
        let n = 2;
        let (history, top_face) = connected_solid(n).expect("connected_solid(2) should succeed");
        let model = history.model();

        // Exact counts for n = 2:
        // V = 6*n^2 + 2 = 26
        // E = 12*n^2 = 48
        // F = 6*n^2 = 24
        let expected_v = 6 * n * n + 2;
        let expected_e = 12 * n * n;
        let expected_f = 6 * n * n;
        assert_eq!(model.vertices().len(), expected_v);
        assert_eq!(model.edges().len(), expected_e);
        assert_eq!(model.faces().len(), expected_f);

        // Euler characteristic V - E + F = 2
        let v = model.vertices().len() as i64;
        let e = model.edges().len() as i64;
        let f = model.faces().len() as i64;
        assert_eq!(v - e + f, 2);

        // Full model validation (quadratic checks, planarity, winding, triangulated area)
        model.validate().expect("model validation must pass");

        // Top face must exist and have normal along +Z
        let face = model.faces().get(&top_face).expect("top face exists");
        assert!(face.plane.normal.z > 0.9);

        // All edge face incidences must be exactly 2
        let mut edge_counts = BTreeMap::<[VertexId; 2], usize>::new();
        for face in model.faces().values() {
            for (a, b) in face.outer.iter().zip(face.outer.iter().cycle().skip(1)) {
                let mut pair = [*a, *b];
                pair.sort();
                *edge_counts.entry(pair).or_insert(0) += 1;
            }
        }
        assert_eq!(edge_counts.len(), expected_e);
        for (pair, count) in &edge_counts {
            assert_eq!(*count, 2, "edge {:?} should have incidence 2", pair);
        }

        // Verify all model edges have incidence 2
        for edge in model.edges().values() {
            assert_eq!(
                edge_counts.get(&edge.vertices).copied(),
                Some(2),
                "model edge {:?} should have incidence 2",
                edge.vertices
            );
        }
    }

    #[test]
    fn unit_fixture_validates_cube() {
        let n = 1;
        let (history, top_face) = connected_solid(n).expect("connected_solid(1) should succeed");
        let model = history.model();
        assert_eq!(model.vertices().len(), 8);
        assert_eq!(model.edges().len(), 12);
        assert_eq!(model.faces().len(), 6);
        model.validate().expect("n=1 cube must validate");
        assert!(model.faces()[&top_face].plane.normal.z > 0.9);
    }

    #[test]
    fn counts_for_n_60() {
        let n = 60;
        let expected_v = 6 * n * n + 2;
        let expected_e = 12 * n * n;
        let expected_f = 6 * n * n;
        assert_eq!(expected_f, 21_600);
        assert_eq!(expected_e, 43_200);
        assert_eq!(expected_v, 21_602);
        assert_eq!(expected_v as i64 - expected_e as i64 + expected_f as i64, 2);
    }
}
