use super::*;
#[test]
fn precompute_camera_quality_range_cancel_memory_source_change_output_and_preferences() {
    let t = crate::test_support::TempDir::new();
    super::verify(&t.0).unwrap();
}
#[test]
fn late_render_realtime_skips_screen_frames_every_frame_never_skips() {
    let range = Range {
        first: 2,
        end: 8,
        repeat: false,
    };
    let fps = Fps {
        numerator: 24,
        denominator: 1,
    };
    let mut rt = Cursor::new(range, 2, fps, Mode::RealTime, 0.).unwrap();
    assert_eq!(rt.step(0.2, 1).0, 6);
    let mut every = Cursor::new(range, 2, fps, Mode::EveryFrame, 0.).unwrap();
    let mut seen = vec![];
    for pass in 0..7 {
        let (f, done) = every.step(pass as f64 * 0.2, pass);
        seen.push(f);
        every.presented(pass);
        if done {
            break;
        }
    }
    assert_eq!(seen, vec![2, 3, 4, 5, 6, 7, 7]);
    assert_eq!(every.step(f64::NAN, 100), (7, true));
    let mut looped = Cursor::new(
        Range {
            first: 4,
            end: 5,
            repeat: true,
        },
        4,
        fps,
        Mode::EveryFrame,
        0.,
    )
    .unwrap();
    looped.presented(0);
    assert_eq!(looped.step(10., 1), (4, false));
    assert_eq!(looped.step(11., 1), (4, false));
}
