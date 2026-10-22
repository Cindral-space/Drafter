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
