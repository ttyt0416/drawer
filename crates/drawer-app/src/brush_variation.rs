use super::*;
use drawer_core::brush_variation::{Blend, Mask, Scope, Settings};
pub(super) fn settings_ui(ui: &mut egui::Ui, s: &mut Settings) {
    super::edit::number(
        ui,
        &mut s.hue,
        0.0..=180.0,
        1.0,
        " deg hue variation (+/-)",
        "brush-variation",
    );
    super::edit::slider(
        ui,
        &mut s.saturation,
        0.0..=1.0,
        "Saturation variation (+/-)",
        "brush-variation",
    );
    super::edit::slider(
        ui,
        &mut s.value,
        0.0..=1.0,
        "HSV brightness variation (+/-)",
        "brush-variation",
    );
    super::edit::number(
        ui,
        &mut s.seed,
        0.0..=u32::MAX as f64,
        1.0,
        " color / secondary grain seed",
        "brush-variation",
    );
    egui::ComboBox::from_id_salt("variation-scope")
        .selected_text(s.scope.label())
        .show_ui(ui, |ui| {
            for scope in Scope::ALL {
                ui.selectable_value(&mut s.scope, scope, scope.label());
            }
        });
    ui.label("Sample colors (alpha is unchanged)");
    let (rect, _) = ui.allocate_exact_size(egui::vec2(192.0, 24.0), egui::Sense::hover());
    for n in 0..8 {
        let c = s.color([60, 120, 200, 255], n);
        ui.painter().rect_filled(
            egui::Rect::from_min_size(
                rect.min + egui::vec2(n as f32 * 24.0, 0.0),
                egui::vec2(24.0, 24.0),
            ),
            0.0,
            egui::Color32::from_rgb(c[0], c[1], c[2]),
        );
    }
    crate::help::checkbox(
        ui,
        &mut s.dual.enabled,
        "Combine a second local tip mask",
        "brush-variation",
    );
    ui.add_enabled_ui(s.dual.enabled, |ui| {
        egui::ComboBox::from_id_salt("secondary-mask")
            .selected_text(s.dual.mask.label())
            .show_ui(ui, |ui| {
                for mask in Mask::ALL {
                    ui.selectable_value(&mut s.dual.mask, mask, mask.label());
                }
            });
        egui::ComboBox::from_id_salt("mask-blend")
            .selected_text(s.dual.blend.label())
            .show_ui(ui, |ui| {
                for blend in Blend::ALL {
                    ui.selectable_value(&mut s.dual.blend, blend, blend.label());
                }
            });
        super::edit::slider(
            ui,
            &mut s.dual.opacity,
            0.0..=1.0,
            "Secondary mask opacity (0 = no effect)",
            "brush-variation",
        );
        super::edit::slider(
            ui,
            &mut s.dual.scale,
            0.1..=1.0,
            "Secondary tip scale",
            "brush-variation",
        );
        super::edit::number(
            ui,
            &mut s.dual.angle,
            -180.0..=180.0,
            1.0,
            " deg secondary angle",
            "brush-variation",
        );
        for n in 0..2 {
            ui.push_id(n, |ui| {
                super::edit::number(
                    ui,
                    &mut s.dual.offset[n],
                    -1.0..=1.0,
                    0.01,
                    if n == 0 {
                        " X offset / primary radius"
                    } else {
                        " Y offset / primary radius"
                    },
                    "brush-variation",
                );
            });
        }
    });
    ui.label("The secondary tip combines coverage, using the same ink and stroke history. Add / maximum / difference can extend the footprint. The configured seed is frozen with the stroke; using it again reproduces the variation.");
}

pub(super) fn smoke() -> Result<(), String> {
    use drawer_core::{brush_variation::Dual, input_replay::Sequence};
    use std::sync::atomic::{AtomicBool, AtomicUsize};
    drawer_core::brush_variation::verify()?;
    let cancel = AtomicBool::new(false);
    let progress = AtomicUsize::new(0);
    let mut s = Sequence {
        size: 10.0,
        ..Default::default()
    };
    s.pressure.tools[0].size = false;
    s.pressure.tools[0].opacity = true;
    for sample in &mut s.samples {
        sample.event.pressure = Some(1.0);
    }
    // Keep the same reconstructed path for the disabled/transparent secondary comparison.
    s.variations[0].seed = 91;
    let plain = super::input_replay::run(&s, &cancel, &progress)?;
    s.variations[0].dual = Dual {
        enabled: true,
        opacity: 0.0,
        mask: Mask::Grain,
        blend: Blend::Add,
        ..Default::default()
    };
    let transparent = super::input_replay::run(&s, &cancel, &progress)?;
    if !plain.record.passed || !transparent.record.passed || plain.pixels != transparent.pixels {
        return Err("Transparent secondary changed production pixels".into());
    }
    s.variations[0] = Settings {
        hue: 90.0,
        saturation: 0.3,
        value: 0.3,
        scope: Scope::Dab,
        seed: 251,
        dual: Dual {
            enabled: true,
            mask: Mask::Grain,
            blend: Blend::Multiply,
            ..Default::default()
        },
    };
    let a = super::input_replay::run(&s, &cancel, &progress)?;
    let b = super::input_replay::run(&s, &cancel, &progress)?;
    if !a.record.passed || !b.record.passed || a.pixels != b.pixels || a.pixels == plain.pixels {
        return Err("Seeded production color/dual mask was ignored or unstable".into());
    }
    s.variations[0].seed += 1;
    let other = super::input_replay::run(&s, &cancel, &progress)?;
    if !other.record.passed || a.pixels == other.pixels {
        return Err("Production color/grain seed did not change output".into());
    }
    s.samples.pop();
    s.expected_strokes = 0;
    let cancelled = super::input_replay::run(&s, &cancel, &progress)?;
    if !cancelled.record.passed || cancelled.pixels.iter().any(|p| p[3] != 0) {
        return Err("Cancelled variation changed source".into());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn production_color_dual_seed_transparency_and_cancel() {
        super::smoke().unwrap();
    }
}
