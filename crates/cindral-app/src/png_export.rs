//! Bounded PNG options, conversion and cancellable atomic publication.
use cindral_render::{
    camera::{Camera, Projection, FOV_Y},
    capture::row_stride,
};
use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU8, Ordering},
        Arc, Mutex,
    },
};

/// Maximum working render dimension (including supersampling).
pub const MAX_DIMENSION: u32 = 8192;
/// Export working-set budget; reserves additional space for the already running editor.
pub const MEMORY_CAP: u64 = 512 * 1024 * 1024;
/// Supersampling per dimension; no new GPU pipelines.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Aa {
    /// One sample per output pixel.
    Off,
    /// Four samples per output pixel.
    #[default]
    Medium,
    /// Nine samples per output pixel.
    High,
}
impl Aa {
    /// Linear render-size multiplier.
    pub fn scale(self) -> u32 {
        match self {
            Self::Off => 1,
            Self::Medium => 2,
            Self::High => 3,
        }
    }
}
/// Persistent export preferences. Overwrite authorization is never persisted.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Options {
    /// Application chrome only; never used by an exporter.
    pub theme: crate::theme::Preference,
    /// Output width.
    pub width: u32,
    /// Output height.
    pub height: u32,
    /// Quality.
    pub aa: Aa,
    /// Omit environment fill.
    pub transparent: bool,
    /// Match the physical viewport on the next export.
    pub match_window: bool,
    /// Editing context snapshot, excluded from persistent settings and PNG metadata.
    #[serde(skip)]
    pub context: Vec<cindral_core::InstanceId>,
    /// Last chosen directory (never embedded in the image).
    pub folder: PathBuf,
    /// Recently used material colors (display sRGB bytes), newest first.
    pub recent_colors: Vec<[u8; 3]>,
    /// Material-library folders scanned for user image collections.
    pub library_folders: Vec<String>,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            theme: Default::default(),
            width: 1920,
            height: 1080,
            aa: Aa::Medium,
            transparent: false,
            match_window: true,
            context: Vec::new(),
            folder: PathBuf::new(),
            recent_colors: Vec::new(),
            library_folders: Vec::new(),
        }
    }
}
impl Options {
    /// Records a used color: deduplicates, newest first, capped at 12.
    pub fn push_recent_color(&mut self, color: [u8; 3]) {
        self.recent_colors.retain(|c| *c != color);
        self.recent_colors.insert(0, color);
        self.recent_colors.truncate(12);
    }
    /// Conservative peak estimate: GPU color/depth, padded readback and CPU copy,
    /// final pixels/encoder, upload geometry headroom, plus 128 MiB textures/shadows/headroom.
    pub fn estimate(&self, geometry_bytes: u64) -> Result<([u32; 2], u64), String> {
        let w = self
            .width
            .checked_mul(self.aa.scale())
            .ok_or("PNG size overflow")?;
        let h = self
            .height
            .checked_mul(self.aa.scale())
            .ok_or("PNG size overflow")?;
        if w == 0 || h == 0 || w > MAX_DIMENSION || h > MAX_DIMENSION {
            return Err("PNG render dimension must be 1..8192 including AA (Medium ×2, High ×3); reduce size or AA".into());
        }
        let aspect = self.width as f32 / self.height as f32;
        if !(0.001..=1000.).contains(&aspect) {
            return Err("PNG aspect must be between 1:1000 and 1000:1 (camera limit)".into());
        }
        let bytes = u64::from(w) * u64::from(h) * 12
            + u64::from(row_stride(w)) * u64::from(h)
            + u64::from(self.width) * u64::from(self.height) * 8
            + 128 * 1024 * 1024;
        let bytes = bytes
            .checked_add(
                geometry_bytes
                    .checked_mul(4)
                    .ok_or("PNG memory estimate overflow")?,
            )
            .ok_or("PNG memory estimate overflow")?;
        if bytes > MEMORY_CAP {
            return Err(format!(
                "PNG estimate {} MiB exceeds 512 MiB budget; reduce size or AA",
                bytes.div_ceil(1024 * 1024)
            ));
        }
        Ok(([w, h], bytes))
    }
    /// Load a complete valid preference file; malformed/interrupted input falls back safely.
    pub fn load(path: &Path) -> Self {
        std::fs::read(path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Self>(&bytes).ok())
            .filter(|v| v.estimate(0).is_ok())
            .unwrap_or_default()
    }
    /// Crash-safe replacement in the same directory.
    pub fn save(&self, path: &Path) -> Result<(), String> {
        use std::io::Write;
        let parent = path.parent().ok_or("preferences need a directory")?;
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        let mut file = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
        file.write_all(&serde_json::to_vec(self).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        file.as_file().sync_all().map_err(|e| e.to_string())?;
        file.persist(path).map_err(|e| e.to_string())?;
        Ok(())
    }
}
#[derive(Default)]
struct State {
    cancelled: bool,
    committed: bool,
}
/// Cancellation is serialized with final publication; a successful cancel cannot leave output.
#[derive(Clone, Default)]
pub struct Control {
    state: Arc<Mutex<State>>,
    progress: Arc<AtomicU8>,
}
impl Control {
    /// Cancel until the atomic publication boundary. False means already published.
    pub fn cancel(&self) -> bool {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if s.committed {
            return false;
        }
        s.cancelled = true;
        true
    }
    /// Stop work at phase/row boundaries.
    pub fn check(&self) -> Result<(), String> {
        if self
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .cancelled
        {
            Err("PNG export cancelled".into())
        } else {
            Ok(())
        }
    }
    pub(crate) fn set_progress(&self, v: u8) {
        self.progress.store(v, Ordering::Relaxed);
    }
    /// Completed phase fraction (GPU commands already submitted finish before cancellation).
    pub fn progress(&self) -> f32 {
        f32::from(self.progress.load(Ordering::Relaxed)) / 100.
    }
}
/// Projection policy: preserve vertical FOV/orthographic height, extending/cropping the sides.
pub fn framing(camera: &Camera, size: [u32; 2]) -> glam::Mat4 {
    camera.view_projection(size[0] as f32 / size[1] as f32)
}
/// Shared viewport horizon as a fraction from the top, with no camera roll.
pub fn horizon(camera: &Camera) -> f32 {
    if camera.projection == Projection::Orthographic {
        if camera.pitch >= 0. {
            0.
        } else {
            1.
        }
    } else {
        (0.5 - (camera.pitch.sin() / camera.pitch.cos().max(1e-6)) / (FOV_Y * 0.5).tan() * 0.5)
            .clamp(0., 1.)
    }
}
fn linear(v: f32) -> f32 {
    if v <= 0.04045 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}
fn srgb(v: f32) -> f32 {
    if v <= 0.0031308 {
        12.92 * v
    } else {
        1.055 * v.powf(1. / 2.4) - 0.055
    }
}
/// Resolve premultiplied samples, composite the viewport environment and unpremultiply
/// once. The normal egui gamma framebuffer stores scene values directly; sRGB surfaces
/// apply their hardware transfer after linear blending. No second transfer in gamma mode.
pub fn resolve(
    raw: &[u8],
    options: &Options,
    camera: &Camera,
    style: &cindral_core::ViewStyle,
    srgb_surface: bool,
    control: &Control,
) -> Result<Vec<u8>, String> {
    let ([w, h], _) = options.estimate(0)?;
    if raw.len() != w as usize * h as usize * 4 {
        return Err("incomplete PNG pixels".into());
    }
    let scale = options.aa.scale();
    let horizon = horizon(camera);
    let mut output = vec![0; options.width as usize * options.height as usize * 4];
    for y in 0..options.height {
        control.check()?;
        for x in 0..options.width {
            let mut total = [0f32; 4];
            for sy in 0..scale {
                for sx in 0..scale {
                    let yy = y * scale + sy;
                    let i = ((yy * w + x * scale + sx) * 4) as usize;
                    let alpha = raw[i + 3] as f32 / 255.;
                    let mut rgb = [
                        raw[i] as f32 / 255.,
                        raw[i + 1] as f32 / 255.,
                        raw[i + 2] as f32 / 255.,
                    ];
                    let a = if options.transparent {
                        alpha
                    } else {
                        let bg = if (yy as f32 + 0.5) / (h as f32) < horizon {
                            style.sky
                        } else {
                            style.ground
                        }
                        .unwrap_or(style.background);
                        for c in 0..3 {
                            let b = bg[c] as f32 / 255.;
                            rgb[c] += (if srgb_surface { linear(b) } else { b }) * (1. - alpha);
                        }
                        1.
                    };
                    // Keep premultiplied values through the filter; transparent RGB becomes zero.
                    for c in 0..3 {
                        total[c] += rgb[c];
                    }
                    total[3] += a;
                }
            }
            let n = (scale * scale) as f32;
            let a = total[3] / n;
            let out = ((y * options.width + x) * 4) as usize;
            for c in 0..3 {
                let v = if a > 0. { total[c] / n / a } else { 0. };
                output[out + c] =
                    ((if srgb_surface { srgb(v) } else { v }).clamp(0., 1.) * 255.).round() as u8;
            }
            output[out + 3] = (a * 255.).round() as u8;
        }
        control.set_progress(65 + (y * 25 / options.height) as u8);
    }
    Ok(output)
}
/// Validate destination before touching the GPU. Publication checks again without a race.
pub fn destination(path: &Path, force: bool) -> Result<(), String> {
    if path.file_name().is_none() {
        return Err("PNG destination needs a filename".into());
    }
    if path.exists() && !force {
        return Err(format!(
            "{} already exists; confirm replacement or pass --force",
            path.display()
        ));
    }
    if path.is_dir() {
        return Err("PNG destination is a directory".into());
    }
    Ok(())
}
/// Encode metadata and pixels into a temporary file, then publish atomically.
pub fn write_png(
    path: &Path,
    options: &Options,
    rgba: &[u8],
    document: &str,
    force: bool,
    control: &Control,
) -> Result<(), String> {
    options.estimate(0)?;
    if rgba.len() != options.width as usize * options.height as usize * 4 {
        return Err("incomplete PNG pixels".into());
    }
    destination(path, force)?;
    control.check()?;
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut temp = tempfile::NamedTempFile::new_in(parent)
        .map_err(|e| format!("PNG destination {}: {e}", path.display()))?;
    {
        let mut encoder = png::Encoder::new(temp.as_file_mut(), options.width, options.height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_source_srgb(png::SrgbRenderingIntent::Perceptual);
        encoder
            .add_text_chunk("Software".into(), "Cindral Drafter".into())
            .map_err(|e| e.to_string())?;
        let name = Path::new(document)
            .file_name()
            .unwrap_or_default()
            .to_string_lossy();
        encoder
            .add_itxt_chunk("Document".into(), name.into_owned())
            .map_err(|e| e.to_string())?;
        let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
        {
            use std::io::Write;
            let mut stream = writer.stream_writer().map_err(|e| e.to_string())?;
            for row in rgba.chunks(options.width as usize * 4) {
                control.check()?;
                stream
                    .write_all(row)
                    .map_err(|e| format!("PNG write failed: {e}"))?;
            }
            stream.finish().map_err(|e| e.to_string())?;
        }
        writer.finish().map_err(|e| e.to_string())?;
    }
    temp.as_file().sync_all().map_err(|e| e.to_string())?;
    let mut state = control.state.lock().unwrap_or_else(|e| e.into_inner());
    if state.cancelled {
        return Err("PNG export cancelled".into());
    }
    if force {
        temp.persist(path)
    } else {
        temp.persist_noclobber(path)
    }
    .map_err(|e| format!("PNG publication failed: {e}"))?;
    state.committed = true;
    control.set_progress(100);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn small() -> Options {
        Options {
            width: 2,
            height: 2,
            aa: Aa::Off,
            match_window: false,
            ..Default::default()
        }
    }
    #[test]
    fn size_and_memory_limits() {
        let mut o = small();
        assert_eq!(o.estimate(0).unwrap().0, [2, 2]);
        for (w, h) in [(0, 1), (1, 0), (u32::MAX, 1), (8193, 1)] {
            o.width = w;
            o.height = h;
            assert!(o.estimate(0).is_err());
        }
        o.width = 1920;
        o.height = 1080;
        o.aa = Aa::High;
        assert_eq!(o.estimate(0).unwrap().0, [5760, 3240]);
        o.width = 3840;
        assert!(o.estimate(0).is_err());
        o = small();
        assert!(o.estimate(MEMORY_CAP).is_err());
    }
    #[test]
    fn vertical_framing_does_not_change_camera_or_rays() {
        for projection in [Projection::Perspective, Projection::Orthographic] {
            let camera = Camera {
                projection,
                ..Default::default()
            };
            let a = framing(&camera, [800, 800]);
            let b = framing(&camera, [1600, 800]);
            assert!((a.y_axis.y - b.y_axis.y).abs() < 1e-6);
            assert!((a.x_axis.x - b.x_axis.x * 2.).abs() < 1e-6);
            assert_eq!(a, camera.view_projection(1.));
            assert_eq!(horizon(&camera), horizon(&camera.clone()));
        }
    }
    #[test]
    fn gamma_surface_alpha_and_filtering() {
        let mut o = small();
        o.transparent = true;
        let raw = [64, 32, 16, 128, 0, 0, 0, 0, 255, 0, 0, 255, 0, 255, 0, 255];
        let style = Default::default();
        let control = Control::default();
        let camera = Camera::default();
        let out = resolve(&raw, &o, &camera, &style, false, &control).unwrap();
        assert_eq!(&out[..8], &[128, 64, 32, 128, 0, 0, 0, 0]);
        let out = resolve(&raw, &o, &camera, &style, true, &control).unwrap();
        assert!((out[0] as i32 - 187).abs() <= 1);
        o.width = 1;
        o.height = 1;
        o.aa = Aa::Medium;
        let out = resolve(&raw, &o, &camera, &style, false, &control).unwrap();
        assert_eq!(out[3], 160);
        assert_eq!(out[0], 128); // premultiplied average, then divide by total alpha
    }
    #[test]
    fn preferences_roundtrip_and_bad_file_fallback() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("prefs.json");
        let mut o = small();
        o.transparent = true;
        o.folder = dir.path().join("spaces and unicode 房");
        o.aa = Aa::High;
        o.save(&path).unwrap();
        assert_eq!(Options::load(&path), o);
        std::fs::write(&path, b"{interrupted").unwrap();
        assert_eq!(Options::load(&path), Options::default());
    }
    #[test]
    fn atomic_cancel_overwrite_failure_metadata_and_dimensions() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out.png");
        let options = small();
        let pixels = vec![255; 16];
        let cancelled = Control::default();
        assert!(cancelled.cancel());
        assert!(write_png(
            &path,
            &options,
            &pixels,
            "private/model.cdraft",
            false,
            &cancelled
        )
        .is_err());
        assert!(!path.exists());
        let control = Control::default();
        write_png(
            &path,
            &options,
            &pixels,
            "private/model.cdraft",
            false,
            &control,
        )
        .unwrap();
        assert!(!control.cancel());
        let old = std::fs::read(&path).unwrap();
        assert!(write_png(&path, &options, &pixels, "x", false, &Control::default()).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), old);
        let decoder =
            png::Decoder::new(std::io::BufReader::new(std::fs::File::open(&path).unwrap()));
        let reader = decoder.read_info().unwrap();
        assert_eq!((reader.info().width, reader.info().height), (2, 2));
        assert_eq!(
            reader
                .info()
                .uncompressed_latin1_text
                .iter()
                .map(|t| (&*t.keyword, &*t.text))
                .collect::<Vec<_>>(),
            vec![("Software", "Cindral Drafter")]
        );
        assert_eq!(reader.info().utf8_text[0].keyword, "Document");
        assert_eq!(
            reader.info().utf8_text[0].get_text().unwrap(),
            "model.cdraft"
        );
        let missing = dir.path().join("missing/out.png");
        assert!(write_png(&missing, &options, &pixels, "x", false, &Control::default()).is_err());
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
        write_png(
            &path,
            &options,
            &pixels,
            "房间.cdraft",
            true,
            &Control::default(),
        )
        .unwrap();
    }
    #[cfg(unix)]
    #[test]
    fn unwritable_destination_and_no_partial_file() {
        // /proc is unwritable even when tests execute as root.
        let path = Path::new("/proc/cindral-png-export-test.png");
        assert!(write_png(path, &small(), &[0; 16], "x", false, &Control::default()).is_err());
        assert!(!path.exists());
    }
}
#[cfg(test)]
mod cancellation_test {
    #[test]
    fn concurrent_cancel_never_publishes_after_successful_cancel() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("race.png");
        let options = super::Options {
            width: 1024,
            height: 512,
            aa: super::Aa::Off,
            ..Default::default()
        };
        let control = super::Control::default();
        let worker = control.clone();
        let destination = path.clone();
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let start = barrier.clone();
        let thread = std::thread::spawn(move || {
            let rgba = vec![123; 1024 * 512 * 4];
            start.wait();
            super::write_png(&destination, &options, &rgba, "test", false, &worker)
        });
        barrier.wait();
        let cancelled = control.cancel();
        let result = thread.join().unwrap();
        if cancelled {
            assert!(result.is_err());
            assert!(!path.exists());
            assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
        } else {
            assert!(result.is_ok());
            assert!(path.exists());
        }
    }
}
