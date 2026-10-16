use super::*;
use drawer_core::brush_composite::{Mode, Settings};
pub(super) fn smoke(root: &std::path::Path) -> Result<(), String> {
    use std::sync::atomic::{AtomicBool, AtomicUsize};
    drawer_core::brush_composite::verify(root)?;
    let cancel = AtomicBool::new(false);
    let progress = AtomicUsize::new(0);
    for mode in Mode::ALL {
        let mut seq = drawer_core::input_replay::Sequence {
            background: [80, 150, 210, 128],
            color: [220, 90, 40, 192],
            ..Default::default()
        };
        seq.materials[0].composition = Settings {
            mode,
            seed: 71,
            ..Default::default()
        };
        seq.profiles[0].minimum_opacity = 1.0;
        seq.pressure.tools[0].size = false;
        let a = super::input_replay::run(&seq, &cancel, &progress)?;
        let b = super::input_replay::run(&seq, &cancel, &progress)?;
        if !a.record.passed || a.pixels != b.pixels || a.pixels.iter().all(|p| *p == seq.background)
        {
            return Err(format!(
                "Composition input {mode:?} did not draw/repeat/undo"
            ));
        }
        seq.samples.pop();
        seq.expected_strokes = 0;
        let c = super::input_replay::run(&seq, &cancel, &progress)?;
        if !c.record.passed || c.pixels.iter().any(|p| *p != seq.background) {
            return Err("Composition pending stroke was not cancelled".into());
        }
    }
    let ctx = egui::Context::default();
    let user = root.join("composition-user");
    let mut app = Drawer::new(&ctx, None)
        .with_storage(UserStorage::at(user.clone()).map_err(|e| e.to_string())?);
    app.sync_tool_properties();
    let baseline = app.capture_workspace();
    let mut preset =
        drawer_core::brush_presets::Preset::capture(&baseline, BrushTip::Pen, "Dissolve pressure")?;
    preset.material.composition = Settings {
        mode: Mode::Dissolve,
        seed: 88,
        render_quality: drawer_core::color_mix::Quality::High,
        mixing_pressure: drawer_core::brush_dynamics::Curve {
            enabled: true,
            curve: [1.0, 0.8, 0.5, 0.2, 0.0],
            ..Default::default()
        },
    };
    preset.material.quality = drawer_core::color_mix::Quality::High;
    preset.filters.pressure_enabled = true;
    preset.filters.pressure_strength = 0.7;
    preset.values[3] = 0.6;
    let encoded = preset.encode().map_err(|e| e.to_string())?;
    let preset = drawer_core::brush_presets::Preset::decode(&encoded).map_err(|e| e.to_string())?;
    let next = preset.apply(&baseline)?;
    if next.input.materials[1..] != baseline.input.materials[1..]
        || next.input.filters[1..] != baseline.input.filters[1..]
        || next.input.materials[0] != preset.material
    {
        return Err("Composition preset leaked across brushes".into());
    }
    app.stage_workspace(next, app.workspaces.store.active.clone());
    app.apply_workspace(
        drawer_core::workspaces::Scope {
            other: true,
            ..Default::default()
        },
        None,
    )?;
    let restarted = Drawer::new(&egui::Context::default(), None)
        .with_storage(UserStorage::at(user).map_err(|e| e.to_string())?);
    let saved = restarted.capture_workspace();
    if saved.input.materials[0] != preset.material || saved.input.filters[0] != preset.filters {
        return Err("Composition/quality/pressure/stabilization not restored".into());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn production_composition_replay_cancel_and_independent_settings_restart() {
        let root = crate::tests::TestDirectory::new();
        super::smoke(&root.0).unwrap();
    }
}
