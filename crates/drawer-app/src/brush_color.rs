use super::*;
use drawer_core::{
    brush_variation::{Blend, Controls, Dual, Mask},
    color_mix::Space,
    input_replay::Sequence,
};
use std::sync::atomic::{AtomicBool, AtomicUsize};
fn dot() -> Sequence {
    let mut s = Sequence {
        color: [255, 0, 0, 128],
        ..Default::default()
    };
    s.pressure.tools[0].size = false;
    s.pressure.tools[0].opacity = false;
    for sample in &mut s.samples {
        sample.event.position = [24.5, 20.5];
        sample.event.pressure = Some(1.0);
    }
    s
}
pub(super) fn smoke(root: &std::path::Path) -> Result<(), String> {
    drawer_core::brush_color::verify(root)?;
    verify_modes()?;
    verify_mixing()?;
    verify_settings(root)
}
fn verify_modes() -> Result<(), String> {
    let cancel = AtomicBool::new(false);
    let progress = AtomicUsize::new(0);
    let mut changed = 0;
    for mode in Blend::ALL {
        let mut results = vec![];
        for rgb in [false, true] {
            let mut s = dot();
            s.variations[0].dual = Dual {
                enabled: true,
                blend: mode,
                scale: 1.0,
                mask: Mask::Square,
                controls: Controls {
                    rgb,
                    color: [0, 0, 255, 64],
                    ..Default::default()
                },
                ..Default::default()
            };
            s.expected_strokes = usize::from(
                !rgb || !matches!(mode, Blend::ColorBurn | Blend::LinearBurn | Blend::HardMix),
            );
            let result = super::input_replay::run(&s, &cancel, &progress)?;
            if !result.record.passed {
                return Err(format!("Dual production stroke count {mode:?}/{rgb}"));
            }
            results.push(result.pixels);
        }
        if results[0] != results[1] {
            changed += 1;
        }
    }
    if changed < 10 {
        return Err("Secondary RGB did not affect production color/alpha".into());
    }
    let mut s = dot();
    s.color = [200, 40, 90, 0];
    s.variations[0].dual = Dual {
        enabled: true,
        blend: Blend::Normal,
        scale: 1.0,
        controls: Controls {
            rgb: true,
            color: [0, 0, 255, 64],
            ..Default::default()
        },
        ..Default::default()
    };
    let result = super::input_replay::run(&s, &cancel, &progress)?;
    if !result.record.passed || result.pixels[20 * 64 + 24] != [0, 0, 255, 64] {
        return Err("Production replay skipped secondary ink over transparent primary".into());
    }
    Ok(())
}
fn verify_mixing() -> Result<(), String> {
    let cancel = AtomicBool::new(false);
    let progress = AtomicUsize::new(0);
    let mut colors = vec![];
    for (space, brightness) in [
        (Space::Standard, 0.0),
        (Space::LabD65, 0.0),
        (Space::Standard, 1.0),
    ] {
        let mut s = dot();
        s.color = [255, 0, 0, 255];
        s.background = [0, 0, 255, 255];
        s.materials[0] = drawer_core::brush_material::Settings {
            mode: drawer_core::brush_material::Mode::RunningColor,
            space,
            brightness,
            mixing: 0.5,
            supply: 1.0,
            ..Default::default()
        };
        let result = super::input_replay::run(&s, &cancel, &progress)?;
        if !result.record.passed {
            return Err("Lab/lightness production stroke failed".into());
        }
        colors.push(result.pixels[20 * 64 + 24]);
    }
    if colors[0] != [128, 0, 128, 255] || colors[1] != [202, 0, 136, 255] || colors[2] == colors[0]
    {
        return Err(format!("Production RGB/Lab/lightness: {colors:?}"));
    }
    Ok(())
}
fn verify_settings(root: &std::path::Path) -> Result<(), String> {
    let user = root.join("brush-color-user");
    let ctx = egui::Context::default();
    let mut app = Drawer::new(&ctx, None)
        .with_storage(UserStorage::at(user.clone()).map_err(|e| e.to_string())?);
    app.create_canvas(CanvasSpec {
        width: 8,
        height: 8,
        dpi: 300.0,
        background: Background::Transparent,
    })
    .map_err(|e| e.to_string())?;
    app.sync_tool_properties();
    let before = app.documents[0].snapshot();
    let mut s = app.capture_workspace();
    s.input.materials[0].space = Space::LabD65;
    s.input.materials[0].brightness = 0.66;
    s.input.variations[0].dual = Dual {
        enabled: true,
        blend: Blend::HeightLinear,
        controls: Controls {
            rgb: true,
            color: [20, 100, 220, 63],
            linked_size: false,
            size: 12.0,
        },
        ..Default::default()
    };
    let preset = drawer_core::brush_presets::Preset::capture(&s, BrushTip::Pen, "Two inks Lab")?;
    if drawer_core::brush_presets::Preset::decode(&preset.encode().map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?
        != preset
    {
        return Err("Lab/dual preset round trip lost controls".into());
    }
    app.stage_workspace(s, app.workspaces.store.active.clone());
    app.apply_workspace(
        drawer_core::workspaces::Scope {
            other: true,
            ..Default::default()
        },
        None,
    )?;
    if app.documents[0].snapshot() != before || app.documents[0].history_counts() != (0, 0) {
        return Err("Color settings rewrote the picture".into());
    }
    let restarted = Drawer::new(&egui::Context::default(), None)
        .with_storage(UserStorage::at(user).map_err(|e| e.to_string())?);
    if restarted.workspaces.store.current.input.materials[0].space != Space::LabD65
        || restarted.workspaces.store.current.input.materials[0].brightness != 0.66
        || restarted.workspaces.store.current.input.variations[0].dual != preset.variation.dual
    {
        return Err("Lab/dual controls did not survive restart".into());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn production_dual_rgb_lab_lightness_and_restart() {
        let root = crate::tests::TestDirectory::new();
        super::smoke(&root.0).unwrap();
    }
}
