use super::*;
pub(super) fn run(root: &std::path::Path) -> Result<(), String> {
    drawer_core::mannequin_fit_verify::verify(root)?;
    let dir = root.join("mannequin-fit-ui");
    std::fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let result = (|| {
        let check = |b: bool, s: &str| if b { Ok(()) } else { Err(s.to_owned()) };
        let ctx = egui::Context::default();
        let mut app = Drawer::new(&ctx, None)
            .with_storage(UserStorage::at(dir.join("data")).map_err(|e| e.to_string())?);
        app.simple_view = true;
        app.add_document(drawer_core::model_edit_verify::fixture(
            drawer_core::human_rig::Kind::Head,
        )?);
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
                .model_controls
                .controls
                .get(key)
                .ok_or_else(|| format!("Missing fit button{key}"))?;
            check(
                r.center().y < 690.,
                &format!("Fit button clipped{key}:{}", r.center().y),
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
            while app.model_controls.job.is_some() {
                frame(app, t, vec![]);
                if std::time::Instant::now() > end {
                    return Err("Fit worker timeout".into());
                }
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            settle(app, t);
            Ok(())
        };
        app.execute_command(&ctx, "model-controls");
        settle(&mut app, &mut time);
        click(&mut app, &mut time, "fit-expand")?;
        app.model_controls.draft.form.strength = 1.5;
        app.model_controls.draft.form.parts[1] = [1.5, 0.7, 1.2];
        settle(&mut app, &mut time);
        let before = app.documents[0].snapshot();
        click(&mut app, &mut time, "fit-form")?;
        drain(&mut app, &mut time)?;
        check(
            app.model_controls.staged.is_some() && app.documents[0].snapshot() == before,
            &format!("Fit preview absent:{}", app.model_controls.notice),
        )?;
        click(&mut app, &mut time, "cancel")?;
        check(
            app.documents[0].snapshot() == before && !app.model_edit_pending(),
            "Fit cancel changed original",
        )?;
        app.model_controls.draft.form.strength = 1.;
        settle(&mut app, &mut time);
        click(&mut app, &mut time, "fit-form")?;
        drain(&mut app, &mut time)?;
        click(&mut app, &mut time, "apply")?;
        check(
            app.documents[0].scene.world3d.rigs[&1].form.strength == 1.,
            "Fit pointer apply missing",
        )?;
        click(&mut app, &mut time, "undo")?;
        check(
            app.documents[0].snapshot() == before,
            "Fit pointer Undo differs",
        )?;
        app.model_controls.draft.target.face = 1.8;
        app.model_controls.draft.blend = 1.;
        settle(&mut app, &mut time);
        click(&mut app, &mut time, "fit-blend")?;
        drain(&mut app, &mut time)?;
        click(&mut app, &mut time, "apply")?;
        check(
            app.documents[0].scene.world3d.rigs[&1].shape.face == 1.8,
            "Head blend pointer apply missing",
        )?;
        app.model_controls.draft.form.strength = 0.8;
        settle(&mut app, &mut time);
        click(&mut app, &mut time, "fit-form")?;
        app.documents[0].current_frame += 1;
        settle(&mut app, &mut time);
        check(!app.model_edit_pending(), "Fit stale frame survived")?;
        app.stage_model_controls(
            &ctx,
            Edit::Fit {
                rig: 1,
                change: drawer_core::mannequin_fit::Change::Form(Default::default()),
                base: false,
            },
        )?;
        app.active = None;
        settle(&mut app, &mut time);
        check(
            !app.model_edit_pending() && !app.model_controls.open,
            "Fit document close did not join job",
        )?;
        Ok(())
    })();
    result.and(std::fs::remove_dir_all(dir).map_err(|e| e.to_string()))
}
