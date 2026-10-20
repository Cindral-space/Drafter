//! Reproducible native viewport capture for manual GPU validation.
//!
//! Run `cargo run -p cindral-app --example capture_viewport -- capture.ppm`.

use std::io::Write;

use eframe::egui;

struct Capture {
    app: cindral_app::DrafterApp,
    output: std::path::PathBuf,
    frames: usize,
    incremental: bool,
    styles: bool,
    modify_face: Option<cindral_core::FaceId>,
}

impl eframe::App for Capture {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        if self.incremental && self.frames == 1 {
            if let Err(error) = seed(&mut self.app) {
                eprintln!("Incremental capture failed: {error}");
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                return;
            }
        }
        if let Some(face) = self.modify_face {
            let result = match self.frames {
                1 => self
                    .app
                    .preview(cindral_core::Command::PushPull { face, distance: 1. }),
                2 => self.app.cancel_preview(),
                3 => self.app.preview(cindral_core::Command::PushPull {
                    face,
                    distance: -3.,
                }),
                4 => self.app.commit_preview(),
                _ => Ok(false),
            };
            if let Err(e) = result {
                eprintln!("Modify capture failed: {e}");
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                return;
            }
        }
        if self.modify_face.is_some() && self.frames == 4 {
            if let Some(face) = self.app.model().faces().keys().next().copied() {
                self.app.select_connected(cindral_tools::Hit::Face(face));
            }
        }
        if self.styles {
            let modes = [cindral_core::StyleMode::Shaded, cindral_core::StyleMode::Wireframe, cindral_core::StyleMode::HiddenLine, cindral_core::StyleMode::XRay, cindral_core::StyleMode::Textured];
            if let Some(mode) = modes.get(self.frames) {
                self.app.set_view_style(cindral_core::ViewStyle { mode: *mode, profiles: true, soft_edges: true, smooth_edges: true, sky: Some([68, 96, 125]), ground: Some([47, 54, 43]), ..Default::default() });
            }
        }
        self.app.update(ctx, frame);
        self.frames += 1;
        if self.frames == if self.styles { 7 } else if self.modify_face.is_some() { 6 } else { 3 } {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
        }
        let capture = ctx.input(|input| {
            input.events.iter().find_map(|event| {
                if let egui::Event::Screenshot { image, .. } = event {
                    Some(image.clone())
                } else {
                    None
                }
            })
        });
        if let Some(image) = capture {
            match save_ppm(&self.output, &image) {
                Ok(()) => println!("Captured {}", self.output.display()),
                Err(error) => eprintln!("Capture failed: {error}"),
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        if self.frames > 300 {
            eprintln!("Capture timed out waiting for GPU readback");
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        ctx.request_repaint();
    }
}

fn save_ppm(path: &std::path::Path, image: &egui::ColorImage) -> std::io::Result<()> {
    let mut file = std::io::BufWriter::new(std::fs::File::create(path)?);
    write!(file, "P6\n{} {}\n255\n", image.width(), image.height())?;
    for pixel in &image.pixels {
        file.write_all(&pixel.to_array()[..3])?;
    }
    file.flush()
}

fn main() -> eframe::Result {
    let output = std::env::args_os()
        .nth(1)
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from("capture.ppm"));
    let options = eframe::NativeOptions {
        renderer: eframe::Renderer::Wgpu,
        multisampling: 4,
        viewport: egui::ViewportBuilder::default().with_inner_size([1280.0, 800.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Cindral Drafter · viewport validation",
        options,
        Box::new(move |cc| {
            let mut app = cindral_app::DrafterApp::new(cc)?;
            let incremental = std::env::args().any(|arg| arg == "--incremental");
            let modify_face = if std::env::args().any(|arg| arg == "--modify") {
                Some(seed_modify(&mut app)?)
            } else {
                None
            };
            if let Some(path) = std::env::var_os("CINDRAL_CAPTURE_DRAFT") {
                app.open_path(std::path::Path::new(&path))?;
            } else if std::env::args().any(|arg| arg == "--organization") {
                seed_organization(&mut app)?;
            } else if incremental {
                let mut tool = cindral_tools::ToolSession::new(cindral_tools::ToolKind::Rectangle);
                tool.click(cindral_core::DVec3::new(-3., -2., 0.))?;
                if let Some(command) = tool.measure("4m,3m")? {
                    app.execute(command)?;
                }
            } else if std::env::args().any(|arg| arg == "--demo") {
                seed(&mut app)?;
            }
            Ok(Box::new(Capture {
                app,
                output,
                frames: 0,
                incremental,
                styles: std::env::args().any(|arg| arg == "--styles"),
                modify_face,
            }))
        }),
    )
}

fn seed(app: &mut cindral_app::DrafterApp) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    use cindral_core::DVec3 as V;
    use cindral_tools::{ToolKind, ToolSession};
    for (kind, center, measure) in [
        (ToolKind::Rectangle, V::new(-3., -2., 0.), "4m,3m"),
        (ToolKind::Circle, V::new(3., 0., 0.), "1.2m"),
        (ToolKind::Polygon, V::new(0., 3., 0.), "1m"),
    ] {
        let mut tool = ToolSession::new(kind);
        tool.click(center)?;
        if let Some(command) = tool.measure(measure)? {
            app.execute(command)?;
        }
    }
    let mut tool = ToolSession::new(ToolKind::Arc);
    tool.click(V::new(-3., 3., 0.))?;
    tool.click(V::new(-2., 3., 0.))?;
    if let Some(command) = tool.measure("130deg")? {
        app.execute(command)?;
    }
    Ok(())
}

fn seed_modify(
    app: &mut cindral_app::DrafterApp,
) -> Result<cindral_core::FaceId, Box<dyn std::error::Error + Send + Sync>> {
    use cindral_core::{Command, DVec3 as V};
    app.execute(Command::AddPolyline {
        points: vec![
            V::new(-2., -2., 0.),
            V::new(2., -2., 0.),
            V::new(2., 2., 0.),
            V::new(-2., 2., 0.),
        ],
        closed: true,
    })?;
    let face = *app.model().faces().keys().next().ok_or("missing base")?;
    app.execute(Command::PushPull { face, distance: 2. })?;
    app.execute(Command::AddPolyline {
        points: vec![
            V::new(-0.7, -0.7, 2.),
            V::new(0.7, -0.7, 2.),
            V::new(0.7, 0.7, 2.),
            V::new(-0.7, 0.7, 2.),
        ],
        closed: true,
    })?;
    let face = *app
        .model()
        .faces()
        .iter()
        .find(|(_, f)| {
            f.plane.normal.z > 0.9 && (f.plane.distance - 2.).abs() < 1e-7 && f.holes.is_empty()
        })
        .ok_or("missing inset")?
        .0;
    let faces = app.model().faces().keys().copied().collect();
    app.execute(Command::PaintFace {
        faces,
        color: [195, 130, 75, 255],
    })?;
    app.select_connected(cindral_tools::Hit::Face(face));
    Ok(face)
}

fn seed_organization(
    app: &mut cindral_app::DrafterApp,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    use cindral_core::{Command, DVec3 as V, DefinitionKind, Material, OrganizationCommand as Org};
    use glam::DMat4;
    app.execute(Command::AddPolyline {
        points: vec![
            V::new(-1., -1., 0.),
            V::new(1., -1., 0.),
            V::new(1., 1., 0.),
            V::new(-1., 1., 0.),
        ],
        closed: true,
    })?;
    let face = *app.model().faces().keys().next().ok_or("missing face")?;
    app.execute(Command::PushPull { face, distance: 2. })?;
    app.select_connected(cindral_tools::Hit::Face(face));
    let first = app.create_container(DefinitionKind::Component)?;
    let definition = app
        .document()
        .instance(&first)
        .ok_or("missing component")?
        .definition;
    app.organize(Org::Rename {
        path: first.clone(),
        name: "Shared block".into(),
    })?;
    app.organize(Org::CreateTag("Structure".into()))?;
    let tag = *app.document().tags().keys().next().ok_or("missing tag")?;
    app.organize(Org::AssignTag {
        path: first.clone(),
        tag: Some(tag),
    })?;
    app.organize(Org::CreateMaterial(Material {
        name: "Amber".into(),
        color: [0.85, 0.4, 0.12],
        opacity: 1.,
        texture: None,
    }))?;
    let amber = *app
        .document()
        .materials()
        .keys()
        .next_back()
        .ok_or("missing material")?;
    app.organize(Org::PaintInstance {
        path: first.clone(),
        material: Some(amber),
    })?;
    app.organize(Org::CreateMaterial(Material {
        name: "Glass".into(),
        color: [0.1, 0.6, 0.8],
        opacity: 0.4,
        texture: Some("glass-slot".into()),
    }))?;
    let glass = *app
        .document()
        .materials()
        .keys()
        .next_back()
        .ok_or("missing material")?;
    for (name, transform) in [
        ("Glass copy", DMat4::from_translation(V::new(3., 0., 1.))),
        (
            "Mirrored copy",
            DMat4::from_translation(V::new(0., 3., 1.)) * DMat4::from_scale(V::new(-1., 0.7, 1.5)),
        ),
    ] {
        app.organize(Org::AddInstance {
            definition,
            name: name.into(),
            transform,
        })?;
        let id = *app
            .document()
            .instances(cindral_core::DefinitionId(0))
            .and_then(|m| m.keys().next_back())
            .ok_or("missing copy")?;
        app.organize(Org::PaintInstance {
            path: vec![id],
            material: Some(glass),
        })?;
    }
    app.execute(Command::AddPolyline {
        points: vec![
            V::new(-4., 0., 0.),
            V::new(-3., 0., 0.),
            V::new(-3., 1., 0.),
            V::new(-4., 1., 0.),
        ],
        closed: true,
    })?;
    let face = *app
        .model()
        .faces()
        .keys()
        .next()
        .ok_or("missing group face")?;
    app.select_connected(cindral_tools::Hit::Face(face));
    let group = app.create_container(DefinitionKind::Group)?;
    app.enter_context(group)?;
    app.organize(Org::AddInstance {
        definition,
        name: "Nested block".into(),
        transform: DMat4::from_translation(V::Z),
    })?;
    app.exit_context()?;
    app.enter_context(first)?;
    Ok(())
}
