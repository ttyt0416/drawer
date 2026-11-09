use super::*;
pub(super) fn run(root: &std::path::Path) -> Result<(), String> {
    let dir = root.join("preset-exchange-ui");
    std::fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let result = (|| {
        let source_dir = dir.join("source");
        std::fs::create_dir(&source_dir).map_err(|e| e.to_string())?;
        drawer_core::tool_bundle::verify(&source_dir)?;
        let source = source_dir.join("owned-tool-bundle");
        let user = dir.join("data");
        let ctx = egui::Context::default();
        let mut app = Drawer::new(&ctx, None)
            .with_storage(UserStorage::at(user.clone()).map_err(|e| e.to_string())?);
        app.simple_view = true;
        app.add_document(drawer_core::png_io_verify::fixture()?);
        let mut time = 0.;
        let frame = |a: &mut Drawer, t: &mut f64, events| {
            *t += 0.05;
            input_replay::frame(&ctx, a, *t, events);
        };
        let settle = |a: &mut Drawer, t: &mut f64| {
            for _ in 0..3 {
                frame(a, t, vec![]);
            }
        };
        let click = |a: &mut Drawer, t: &mut f64, key: &str| -> Result<(), String> {
            let r = a
                .brush_presets
                .controls
                .get(key)
                .copied()
                .ok_or_else(|| format!("Missing preset exchange control {key}"))?;
            if r.center().y > 690. {
                return Err(format!("Clipped preset control {key}"));
            }
            for pressed in [true, false] {
                frame(
                    a,
                    t,
                    vec![
                        egui::Event::PointerMoved(r.center()),
                        egui::Event::PointerButton {
                            pos: r.center(),
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: Default::default(),
                        },
                    ],
                );
            }
            settle(a, t);
            Ok(())
        };
        let stage = |a: &mut Drawer, t: &mut f64| -> Result<(), String> {
            a.brush_presets.open = true;
            settle(a, t);
            let mut e = a.brush_presets.editor.take().ok_or("No preset draft")?;
            a.stage_portable_tool_bundle(&mut e.tools, &source)?;
            a.brush_presets.editor = Some(e);
            settle(a, t);
            Ok(())
        };
        settle(&mut app, &mut time);
        let doc = app.documents[0].snapshot();
        let tools = app.capture_workspace().user_tools;
        let materials = app.materials.library.clone();
        let path = user.join("materials/library.json");
        stage(&mut app, &mut time)?;
        if app.materials.library != materials
            || path.exists()
            || app.workspaces.store.current.user_tools != tools
        {
            return Err("Bundle preparation registered materials / slots".into());
        }
        click(&mut app, &mut time, "cancel")?;
        if app.materials.library != materials
            || path.exists()
            || app.workspaces.store.current.user_tools != tools
        {
            return Err("Canceled bundle partially registered materials / slots".into());
        }
        stage(&mut app, &mut time)?;
        click(&mut app, &mut time, "apply")?;
        if app.brush_presets.open
            || app.materials.library.assets().is_empty()
            || app.workspaces.store.current.user_tools.tools.is_empty()
        {
            return Err(format!("Tool bundle apply failed: {:?}", app.status));
        }
        drawer_core::tool_bundle::validate_group(
            &app.workspaces.store.current.user_tools,
            &app.materials.library,
        )?;
        let saved_tools = app.workspaces.store.current.user_tools.clone();
        let saved_mats = app.materials.library.clone();
        let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
        // A workspace write failure after material publication must roll back registration.
        stage(&mut app, &mut time)?;
        let block = user.join("settings/workspaces.json.write.tmp");
        std::fs::write(&block, b"foreign").map_err(|e| e.to_string())?;
        click(&mut app, &mut time, "apply")?;
        if !app.brush_presets.open
            || app.materials.library != saved_mats
            || app.workspaces.store.current.user_tools != saved_tools
            || std::fs::read(&path).map_err(|e| e.to_string())? != bytes
            || std::fs::read(&block).map_err(|e| e.to_string())? != b"foreign"
        {
            return Err(
                "Failed bundle apply left partial materials/settings or changed foreign temp"
                    .into(),
            );
        }
        std::fs::remove_file(block).map_err(|e| e.to_string())?;
        click(&mut app, &mut time, "cancel")?;
        let ctx2 = egui::Context::default();
        let restarted = Drawer::new(&ctx2, None)
            .with_storage(UserStorage::at(user.clone()).map_err(|e| e.to_string())?);
        if restarted.materials.library != saved_mats
            || restarted.workspaces.store.current.user_tools != saved_tools
        {
            return Err("Tool bundle did not survive restart".into());
        }
        let mut bad = Preset::capture(&app.capture_workspace(), BrushTip::Pen, "Missing tip")?;
        bad.sequence.enabled = true;
        bad.sequence.tips[0] = u32::MAX;
        if drawer_core::tool_bundle::validate_preset(&bad, &app.materials.library).is_ok() {
            return Err("Missing tip reference accepted".into());
        }
        if app.documents[0].snapshot() != doc {
            return Err("Preset exchange changed drawing history/pixels".into());
        }
        Ok(())
    })();
    result.and(std::fs::remove_dir_all(dir).map_err(|e| e.to_string()))
}
