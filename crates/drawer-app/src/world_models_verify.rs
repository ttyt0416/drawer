use super::*;
pub(super) fn run(root: &std::path::Path) -> Result<(), String> {
    drawer_core::model_library_verify::verify(root)?;
    let dir = root.join("models-ui-verify");
    std::fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let result = (|| {
        let check = |b: bool, s: &str| if b { Ok(()) } else { Err(s.to_owned()) };
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
        let click = |app: &mut Drawer, t: &mut f64, key: &str, world: bool| -> Result<(), String> {
            let r = if world {
                *app.world3d
                    .controls
                    .get(key)
                    .ok_or_else(|| format!("Missing3D control{key}"))?
            } else {
                *app.world_models
                    .controls
                    .get(key)
                    .ok_or_else(|| format!("Missing model control{key}"))?
            };
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
            for _ in 0..3 {
                frame(app, t, vec![]);
            }
            Ok(())
        };
        let drain = |app: &mut Drawer, t: &mut f64| -> Result<(), String> {
            let end = std::time::Instant::now() + std::time::Duration::from_secs(30);
            while app.models_busy() {
                frame(app, t, vec![]);
                if std::time::Instant::now() > end {
                    return Err("Model worker timeout".into());
                }
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            for _ in 0..3 {
                frame(app, t, vec![]);
            }
            Ok(())
        };
        app.execute_command(&ctx, "model-library");
        for _ in 0..3 {
            frame(&mut app, &mut time, vec![]);
        }
        check(app.world_models.open, "Model library command did not open")?;
        let original = app.documents[0].snapshot();
        click(&mut app, &mut time, "primitive", false)?;
        drain(&mut app, &mut time)?;
        check(
            app.has_world_preview() && app.documents[0].snapshot() == original,
            &format!(
                "Primitive pointer preview failed: {}",
                app.world_models.notice
            ),
        )?;
        click(&mut app, &mut time, "apply", true)?;
        check(
            app.documents[0].scene.worlds.len() == 1,
            "Primitive adoption failed",
        )?;
        let path = drawer_core::model_library_verify::write_fixture(&dir.join("input"))?;
        let source = std::fs::read(&path).map_err(|e| e.to_string())?;
        app.execute_command(&ctx, "model-library");
        for _ in 0..3 {
            frame(&mut app, &mut time, vec![]);
        }
        app.world_models.path = path.display().to_string();
        let before = app.documents[0].snapshot();
        click(&mut app, &mut time, "import", false)?;
        drain(&mut app, &mut time)?;
        check(
            app.has_world_preview()
                && app.world3d.controls.contains_key("model-review")
                && app.documents[0].snapshot() == before,
            &format!("OBJ review missing: {}", app.world_models.notice),
        )?;
        click(&mut app, &mut time, "apply", true)?;
        check(
            app.documents[0].snapshot() == before && app.has_world_preview(),
            "Import warnings were bypassed",
        )?;
        click(&mut app, &mut time, "model-review", true)?;
        let history = app.documents[0].history_counts();
        click(&mut app, &mut time, "apply", true)?;
        check(
            app.documents[0].scene.worlds.len() == 3
                && app.documents[0].scene.world3d.surfaces.len() == 3
                && app.documents[0].history_counts().0 == history.0 + 1,
            "Reviewed import was not one complete Undo",
        )?;
        app.documents[0].undo();
        check(
            app.documents[0].snapshot() == before,
            "Imported asset Undo incomplete",
        )?;
        app.documents[0].redo();
        app.execute_command(&ctx, "model-library");
        for _ in 0..3 {
            frame(&mut app, &mut time, vec![]);
        }
        app.world_models.ids = vec![2];
        let before = app.documents[0].snapshot();
        click(&mut app, &mut time, "duplicate", false)?;
        drain(&mut app, &mut time)?;
        check(
            app.has_world_preview() && app.documents[0].snapshot() == before,
            &format!(
                "Duplicate preview failed: {}; preview {}; busy {}; open {}; source changed {}; button {:?}",
                app.world_models.notice,
                app.has_world_preview(),
                app.models_busy(),
                app.world_models.open,
                app.documents[0].snapshot() != before,
                app.world_models.controls.get("duplicate")
            ),
        )?;
        click(&mut app, &mut time, "apply", true)?;
        check(
            app.documents[0].scene.worlds.len() == 4
                && app.documents[0].scene.world3d.surfaces[&4]
                    == app.documents[0].scene.world3d.surfaces[&2],
            "Duplicate lost topology / attributes",
        )?;
        app.execute_command(&ctx, "model-library");
        for _ in 0..3 {
            frame(&mut app, &mut time, vec![]);
        }
        app.world_models.ids = vec![2, 4];
        click(&mut app, &mut time, "align", false)?;
        drain(&mut app, &mut time)?;
        check(
            app.has_world_preview(),
            &format!("Alignment preview failed: {}", app.world_models.notice),
        )?;
        click(&mut app, &mut time, "apply", true)?;
        let a = drawer_core::world3d::center(&app.documents[0].scene.worlds[1]);
        let b = drawer_core::world3d::center(&app.documents[0].scene.worlds[3]);
        check(
            (a[0] - b[0]).abs() < 1e-8,
            "Pointer alignment missed target",
        )?;
        let task = || Edit::Primitive {
            shape: Shape::Room,
            detail: 12,
            placement: Default::default(),
        };
        let before = app.documents[0].snapshot();
        app.stage_model(&ctx, task())?;
        app.world_models
            .job
            .as_ref()
            .unwrap()
            .cancel
            .store(true, Ordering::Relaxed);
        drain(&mut app, &mut time)?;
        check(
            !app.has_world_preview() && app.documents[0].snapshot() == before,
            "Cancelled model preparation changed source",
        )?;
        app.stage_model(&ctx, task())?;
        app.documents[0].edit_pixel(0, 0, 0, [1, 2, 3, 255]);
        let changed = app.documents[0].snapshot();
        drain(&mut app, &mut time)?;
        check(
            !app.has_world_preview() && app.documents[0].snapshot() == changed,
            "Stale model preparation replaced drawing",
        )?;
        app.stage_model(&ctx, task())?;
        app.request_close(app.active_tab.ok_or("Model document tab missing")?);
        if app.pending_close.is_some() {
            app.accept_close();
        }
        frame(&mut app, &mut time, vec![]);
        check(
            app.documents.is_empty() && !app.models_busy() && !app.has_world_preview(),
            "Closed model document retained worker / snapshot",
        )?;
        check(
            std::fs::read(&path).map_err(|e| e.to_string())? == source,
            "UI import edited source OBJ",
        )?;
        Ok(())
    })();
    let clean = std::fs::remove_dir_all(&dir).map_err(|e| e.to_string());
    result.and(clean)
}
