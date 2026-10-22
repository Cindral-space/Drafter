//! Display-free COLLADA command-line export.
use crate::{DaeOptions,DaeSelection,DaeUnits,ExportSummary,IoError,Result};
use cindral_core::{EdgeId,FaceId,InstanceId};
use std::{collections::BTreeSet,ffi::OsString,path::PathBuf};
fn invalid(message: &str)->IoError {IoError::Invalid(message.into())}
fn parse_id(value:Option<OsString>)->Result<u64> {
    value.and_then(|v|v.to_str().and_then(|s|s.parse().ok())).ok_or_else(||invalid("selection ID must be an unsigned integer"))
}
/// Execute `--export-dae INPUT OUTPUT` and export flags, excluding the executable name.
/// Paths retain platform-native encoding; errors are returned without opening the desktop.
pub fn export_dae_cli(args:impl IntoIterator<Item=OsString>)->Result<ExportSummary> {
    let mut args=args.into_iter();
    if args.next().as_deref()!=Some(std::ffi::OsStr::new("--export-dae")) {return Err(invalid("expected --export-dae INPUT.cdraft OUTPUT.dae"));}
    let input=PathBuf::from(args.next().filter(|s|!s.is_empty()).ok_or_else(||invalid("missing input .cdraft path"))?);
    let output=PathBuf::from(args.next().filter(|s|!s.is_empty()).ok_or_else(||invalid("missing output .dae path"))?);
    if input.extension()!=Some(std::ffi::OsStr::new("cdraft")) || output.extension()!=Some(std::ffi::OsStr::new("dae")) {return Err(invalid("expected input .cdraft and output .dae paths"));}
    let mut options=DaeOptions::default();let mut selection=DaeSelection::default();let mut selection_only=false;
    let mut seen=BTreeSet::new();let mut selected=false;
    while let Some(arg)=args.next() {
        let flag=arg.to_str().ok_or_else(||invalid("export flags must be UTF-8"))?;
        if !matches!(flag,"--face"|"--edge"|"--instance") && !seen.insert(flag.to_owned()) {return Err(invalid(&format!("duplicate export flag {flag}")));}
        match flag {
            "--units"=>{options.units=match args.next().as_deref().and_then(|s|s.to_str()) {Some("meters")=>DaeUnits::Meters,Some("inches")=>DaeUnits::Inches,_=>return Err(invalid("--units requires meters or inches"))};}
            "--triangulate-all"=>options.triangulate_all=true,
            "--include-hidden"=>options.include_hidden=true,
            "--no-textures"=>options.include_textures=false,
            "--force"=>options.force=true,
            "--selection-only"=>selection_only=true,
            "--face"=>{selection.faces.insert(FaceId::from_raw(parse_id(args.next())?));selected=true;}
            "--edge"=>{selection.edges.insert(EdgeId::from_raw(parse_id(args.next())?));selected=true;}
            "--instance"=>{selection.instances.insert(InstanceId(parse_id(args.next())?));selected=true;}
            "--context"=>{
                let value=args.next().ok_or_else(||invalid("--context requires ID/ID"))?;
                let path=value.to_str().ok_or_else(||invalid("invalid --context path"))?;
                if path.is_empty(){return Err(invalid("--context requires ID/ID"));}
                selection.context=path.split('/').map(|id|id.parse::<u64>().map(InstanceId).map_err(|_|invalid("--context requires slash-separated unsigned IDs"))).collect::<Result<_>>()?;
                selected=true;
            }
            _=>return Err(invalid(&format!("unknown export flag {flag}"))),
        }
    }
    if selected && !selection_only {return Err(invalid("selection IDs and --context require --selection-only"));}
    if selection_only {options.selection=Some(selection);}
    let bundle=crate::load_draft(&input)?;
    crate::export_dae(&output,&bundle,&options,&crate::DaeControl::default())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_to_dae_cli_safety_units_and_empty_selection() {
        let dir=tempfile::tempdir().unwrap();let input=dir.path().join("input.cdraft");let output=dir.path().join("output.dae");
        crate::save_draft(&input,&crate::DraftBundle::default()).unwrap();
        let base=vec![OsString::from("--export-dae"),input.into_os_string(),output.clone().into_os_string()];
        export_dae_cli(base.clone()).unwrap();let original=std::fs::read(&output).unwrap();
        assert!(export_dae_cli(base.clone()).unwrap_err().to_string().contains("already exists"));assert_eq!(std::fs::read(&output).unwrap(),original);
        let mut args=base;args.extend(["--force","--units","inches","--selection-only","--no-textures"].map(OsString::from));
        assert_eq!(export_dae_cli(args).unwrap().instances,0);
        assert!(std::fs::read_to_string(&output).unwrap().contains("meter=\"0.0254\""));
    }
    #[test]
    fn malformed_options_fail_before_loading() {
        for flags in [vec!["--units","feet"],vec!["--force","--force"],vec!["--face","bad"],vec!["--face","1"],vec!["--context","1//2","--selection-only"],vec!["--context","1"],vec!["--unknown"],vec!["--edge"]] {
            let mut args=vec!["--export-dae","absent.cdraft","output.dae"];args.extend(flags);
            assert!(matches!(export_dae_cli(args.into_iter().map(OsString::from)),Err(IoError::Invalid(_))));
        }
        assert!(export_dae_cli([OsString::from("--export-dae")]).is_err());
    }
}
