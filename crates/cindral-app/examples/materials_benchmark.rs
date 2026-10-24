//! Phase 2 materials workload on a private model file.
//! Usage: cargo run --release -p cindral-app --features kernel-benchmark --example materials_benchmark -- /path/model.skp
fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: materials_benchmark /path/model.skp");
    let (tx, rx) = std::sync::mpsc::channel();
    let watchdog = std::thread::spawn(move || {
        if rx
            .recv_timeout(std::time::Duration::from_secs(120))
            .is_err()
        {
            eprintln!("materials benchmark watchdog: 120s timeout");
            std::process::exit(2);
        }
    });
    let result = cindral_app::materials_real_file::run(std::path::Path::new(&path));
    let _ = tx.send(());
    watchdog.join().ok();
    if let Err(e) = result {
        eprintln!("{e}");
        std::process::exit(1);
    }
}
