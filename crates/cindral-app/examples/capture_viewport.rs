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
        self.app.update(ctx, frame);
        self.frames += 1;
        if self.frames == 3 {
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
            if incremental {
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
