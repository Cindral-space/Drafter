//! Single-image PNG command line entry point.
use crate::{png_export::{Aa, Control, Options}, png_job};
use std::{ffi::OsString,path::PathBuf};
/// Run `--export-png in.cdraft out.png --width W --height H` without opening a window.
pub fn run(args:impl IntoIterator<Item=OsString>)->Result<(),String> {
    let mut args=args.into_iter();
    if args.next().as_deref()!=Some(std::ffi::OsStr::new("--export-png")) {return Err("expected --export-png".into());}
    let input=PathBuf::from(args.next().ok_or("missing input.cdraft")?);
    let output=PathBuf::from(args.next().ok_or("missing output.png")?);
    let mut options=Options {match_window:false,..Default::default()};
    let mut width=false;let mut height=false;let mut scene=None;let mut force=false;
    while let Some(flag)=args.next() {
        match flag.to_str().ok_or("invalid PNG flag")? {
            "--width"=>{options.width=args.next().and_then(|v|v.to_str().and_then(|v|v.parse().ok())).ok_or("invalid --width")?;width=true;}
            "--height"=>{options.height=args.next().and_then(|v|v.to_str().and_then(|v|v.parse().ok())).ok_or("invalid --height")?;height=true;}
            "--aa"=>{options.aa=match args.next().and_then(|s|s.into_string().ok()).as_deref(){Some("off")=>Aa::Off,Some("medium")=>Aa::Medium,Some("high")=>Aa::High,_=>return Err("--aa must be off, medium or high".into())};}
            "--transparent"=>options.transparent=true,
            "--scene"=>scene=Some(args.next().and_then(|s|s.into_string().ok()).ok_or("missing --scene NAME")?),
            "--force"=>force=true,
            _=>return Err(format!("unknown PNG flag {}",flag.to_string_lossy())),
        }
    }
    if !width||!height {return Err("PNG requires --width W --height H".into());}
    options.estimate(0)?;
    let bundle=cindral_io::load_draft(&input).map_err(|e|e.to_string())?;
    let name=input.file_name().unwrap_or_default().to_string_lossy();
    png_job::export(bundle,None,scene.as_deref(),&output,&options,false,&name,force,&Control::default())?;
    println!("Exported {} ({}×{})",output.display(),options.width,options.height);Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn rejects_missing_sizes_bad_quality_and_unknown_flags_before_gpu() {
        for (args,expected) in [
            (vec!["--export-png","in.cdraft","out.png"],"requires --width"),
            (vec!["--export-png","in.cdraft","out.png","--aa","bad"],"--aa must"),
            (vec!["--export-png","in.cdraft","out.png","--wat"],"unknown PNG flag"),
            (vec!["--export-png","in.cdraft","out.png","--width","0","--height","1"],"dimension"),
        ] {
            assert!(super::run(args.into_iter().map(std::ffi::OsString::from)).unwrap_err().contains(expected));
        }
    }
}
