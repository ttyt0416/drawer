//! Real codec roundtrips, reusable by the installed offline smoke test.
use crate::{
    animation_export::{self as output, Format, Options},
    document::DEFAULT_DOCUMENT_BUDGET as B,
};
use std::{
    path::Path,
    sync::atomic::{AtomicBool, AtomicU32},
};
fn check(b: bool, m: &str) -> Result<(), String> {
    if b { Ok(()) } else { Err(m.into()) }
}
pub fn verify(root: &Path) -> Result<(), String> {
    let dir = root.join("animation-delivery-verify");
    std::fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let result = (|| {
        let mut d = crate::animation_export_verify::fixture();
        d.animation.as_mut().unwrap().frames = 10;
        d.animation.as_mut().unwrap().tracks[0].end = 10;
        let before = d.snapshot();
        let frame = d.current_frame;
        let history = d.history_counts();
        let revision = d.revision;
        let dirty = d.dirty;
        let cancel = AtomicBool::new(false);
        let progress = AtomicU32::new(0);
        for (format, extension) in [
            (Format::Gif, "gif"),
            (Format::Webp, "webp"),
            (Format::Apng, "apng"),
        ] {
            let o = Options {
                last: 9,
                format,
                plays: 2,
                ..Default::default()
            };
            let s = output::prepare(&d, o, B)?;
            let out = dir.join(format!("result.{extension}"));
            let report = output::publish(&s, &out, false, &cancel, &progress);
            check(
                report.error.is_none() && report.frames == 10 && out.is_file(),
                &format!("{extension} roundtrip: {:?}", report.error),
            )?;
            let original = std::fs::read(&out).map_err(|e| e.to_string())?;
            let second = dir.join(format!("repeat.{extension}"));
            output::write(&s, &second, false, &cancel, &progress)?;
            check(
                original == std::fs::read(&second).map_err(|e| e.to_string())?,
                "Repeated frozen animation changed bytes",
            )?;
            check(
                output::write(&s, &out, false, &cancel, &progress).is_err(),
                "Output replacement bypassed confirmation",
            )?;
            if format == Format::Webp {
                let mut r = image_webp::WebPDecoder::new(std::io::BufReader::new(
                    std::fs::File::open(&out).unwrap(),
                ))
                .map_err(|e| e.to_string())?;
                let mut bytes = vec![0; 8];
                check(
                    r.dimensions() == (2, 1)
                        && r.num_frames() == 10
                        && r.icc_profile().map_err(|e| e.to_string())?.is_some(),
                    "WebP size/count/ICC",
                )?;
                let mut total = 0;
                for n in 0..10 {
                    total += r.read_frame(&mut bytes).map_err(|e| e.to_string())?;
                    if n == 0 {
                        check(bytes[3] == 128 && bytes[7] == 0, "WebP RGBA alpha")?;
                    }
                    if n == 1 {
                        check(
                            bytes[3] == 255 && bytes[7] == 64,
                            "WebP semi-alpha frame replacement",
                        )?;
                    }
                }
                check(total == 334, "WebP cumulative rational timing")?;
                check(
                    matches!(r.loop_count(),image_webp::LoopCount::Times(n)if n.get()==2),
                    "WebP complete loop count",
                )?;
            }
            if format == Format::Gif {
                let mut options = gif::DecodeOptions::new();
                options.set_color_output(gif::ColorOutput::RGBA);
                let mut r = options
                    .read_info(std::fs::File::open(&out).unwrap())
                    .map_err(|e| e.to_string())?;
                let mut total = 0;
                let mut n = 0;
                while let Some(f) = r.read_next_frame().map_err(|e| e.to_string())? {
                    total += f.delay as u32;
                    if n == 0 {
                        check(
                            f.buffer[3] == 255 && f.buffer[7] == 0,
                            "GIF explicit alpha threshold",
                        )?;
                    }
                    if n == 1 {
                        check(
                            f.buffer[3] == 255 && f.buffer[7] == 0,
                            "GIF semi-alpha loss policy",
                        )?;
                    }
                    n += 1;
                }
                check(
                    n == 10 && total == 33,
                    "GIF cumulative rational timing/count",
                )?;
                check(
                    r.repeat() == gif::Repeat::Finite(1),
                    "GIF plays to repeat translation",
                )?;
            }
            let receipt: serde_json::Value =
                serde_json::from_slice(&std::fs::read(report.log.unwrap()).unwrap())
                    .map_err(|e| e.to_string())?;
            check(
                receipt["status"] == "completed"
                    && receipt["completed_frames"] == 10
                    && receipt["not_completed"].as_array().unwrap().is_empty(),
                "Output receipt completion",
            )?;
        }
        let s = output::prepare(
            &d,
            Options {
                format: Format::Webp,
                ..Default::default()
            },
            B,
        )?;
        let single = dir.join("single.webp");
        output::write(&s, &single, false, &cancel, &progress)?;
        check(
            output::prepare(
                &d,
                Options {
                    format: Format::Movie,
                    ..Default::default()
                },
                B,
            )
            .err()
            .is_some_and(|e| e.contains("PNG")),
            "Missing local encoder did not offer PNG+WAV",
        )?;
        let mut fast = d.clone();
        fast.animation.as_mut().unwrap().fps = crate::animation::Fps {
            numerator: 240,
            denominator: 1,
        };
        check(
            output::prepare(
                &fast,
                Options {
                    format: Format::Gif,
                    ..Default::default()
                },
                B,
            )
            .is_err(),
            "GIF silently accepted unrepresentable zero-delay frames",
        )?;
        check(
            d.snapshot() == before
                && d.current_frame == frame
                && d.history_counts() == history
                && d.revision == revision
                && d.dirty == dirty,
            "Output changed live document/view/history",
        )?;
        Ok(())
    })();
    let cleanup = std::fs::remove_dir_all(&dir).map_err(|e| e.to_string());
    result.and(cleanup)
}
#[cfg(test)]
mod tests {
    #[test]
    fn actual_formats_timing_alpha_icc_repeat_and_reexport() {
        let t = crate::test_support::TempDir::new();
        super::verify(&t.0).unwrap();
    }
}
