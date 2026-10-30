use super::*;
#[test]
fn monotonic_idle_focus_switch_pause_and_suspend_never_overlap() {
    let mut c = Clock::default();
    assert_eq!(c.step(0, Some(1), true, false), None);
    assert_eq!(c.step(1000, Some(1), true, false), None);
    assert_eq!(c.step(2000, Some(1), true, true), None);
    let mut total = 0;
    for t in (3000..=304000).step_by(1000) {
        total += c.step(t, Some(1), true, false).map_or(0, |(_, n)| n);
    }
    assert_eq!(total, IDLE_MS);
    assert!(!c.running(304000));
    assert_eq!(c.step(305000, Some(2), true, true), None);
    assert_eq!(c.step(306000, Some(1), true, true), Some((2, 1000)));
    assert_eq!(c.step(307000, Some(1), false, false), Some((1, 1000)));
    assert_eq!(c.step(308000, Some(1), false, true), None);
    assert_eq!(c.step(309000, Some(1), true, false), None);
    assert_eq!(c.step(310000, Some(1), true, true), None);
    assert_eq!(c.step(3_000_000, Some(1), true, false), None); // sleep/stalled loop
    assert_eq!(c.step(3_001_000, Some(1), true, false), None);
    assert_eq!(c.step(3_002_000, Some(1), true, true), None);
    assert_eq!(c.step(10, Some(1), true, false), None); // clock reversal rejected
    assert_eq!(c.step(20, Some(1), true, false), None);
    let mut s = Snapshot::new(identity());
    s.add(500).unwrap();
    s.pause(true).unwrap();
    s.add(900).unwrap();
    assert_eq!(s.milliseconds, 500);
    s.pause(false).unwrap();
    s.reset().unwrap();
    s.add(20).unwrap();
    s.undo_reset().unwrap();
    assert_eq!(s.milliseconds, 520);
}
#[test]
fn journal_restart_reset_native_snapshot_conflict_and_failure_preserve_data() {
    let t = crate::test_support::TempDir::new();
    super::verify(&t.0).unwrap();
}
#[test]
fn malformed_limits_and_legacy_identity_are_checked() {
    let mut s = Snapshot::new(identity());
    assert!(s.valid());
    s.milliseconds = MAX_MS;
    s.add(1).unwrap_err();
    assert_eq!(s.milliseconds, MAX_MS);
    s.sequence = u64::MAX;
    s.reset().unwrap_err();
    assert_eq!(s.milliseconds, MAX_MS);
    s.id = "../outside".into();
    assert!(!s.valid());
    assert_eq!(
        legacy_identity(Path::new("C:/Test/A.drawer")),
        legacy_identity(Path::new("c:/test/a.drawer"))
    );
    assert_ne!(identity(), identity());
}
