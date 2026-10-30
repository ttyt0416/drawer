use super::*;
#[test]
fn independent_and_shared_arrangements_native_audio_undo_and_global_sources() {
    let t = crate::test_support::TempDir::new();
    super::verify(&t.0).unwrap();
}
#[test]
fn timecode_minute_tenth_minute_and_hour_boundaries_do_not_drop_data() {
    let f = Fps {
        numerator: 30000,
        denominator: 1001,
    };
    assert_eq!(timecode(1799, f, true).unwrap(), "00:00:59;29");
    assert_eq!(timecode(1800, f, true).unwrap(), "00:01:00;02");
    assert_eq!(timecode(17981, f, true).unwrap(), "00:09:59;29");
    assert_eq!(timecode(17982, f, true).unwrap(), "00:10:00;00");
    assert_eq!(timecode(107892, f, true).unwrap(), "01:00:00;00");
    let f = Fps {
        numerator: 60000,
        denominator: 1001,
    };
    assert_eq!(timecode(3600, f, true).unwrap(), "00:01:00;04");
    assert_eq!(timecode(35964, f, true).unwrap(), "00:10:00;00");
    assert!(timecode(0, Fps::default(), true).is_err());
}
