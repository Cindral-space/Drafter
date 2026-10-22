//! Replacing a completed operation is atomic and preserves one history entry.
use cindral_core::{Command, DocumentHistory, DVec3};
#[test]
fn replacement_failure_and_one_entry_undo_redo() {
    let mut h = DocumentHistory::new();
    h.execute(Command::AddEdge{start:DVec3::ZERO,end:DVec3::X}).unwrap();
    let depth = h.undo_depth();
    for n in [2.,3.] { assert!(h.replace_last(Command::AddEdge{start:DVec3::ZERO,end:DVec3::X*n}).unwrap()); assert_eq!(h.undo_depth(),depth); }
    let model = h.model().clone();
    assert!(h.replace_last(Command::AddEdge{start:DVec3::ZERO,end:DVec3::splat(f64::NAN)}).is_err());
    assert_eq!(h.model(),&model); assert_eq!(h.undo_depth(),depth);
    h.undo().unwrap(); assert!(h.model().edges().is_empty());
    h.redo().unwrap(); assert_eq!(h.model(),&model);
}
