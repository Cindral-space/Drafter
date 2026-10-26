//! Authored import/drawing comparisons and reversible Push/Pull regressions.
use cindral_core::*;
use cindral_io::*;

fn cube(hole: bool) -> BulkGeometry {
    let mut g = BulkGeometry {
        vertices: vec![
            DVec3::new(0., 0., 0.),
            DVec3::new(1., 0., 0.),
            DVec3::new(1., 1., 0.),
            DVec3::new(0., 1., 0.),
            DVec3::new(0., 0., 1.),
            DVec3::new(1., 0., 1.),
            DVec3::new(1., 1., 1.),
            DVec3::new(0., 1., 1.),
        ],
        faces: [
            [0, 3, 2, 1],
            [4, 5, 6, 7],
            [0, 1, 5, 4],
            [1, 2, 6, 5],
            [2, 3, 7, 6],
            [3, 0, 4, 7],
        ]
        .into_iter()
        .map(|outer| BulkFace {
            outer: outer.to_vec(),
            ..Default::default()
        })
        .collect(),
        ..Default::default()
    };
    if hole {
        g.vertices.extend([
            DVec3::new(0.25, 0.25, 1.),
            DVec3::new(0.25, 0.75, 1.),
            DVec3::new(0.75, 0.75, 1.),
            DVec3::new(0.75, 0.25, 1.),
        ]);
        g.faces[1].holes.push(vec![8, 9, 10, 11]);
    }
    g
}
fn panel() -> BulkGeometry {
    let mut g = cube(true);
    g.faces.push(BulkFace {
        outer: vec![11, 10, 9, 8],
        ..Default::default()
    });
    g
}
fn direct(g: BulkGeometry) -> DocumentHistory {
    let mut d = DocumentHistory::new().to_data();
    d.definitions.get_mut(&DefinitionId(0)).unwrap().geometry =
        g.build(false, false).unwrap().to_data();
    DocumentHistory::from_data(d).unwrap()
}
fn source(g: BulkGeometry) -> ImportedScene {
    ImportedScene {
        root_definition: Some(0),
        definitions: vec![ImportedDefinition {
            name: "Model".into(),
            geometry: g,
            ..Default::default()
        }],
        roots: vec![ImportedInstance::default()],
        ..Default::default()
    }
}
fn imported(scene: ImportedScene) -> DocumentHistory {
    convert_scene(
        scene,
        &ImportOptions {
            as_component: false,
            merge_coplanar: false,
            soften_coplanar: false,
        },
        &ImportControl::default(),
    )
    .unwrap()
    .history
}
fn top(h: &DocumentHistory, inner: bool) -> FaceId {
    h.model()
        .faces()
        .iter()
        .find(|(id, f)| {
            f.plane.normal.z > 0.9
                && (f.plane.distance - 1.).abs() < TOLERANCE
                && (!inner || h.model().face_area(**id).unwrap() < 0.3)
        })
        .map(|(id, _)| *id)
        .expect("picked top face must be in active context")
}
fn same_geometry(a: &DocumentData, b: &DocumentData) {
    assert_eq!(
        a.definitions.keys().collect::<Vec<_>>(),
        b.definitions.keys().collect::<Vec<_>>()
    );
    for (id, a) in &a.definitions {
        let b = &b.definitions[id];
        assert_eq!(a.geometry.vertices, b.geometry.vertices);
        assert_eq!(a.geometry.edges, b.geometry.edges);
        assert_eq!(a.geometry.faces, b.geometry.faces);
        assert_eq!(a.instances, b.instances);
    }
}
fn edit(h: &mut DocumentHistory, distance: f64, inner: bool) {
    let before = h.to_data();
    let face = top(h, inner);
    assert!(h.execute(Command::PushPull { face, distance }).unwrap());
    h.model().validate().unwrap();
    let after = h.to_data();
    assert_ne!(before.definitions, after.definitions);
    assert!(h.undo().unwrap());
    same_geometry(&h.to_data(), &before);
    assert!(h.redo().unwrap());
    same_geometry(&h.to_data(), &after);
}
#[test]
fn bulk_and_imported_cube_out_in_and_hole_face() {
    for build in [direct, |g| imported(source(g))] {
        for distance in [0.2, -0.2] {
            let mut h = build(cube(false));
            edit(&mut h, distance, false);
            assert!((h.model().bounds().unwrap().1.z - (1. + distance)).abs() < TOLERANCE);
        }
        edit(&mut build(cube(true)), 0.2, false);
    }
}
#[test]
fn bulk_and_imported_pocket_and_through_hole() {
    for build in [direct, |g| imported(source(g))] {
        let mut h = build(panel());
        edit(&mut h, -0.25, true);
        assert!(h
            .model()
            .faces()
            .values()
            .any(|f| (f.plane.normal.z.abs() - 1.).abs() < TOLERANCE
                && (f.plane.distance.abs() - 0.75).abs() < TOLERANCE));
        assert!((h.model().bounds().unwrap().1.z - 1.).abs() < TOLERANCE);
        let mut h = build(panel());
        edit(&mut h, -1.2, true);
        assert_eq!(
            h.model()
                .faces()
                .values()
                .filter(|f| !f.holes.is_empty())
                .count(),
            2,
            "top and bottom must both be pierced"
        );
    }
}
#[test]
fn entered_shared_component_out_pocket_through_and_hole_face() {
    for (g, distance, inner) in [
        (cube(false), 0.2, false),
        (panel(), -0.25, true),
        (panel(), -1.2, true),
        (cube(true), 0.2, false),
    ] {
        let mut scene = source(cube(false));
        scene.definitions.push(ImportedDefinition {
            geometry: g,
            ..Default::default()
        });
        scene.definitions[0].instances = [2., 4.]
            .into_iter()
            .map(|x| ImportedInstance {
                definition: 1,
                transform: glam::DMat4::from_translation(DVec3::X * x),
                ..Default::default()
            })
            .collect();
        let mut h = imported(scene);
        let root = h.model().to_data();
        let occurrences = h.occurrences().unwrap();
        let children = occurrences
            .iter()
            .filter(|o| !o.path.is_empty())
            .collect::<Vec<_>>();
        assert_eq!(children.len(), 2);
        assert_eq!(children[0].definition, children[1].definition);
        let definition = children[0].definition;
        h.enter(children[0].path.clone()).unwrap();
        edit(&mut h, distance, inner);
        let edited = h.model().to_data();
        assert_eq!(h.definition_model(DefinitionId(0)).unwrap().to_data(), root);
        h.enter(children[1].path.clone()).unwrap();
        assert_eq!(h.model().to_data(), edited);
        assert_eq!(h.active_definition(), definition);
    }
}
#[test]
fn tolerance_welding_and_reversed_winding_preserve_closed_adjacency() {
    let mut g = cube(false);
    let positions = g.vertices.clone();
    g.vertices.clear();
    for (i, face) in g.faces.iter_mut().enumerate() {
        for v in &mut face.outer {
            let p = positions[*v] + DVec3::splat(if i % 2 == 0 { TOLERANCE * 0.1 } else { 0. });
            *v = g.vertices.len();
            g.vertices.push(p);
        }
        if i % 2 == 0 {
            face.outer.reverse();
        }
    }
    let mut h = imported(source(g));
    assert_eq!(h.model().vertices().len(), 8);
    assert_eq!(h.model().edges().len(), 12);
    for edge in h.model().edges().values() {
        let incident = h
            .model()
            .faces()
            .values()
            .filter(|f| {
                f.outer
                    .iter()
                    .zip(f.outer.iter().cycle().skip(1))
                    .any(|(a, b)| edge.vertices.contains(a) && edge.vertices.contains(b))
            })
            .count();
        assert_eq!(incident, 2);
    }
    // Pick the geometrically highest horizontal face; its authored normal may be reversed.
    let face = *h
        .model()
        .faces()
        .iter()
        .find(|(_, f)| {
            f.outer
                .iter()
                .all(|v| h.model().vertices()[v].position.z > 0.9)
        })
        .unwrap()
        .0;
    let distance = h.model().faces()[&face].plane.normal.z.signum() * 0.2;
    assert!(h.execute(Command::PushPull { face, distance }).unwrap());
    h.model().validate().unwrap();
}
#[test]
fn rectangle_tool_cube_matches_imported_shape_and_edits() {
    let mut h = DocumentHistory::new();
    let mut rectangle = cindral_tools::ToolSession::new(cindral_tools::ToolKind::Rectangle);
    rectangle.click(DVec3::ZERO).unwrap();
    h.execute(rectangle.click(DVec3::new(1., 1., 0.)).unwrap().unwrap())
        .unwrap();
    let face = *h.model().faces().keys().next().unwrap();
    h.execute(Command::PushPull { face, distance: 1. }).unwrap();
    let bulk = imported(source(cube(false)));
    assert_eq!(h.model().bounds(), bulk.model().bounds());
    assert_eq!(h.model().vertices().len(), bulk.model().vertices().len());
    assert_eq!(h.model().edges().len(), bulk.model().edges().len());
    assert_eq!(h.model().faces().len(), bulk.model().faces().len());
    edit(&mut h, 0.2, false);
}
#[test]
#[ignore = "release-only imported cube operation timing"]
#[allow(clippy::assertions_on_constants)]
fn imported_cube_release_timing() {
    assert!(!cfg!(debug_assertions), "use --release");
    let mut h = imported(source(cube(false)));
    let face = top(&h, false);
    let start = std::time::Instant::now();
    assert!(h
        .execute(Command::PushPull {
            face,
            distance: 0.2
        })
        .unwrap());
    eprintln!(
        "Imported cube Push/Pull: {:.3}ms",
        start.elapsed().as_secs_f64() * 1000.
    );
}
#[test]
fn drawn_rectangle_panel_matches_bulk_hole_face_and_pocket() {
    let mut drawn = DocumentHistory::new();
    let mut rectangle = cindral_tools::ToolSession::new(cindral_tools::ToolKind::Rectangle);
    rectangle.click(DVec3::ZERO).unwrap();
    drawn
        .execute(rectangle.click(DVec3::new(1., 1., 0.)).unwrap().unwrap())
        .unwrap();
    let face = *drawn.model().faces().keys().next().unwrap();
    drawn
        .execute(Command::PushPull { face, distance: 1. })
        .unwrap();
    rectangle
        .set_plane(cindral_tools::DrawingPlane {
            origin: DVec3::Z,
            normal: DVec3::Z,
        })
        .unwrap();
    rectangle.click(DVec3::new(0.25, 0.25, 1.)).unwrap();
    drawn
        .execute(
            rectangle
                .click(DVec3::new(0.75, 0.75, 1.))
                .unwrap()
                .unwrap(),
        )
        .unwrap();
    let mut bulk = imported(source(panel()));
    let areas = |h: &DocumentHistory| {
        let mut areas = h
            .model()
            .faces()
            .keys()
            .map(|id| (h.model().face_area(*id).unwrap() * 1e8).round() as i64)
            .collect::<Vec<_>>();
        areas.sort();
        areas
    };
    assert_eq!(areas(&drawn), areas(&bulk));
    assert_eq!(
        drawn.model().vertices().len(),
        bulk.model().vertices().len()
    );
    assert_eq!(drawn.model().edges().len(), bulk.model().edges().len());
    for h in [&mut drawn, &mut bulk] {
        edit(h, -0.25, true);
    }
    assert_eq!(areas(&drawn), areas(&bulk));
}
#[test]
fn source_root_marker_cannot_explode_or_drop_an_actual_placement() {
    let mut scene = source(cube(false));
    scene.roots[0].transform = glam::DMat4::from_translation(DVec3::X);
    let error = convert_scene(scene, &ImportOptions::default(), &ImportControl::default())
        .err()
        .unwrap();
    assert!(error.to_string().contains("invalid source root container"));
}

#[test]
fn sixty_imported_and_drawn_previews_preserve_equivalent_geometry_and_history() {
    let mut drawn = DocumentHistory::new();
    let mut rectangle = cindral_tools::ToolSession::new(cindral_tools::ToolKind::Rectangle);
    rectangle.click(DVec3::ZERO).unwrap();
    drawn
        .execute(rectangle.click(DVec3::new(1., 1., 0.)).unwrap().unwrap())
        .unwrap();
    let face = *drawn.model().faces().keys().next().unwrap();
    drawn
        .execute(Command::PushPull { face, distance: 1. })
        .unwrap();
    let mut imported = imported(source(cube(false)));
    // Compare exact boundary coordinates, independent of allocation IDs and winding.
    let signature = |h: &DocumentHistory| {
        let mut faces = h
            .model()
            .faces()
            .values()
            .map(|f| {
                let mut points = f
                    .outer
                    .iter()
                    .map(|id| {
                        h.model().vertices()[id]
                            .position
                            .to_array()
                            .map(|p| (p * 1e8).round() as i64)
                    })
                    .collect::<Vec<_>>();
                points.sort();
                points
            })
            .collect::<Vec<_>>();
        faces.sort();
        faces
    };
    let original = signature(&drawn);
    assert_eq!(signature(&imported), original);
    let faces = [top(&drawn, false), top(&imported, false)];
    let depths = [drawn.undo_depth(), imported.undo_depth()];
    for i in 1..=60 {
        let distance = i as f64 * 0.002 * if i % 2 == 0 { 1. } else { -1. };
        for (h, face) in [(&mut drawn, faces[0]), (&mut imported, faces[1])] {
            h.preview(Command::PushPull { face, distance }).unwrap();
        }
        assert_eq!(signature(&drawn), signature(&imported));
    }
    let final_shape = signature(&drawn);
    for (i, h) in [&mut drawn, &mut imported].into_iter().enumerate() {
        h.commit_preview().unwrap();
        assert_eq!(h.undo_depth(), depths[i] + 1);
        h.model().validate().unwrap();
        h.undo().unwrap();
        assert_eq!(signature(h), original);
        h.redo().unwrap();
        assert_eq!(signature(h), final_shape);
        let face = faces[i];
        h.preview(Command::PushPull {
            face,
            distance: 0.05,
        })
        .unwrap();
        h.cancel_preview().unwrap();
        assert_eq!(signature(h), final_shape);
    }
}

#[test]
fn imported_closed_box_extension_heals_old_perimeter() {
    let mut h = imported(source(cube(false)));
    let face = top(&h, false);
    h.execute(Command::PushPull {
        face,
        distance: 0.2,
    })
    .unwrap();
    h.model().validate().unwrap();
    assert_eq!(
        (
            h.model().vertices().len(),
            h.model().edges().len(),
            h.model().faces().len()
        ),
        (8, 12, 6)
    );
    assert!(!h.model().edges().values().any(|e| {
        e.vertices
            .iter()
            .all(|v| (h.model().vertices()[v].position.z - 1.).abs() < TOLERANCE)
    }));
}

#[test]
fn imported_attached_open_surfaces_replace_base_in_both_edit_paths() {
    for folded in [false, true] {
        let g = BulkGeometry {
            vertices: vec![
                DVec3::ZERO,
                DVec3::X,
                DVec3::X + DVec3::Y,
                DVec3::Y,
                DVec3::new(2., 0., if folded { -1. } else { 0. }),
                DVec3::new(2., 1., if folded { -1. } else { 0. }),
            ],
            faces: [vec![0, 1, 2, 3], vec![1, 4, 5, 2]]
                .into_iter()
                .map(|outer| BulkFace {
                    outer,
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        };
        let mut h = imported(source(g));
        let face = *h
            .model()
            .faces()
            .iter()
            .find(|(_, f)| {
                f.outer
                    .iter()
                    .all(|v| h.model().vertices()[v].position.x <= 1.)
            })
            .unwrap()
            .0;
        let before = h.to_data();
        h.preview(Command::PushPull {
            face,
            distance: 0.2,
        })
        .unwrap();
        h.model().validate().unwrap();
        assert!(
            !h.model().faces().values().any(|f| f.outer.iter().all(|v| {
                let p = h.model().vertices()[v].position;
                p.x <= 1. && p.z.abs() < TOLERANCE
            })),
            "folded={folded}: starting face survived"
        );
        let after = h.to_data();
        h.cancel_preview().unwrap();
        same_geometry(&h.to_data(), &before);
        h.preview(Command::PushPull {
            face,
            distance: 0.2,
        })
        .unwrap();
        h.commit_preview().unwrap();
        same_geometry(&h.to_data(), &after);
        h.undo().unwrap();
        same_geometry(&h.to_data(), &before);
        h.redo().unwrap();
        same_geometry(&h.to_data(), &after);
    }
}

/// Opt-in check against a private file; never stages or copies its source data.
#[test]
fn private_skp_starting_faces_are_replaced_when_present() {
    let Some(path) = std::env::var_os("CINDRAL_SKP_CLEANUP_FILE") else {
        return;
    };
    let mut h = import_file(
        std::path::Path::new(&path),
        &ImportOptions {
            as_component: false,
            ..Default::default()
        },
        &ImportControl::default(),
    )
    .unwrap()
    .history;
    let mut definitions = std::collections::BTreeSet::new();
    let mut checked = 0;
    for occurrence in h.occurrences().unwrap() {
        if !definitions.insert(occurrence.definition) {
            continue;
        }
        h.enter(occurrence.path).unwrap();
        let Some(face) = h.model().faces().keys().copied().max_by(|a, b| {
            h.model()
                .face_area(*a)
                .unwrap()
                .total_cmp(&h.model().face_area(*b).unwrap())
        }) else {
            continue;
        };
        let positions = |m: &Model, f: &Face| {
            let mut points = f
                .outer
                .iter()
                .map(|v| m.vertices()[v].position.to_array().map(f64::to_bits))
                .collect::<Vec<_>>();
            points.sort();
            points
        };
        let original = positions(h.model(), &h.model().faces()[&face]);
        // Only attached source faces are subject to replacement; isolated
        // imported profiles intentionally keep a closure on first extrusion.
        let boundary = h.model().face_edges(face);
        let attached = h.model().faces().iter().any(|(other, _)| {
            *other != face
                && h.model()
                    .face_edges(*other)
                    .iter()
                    .any(|e| boundary.contains(e))
        });
        if !attached {
            continue;
        }
        let before = h.to_data();
        h.preview(Command::PushPull {
            face,
            distance: 0.05,
        })
        .unwrap();
        h.model().validate().unwrap();
        assert!(
            !h.model()
                .faces()
                .values()
                .any(|f| positions(h.model(), f) == original),
            "definition {:?}: original face remains",
            occurrence.definition
        );
        h.cancel_preview().unwrap();
        same_geometry(&h.to_data(), &before);
        checked += 1;
        if checked == 3 {
            break;
        }
    }
    assert!(checked > 0, "no attached faces exercised");
    eprintln!("private SKP cleanup: {checked} definitions exercised");
}
#[test]
fn push_pull_moves_merged_wall_as_one_face() {
    // A 4x2 grid of eight unit quads (16 triangles) merges into one face; Push/Pull
    // must move that whole face as one, like an authored polygon.
    let mut g = BulkGeometry::default();
    let idx = |x: usize, y: usize| y * 5 + x;
    for y in 0..=2 {
        for x in 0..=4 {
            g.vertices.push(DVec3::new(x as f64, y as f64, 0.));
        }
    }
    for y in 0..2 {
        for x in 0..4 {
            for outer in [
                [idx(x, y), idx(x + 1, y), idx(x + 1, y + 1)],
                [idx(x, y), idx(x + 1, y + 1), idx(x, y + 1)],
            ] {
                g.faces.push(BulkFace {
                    outer: outer.to_vec(),
                    ..Default::default()
                });
            }
        }
    }
    let mut h = convert_scene(
        source(g),
        &ImportOptions {
            as_component: false,
            merge_coplanar: true,
            soften_coplanar: true,
        },
        &ImportControl::default(),
    )
    .unwrap()
    .history;
    assert_eq!(h.model().faces().len(), 1);
    let (face, original) = h.model().faces().iter().next().unwrap();
    let (face, original) = (*face, original.clone());
    let area = h.model().face_area(face).unwrap();
    assert!(h.execute(Command::PushPull { face, distance: 1. }).unwrap());
    h.model().validate().unwrap();
    // One side wall per boundary edge (12-edge perimeter), the moved face, and
    // the retained base face of the loose surface.
    assert_eq!(h.model().faces().len(), 14);
    let (pulled, pulled_face) = h
        .model()
        .faces()
        .iter()
        .find(|(_, f)| f.plane.distance > 0.5 && f.plane.normal.z > 0.9)
        .expect("pulled face");
    assert_eq!(pulled_face.outer.len(), original.outer.len());
    assert_eq!(pulled_face.holes.len(), original.holes.len());
    assert!((h.model().face_area(*pulled).unwrap() - area).abs() < 1e-9);
    let before = h.to_data();
    assert!(h.undo().unwrap());
    assert_eq!(h.model().faces().len(), 1);
    assert_eq!(h.model().faces().values().next().unwrap(), &original);
    assert!(h.redo().unwrap());
    assert_eq!(h.to_data(), before);
}

fn assert_open_boundary_equivalence(a: &Model, b: &Model) {
    a.validate().expect("model a valid");
    b.validate().expect("model b valid");
    let area_a: f64 = a
        .faces()
        .keys()
        .map(|&id| a.face_area(id).unwrap_or(0.0))
        .sum();
    let area_b: f64 = b
        .faces()
        .keys()
        .map(|&id| b.face_area(id).unwrap_or(0.0))
        .sum();
    assert!(
        (area_a - area_b).abs() < TOLERANCE * area_a.max(1.0),
        "area mismatch: {area_a} vs {area_b}"
    );
    let (amin, amax) = a.bounds().unwrap();
    let (bmin, bmax) = b.bounds().unwrap();
    assert!(amin.distance(bmin) < TOLERANCE && amax.distance(bmax) < TOLERANCE);
}

fn assert_model_equivalence(fast: &Model, slow: &Model) {
    fast.validate().expect("fast validate");
    slow.validate().expect("slow validate");

    assert_eq!(
        fast.vertices().len(),
        slow.vertices().len(),
        "vertex count mismatch: fast {} vs slow {}",
        fast.vertices().len(),
        slow.vertices().len()
    );
    assert_eq!(
        fast.edges().len(),
        slow.edges().len(),
        "edge count mismatch: fast {} vs slow {}",
        fast.edges().len(),
        slow.edges().len()
    );
    assert_eq!(
        fast.faces().len(),
        slow.faces().len(),
        "face count mismatch: fast {} vs slow {}",
        fast.faces().len(),
        slow.faces().len()
    );

    let area_fast: f64 = fast
        .faces()
        .keys()
        .map(|&id| fast.face_area(id).unwrap_or(0.0))
        .sum();
    let area_slow: f64 = slow
        .faces()
        .keys()
        .map(|&id| slow.face_area(id).unwrap_or(0.0))
        .sum();
    assert!(
        (area_fast - area_slow).abs() < 1e-4,
        "total area mismatch: fast {area_fast} vs slow {area_slow}"
    );

    let mut areas_fast: Vec<f64> = fast
        .faces()
        .keys()
        .map(|&id| fast.face_area(id).unwrap_or(0.0))
        .collect();
    let mut areas_slow: Vec<f64> = slow
        .faces()
        .keys()
        .map(|&id| slow.face_area(id).unwrap_or(0.0))
        .collect();
    areas_fast.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    areas_slow.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    for (af, as_) in areas_fast.iter().zip(areas_slow.iter()) {
        assert!((af - as_).abs() < 1e-4, "face area mismatch: {af} vs {as_}");
    }

    let quantize_normal = |n: DVec3| -> (i64, i64, i64) {
        (
            (n.x * 1000.0).round() as i64,
            (n.y * 1000.0).round() as i64,
            (n.z * 1000.0).round() as i64,
        )
    };
    let mut normals_fast: Vec<_> = fast
        .faces()
        .values()
        .map(|f| quantize_normal(f.plane.normal))
        .collect();
    let mut normals_slow: Vec<_> = slow
        .faces()
        .values()
        .map(|f| quantize_normal(f.plane.normal))
        .collect();
    normals_fast.sort();
    normals_slow.sort();
    assert_eq!(normals_fast, normals_slow, "normals mismatch");

    let mut mats_fast: Vec<_> = fast
        .faces()
        .values()
        .map(|f| (f.material, f.color))
        .collect();
    let mut mats_slow: Vec<_> = slow
        .faces()
        .values()
        .map(|f| (f.material, f.color))
        .collect();
    mats_fast.sort();
    mats_slow.sort();
    assert_eq!(mats_fast, mats_slow, "materials/colors mismatch");
}

fn verify_history_pushpull_equivalence(initial: &DocumentHistory, face: FaceId, distance: f64) {
    let mut fast = initial.clone();
    let mut slow = initial.clone();

    let fast_changed = fast
        .execute(Command::PushPull { face, distance })
        .expect("fast execute");
    let slow_changed = cindral_core::benchmark::with_reference(|| {
        slow.execute(Command::PushPull { face, distance })
    })
    .expect("slow execute");
    assert_eq!(fast_changed, slow_changed, "changed return mismatch");

    assert_model_equivalence(fast.model(), slow.model());
    assert_eq!(fast.tags(), slow.tags(), "tags mismatch");

    if fast_changed {
        assert!(fast.undo().expect("fast undo"));
        assert!(slow.undo().expect("slow undo"));
        assert_model_equivalence(fast.model(), initial.model());
        assert_model_equivalence(slow.model(), initial.model());
        assert_eq!(fast.tags(), initial.tags());

        assert!(fast.redo().expect("fast redo"));
        assert!(slow.redo().expect("slow redo"));
        assert_model_equivalence(fast.model(), slow.model());
        assert_eq!(fast.tags(), slow.tags());
    }
}

#[test]
fn test_synthetic_faces_pushpull_equivalence() {
    let cube_solid = direct(cube(false));
    println!(
        "INITIAL cube_solid: vertices={}, faces={}, edges={}",
        cube_solid.model().vertices().len(),
        cube_solid.model().faces().len(),
        cube_solid.model().edges().len(),
    );
    let top_face = *cube_solid
        .model()
        .faces()
        .iter()
        .find(|(_, f)| (f.plane.normal - DVec3::Z).length() < 1e-4)
        .unwrap()
        .0;

    let mut fast = cube_solid.clone();
    let mut slow = cube_solid.clone();
    fast.execute(Command::PushPull {
        face: top_face,
        distance: -0.3,
    })
    .unwrap();
    cindral_core::benchmark::with_reference(|| {
        slow.execute(Command::PushPull {
            face: top_face,
            distance: -0.3,
        })
    })
    .unwrap();
    println!(
        "FAST -0.3: vertices={}, faces={}, edges={}",
        fast.model().vertices().len(),
        fast.model().faces().len(),
        fast.model().edges().len(),
    );
    println!(
        "SLOW -0.3: vertices={}, faces={}, edges={}",
        slow.model().vertices().len(),
        slow.model().faces().len(),
        slow.model().edges().len(),
    );
    for (v, pos) in fast.model().vertices() {
        println!("FAST v{:?}: {:?}", v, pos.position);
    }
    for (v, pos) in slow.model().vertices() {
        println!("SLOW v{:?}: {:?}", v, pos.position);
    }
    for (fid, f) in fast.model().faces() {
        println!(
            "FAST face {:?}: normal={:?}, outer={:?}",
            fid, f.plane.normal, f.outer
        );
    }
    for (fid, f) in slow.model().faces() {
        println!(
            "SLOW face {:?}: normal={:?}, outer={:?}",
            fid, f.plane.normal, f.outer
        );
    }

    // Outward push
    verify_history_pushpull_equivalence(&cube_solid, top_face, 0.5);
    // Negative depth push
    verify_history_pushpull_equivalence(&cube_solid, top_face, -0.3);
    // Zero depth push
    verify_history_pushpull_equivalence(&cube_solid, top_face, 0.0);

    // Test annulus in free space (no coplanar merging side walls)
    fn create_annulus() -> DocumentHistory {
        let g = BulkGeometry {
            vertices: vec![
                DVec3::new(0., 0., 0.),
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
        direct(g)
    }

    let annulus = create_annulus();
    let annulus_face = *annulus.model().faces().keys().next().unwrap();
    verify_history_pushpull_equivalence(&annulus, annulus_face, 0.5);
    verify_history_pushpull_equivalence(&annulus, annulus_face, -0.3);
    verify_history_pushpull_equivalence(&annulus, annulus_face, 0.0);
}

fn verify_mybedroom_face_equivalence(initial: &DocumentHistory, face: FaceId, distance: f64) {
    let mut fast = initial.clone();
    let mut slow = initial.clone();

    let fast_changed = fast
        .execute(Command::PushPull { face, distance })
        .expect("fast execute");
    let slow_changed = cindral_core::benchmark::with_reference(|| {
        slow.execute(Command::PushPull { face, distance })
    })
    .expect("slow execute");
    assert_eq!(fast_changed, slow_changed, "changed return mismatch");

    assert_open_boundary_equivalence(fast.model(), slow.model());
    assert_eq!(fast.tags(), slow.tags(), "tags mismatch");

    if fast_changed {
        assert!(fast.undo().expect("fast undo"));
        assert!(slow.undo().expect("slow undo"));
        assert_open_boundary_equivalence(fast.model(), initial.model());
        assert_eq!(fast.tags(), initial.tags());

        assert!(fast.redo().expect("fast redo"));
        assert!(slow.redo().expect("slow redo"));
        assert_open_boundary_equivalence(fast.model(), slow.model());
        assert_eq!(fast.tags(), slow.tags());
    }
}

#[test]
fn test_mybedroom_real_faces_pushpull_equivalence() {
    let path = std::path::Path::new("/home/earl/Desktop/Cindral/Drafter/SKP/mybedroom.skp");
    if !path.exists() {
        return;
    }
    let h = import_file(
        path,
        &ImportOptions {
            as_component: false,
            ..Default::default()
        },
        &ImportControl::default(),
    )
    .unwrap()
    .history;

    // Item 1: planar face with no hole (Face 900)
    let face_900 = FaceId::from_raw(900);
    println!("Face 900 0.05");
    verify_mybedroom_face_equivalence(&h, face_900, 0.05);
    println!("Face 900 -0.05");
    verify_mybedroom_face_equivalence(&h, face_900, -0.05);
    println!("Face 900 0.0");
    verify_mybedroom_face_equivalence(&h, face_900, 0.0);

    // Item 2: face with inner ring / cavity (Face 905)
    let face_905 = FaceId::from_raw(905);
    println!("Face 905 plane: {:?}", h.model().faces()[&face_905].plane);
    println!("Face 905 0.05");
    verify_mybedroom_face_equivalence(&h, face_905, 0.05);
    verify_mybedroom_face_equivalence(&h, face_905, -0.05);
    verify_mybedroom_face_equivalence(&h, face_905, 0.0);

    // Additional real faces from mybedroom
    for &raw_id in &[348, 364, 390] {
        let fid = FaceId::from_raw(raw_id);
        if h.model().faces().contains_key(&fid) {
            verify_mybedroom_face_equivalence(&h, fid, 0.05);
            verify_mybedroom_face_equivalence(&h, fid, -0.05);
            verify_mybedroom_face_equivalence(&h, fid, 0.0);
        }
    }
}
