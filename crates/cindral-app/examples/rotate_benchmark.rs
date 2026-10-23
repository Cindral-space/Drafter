//! Run with --release and an external hard timeout; private input never becomes an artifact.
fn main() -> Result<(), String> {
    let path = std::env::args_os()
        .nth(1)
        .ok_or("usage: rotate_benchmark /private/model.skp")?;
    cindral_app::real_file_benchmark::run_rotate(std::path::Path::new(&path))
}
