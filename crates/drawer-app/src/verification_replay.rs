use super::editor::check;
use drawer_core::{input_replay::Sequence, pointer_input::Source};
use sha2::{Digest, Sha256};
use std::{
    path::Path,
    sync::atomic::{AtomicBool, AtomicUsize},
};

pub(super) fn run(root: &Path) -> Result<serde_json::Value, String> {
    let mut reports = Vec::new();
    for (name, mouse) in [("pen-tilt", false), ("mouse", true)] {
        let mut sequence = Sequence::default();
        sequence.variations[0].seed = 301;
        sequence.variations[0].hue = 0.2;
        sequence.variations[0].scope = drawer_core::brush_variation::Scope::Dab;
        for (i, sample) in sequence.samples.iter_mut().enumerate() {
            sample.event.attributes.tilt = (!mouse).then_some([0.2, -0.3]);
            sample.event.attributes.time_micros = Some(sample.micros);
            if mouse {
                sample.event.source = Source::MOUSE;
                sample.event.pressure = None;
            }
            sample.event.position[1] += i as f64;
        }
        sequence.pen_backend_available = !mouse;
        let path = root.join(format!("{name}.json"));
        sequence.save(&path).map_err(|e| e.to_string())?;
        let loaded = Sequence::read(&path).map_err(|e| e.to_string())?;
        check(
            sequence == loaded,
            "Recorded coordinates, pressure, clock, tilt or seeds changed",
        )?;
        let hash = format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&loaded).map_err(|e| e.to_string())?)
        );
        let mut pixels = None;
        let mut runs = Vec::new();
        for n in 0..3 {
            let output =
                crate::input_replay::run(&loaded, &AtomicBool::new(false), &AtomicUsize::new(0))?;
            let m = output
                .record
                .measurement
                .as_ref()
                .ok_or("Missing replay measurement")?;
            check(
                output.record.passed
                    && !output.record.incomplete_cancelled
                    && m.input_sha256 == hash
                    && m.version == 1
                    && !m.physical_device_validated
                    && m.display_latency_ms.is_none()
                    && !m.cpu_frames_ms.is_empty()
                    && m.cpu_frames_ms.iter().all(|v| v.is_finite() && *v >= 0.)
                    && m.cpu_total_ms.is_finite()
                    && m.cpu_total_ms > 0.,
                "Replay result identity/timing failed",
            )?;
            if let Some(previous) = &pixels {
                check(*previous == output.pixels, "Repeated replay RGBA differs")?;
            }
            pixels = Some(output.pixels);
            let report_path = root.join(format!("{name}-{n}.result.json"));
            output
                .record
                .save(&report_path)
                .map_err(|e| e.to_string())?;
            check(
                Sequence::read(&report_path).map_err(|e| e.to_string())? == loaded,
                "Result cannot restore its original input",
            )?;
            runs.push(serde_json::to_value(output.record).map_err(|e| e.to_string())?);
        }
        reports.push(serde_json::json!({"case":name,"inputSha256":hash,"seed":301,"exactRgbaRepeats":3,"runs":runs}));
    }
    Ok(
        serde_json::json!({"mode":"local recorded input through production CPU editor", "cases":reports,
        "timingScope":"CPU input dispatch and editor frame; recorded event clock is not wall-clock playback or display latency"}),
    )
}
#[cfg(test)]
mod tests {
    #[test]
    fn saved_input_seeds_identity_timing_and_exact_repeats() {
        let root = crate::tests::TestDirectory::new();
        super::run(&root.0).unwrap();
    }
}
