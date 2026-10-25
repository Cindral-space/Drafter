//! Shared window-free scene preparation for desktop export, CLI and GPU tests.
use crate::{
    png_export::{self, Control, Options},
    DrafterApp,
};
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
pub fn render(
    bundle: cindral_io::DraftBundle,
    camera: Option<Camera>,
    scene: Option<&str>,
    options: &Options,
    srgb_surface: bool,
    control: &Control,
) -> Result<Image, String> {
    control.check()?;
    let (size, _) = options.estimate(0)?;
    let history = cindral_io::validate_bundle(&bundle).map_err(|e| e.to_string())?;
    let mut app = DrafterApp {
        history,
        presentation: bundle.presentation,
        ..Default::default()
    };
    app.files.textures = bundle.textures;
    app.history
        .enter(options.context.clone())
        .map_err(|e| e.to_string())?;
    if let Some(camera) = camera {
        app.camera = camera;
    } else {
        app.files.fit = true;
    }
    if let Some(name) = scene {
        let i = app
            .presentation
            .scenes
            .iter()
            .position(|s| s.name == name)
            .ok_or_else(|| format!("saved scene not found: {name}"))?;
        app.restore_scene(i, 0., false);
        if let Some(error) = app.error.take() {
            return Err(error);
        }
    }
    control.set_progress(10);
    control.check()?;
    app.sync_file_textures().map_err(|e| e.to_string())?;
    if let Some(error) = app.error.take() {
        return Err(error);
    }
    if app.files.fit {
        app.extents(options.width as f32 / options.height as f32);
    }
    app.prepare_sections();
    if let Some(error) = app.error.take() {
        return Err(error);
    }
    control.check()?;
    let update = app.pending.take().unwrap_or_default();
    let geometry = update
        .geometry
        .iter()
        .flat_map(|d| d.mesh.face_writes.iter().chain(&d.mesh.edge_writes))
        .map(|w| w.bytes.len() as u64)
        .sum::<u64>()
        + update.batches.as_ref().map_or(0, |b| {
            b.iter()
                .map(|b| b.instances.len() as u64 * 160)
                .sum::<u64>()
        })
        + app.section_ui.caps.len() as u64
            * std::mem::size_of::<cindral_render::FaceVertex>() as u64
        + update.textures.as_ref().map_or(0, |t| {
            t.layers
                .iter()
                .flat_map(|l| l.levels.iter())
                .map(|l| l.rgba.len() as u64)
                .sum::<u64>()
        });
    let (_, estimate) = options.estimate(geometry)?;
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::from_env_or_default());
    let adapter = block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        compatible_surface: None,
        ..Default::default()
    }))
    .map_err(|e| format!("PNG needs a headless wgpu adapter (GL may require a display): {e}"))?;
    if size
        .iter()
        .any(|&n| n > adapter.limits().max_texture_dimension_2d)
    {
        return Err("PNG size exceeds this adapter's texture limit; reduce size or AA".into());
    }
    let limits = wgpu::Limits {
        max_texture_dimension_2d: 8192.min(adapter.limits().max_texture_dimension_2d),
        ..Default::default()
    };
    let (device, queue) = block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_limits: limits,
        ..Default::default()
    }))
    .map_err(|e| format!("PNG GPU device unavailable: {e}"))?;
    device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
    device.push_error_scope(wgpu::ErrorFilter::Validation);
    device.push_error_scope(wgpu::ErrorFilter::Internal);
    let result = (|| {
        control.check()?;
        control.set_progress(25);
        let mut renderer = SceneRenderer::new(&device, wgpu::TextureFormat::Rgba8Unorm, 1);
        renderer.apply_update(&device, &queue, &update);
        renderer.set_style(app.presentation.style.clone());
        renderer.set_section(
            &device,
            &queue,
            app.presentation.sections.cut().cloned(),
            &app.section_ui.caps,
        );
        renderer.set_shadows(app.presentation.shadows.clone(), app.section_ui.bounds);
        control.check()?;
        control.set_progress(40);
        renderer.capture(
            &device,
            &queue,
            png_export::framing(&app.camera, size).to_cols_array_2d(),
            size,
        )
    })();
    let mut gpu_error = None;
    for _ in 0..3 {
        if let Some(e) = block_on(device.pop_error_scope()) {
            gpu_error = Some(format!("PNG GPU allocation/render failed: {e}"));
        }
    }
    if let Some(e) = gpu_error {
        return Err(e);
    }
    let raw = result?;
    control.check()?;
    control.set_progress(65);
    let rgba = png_export::resolve(
        &raw,
        options,
        &app.camera,
        &app.presentation.style,
        srgb_surface,
        control,
    )?;
    Ok(Image {
        rgba,
        estimated_bytes: estimate,
        adapter: adapter.get_info(),
    })
}
/// Render and atomically publish one PNG, with no visible window.
#[allow(clippy::too_many_arguments)]
pub fn export(
    bundle: cindral_io::DraftBundle,
    camera: Option<Camera>,
    scene: Option<&str>,
    path: &Path,
    options: &Options,
    srgb_surface: bool,
    document: &str,
    force: bool,
    control: &Control,
) -> Result<Image, String> {
    png_export::destination(path, force)?;
    // Fail unwritable/missing parent before GPU work, keeping only an unnamed temporary.
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    drop(
        tempfile::NamedTempFile::new_in(parent)
            .map_err(|e| format!("PNG destination {}: {e}", path.display()))?,
    );
    let image = render(bundle, camera, scene, options, srgb_surface, control)?;
    png_export::write_png(path, options, &image.rgba, document, force, control)?;
    Ok(image)
}

#[cfg(test)]
mod tests {
    use super::*;
    use cindral_core::{Command, DVec3, StyleMode};
    use eframe::egui;
    fn cube() -> cindral_io::DraftBundle {
        let mut app = DrafterApp::default();
        app.execute(Command::AddPolyline {
            points: vec![DVec3::ZERO, DVec3::X, DVec3::X + DVec3::Y, DVec3::Y],
            closed: true,
        })
        .unwrap();
        let face = *app.document().model().faces().keys().next().unwrap();
        app.execute(Command::PushPull { face, distance: 1. })
            .unwrap();
        let mut bundle = app.file_values();
        bundle.presentation.style.sky = None;
        bundle.presentation.style.ground = None;
        // Explicit material-like face color, avoiding a default-gray-only regression.
        for definition in bundle.document.definitions.values_mut() {
            for face in definition.geometry.faces.values_mut() {
                face.color = [220, 50, 30, 255];
            }
        }
        bundle
    }
    /// Cube painted through the GPU material table (palette material, not
    /// legacy vertex colors), plus the swatch color for expectations.
    fn material_cube(color: [u8; 3]) -> cindral_io::DraftBundle {
        use cindral_core::{FaceId, Material, OrganizationCommand as Org};
        let mut app = DrafterApp::default();
        app.execute(Command::AddPolyline {
            points: vec![DVec3::ZERO, DVec3::X, DVec3::X + DVec3::Y, DVec3::Y],
            closed: true,
        })
        .unwrap();
        let face = *app.document().model().faces().keys().next().unwrap();
        app.execute(Command::PushPull { face, distance: 1. })
            .unwrap();
        app.org_command(Org::CreateMaterial(Material {
            name: "Swatch".into(),
            color,
            opacity: 1.,
            texture: None,
            texture_mapping: None,
        }));
        let material = app.history.materials().keys().next_back().copied().unwrap();
        let faces: Vec<FaceId> = app.document().model().faces().keys().copied().collect();
        app.org_command(Org::PaintFaces {
            faces,
            material: Some(material),
        });
        let mut bundle = app.file_values();
        bundle.presentation.style.sky = None;
        bundle.presentation.style.ground = None;
        bundle
    }
    /// Orthographic top view: only the cube's +Z face is visible.
    fn top_camera() -> Camera {
        let mut camera = camera();
        camera.set_view(cindral_render::camera::StandardView::Top);
        camera.projection = cindral_render::camera::Projection::Orthographic;
        camera
    }
    fn options() -> Options {
        Options {
            width: 64,
            height: 64,
            aa: png_export::Aa::Off,
            match_window: false,
            ..Default::default()
        }
    }
    fn camera() -> Camera {
        Camera {
            target: glam::Vec3::splat(0.5),
            distance: 4.,
            ..Default::default()
        }
    }
    fn image(bundle: cindral_io::DraftBundle, o: &Options, scene: Option<&str>) -> Image {
        render(bundle, Some(camera()), scene, o, false, &Control::default()).unwrap()
    }
    fn adapter_unavailable(e: &str) -> bool {
        e.contains("headless wgpu adapter") || e.contains("GPU device unavailable")
    }
    #[test]
    fn gpu_cube_transparent_sections_shadows_saved_scene_and_viewport_match() {
        let bundle = cube();
        let o = options();
        let baseline = match render(
            bundle.clone(),
            Some(camera()),
            None,
            &o,
            false,
            &Control::default(),
        ) {
            Ok(image) => image,
            Err(e) if adapter_unavailable(&e) => {
                eprintln!("SKIP PNG GPU checks: {e}");
                return;
            }
            Err(e) => panic!("{e}"),
        };
        // The complete GPU/export path must ignore the persisted chrome preference.
        let ctx = egui::Context::default();
        crate::workspace_ui::configure_style(&ctx);
        for theme in [
            crate::theme::Preference::Light,
            crate::theme::Preference::Dark,
        ] {
            theme.apply(&ctx);
            let mut themed = o.clone();
            themed.theme = theme;
            assert_eq!(image(bundle.clone(), &themed, None).rgba, baseline.rgba);
        }
        assert_eq!(baseline.rgba.len(), 64 * 64 * 4);
        assert_eq!(&baseline.rgba[..4], &[237, 240, 245, 255]);
        let center = (32 * 64 + 32) * 4;
        assert!(baseline.rgba[center] > baseline.rgba[center + 1] * 2);
        let mut transparent = o.clone();
        transparent.transparent = true;
        let alpha = image(bundle.clone(), &transparent, None);
        for i in [0, 63 * 4, 63 * 64 * 4, (64 * 64 - 1) * 4] {
            assert_eq!(&alpha.rgba[i..i + 4], &[0, 0, 0, 0]);
        }
        assert_eq!(alpha.rgba[center + 3], 255);
        let mut cut = bundle.clone();
        cut.presentation.sections.enabled = true;
        cut.presentation.sections.active = Some(0);
        cut.presentation.sections.planes.push(
            cindral_core::SectionPlane::through("cut".into(), DVec3::splat(0.5), DVec3::Z).unwrap(),
        );
        assert_ne!(image(cut, &o, None).rgba, baseline.rgba);
        let mut shadow = bundle.clone();
        shadow.presentation.shadows.enabled = true;
        shadow.presentation.shadows.on_ground = true;
        assert_ne!(image(shadow, &o, None).rgba, baseline.rgba);
        let mut app = DrafterApp {
            history: cindral_io::validate_bundle(&bundle).unwrap(),
            ..Default::default()
        };
        let mut scene = app.history.capture_scene(
            "Wire".into(),
            cindral_core::SceneCamera {
                target: camera().target.to_array(),
                distance: camera().distance,
                yaw: camera().yaw,
                pitch: camera().pitch,
                orthographic: false,
            },
            bundle.presentation.style.clone(),
        );
        scene.style.mode = StyleMode::Wireframe;
        let mut saved = bundle.clone();
        saved.presentation.scenes.push(scene.clone());
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("scene.cdraft");
        let output = dir.path().join("scene.png");
        cindral_io::save_draft(&input, &saved).unwrap();
        crate::png_cli::run([
            std::ffi::OsString::from("--export-png"),
            input.clone().into_os_string(),
            output.clone().into_os_string(),
            "--width".into(),
            "64".into(),
            "--height".into(),
            "64".into(),
            "--aa".into(),
            "off".into(),
            "--scene".into(),
            "Wire".into(),
        ])
        .unwrap();
        let mut reader = png::Decoder::new(std::io::BufReader::new(
            std::fs::File::open(output).unwrap(),
        ))
        .read_info()
        .unwrap();
        let mut decoded = vec![0; 64 * 64 * 4];
        reader.next_frame(&mut decoded).unwrap();
        // The CLI applies the desktop sRGB-surface encode, so compare with
        // the same production path; the linear render must differ.
        let selected = render(
            saved,
            Some(camera()),
            Some("Wire"),
            &o,
            true,
            &Control::default(),
        )
        .unwrap();
        assert_eq!(selected.rgba, decoded);
        let mut wire = bundle.clone();
        wire.presentation.style = scene.style;
        assert_eq!(
            selected.rgba,
            render(wire, Some(camera()), None, &o, true, &Control::default())
                .unwrap()
                .rgba
        );
        assert_ne!(selected.rgba, baseline.rgba);
        // A real egui/wgpu callback composites the same camera/size into the window format.
        app.presentation = bundle.presentation.clone();
        app.camera = camera();
        app.sync_file_textures().unwrap();
        app.prepare_sections();
        let displayed = viewport_callback(&mut app, false);
        assert!(
            displayed
                .iter()
                .zip(&baseline.rgba)
                .all(|(&a, &b)| a.abs_diff(b) <= 2),
            "viewport/export differ"
        );
        app.scene = Default::default();
        app.sync_file_textures().unwrap();
        let displayed = viewport_callback(&mut app, true);
        let linear_export = render(
            bundle.clone(),
            Some(camera()),
            None,
            &o,
            true,
            &Control::default(),
        )
        .unwrap();
        assert!(
            displayed
                .iter()
                .zip(&linear_export.rgba)
                .all(|(&a, &b)| a.abs_diff(b) <= 2),
            "sRGB viewport/export differ"
        );
        let control = Control::default();
        control.cancel();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cancel.png");
        assert!(export(
            bundle,
            Some(camera()),
            None,
            &path,
            &o,
            false,
            "test.cdraft",
            false,
            &control
        )
        .is_err());
        assert!(!path.exists());
    }
    /// A palette-material face renders the swatch color: the panel swatch
    /// (the stored sRGB bytes), the viewport pixel and the exported PNG
    /// pixel all agree with the shaded sRGB value within tolerance, for a
    /// mid-gray and a saturated color (review #1). Both legs use the
    /// production sRGB-surface path the desktop app exports and displays
    /// with (`target_format.is_srgb()`).
    #[test]
    fn material_swatch_viewport_and_export_pixel_agree() {
        for color in [[128u8, 128, 128], [30, 200, 90]] {
            let bundle = material_cube(color);
            let o = options();
            let baseline = match render(
                bundle.clone(),
                Some(top_camera()),
                None,
                &o,
                true,
                &Control::default(),
            ) {
                Ok(image) => image,
                Err(e) if adapter_unavailable(&e) => {
                    eprintln!("SKIP PNG GPU checks: {e}");
                    return;
                }
                Err(e) => panic!("{e}"),
            };
            let center = (32 * 64 + 32) * 4;
            // scene.wgsl face lighting for the +Z face: fixed key/fill
            // lights, shaded in linear space, sRGB-encoded by the
            // Rgba8UnormSrgb target.
            let key = glam::Vec3::new(0.45, -0.35, 0.82).normalize();
            let fill = glam::Vec3::new(-0.55, 0.60, 0.58).normalize();
            let shade = 0.45 + 0.40 * key.z.abs() + 0.15 * fill.z.abs();
            let expected: [u8; 3] = color.map(|c| {
                let linear = cindral_core::srgb_to_linear(f32::from(c) / 255.);
                (255. * cindral_core::linear_to_srgb(linear * shade))
                    .round()
                    .clamp(0., 255.) as u8
            });
            let pixel = [
                baseline.rgba[center],
                baseline.rgba[center + 1],
                baseline.rgba[center + 2],
            ];
            assert!(
                pixel.iter().zip(expected).all(|(&a, b)| a.abs_diff(b) <= 2),
                "{color:?}: export pixel {pixel:?} != shaded swatch {expected:?}"
            );
            // The live viewport composites the same color.
            let mut app = DrafterApp {
                history: cindral_io::validate_bundle(&bundle).unwrap(),
                ..Default::default()
            };
            app.presentation = bundle.presentation.clone();
            app.camera = top_camera();
            app.sync_file_textures().unwrap();
            app.prepare_sections();
            let displayed = viewport_callback(&mut app, true);
            assert!(
                displayed
                    .iter()
                    .zip(&baseline.rgba)
                    .all(|(&a, &b)| a.abs_diff(b) <= 2),
                "{color:?}: viewport/export differ"
            );
        }
    }

    /// The CLI export (--export-png) and the desktop GUI export job must
    /// produce the same pixels for the same scene: both emulate the sRGB
    /// surface the viewport composites with (the CLI fix for washed-out
    /// colors; the offscreen capture is always linear). Skipped without a
    /// headless GPU adapter.
    #[test]
    fn cli_export_matches_gui_export_job() {
        for color in [[128u8, 128, 128], [30, 200, 90]] {
            let bundle = material_cube(color);
            let o = options();
            let dir = tempfile::tempdir().unwrap();
            // The CLI has no camera flag: it fits the default camera, so the
            // GUI leg must use the same camera-less path for the comparison.
            let gui = match export(
                bundle.clone(),
                None,
                None,
                &dir.path().join("gui.png"),
                &o,
                true,
                "parity.cdraft",
                false,
                &Control::default(),
            ) {
                Ok(image) => image,
                Err(e) if adapter_unavailable(&e) => {
                    eprintln!("SKIP PNG GPU checks: {e}");
                    return;
                }
                Err(e) => panic!("{e}"),
            };
            let dir = tempfile::tempdir().unwrap();
            let input = dir.path().join("parity.cdraft");
            let output = dir.path().join("cli.png");
            cindral_io::save_draft(&input, &bundle).unwrap();
            crate::png_cli::run([
                std::ffi::OsString::from("--export-png"),
                input.into_os_string(),
                output.clone().into_os_string(),
                "--width".into(),
                "64".into(),
                "--height".into(),
                "64".into(),
                "--aa".into(),
                "off".into(),
            ])
            .unwrap();
            let mut reader = png::Decoder::new(std::io::BufReader::new(
                std::fs::File::open(output).unwrap(),
            ))
            .read_info()
            .unwrap();
            let mut decoded = vec![0; 64 * 64 * 4];
            reader.next_frame(&mut decoded).unwrap();
            assert!(
                decoded
                    .iter()
                    .zip(&gui.rgba)
                    .all(|(&a, &b)| a.abs_diff(b) <= 2),
                "{color:?}: CLI export differs from GUI export job"
            );
        }
    }

    fn bg(v: u8, srgb: bool) -> f64 {
        let v = f64::from(v) / 255.;
        if srgb {
            ((v + 0.055) / 1.055).powf(2.4)
        } else {
            v
        }
    }
    fn viewport_callback(app: &mut DrafterApp, srgb: bool) -> Vec<u8> {
        use eframe::egui_wgpu;
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::from_env_or_default());
        let adapter = block_on(instance.request_adapter(&Default::default())).unwrap();
        let (device, queue) = block_on(adapter.request_device(&Default::default())).unwrap();
        let format = if srgb {
            wgpu::TextureFormat::Rgba8UnormSrgb
        } else {
            wgpu::TextureFormat::Rgba8Unorm
        };
        let mut renderer = egui_wgpu::Renderer::new(
            &device,
            format,
            egui_wgpu::RendererOptions {
                dithering: false,
                ..Default::default()
            },
        );
        renderer
            .callback_resources
            .insert(SceneRenderer::new(&device, format, 1));
        let ctx = egui::Context::default();
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(64., 64.),
                )),
                ..Default::default()
            },
            |ctx| {
                let painter = ctx.layer_painter(egui::LayerId::background());
                painter.add(egui_wgpu::Callback::new_paint_callback(
                    egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(64., 64.)),
                    cindral_render::SceneCallback {
                        view_projection: app.camera.view_projection(1.).to_cols_array_2d(),
                        size: [64, 64],
                        update: app.pending.take(),
                        style: app.presentation.style.clone(),
                        section: None,
                        caps: Default::default(),
                        shadows: app.presentation.shadows.clone(),
                        bounds: app.section_ui.bounds,
                    },
                ));
            },
        );
        let jobs = ctx.tessellate(output.shapes, 1.);
        for (id, delta) in output.textures_delta.set {
            renderer.update_texture(&device, &queue, id, &delta);
        }
        let target = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("PNG viewport regression"),
            size: wgpu::Extent3d {
                width: 64,
                height: 64,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let mut encoder = device.create_command_encoder(&Default::default());
        let screen = egui_wgpu::ScreenDescriptor {
            size_in_pixels: [64, 64],
            pixels_per_point: 1.,
        };
        let mut commands = renderer.update_buffers(&device, &queue, &mut encoder, &jobs, &screen);
        {
            let view = target.create_view(&Default::default());
            let mut pass = encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: None,
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        resolve_target: None,
                        depth_slice: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color {
                                r: bg(237, srgb),
                                g: bg(240, srgb),
                                b: bg(245, srgb),
                                a: 1.,
                            }),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                })
                .forget_lifetime();
            renderer.render(&mut pass, &jobs, &screen);
        }
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: 64 * 256,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        encoder.copy_texture_to_buffer(
            target.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(256),
                    rows_per_image: Some(64),
                },
            },
            wgpu::Extent3d {
                width: 64,
                height: 64,
                depth_or_array_layers: 1,
            },
        );
        commands.push(encoder.finish());
        queue.submit(commands);
        let (tx, rx) = std::sync::mpsc::channel();
        buffer.slice(..).map_async(wgpu::MapMode::Read, move |r| {
            tx.send(r).unwrap();
        });
        device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: Some(std::time::Duration::from_secs(30)),
            })
            .unwrap();
        rx.recv().unwrap().unwrap();
        let pixels = buffer.slice(..).get_mapped_range().to_vec();
        buffer.unmap();
        pixels
    }

    /// An oversized texture array fails inside the wgpu error scope with a
    /// readable message; the previous array and bindings stay in use and a
    /// valid update afterwards still succeeds. The CPU document is never
    /// touched by a GPU allocation failure.
    #[test]
    fn texture_array_allocation_failure_reports_error_and_keeps_previous_array() {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::from_env_or_default());
        let adapter = match block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            compatible_surface: None,
            ..Default::default()
        })) {
            Ok(adapter) => adapter,
            Err(e) => {
                eprintln!("skipping: no wgpu adapter ({e})");
                return;
            }
        };
        let (device, queue) = match block_on(adapter.request_device(&Default::default())) {
            Ok(pair) => pair,
            Err(e) => {
                eprintln!("skipping: no wgpu device ({e})");
                return;
            }
        };
        let mut renderer = SceneRenderer::new(&device, wgpu::TextureFormat::Rgba8Unorm, 1);
        let limit = device.limits().max_texture_dimension_2d;
        let oversized = cindral_render::SceneUpdate {
            textures: Some(std::sync::Arc::new(cindral_render::TextureArrayData {
                edge: limit + 1,
                layers: vec![cindral_render::TextureMips {
                    levels: vec![cindral_render::TexturePixels::new(
                        limit + 1,
                        limit + 1,
                        Vec::new(),
                    )],
                }],
            })),
            ..Default::default()
        };
        renderer.apply_update(&device, &queue, &oversized);
        let errors = renderer
            .texture_errors
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        assert_eq!(errors.len(), 1, "allocation failure must be reported");
        assert!(errors[0].contains("materials texture array allocation failed"));

        // The renderer survives with the previous (default) array, and a
        // valid update replaces it without a new error.
        let valid = cindral_render::SceneUpdate {
            textures: Some(std::sync::Arc::new(cindral_render::TextureArrayData {
                edge: 1024,
                layers: vec![
                    cindral_render::TextureMips::white(1024),
                    cindral_render::TextureMips::white(1024),
                ],
            })),
            ..Default::default()
        };
        renderer.apply_update(&device, &queue, &valid);
        let errors = renderer
            .texture_errors
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        assert_eq!(errors.len(), 1, "valid update must not add errors");
    }
}
