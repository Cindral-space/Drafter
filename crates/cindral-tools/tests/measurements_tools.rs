//! Scripted shared-parser/retained-operation/history contracts for every adapter.
use cindral_core::{DocumentHistory, DVec3};
use cindral_tools::{ToolKind, ToolSession};
#[test]
fn drawing_repeated_measurement_and_invalid_input() {
    for kind in [ToolKind::Line,ToolKind::Rectangle,ToolKind::Circle,ToolKind::Arc,ToolKind::Polygon] {
        let mut h = DocumentHistory::new(); let mut t = ToolSession::new(kind);
        t.click(DVec3::ZERO).unwrap();
        if kind == ToolKind::Arc { t.click(DVec3::X).unwrap(); }
        t.pointer_move(DVec3::new(2.,3.,0.)).unwrap();
        let entries = if kind == ToolKind::Rectangle { ["2m,3m","4m","5m,6m"] } else if kind == ToolKind::Arc { ["90deg","120deg","45deg"] } else { ["2m","3m","4m"] };
        for (i,text) in entries.into_iter().enumerate() {
            let command = t.measure(text).unwrap().unwrap();
            if i == 0 { h.execute(command).unwrap(); } else { h.replace_last(command).unwrap(); }
            assert_eq!(h.undo_depth(),1,"{kind:?}");
            let saved = h.model().clone(); let readout = t.measurement_readout();
            assert!(t.measure("garbage").is_err());
            assert_eq!(h.model(),&saved); assert_eq!(t.measurement_readout(),readout);
        }
        let final_model = h.model().clone(); h.undo().unwrap(); assert!(h.model().edges().is_empty());
        h.redo().unwrap(); assert_eq!(h.model(),&final_model);
        t.cancel(); assert!(!t.editing_completed()); assert!(t.measure("2m").is_err() || matches!(kind, ToolKind::Circle|ToolKind::Polygon));
    }
}
#[test]
fn coordinates_signed_line_sides_and_arc_radius() {
    let mut t = ToolSession::new(ToolKind::Line); t.click(DVec3::X).unwrap();
    let c = t.measure("[2m,3m,4m]").unwrap().unwrap();
    assert!(matches!(c,cindral_core::Command::AddEdge{end,..} if end==DVec3::new(2.,3.,4.)));
    t.cancel(); t.click(DVec3::ZERO).unwrap(); t.pointer_move(DVec3::X).unwrap();
    assert!(matches!(t.measure("-2m").unwrap().unwrap(),cindral_core::Command::AddEdge{end,..} if end==DVec3::X*-2.));
    for kind in [ToolKind::Circle,ToolKind::Polygon,ToolKind::Arc] {
        t.set_kind(kind); assert!(t.measure("12s").unwrap().is_none()); assert_eq!(t.segments(),12);
        t.click(DVec3::ZERO).unwrap(); t.pointer_move(DVec3::X).unwrap();
        if kind == ToolKind::Arc { assert!(t.measure("2m").unwrap().is_none()); t.pointer_move(DVec3::Y).unwrap(); }
        let c = t.measure(if kind==ToolKind::Arc {"90deg"} else {"2m"}).unwrap().unwrap();
        let mut h=DocumentHistory::new(); h.execute(c).unwrap();
        h.replace_last(t.measure("24s").unwrap().unwrap()).unwrap();
        assert_eq!(h.model().edges().len(),24); assert_eq!(h.undo_depth(),1);
    }
}
