use super::*;
pub(super) fn run(root: &std::path::Path) -> Result<(), String> {
    drawer_core::scene_render_verify::verify(root)?;
    let dir = root.join("scene-render-ui");
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
            for _ in 0..12 {
                let r = *app
                    .lighting
                    .controls
                    .get(key)
                    .ok_or_else(|| format!("Missing render control{key}"))?;
                let visible = *app
                    .lighting
                    .controls
                    .get("render-scroll")
                    .ok_or("Missing render scroll viewport")?;
                if ["preview", "apply", "discard"].contains(&key)
                    || visible.shrink(5.).contains(r.center())
                {
                    break;
                }
                let target = r
                    .center()
                    .y
                    .clamp(visible.top() + 10., visible.bottom() - 10.);
                let delta = (target - r.center().y).clamp(-240., 240.);
                frame(
                    app,
                    t,
                    vec![
                        egui::Event::PointerMoved(egui::pos2(
                            visible.left() + 4.,
                            visible.center().y,
                        )),
                        egui::Event::MouseWheel {
                            unit: egui::MouseWheelUnit::Point,
                            phase: egui::TouchPhase::Move,
                            delta: egui::vec2(0., delta),
                            modifiers: Default::default(),
                        },
                    ],
                );
                settle(app, t);
            }
            let r = *app
                .lighting
                .controls
                .get(key)
                .ok_or("Missing lighting control after scroll")?;
            let visible = *app
                .lighting
                .controls
                .get("render-scroll")
                .ok_or("Missing render scroll viewport")?;
            check(
                ["preview", "apply", "discard"].contains(&key) || visible.contains(r.center()),
                &format!("Render control clipped: {key} button{r:?} viewport{visible:?}"),
            )?;
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
            while app.lighting.job.is_some() {
                frame(app, t, vec![]);
                if std::time::Instant::now() > end {
                    return Err("Render job timeout".into());
                }
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            settle(app, t);
            Ok(())
        };
        app.execute_command(&ctx, "scene-lighting");
        settle(&mut app, &mut time);
        let original = app.documents[0].snapshot();
        click(&mut app, &mut time, "render-expand")?;
        check(
            app.lighting.settings.render.is_some(),
            "Render options did not expand",
        )?;
        click(&mut app, &mut time, "render-fog")?;
        check(
            app.lighting.settings.render.as_ref().unwrap().fog.is_some(),
            "Fog pointer toggle failed",
        )?;
        click(&mut app, &mut time, "render-object-1")?;
        click(&mut app, &mut time, "render-receive")?;
        check(
            !app.lighting.settings.render.as_ref().unwrap().objects[&1].receive,
            "Receive-shadow pointer toggle failed",
        )?;
        click(&mut app, &mut time, "preview")?;
        drain(&mut app, &mut time)?;
        check(
            app.lighting.staged.is_some() && app.documents[0].snapshot() == original,
            &format!("Advanced lighting preview failed:{}", app.lighting.notice),
        )?;
        check(
            app.begin_model_controls().is_err(),
            "Model editor replaced advanced lighting job",
        )?;
        click(&mut app, &mut time, "apply")?;
        let applied = app.documents[0].snapshot();
        check(
            applied != original
                && app.documents[0]
                    .scene
                    .world3d
                    .lighting
                    .as_ref()
                    .unwrap()
                    .render
                    .as_ref()
                    .unwrap()
                    .fog
                    .is_some(),
            "Render apply missing",
        )?;
        app.documents[0].undo();
        settle(&mut app, &mut time);
        check(
            app.documents[0].snapshot() == original,
            "Render Undo incomplete",
        )?;
        app.documents[0].redo();
        settle(&mut app, &mut time);
        check(
            app.documents[0].snapshot() == applied,
            "Render Redo incomplete",
        )?;
        app.lighting
            .settings
            .render
            .as_mut()
            .unwrap()
            .fog
            .as_mut()
            .unwrap()
            .end = 2.;
        app.lighting
            .settings
            .render
            .as_mut()
            .unwrap()
            .fog
            .as_mut()
            .unwrap()
            .start = 2.;
        settle(&mut app, &mut time);
        click(&mut app, &mut time, "preview")?;
        drain(&mut app, &mut time)?;
        check(
            !app.lighting_pending() && app.documents[0].snapshot() == applied,
            "Invalid fog was staged/applied",
        )?;
        click(&mut app, &mut time, "discard")?;
        app.stage_lighting(&ctx)?;
        app.documents[0].current_frame += 1;
        settle(&mut app, &mut time);
        check(
            !app.lighting_pending(),
            "Frame change retained lighting job",
        )?;
        app.stage_lighting(&ctx)?;
        app.active = None;
        settle(&mut app, &mut time);
        check(
            !app.lighting_pending() && !app.lighting.open,
            "Document close did not join lighting worker",
        )?;
        Ok(())
    })();
    result.and(std::fs::remove_dir_all(dir).map_err(|e| e.to_string()))
}
