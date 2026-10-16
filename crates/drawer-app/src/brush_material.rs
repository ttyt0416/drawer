use super::*;
use drawer_core::brush_material::{Mode, Settings};
pub(super) fn settings_ui(ui: &mut egui::Ui, s: &mut Settings, tip: BrushTip, overwrite: bool) {
    egui::ComboBox::from_id_salt("brush-render-quality")
        .selected_text(s.composition.render_quality.label())
        .show_ui(ui, |ui| {
            for quality in drawer_core::color_mix::Quality::ALL {
                ui.selectable_value(&mut s.composition.render_quality, quality, quality.label());
            }
        });
    ui.label(
        "Rendering quality samples the geometric edge. Pixel pen remains on the exact pixel grid.",
    );
    if tip == BrushTip::Eraser {
        crate::help::checkbox(
            ui,
            &mut s.alpha_lock,
            "Keep the layer's original alpha",
            "brush-material",
        );
        ui.label("Eraser removes alpha. Pigment mixing controls belong to color brushes.");
        return;
    }
    egui::ComboBox::from_id_salt("brush-composition")
        .selected_text(s.composition.mode.label())
        .show_ui(ui, |ui| {
            for mode in drawer_core::brush_composite::Mode::ALL {
                ui.selectable_value(&mut s.composition.mode, mode, mode.label());
            }
        });
    super::edit::number(
        ui,
        &mut s.composition.seed,
        0.0..=u32::MAX as f64,
        1.0,
        " dissolve seed",
        "brush-composite",
    );
    if tip == BrushTip::PixelPen
        && overwrite
        && s.composition.mode == drawer_core::brush_composite::Mode::Normal
    {
        crate::help::checkbox(
            ui,
            &mut s.alpha_lock,
            "Keep the layer's original alpha",
            "tool-modes",
        );
        ui.label("Pixel overwrite uses the brush RGBA directly. Pigment pickup and diffusion are bypassed.");
        return;
    }
    ui.collapsing("Pressure controls pigment pickup", |ui| {
        crate::help::checkbox(
            ui,
            &mut s.composition.mixing_pressure.enabled,
            "Enable pressure mixing multiplier",
            "brush-composite",
        );
        for (n, value) in s.composition.mixing_pressure.curve.iter_mut().enumerate() {
            super::edit::slider(
                ui,
                value,
                0.0..=1.0,
                format!("Pickup multiplier at {}% pressure", n * 25),
                "brush-composite",
            );
        }
    });
    let smudge = tip == BrushTip::Smudge;
    if smudge {
        s.mode = Mode::Smudge;
        s.supply = 0.0;
        s.colorless = true;
    }
    ui.add_enabled_ui(!smudge, |ui| {
        egui::ComboBox::from_id_salt("pigment-mode")
            .selected_text(s.mode.label())
            .show_ui(ui, |ui| {
                for mode in Mode::ALL {
                    ui.selectable_value(&mut s.mode, mode, mode.label());
                }
            });
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
        egui::ComboBox::from_id_salt("mix-space")
            .selected_text(s.space.label())
            .show_ui(ui, |ui| {
                for space in drawer_core::color_mix::Space::ALL {
                    ui.selectable_value(&mut s.space, space, space.label());
                }
            });
        super::edit::slider(
            ui,
            &mut s.mixing,
            0.0..=1.0,
            "Layer pigment pickup",
            "brush-material",
        );
    });
    ui.add_enabled_ui(
        s.mode != Mode::Paint && s.mode != Mode::RunningColor,
        |ui| {
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
        },
    );
    ui.add_enabled_ui(s.mode == Mode::RunningColor, |ui| {
        egui::ComboBox::from_id_salt("mix-quality")
            .selected_text(s.quality.label())
            .show_ui(ui, |ui| {
                for quality in drawer_core::color_mix::Quality::ALL {
                    ui.selectable_value(&mut s.quality, quality, quality.label());
                }
            });
    });
    if s.mode == Mode::RunningColor {
        ui.label("Running color uses the current local pigment without carrying color from earlier positions. High quality reads more samples inside the footprint and selection.");
    }
    ui.add_enabled_ui(s.mode == Mode::Water, |ui| {
        super::edit::slider(
            ui,
            &mut s.supply_threshold,
            0.0..=1.0,
            "Minimum pressure for foreground supply (0 = off)",
            "tool-modes",
        );
        super::edit::slider(
            ui,
            &mut s.water,
            0.0..=1.0,
            "Water / diffusion",
            "brush-material",
        );
    });
    super::edit::slider(
        ui,
        &mut s.dilution,
        0.0..=1.0,
        "Dilution (independent of water)",
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
    ui.add_enabled_ui(!smudge, |ui| {
        crate::help::checkbox(
            ui,
            &mut s.colorless,
            "Colorless smudge (no foreground supply)",
            "brush-material",
        )
    });
    ui.label("Smudge always carries existing pigment. Density retention can paint into empty pixels; alpha lock prevents that.");
    if ui
        .add_enabled(
            !smudge,
            egui::Button::new("Load watercolor values into draft"),
        )
        .clicked()
    {
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

pub(super) fn smoke_independent() -> Result<(), String> {
    use drawer_core::input_replay::Sequence;
    use std::sync::atomic::{AtomicBool, AtomicUsize};
    drawer_core::brush_material::verify_independent()?;
    let cancel = AtomicBool::new(false);
    let progress = AtomicUsize::new(0);
    for mouse in [false, true] {
        let mut s = Sequence::default();
        s.pressure.tools[0].size = false;
        s.pressure.tools[0].opacity = true;
        s.materials[0] = Settings {
            mode: Mode::Water,
            supply: 1.0,
            mixing: 0.0,
            retention: 0.0,
            water: 0.0,
            dilution: 0.5,
            ..Default::default()
        };
        for sample in &mut s.samples {
            sample.event.pressure = Some(1.0);
            if mouse {
                sample.event.source = drawer_core::pointer_input::Source::MOUSE;
                sample.event.pressure = None;
            }
        }
        let half = super::input_replay::run(&s, &cancel, &progress)?;
        if !half.record.passed
            || half.pixels.iter().any(|p| p[3] > 128)
            || half.pixels.iter().all(|p| p[3] != 128)
        {
            return Err("Dry dilution did not reduce the native mouse/pen pigment alpha".into());
        }
        for water in [0.0, 1.0] {
            s.materials[0].water = water;
            s.materials[0].dilution = 1.0;
            s.expected_strokes = 0;
            let blank = super::input_replay::run(&s, &cancel, &progress)?;
            if !blank.record.passed || blank.pixels.iter().any(|p| p[3] > 0) {
                return Err("Full dilution depended on the diffusion water setting".into());
            }
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
    #[test]
    fn independent_zero_water_dilution_in_mouse_and_pen_pipeline() {
        super::smoke_independent().unwrap();
    }
}
