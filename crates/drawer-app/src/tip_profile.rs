use super::*;
use drawer_core::tip_profile::{Image, Settings};
pub(super) fn settings_ui(ui: &mut egui::Ui, s: &mut Settings, tip: BrushTip) {
    if tip == BrushTip::PixelPen {
        crate::help::checkbox(
            ui,
            &mut s.pixel.overwrite,
            "Overwrite with brush RGBA (no buildup)",
            "tool-modes",
        );
        ui.label("Overwrite replaces pixel RGB and scales its alpha by coverage. It bypasses pigment/mouse postprocessing; special composition and clone keep their own rules.");
        crate::help::checkbox(
            ui,
            &mut s.pixel.pressure,
            "Pixel pen reacts to pressure",
            "pixel-eyedropper",
        );
        crate::help::checkbox(
            ui,
            &mut s.pixel.grid,
            "Pixel precision: snap to integer pixel grid",
            "pixel-eyedropper",
        );
        ui.label("Both coordinate modes keep a binary edge. A single pixel stays grid-aligned in both modes. Pressure off disables size, opacity and pressure-only dynamics/mixing; speed and tilt remain independent.");
    }
    for (opacity, value) in [(false, &mut s.minimum_size), (true, &mut s.minimum_opacity)] {
        let mut percent = *value * 100.0;
        super::edit::number(
            ui,
            &mut percent,
            0.0..=100.0,
            1.0,
            if opacity {
                "% minimum opacity"
            } else {
                "% minimum size"
            },
            "tip-profile",
        );
        *value = percent / 100.0;
    }
    ui.label("Minimum output at zero pressure; 100% keeps the maximum. Each pressure axis must be enabled in Brush pressure.");
    egui::ComboBox::from_id_salt("owned-tip-image")
        .selected_text(s.image.label())
        .show_ui(ui, |ui| {
            for image in Image::ALL {
                ui.selectable_value(&mut s.image, image, image.label());
            }
        });
    let mask = s.mask(tip, 16);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(48.0, 48.0), egui::Sense::hover());
    for (n, a) in mask.into_iter().enumerate() {
        let q = rect.min + egui::vec2((n % 16) as f32 * 3.0, (n / 16) as f32 * 3.0);
        ui.painter().rect_filled(
            egui::Rect::from_min_size(q, egui::vec2(3.0, 3.0)),
            0.0,
            egui::Color32::from_gray(a),
        );
    }
    ui.label("Tool properties → Hardness controls edge softness. Pixel pen keeps a binary edge.");
}

pub(super) fn smoke() -> Result<(), String> {
    use drawer_core::input_replay::Sequence;
    use std::sync::atomic::{AtomicBool, AtomicUsize};
    drawer_core::tip_profile::verify()?;
    let cancel = AtomicBool::new(false);
    let progress = AtomicUsize::new(0);
    let mut s = Sequence {
        size: 8.0,
        ..Default::default()
    };
    s.profiles[0] = Settings {
        minimum_size: 1.0,
        minimum_opacity: 1.0,
        pixel: Default::default(),
        image: Image::Square,
    };
    s.pressure.tools[0].size = true;
    s.pressure.tools[0].opacity = true;
    for sample in &mut s.samples {
        sample.event.pressure = Some(0.0);
    }
    let minimum = super::input_replay::run(&s, &cancel, &progress)?;
    for sample in &mut s.samples {
        sample.event.pressure = Some(1.0);
    }
    let maximum = super::input_replay::run(&s, &cancel, &progress)?;
    if !minimum.record.passed || !maximum.record.passed || minimum.pixels != maximum.pixels {
        return Err("Equal min/max output diverged at zero/full pen pressure".into());
    }
    s.profiles[0] = Default::default();
    for sample in &mut s.samples {
        sample.event.pressure = Some(0.0);
    }
    s.expected_strokes = 0;
    let blank = super::input_replay::run(&s, &cancel, &progress)?;
    if !blank.record.passed || blank.pixels.iter().any(|p| p[3] > 0) {
        return Err("Default zero-pressure output did not stay empty".into());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn zero_pressure_minimum_equals_maximum_in_actual_input_pipeline() {
        super::smoke().unwrap();
    }
}
