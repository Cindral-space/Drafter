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
