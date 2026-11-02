use super::*;
pub(super) fn run(root: &std::path::Path) -> Result<(), String> {
    drawer_core::human_rig_verify::verify(root)?;
    let dir = root.join("mannequin-ui");
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
        let settle = |app: &mut Drawer, t: &mut f64| {
            for _ in 0..3 {
                frame(app, t, vec![]);
            }
        };
        let click = |app: &mut Drawer, t: &mut f64, key: &str| -> Result<(), String> {
            let r = *app
                .human_rig
                .controls
                .get(key)
                .ok_or_else(|| format!("Missing mannequin control {key}"))?;
            check(r.center().y < 690., "Control outside window")?;
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
        app.execute_command(&ctx, "human-rig");
        settle(&mut app, &mut time);
        check(app.human_rig.open, "Mannequin command missing")?;
        let before = app.documents[0].snapshot();
        click(&mut app, &mut time, "body")?;
        check(
            app.rig_pending() && app.documents[0].snapshot() == before,
            &format!("Body preview failed: {}", app.human_rig.notice),
        )?;
        check(
            app.begin_world3d().is_err() && app.begin_model_library().is_err(),
            "Other3D edit overwrote rig draft",
        )?;
        let first = app
            .human_rig
            .staged
            .as_ref()
            .unwrap()
            .scene()
            .worlds
            .clone();
        let r = app.human_rig.controls["shape-Height"];
        let pos = egui::pos2(r.left() + 20., r.center().y);
        for pressed in [true, false] {
            frame(
                &mut app,
                &mut time,
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: Default::default(),
                    },
                ],
            );
        }
        settle(&mut app, &mut time);
        check(
            app.human_rig.staged.as_ref().unwrap().scene().worlds != first
                && app.documents[0].snapshot() == before,
            &format!(
                "Shape slider did not change new preview geometry: rect={r:?}, shape={:?}",
                app.human_rig.draft.as_ref().map(|r| r.shape)
            ),
        )?;
        click(&mut app, &mut time, "apply")?;
        check(
            app.documents[0].scene.world3d.rigs.len() == 1,
            "Body adoption failed",
        )?;
        let committed = app.documents[0].snapshot();
        app.documents[0].undo();
        check(
            app.documents[0].snapshot() == before,
            "Creation Undo failed",
        )?;
        app.documents[0].redo();
        check(
            app.documents[0].snapshot() == committed,
            "Creation Redo failed",
        )?;
        settle(&mut app, &mut time);
        // Change joint via real slider pointer input, preserving source until Apply.
        let r = app.human_rig.controls["angle-0"];
        let pos = egui::pos2(r.left() + 20., r.center().y);
        for pressed in [true, false] {
            frame(
                &mut app,
                &mut time,
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: Default::default(),
                    },
                ],
            );
        }
        settle(&mut app, &mut time);
        check(
            app.rig_pending() && app.documents[0].snapshot() == committed,
            "Pointer pose did not stage",
        )?;
        click(&mut app, &mut time, "mirror")?;
        let draft = app.human_rig.draft.clone();
        click(&mut app, &mut time, "mirror")?;
        click(&mut app, &mut time, "mirror")?;
        check(app.human_rig.draft == draft, "UI mirror twice differs")?;
        click(&mut app, &mut time, "discard")?;
        check(
            !app.rig_pending() && app.documents[0].snapshot() == committed,
            "Discard changed source",
        )?;
        app.human_rig.draft.as_mut().unwrap().shape.shoulders = 1.4;
        app.human_rig.joint = 10;
        app.human_rig.draft.as_mut().unwrap().angles[10][0] = 50.;
        let mut panel = std::mem::take(&mut app.human_rig);
        panel.prepare(&app.documents[0], DEFAULT_DOCUMENT_BUDGET)?;
        app.human_rig = panel;
        settle(&mut app, &mut time);
        click(&mut app, &mut time, "apply")?;
        check(
            app.documents[0].scene.world3d.rigs[&1].angles[10][0] == 50.
                && app.documents[0].scene.world3d.rigs[&1].shape.shoulders == 1.4,
            "Multi-joint shape draft lost",
        )?;
        let posed = app.documents[0].snapshot();
        click(&mut app, &mut time, "detach")?;
        check(
            app.rig_pending() && app.documents[0].snapshot() == posed,
            "Detach was not previewed",
        )?;
        click(&mut app, &mut time, "apply")?;
        check(
            !app.human_rig.controls.contains_key("angle-0")
                && app.human_rig.notice.contains("Applied"),
            "Rigless model exposes joint editing",
        )?;
        let rigless = app.documents[0].snapshot();
        click(&mut app, &mut time, "hand")?;
        check(app.rig_pending(), "Hand preview failed")?;
        app.documents[0].edit_pixel(0, 0, 0, [1, 2, 3, 255]);
        settle(&mut app, &mut time);
        check(!app.rig_pending(), "Changed source retained preview")?;
        click(&mut app, &mut time, "head")?;
        app.human_rig.open = false;
        settle(&mut app, &mut time);
        check(!app.rig_pending(), "Closing editor retained draft")?;
        app.begin_human_rig()?;
        settle(&mut app, &mut time);
        click(&mut app, &mut time, "hand")?;
        click(&mut app, &mut time, "apply")?;
        check(
            app.documents[0]
                .scene
                .world3d
                .rigs
                .values()
                .any(|r| r.kind == Kind::Hand),
            "Hand Apply failed",
        )?;
        check(
            app.documents[0].snapshot() != rigless,
            "No document changes",
        )?;
        click(&mut app, &mut time, "head")?;
        app.request_close(app.active_tab.ok_or("Mannequin document tab missing")?);
        if app.pending_close.is_some() {
            app.accept_close();
        }
        settle(&mut app, &mut time);
        check(
            !app.rig_pending() && !app.human_rig.open,
            "Closed document retained preview",
        )?;
        Ok(())
    })();
    result.and(std::fs::remove_dir_all(dir).map_err(|e| e.to_string()))
}
