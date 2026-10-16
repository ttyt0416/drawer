use super::*;
use drawer_core::brush_pattern::{Decoration, Mode, Settings};
pub(super) fn settings_ui(ui: &mut egui::Ui, s: &mut Settings) {
    ui.checkbox(
        &mut s.behavior.screen_size,
        "Keep size in physical screen pixels",
    );
    ui.checkbox(
        &mut s.behavior.minimum_pixel,
        "Minimum diameter: one document pixel",
    );
    ui.checkbox(
        &mut s.behavior.spacing_density,
        "Compensate density for path spacing",
    );
    ui.checkbox(
        &mut s.behavior.continuous,
        "Continuous emission while held (mouse or pen)",
    );
    if s.behavior.continuous {
        super::edit::number(
            ui,
            &mut s.behavior.rate,
            1.0..=200.0,
            1.0,
            " emissions per second",
            "brush-pattern",
        );
        ui.label("Emits on press and at fixed time intervals, including while stationary. Time emission ignores path spacing compensation. Release stops; Escape cancels. Focus loss cancels, except valid watercolor edge strokes, which finish. Existing Mouse spray keeps its own controls when selected.");
    }
    ui.checkbox(
        &mut s.behavior.fixed_scatter,
        "Keep scatter offsets fixed per tip and particle",
    );
    ui.label("Screen size resolves at press using physical display zoom; resolved diameter is capped at 4096 document px. Minimum applies after size dynamics; zero endpoint geometry remains empty. Spacing compensation uses .15 as its reference for accumulating coverage; capped/pixel/clone/special modes keep their coverage rules.");
    egui::ComboBox::from_id_salt("brush-pattern")
        .selected_text(s.mode.label())
        .show_ui(ui, |ui| {
            for mode in Mode::ALL {
                ui.selectable_value(&mut s.mode, mode, mode.label());
            }
        });
    match s.mode {
        Mode::Scatter => {
            super::edit::number(
                ui,
                &mut s.width,
                0.0..=4096.0,
                0.1,
                " px scatter diameter",
                "brush-pattern",
            );
            super::edit::number(
                ui,
                &mut s.count,
                1.0..=64.0,
                1.0,
                " particles per path stamp",
                "brush-pattern",
            );
            super::edit::slider(
                ui,
                &mut s.aspect,
                0.05..=1.0,
                "Scatter ellipse aspect",
                "brush-pattern",
            );
            super::edit::number(
                ui,
                &mut s.seed,
                0.0..=u32::MAX as f64,
                1.0,
                " scatter seed",
                "brush-pattern",
            );
            ui.label("Path spacing uses the tool's Spacing value. The ellipse direction and individual particle direction are separate choices in size dynamics.");
        }
        Mode::Ribbon => {
            ui.label("Connected strip with round joins/caps. Width is size × aspect. Keeps half-pixel path samples through turns; overlapping strip coverage is applied once. Ordinary stamp spacing is ignored.");
        }
        Mode::Decoration => {
            egui::ComboBox::from_id_salt("decoration-mask")
                .selected_text(s.decoration.label())
                .show_ui(ui, |ui| {
                    for mask in Decoration::ALL {
                        ui.selectable_value(&mut s.decoration, mask, mask.label());
                    }
                });
            super::edit::number(
                ui,
                &mut s.interval,
                0.25..=16.0,
                0.05,
                " tip widths between decorations",
                "brush-pattern",
            );
            let (rect, _) = ui.allocate_exact_size(egui::vec2(64.0, 64.0), egui::Sense::hover());
            for y in 0..32 {
                for x in 0..32 {
                    let q = [(x as f64 + 0.5) / 16.0 - 1.0, (y as f64 + 0.5) / 16.0 - 1.0];
                    let a = if s.decoration.distance(q) < 1.0 {
                        220
                    } else {
                        35
                    };
                    ui.painter().rect_filled(
                        egui::Rect::from_min_size(
                            rect.min + egui::vec2(x as f32 * 2.0, y as f32 * 2.0),
                            egui::vec2(2.0, 2.0),
                        ),
                        0.0,
                        egui::Color32::from_gray(a),
                    );
                }
            }
        }
        Mode::Dabs => {}
    }
    ui.label("Owned procedural shapes; no external material files. Settings freeze on press. Cancel discards the whole stroke; normal release adds one undo step. Combining timed mouse spray with a pattern applies the pattern to every timed particle.");
}

pub(super) fn smoke() -> Result<(), String> {
    use drawer_core::input_replay::Sequence;
    use std::sync::atomic::{AtomicBool, AtomicUsize};
    drawer_core::brush_pattern::verify()?;
    let cancel = AtomicBool::new(false);
    let progress = AtomicUsize::new(0);
    let mut s = Sequence {
        size: 8.0,
        ..Default::default()
    };
    s.pressure.tools[0].size = false;
    s.pressure.tools[0].opacity = false;
    for event in &mut s.samples {
        event.event.pressure = Some(1.0);
    }
    let mut results = Vec::new();
    for mode in [Mode::Scatter, Mode::Ribbon, Mode::Decoration] {
        s.patterns[0] = Settings {
            mode,
            width: 16.0,
            count: 5,
            seed: 123,
            interval: 2.0,
            ..Default::default()
        };
        let a = super::input_replay::run(&s, &cancel, &progress)?;
        let b = super::input_replay::run(&s, &cancel, &progress)?;
        if !a.record.passed
            || !b.record.passed
            || a.pixels != b.pixels
            || results.contains(&a.pixels)
        {
            return Err("Pattern production replay diverged or duplicated another mode".into());
        }
        results.push(a.pixels);
    }
    s.patterns[0].mode = Mode::Scatter;
    s.patterns[0].seed += 1;
    let other = super::input_replay::run(&s, &cancel, &progress)?;
    if !other.record.passed || other.pixels == results[0] {
        return Err("Scatter seed was not used by production input".into());
    }
    s.samples.pop();
    s.expected_strokes = 0;
    let cancelled = super::input_replay::run(&s, &cancel, &progress)?;
    if !cancelled.record.passed || cancelled.pixels.iter().any(|p| p[3] != 0) {
        return Err("Cancelled pattern stroke changed original pixels".into());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn pattern_modes_actual_input_seed_and_cancel() {
        super::smoke().unwrap();
    }
}
