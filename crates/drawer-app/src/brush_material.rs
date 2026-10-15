use super::*;
use drawer_core::brush_material::{Mode, Settings};
pub(super) fn settings_ui(ui: &mut egui::Ui, s: &mut Settings) {
    egui::ComboBox::from_id_salt("pigment-mode")
        .selected_text(s.mode.label())
        .show_ui(ui, |ui| {
            for mode in Mode::ALL {
                ui.selectable_value(&mut s.mode, mode, mode.label());
            }
        });
    ui.add_enabled_ui(s.mode != Mode::Smudge && !s.colorless, |ui| {
        super::edit::slider(
            ui,
            &mut s.supply,
            0.0..=1.0,
            "Foreground supply",
            "brush-material",
        );
    });
    ui.add_enabled_ui(s.mode != Mode::Paint || s.colorless, |ui| {
        super::edit::slider(
            ui,
            &mut s.mixing,
            0.0..=1.0,
            "Layer pigment pickup",
            "brush-material",
        );
        super::edit::slider(
            ui,
            &mut s.retention,
            0.0..=1.0,
            "Carried color retention",
            "brush-material",
        );
        super::edit::number(
            ui,
            &mut s.smudge_distance,
            0.0..=4096.0,
            1.0,
            " px smear distance",
            "brush-material",
        );
    });
    super::edit::slider(
        ui,
        &mut s.water,
        0.0..=1.0,
        "Water / diffusion",
        "brush-material",
    );
    super::edit::slider(
        ui,
        &mut s.dilution,
        0.0..=1.0,
        "Dilution (0: none)",
        "brush-material",
    );
    crate::help::checkbox(
        ui,
        &mut s.keep_density,
        "Keep pigment density when leaving painted areas",
        "brush-material",
    );
    crate::help::checkbox(
        ui,
        &mut s.alpha_lock,
        "Keep the layer's original alpha",
        "brush-material",
    );
    crate::help::checkbox(
        ui,
        &mut s.colorless,
        "Colorless smudge (no foreground supply)",
        "brush-material",
    );
    ui.label("Smudge always carries existing pigment. Density retention can paint into empty pixels; alpha lock prevents that.");
    if ui.button("Load watercolor values into draft").clicked() {
        *s = Settings {
            mode: Mode::Water,
            supply: 0.2,
            mixing: 0.5,
            retention: 0.8,
            water: 0.5,
            ..Default::default()
        };
        super::edit::discard_layer(ui.ctx(), ui.layer_id());
    }
}

pub(super) fn smoke() -> Result<(), String> {
    use drawer_core::input_replay::Sequence;
    use std::sync::atomic::{AtomicBool, AtomicUsize};
    let cancel = AtomicBool::new(false);
    let progress = AtomicUsize::new(0);
    for mouse in [false, true] {
        let mut s = Sequence::default();
        s.materials[0] = Settings {
            mode: Mode::Water,
            supply: 0.2,
            mixing: 0.5,
            retention: 0.8,
            water: 0.5,
            ..Default::default()
        };
        if mouse {
            for sample in &mut s.samples {
                sample.event.source = drawer_core::pointer_input::Source::MOUSE;
                sample.event.pressure = None;
            }
        }
        let a = super::input_replay::run(&s, &cancel, &progress)?;
        let b = super::input_replay::run(&s, &cancel, &progress)?;
        if !a.record.passed
            || !b.record.passed
            || a.pixels != b.pixels
            || a.pixels.iter().all(|p| p[3] == 0)
        {
            return Err("Watercolor device replay/pixels/history diverged".into());
        }
        s.background = [255, 0, 0, 255];
        s.color = [0, 255, 0, 0];
        s.materials[0] = Settings {
            mode: Mode::Smudge,
            colorless: true,
            retention: 1.0,
            mixing: 0.0,
            water: 0.0,
            ..Default::default()
        };
        s.expected_strokes = 0;
        let no_foreground = super::input_replay::run(&s, &cancel, &progress)?;
        if !no_foreground.record.passed
            || no_foreground.pixels.iter().any(|p| *p != [255, 0, 0, 255])
        {
            return Err(
                "Colorless transparent-foreground smudge changed a uniform opaque layer".into(),
            );
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn pigment_water_native_mouse_replay_and_transparent_foreground() {
        super::smoke().unwrap();
    }
}
