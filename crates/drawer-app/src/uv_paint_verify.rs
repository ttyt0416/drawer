use super::*;
pub(super) fn run(root: &std::path::Path) -> Result<(), String> {
    drawer_core::uv_paint_verify::verify(root)?;
    let dir = root.join("uv-paint-ui");
    std::fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let result = (|| {
        let check = |b: bool, s: &str| if b { Ok(()) } else { Err(s.to_owned()) };
        let ctx = egui::Context::default();
        let mut app = Drawer::new(&ctx, None)
            .with_storage(UserStorage::at(dir.join("data")).map_err(|e| e.to_string())?);
        app.simple_view = true;
        let (d, path) = drawer_core::uv_paint_verify::fixture(&dir)?;
        let original = std::fs::read(&path).map_err(|e| e.to_string())?;
        app.add_document(d);
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
        let button = |app: &mut Drawer, t: &mut f64, key: &str| -> Result<(), String> {
            let r = *app.uv_paint.controls.get(key).ok_or("Missing UV control")?;
            check(r.center().y < 690., "UV control clipped")?;
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
        let stroke = |app: &mut Drawer, t: &mut f64| -> Result<(), String> {
            let r = app.uv_paint.controls["pad"];
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
        app.execute_command(&ctx, "uv-paint");
        settle(&mut app, &mut time);
        check(
            app.uv_paint.open && app.uv_paint.controls.contains_key("uv"),
            "UV command / tile missing",
        )?;
        app.uv_paint.brush = Brush {
            size: 64,
            radius: 4.,
            opacity: 1.,
            rgb: [255, 0, 0],
        };
        let before = app.documents[0].snapshot();
        stroke(&mut app, &mut time)?;
        check(
            app.uv_pending() && app.documents[0].snapshot() == before,
            &format!("UV pointer preview failed: {}", app.uv_paint.notice),
        )?;
        check(
            app.begin_world3d().is_err() && app.begin_lighting().is_err(),
            "Another3D editor replaced UV stroke",
        )?;
        button(&mut app, &mut time, "apply")?;
        let painted = app.documents[0].snapshot();
        check(
            drawer_core::render::Renderer::new(&app.documents[0]).pixel([32., 32.])
                == [255, 0, 0, 255],
            "UV pointer Apply did not render",
        )?;
        app.documents[0].undo();
        check(
            app.documents[0].snapshot() == before,
            "UV stroke not one Undo",
        )?;
        app.documents[0].redo();
        check(
            app.documents[0].snapshot() == painted,
            "UV stroke Redo failed",
        )?;
        settle(&mut app, &mut time);
        app.uv_paint.brush.rgb = [0, 0, 255];
        stroke(&mut app, &mut time)?;
        button(&mut app, &mut time, "discard")?;
        check(
            !app.uv_pending() && app.documents[0].snapshot() == painted,
            "UV discard changed map",
        )?;
        stroke(&mut app, &mut time)?;
        app.documents[0].edit_pixel(0, 0, 0, [1, 2, 3, 255]);
        settle(&mut app, &mut time);
        check(!app.uv_pending(), "Changed source retained UV stroke")?;
        let mut s = app.documents[0].scene.clone();
        s.world3d.surfaces.get_mut(&1).unwrap().triangles[1].t =
            s.world3d.surfaces[&1].triangles[0].t;
        app.documents[0].edit_scene(s)?;
        settle(&mut app, &mut time);
        let before = app.documents[0].snapshot();
        stroke(&mut app, &mut time)?;
        check(app.uv_pending(), "Overlap preview missing")?;
        button(&mut app, &mut time, "apply")?;
        check(
            app.uv_pending() && app.documents[0].snapshot() == before,
            "UV warning bypassed",
        )?;
        button(&mut app, &mut time, "review")?;
        button(&mut app, &mut time, "apply")?;
        check(!app.uv_pending(), "Reviewed UV stroke not applied")?;
        stroke(&mut app, &mut time)?;
        app.request_close(app.active_tab.ok_or("UV tab missing")?);
        if app.pending_close.is_some() {
            app.accept_close();
        }
        settle(&mut app, &mut time);
        check(
            !app.uv_pending() && !app.uv_paint.open,
            "Closed document retained texture draft",
        )?;
        check(
            std::fs::read(&path).map_err(|e| e.to_string())? == original,
            "UV UI changed original OBJ",
        )?;
        Ok(())
    })();
    result.and(std::fs::remove_dir_all(dir).map_err(|e| e.to_string()))
}
