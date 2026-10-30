use crate::{
    animation::{Cell, Exposure, Fps, Timeline, Track},
    animation_export::{self as output, Format, Options},
    document::{Background, CanvasSpec, DEFAULT_DOCUMENT_BUDGET as B, Document},
};
use std::{
    fs,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU32},
    },
};
pub fn fixture() -> Document {
    let mut d = Document::create_with_color(
        CanvasSpec {
            width: 2,
            height: 1,
            dpi: 300.,
            background: Background::Transparent,
        },
        B,
        Default::default(),
    )
    .unwrap();
    let mut s = d.snapshot();
    s.animation = Some(Timeline {
        fps: Fps {
            numerator: 30000,
            denominator: 1001,
        },
        frames: 3,
        audio: vec![],
        tracks: vec![Track {
            name: "Colors".into(),
            cells: vec![
                Cell {
                    name: "Red".into(),
                    width: 2,
                    height: 1,
                    offset: [0, 0],
                    pixels: Arc::new(vec![[255, 0, 0, 128], [0, 0, 0, 0]]),
                },
                Cell {
                    name: "Blue".into(),
                    width: 2,
                    height: 1,
                    offset: [0, 0],
                    pixels: Arc::new(vec![[0, 0, 255, 255], [0, 200, 100, 64]]),
                },
            ],
            exposures: vec![
                Exposure { frame: 0, cell: 0 },
                Exposure { frame: 1, cell: 1 },
                Exposure { frame: 2, cell: 0 },
            ],
            end: 3,
        }],
    });
    d.commit_state(s).unwrap();
    d.commit_state(crate::cels::enable(&d, B).unwrap()).unwrap();
    d.current_frame = 2;
    d
}
fn check(ok: bool, message: &str) -> Result<(), String> {
    if ok { Ok(()) } else { Err(message.into()) }
}
pub fn verify(root: &Path) -> Result<(), String> {
    let dir = root.join("animation-export-verify");
    fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let result = (|| {
        let mut d = fixture();
        let state = d.snapshot();
        let history = d.history_counts();
        let revision = d.revision;
        let frame = d.current_frame;
        let dirty = d.dirty;
        let opts = Options {
            first: 0,
            last: 2,
            format: Format::PngSequence,
            plays: 0,
            ..Default::default()
        };
        check(
            output::prepare(&d, Options { last: 3, ..opts }, B).is_err()
                && output::prepare(
                    &d,
                    Options {
                        first: 2,
                        last: 1,
                        ..opts
                    },
                    B,
                )
                .is_err()
                && output::prepare(&d, opts, 8).is_err(),
            "Invalid range/budget accepted",
        )?;
        let snapshot = output::prepare(&d, opts, B)?;
        let cancel = AtomicBool::new(false);
        let progress = AtomicU32::new(0);
        let seq = dir.join("sequence");
        let r = output::write(&snapshot, &seq, false, &cancel, &progress)?;
        check(
            r.frames == 3
                && r.files.len() == 4
                && progress.load(std::sync::atomic::Ordering::Relaxed) == 3,
            "PNG sequence count/progress",
        )?;
        check(
            d.snapshot() == state
                && d.current_frame == frame
                && d.history_counts() == history
                && d.revision == revision
                && d.dirty == dirty,
            "PNG export changed source/frame/history/dirty",
        )?;
        let first = image::open(seq.join("frame-000001.png"))
            .map_err(|e| e.to_string())?
            .into_rgba8();
        check(
            first.get_pixel(0, 0).0 == [255, 0, 0, 128] && first.get_pixel(1, 0).0 == [0, 0, 0, 0],
            "PNG alpha/color loss",
        )?;
        check(
            output::write(&snapshot, &seq, true, &cancel, &progress).is_err(),
            "Existing sequence folder was merged",
        )?;
        // Mutating the live shared definition after capture cannot alter the frozen output.
        let source = d.scene.cels.definitions[0].object;
        let mut child = d.scene.cels.get(source).unwrap().open()?;
        child.edit_pixel(0, 0, 0, [0, 255, 0, 255]);
        d.commit_state(crate::cels::replace(&d, source, &child, B)?)?;
        d.current_frame = 1;
        let again = dir.join("frozen");
        output::write(&snapshot, &again, false, &cancel, &progress)?;
        for n in 1..=3 {
            let name = format!("frame-{n:06}.png");
            check(
                fs::read(seq.join(&name)).unwrap() == fs::read(again.join(&name)).unwrap(),
                "Output followed a later live edit",
            )?;
        }
        check(
            d.undo() && d.snapshot() == state,
            "Live test edit Undo failed",
        )?;
        d.current_frame = frame;
        let apng = output::prepare(
            &d,
            Options {
                format: Format::Apng,
                ..opts
            },
            B,
        )?;
        let path = dir.join("result.apng");
        output::write(&apng, &path, false, &cancel, &progress)?;
        let mut reader = png::Decoder::new(std::io::BufReader::new(fs::File::open(&path).unwrap()))
            .read_info()
            .map_err(|e| e.to_string())?;
        check(
            reader
                .info()
                .animation_control
                .is_some_and(|c| c.num_frames == 3 && c.num_plays == 0)
                && reader.info().icc_profile.is_some(),
            "APNG count/loop/ICC missing",
        )?;
        let mut bytes = vec![0; reader.output_buffer_size().unwrap()];
        for n in 1..=3 {
            let info = reader.next_frame(&mut bytes).map_err(|e| e.to_string())?;
            let control = reader.info().frame_control.unwrap();
            check(
                control.delay_num == 1001
                    && control.delay_den == 30000
                    && control.blend_op == png::BlendOp::Source
                    && control.dispose_op == png::DisposeOp::None,
                "APNG rational timing/full-frame replacement",
            )?;
            let expected = image::open(seq.join(format!("frame-{n:06}.png")))
                .unwrap()
                .into_rgba8()
                .into_raw();
            check(
                bytes[..info.buffer_size()] == expected,
                "Decoded APNG differs from PNG sequence",
            )?;
        }
        drop(reader);
        let old = fs::read(&path).unwrap();
        check(
            output::write(&apng, &path, false, &cancel, &progress).is_err()
                && fs::read(&path).unwrap() == old,
            "APNG overwrote without confirmation",
        )?;
        output::write(&apng, &path, true, &cancel, &progress)?;
        check(
            fs::read(&path).unwrap() == old,
            "Repeated APNG changed bytes",
        )?;
        let one = output::prepare(
            &d,
            Options {
                first: 2,
                last: 2,
                format: Format::Apng,
                plays: 1,
                ..Default::default()
            },
            B,
        )?;
        output::write(&one, &dir.join("single.apng"), false, &cancel, &progress)?;
        check(
            output::write(
                &one,
                &dir.join("cancel.apng"),
                false,
                &AtomicBool::new(true),
                &progress,
            )
            .is_err()
                && !dir.join("cancel.apng").exists(),
            "Pre-cancel published output",
        )?;
        check(
            d.snapshot() == state
                && d.current_frame == frame
                && d.dirty == dirty
                && d.revision > revision
                && d.history_counts().0 == history.0,
            "Export changed drawing/frame/history/dirty",
        )?;
        check(
            fs::read_dir(&dir).unwrap().all(|e| {
                !e.unwrap()
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".Drawer-animation")
            }),
            "Owned export staging folder left behind",
        )?;
        Ok(())
    })();
    let cleanup = fs::remove_dir_all(&dir).map_err(|e| e.to_string());
    result.and(cleanup)
}

pub type PngReader = png::Reader<std::io::BufReader<fs::File>>;
pub fn reader(path: &Path) -> Result<PngReader, String> {
    png::Decoder::new(std::io::BufReader::new(
        fs::File::open(path).map_err(|e| e.to_string())?,
    ))
    .read_info()
    .map_err(|e| e.to_string())
}
