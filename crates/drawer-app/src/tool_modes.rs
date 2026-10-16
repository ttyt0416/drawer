use super::*;
use drawer_core::{
    assets::BrushTip,
    brush_material::{Mode, Settings},
    input_replay::{Sample, Sequence},
    pointer_input::{Phase, Source},
};
fn dots(tool: BrushTip, background: [u8; 4], repeats: usize) -> Sequence {
    let mut seq = Sequence {
        tool,
        size: 1.0,
        aspect: 1.0,
        background,
        color: [220, 70, 30, 64],
        ..Default::default()
    };
    let n = BrushTip::ALL.iter().position(|t| *t == tool).unwrap();
    seq.pressure.tools[n].size = false;
    seq.pressure.tools[n].opacity = false;
    let template = seq.samples[0].event;
    seq.samples.clear();
    for stroke in 0..repeats {
        for (step, phase) in [Phase::Down, Phase::Move, Phase::Up]
            .into_iter()
            .enumerate()
        {
            let mut event = template;
            event.source = Source::MOUSE;
            event.phase = phase;
            event.position = [24.5, 20.5];
            event.pressure = None;
            seq.samples.push(Sample {
                micros: (stroke * 1000 + step * 100) as u64,
                event,
            });
        }
    }
    seq.expected_strokes = repeats;
    seq
}
pub(super) fn smoke(root: &std::path::Path) -> Result<(), String> {
    use std::sync::atomic::{AtomicBool, AtomicUsize};
    drawer_core::tool_modes::verify(root)?;
    let cancel = AtomicBool::new(false);
    let progress = AtomicUsize::new(0);
    for alpha in [0, 128, 255] {
        for repeats in [1, 2, 10] {
            let background = [20, 40, 80, alpha];
            let mut seq = dots(BrushTip::PixelPen, background, repeats);
            seq.profiles[6].pixel.overwrite = true;
            seq.profiles[6].pixel.pressure = false;
            seq.expected_strokes = 1;
            let overwrite = super::input_replay::run(&seq, &cancel, &progress)?;
            let at = (20 * seq.width + 24) as usize;
            if !overwrite.record.passed
                || overwrite.pixels[at] != seq.color
                || overwrite
                    .pixels
                    .iter()
                    .enumerate()
                    .any(|(n, p)| n != at && *p != background)
            {
                return Err("Pixel overwrite production input accumulated/escaped bounds".into());
            }
            let normal_seq = dots(BrushTip::Pen, background, repeats);
            let normal = super::input_replay::run(&normal_seq, &cancel, &progress)?;
            if !normal.record.passed
                || ((alpha > 0 || repeats > 1) && normal.pixels[at] == overwrite.pixels[at])
            {
                return Err("Normal accumulation was not distinct from overwrite".into());
            }
        }
    }
    for alpha in [0, 128, 255] {
        let mut seq = dots(BrushTip::Marker, [20, 40, 80, alpha], 1);
        seq.materials[4].alpha_lock = true;
        seq.expected_strokes = usize::from(alpha > 0);
        let outcome = super::input_replay::run(&seq, &cancel, &progress)?;
        if !outcome.record.passed
            || outcome.pixels.iter().any(|p| p[3] != alpha)
            || (alpha == 0 && outcome.pixels.iter().any(|p| *p != seq.background))
        {
            return Err("Actual marker changed the original alpha array".into());
        }
    }
    let mut transparent = dots(BrushTip::PixelPen, [20, 40, 80, 255], 1);
    transparent.color[3] = 0;
    transparent.profiles[6].pixel.overwrite = true;
    transparent.profiles[6].pixel.pressure = false;
    let cleared = super::input_replay::run(&transparent, &cancel, &progress)?;
    if !cleared.record.passed
        || cleared.pixels[(20 * transparent.width + 24) as usize] != transparent.color
    {
        return Err("Actual transparent RGBA overwrite did not replace alpha".into());
    }
    for p in [0.1999, 0.2, 0.2001] {
        let mut seq = Sequence {
            background: [0, 0, 255, 255],
            color: [0, 255, 0, 255],
            ..Default::default()
        };
        seq.pressure.tools[0].size = false;
        seq.pressure.tools[0].opacity = false;
        seq.materials[0] = Settings {
            mode: Mode::Water,
            supply: 1.0,
            mixing: 1.0,
            retention: 0.0,
            smudge_distance: 0.0,
            water: 0.0,
            ..Default::default()
        };
        for sample in &mut seq.samples {
            sample.event.pressure = Some(p);
        }
        seq.expected_strokes = if p < 0.2 { 0 } else { 1 };
        let outcome = super::input_replay::run(&seq, &cancel, &progress)?;
        if !outcome.record.passed
            || ((p < 0.2) && outcome.pixels.iter().any(|v| *v != seq.background))
            || ((p >= 0.2) && outcome.pixels.iter().all(|v| *v == seq.background))
        {
            return Err("Actual pen supply threshold boundary failed".into());
        }
    }
    for colorless in [false, true] {
        let mut seq = Sequence {
            background: [0, 0, 255, 255],
            color: [0, 255, 0, 255],
            ..Default::default()
        };
        seq.pressure.tools[0].size = false;
        seq.pressure.tools[0].opacity = false;
        seq.materials[0] = Settings {
            mode: Mode::Water,
            supply: 1.0,
            mixing: 1.0,
            retention: 0.0,
            smudge_distance: 0.0,
            water: 0.0,
            colorless,
            ..Default::default()
        };
        for sample in &mut seq.samples {
            sample.event.source = Source::MOUSE;
            sample.event.pressure = None;
        }
        seq.expected_strokes = if colorless { 0 } else { 1 };
        let outcome = super::input_replay::run(&seq, &cancel, &progress)?;
        if !outcome.record.passed
            || (colorless && outcome.pixels.iter().any(|v| *v != seq.background))
        {
            return Err("Mouse colorless supply toggle failed".into());
        }
    }
    let ctx = egui::Context::default();
    let user = root.join("tool-modes-user");
    let mut app = Drawer::new(&ctx, None)
        .with_storage(UserStorage::at(user.clone()).map_err(|e| e.to_string())?);
    app.sync_tool_properties();
    app.create_canvas(CanvasSpec {
        width: 2,
        height: 2,
        dpi: 300.0,
        background: Background::Transparent,
    })
    .map_err(|e| e.to_string())?;
    let before = app.documents[0].snapshot();
    let mut next = app.capture_workspace();
    next.input.profiles[6].pixel.overwrite = true;
    next.input.materials[4].alpha_lock = true;
    next.input.materials[0].mode = Mode::Water;
    next.input.materials[0].supply_threshold = 0.33;
    next.input.materials[0].colorless = true;
    for tip in [BrushTip::PixelPen, BrushTip::Marker, BrushTip::Pen] {
        let p = drawer_core::brush_presets::Preset::capture(&next, tip, "Tool mode")?;
        let copy =
            drawer_core::brush_presets::Preset::decode(&p.encode().map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?;
        if p != copy {
            return Err("Tool mode preset values lost".into());
        }
    }
    app.stage_workspace(next.clone(), app.workspaces.store.active.clone());
    app.apply_workspace(
        drawer_core::workspaces::Scope {
            other: true,
            ..Default::default()
        },
        None,
    )?;
    let restarted = Drawer::new(&egui::Context::default(), None)
        .with_storage(UserStorage::at(user).map_err(|e| e.to_string())?);
    if restarted.capture_workspace().input.profiles != next.input.profiles
        || restarted.capture_workspace().input.materials != next.input.materials
        || app.documents[0].snapshot() != before
        || app.documents[0].history_counts() != (0, 0)
    {
        return Err("Tool modes settings restart changed data or artwork".into());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn actual_overwrite_accumulation_water_mouse_toggle_and_settings() {
        let root = crate::tests::TestDirectory::new();
        super::smoke(&root.0).unwrap();
    }
}
