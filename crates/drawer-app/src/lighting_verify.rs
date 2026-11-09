use super::*;
pub(super) fn run(root: &std::path::Path) -> Result<(), String> {
    drawer_core::lighting_verify::verify(root)?;
    let dir = root.join("lighting-ui");
    std::fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let result = (|| {
        let check = |b: bool, s: &str| if b { Ok(()) } else { Err(s.to_owned()) };
        let ctx = egui::Context::default();
        let mut app = Drawer::new(&ctx, None)
            .with_storage(UserStorage::at(dir.join("data")).map_err(|e| e.to_string())?);
        app.simple_view = true;
        app.add_document(drawer_core::lighting_verify::fixture()?);
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
                .lighting
                .controls
                .get(key)
                .ok_or("Missing lighting control")?;
            check(r.center().y < 690., "Lighting control clipped")?;
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
            let end = std::time::Instant::now() + std::time::Duration::from_secs(30);
            while app.lighting.job.is_some() {
                frame(app, t, vec![]);
                if std::time::Instant::now() > end {
                    return Err("Lighting preparation timed out".into());
                }
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            settle(app, t);
            Ok(())
        };
        app.execute_command(&ctx, "scene-lighting");
        settle(&mut app, &mut time);
        check(app.lighting.open, "Lighting command failed")?;
        let before = app.documents[0].snapshot();
        app.lighting.settings.style = Style::Flat;
        app.lighting.settings.ambient = 0.;
        click(&mut app, &mut time, "clear")?;
        click(&mut app, &mut time, "guides")?;
        click(&mut app, &mut time, "preview")?;
        check(
            app.lighting_pending()
                && app.lighting.image.is_some()
                && app.documents[0].snapshot() == before,
            &format!("Lighting preview failed: {}", app.lighting.notice),
        )?;
        check(
            app.begin_world3d().is_err()
                && app.begin_human_rig().is_err()
                && app.begin_model_library().is_err()
                && app.begin_model_presets().is_err(),
            "Lighting draft overwritten",
        )?;
        click(&mut app, &mut time, "apply")?;
        let saved = app.documents[0].snapshot();
        check(
            drawer_core::render::Renderer::new(&app.documents[0]).pixel([32., 32.])
                == [0, 0, 0, 255]
                && app.canvas_view.world_guide_lines > 0,
            "Dark render / guide overlay missing",
        )?;
        app.documents[0].undo();
        check(
            app.documents[0].snapshot() == before,
            "Lighting Apply not one Undo",
        )?;
        app.documents[0].redo();
        check(app.documents[0].snapshot() == saved, "Lighting Redo failed")?;
        settle(&mut app, &mut time);
        click(&mut app, &mut time, "add")?;
        click(&mut app, &mut time, "preview")?;
        check(app.lighting_pending(), "Added light did not stage")?;
        click(&mut app, &mut time, "discard")?;
        check(
            !app.lighting_pending() && app.documents[0].snapshot() == saved,
            "Discard changed source",
        )?;
        click(&mut app, &mut time, "preview")?;
        app.documents[0].edit_pixel(0, 0, 0, [1, 2, 3, 255]);
        settle(&mut app, &mut time);
        check(!app.lighting_pending(), "Stale lighting retained")?;
        click(&mut app, &mut time, "preview")?;
        app.lighting.open = false;
        settle(&mut app, &mut time);
        check(!app.lighting_pending(), "Closed lighting retained preview")?;
        app.begin_lighting()?;
        settle(&mut app, &mut time);
        click(&mut app, &mut time, "preview")?;
        app.request_close(app.active_tab.ok_or("Lighting document tab missing")?);
        if app.pending_close.is_some() {
            app.accept_close();
        }
        settle(&mut app, &mut time);
        check(
            !app.lighting_pending() && !app.lighting.open,
            "Closed document retained lighting",
        )?;
        Ok(())
    })();
    result.and(std::fs::remove_dir_all(dir).map_err(|e| e.to_string()))
}
