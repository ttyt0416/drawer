use super::*;
use crate::document::DEFAULT_DOCUMENT_BUDGET as B;
#[test]
fn geometry_audio_sync_batch_names_native_undo_and_export_guide_isolation() {
    let t = crate::test_support::TempDir::new();
    crate::animation_finish_verify::verify(&t.0).unwrap();
}
#[test]
fn rule_syntax_filename_caps_counters_cancel_stale_and_clear_frames() {
    let r = Rule::default();
    assert_eq!(r.name(0, 4, "old").unwrap(), "Cel_001");
    assert_eq!(
        Rule {
            pattern: "{{{old}}}_{id}_{n}".into(),
            width: 0,
            ..r.clone()
        }
        .name(0, 4, "old")
        .unwrap(),
        "{old}_4_1"
    );
    for p in [
        "", "{bad}", "{", "}", "CON.txt", "LPT9", "../name", "name.", "name ", "A:B",
    ] {
        assert!(
            Rule {
                pattern: p.into(),
                ..r.clone()
            }
            .name(0, 1, "x")
            .is_err(),
            "{p}"
        );
    }
    assert!(
        Rule {
            pattern: "A".repeat(256),
            ..r.clone()
        }
        .name(0, 1, "x")
        .is_err()
    );
    assert!(
        Rule {
            start: u32::MAX,
            ..r.clone()
        }
        .name(1, 1, "x")
        .is_err()
    );
    let (mut d, _) = crate::animation_finish_verify::fixture().unwrap();
    let e = || Edit::Output {
        settings: Settings {
            safe: Some([1, 100]),
            area: Some(Area::Canvas),
        },
        first: 0,
        end: 100,
        bounds: Some(Rect {
            min: [0., 0.],
            max: [4., 3.],
        }),
    };
    let before = d.snapshot();
    assert!(prepare(&d, e(), B, &AtomicBool::new(true)).is_err());
    assert!(prepare(&d, e(), 1, &AtomicBool::new(false)).is_err());
    assert_eq!(d.snapshot(), before);
    let s = prepare(&d, e(), B, &AtomicBool::new(false)).unwrap();
    d.edit_pixel(0, 3, 2, [8, 9, 0, 255]);
    let edited = d.snapshot();
    assert!(s.commit(&mut d).is_err());
    assert_eq!(d.snapshot(), edited);
    prepare(&d, e(), B, &AtomicBool::new(false))
        .unwrap()
        .commit(&mut d)
        .unwrap();
    let saved = d.snapshot();
    prepare(&d, Edit::ClearFrames, B, &AtomicBool::new(false))
        .unwrap()
        .commit(&mut d)
        .unwrap();
    assert!(d.scene.animation_output.keys.is_empty());
    d.undo();
    assert_eq!(d.snapshot(), saved);
}
