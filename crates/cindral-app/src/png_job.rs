//! Shared window-free scene preparation for desktop export, CLI and GPU tests.
use crate::{png_export::{self, Control, Options}, DrafterApp};
use cindral_render::{camera::Camera, capture::block_on, SceneRenderer};
use eframe::egui_wgpu::wgpu;
use std::path::Path;

/// Fully prepared image and conservative working-set estimate.
pub struct Image {
    /// Straight-alpha sRGB PNG pixels in top-to-bottom order.
    pub rgba: Vec<u8>,
    /// Estimated export bytes including upload headroom.
    pub estimated_bytes: u64,
    /// Actual adapter used (useful for release measurements).
    pub adapter: wgpu::AdapterInfo,
}
/// Render an immutable document snapshot with the production scene pipelines.
/// With no camera/scene, fit the default camera (native archives do not persist a live camera).
pub fn render(bundle: cindral_io::DraftBundle, camera: Option<Camera>, scene: Option<&str>,
    options:&Options, srgb_surface:bool, control:&Control)->Result<Image,String> {
    control.check()?;let (size,_)=options.estimate(0)?;
    let history=cindral_io::validate_bundle(&bundle).map_err(|e|e.to_string())?;
    let mut app=DrafterApp {history,presentation:bundle.presentation,..Default::default()};
    app.files.textures=bundle.textures;
    if let Some(camera)=camera {app.camera=camera;} else {app.files.fit=true;}
    if let Some(name)=scene {
        let i=app.presentation.scenes.iter().position(|s|s.name==name).ok_or_else(||format!("saved scene not found: {name}"))?;
        app.restore_scene(i,0.,false);
        if let Some(error)=app.error.take() {return Err(error);}
    }
    control.set_progress(10);control.check()?;
    app.sync_file_textures().map_err(|e|e.to_string())?;
    if let Some(error)=app.error.take() {return Err(error);}
    if app.files.fit {app.extents(options.width as f32/options.height as f32);}
    app.prepare_sections();
    if let Some(error)=app.error.take() {return Err(error);}
    control.check()?;
    let update=app.pending.take().unwrap_or_default();
    let geometry=update.geometry.iter().flat_map(|d|d.mesh.face_writes.iter().chain(&d.mesh.edge_writes)).map(|w|w.bytes.len() as u64).sum::<u64>()
        +update.batches.as_ref().map_or(0,|b|b.iter().map(|b|b.instances.len() as u64*160).sum::<u64>())
        +app.section_ui.caps.len() as u64*std::mem::size_of::<cindral_render::FaceVertex>() as u64
        +update.textures.as_ref().map_or(0,|t|t.iter().map(|t|t.rgba.len() as u64).sum::<u64>());
    let (_,estimate)=options.estimate(geometry)?;
    let instance=wgpu::Instance::new(&wgpu::InstanceDescriptor::from_env_or_default());
    let adapter=block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {compatible_surface:None,..Default::default()}))
        .map_err(|e|format!("PNG needs a headless wgpu adapter (GL may require a display): {e}"))?;
    if size.iter().any(|&n|n>adapter.limits().max_texture_dimension_2d) {return Err("PNG size exceeds this adapter's texture limit; reduce size or AA".into());}
    let mut limits=wgpu::Limits::default();
    limits.max_texture_dimension_2d=8192.min(adapter.limits().max_texture_dimension_2d);
    let (device,queue)=block_on(adapter.request_device(&wgpu::DeviceDescriptor {required_limits:limits,..Default::default()}))
        .map_err(|e|format!("PNG GPU device unavailable: {e}"))?;
    device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
    device.push_error_scope(wgpu::ErrorFilter::Validation);
    device.push_error_scope(wgpu::ErrorFilter::Internal);
    let result=(|| {
        control.check()?;control.set_progress(25);
        let mut renderer=SceneRenderer::new(&device,wgpu::TextureFormat::Rgba8Unorm,1);
        renderer.apply_update(&device,&queue,&update);
        renderer.set_style(app.presentation.style.clone());
        renderer.set_section(&device,&queue,app.presentation.sections.cut().cloned(),&app.section_ui.caps);
        renderer.set_shadows(app.presentation.shadows.clone(),app.section_ui.bounds);
        control.check()?;control.set_progress(40);
        renderer.capture(&device,&queue,png_export::framing(&app.camera,size).to_cols_array_2d(),size)
    })();
    let mut gpu_error=None;
    for _ in 0..3 {if let Some(e)=block_on(device.pop_error_scope()) {gpu_error=Some(format!("PNG GPU allocation/render failed: {e}"));}}
    if let Some(e)=gpu_error {return Err(e);}
    let raw=result?;control.check()?;control.set_progress(65);
    let rgba=png_export::resolve(&raw,options,&app.camera,&app.presentation.style,srgb_surface,control)?;
    Ok(Image {rgba,estimated_bytes:estimate,adapter:adapter.get_info()})
}
/// Render and atomically publish one PNG, with no visible window.
#[allow(clippy::too_many_arguments)]
pub fn export(bundle:cindral_io::DraftBundle,camera:Option<Camera>,scene:Option<&str>,path:&Path,
    options:&Options,srgb_surface:bool,document:&str,force:bool,control:&Control)->Result<Image,String> {
    png_export::destination(path,force)?;
    // Fail unwritable/missing parent before GPU work, keeping only an unnamed temporary.
    let parent=path.parent().filter(|p|!p.as_os_str().is_empty()).unwrap_or(Path::new("."));
    drop(tempfile::NamedTempFile::new_in(parent).map_err(|e|format!("PNG destination {}: {e}",path.display()))?);
    let image=render(bundle,camera,scene,options,srgb_surface,control)?;
    png_export::write_png(path,options,&image.rgba,document,force,control)?;
    Ok(image)
}
