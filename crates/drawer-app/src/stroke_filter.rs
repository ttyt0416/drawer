use super::*;
use drawer_core::stroke_filter::Settings;
pub(super) fn settings_ui(ui: &mut egui::Ui, s: &mut Settings) {
    crate::help::checkbox(
        ui,
        &mut s.position_enabled,
        "Enable position stabilization",
        "stroke-filter",
    );
    crate::help::checkbox(
        ui,
        &mut s.pressure_enabled,
        "Enable device pressure smoothing",
        "stroke-filter",
    );
    ui.add_enabled_ui(s.pressure_enabled, |ui| {
        super::edit::slider(
            ui,
            &mut s.pressure_strength,
            0.0..=0.95,
            "Pressure smoothing strength",
            "stroke-filter",
        );
    });
    ui.label("Position strength is the tool's stabilization value above. Device pressure is filtered once per input event. The mouse's distance/time pressure envelope is configured separately.");
}

pub(super) fn smoke() -> Result<(), String> {
    use drawer_core::input_replay::Sequence;
    use std::sync::atomic::{AtomicBool, AtomicUsize};
    drawer_core::stroke_filter::verify()?;
    let cancel = AtomicBool::new(false);
    let progress = AtomicUsize::new(0);
    let mut s = Sequence::default();
    s.smoothing[0] = 0.8;
    s.filters[0] = Settings {
        position_enabled: false,
        pressure_enabled: true,
        pressure_strength: 0.75,
    };
    s.pressure.tools[0].size = true;
    s.pressure.tools[0].opacity = true;
    for (sample, p) in s.samples.iter_mut().zip([0.2, 0.8, 0.2, 0.8]) {
        sample.event.pressure = Some(p);
    }
    let filtered = super::input_replay::run(&s, &cancel, &progress)?;
    s.filters[0].pressure_enabled = false;
    let raw = super::input_replay::run(&s, &cancel, &progress)?;
    for (sample, q) in s.samples.iter_mut().zip([0.2, 0.35, 0.3125, 0.434375]) {
        sample.event.pressure = Some(q);
    }
    let expected = super::input_replay::run(&s, &cancel, &progress)?;
    if !filtered.record.passed
        || !expected.record.passed
        || filtered.pixels != expected.pixels
        || filtered.pixels == raw.pixels
    {
        return Err("Device pressure filter was coupled to position or interpolated dabs".into());
    }
    for sample in &mut s.samples {
        sample.event.source = drawer_core::pointer_input::Source::MOUSE;
        sample.event.pressure = None;
    }
    let raw = super::input_replay::run(&s, &cancel, &progress)?;
    s.filters[0].pressure_enabled = true;
    let smooth = super::input_replay::run(&s, &cancel, &progress)?;
    if !raw.record.passed || !smooth.record.passed || raw.pixels != smooth.pixels {
        return Err("Missing mouse pressure was changed from full pressure".into());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn native_pen_event_filter_and_missing_mouse_pressure_use_production_replay() {
        super::smoke().unwrap();
    }
}
