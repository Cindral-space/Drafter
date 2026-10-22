//! Opt-in CPU phase timings. Nested phases are inclusive; GPU timers measure host work.
use std::{cell::RefCell, sync::OnceLock, time::Instant};
/// Stable phase names shared by the editor and regression replay.
pub const NAMES: [&str; 10] = ["pick", "inference/snap", "tool update", "kernel edit", "mesh rebuild", "GPU upload (CPU)", "shadow pass (CPU)", "UI/outliner", "bounds/sections", "render + GPU completion"];
/// Most recently completed UI frame, in milliseconds.
#[derive(Clone, Copy, Debug, Default)]
pub struct Sample {
/// Inclusive phase durations in milliseconds.
pub phases: [f64; 10],
/// UI update duration in milliseconds.
pub total: f64 }
thread_local! { static ACTIVE: RefCell<Sample> = RefCell::new(Sample::default()); static LAST: RefCell<Sample> = RefCell::new(Sample::default()); }
/// Cached once; disabled frames perform no clock reads or allocations.
pub fn enabled() -> bool { static ENABLED: OnceLock<bool> = OnceLock::new(); *ENABLED.get_or_init(|| std::env::var("CINDRAL_PROFILE").as_deref() == Ok("1")) }
/// Inclusive host duration for a named phase.
pub struct Scope { phase: usize, start: Option<Instant> }
impl Scope {
/// Begin a named phase when profiling is enabled.
pub fn new(phase: usize) -> Self { Self { phase, start: enabled().then(Instant::now) } } }
impl Drop for Scope { fn drop(&mut self) { if let Some(start) = self.start { let duration = start.elapsed().as_secs_f64()*1000.; ACTIVE.with_borrow_mut(|s| s.phases[self.phase] += duration); if duration > 33. && self.phase >= 5 && self.phase <= 6 { eprintln!("CINDRAL_PROFILE {}={duration:.3}ms", NAMES[self.phase]); } } } }
/// Delimits one editor UI update (render callbacks run later).
pub struct Frame(Option<Instant>);
impl Default for Frame { fn default() -> Self { Self::new() } }
impl Frame {
/// Begin an editor UI frame.
pub fn new() -> Self { let start = enabled().then(Instant::now); if start.is_some() { ACTIVE.with_borrow_mut(|s| *s = Sample::default()); } Self(start) } }
impl Drop for Frame { fn drop(&mut self) { if let Some(start) = self.0 { ACTIVE.with_borrow_mut(|s| { s.total=start.elapsed().as_secs_f64()*1000.; LAST.with_borrow_mut(|last| *last=*s); if s.total>33. { eprintln!("CINDRAL_PROFILE UI frame={:.3}ms phases={:?} names={:?} (inclusive; render callbacks reported separately)", s.total, s.phases, NAMES); } }); } } }
/// Snapshot used by the real-file regression runner.
pub fn last() -> Sample { ACTIVE.with_borrow(|s| *s) }
