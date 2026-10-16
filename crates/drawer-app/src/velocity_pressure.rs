use super::*;
pub(super) fn smoke(root: &std::path::Path) -> Result<(), String> {
    use drawer_core::{
        brush_dynamics::Curve,
        input_replay::Sequence,
        pointer_input::Source,
        pressure::{Calibration, Response},
    };
    use std::sync::atomic::{AtomicBool, AtomicUsize};
    drawer_core::brush_dynamics::verify_velocity(root)?;
    let cancel = AtomicBool::new(false);
    let progress = AtomicUsize::new(0);
    for mouse in [false, true] {
        let mut seq = Sequence {
            size: 8.0,
            aspect: 1.0,
            ..Default::default()
        };
        seq.pressure.tools[0].size = false;
        seq.pressure.tools[0].opacity = false;
        seq.dynamics[0].opacity_speed = Curve {
            enabled: true,
            maximum: 1000.0,
            curve: [1.0, 0.75, 0.5, 0.25, 0.0],
            output_minimum: 0.1,
            ..Default::default()
        };
        for sample in &mut seq.samples {
            sample.event.pressure = Some(1.0);
            if mouse {
                sample.event.source = Source::MOUSE;
                sample.event.pressure = None;
            }
        }
        let fast = super::input_replay::run(&seq, &cancel, &progress)?;
        for sample in &mut seq.samples {
            sample.micros *= 100;
        }
        let slow = super::input_replay::run(&seq, &cancel, &progress)?;
        if !fast.record.passed || !slow.record.passed || fast.pixels == slow.pixels {
            return Err("Real mouse/pen speed did not independently control opacity".into());
        }
        let repeat = super::input_replay::run(&seq, &cancel, &progress)?;
        if repeat.pixels != slow.pixels {
            return Err("Speed output was not reproducible".into());
        }
        for sample in &mut seq.samples {
            sample.micros = 0;
        }
        let equal = super::input_replay::run(&seq, &cancel, &progress)?;
        if !equal.record.passed {
            return Err("Equal-time production replay failed".into());
        }
        seq.samples.pop();
        seq.expected_strokes = 0;
        let cancelled = super::input_replay::run(&seq, &cancel, &progress)?;
        if !cancelled.record.passed || cancelled.pixels.iter().any(|p| p[3] != 0) {
            return Err("Speed stroke cancellation changed pixels".into());
        }
    }
    let mut seq = Sequence {
        size: 8.0,
        aspect: 1.0,
        ..Default::default()
    };
    seq.pressure.tools[0].size = false;
    seq.pressure.tools[0].opacity = true;
    for sample in &mut seq.samples {
        sample.event.pressure = Some(0.5);
    }
    let normal = super::input_replay::run(&seq, &cancel, &progress)?;
    let source = seq.samples[0].event.source;
    seq.pressure.calibrations[0] = Some(Calibration {
        kind: source.kind,
        device: source.device,
        response: Response {
            curve: [0.0, 0.1, 0.2, 0.7, 1.0],
            ..Default::default()
        },
    });
    let calibrated = super::input_replay::run(&seq, &cancel, &progress)?;
    if !normal.record.passed
        || !calibrated.record.passed
        || normal.pixels == calibrated.pixels
        || calibrated.pixels.iter().any(|p| p[3] > 51)
    {
        return Err("Per-device pressure was not used in the canvas input path".into());
    }
    for sample in &mut seq.samples {
        sample.event.source.device = source.device.wrapping_add(1);
    }
    let other = super::input_replay::run(&seq, &cancel, &progress)?;
    if !other.record.passed || other.pixels != normal.pixels {
        return Err("A calibration leaked to another device ID".into());
    }
    let ctx = egui::Context::default();
    let user = root.join("velocity-user");
    let mut app = Drawer::new(&ctx, None)
        .with_storage(UserStorage::at(user.clone()).map_err(|e| e.to_string())?);
    app.sync_tool_properties();
    let baseline = app.capture_workspace();
    let mut next = baseline.clone();
    next.pressure = seq.pressure;
    next.input.dynamics[0].opacity_speed = Curve {
        enabled: true,
        maximum: 1500.0,
        output_minimum: 0.2,
        ..Default::default()
    };
    let preset =
        drawer_core::brush_presets::Preset::capture(&next, BrushTip::Pen, "Speed opacity")?;
    let preset =
        drawer_core::brush_presets::Preset::decode(&preset.encode().map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
    if preset.apply(&baseline)?.input.dynamics[0].opacity_speed
        != next.input.dynamics[0].opacity_speed
        || next.input.dynamics[1..] != baseline.input.dynamics[1..]
    {
        return Err("Speed opacity preset or brush isolation failed".into());
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
    if restarted.capture_workspace().input.dynamics != next.input.dynamics
        || restarted.capture_workspace().pressure != next.pressure
    {
        return Err("Velocity/device calibration settings lost on restart".into());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn speed_opacity_mouse_pen_timestamps_device_calibration_and_restart() {
        let root = crate::tests::TestDirectory::new();
        super::smoke(&root.0).unwrap();
    }
}
