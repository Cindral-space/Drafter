//! Bounded connected-solid cold/steady preview, commit, undo and redo regression workload.
use cindral_core::{connected_fixture::connected_solid, Command, DVec3};
use std::time::Instant;
fn ms(start: Instant) -> f64 {
    start.elapsed().as_secs_f64() * 1000.
}
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
    let args = std::env::args().collect::<Vec<_>>();
    let mode = args.get(1).map_or("pushpull", String::as_str);
    if !matches!(mode, "pushpull" | "move") {
        return Err("usage: connected_benchmark [pushpull|move] [segments]".into());
    }
    let segments = args
        .get(2)
        .map(|s| s.parse::<usize>())
        .transpose()?
        .unwrap_or(60);
    let start = Instant::now();
    let (mut history, face) = connected_solid(segments)?;
    println!(
        "mode {mode}; segments {segments}; vertices {}; edges {}; faces {}; setup_ms {:.3}",
        history.model().vertices().len(),
        history.model().edges().len(),
        history.model().faces().len(),
        ms(start)
    );
    let vertices = history
        .model()
        .vertices()
        .keys()
        .copied()
        .collect::<Vec<_>>();
    let before = history.model().to_data();
    let mut samples = Vec::new();
    for i in 0..31 {
        let start = Instant::now();
        let distance = 0.1 + i as f64 * 0.001;
        let command = if mode == "pushpull" {
            Command::PushPull { face, distance }
        } else {
            Command::Transform {
                vertices: vertices.clone(),
                matrix: glam::DMat4::from_translation(DVec3::Z * distance),
                copy: false,
            }
        };
        history.preview(command)?;
        samples.push(ms(start));
        if history.undo_depth() != 0 {
            return Err("preview recorded an undo entry".into());
        }
    }
    let cold = samples[0];
    let start = Instant::now();
    history.commit_preview()?;
    let commit = ms(start);
    if history.undo_depth() != 1 {
        return Err("commit must record exactly one undo entry".into());
    }
    let after = history.model().to_data();
    let start = Instant::now();
    history.undo()?;
    let undo = ms(start);
    if history.model().to_data() != before {
        return Err("undo did not restore exact geometry".into());
    }
    let start = Instant::now();
    history.redo()?;
    let redo = ms(start);
    if history.model().to_data() != after {
        return Err("redo did not restore committed geometry".into());
    }
    let validation = Instant::now();
    history.model().validate()?;
    let validation = ms(validation);
    let mut steady = samples[1..].to_vec();
    steady.sort_by(f64::total_cmp);
    let max = samples.iter().copied().fold(0., f64::max);
    println!("preview_cold_ms {cold:.3}; preview_mean_ms {:.3}; preview_p95_ms {:.3}; preview_max_ms {max:.3}; commit_ms {commit:.3}; undo_ms {undo:.3}; redo_ms {redo:.3}; full_validation_ms {validation:.3}",steady.iter().sum::<f64>()/steady.len() as f64,steady[28]);
    let scale = ((segments as f64 / 60.).powi(2)).max(1.);
    println!(
        "preview_budget_ms {:.3}; commit_budget_ms {:.3}; budget_pass {}",
        16. * scale,
        100. * scale,
        max < 16. * scale && commit < 100. * scale
    );
    if std::env::var_os("CINDRAL_ENFORCE_BUDGET").is_some()
        && (max >= 16. * scale || commit >= 100. * scale)
    {
        return Err("connected edit exceeded its regression budget".into());
    }
    Ok(())
}
