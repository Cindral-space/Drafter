//! Release-only real-file pointer replay, no synthetic fallback.
fn main() -> Result<(), String> {
    if std::env::var_os("CINDRAL_TRACE").is_some() {
        tracing_subscriber::fmt()
            .with_env_filter("info")
            .with_span_events(tracing_subscriber::fmt::format::FmtSpan::CLOSE)
            .with_ansi(false)
            .init();
    }
    let path = std::env::args_os()
        .nth(1)
        .ok_or("usage: real_file_benchmark /private/model.skp")?;
    cindral_app::real_file_benchmark::run(std::path::Path::new(&path))
}
