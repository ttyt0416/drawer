use super::*;
pub(super) fn run(root: &std::path::Path) -> Result<(), String> {
    drawer_core::model_edit_verify::verify(root)?;
    let dir = root.join("model-edit-ui");
    std::fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let result = (|| {
        let check = |ok: bool, s: &str| if ok { Ok(()) } else { Err(s.to_owned()) };
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
                .ok_or_else(|| format!("Missing model control {key}"))?;
            check(
                r.center().y < 690.,
                &format!("Model control clipped: {key}"),
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
                    return Err("Model worker timeout".into());
                }
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            settle(app, t);
            Ok(())
        };
        app.execute_command(&ctx, "model-controls");
        settle(&mut app, &mut time);
        let before = app.documents[0].snapshot();
        app.model_controls.draft.expressions = [1., 0.5, 0.25];
        settle(&mut app, &mut time);
        click(&mut app, &mut time, "expressions")?;
        drain(&mut app, &mut time)?;
        check(
            app.model_controls.staged.is_some() && app.documents[0].snapshot() == before,
            &format!("Expression preview missing: {}", app.model_controls.notice),
        )?;
        check(
            app.begin_lighting().is_err() && app.begin_scene_cameras().is_err(),
            "Other editor replaced model preview",
        )?;
        click(&mut app, &mut time, "apply")?;
        check(
            app.documents[0].scene.world3d.rigs[&1].expressions == [1., 0.5, 0.25],
            "Mouse expression apply failed",
        )?;
        click(&mut app, &mut time, "lock-0")?;
        click(&mut app, &mut time, "limits")?;
        drain(&mut app, &mut time)?;
        click(&mut app, &mut time, "apply")?;
        check(
            app.documents[0].scene.world3d.rigs[&1].controls[0].locked[0],
            "Mouse joint lock failed",
        )?;
        let locked = app.documents[0].snapshot();
        app.model_controls.draft.angles[0] = 20.;
        settle(&mut app, &mut time);
        click(&mut app, &mut time, "joint")?;
        drain(&mut app, &mut time)?;
        check(
            !app.model_edit_pending() && app.documents[0].snapshot() == locked,
            "Locked joint changed / invalid preview retained",
        )?;
        click(&mut app, &mut time, "cancel")?;
        click(&mut app, &mut time, "lock-0")?;
        click(&mut app, &mut time, "limits")?;
        drain(&mut app, &mut time)?;
        click(&mut app, &mut time, "apply")?;
        click(&mut app, &mut time, "actor-2")?;
        app.model_controls.draft.parent = Some(1);
        app.model_controls.draft.bone = Some(1);
        settle(&mut app, &mut time);
        let before = app.documents[0].snapshot();
        click(&mut app, &mut time, "parent")?;
        drain(&mut app, &mut time)?;
        check(
            app.model_controls.staged.is_some() && app.documents[0].snapshot() == before,
            &format!("Bone attach preview missing: {}", app.model_controls.notice),
        )?;
        click(&mut app, &mut time, "apply")?;
        check(
            app.documents[0].scene.world3d.editing.attachments[&2].parent == 1,
            "Mouse bone attach failed",
        )?;
        app.model_controls.draft.name = "Stretch".into();
        settle(&mut app, &mut time);
        click(&mut app, &mut time, "add-key")?;
        drain(&mut app, &mut time)?;
        click(&mut app, &mut time, "apply")?;
        check(
            app.documents[0].scene.world3d.editing.morphs[&2].keys.len() == 1,
            "Shape target registration failed",
        )?;
        app.model_controls.draft.weight = 1.;
        settle(&mut app, &mut time);
        let base = app.documents[0].snapshot();
        click(&mut app, &mut time, "weight")?;
        drain(&mut app, &mut time)?;
        check(
            app.model_controls.staged.is_some() && app.documents[0].snapshot() == base,
            "Shape preview changed document",
        )?;
        click(&mut app, &mut time, "cancel")?;
        app.model_controls.draft.weight = 1.;
        settle(&mut app, &mut time);
        click(&mut app, &mut time, "weight")?;
        drain(&mut app, &mut time)?;
        click(&mut app, &mut time, "apply")?;
        check(
            app.documents[0].scene.world3d.editing.morphs[&2].keys[0].weight == 1.,
            "Mouse shape weight failed",
        )?;
        click(&mut app, &mut time, "undo")?;
        check(
            app.documents[0].snapshot() == base,
            "Shape weight not one Undo",
        )?;
        click(&mut app, &mut time, "actor-1")?;
        app.model_controls.draft.angles[2] = 20.;
        settle(&mut app, &mut time);
        let before = app.documents[0].snapshot();
        click(&mut app, &mut time, "joint")?;
        drain(&mut app, &mut time)?;
        click(&mut app, &mut time, "apply")?;
        check(
            app.documents[0].scene.worlds[1].vertices != before.scene.worlds[1].vertices,
            "Mouse base joint did not carry attached child",
        )?;
        click(&mut app, &mut time, "undo")?;
        check(
            app.documents[0].snapshot() == before,
            "Joint Undo did not restore child / hierarchy / pose",
        )?;
        click(&mut app, &mut time, "actor-2")?;
        click(&mut app, &mut time, "detach")?;
        drain(&mut app, &mut time)?;
        click(&mut app, &mut time, "apply")?;
        check(
            !app.documents[0]
                .scene
                .world3d
                .editing
                .attachments
                .contains_key(&2),
            "Mouse parent detach failed",
        )?;
        let frozen = app.documents[0].snapshot();
        app.stage_model_controls(
            &ctx,
            Edit::Parent {
                child: 2,
                parent: Some(1),
                joint: Some(1),
                keep: Keep::World,
            },
        )?;
        app.documents[0].current_frame = 1;
        drain(&mut app, &mut time)?;
        check(
            !app.model_edit_pending() && app.documents[0].snapshot() == frozen,
            "Frame change retained model edit",
        )?;
        app.stage_model_controls(
            &ctx,
            Edit::Parent {
                child: 2,
                parent: Some(1),
                joint: Some(1),
                keep: Keep::World,
            },
        )?;
        app.request_close(app.active_tab.ok_or("Missing model tab")?);
        if app.pending_close.is_some() {
            app.accept_close();
        }
        settle(&mut app, &mut time);
        check(
            !app.model_controls.open && !app.model_edit_pending(),
            "Closed document retained model job",
        )?;
        Ok(())
    })();
    result.and(std::fs::remove_dir_all(dir).map_err(|e| e.to_string()))
}
