//! Subdivided connected solid fixture benchmark.
//!
//! Measures ONE timed edit on a subdivided closed cube solid (either pushpull of
//! one top-grid face by 0.1 along normal, or whole-solid move of all vertices
//! via affine translation).

use std::io::Write;
use std::time::Instant;

use glam::DMat4;

use cindral_core::connected_fixture::connected_solid;
use cindral_core::{Command, DVec3, VertexId};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::var_os("CINDRAL_PROFILE").is_some() {
        let _ = tracing_subscriber::fmt()
            .with_ansi(false)
            .with_span_events(
                tracing_subscriber::fmt::format::FmtSpan::ENTER
                    | tracing_subscriber::fmt::format::FmtSpan::CLOSE,
            )
            .with_env_filter("cindral_core=info")
            .try_init();
    }
    let args: Vec<String> = std::env::args().collect();
    let mut mode = "pushpull".to_string();
    let mut segments = 60usize;

    if args.len() > 1 {
        let arg1 = args[1].to_lowercase();
        if arg1 == "pushpull" || arg1 == "move" {
            mode = arg1;
            if args.len() > 2 {
                segments = args[2].parse::<usize>()?;
            }
        } else if let Ok(n) = arg1.parse::<usize>() {
            segments = n;
        } else {
            eprintln!("Usage: connected_benchmark [pushpull|move] [segments]");
            std::process::exit(1);
        }
    }

    let setup_start = Instant::now();
    let (mut history, top_face) = connected_solid(segments)?;
    let setup_ms = setup_start.elapsed().as_secs_f64() * 1000.0;

    println!(
        "mode: {}; fixture: {} vertices / {} edges / {} faces (n={}); setup {:.3} ms",
        mode,
        history.model().vertices().len(),
        history.model().edges().len(),
        history.model().faces().len(),
        segments,
        setup_ms,
    );
    std::io::stdout().flush()?;

    let (edit_ms, result) = match mode.as_str() {
        "pushpull" => {
            let edit = Command::PushPull {
                face: top_face,
                distance: 0.1,
            };
            let start = Instant::now();
            let res = history.execute(edit);
            (start.elapsed().as_secs_f64() * 1000.0, res)
        }
        "move" => {
            let all_vertices: Vec<VertexId> = history.model().vertices().keys().copied().collect();
            let edit = Command::Transform {
                vertices: all_vertices,
                matrix: DMat4::from_translation(DVec3::new(0.0, 0.0, 0.1)),
                copy: false,
            };
            let start = Instant::now();
            let res = history.execute(edit);
            (start.elapsed().as_secs_f64() * 1000.0, res)
        }
        _ => return Err("invalid benchmark mode".into()),
    };

    println!("edit_ms: {:.3}; result: {:?}", edit_ms, result);
    std::io::stdout().flush()?;
    result?;
    Ok(())
}
