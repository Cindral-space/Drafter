//! Headless release Push/Pull performance investigation.
fn main() -> Result<(), String> {
    if std::env::var_os("CINDRAL_PROFILE").is_some() {
        tracing_subscriber::fmt().with_env_filter("info").with_span_events(tracing_subscriber::fmt::format::FmtSpan::CLOSE).with_ansi(false).init();
    }
    cindral_app::import_pushpull_benchmark::run(|_| Ok(()))
}
