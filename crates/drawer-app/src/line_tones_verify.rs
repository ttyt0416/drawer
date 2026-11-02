use super::*;
pub(super) fn run(root: &std::path::Path) -> Result<(), String> {
    drawer_core::line_tones_verify::verify(root)?;
    let dir = root.join("line-tones-ui");
    std::fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let result = (|| {
        let check = |b: bool, s: &str| if b { Ok(()) } else { Err(s.to_owned()) };
        let ctx = egui::Context::default();
        let mut app = Drawer::new(&ctx, None)
            .with_storage(UserStorage::at(dir.join("data")).map_err(|e| e.to_string())?);
        app.simple_view = true;
        app.add_document(drawer_core::line_tones_verify::image(true)?);
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
                .line_tones
                .controls
                .get(key)
                .ok_or("Missing extraction control")?;
            check(r.center().y < 690., "Extraction control clipped")?;
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
            let until = std::time::Instant::now() + std::time::Duration::from_secs(30);
            while app.line_tones.job.is_some() {
                frame(app, t, vec![]);
                if std::time::Instant::now() > until {
                    return Err("Extraction worker timeout".into());
                }
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            settle(app, t);
            Ok(())
        };
        app.execute_command(&ctx, "line-tones");
        settle(&mut app, &mut time);
        check(app.line_tones.open, "Extraction command missing")?;
        let before = app.documents[0].snapshot();
        click(&mut app, &mut time, "preview")?;
        drain(&mut app, &mut time)?;
        check(
            app.line_tones.staged.is_some() && app.documents[0].snapshot() == before,
            &format!("Extraction preview failed: {}", app.line_tones.notice),
        )?;
        check(
            app.begin_uv_paint().is_err() && app.begin_world3d().is_err(),
            "Another3D edit replaced extraction",
        )?;
        click(&mut app, &mut time, "apply")?;
        check(
            app.line_tones.staged.is_some() && app.documents[0].snapshot() == before,
            "Dense preview applied without review",
        )?;
        click(&mut app, &mut time, "review")?;
        click(&mut app, &mut time, "apply")?;
        let created = app.documents[0].snapshot();
        check(
            app.documents[0].layers.len() == 3 && app.documents[0].layers[0] == before.layers[0],
            "New layers / original preservation failed",
        )?;
        app.documents[0].undo();
        check(
            app.documents[0].snapshot() == before,
            "Extraction not one Undo",
        )?;
        app.documents[0].redo();
        check(
            app.documents[0].snapshot() == created,
            "Extraction Redo failed",
        )?;
        settle(&mut app, &mut time);
        click(&mut app, &mut time, "preview")?;
        drain(&mut app, &mut time)?;
        click(&mut app, &mut time, "cancel")?;
        check(
            !app.line_tones_pending() && app.documents[0].snapshot() == created,
            "Cancel changed drawing",
        )?;
        app.line_tones.settings.strength = 0.;
        settle(&mut app, &mut time);
        let history = app.documents[0].history_counts();
        click(&mut app, &mut time, "preview")?;
        drain(&mut app, &mut time)?;
        click(&mut app, &mut time, "apply")?;
        check(
            app.documents[0].history_counts() == history,
            "Zero strength added history",
        )?;
        app.line_tones.settings.strength = 1.;
        settle(&mut app, &mut time);
        app.stage_line_tones(&ctx)?;
        app.documents[0].edit_pixel(0, 0, 0, [1, 2, 3, 255]);
        drain(&mut app, &mut time)?;
        check(
            !app.line_tones_pending(),
            "Changed source retained extraction",
        )?;
        app.stage_line_tones(&ctx)?;
        app.line_tones
            .job
            .as_ref()
            .unwrap()
            .cancel
            .store(true, Ordering::Relaxed);
        drain(&mut app, &mut time)?;
        check(
            !app.line_tones_pending(),
            "Cancelled worker retained result",
        )?;
        app.stage_line_tones(&ctx)?;
        app.request_close(app.active_tab.ok_or("Extraction tab missing")?);
        if app.pending_close.is_some() {
            app.accept_close();
        }
        settle(&mut app, &mut time);
        check(
            !app.line_tones_pending() && !app.line_tones.open,
            "Closed document retained extraction",
        )?;
        Ok(())
    })();
    result.and(std::fs::remove_dir_all(dir).map_err(|e| e.to_string()))
}
