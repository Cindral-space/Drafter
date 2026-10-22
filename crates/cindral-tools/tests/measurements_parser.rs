//! Shared syntax, locale, conversion and totality regression tests.
use cindral_tools::measurements::*;
fn length(s: &str, settings: MeasurementSettings) -> f64 {
    let Measurement::Values(v) = parse(s, settings).unwrap() else { panic!("not values") };
    v[0].length(settings).unwrap()
}
#[test]
fn lengths_units_and_locales() {
    for (s, meters) in [("2m",2.),("150cm",1.5),("25mm",0.025),("2.5",2.5),("3'",0.9144),("6\"",0.1524),("3'6\"",1.0668),("3'-6\"",1.0668),("1/2\"",0.0127),("3 1/2\"",0.0889),("-3'6\"",-1.0668),("-3 1/2\"",-0.0889),("-2m",-2.),("12in",0.3048),("1ft",0.3048)] {
        assert!((length(s, Default::default())-meters).abs()<1e-12, "{s}");
    }
    let settings = MeasurementSettings { unit: LengthUnit::Millimeter, decimal_comma: true, ..Default::default() };
    assert_eq!(length("2,5", settings),0.0025);
    assert_eq!(parse("2m;3m",settings).unwrap(),Measurement::Values(vec![Quantity::Length(2.),Quantity::Length(3.)]));
    assert_eq!(parse("2m,3m",Default::default()).unwrap(),Measurement::Values(vec![Quantity::Length(2.),Quantity::Length(3.)]));
    assert!(parse("2.5",settings).is_err());
}
#[test]
fn structured_forms_and_rejections() {
    let settings = MeasurementSettings::default();
    assert_eq!(parse("24s",settings).unwrap(),Measurement::Sides(24));
    for s in ["3x","*3"] { assert_eq!(parse(s,settings).unwrap(),Measurement::Copies { count:3,divide:false }); }
    assert_eq!(parse("/3",settings).unwrap(),Measurement::Copies { count:3,divide:true });
    assert!(matches!(parse("[1m,2m,3m]",settings).unwrap(),Measurement::Coordinates{relative:false,..}));
    assert!(matches!(parse("<1,2,3>",settings).unwrap(),Measurement::Coordinates{relative:true,..}));
    assert_eq!(parse("8:12",settings).unwrap(),Measurement::Slope(8_f64.atan2(12.).to_degrees()));
    assert_eq!(parse("90deg",settings).unwrap(),Measurement::Values(vec![Quantity::Degrees(90.)]));
    assert_eq!(parse("150%",settings).unwrap(),Measurement::Values(vec![Quantity::Number(1.5)]));
    for s in ["", " ","NaN","inf","1e999","2mfoo","1m2cm","3'bad", "1/0\"","1//2\"","3'-6", "[1,2]", "<1,2,3]", "1,2,3,4", "2,,3", "0x", "257s", "8:0", "1:2:3", "🦀"] { assert!(parse(s,settings).is_err(),"{s}"); }
}
#[test]
fn rounding_precision_and_marks() {
    let s = MeasurementSettings::default();
    assert_eq!(format_length(2.,s),"2.000m");
    assert_eq!(format_length(1.23456,s),"~1.235m");
    assert_eq!(format_length(1.23456,MeasurementSettings{decimal_comma:true,..s}),"~1,235m");
    assert_eq!(format_length(0.0254,MeasurementSettings{unit:LengthUnit::Inch,..s}),"1.000in");
}
proptest::proptest! {
    #[test]
    fn arbitrary_strings_never_panic(input in proptest::prelude::any::<String>(), comma in proptest::prelude::any::<bool>()) {
        let _ = parse(&input,MeasurementSettings{decimal_comma:comma,..Default::default()});
    }
}

#[test]
fn box_lifecycle_and_idle_cache() {
    let mut b = MeasurementsBox::default();
    let readout = Readout { label: "Length",values:[2.,0.,0.],count:1,kind:ValueKind::Length };
    b.update(readout,Default::default());
    assert_eq!(b.phase,BoxPhase::Live);
    let pointer = b.display.as_ptr();
    b.update(readout,Default::default());
    assert_eq!(pointer,b.display.as_ptr());
    b.type_text("3'6\""); b.invalid = true;
    b.update(Readout{values:[4.,0.,0.],..readout},Default::default());
    assert_eq!(b.text,"3'6\"");
    assert!(b.escape()); assert!(!b.invalid); assert_eq!(b.display(),"4.000m");
    assert!(!b.escape()); assert_eq!(b.phase,BoxPhase::Empty);
    b.type_text("2"); b.backspace(); assert!(b.text.is_empty());
    b.applied(); assert_eq!(b.phase,BoxPhase::Applied);
    b.completed(); assert_eq!(b.phase,BoxPhase::Completed);
    b.reset(); assert_eq!(b.phase,BoxPhase::Empty);
}
