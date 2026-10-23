//! Current-view image options and responsive worker status.
use super::*;
use crate::png_export::{Aa,Control,Options};
use std::{path::PathBuf,sync::mpsc};
struct Job {control:Control,receiver:mpsc::Receiver<Result<String,String>>}
impl Drop for Job {fn drop(&mut self){self.control.cancel();}}
pub(crate) struct PngUi {
    pub(crate) open:bool,
    options:Options,
    prefs:PathBuf,
    job:Option<Job>,
    status:String,
    pub(crate) viewport:[u32;2],
    pub(crate) srgb_surface:bool,
}
impl Default for PngUi {
    fn default()->Self {
        let prefs=preferences_path();
        Self {open:false,options:Options::load(&prefs),prefs,job:None,status:String::new(),viewport:[1280,800],srgb_surface:false}
    }
}
fn preferences_path()->PathBuf {
    let root=std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or_else(||{
        #[cfg(target_os="windows")]
        if let Some(p)=std::env::var_os("APPDATA"){return PathBuf::from(p);}
        let home=std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
        #[cfg(target_os="macos")]
        {home.join("Library/Application Support")}
        #[cfg(not(target_os="macos"))]
        {home.join(".config")}
    });root.join("cindral-drafter/png-export.json")
}
impl DrafterApp {
    fn start_png(&mut self,path:PathBuf,force:bool) {
        self.cancel_modify();self.tool.cancel();self.cancel_section();
        let options=self.png.options.clone();
        if let Err(e)=options.estimate(0) {self.png.status=e;return;}
        let bundle=self.file_values();let camera=self.camera.clone();let srgb=self.png.srgb_surface;
        let name=self.files.path.as_ref().and_then(|p|p.file_name()).map(|s|s.to_string_lossy().into_owned()).unwrap_or_else(||"Untitled".into());
        let control=Control::default();let worker=control.clone();let (sender,receiver)=mpsc::channel();
        std::thread::spawn(move|| {
            let result=crate::png_job::export(bundle,Some(camera),None,&path,&options,srgb,&name,force,&worker)
                .map(|_|format!("Exported {} ({}×{})",path.display(),options.width,options.height));
            let _=sender.send(result);
        });
        self.png.job=Some(Job{control,receiver});self.png.status="Preparing PNG…".into();
    }
    pub(crate) fn png_window(&mut self,ctx:&egui::Context) {
        if let Some(job)=&self.png.job {match job.receiver.try_recv() {
            Ok(result)=>{self.png.job=None;self.png.status=result.unwrap_or_else(|e|e);}
            Err(mpsc::TryRecvError::Disconnected)=>{self.png.job=None;self.png.status="PNG worker stopped before completion".into();}
            Err(mpsc::TryRecvError::Empty)=>ctx.request_repaint_after(std::time::Duration::from_millis(50)),
        }}
        if !self.png.open{return;}
        if self.png.options.match_window && self.png.job.is_none() {
            self.png.options.width=self.png.viewport[0];self.png.options.height=self.png.viewport[1];
        }
        let mut export=false;
        egui::Window::new("Export Image (PNG)").collapsible(false).resizable(false).show(ctx,|ui|{
            let state=&mut self.png;
            ui.add_enabled_ui(state.job.is_none(),|ui|{
                egui::ComboBox::from_id_salt("png-size").selected_text(if state.options.match_window {"Match window"}else{"Explicit size"}).show_ui(ui,|ui|{
                    ui.selectable_value(&mut state.options.match_window,true,"Match window");
                    for (label,w,h) in [("1920×1080",1920,1080),("3840×2160",3840,2160)] {
                        if ui.selectable_label(!state.options.match_window && state.options.width==w && state.options.height==h,label).clicked(){state.options.match_window=false;state.options.width=w;state.options.height=h;}
                    }
                    if ui.selectable_label(!state.options.match_window,"Custom").clicked(){state.options.match_window=false;}
                });
                ui.horizontal(|ui|{
                    ui.add_enabled_ui(!state.options.match_window,|ui|{
                        ui.add(egui::DragValue::new(&mut state.options.width).range(1..=8192).prefix("Width "));
                        ui.add(egui::DragValue::new(&mut state.options.height).range(1..=8192).prefix("Height "));
                    });
                });
                ui.horizontal(|ui|{ui.label("Antialiasing");for aa in [Aa::Off,Aa::Medium,Aa::High] {ui.selectable_value(&mut state.options.aa,aa,format!("{aa:?}"));}});
                ui.checkbox(&mut state.options.transparent,"Transparent background");
                ui.label("Vertical framing is fixed. A wider image extends the sides; a narrower image crops them.");
                ui.label("UI, selection, tools, axes, guides and grid are excluded.");
                match state.options.estimate(0) {
                    Ok((_,bytes))=>{ui.label(format!("Base estimate {} MiB + scene data; 512 MiB budget, render limit 8192 including AA.",bytes.div_ceil(1024*1024)));if ui.button("Choose file and export…").clicked(){export=true;}}
                    Err(e)=>{ui.colored_label(Color32::LIGHT_RED,e);}
                }
            });
            if let Some(job)=&state.job {
                ui.add(egui::ProgressBar::new(job.control.progress()).show_percentage());
                if ui.button("Cancel export").clicked(){state.status=if job.control.cancel(){"Cancelling (submitted GPU work must finish)…"}else{"Export already completed"}.into();}
            } else if ui.button("Close").clicked(){state.open=false;}
            ui.label(&state.status);
        });
        if export {
            let mut dialog=rfd::FileDialog::new().add_filter("PNG image",&["png"]).set_file_name("view.png");
            if !self.png.options.folder.as_os_str().is_empty(){dialog=dialog.set_directory(&self.png.options.folder);}
            if let Some(mut path)=dialog.save_file(){
                path.set_extension("png");let mut force=false;
                if path.exists(){
                    if rfd::MessageDialog::new().set_title("Replace PNG?").set_description(format!("Replace {}?",path.display())).set_buttons(rfd::MessageButtons::YesNo).show()!=rfd::MessageDialogResult::Yes{return;}
                    force=true;
                }
                self.png.options.folder=path.parent().unwrap_or(std::path::Path::new(".")).to_path_buf();
                if let Err(e)=self.png.options.save(&self.png.prefs){self.png.status=format!("Could not save PNG preferences: {e}");return;}
                self.start_png(path,force);
            }
        }
    }
}
