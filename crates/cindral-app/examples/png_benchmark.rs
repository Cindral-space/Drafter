//! Release-only complete single PNG export timing; one size per process for peak RSS.
use cindral_app::{
    png_export::{Aa, Control, Options},
    png_job,
};
use std::time::Instant;
fn main() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let width: u32 = args.next().ok_or("width")?.parse().map_err(|_| "width")?;
    let height: u32 = args.next().ok_or("height")?.parse().map_err(|_| "height")?;
    let options = Options {
        width,
        height,
        aa: Aa::High,
        match_window: false,
        ..Default::default()
    };
    if let Err(e) = options.estimate(0) {
        println!("{width}x{height} High: REFUSED: {e}");
        return Ok(());
    }
    let bundle = if let Some(path) = args.next() {
        cindral_io::load_draft(std::path::Path::new(&path)).map_err(|e| e.to_string())?
    } else {
        cindral_io::DraftBundle {
            document: cindral_app::organization_benchmark::run_cpu()
                .map_err(|e| e.to_string())?
                .section_document,
            ..Default::default()
        }
    };
    let dir = tempfile::tempdir().map_err(|e| e.to_string())?;
    let path = dir.path().join("benchmark.png");
    let started = Instant::now();
    let image = png_job::export(
        bundle,
        None,
        None,
        &path,
        &options,
        true, // same sRGB-surface emulation as the CLI and GUI export
        "benchmark.cdraft",
        false,
        &Control::default(),
    )?;
    let elapsed = started.elapsed();
    if image.adapter.device_type == eframe::egui_wgpu::wgpu::DeviceType::Cpu {
        return Err("software adapter is not a Vega 11 hardware measurement".into());
    }
    println!(
        "{width}x{height} High: {:.3} ms; estimate {} MiB; PNG {} bytes; adapter {:?}",
        elapsed.as_secs_f64() * 1000.,
        image.estimated_bytes.div_ceil(1024 * 1024),
        std::fs::metadata(&path).map_err(|e| e.to_string())?.len(),
        image.adapter
    );
    #[cfg(target_os = "linux")]
    {
        let status = std::fs::read_to_string("/proc/self/status").map_err(|e| e.to_string())?;
        for line in status.lines().filter(|l| l.starts_with("VmHWM:")) {
            println!("Process peak RSS: {line}");
        }
    }
    Ok(())
}
