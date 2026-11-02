use super::*;
pub(super) fn run(root: &std::path::Path) -> Result<(), String> {
    drawer_core::model_import_verify::verify(root)?;
    let dir = root.join("model-import-ui");
    std::fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let result = (|| {
        drawer_core::model_import_verify::fixture(&dir.join("source"), true)?;
        let source = dir.join("source/model.gltf");
        let original_file = std::fs::read(&source).map_err(|e| e.to_string())?;
        let check = |ok: bool, s: &str| if ok { Ok(()) } else { Err(s.to_owned()) };
        let ctx = egui::Context::default();
        let mut app = Drawer::new(&ctx, None)
            .with_storage(UserStorage::at(dir.join("data")).map_err(|e| e.to_string())?);
        app.simple_view = true;
        app.add_document(drawer_core::model_library_verify::blank()?);
        let mut time = 0.;
        let frame = |app: &mut Drawer, t: &mut f64, events| {
            *t += 0.05;
            input_replay::frame(&ctx, app, *t, events);
        };
        let settle = |app: &mut Drawer, t: &mut f64| {
            for _ in 0..3 {
                frame(app, t, vec![]);
            }
        };
        let click = |app: &mut Drawer, t: &mut f64, key: &str, world: bool| -> Result<(), String> {
            let r = if world {
                *app.world3d
                    .controls
                    .get(key)
                    .ok_or_else(|| format!("Missing world import control {key}"))?
            } else {
                *app.world_models
                    .controls
                    .get(key)
                    .ok_or_else(|| format!("Missing import control {key}"))?
            };
            check(r.center().y < 690., "Import control clipped")?;
            for pressed in [true, false] {
                frame(
                    app,
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
            settle(app, t);
            Ok(())
        };
        let drain = |app: &mut Drawer, t: &mut f64| -> Result<(), String> {
            let end = std::time::Instant::now() + std::time::Duration::from_secs(30);
            while app.models_busy() {
                frame(app, t, vec![]);
                if std::time::Instant::now() > end {
                    return Err("Import worker timeout".into());
                }
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            settle(app, t);
            Ok(())
        };
        app.execute_command(&ctx, "model-library");
        settle(&mut app, &mut time);
        app.world_models.path = source.display().to_string();
        app.world_models.fit = false;
        let before = app.documents[0].snapshot();
        click(&mut app, &mut time, "import", false)?;
        drain(&mut app, &mut time)?;
        check(
            app.has_world_preview() && app.documents[0].snapshot() == before,
            &format!("glTF preview missing: {}", app.world_models.notice),
        )?;
        click(&mut app, &mut time, "apply", true)?;
        check(
            app.documents[0].snapshot() == before && app.has_world_preview(),
            "Loss notices bypassed",
        )?;
        click(&mut app, &mut time, "model-review", true)?;
        click(&mut app, &mut time, "apply", true)?;
        let imported = app.documents[0].snapshot();
        check(
            app.documents[0].scene.world3d.surfaces[&1]
                .imported
                .as_ref()
                .and_then(|a| a.skin.as_ref())
                .is_some_and(|r| r.names.len() == 2),
            "Imported rig data missing",
        )?;
        app.documents[0].undo();
        check(
            app.documents[0].snapshot() == before,
            "Import Undo incomplete",
        )?;
        app.documents[0].redo();
        check(
            app.documents[0].snapshot() == imported,
            "Import Redo incomplete",
        )?;
        app.execute_command(&ctx, "model-library");
        settle(&mut app, &mut time);
        click(&mut app, &mut time, "import", false)?;
        drain(&mut app, &mut time)?;
        click(&mut app, &mut time, "discard", true)?;
        check(
            app.documents[0].snapshot() == imported && !app.has_world_preview(),
            "Import cancel changed scene",
        )?;
        app.execute_command(&ctx, "model-library");
        settle(&mut app, &mut time);
        app.stage_model(
            &ctx,
            Edit::Import {
                path: source.clone(),
                placement: Default::default(),
            },
        )?;
        app.documents[0].edit_pixel(0, 0, 0, [1, 2, 3, 255]);
        drain(&mut app, &mut time)?;
        check(!app.has_world_preview(), "Stale import adopted")?;
        app.stage_model(
            &ctx,
            Edit::Import {
                path: source.clone(),
                placement: Default::default(),
            },
        )?;
        app.request_close(app.active_tab.ok_or("Missing import tab")?);
        if app.pending_close.is_some() {
            app.accept_close();
        }
        settle(&mut app, &mut time);
        check(
            !app.models_busy() && !app.has_world_preview(),
            "Closed document retained model worker",
        )?;
        check(
            std::fs::read(source).map_err(|e| e.to_string())? == original_file,
            "UI import modified source",
        )?;
        Ok(())
    })();
    result.and(std::fs::remove_dir_all(dir).map_err(|e| e.to_string()))
}
