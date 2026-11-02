use super::*;
pub(super) fn run(root: &std::path::Path) -> Result<(), String> {
    drawer_core::environment_verify::verify(root)?;
    let dir = root.join("environment-ui");
    std::fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let result = (|| {
        let path = drawer_core::environment_verify::fixture(&dir.join("source"))?;
        let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
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
        let click = |app: &mut Drawer, t: &mut f64, key: &str| -> Result<(), String> {
            let r = *app
                .environment
                .controls
                .get(key)
                .ok_or_else(|| format!("Missing panorama control {key}"))?;
            check(r.center().y < 690., "Panorama control clipped")?;
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
            while app.environment.job.is_some() {
                frame(app, t, vec![]);
                if std::time::Instant::now() > end {
                    return Err("Panorama worker timeout".into());
                }
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            settle(app, t);
            Ok(())
        };
        app.execute_command(&ctx, "scene-environment");
        settle(&mut app, &mut time);
        app.environment.path = path.display().to_string();
        settle(&mut app, &mut time);
        let before = app.documents[0].snapshot();
        click(&mut app, &mut time, "file")?;
        drain(&mut app, &mut time)?;
        check(
            app.environment.staged.is_some() && app.documents[0].snapshot() == before,
            &format!("Panorama preview missing: {}", app.environment.notice),
        )?;
        check(
            app.begin_lighting().is_err() && app.begin_world3d().is_err(),
            "Another3D editor replaced environment preview",
        )?;
        click(&mut app, &mut time, "apply")?;
        check(
            app.environment.staged.is_some() && app.documents[0].snapshot() == before,
            "Unreviewed panorama applied",
        )?;
        click(&mut app, &mut time, "review")?;
        click(&mut app, &mut time, "apply")?;
        let embedded = app.documents[0].snapshot();
        check(
            app.documents[0].scene.world3d.environment.is_some(),
            "Panorama not embedded",
        )?;
        app.documents[0].undo();
        check(
            app.documents[0].snapshot() == before,
            "Panorama not one Undo",
        )?;
        app.documents[0].redo();
        check(
            app.documents[0].snapshot() == embedded,
            "Panorama Redo differs",
        )?;
        settle(&mut app, &mut time);
        check(
            std::fs::read(&path).map_err(|e| e.to_string())? == bytes,
            "Source panorama changed",
        )?;
        std::fs::remove_file(&path).map_err(|e| e.to_string())?;
        app.environment.settings.yaw = 180.;
        click(&mut app, &mut time, "values")?;
        drain(&mut app, &mut time)?;
        check(
            app.environment.staged.is_some(),
            "Embedded settings require source file",
        )?;
        click(&mut app, &mut time, "cancel")?;
        check(
            app.documents[0].snapshot() == embedded && !app.environment_pending(),
            "Panorama cancel changed scene",
        )?;
        click(&mut app, &mut time, "values")?;
        drain(&mut app, &mut time)?;
        click(&mut app, &mut time, "review")?;
        click(&mut app, &mut time, "apply")?;
        check(
            app.documents[0]
                .scene
                .world3d
                .environment
                .as_ref()
                .unwrap()
                .settings
                .yaw
                == 180.,
            "Embedded yaw not applied",
        )?;
        click(&mut app, &mut time, "remove")?;
        drain(&mut app, &mut time)?;
        click(&mut app, &mut time, "apply")?;
        check(
            app.documents[0].scene.world3d.environment.is_none(),
            "Panorama removal failed",
        )?;
        app.documents[0].undo();
        settle(&mut app, &mut time);
        check(
            app.documents[0].scene.world3d.environment.is_some(),
            "Removal Undo lost image",
        )?;
        click(&mut app, &mut time, "file")?;
        drain(&mut app, &mut time)?;
        check(
            !app.environment_pending(),
            "Missing source retained invalid panorama",
        )?;
        app.stage_environment(&ctx, Edit::Settings(app.environment.settings))?;
        app.documents[0].edit_pixel(0, 0, 0, [1, 2, 3, 255]);
        drain(&mut app, &mut time)?;
        check(!app.environment_pending(), "Stale panorama retained")?;
        app.stage_environment(&ctx, Edit::Settings(app.environment.settings))?;
        app.environment
            .job
            .as_ref()
            .unwrap()
            .cancel
            .store(true, Ordering::Relaxed);
        drain(&mut app, &mut time)?;
        check(
            !app.environment_pending(),
            "Cancelled panorama result retained",
        )?;
        app.stage_environment(&ctx, Edit::Settings(app.environment.settings))?;
        app.request_close(app.active_tab.ok_or("Missing panorama tab")?);
        if app.pending_close.is_some() {
            app.accept_close();
        }
        settle(&mut app, &mut time);
        check(
            !app.environment.open && !app.environment_pending(),
            "Closed document retained panorama job",
        )?;
        Ok(())
    })();
    result.and(std::fs::remove_dir_all(dir).map_err(|e| e.to_string()))
}
