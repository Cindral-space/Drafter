//! Cindral Drafter desktop entry point.

fn main() -> eframe::Result {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
    let options = eframe::NativeOptions {
        renderer: eframe::Renderer::Wgpu,
        multisampling: 4,
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 800.0])
            .with_min_inner_size([720.0, 480.0])
            .with_maximized(true),
        // Always use the launch default instead of restoring a smaller window.
        persist_window: false,
        ..Default::default()
    };
    eframe::run_native(
        "Cindral Drafter",
        options,
        Box::new(|cc| Ok(Box::new(cindral_app::DrafterApp::new(cc)?))),
    )
}
