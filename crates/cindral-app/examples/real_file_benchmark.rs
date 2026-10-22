//! Release-only real-file pointer replay, no synthetic fallback.
fn main() -> Result<(),String> {
    let path=std::env::args_os().nth(1).ok_or("usage: real_file_benchmark /private/model.skp")?;
    cindral_app::real_file_benchmark::run(std::path::Path::new(&path))
}
