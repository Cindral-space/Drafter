//! Reproducible native viewport capture for manual GPU validation.
//!
//! Run `cargo run -p cindral-app --example capture_viewport -- capture.ppm`.

use std::io::Write;

use eframe::egui;

struct Capture {
    app: cindral_app::DrafterApp,
    output: std::path::PathBuf,
    frames: usize,
}

impl eframe::App for Capture {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
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
            Ok(Box::new(Capture {
                app: cindral_app::DrafterApp::new(cc)?,
                output,
                frames: 0,
            }))
        }),
    )
}
