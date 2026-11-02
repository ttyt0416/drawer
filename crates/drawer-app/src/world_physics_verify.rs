use super::*;
pub(super) fn run(root: &std::path::Path) -> Result<(), String> {
    drawer_core::world_physics_verify::verify(root)?;
    let dir = root.join("world-physics-ui");
    std::fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let result = (|| {
        let check = |b: bool, s: &str| if b { Ok(()) } else { Err(s.to_owned()) };
        let ctx = egui::Context::default();
        let mut app = Drawer::new(&ctx, None)
            .with_storage(UserStorage::at(dir.join("data")).map_err(|e| e.to_string())?);
        app.simple_view = true;
        app.add_document(drawer_core::world_physics_verify::fixture()?);
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
                    .ok_or("Missing placement adoption control")?
            } else {
                *app.world_physics
                    .controls
                    .get(key)
                    .ok_or("Missing placement control")?
            };
            check(r.center().y < 690., "Placement control clipped")?;
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
        app.execute_command(&ctx, "world-physics");
        settle(&mut app, &mut time);
        check(app.world_physics.open, "Placement command did not open")?;
        click(&mut app, &mut time, "select-1", false)?;
        let before = app.documents[0].snapshot();
        click(&mut app, &mut time, "floor", false)?;
        check(
            app.has_world_preview() && app.documents[0].snapshot() == before,
            &format!("Floor preview failed: {}", app.world_physics.notice),
        )?;
        click(&mut app, &mut time, "discard", true)?;
        check(
            app.documents[0].snapshot() == before && !app.has_world_preview(),
            "Placement cancel changed source",
        )?;
        app.begin_world_physics()?;
        settle(&mut app, &mut time);
        app.world_physics.timestep = 1e6;
        click(&mut app, &mut time, "settle", false)?;
        check(
            app.has_world_preview(),
            &format!("Settle preview failed: {}", app.world_physics.notice),
        )?;
        click(&mut app, &mut time, "apply", true)?;
        check(
            app.has_world_preview() && app.documents[0].snapshot() == before,
            "Unreviewed physics warning applied",
        )?;
        click(&mut app, &mut time, "model-review", true)?;
        click(&mut app, &mut time, "apply", true)?;
        check(
            (drawer_core::world3d::center(&app.documents[0].scene.worlds[0])[1] - 2.).abs() < 1e-7,
            "Gravity Apply failed",
        )?;
        let landed = app.documents[0].snapshot();
        app.documents[0].undo();
        check(
            app.documents[0].snapshot() == before,
            "Gravity not one Undo",
        )?;
        app.documents[0].redo();
        check(app.documents[0].snapshot() == landed, "Gravity Redo failed")?;
        app.begin_world_physics()?;
        settle(&mut app, &mut time);
        click(&mut app, &mut time, "array", false)?;
        click(&mut app, &mut time, "apply", true)?;
        check(
            app.documents[0].scene.worlds.len() == 4,
            "Grid copies failed",
        )?;
        app.begin_world_physics()?;
        settle(&mut app, &mut time);
        click(&mut app, &mut time, "collider", false)?;
        click(&mut app, &mut time, "apply", true)?;
        check(
            app.documents[0].scene.world3d.colliders.contains(&1),
            "Collider mark not saved",
        )?;
        app.begin_world_physics()?;
        settle(&mut app, &mut time);
        click(&mut app, &mut time, "floor", false)?;
        app.documents[0].edit_pixel(0, 0, 0, [1, 2, 3, 255]);
        settle(&mut app, &mut time);
        check(!app.has_world_preview(), "Stale placement retained")?;
        app.begin_world_physics()?;
        settle(&mut app, &mut time);
        click(&mut app, &mut time, "floor", false)?;
        app.request_close(app.active_tab.ok_or("Placement document tab missing")?);
        if app.pending_close.is_some() {
            app.accept_close();
        }
        settle(&mut app, &mut time);
        check(
            !app.has_world_preview() && !app.world_physics.open,
            "Closed document retained placement",
        )?;
        Ok(())
    })();
    result.and(std::fs::remove_dir_all(dir).map_err(|e| e.to_string()))
}
