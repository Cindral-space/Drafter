//! Bulk construction invariants and import transaction behavior.
use cindral_core::*;
fn square() -> BulkGeometry {
    BulkGeometry {
        vertices: vec![
            DVec3::ZERO,
            DVec3::X,
            DVec3::X + DVec3::Y,
            DVec3::Y,
            DVec3::ZERO + DVec3::splat(TOLERANCE / 4.),
        ],
        faces: vec![
            BulkFace {
                outer: vec![0, 1, 2],
                ..Default::default()
            },
            BulkFace {
                outer: vec![4, 2, 3],
                ..Default::default()
            },
        ],
        ..Default::default()
    }
}
#[test]
fn weld_merge_style_and_validate() {
    let m = square().build(true, true).unwrap();
    assert_eq!(m.vertices().len(), 4);
    assert_eq!(m.faces().len(), 1);
    assert_eq!(m.edges().len(), 4);
    m.validate().unwrap();
    let m = square().build(false, true).unwrap();
    assert_eq!(m.faces().len(), 2);
    assert_eq!(m.edges().values().filter(|e| e.soft && e.smooth).count(), 1);
}
#[test]
fn import_undo_redo_then_local_edit() {
    let mut data = DocumentHistory::new().to_data();
    data.definitions.get_mut(&DefinitionId(0)).unwrap().geometry =
        square().build(true, true).unwrap().to_data();
    let mut h = DocumentHistory::from_import_data(data).unwrap();
    let original = h.to_data();
    assert_eq!(h.undo_depth(), 1);
    h.execute(Command::AddEdge {
        start: DVec3::new(10., 0., 0.),
        end: DVec3::new(11., 0., 0.),
    })
    .unwrap();
    assert_eq!(h.undo_depth(), 2);
    h.undo().unwrap();
    assert_eq!(h.to_data().definitions, original.definitions);
    h.undo().unwrap();
    assert!(h.model().faces().is_empty());
    assert!(!h.changes().faces.is_empty());
    h.redo().unwrap();
    assert_eq!(h.to_data(), original);
    h.redo().unwrap();
    assert_eq!(h.model().edges().len(), 5);
}
#[test]
fn bad_indices_nonfinite_cancel() {
    let mut g = square();
    g.faces[0].outer[1] = usize::MAX;
    assert!(g.build(false, false).is_err());
    let mut g = square();
    g.vertices[0].x = f64::NAN;
    assert!(g.build(false, false).is_err());
    assert!(square().build_cancellable(true, true, || true).is_err());
}
#[test]
fn append_retains_existing_history_and_imports_one_step() {
    let mut h = DocumentHistory::new();
    h.execute(Command::AddEdge {
        start: DVec3::new(10., 0., 0.),
        end: DVec3::new(11., 0., 0.),
    })
    .unwrap();
    let before = h.to_data();
    let mut data = DocumentHistory::new().to_data();
    data.definitions.get_mut(&DefinitionId(0)).unwrap().geometry =
        square().build(true, true).unwrap().to_data();
    h.append_import(data).unwrap();
    assert_eq!(h.undo_depth(), 2);
    assert_eq!(h.model().faces().len(), 1);
    assert_eq!(h.model().edges().len(), 5);
    h.undo().unwrap();
    assert_eq!(h.to_data().definitions, before.definitions);
    h.undo().unwrap();
    assert!(h.model().edges().is_empty());
    h.redo().unwrap();
    h.redo().unwrap();
    assert_eq!(h.model().edges().len(), 5);
}
#[test]
fn holes_and_material_boundaries_survive_cleanup() {
    let mut g = BulkGeometry {
        vertices: vec![
            DVec3::ZERO,
            DVec3::new(4., 0., 0.),
            DVec3::new(4., 4., 0.),
            DVec3::new(0., 4., 0.),
            DVec3::new(1., 1., 0.),
            DVec3::new(3., 1., 0.),
            DVec3::new(3., 3., 0.),
            DVec3::new(1., 3., 0.),
        ],
        faces: vec![BulkFace {
            outer: vec![0, 1, 2, 3],
            holes: vec![vec![4, 5, 6, 7]],
            ..Default::default()
        }],
        ..Default::default()
    };
    let model = g.build(true, true).unwrap();
    model.validate().unwrap();
    assert!(
        (model
            .face_area(*model.faces().keys().next().unwrap())
            .unwrap()
            - 12.)
            .abs()
            < 1e-10
    );
    g = square();
    g.faces[0].material = Some(MaterialId(1));
    g.faces[1].material = Some(MaterialId(2));
    let model = g.build(true, true).unwrap();
    assert_eq!(model.faces().len(), 2);
    assert!(model.edges().values().all(|e| !e.soft));
}
#[test]
fn append_component_sharing_and_tags_restore() {
    let mut source = DocumentHistory::new();
    source
        .execute(Command::AddPolyline {
            points: vec![DVec3::ZERO, DVec3::X, DVec3::X + DVec3::Y, DVec3::Y],
            closed: true,
        })
        .unwrap();
    source
        .organization(OrganizationCommand::Create {
            kind: DefinitionKind::Component,
            name: "Shared".into(),
            vertices: source.model().vertices().keys().copied().collect(),
        })
        .unwrap();
    let definition = source
        .instances(DefinitionId(0))
        .unwrap()
        .values()
        .next()
        .unwrap()
        .definition;
    source
        .organization(OrganizationCommand::AddInstance {
            definition,
            name: "Second".into(),
            transform: glam::DMat4::from_translation(DVec3::new(2., 0., 0.)),
        })
        .unwrap();
    source
        .organization(OrganizationCommand::CreateTag("Imported tag".into()))
        .unwrap();
    let mut destination = DocumentHistory::new();
    destination.append_import(source.to_data()).unwrap();
    let imported = destination.to_data();
    assert_eq!(destination.undo_depth(), 1);
    assert_eq!(destination.instances(DefinitionId(0)).unwrap().len(), 2);
    let defs: std::collections::BTreeSet<_> = destination
        .instances(DefinitionId(0))
        .unwrap()
        .values()
        .map(|i| i.definition)
        .collect();
    assert_eq!(defs.len(), 1);
    assert_eq!(destination.tags().len(), 1);
    destination.undo().unwrap();
    assert!(destination.instances(DefinitionId(0)).unwrap().is_empty());
    assert!(destination.tags().is_empty());
    destination.redo().unwrap();
    assert_eq!(destination.to_data().definitions, imported.definitions);
    assert_eq!(destination.tags().len(), 1);
}
#[test]
fn explicit_mesh_edges_do_not_restore_cleaned_diagonals() {
    let mut g = square();
    g.edges.push(BulkEdge {
        smooth: false,
        vertices: [0, 2],
        ..Default::default()
    });
    let m = g.clone().build(true, true).unwrap();
    assert_eq!(m.faces().len(), 1);
    assert_eq!(m.edges().len(), 4);
    let m = g.build(false, true).unwrap();
    assert_eq!(m.edges().values().filter(|e| e.soft && e.smooth).count(), 1);
}
#[test]
fn loose_append_preserves_existing_ids_and_branch_allocation_floor() {
    let mut h = DocumentHistory::new();
    h.execute(Command::AddPolyline {
        points: vec![
            DVec3::new(10., 0., 0.),
            DVec3::new(11., 0., 0.),
            DVec3::new(11., 1., 0.),
            DVec3::new(10., 1., 0.),
        ],
        closed: true,
    })
    .unwrap();
    let before = h.model().to_data();
    h.execute(Command::AddEdge {
        start: DVec3::new(20., 0., 0.),
        end: DVec3::new(21., 0., 0.),
    })
    .unwrap();
    let used_floor = h.model().to_data().next_id;
    h.undo().unwrap();
    let mut incoming = DocumentHistory::new().to_data();
    incoming
        .definitions
        .get_mut(&DefinitionId(0))
        .unwrap()
        .geometry = square().build(true, true).unwrap().to_data();
    h.append_import(incoming).unwrap();
    for (id, v) in &before.vertices {
        assert_eq!(h.model().vertices().get(id), Some(v));
    }
    for (id, e) in &before.edges {
        assert_eq!(h.model().edges().get(id), Some(e));
    }
    for (id, f) in &before.faces {
        assert_eq!(h.model().faces().get(id), Some(f));
    }
    assert!(h
        .model()
        .vertices()
        .keys()
        .filter(|id| !before.vertices.contains_key(id))
        .all(|id| id.value() >= used_floor));
    h.undo().unwrap();
    assert_eq!(h.model().to_data(), before);
    h.redo().unwrap();
    assert_eq!(h.model().faces().len(), 2);
}

#[test]
fn imported_edge_flags_remain_independent_through_geometry_roundtrip() {
    let mut geometry = square();
    geometry.edges.push(BulkEdge {
        vertices: [0, 1],
        soft: false,
        smooth: true,
        hidden: true,
    });
    let model = geometry.build(false, false).unwrap();
    let edge = model.edges().values().find(|edge| edge.hidden).unwrap();
    assert!(edge.smooth);
    assert!(!edge.soft);
    let data = model.to_data();
    let restored = Model::from_data(data).unwrap();
    assert_eq!(restored.edges(), model.edges());
}

#[test]
fn hidden_import_boundary_split_preserves_flags_and_undo() {
    let mut geometry = square();
    geometry.edges.push(BulkEdge {
        vertices: [0, 1],
        soft: false,
        smooth: true,
        hidden: true,
    });
    let mut data = DocumentHistory::new().to_data();
    data.definitions.get_mut(&DefinitionId(0)).unwrap().geometry =
        geometry.build(false, false).unwrap().to_data();
    let mut history = DocumentHistory::from_import_data(data).unwrap();
    let before = history.model().to_data();
    history
        .execute(Command::AddEdge {
            start: DVec3::new(0.5, 0., 0.),
            end: DVec3::new(0.5, -1., 0.),
        })
        .unwrap();
    assert_eq!(
        history
            .model()
            .edges()
            .values()
            .filter(|edge| edge.hidden && edge.smooth && !edge.soft)
            .count(),
        2
    );
    history.undo().unwrap();
    assert_eq!(history.model().to_data(), before);
}

fn grid(
    w: usize,
    h: usize,
    skip: &[(usize, usize)],
    material: impl Fn((usize, usize)) -> Option<MaterialId>,
) -> BulkGeometry {
    let mut g = BulkGeometry::default();
    let idx = |x: usize, y: usize| y * (w + 1) + x;
    for y in 0..=h {
        for x in 0..=w {
            g.vertices.push(DVec3::new(x as f64, y as f64, 0.));
        }
    }
    for y in 0..h {
        for x in 0..w {
            if skip.contains(&(x, y)) {
                continue;
            }
            let m = material((x, y));
            for outer in [
                [idx(x, y), idx(x + 1, y), idx(x + 1, y + 1)],
                [idx(x, y), idx(x + 1, y + 1), idx(x, y + 1)],
            ] {
                g.faces.push(BulkFace {
                    outer: outer.to_vec(),
                    material: m,
                    ..Default::default()
                });
            }
        }
    }
    g
}
fn total_area(m: &Model) -> f64 {
    m.faces().keys().map(|id| m.face_area(*id).unwrap()).sum()
}
#[test]
fn flat_wall_of_2000_triangles_merges_to_one_face() {
    let g = grid(40, 25, &[], |_| None);
    assert_eq!(g.faces.len(), 2000);
    let (m, stats) = g.build_cancellable_with_stats(true, true, || false).unwrap();
    assert_eq!(m.faces().len(), 1);
    assert_eq!(stats.duplicates_removed, 0);
    assert_eq!(stats.region_fallbacks, 0);
    assert!((total_area(&m) - 1000.).abs() < 1e-9);
    m.validate().unwrap();
}
#[test]
fn wall_with_window_hole_keeps_hole() {
    let g = grid(3, 3, &[(1, 1)], |_| None);
    let m = g.build(true, true).unwrap();
    assert_eq!(m.faces().len(), 1);
    let face = m.faces().values().next().unwrap();
    assert_eq!(face.holes.len(), 1);
    assert!((total_area(&m) - 8.).abs() < 1e-9);
    m.validate().unwrap();
}
#[test]
fn different_material_regions_stay_separate() {
    let g = grid(4, 2, &[], |(x, _)| Some(MaterialId(if x < 2 { 1 } else { 2 })));
    let m = g.build(true, true).unwrap();
    assert_eq!(m.faces().len(), 2);
    m.validate().unwrap();
}
#[test]
fn curved_surface_is_not_flattened() {
    // Steep bend: neighboring planes differ far beyond the coplanarity tolerance,
    // so every triangle survives.
    let mut g = BulkGeometry::default();
    let mut angle = 0f64;
    let mut base = DVec3::ZERO;
    for _ in 0..20 {
        let dir = DVec3::new(angle.cos(), 0., angle.sin());
        let next = base + dir;
        let top = g.vertices.len();
        g.vertices.extend([base, next, next + DVec3::Y, base + DVec3::Y]);
        g.faces.push(BulkFace {
            outer: vec![top, top + 1, top + 2],
            ..Default::default()
        });
        g.faces.push(BulkFace {
            outer: vec![top, top + 2, top + 3],
            ..Default::default()
        });
        base = next;
        angle += 10f64.to_radians();
    }
    // Each segment's own triangle pair is exactly coplanar and merges into a quad;
    // the 10-degree joints never merge, so the bend is not flattened.
    let m = g.build(true, true).unwrap();
    assert_eq!(m.faces().len(), 20);
    m.validate().unwrap();
}
#[test]
fn locally_coplanar_curve_falls_back_to_pairwise() {
    // Bend of 1e-8 radians per unit segment: pairs stay within the tolerance and
    // merge, but the region as a whole is not planar, so it falls back to the
    // old pairwise result (quads) instead of being flattened.
    let mut g = BulkGeometry::default();
    let mut angle = 0f64;
    let mut base = DVec3::ZERO;
    for _ in 0..2000 {
        let next = base + DVec3::new(angle.cos(), 0., angle.sin());
        let top = g.vertices.len();
        g.vertices.extend([base, next, next + DVec3::Y, base + DVec3::Y]);
        g.faces.push(BulkFace {
            outer: vec![top, top + 1, top + 2],
            ..Default::default()
        });
        g.faces.push(BulkFace {
            outer: vec![top, top + 2, top + 3],
            ..Default::default()
        });
        base = next;
        angle += 1e-8;
    }
    let (m, stats) = g
        .clone()
        .build_cancellable_with_stats(true, true, || false)
        .unwrap();
    assert_eq!(stats.region_fallbacks, 1);
    assert_eq!(m.faces().len(), 2000);
    let (flat, _) = g
        .build_cancellable_with_stats(false, false, || false)
        .unwrap();
    assert!((total_area(&m) - total_area(&flat)).abs() < 1e-6);
    m.validate().unwrap();
}
#[test]
fn exact_duplicate_removed_opposite_winding_kept() {
    let mut g = grid(1, 1, &[], |_| None);
    g.vertices.extend([
        DVec3::new(5., 0., 0.),
        DVec3::new(6., 0., 0.),
        DVec3::new(6., 1., 0.),
    ]);
    // Exact duplicate of the grid's first triangle [0, 1, 3].
    g.faces.push(BulkFace {
        outer: vec![0, 1, 3],
        ..Default::default()
    });
    // Same vertex set, opposite windings: an intentional double-sided surface.
    g.faces.push(BulkFace {
        outer: vec![4, 5, 6],
        ..Default::default()
    });
    g.faces.push(BulkFace {
        outer: vec![6, 5, 4],
        ..Default::default()
    });
    let (m, stats) = g.build_cancellable_with_stats(true, false, || false).unwrap();
    assert_eq!(stats.duplicates_removed, 1);
    // The grid quad plus both sides of the double-sided triangle.
    assert_eq!(m.faces().len(), 3);
    m.validate().unwrap();
}
#[test]
fn oversized_region_splits_deterministically() {
    // 1x2100 cells: 4200 coplanar triangles whose outer boundary (4202 corners)
    // cannot fit one bounded face, so the region splits at the BFS midpoint.
    let g = grid(1, 2100, &[], |_| None);
    let (m, stats) = g.clone().build_cancellable_with_stats(true, true, || false).unwrap();
    assert_eq!(m.faces().len(), 2);
    assert_eq!(stats.region_fallbacks, 0);
    assert!((total_area(&m) - 2100.).abs() < 1e-9);
    m.validate().unwrap();
    let again = g.build(true, true).unwrap();
    assert_eq!(again.to_data(), m.to_data());
}
#[test]
fn cancellation_during_region_merge() {
    let g = grid(40, 25, &[], |_| None);
    let calls = std::sync::atomic::AtomicUsize::new(0);
    // Welding (1066 vertices) and face construction (2000 faces) check four times
    // at 1024-item intervals; the fifth check runs inside the region merge.
    let (model, error) = {
        let mut model = None;
        let mut error = None;
        let result = g.clone().build_cancellable_with_stats(true, true, || {
            calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) >= 5
        });
        match result {
            Ok(m) => model = Some(m),
            Err(e) => error = Some(e),
        }
        (model, error)
    };
    assert!(model.is_none());
    let error = error.unwrap();
    assert!(error.to_string().contains("cancelled"));
}
#[test]
fn region_area_equivalence_and_split_preserve_surface_area() {
    for g in [
        grid(40, 25, &[], |_| None),
        grid(3, 3, &[(1, 1)], |_| None),
        grid(1, 2100, &[], |_| None),
        grid(4, 2, &[], |(x, _)| Some(MaterialId(if x < 2 { 1 } else { 2 }))),
    ] {
        let (merged, _) = g
            .clone()
            .build_cancellable_with_stats(true, false, || false)
            .unwrap();
        let (triangles, _) = g
            .build_cancellable_with_stats(false, false, || false)
            .unwrap();
        let before = total_area(&triangles);
        let after = total_area(&merged);
        assert!(before > 0.);
        assert!(
            (before - after).abs() <= 1e-9 * before.max(1.),
            "surface area changed: {before} vs {after}"
        );
        merged.validate().unwrap();
    }
}
#[test]
fn malformed_meshes_never_panic() {
    let square_with_touching_hole = || {
        let mut g = grid(2, 2, &[], |_| None);
        // Hole corner welded onto the outer boundary (self-touching region).
        g.faces.push(BulkFace {
            outer: vec![0, 1, 5],
            holes: vec![vec![0, 5, 6]],
            ..Default::default()
        });
        g
    };
    let nonmanifold_edge = || {
        let mut g = grid(1, 1, &[], |_| None);
        g.vertices.push(DVec3::new(0.5, 0.5, 0.));
        // Third triangle on the diagonal edge.
        g.faces.push(BulkFace {
            outer: vec![0, 3, 4],
            ..Default::default()
        });
        g
    };
    let repeated_ring_vertex = || {
        let mut g = grid(1, 1, &[], |_| None);
        g.faces.push(BulkFace {
            outer: vec![0, 1, 2, 1],
            ..Default::default()
        });
        g
    };
    let triple_duplicate = || {
        let mut g = grid(1, 1, &[], |_| None);
        g.faces.push(g.faces[0].clone());
        g.faces.push(g.faces[0].clone());
        g
    };
    let duplicate_different_material = || {
        let mut g = grid(1, 1, &[], |_| None);
        g.faces.push(BulkFace {
            outer: vec![0, 1, 2],
            material: Some(MaterialId(7)),
            ..Default::default()
        });
        g
    };
    let collinear = || {
        let mut g = BulkGeometry::default();
        g.vertices = vec![DVec3::ZERO, DVec3::X, DVec3::X * 2.];
        g.faces.push(BulkFace {
            outer: vec![0, 1, 2],
            ..Default::default()
        });
        g
    };
    let welded_sliver = || {
        let mut g = BulkGeometry::default();
        g.vertices = vec![
            DVec3::ZERO,
            DVec3::X,
            DVec3::new(1., TOLERANCE / 4., 0.),
            DVec3::Y,
        ];
        g.faces.push(BulkFace {
            outer: vec![0, 1, 2],
            ..Default::default()
        });
        g.faces.push(BulkFace {
            outer: vec![0, 2, 3],
            ..Default::default()
        });
        g
    };
    let huge_coordinates = || {
        let mut g = BulkGeometry::default();
        g.vertices = vec![
            DVec3::ZERO,
            DVec3::X * 1e30,
            DVec3::new(1e30, 1e30, 0.),
        ];
        g.faces.push(BulkFace {
            outer: vec![0, 1, 2],
            ..Default::default()
        });
        g
    };
    let out_of_range_index = || {
        let mut g = grid(1, 1, &[], |_| None);
        g.faces[0].outer[1] = usize::MAX;
        g
    };
    let hole_filled_by_face = || {
        // One face with a square hole and another face exactly filling it.
        let mut g = BulkGeometry::default();
        for y in 0..3 {
            for x in 0..3 {
                g.vertices.push(DVec3::new(x as f64, y as f64, 0.));
            }
        }
        g.faces.push(BulkFace {
            outer: vec![0, 1, 2, 3, 4, 5, 6, 7, 8].iter().map(|i| *i).collect(),
            ..Default::default()
        });
        g.faces.clear();
        g.faces.push(BulkFace {
            outer: vec![0, 1, 4, 3],
            holes: vec![vec![4, 5, 8, 7]],
            ..Default::default()
        });
        g.faces.push(BulkFace {
            outer: vec![4, 5, 8, 7],
            ..Default::default()
        });
        g
    };
    let meshes = [
        square_with_touching_hole(),
        nonmanifold_edge(),
        repeated_ring_vertex(),
        triple_duplicate(),
        duplicate_different_material(),
        collinear(),
        welded_sliver(),
        huge_coordinates(),
        out_of_range_index(),
        hole_filled_by_face(),
    ];
    for g in &meshes {
        for merge in [true, false] {
            for soften in [true, false] {
                // Must neither panic nor hang; any error is a valid rejection.
                // Overlapping hostile geometry may keep its overlaps (imports
                // preserve them), so only the build's own mesh validation applies.
                let _ = g.clone().build(merge, soften);
            }
        }
    }
}
