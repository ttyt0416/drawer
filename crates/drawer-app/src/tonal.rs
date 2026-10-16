use super::*;
use drawer_core::{
    brush_presets::Preset,
    tonal::{Band, Mode, Settings},
    tool_locks as locks,
};
pub(super) fn smoke(root: &std::path::Path) -> Result<(), String> {
    drawer_core::tonal_verify::verify(root)?;
    for mode in [Mode::Dodge, Mode::Burn] {
        for band in Band::ALL {
            production(mode, band)?;
        }
    }
    verify_builtin(root)?;
    verify_user(root)
}
fn production(mode: Mode, band: Band) -> Result<(), String> {
    let ctx = egui::Context::default();
    let mut app = Drawer::new(&ctx, None);
    app.simple_view = true;
    app.create_canvas(CanvasSpec {
        width: 64,
        height: 48,
        dpi: 300.0,
        background: Background::Transparent,
    })
    .map_err(|e| e.to_string())?;
    for y in 0..48 {
        for x in 0..64 {
            let value = if x < 21 {
                40
            } else if x < 43 {
                128
            } else {
                220
            };
            app.documents[0].layers[0].pixels[y * 64 + x] = [value, value, value, 128];
        }
    }
    app.references.session.workspace.color = [1, 2, 3, 0];
    app.panels.settings.brush_size = 8.0;
    app.workspaces.store.current.pressure.tools[0].size = false;
    app.workspaces.store.current.pressure.tools[0].opacity = false;
    app.workspaces.store.current.input.smoothing[0] = 0.0;
    let s = &mut app.workspaces.store.current.input.materials[0];
    s.tonal = Settings {
        mode,
        band,
        strength: 0.5,
    };
    s.mode = drawer_core::brush_material::Mode::Water;
    s.water = 1.0;
    s.edge = 1.0;
    s.composition.mode = drawer_core::brush_composite::Mode::ReplaceAlpha;
    super::input_replay::frame(&ctx, &mut app, 0.0, vec![]);
    super::input_replay::frame(&ctx, &mut app, 0.1, vec![]);
    let before = app.documents[0].snapshot();
    use drawer_core::pointer_input::Phase;
    let events = [
        ([4.5, 24.5], Phase::Down),
        ([59.5, 24.5], Phase::Move),
        ([59.5, 24.5], Phase::Up),
    ]
    .map(|(p, phase)| super::precision::pointer(&ctx, &app, p, phase));
    app.drawing.virtual_events(events.to_vec(), false);
    super::input_replay::frame(&ctx, &mut app, 0.2, vec![]);
    for (x, value, b) in [
        (10, 40, Band::Shadows),
        (32, 128, Band::Midtones),
        (54, 220, Band::Highlights),
    ] {
        let expected = if band == b {
            match (mode, value) {
                (Mode::Dodge, 40) => 148,
                (Mode::Dodge, 128) => 192,
                (Mode::Dodge, 220) => 238,
                (Mode::Burn, 40) => 20,
                (Mode::Burn, 128) => 64,
                (Mode::Burn, 220) => 110,
                _ => unreachable!(),
            }
        } else {
            value
        };
        if app.documents[0].layers[0].pixel(64, 48, x, 24) != [expected, expected, expected, 128] {
            return Err(format!(
                "Production tone pixel failed: {mode:?}/{band:?} at {x}: {:?}",
                app.documents[0].layers[0].pixel(64, 48, x, 24)
            ));
        }
    }
    let after = app.documents[0].snapshot();
    if app.documents[0].history_counts() != (1, 0)
        || !app.documents[0].undo()
        || app.documents[0].snapshot() != before
        || !app.documents[0].redo()
        || app.documents[0].snapshot() != after
    {
        return Err("Production tone undo/redo failed".into());
    }
    Ok(())
}
fn verify_builtin(root: &std::path::Path) -> Result<(), String> {
    let ctx = egui::Context::default();
    let user = root.join("locked-builtin-user");
    let mut app = Drawer::new(&ctx, None)
        .with_storage(UserStorage::at(user.clone()).map_err(|e| e.to_string())?);
    app.sync_tool_properties();
    app.simple_view = true;
    app.create_canvas(CanvasSpec {
        width: 64,
        height: 48,
        dpi: 300.0,
        background: Background::Color([220; 3]),
    })
    .map_err(|e| e.to_string())?;
    super::input_replay::frame(&ctx, &mut app, 0.0, vec![]);
    super::input_replay::frame(&ctx, &mut app, 0.1, vec![]);
    let baseline = app.capture_workspace();
    let next = locks::set(&baseline, BrushTip::Pen, true)?;
    app.restore_workspace(&next);
    app.workspaces.store.current = next;
    app.sync_tool_properties();
    app.panels.settings.brush_size = 37.0;
    app.workspaces.store.current.input.materials[0].tonal = Settings {
        mode: Mode::Dodge,
        band: Band::Highlights,
        strength: 0.73,
    };
    app.sync_tool_properties();
    app.persist_workspace()?;
    if app.panels.settings.brush_size != 37.0
        || app.workspaces.store.current.input.materials[0]
            .tonal
            .strength
            != 0.73
    {
        return Err("Saving settings discarded live locked temporary values".into());
    }
    use drawer_core::pointer_input::Phase;
    for (n, point) in [[12.0, 24.0], [48.0, 24.0]].into_iter().enumerate() {
        let events = [Phase::Down, Phase::Up]
            .map(|phase| super::precision::pointer(&ctx, &app, point, phase));
        app.drawing.virtual_events(events.to_vec(), false);
        super::input_replay::frame(&ctx, &mut app, 0.2 + n as f64 * 0.1, vec![]);
        if app.panels.settings.brush_size != 37.0
            || app.workspaces.store.current.input.materials[0]
                .tonal
                .strength
                != 0.73
        {
            return Err("Repeated strokes discarded locked temporary values".into());
        }
    }
    if app.documents[0].history_counts() != (2, 0) {
        return Err("Locked tool repeated strokes were not committed separately".into());
    }
    app.set_tool(BrushTip::Pen);
    if app.panels.settings.brush_size != 37.0 {
        return Err("Selecting same locked tool discarded temporary values".into());
    }
    let restarted = Drawer::new(&egui::Context::default(), None)
        .with_storage(UserStorage::at(user).map_err(|e| e.to_string())?);
    if restarted.capture_workspace().input.materials[0] != baseline.input.materials[0]
        || restarted.panels.settings.brush_size != baseline.panels.brush_size
    {
        return Err("Restart retained locked temporary values".into());
    }
    app.set_tool(BrushTip::Pencil);
    app.set_tool(BrushTip::Pen);
    if app.panels.settings.brush_size != baseline.panels.brush_size
        || app.workspaces.store.current.input.materials[0] != baseline.input.materials[0]
    {
        return Err("Tool switch did not restore locked builtin".into());
    }
    app.panels.settings.brush_size = 29.0;
    app.sync_tool_properties();
    app.workspaces.store.current.input.modifiers[0] = Some(BrushTip::Pencil);
    super::input_replay::frame(
        &ctx,
        &mut app,
        0.5,
        vec![egui::Event::ModifiersChanged(egui::Modifiers::CTRL)],
    );
    if app.selected != BrushTip::Pencil
        || app.tool_input.controller.persisted_tool() != Some(BrushTip::Pen)
    {
        return Err("Locked modifier switch cancelled its controller state".into());
    }
    super::input_replay::frame(
        &ctx,
        &mut app,
        0.6,
        vec![egui::Event::ModifiersChanged(egui::Modifiers::NONE)],
    );
    if app.selected != BrushTip::Pen || app.panels.settings.brush_size != baseline.panels.brush_size
    {
        return Err("Modifier release failed to restore the locked baseline".into());
    }
    app.workspaces.store.current.input.modifiers[0] = None;
    // Tone controls survive a preset and restart when the tool is unlocked.
    let mut next = locks::set(&app.capture_workspace(), BrushTip::Pen, false)?;
    next.input.materials[0].tonal = Settings {
        mode: Mode::Burn,
        band: Band::Midtones,
        strength: 0.66,
    };
    let p = Preset::capture(&next, BrushTip::Pen, "Tone copy")?;
    if Preset::decode(&p.encode().map_err(|e| e.to_string())?).map_err(|e| e.to_string())? != p {
        return Err("Tone preset round trip failed".into());
    }
    app.restore_workspace(&next);
    app.workspaces.store.current = next;
    app.persist_workspace()?;
    Ok(())
}
fn verify_user(root: &std::path::Path) -> Result<(), String> {
    let ctx = egui::Context::default();
    let user = root.join("locked-user-tool");
    let mut app = Drawer::new(&ctx, None)
        .with_storage(UserStorage::at(user.clone()).map_err(|e| e.to_string())?);
    app.sync_tool_properties();
    let mut s = app.capture_workspace();
    let a = Preset::capture(&s, BrushTip::Pen, "Locked ink")?;
    let mut b = a.clone();
    b.name = "Other ink".into();
    b.values[0] = 5.0;
    let id =
        s.user_tools
            .create_tool(1, a, drawer_core::tool_properties::Property::ALL.to_vec())?;
    let other =
        s.user_tools
            .create_tool(1, b, drawer_core::tool_properties::Property::ALL.to_vec())?;
    app.restore_workspace(&s);
    app.workspaces.store.current = s;
    app.select_user_tool(id)?;
    let next = locks::set(&app.capture_workspace(), BrushTip::Pen, true)?;
    app.restore_workspace(&next);
    app.workspaces.store.current = next;
    app.sync_tool_properties();
    let original = app
        .capture_workspace()
        .user_tools
        .entry(id)
        .unwrap()
        .preset
        .clone();
    app.panels.settings.brush_size = 41.0;
    app.workspaces.store.current.input.materials[0].tonal.mode = Mode::Burn;
    app.sync_tool_properties();
    app.persist_workspace()?;
    app.select_user_tool(id)?;
    if app.panels.settings.brush_size != 41.0
        || app.capture_workspace().user_tools.entry(id).unwrap().preset != original
    {
        return Err("Same user tool or auto-save changed the lock baseline".into());
    }
    let restarted = Drawer::new(&egui::Context::default(), None)
        .with_storage(UserStorage::at(user).map_err(|e| e.to_string())?);
    if restarted.panels.settings.brush_size != original.values[0]
        || restarted.capture_workspace().input.materials[0] != original.material
    {
        return Err("Restart failed to restore locked user tool".into());
    }
    app.select_user_tool(other)?;
    app.select_user_tool(id)?;
    if app.panels.settings.brush_size != original.values[0]
        || app.capture_workspace().input.materials[0] != original.material
    {
        return Err("User tool switch failed to restore lock baseline".into());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn production_tone_bands_and_locked_tool_restart() {
        let root = crate::tests::TestDirectory::new();
        super::smoke(&root.0).unwrap();
    }
}
