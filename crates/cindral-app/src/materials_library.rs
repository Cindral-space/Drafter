//! User material library: off-UI-thread folder scanning and 64x64 thumbnailing.
//!
//! [`scan_folder`] is the synchronous reference implementation; [`LibraryScanJob`]
//! runs it on a dedicated worker thread so the UI never blocks, and
//! [`LibraryCache`] keeps one in-flight job per library folder.

use eframe::egui::{Color32, ColorImage};
use image::ImageReader;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver};
use std::sync::{Arc, Mutex};

/// Maximum number of image files considered in one folder scan.
const MAX_FILES: usize = 2000;
/// Maximum decoded image width/height accepted by the reader limits.
const MAX_DECODE_DIM: u32 = 8192;
/// Maximum allocation (in bytes) the decoder may make for one image.
const MAX_DECODE_ALLOC: usize = 256 * 1024 * 1024;
/// Thumbnail edge length in pixels.
const THUMB_SIZE: u32 = 64;

/// One material image with its pre-rendered 64x64 RGBA thumbnail.
#[derive(Clone)]
pub struct LibraryImage {
    /// Full path of the source image file.
    pub path: PathBuf,
    /// File name (with extension) as shown in the library UI.
    pub name: String,
    /// 64x64 RGBA thumbnail.
    pub thumb: ColorImage,
}

/// Result of scanning one library folder.
#[allow(dead_code)] // folder retained for callers that key scans by origin
#[derive(Clone, Default)]
pub struct LibraryScan {
    /// The folder that was scanned.
    pub folder: PathBuf,
    /// Decoded images, sorted by file name.
    pub images: Vec<LibraryImage>,
    /// Files that were present but skipped (unreadable, corrupt, oversized,
    /// duplicate basename).
    pub skipped: usize,
}

impl LibraryScan {
    fn empty(folder: &Path) -> Self {
        LibraryScan {
            folder: folder.to_path_buf(),
            images: Vec::new(),
            skipped: 0,
        }
    }
}

/// Synchronously scans `folder` (non-recursive) and builds thumbnails.
///
/// Only direct children with `png`/`jpg`/`jpeg` extensions (case-insensitive)
/// are decoded, capped at [`MAX_FILES`] entries (overflow beyond the cap is
/// simply not scanned). Files that fail to read or decode, non-image files and
/// duplicate basenames are counted in [`LibraryScan::skipped`] and never panic.
/// Duplicate basenames across extensions keep the first entry in sort order.
#[allow(dead_code)] // synchronous reference implementation; workers use the cancellable variant
pub fn scan_folder(folder: &Path) -> LibraryScan {
    scan_folder_cancellable(folder, &AtomicBool::new(false))
}

fn scan_folder_cancellable(folder: &Path, cancel: &AtomicBool) -> LibraryScan {
    let entries = match std::fs::read_dir(folder) {
        Ok(entries) => entries,
        // Unreadable/nonexistent folder: empty scan, no panic.
        Err(_) => return LibraryScan::empty(folder),
    };
    let mut candidates: Vec<(String, PathBuf)> = Vec::new();
    let mut skipped = 0usize;
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase());
        match ext.as_deref() {
            Some("png" | "jpg" | "jpeg") => candidates.push((name.to_string(), path)),
            // Any other regular file (e.g. README.txt) counts as skipped.
            _ => skipped += 1,
        }
    }
    candidates.sort_by(|a, b| a.0.cmp(&b.0));
    candidates.truncate(MAX_FILES);

    let mut scan = LibraryScan {
        folder: folder.to_path_buf(),
        images: Vec::new(),
        skipped,
    };
    let mut seen_stems: HashSet<String> = HashSet::new();
    for (name, path) in candidates {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let stem_key = Path::new(&name)
            .file_stem()
            .and_then(|s| s.to_str())
            .map(|s| s.to_ascii_lowercase())
            .unwrap_or_default();
        if !seen_stems.insert(stem_key) {
            scan.skipped += 1;
            continue;
        }
        match load_thumbnail(&path) {
            Ok(thumb) => scan.images.push(LibraryImage { path, name, thumb }),
            Err(()) => scan.skipped += 1,
        }
    }
    scan
}

/// Decodes one image file and downsamples it to a 64x64 thumbnail.
///
/// Any read or decode failure (missing file, truncated data, dimensions above
/// the reader limits) is reported as `Err(())`; this function never panics.
fn load_thumbnail(path: &Path) -> Result<ColorImage, ()> {
    let mut reader = ImageReader::open(path).map_err(|_| ())?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_DECODE_DIM);
    limits.max_image_height = Some(MAX_DECODE_DIM);
    limits.max_alloc = Some(MAX_DECODE_ALLOC as u64);
    reader.limits(limits);
    let reader = reader.with_guessed_format().map_err(|_| ())?;
    let img = reader.decode().map_err(|_| ())?;
    let rgba = img.to_rgba8();
    let (w, h) = (rgba.width(), rgba.height());
    Ok(downsample_64(rgba.as_raw(), w, h))
}

/// Downsamples an RGBA8 buffer to a 64x64 thumbnail by box averaging.
///
/// Every output pixel averages all source pixels falling in its
/// `w/64 * h/64` block, so the full source is covered exactly once. Guarded
/// against degenerate `w == 0 || h == 0` inputs (returns a transparent image).
pub fn downsample_64(rgba: &[u8], w: u32, h: u32) -> ColorImage {
    let mut pixels = Vec::with_capacity((THUMB_SIZE * THUMB_SIZE) as usize);
    if w == 0 || h == 0 || rgba.len() < (w as usize) * (h as usize) * 4 {
        pixels.resize((THUMB_SIZE * THUMB_SIZE) as usize, Color32::TRANSPARENT);
        return ColorImage::new([THUMB_SIZE as usize, THUMB_SIZE as usize], pixels);
    }
    let (w, h) = (w as usize, h as usize);
    let out = THUMB_SIZE as usize;
    for oy in 0..out {
        let y0 = oy * h / out;
        let y1 = ((oy + 1) * h / out).max(y0 + 1);
        for ox in 0..out {
            let x0 = ox * w / out;
            let x1 = ((ox + 1) * w / out).max(x0 + 1);
            let (mut r, mut g, mut b, mut a) = (0u64, 0u64, 0u64, 0u64);
            for y in y0..y1 {
                let row = y * w * 4;
                for x in x0..x1 {
                    let px = row + x * 4;
                    r += rgba[px] as u64;
                    g += rgba[px + 1] as u64;
                    b += rgba[px + 2] as u64;
                    a += rgba[px + 3] as u64;
                }
            }
            let n = ((x1 - x0) * (y1 - y0)) as u64;
            pixels.push(Color32::from_rgba_unmultiplied(
                (r / n) as u8,
                (g / n) as u8,
                (b / n) as u8,
                (a / n) as u8,
            ));
        }
    }
    ColorImage::new([out, out], pixels)
}

/// Asynchronous folder scan running on a dedicated worker thread.
pub struct LibraryScanJob {
    folder: PathBuf,
    rx: Receiver<LibraryScan>,
    cancel: Arc<AtomicBool>,
    done: bool,
}

impl LibraryScanJob {
    /// Spawns a scan of `folder` on a thread named `cindral-materials-library`.
    pub fn spawn(folder: PathBuf) -> Self {
        let (tx, rx) = channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let thread_cancel = Arc::clone(&cancel);
        let thread_folder = folder.clone();
        let thread_tx = tx.clone();
        let handle = std::thread::Builder::new()
            .name("cindral-materials-library".to_string())
            .spawn(move || {
                // Never let a worker panic propagate to the process; a failed
                // scan degrades to an empty result.
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    scan_folder_cancellable(&thread_folder, &thread_cancel)
                }))
                .unwrap_or_else(|_| LibraryScan::empty(&thread_folder));
                let _ = thread_tx.send(result);
            });
        // If the thread could not even be spawned, report an empty scan.
        if handle.is_err() {
            let _ = tx.send(LibraryScan::empty(&folder));
        }
        LibraryScanJob {
            folder,
            rx,
            cancel,
            done: false,
        }
    }

    /// Non-blocking poll: `None` while running, `Some` exactly once when the
    /// scan has finished; later calls return `None` again.
    pub fn poll(&mut self) -> Option<LibraryScan> {
        if self.done {
            return None;
        }
        match self.rx.try_recv() {
            Ok(scan) => {
                self.done = true;
                Some(scan)
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => None,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                // Defensive: the worker always sends via catch_unwind, so a
                // disconnected channel means it never ran; report empty.
                self.done = true;
                Some(LibraryScan::empty(&self.folder))
            }
        }
    }

    /// Requests an early stop; the worker finishes the file in flight and
    /// returns the partial scan. The result still arrives via [`poll`].
    pub fn cancel(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

/// Keeps one scan (and at most one in-flight job) per registered folder.
pub struct LibraryCache {
    folders: Vec<PathBuf>,
    scans: HashMap<PathBuf, LibraryScan>,
    jobs: Mutex<HashMap<PathBuf, LibraryScanJob>>,
}

impl LibraryCache {
    /// Creates a cache pre-registered with `folders` (no scans yet).
    pub fn new(folders: Vec<PathBuf>) -> Self {
        LibraryCache {
            folders,
            scans: HashMap::new(),
            jobs: Mutex::new(HashMap::new()),
        }
    }

    /// Starts (or keeps) the in-flight scan job for `folder`.
    ///
    /// Only one job per folder runs at a time; a second call while a job is
    /// still running is a no-op. The stored scan is replaced when the job
    /// completes through [`LibraryCache::poll`].
    pub fn refresh(&mut self, folder: &Path) {
        let mut jobs = self.jobs.lock().expect("library cache job map poisoned");
        if !jobs.contains_key(folder) {
            jobs.insert(
                folder.to_path_buf(),
                LibraryScanJob::spawn(folder.to_path_buf()),
            );
        }
    }

    /// Polls all in-flight jobs, storing each finished scan.
    pub fn poll(&mut self) {
        let mut jobs = self.jobs.lock().expect("library cache job map poisoned");
        let keys: Vec<PathBuf> = jobs.keys().cloned().collect();
        let mut finished: Vec<(PathBuf, LibraryScan)> = Vec::new();
        for key in keys {
            if let Some(job) = jobs.get_mut(&key) {
                if let Some(scan) = job.poll() {
                    finished.push((key, scan));
                }
            }
        }
        for (key, scan) in finished {
            jobs.remove(&key);
            self.scans.insert(key, scan);
        }
    }

    /// The stored scan for `folder`, if one has completed.
    pub fn scan(&self, folder: &Path) -> Option<&LibraryScan> {
        self.scans.get(folder)
    }

    /// All registered library folders.
    pub fn folders(&self) -> &[PathBuf] {
        &self.folders
    }

    /// Registers `folder` (no-op if already registered).
    pub fn add_folder(&mut self, folder: PathBuf) {
        if !self.folders.contains(&folder) {
            self.folders.push(folder);
        }
    }

    /// Unregisters `folder`, dropping its stored scan and cancelling any
    /// in-flight job.
    pub fn remove_folder(&mut self, folder: &Path) {
        self.folders.retain(|f| f != folder);
        self.scans.remove(folder);
        if let Some(mut job) = self
            .jobs
            .lock()
            .expect("library cache job map poisoned")
            .remove(folder)
        {
            job.cancel();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::time::{Duration, Instant};

    /// Writes a valid RGB PNG built with the `image` crate.
    fn write_png(path: &Path, size: u32, color: [u8; 3]) {
        let img = image::RgbImage::from_fn(size, size, |_, _| image::Rgb(color));
        img.save(path).expect("png save");
    }

    fn unique_temp_dir(label: &str) -> PathBuf {
        let base = std::env::temp_dir();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let dir = base.join(format!(
            "cindral-matlib-{label}-{}-{nanos}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("mkdir");
        dir
    }

    fn wait_for_scan(cache: &mut LibraryCache, folder: &Path) -> bool {
        let deadline = Instant::now() + Duration::from_secs(30);
        while Instant::now() < deadline {
            cache.poll();
            if cache.scan(folder).is_some() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        false
    }

    #[test]
    fn scan_counts_valid_corrupt_and_nonimage_files() {
        let dir = unique_temp_dir("basic");
        write_png(&dir.join("a.png"), 4, [255, 0, 0]);
        write_png(&dir.join("b.png"), 4, [0, 255, 0]);
        write_png(&dir.join("c.jpg"), 4, [0, 0, 255]);
        std::fs::File::create(dir.join("corrupt.png"))
            .and_then(|mut f| f.write_all(b"not a png at all"))
            .expect("write corrupt");
        std::fs::write(dir.join("note.txt"), b"ignore me").expect("write txt");

        let scan = scan_folder(&dir);
        assert_eq!(scan.images.len(), 3);
        assert_eq!(scan.skipped, 2, "corrupt.png + note.txt (non-image)");
        let names: Vec<&str> = scan.images.iter().map(|i| i.name.as_str()).collect();
        assert_eq!(names, vec!["a.png", "b.png", "c.jpg"], "sorted by name");
        for img in &scan.images {
            assert_eq!(img.thumb.size, [64, 64]);
            assert_eq!(img.thumb.pixels.len(), 64 * 64);
        }
        std::fs::remove_dir_all(&dir).expect("cleanup");
    }

    #[test]
    fn duplicate_basenames_keep_first_in_sort_order() {
        let dir = unique_temp_dir("dupes");
        write_png(&dir.join("rock.png"), 2, [255, 0, 0]);
        write_png(&dir.join("rock.jpg"), 2, [0, 255, 0]);
        let scan = scan_folder(&dir);
        assert_eq!(scan.images.len(), 1);
        assert_eq!(scan.images[0].name, "rock.jpg", ".jpg sorts before .png");
        assert_eq!(scan.skipped, 1);
        std::fs::remove_dir_all(&dir).expect("cleanup");
    }

    /// Generating 2050 files is slow; run once via `cargo test -- --ignored`.
    #[test]
    #[ignore = "slow: writes 2050 files"]
    fn scan_is_capped_at_2000_files() {
        let dir = unique_temp_dir("cap");
        let tiny = image::RgbImage::from_fn(1, 1, |_, _| image::Rgb([10, 20, 30]));
        let tiny_path = dir.join("template.png");
        tiny.save(&tiny_path).expect("template png");
        let bytes = std::fs::read(&tiny_path).expect("read template");
        std::fs::remove_file(&tiny_path).expect("remove template");
        for i in 0..2050u32 {
            std::fs::write(dir.join(format!("f{i:05}.png")), &bytes).expect("write tiny");
        }
        let scan = scan_folder(&dir);
        assert_eq!(scan.images.len(), 2000);
        std::fs::remove_dir_all(&dir).expect("cleanup");
    }

    /// A PNG header that declares 9000x9000 (above the 8192 limit) must be
    /// skipped by the reader limits, not decoded or panicked on. The file is
    /// truncated after IHDR so the full pixel data is never written.
    #[test]
    fn oversized_declared_dimensions_are_skipped_not_panic() {
        let dir = unique_temp_dir("limits");
        let path = dir.join("huge.png");
        let mut f = std::fs::File::create(&path).expect("create");
        f.write_all(&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A])
            .expect("signature");
        let mut ihdr = Vec::new();
        ihdr.extend_from_slice(&13u32.to_be_bytes());
        ihdr.extend_from_slice(b"IHDR");
        ihdr.extend_from_slice(&9000u32.to_be_bytes()); // width
        ihdr.extend_from_slice(&9000u32.to_be_bytes()); // height
        ihdr.extend_from_slice(&[8, 6, 0, 0, 0]); // depth, RGBA, filters
        ihdr.extend_from_slice(&crc32(&ihdr[4..]).to_be_bytes());
        f.write_all(&ihdr).expect("ihdr");
        drop(f);

        let scan = scan_folder(&dir);
        assert_eq!(scan.images.len(), 0);
        assert_eq!(scan.skipped, 1, "9000x9000 exceeds the 8192 limit");
        std::fs::remove_dir_all(&dir).expect("cleanup");
    }

    /// Standard PNG chunk CRC-32 (IEEE 802.3 polynomial).
    fn crc32(data: &[u8]) -> u32 {
        let mut crc: u32 = 0xFFFF_FFFF;
        for &byte in data {
            crc ^= byte as u32;
            for _ in 0..8 {
                let mask = (crc & 1).wrapping_neg();
                crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
            }
        }
        !crc
    }

    #[test]
    fn cache_add_refresh_poll_remove_lifecycle() {
        let dir = unique_temp_dir("cache");
        write_png(&dir.join("brick.png"), 4, [200, 60, 60]);
        let mut cache = LibraryCache::new(Vec::new());
        assert_eq!(cache.folders(), &[] as &[PathBuf]);
        cache.add_folder(dir.clone());
        cache.add_folder(dir.clone()); // duplicate add is a no-op
        assert_eq!(cache.folders().len(), 1);
        assert!(cache.scan(&dir).is_none());
        cache.refresh(&dir);
        cache.refresh(&dir); // second refresh while running is a no-op
        assert!(wait_for_scan(&mut cache, &dir), "scan did not complete");
        let scan = cache.scan(&dir).expect("scan present");
        assert_eq!(scan.images.len(), 1);
        assert_eq!(scan.images[0].name, "brick.png");
        cache.remove_folder(&dir);
        assert!(cache.scan(&dir).is_none());
        assert!(cache.folders().is_empty());
        std::fs::remove_dir_all(&dir).expect("cleanup");
    }

    #[test]
    fn downsample_64_averages_quadrants() {
        // 128x128: top-left half red, bottom-right half blue (2px blocks map
        // 1:1 onto output pixels, so quadrant averages are exact).
        let (w, h) = (128u32, 128u32);
        let mut rgba = vec![0u8; (w * h * 4) as usize];
        for y in 0..h {
            for x in 0..w {
                let px = ((y * w + x) * 4) as usize;
                let c = if x < 64 && y < 64 {
                    [255u8, 0, 0, 255]
                } else {
                    [0, 0, 255u8, 255]
                };
                rgba[px..px + 4].copy_from_slice(&c);
            }
        }
        let thumb = downsample_64(&rgba, w, h);
        assert_eq!(thumb.size, [64, 64]);
        let at = |x: usize, y: usize| {
            let c = thumb.pixels[y * 64 + x];
            (c.r(), c.g(), c.b(), c.a())
        };
        assert_eq!(at(10, 10), (255, 0, 0, 255), "top-left quadrant is red");
        assert_eq!(at(31, 31), (255, 0, 0, 255));
        assert_eq!(
            at(32, 32),
            (0, 0, 255, 255),
            "bottom-right quadrant is blue"
        );
        assert_eq!(at(63, 63), (0, 0, 255, 255));
    }

    #[test]
    fn scan_nonexistent_folder_is_empty_without_panic() {
        let missing = std::env::temp_dir().join("cindral-matlib-does-not-exist-42");
        let scan = scan_folder(&missing);
        assert_eq!(scan.images.len(), 0);
        assert_eq!(scan.skipped, 0);
        assert_eq!(scan.folder, missing);
    }
}
