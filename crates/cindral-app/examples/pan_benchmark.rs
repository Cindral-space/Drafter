//! Bounded release performance test: startup/import plus 200 moves per shadow mode.
fn main() -> Result<(), String> {
    let (done, watchdog) = std::sync::mpsc::channel::<()>();
    std::thread::spawn(move || {
        if matches!(
            watchdog.recv_timeout(std::time::Duration::from_secs(120)),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout)
        ) {
            eprintln!("pan benchmark exceeded hard 120s timeout");
            std::process::exit(124);
        }
    });
    let path = std::env::args_os()
        .nth(1)
        .ok_or("usage: pan_benchmark /private/model.skp")?;
    let result = cindral_app::pan_benchmark::run(std::path::Path::new(&path));
    let _ = done.send(());
    result
}
