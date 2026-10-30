use crate::{
    animation_export::{self, Options},
    animation_finish::{self as core, Edit, Rule, Settings, View},
    animation_render::{self, Area},
    document::{Background, CanvasSpec, DEFAULT_DOCUMENT_BUDGET as B, Document},
    keyframes::{self, Owner},
    layer_search::Key,
};
use std::{
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU32},
    },
};
pub fn fixture() -> Result<(Document, u64), String> {
    let mut d = Document::create(
        CanvasSpec {
            width: 4,
            height: 3,
            dpi: 300.,
            background: Background::Transparent,
        },
        B,
    )
    .map_err(|e| e.to_string())?;
    d.edit_pixel(0, 0, 1, [0, 255, 0, 255]);
    d.commit_state(crate::timeline_editor::create(&d, 100, Default::default())?)?;
    let p = crate::timeline_editor::prepare(
        &d,
        crate::timeline_editor::Edit::NewTrack("Cels".into()),
        B,
    )?;
    let track = p.track.unwrap();
    let source = p.source.unwrap();
    d.commit_state(p.state)?;
    let mut child = d.scene.cels.get(source).unwrap().open()?;
    child.edit_pixel(0, 0, 0, [255, 0, 0, 255]);
    d.commit_state(crate::cels::replace(&d, source, &child, B)?)?;
    d.current_frame = 50;
    let p = crate::timeline_editor::prepare(
        &d,
        crate::timeline_editor::Edit::NewCel {
            track,
            name: "Second".into(),
        },
        B,
    )?;
    d.commit_state(p.state)?;
    d.current_frame = 0;
    let mut s = d.snapshot();
    for c in &mut s.animation.as_mut().unwrap().tracks[0].cells {
        c.offset = [-2, 0];
    }
    let mut samples = vec![0i16; 12000];
    samples[0] = 10000;
    s.animation
        .as_mut()
        .unwrap()
        .audio
        .push(crate::animation::Audio {
            name: "Impulse".into(),
            start: 2,
            rate: 12000,
            channels: 1,
            sample_count: 12000,
            samples: Arc::new(samples),
        });
    let cells = s.animation.as_ref().unwrap().tracks[0]
        .exposures
        .iter()
        .map(|e| s.scene.catalog.cell_id(track, e.cell))
        .collect::<Vec<_>>();
    s.scene.shots.entries.push(crate::shot_exchange::Bound {
        track,
        cells,
        shot: crate::shot_exchange::Shot {
            id: 1,
            name: "Original shot".into(),
            start: 0,
            end: 100,
            fps: Default::default(),
            dialogue: String::new(),
            cells: vec![
                crate::shot_exchange::Image {
                    name: "First".into(),
                    frame: 0,
                    path: "first.png".into(),
                    offset: [-2, 0],
                },
                crate::shot_exchange::Image {
                    name: "Second".into(),
                    frame: 50,
                    path: "second.png".into(),
                    offset: [-2, 0],
                },
            ],
        },
    });
    d.commit_state(s)?;
    d.commit_state(crate::timeline_timing::enable(&d, B)?)?;
    d.commit_state(keyframes::prepare(
        &d,
        Owner::Layer(Key::Track(track)),
        keyframes::Edit::Enable(true),
    )?)?;
    Ok((d, track))
}
pub fn verify(root: &Path) -> Result<(), String> {
    let dir = root.join("finish-core-verify");
    std::fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let result = (|| {
        let check = |b: bool, s: &str| if b { Ok(()) } else { Err(s.to_owned()) };
        let cancel = AtomicBool::new(false);
        let progress = AtomicU32::new(0);
        let (mut d, track) = fixture()?;
        let original = d.snapshot();
        let bounds = crate::scene::Rect {
            min: [-1., 0.],
            max: [3., 3.],
        };
        core::prepare(
            &d,
            Edit::Output {
                settings: Settings {
                    safe: Some([80, 80]),
                    area: Some(Area::Overflow),
                },
                first: 3,
                end: 8,
                bounds: Some(bounds),
            },
            B,
            &cancel,
        )?
        .commit(&mut d)?;
        let geometry = core::guides(
            &d,
            View {
                overflow: true,
                ..Default::default()
            },
            B,
        )?;
        check(
            geometry.outside
                && geometry.output.unwrap()[0] == [-1., 0.]
                && geometry.safe.unwrap()[0][0] > -0.61
                && geometry.safe.unwrap()[0][0] < -0.59
                && geometry.overflow.unwrap()[0] == [-2., 0.],
            "Safe/output/overflow geometry or outside range differs",
        )?;
        let changed = d.snapshot();
        d.undo();
        check(d.snapshot() == original, "Output/safe Undo failed")?;
        d.redo();
        check(d.snapshot() == changed, "Output/safe Redo failed")?;
        let options = Options {
            first: 3,
            last: 7,
            view: animation_render::View {
                area: Area::Overflow,
                ..Default::default()
            },
            ..Default::default()
        };
        let capture = animation_export::prepare(&d, options, B)?;
        let pixels = animation_render::frame(&capture, 3, &cancel)?;
        check(
            capture.layout.origin == [-2, 0]
                && capture.layout.width == 6
                && pixels[..4] == [255, 0, 0, 255],
            "Overflow export lost outside cel or transparent framing",
        )?;
        let before = d.snapshot();
        let counts = d.history_counts();
        let dirty = d.dirty;
        d.current_frame = 3;
        let g = core::guides(&d, View::default(), B)?;
        check(!g.outside, "Included frame marked outside")?;
        let _ = core::guides(
            &d,
            View {
                output: false,
                safe: false,
                overflow: false,
                range: false,
            },
            B,
        )?;
        let after = animation_export::prepare(&d, options, B)?;
        check(
            animation_render::frame(&after, 3, &cancel)? == pixels
                && d.snapshot() == before
                && d.history_counts() == counts
                && d.dirty == dirty,
            "Guide visibility changed final pixels/document/Undo",
        )?;
        let result =
            animation_export::publish(&after, &dir.join("frames"), false, &cancel, &progress);
        check(
            result.error.is_none() && result.frames == 5,
            "Configured output range omitted frames",
        )?;
        let id = d.scene.timing.audio[0].id;
        core::prepare(
            &d,
            Edit::Audio {
                id,
                start: 4,
                gain: 50,
                mute: false,
            },
            B,
            &cancel,
        )?
        .commit(&mut d)?;
        let mix = crate::audio_mix::prepare(
            &d,
            crate::audio_mix::Options {
                first: 3,
                last: 7,
                rate: 12000,
                channels: 1,
            },
        )?;
        let offset = (crate::audio_mix::boundary(4, d.animation.as_ref().unwrap().fps, 12000)
            - crate::audio_mix::boundary(3, d.animation.as_ref().unwrap().fps, 12000))
            as usize;
        let (pcm, _) = mix.block(0, offset + 2)?;
        check(
            pcm[..offset].iter().all(|n| *n == 0) && pcm[offset] == 5000 && pcm[offset + 1] == 0,
            "Audio frame start/volume does not match selected output range",
        )?;
        d.commit_state(crate::camera2d::enable(&d, true)?)?;
        let old_def = d.scene.cels.definitions.clone();
        let old_keys = d.scene.keyframes.clone();
        core::prepare(
            &d,
            Edit::Names {
                cels: true,
                shots: true,
                cel: Rule {
                    pattern: "Cel_{n}_{id}".into(),
                    ..Default::default()
                },
                shot: Rule {
                    pattern: "Shot_{n}".into(),
                    ..Default::default()
                },
            },
            B,
            &cancel,
        )?
        .commit(&mut d)?;
        check(
            d.scene.cels.definitions[0].name == "Cel_001_1"
                && d.scene.shots.entries[0].shot.name == "Shot_001"
                && d.scene.keyframes == old_keys,
            "Batch names or animated source keys differ",
        )?;
        for (old, new) in old_def.iter().zip(&d.scene.cels.definitions) {
            check(
                old.same_content(new),
                "Renaming modified native cel content",
            )?;
        }
        let id = d.scene.cels.definitions[0].object;
        let mut child = d.scene.cels.get(id).unwrap().open()?;
        child.edit_pixel(0, 2, 1, [1, 2, 3, 255]);
        let before = d.snapshot();
        let blocked = match crate::cels::replace(&d, id, &child, B) {
            Err(_) => true,
            Ok(s) => d.commit_state(s).is_err(),
        };
        check(
            blocked && d.snapshot() == before,
            "Name-only animation guard exception allowed pixel modification",
        )?;
        let table = crate::timesheet::capture(&d, crate::timesheet::Options::default(), B)?;
        let sheet = crate::timesheet::export(
            &table,
            &dir.join("cel-export"),
            &cancel,
            &progress,
            &mut |from, to| std::fs::rename(from, to).map_err(|e| e.to_string()),
        )?;
        check(
            sheet.images.iter().any(|s| s.path.contains("Cel_001_1")),
            "Cel naming rule was not used by PNG filenames",
        )?;
        let shots = dir.join("shots.drawer-shots");
        crate::shot_exchange::export(&d, &shots, B, &cancel, &mut |from, to| {
            std::fs::rename(from, to).map_err(|e| e.to_string())
        })?;
        let exported: crate::shot_exchange::File =
            serde_json::from_slice(&std::fs::read(&shots).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?;
        check(
            exported.shots[0].name == "Shot_001" && exported.shots[0].cells[0].name == "Cel_001_1",
            "Shot/cel export names differ from reviewed names",
        )?;
        let wav = dir.join("range.wav");
        let dest = crate::audio_mix::Destination::capture(&mix, &wav, false)?;
        crate::audio_mix::write(&mix, &dest, &cancel, &std::sync::atomic::AtomicU64::new(0))?;
        let decoded = crate::audio::decode(&std::fs::read(&wav).map_err(|e| e.to_string())?, None)?;
        check(
            decoded.samples[offset] == 5000 && decoded.samples[..offset].iter().all(|n| *n == 0),
            "WAV output did not preserve frame sync / gain",
        )?;
        let path = dir.join("finish.drawer");
        crate::files::save(&mut d, &path).map_err(|e| e.to_string())?;
        let loaded = crate::files::open(&path, B).map_err(|e| e.to_string())?;
        check(
            loaded.snapshot() == d.snapshot(),
            "Finishing geometry/audio/names changed after native reopen",
        )?;
        let mut bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
        bytes[8..12].copy_from_slice(&105u32.to_le_bytes());
        std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
        check(
            crate::files::open(&path, B).is_err(),
            "Old version accepted new safe/output metadata",
        )?;
        let p = core::Preferences {
            view: View {
                overflow: true,
                ..Default::default()
            },
            ..Default::default()
        };
        p.save(&dir)?;
        check(
            core::Preferences::load(&dir)? == p,
            "Finishing view/naming defaults not restored",
        )?;
        let before = d.snapshot();
        for e in [
            Edit::Output {
                settings: Settings {
                    safe: Some([0, 100]),
                    area: None,
                },
                first: 0,
                end: 1,
                bounds: None,
            },
            Edit::Output {
                settings: Default::default(),
                first: 5,
                end: 5,
                bounds: None,
            },
            Edit::Names {
                cels: false,
                shots: false,
                cel: Default::default(),
                shot: Default::default(),
            },
            Edit::Names {
                cels: true,
                shots: false,
                cel: Rule {
                    pattern: "Same".into(),
                    ..Default::default()
                },
                shot: Default::default(),
            },
        ] {
            check(
                core::prepare(&d, e, B, &cancel).is_err() && d.snapshot() == before,
                "Invalid/duplicate naming or range changed document",
            )?;
        }
        let _ = track;
        Ok(())
    })();
    let clean = std::fs::remove_dir_all(&dir).map_err(|e| e.to_string());
    result.and(clean)
}
