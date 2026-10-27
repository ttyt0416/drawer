use crate::{
    animation::{Audio, Exposure, Fps},
    document::{DEFAULT_DOCUMENT_BUDGET as B, Document},
    layer_properties::{self as props, Target},
    scene::Rect,
    timeline_batch::{self as batch, Pattern, Selector},
    timeline_editor::{self as cels, Edit},
    timeline_time_edit::{self as edit, Choices, Request, Scope},
    timeline_timing::{self as time, Playback},
};
use std::{path::Path, sync::Arc};
fn check(ok: bool, message: &str) -> Result<(), String> {
    if ok { Ok(()) } else { Err(message.into()) }
}
pub fn fixture() -> Document {
    let mut d = crate::animation_export_verify::fixture();
    let mut s = d.snapshot();
    let a = s.animation.as_mut().unwrap();
    a.fps = Fps::default();
    a.frames = 24;
    a.tracks[0].end = 24;
    a.tracks[0].exposures = vec![
        Exposure { frame: 0, cell: 0 },
        Exposure { frame: 1, cell: 1 },
        Exposure { frame: 2, cell: 0 },
        Exposure { frame: 6, cell: 1 },
    ];
    a.audio.push(Audio {
        name: "Original PCM".into(),
        start: 2,
        rate: 24000,
        channels: 1,
        sample_count: 12000,
        samples: Arc::new((0..12000).map(|i| ((i * 13) % 30000) as i16).collect()),
    });
    d.commit_state(s).unwrap();
    let track = d.scene.catalog.tracks.id(0);
    let p = cels::prepare(
        &d,
        Edit::CopyTrack {
            track,
            independent: false,
        },
        B,
    )
    .unwrap();
    d.commit_state(p.state).unwrap();
    let mut s = d.snapshot();
    s.scene.animation_output.keys = vec![
        crate::animation_output::Frame {
            frame: 1,
            bounds: Rect {
                min: [0., 0.],
                max: [1., 1.],
            },
            creation: Default::default(),
        },
        crate::animation_output::Frame {
            frame: 2,
            bounds: Rect {
                min: [0., 0.],
                max: [2., 1.],
            },
            creation: Default::default(),
        },
    ];
    d.commit_state(s).unwrap();
    let p = cels::prepare(
        &d,
        Edit::Label {
            name: "Linked label".into(),
            frame: 3,
        },
        B,
    )
    .unwrap();
    d.commit_state(p.state).unwrap();
    d.current_frame = 3;
    d
}
pub fn accept(d: &mut Document, request: Request) -> Result<(), String> {
    let p = edit::prepare(d, request.clone(), &Choices::new(), B)?;
    let choices = p.issues.iter().map(|i| (i.id.clone(), 0)).collect();
    edit::prepare(d, request, &choices, B)?.commit(d)?;
    Ok(())
}
pub fn verify(root: &Path) -> Result<(), String> {
    let dir = root.join("timeline-timing-verify");
    std::fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let result = (|| {
        let mut d = fixture();
        let original = d.snapshot();
        let count = d.history_counts();
        let request = Request::Fps {
            fps: Fps {
                numerator: 12,
                denominator: 1,
            },
            keep_time: true,
            labels_linked: true,
        };
        let p = edit::prepare(&d, request.clone(), &Choices::new(), B)?;
        check(
            !p.ready()
                && p.issues.iter().any(|i| i.id.starts_with("key-"))
                && p.issues
                    .iter()
                    .any(|i| i.id.starts_with("minimum-exposure")),
            "FPS collision/minimum exposure table absent",
        )?;
        check(
            p.commit(&mut d).is_err() && d.snapshot() == original,
            "Unresolved conversion committed",
        )?;
        let choices = p
            .issues
            .iter()
            .map(|i| (i.id.clone(), usize::from(i.id.starts_with("key-"))))
            .collect();
        let p = edit::prepare(&d, request, &choices, B)?;
        p.commit(&mut d)?;
        check(
            d.animation.as_ref().unwrap().frames == 12
                && d.scene.animation_output.keys.len() == 1
                && d.scene.animation_output.keys[0].bounds.max == [2., 1.]
                && d.scene.cels.labels[0].frame == 2,
            "FPS boundary/key/label conversion",
        )?;
        check(
            d.scene.timing.originals[0].samples
                == original.animation.as_ref().unwrap().audio[0].samples
                && d.animation.as_ref().unwrap().audio[0].rate == 24000,
            "FPS changed original waveform/rate",
        )?;
        check(
            d.history_counts().0 == count.0 + 1 && d.undo() && d.snapshot() == original && d.redo(),
            "FPS conversion not one complete Undo/Redo",
        )?;
        d.undo();
        accept(
            &mut d,
            Request::Fps {
                fps: Fps {
                    numerator: 30000,
                    denominator: 1001,
                },
                keep_time: true,
                labels_linked: false,
            },
        )?;
        check(
            d.animation.as_ref().unwrap().frames == 30 && d.scene.cels.labels[0].frame == 3,
            "29.97fps time preservation/unlinked label",
        )?;
        let samples = d.scene.timing.originals[0].samples.clone();
        let path = dir.join("timing.drawer");
        let saved = d.snapshot();
        crate::files::save(&mut d, &path).map_err(|e| e.to_string())?;
        let mut corrupt = std::fs::read(&path).unwrap();
        let last = corrupt.len() - 1;
        corrupt[last] ^= 1;
        std::fs::write(dir.join("corrupt.drawer"), corrupt).unwrap();
        check(
            crate::files::open(&dir.join("corrupt.drawer"), B).is_err(),
            "Corrupt original PCM was accepted",
        )?;
        let mut reopened = crate::files::open(&path, B).map_err(|e| e.to_string())?;
        check(
            reopened.snapshot() == saved && reopened.scene.timing.originals[0].samples == samples,
            "Native97 PCM backing/clip/range reload",
        )?;
        let mut bytes = std::fs::read(&path).unwrap();
        bytes[8..12].copy_from_slice(&96u32.to_le_bytes());
        std::fs::write(dir.join("old.drawer"), bytes).unwrap();
        check(
            crate::files::open(&dir.join("old.drawer"), B).is_err(),
            "Old header accepted timing backing",
        )?;
        accept(
            &mut reopened,
            Request::Fps {
                fps: Fps::default(),
                keep_time: true,
                labels_linked: true,
            },
        )?;
        check(
            reopened.animation.as_ref().unwrap().frames == 24,
            "29.97→24 conversion",
        )?;
        let before = d.snapshot();
        check(
            edit::prepare(
                &d,
                Request::Fps {
                    fps: Fps {
                        numerator: 0,
                        denominator: 1,
                    },
                    keep_time: false,
                    labels_linked: true,
                },
                &Choices::new(),
                B,
            )
            .is_err()
                && d.snapshot() == before,
            "Zero FPS accepted",
        )?;
        let mut lock = props::properties(&d, Target::Track(1));
        lock.locked = true;
        d.commit_state(props::prepare(&d, &[Target::Track(1)], lock)?)?;
        check(
            edit::prepare(
                &d,
                Request::Fps {
                    fps: Fps::default(),
                    keep_time: true,
                    labels_linked: true,
                },
                &Choices::new(),
                B,
            )
            .is_err(),
            "Locked track FPS edit accepted",
        )?;
        d.undo();
        let audio = d.scene.timing.audio[0].id;
        accept(
            &mut d,
            Request::AudioLock {
                id: audio,
                locked: true,
            },
        )?;
        check(
            edit::prepare(
                &d,
                Request::Fps {
                    fps: Fps::default(),
                    keep_time: false,
                    labels_linked: false,
                },
                &Choices::new(),
                B,
            )
            .is_err(),
            "Locked audio FPS edit accepted",
        )?;
        d.undo();
        let mut s = fixture();
        let base = s.snapshot();
        let track = s.scene.catalog.tracks.id(0);
        let other = s.animation.as_ref().unwrap().tracks[1].clone();
        accept(
            &mut s,
            Request::Splice {
                at: 4,
                count: 3,
                remove: false,
                scope: Scope {
                    all: true,
                    ..Default::default()
                },
                split: true,
                labels_linked: true,
            },
        )?;
        check(
            s.animation.as_ref().unwrap().frames == 27
                && s.animation.as_ref().unwrap().audio.len() == 2
                && s.scene.timing.clips.len() == 4,
            "Insertion did not split crossing cel/audio clips",
        )?;
        check(
            (4..7).all(|f| {
                s.current_frame = f;
                crate::render::Renderer::new(&s).pixel([0.5, 0.5])[3] == 0
            }),
            "Split insertion did not create empty frames",
        )?;
        check(
            s.undo() && s.snapshot() == base,
            "Insertion Undo did not restore keys/audio/clips/references",
        )?;
        accept(
            &mut s,
            Request::Splice {
                at: 4,
                count: 3,
                remove: false,
                scope: Scope {
                    all: true,
                    ..Default::default()
                },
                split: false,
                labels_linked: false,
            },
        )?;
        check(
            s.scene.timing.clips.len() == 2
                && s.animation.as_ref().unwrap().audio.len() == 1
                && s.scene.timing.audio[0]
                    .spans
                    .iter()
                    .any(|p| p.from.is_none()),
            "Unsplit audio insertion lost silent span/original clip",
        )?;
        s.undo();
        accept(
            &mut s,
            Request::Splice {
                at: 4,
                count: 3,
                remove: true,
                scope: Scope {
                    all: false,
                    tracks: vec![track],
                    ..Default::default()
                },
                split: true,
                labels_linked: false,
            },
        )?;
        check(
            s.animation.as_ref().unwrap().tracks[1] == other
                && s.animation.as_ref().unwrap().frames == 24
                && s.animation.as_ref().unwrap().audio == base.animation.as_ref().unwrap().audio,
            "Selected-track deletion changed another owner",
        )?;
        s.undo();
        check(
            edit::prepare(
                &s,
                Request::Splice {
                    at: 0,
                    count: 1,
                    remove: false,
                    scope: Scope::default(),
                    split: false,
                    labels_linked: true,
                },
                &Choices::new(),
                B,
            )
            .is_err(),
            "No-selection insertion accepted",
        )?;
        accept(
            &mut s,
            Request::Splice {
                at: 0,
                count: 24,
                remove: true,
                scope: Scope {
                    all: true,
                    ..Default::default()
                },
                split: true,
                labels_linked: true,
            },
        )?;
        check(
            s.animation.as_ref().unwrap().frames == 1
                && s.animation
                    .as_ref()
                    .unwrap()
                    .tracks
                    .iter()
                    .all(|t| t.exposures.is_empty())
                && s.animation.as_ref().unwrap().audio.is_empty()
                && s.scene.cels.labels.is_empty()
                && s.scene.timing.originals.len() == 1,
            "Delete all did not retain an empty one-frame timeline/original PCM",
        )?;
        check(s.undo() && s.snapshot() == base, "Delete all Undo")?;
        accept(
            &mut s,
            Request::Playback(Playback {
                first: 3,
                end: 7,
                repeat: true,
            }),
        )?;
        check(
            time::playback(&s, 3, 4. / 24., false)?.0 == 3
                && time::playback(&s, 6, 1. / 24., false)?.0 == 3,
            "Playback range duplicated last frame or ignored loop",
        )?;
        accept(
            &mut s,
            Request::Playback(Playback {
                first: 6,
                end: 7,
                repeat: false,
            }),
        )?;
        check(
            time::playback(&s, 0, 1., true)? == (6, true),
            "One-frame playback range",
        )?;
        check(
            edit::prepare(
                &s,
                Request::Splice {
                    at: 0,
                    count: u32::MAX,
                    remove: false,
                    scope: Scope {
                        all: true,
                        ..Default::default()
                    },
                    split: false,
                    labels_linked: true,
                },
                &Choices::new(),
                B,
            )
            .is_err(),
            "Unbounded insertion accepted",
        )?;
        check(
            serde_json::from_str::<Fps>(r#"{"numerator":-24,"denominator":1}"#).is_err(),
            "Negative FPS parsed",
        )?;
        let mut positioned = fixture();
        let before = positioned.snapshot();
        accept(
            &mut positioned,
            Request::Fps {
                fps: Fps {
                    numerator: 30000,
                    denominator: 1001,
                },
                keep_time: false,
                labels_linked: true,
            },
        )?;
        check(
            positioned.animation.as_ref().unwrap().frames == 24
                && positioned.animation.as_ref().unwrap().tracks
                    == before.animation.as_ref().unwrap().tracks
                && positioned.scene.timing.originals[0].samples
                    == before.animation.as_ref().unwrap().audio[0].samples,
            "Frame-position preservation moved cells or resampled audio",
        )?;
        accept(
            &mut positioned,
            Request::Fps {
                fps: Fps::default(),
                keep_time: false,
                labels_linked: true,
            },
        )?;
        check(
            positioned.animation.as_ref().unwrap().audio[0].samples
                == before.animation.as_ref().unwrap().audio[0].samples,
            "Extending a trim did not recover the preserved original tail",
        )?;
        let mut hidden = props::properties(&positioned, Target::Track(1));
        hidden.hidden = true;
        positioned.commit_state(props::prepare(&positioned, &[Target::Track(1)], hidden)?)?;
        check(
            edit::prepare(
                &positioned,
                Request::Fps {
                    fps: Fps::default(),
                    keep_time: true,
                    labels_linked: true,
                },
                &Choices::new(),
                B,
            )
            .is_ok(),
            "Hidden unlocked track could not be retimed",
        )?;
        let mut outside = fixture();
        accept(
            &mut outside,
            Request::Splice {
                at: 0,
                count: 23,
                remove: true,
                scope: Scope {
                    all: true,
                    ..Default::default()
                },
                split: false,
                labels_linked: false,
            },
        )?;
        check(
            outside.scene.cels.labels[0].frame == 3
                && outside.animation.as_ref().unwrap().frames == 1
                && crate::timeline_editor::navigate(
                    &outside,
                    None,
                    crate::timeline_editor::Jump::NextLabel,
                )
                .is_err(),
            "Unlinked out-of-range label was lost or navigable",
        )?;
        let mut audio_delete = fixture();
        accept(
            &mut audio_delete,
            Request::Splice {
                at: 4,
                count: 3,
                remove: true,
                scope: Scope {
                    all: true,
                    ..Default::default()
                },
                split: true,
                labels_linked: true,
            },
        )?;
        check(
            audio_delete.animation.as_ref().unwrap().audio.len() == 2
                && audio_delete.scene.timing.clips.len() == 4,
            "Crossing deletion failed to retain split clips",
        )?;
        let empty_path = dir.join("empty.drawer");
        accept(
            &mut audio_delete,
            Request::Splice {
                at: 0,
                count: 21,
                remove: true,
                scope: Scope {
                    all: true,
                    ..Default::default()
                },
                split: false,
                labels_linked: true,
            },
        )?;
        crate::files::save(&mut audio_delete, &empty_path).map_err(|e| e.to_string())?;
        let mut bytes = std::fs::read(&empty_path).unwrap();
        let last = bytes.len() - 1;
        bytes[last] ^= 1;
        std::fs::write(dir.join("unused-corrupt.drawer"), bytes).unwrap();
        check(
            crate::files::open(&dir.join("unused-corrupt.drawer"), B).is_err(),
            "Unused original PCM corruption was not detected",
        )?;
        let mut b = fixture();
        let track = b.scene.catalog.tracks.id(0);
        let mut state = b.snapshot();
        state.scene.cels.definitions[0].name = "A2".into();
        state.scene.cels.definitions[1].name = "A10".into();
        b.commit_state(state)?;
        let rule = Pattern {
            track,
            selector: Selector::Names {
                first: "A2".into(),
                last: "A10".into(),
            },
            start: 2,
            end: 13,
            hold: 2,
            gap: 1,
            step: 1,
            repeats: 0,
        };
        let old = b.snapshot();
        let p = edit::prepare(&b, Request::Batch(rule.clone()), &Choices::new(), B)?;
        check(
            !p.ready() && !p.rows.is_empty(),
            "Batch collision was not reviewed",
        )?;
        accept(&mut b, Request::Batch(rule.clone()))?;
        let t = &b.animation.as_ref().unwrap().tracks[0];
        check(
            t.end == 24
                && t.exposures.iter().any(|e| e.frame == 4)
                && b.animation.as_ref().unwrap().tracks[1]
                    == old.animation.as_ref().unwrap().tracks[1],
            "Batch gap/range/other track preservation",
        )?;
        check(b.undo() && b.snapshot() == old, "Batch Undo")?;
        let p = cels::prepare(
            &b,
            Edit::NewCel {
                track,
                name: "A10".into(),
            },
            B,
        )?;
        b.commit_state(p.state)?;
        check(
            batch::sources(&b, &rule).is_err(),
            "Duplicate name endpoint silently selected",
        )?;
        let numeric = Pattern {
            selector: Selector::Ids {
                first: 1,
                last: 100,
            },
            step: 2,
            ..rule.clone()
        };
        check(
            batch::sources(&b, &numeric)? == vec![1, 3],
            "Numeric skip/missing ID policy",
        )?;
        let single = Pattern {
            start: 23,
            end: 24,
            hold: 1,
            gap: 0,
            repeats: 1,
            selector: Selector::Ids { first: 1, last: 1 },
            ..rule.clone()
        };
        let before = b.snapshot();
        accept(&mut b, Request::Batch(single))?;
        check(
            b.undo() && b.snapshot() == before,
            "One-frame finite batch Undo",
        )?;
        let mut presets = batch::Presets::default();
        presets.set("Two-on-one-gap".into(), rule)?;
        let json = serde_json::to_vec(&presets).unwrap();
        check(
            serde_json::from_slice::<batch::Presets>(&json).unwrap() == presets,
            "Local preset serialization",
        )?;
        Ok(())
    })();
    let cleanup = std::fs::remove_dir_all(&dir).map_err(|e| e.to_string());
    result.and(cleanup)
}
#[cfg(test)]
mod tests {
    #[test]
    fn rational_fps_collision_choices_pcm_native_batch_splice_and_undo() {
        let root = crate::test_support::TempDir::new();
        super::verify(&root.0).unwrap();
    }
}
