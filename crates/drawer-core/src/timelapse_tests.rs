#[test]
fn durable_recording_failures_cancellation_limits_restart_cutoff_and_frame_output() {
    let t = crate::test_support::TempDir::new();
    super::verify(&t.0, &mut |a, b| {
        if b.exists() {
            return Err("preserve".into());
        }
        std::fs::rename(a, b).map_err(|e| e.to_string())
    })
    .unwrap();
}
#[test]
fn pause_clock_excludes_gaps_and_preserves_restart_base() {
    let mut c = super::Clock::new(500, 10.);
    assert_eq!(c.at(11.), 1500);
    c.pause(11.);
    assert_eq!(c.at(999.), 1500);
    c.resume(1000.);
    assert_eq!(c.at(1002.), 3500);
    c.pause(1002.);
    assert_eq!(super::Clock::new(c.at(9999.), 0.).at(1.), 4500);
    assert_eq!(c.at(f64::NAN), 3500);
}
