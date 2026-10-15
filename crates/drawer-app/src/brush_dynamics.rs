use super::*;
use drawer_core::brush_dynamics::{Curve, Direction, Settings};
pub(super) fn settings_ui(ui: &mut egui::Ui, s: &mut Settings) {
    egui::ComboBox::from_id_salt("dynamic-direction")
        .selected_text(s.direction.label())
        .show_ui(ui, |ui| {
            for direction in Direction::ALL {
                ui.selectable_value(&mut s.direction, direction, direction.label());
            }
        });
    super::edit::number(
        ui,
        &mut s.fallback_angle,
        -180.0..=180.0,
        1.0,
        " deg fixed / missing direction",
        "brush-dynamics",
    );
    super::edit::number(
        ui,
        &mut s.scatter_angle,
        -180.0..=180.0,
        1.0,
        " deg scatter ellipse direction",
        "brush-dynamics",
    );
    crate::help::checkbox(
        ui,
        &mut s.spacing_with_size,
        "Scale spacing with tip size",
        "brush-dynamics",
    );
    for (name, c, limit, unit) in [
        ("Pressure", &mut s.pressure, 1.0, " pressure"),
        ("Tilt", &mut s.tilt, 90.0, " degrees from perpendicular"),
        ("Speed", &mut s.speed, 1e6, " document px/s"),
    ] {
        ui.push_id(name, |ui| {
            ui.collapsing(format!("{name} size factor"), |ui| {
                curve_ui(ui, c, limit, unit)
            });
        });
    }
    super::edit::slider(
        ui,
        &mut s.random_minimum,
        0.0..=1.0,
        "Random size: minimum multiplier (1 = off)",
        "brush-dynamics",
    );
    super::edit::number(
        ui,
        &mut s.seed,
        0.0..=u32::MAX as f64,
        1.0,
        " size random seed",
        "brush-dynamics",
    );
    super::edit::number(
        ui,
        &mut s.minimum_size,
        0.0..=4096.0,
        0.1,
        " px minimum tip size",
        "brush-dynamics",
    );
    super::edit::number(
        ui,
        &mut s.maximum_size,
        0.1..=4096.0,
        0.1,
        " px maximum tip size",
        "brush-dynamics",
    );
    if !s.valid() {
        ui.colored_label(
            egui::Color32::YELLOW,
            "Input minimum must be below maximum; size minimum must not exceed maximum.",
        );
    }
    ui.label("Additional size factors multiply the existing pressure/profile output. Random size is applied last, then the size bounds. Start/end geometry can still reach zero. Missing input uses its multiplier below. Fixed direction also preserves the separate legacy pen-tilt option.");
}
fn curve_ui(ui: &mut egui::Ui, c: &mut Curve, limit: f64, unit: &str) {
    crate::help::checkbox(
        ui,
        &mut c.enabled,
        "Enable this size factor",
        "brush-dynamics",
    );
    super::edit::number(
        ui,
        &mut c.minimum,
        0.0..=limit,
        0.01,
        &format!(" minimum input{unit}"),
        "brush-dynamics",
    );
    super::edit::number(
        ui,
        &mut c.maximum,
        0.0..=limit,
        0.01,
        &format!(" maximum input{unit}"),
        "brush-dynamics",
    );
    super::edit::slider(
        ui,
        &mut c.output_minimum,
        0.0..=1.0,
        "Minimum output multiplier",
        "brush-dynamics",
    );
    super::edit::slider(
        ui,
        &mut c.output_maximum,
        0.0..=1.0,
        "Maximum output multiplier",
        "brush-dynamics",
    );
    for (n, knot) in c.curve.iter_mut().enumerate() {
        super::edit::slider(
            ui,
            knot,
            0.0..=1.0,
            format!("Curve at {}% of input range", n * 25),
            "brush-dynamics",
        );
    }
    super::edit::slider(
        ui,
        &mut c.missing,
        0.0..=1.0,
        "Missing input multiplier",
        "brush-dynamics",
    );
    if c.valid(limit) {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(220.0, 65.0), egui::Sense::hover());
        ui.painter().rect_stroke(
            rect,
            0.0,
            egui::Stroke::new(1.0, egui::Color32::GRAY),
            egui::StrokeKind::Inside,
        );
        let points = (0..=64)
            .map(|n| {
                let x = n as f64 / 64.0;
                let mut active = *c;
                active.enabled = true;
                let y = active.map(Some(c.minimum + (c.maximum - c.minimum) * x));
                egui::pos2(
                    rect.left() + x as f32 * rect.width(),
                    rect.bottom() - y as f32 * rect.height(),
                )
            })
            .collect();
        ui.painter().add(egui::Shape::line(
            points,
            egui::Stroke::new(1.5, egui::Color32::LIGHT_BLUE),
        ));
    }
}

pub(super) fn smoke() -> Result<(), String> {
    use drawer_core::{brush_dynamics::Direction, input_replay::Sequence};
    use std::sync::atomic::{AtomicBool, AtomicUsize};
    drawer_core::brush_dynamics::verify()?;
    let cancel = AtomicBool::new(false);
    let progress = AtomicUsize::new(0);
    let mut s = Sequence {
        size: 12.0,
        aspect: 0.4,
        ..Default::default()
    };
    s.pressure.tools[0].size = false;
    s.pressure.tools[0].opacity = false;
    for sample in &mut s.samples {
        sample.event.pressure = Some(1.0);
    }
    s.dynamics[0].pressure.enabled = true;
    let full = super::input_replay::run(&s, &cancel, &progress)?;
    for sample in &mut s.samples {
        sample.event.pressure = Some(0.25);
    }
    let low = super::input_replay::run(&s, &cancel, &progress)?;
    if !full.record.passed || !low.record.passed || full.pixels == low.pixels {
        return Err("Pressure dynamics did not reach actual input strokes".into());
    }
    s.dynamics[0] = Settings::default();
    s.dynamics[0].tilt.enabled = true;
    s.dynamics[0].tilt.missing = 0.2;
    s.dynamics[0].direction = Direction::Pen;
    s.dynamics[0].fallback_angle = 0.0;
    for sample in &mut s.samples {
        sample.event.pressure = Some(1.0);
        sample.event.attributes.tilt = None;
    }
    let missing = super::input_replay::run(&s, &cancel, &progress)?;
    for sample in &mut s.samples {
        sample.event.attributes.tilt = Some([0.0, std::f64::consts::FRAC_PI_4]);
    }
    let tilt = super::input_replay::run(&s, &cancel, &progress)?;
    if !missing.record.passed || !tilt.record.passed || missing.pixels == tilt.pixels {
        return Err("Tilt size/direction or missing fallback was ignored".into());
    }
    s.dynamics[0] = Settings::default();
    s.dynamics[0].speed.enabled = true;
    s.dynamics[0].speed.curve = [1.0, 0.75, 0.5, 0.25, 0.0];
    for sample in &mut s.samples {
        sample.event.source = drawer_core::pointer_input::Source::MOUSE;
        sample.event.attributes.tilt = None;
        sample.event.pressure = None;
    }
    let fast = super::input_replay::run(&s, &cancel, &progress)?;
    for sample in &mut s.samples {
        sample.micros *= 100;
    }
    let slow = super::input_replay::run(&s, &cancel, &progress)?;
    if !fast.record.passed || !slow.record.passed || fast.pixels == slow.pixels {
        return Err("Mouse timestamp speed curve was ignored".into());
    }
    s.dynamics[0].random_minimum = 0.2;
    s.dynamics[0].seed = 981;
    let a = super::input_replay::run(&s, &cancel, &progress)?;
    let b = super::input_replay::run(&s, &cancel, &progress)?;
    if !a.record.passed || !b.record.passed || a.pixels != b.pixels {
        return Err("Random size production replay changed".into());
    }
    s.samples.pop();
    s.expected_strokes = 0;
    let cancelled = super::input_replay::run(&s, &cancel, &progress)?;
    if !cancelled.record.passed || cancelled.pixels.iter().any(|p| p[3] != 0) {
        return Err("Pending dynamic stroke did not cancel atomically".into());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn actual_input_pressure_tilt_speed_mouse_fallback_and_seed() {
        super::smoke().unwrap();
    }
}
