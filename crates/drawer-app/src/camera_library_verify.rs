use super::*;
pub(super) fn run(root: &std::path::Path) -> Result<(), String> {
    drawer_core::camera_library_verify::verify(root)?;
    let dir = root.join("camera-library-ui");
    std::fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let result = (|| {
        let check = |ok: bool, s: &str| if ok { Ok(()) } else { Err(s.to_owned()) };
        let ctx = egui::Context::default();
        let mut app = Drawer::new(&ctx, None)
            .with_storage(UserStorage::at(dir.join("data")).map_err(|e| e.to_string())?);
        app.simple_view = true;
        app.add_document(drawer_core::camera_library_verify::fixture()?);
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
        let click = |app: &mut Drawer, t: &mut f64, key: &str, views: bool| -> Result<(), String> {
            let r = *if views {
                app.world_views.controls.get(key)
            } else {
                app.scene_cameras.controls.get(key)
            }
            .ok_or_else(|| format!("Missing camera control {key}"))?;
            check(
                r.center().y < 690.,
                &format!("Camera control clipped: {key}"),
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
            while app.scene_cameras.job.is_some() || app.world_views.busy() {
                frame(app, t, vec![]);
                if std::time::Instant::now() > end {
                    return Err("Camera worker timeout".into());
                }
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            settle(app, t);
            Ok(())
        };
        app.execute_command(&ctx, "scene-cameras");
        settle(&mut app, &mut time);
        let original = app.documents[0].snapshot();
        click(&mut app, &mut time, "enable", false)?;
        drain(&mut app, &mut time)?;
        check(
            app.scene_cameras.staged.is_some() && app.documents[0].snapshot() == original,
            &format!("Camera list preview: {}", app.scene_cameras.notice),
        )?;
        check(
            app.begin_lighting().is_err() && app.begin_pose_sequences().is_err(),
            "Other editor replaced camera preview",
        )?;
        click(&mut app, &mut time, "apply", false)?;
        check(
            app.documents[0].scene.world3d.cameras.entries.len() == 1,
            "Camera list not initialized",
        )?;
        click(&mut app, &mut time, "delete", false)?;
        drain(&mut app, &mut time)?;
        check(
            !app.cameras_pending() && app.documents[0].scene.world3d.cameras.entries.len() == 1,
            "Last camera deletion accepted",
        )?;
        app.scene_cameras.name = "Camera copy".into();
        settle(&mut app, &mut time);
        click(&mut app, &mut time, "duplicate", false)?;
        drain(&mut app, &mut time)?;
        click(&mut app, &mut time, "apply", false)?;
        click(&mut app, &mut time, "camera-2", false)?;
        click(&mut app, &mut time, "switch", false)?;
        drain(&mut app, &mut time)?;
        click(&mut app, &mut time, "apply", false)?;
        check(
            app.documents[0].scene.world3d.cameras.active == 2,
            "Mouse camera switch failed",
        )?;
        let before = app.documents[0].snapshot();
        app.scene_cameras.view.as_mut().unwrap().lens.shift = [0.125, 0.];
        settle(&mut app, &mut time);
        click(&mut app, &mut time, "preview", false)?;
        drain(&mut app, &mut time)?;
        check(
            app.scene_cameras.staged.is_some() && app.documents[0].snapshot() == before,
            &format!("Shift preview absent: {}", app.scene_cameras.notice),
        )?;
        click(&mut app, &mut time, "cancel", false)?;
        check(
            app.documents[0].snapshot() == before && !app.cameras_pending(),
            "Camera cancel changed document",
        )?;
        app.scene_cameras.view.as_mut().unwrap().lens.angles = [-15., 25., 10.];
        settle(&mut app, &mut time);
        click(&mut app, &mut time, "level", false)?;
        check(
            app.scene_cameras.view.as_ref().unwrap().lens.vertical,
            "Mouse vertical correction missing",
        )?;
        click(&mut app, &mut time, "preview", false)?;
        drain(&mut app, &mut time)?;
        click(&mut app, &mut time, "apply", false)?;
        check(
            app.documents[0].scene.world3d.lens.unwrap().vertical,
            "Vertical mode not applied",
        )?;
        click(&mut app, &mut time, "undo", false)?;
        check(
            app.documents[0].snapshot() == before,
            "Camera edit not one Undo",
        )?;
        click(&mut app, &mut time, "parallel", false)?;
        click(&mut app, &mut time, "model-1", false)?;
        click(&mut app, &mut time, "model-2", false)?;
        click(&mut app, &mut time, "fit", false)?;
        drain(&mut app, &mut time)?;
        check(
            app.scene_cameras.staged.is_some(),
            &format!("Camera fit preview missing: {}", app.scene_cameras.notice),
        )?;
        click(&mut app, &mut time, "apply", false)?;
        check(
            app.documents[0].scene.world3d.lens.unwrap().projection == Projection::Orthographic,
            "Mouse parallel fit failed",
        )?;
        let frozen = app.documents[0].snapshot();
        click(&mut app, &mut time, "views", false)?;
        settle(&mut app, &mut time);
        drain(&mut app, &mut time)?;
        click(&mut app, &mut time, "Front", true)?;
        app.world_views.preferences.occlusion[0].point = [0., 0., -2.];
        settle(&mut app, &mut time);
        drain(&mut app, &mut time)?;
        let frames = app.world_views.frames.clone();
        click(&mut app, &mut time, "cut", true)?;
        drain(&mut app, &mut time)?;
        check(
            app.world_views.preferences.occlusion[0].enabled
                && app.world_views.frames[0].pixels != frames[0].pixels
                && app.world_views.frames[1].pixels == frames[1].pixels
                && app.documents[0].snapshot() == frozen,
            "Mouse cutaway changed document/other view or did not clip",
        )?;
        click(&mut app, &mut time, "undo", true)?;
        drain(&mut app, &mut time)?;
        check(
            !app.world_views.preferences.occlusion[0].enabled
                && app.documents[0].snapshot() == frozen,
            "Cutaway Undo view changed document / retained cut",
        )?;
        click(&mut app, &mut time, "redo", true)?;
        drain(&mut app, &mut time)?;
        check(
            app.world_views.preferences.occlusion[0].enabled
                && app.documents[0].snapshot() == frozen,
            "Cutaway Redo view changed document / lost cut",
        )?;
        click(&mut app, &mut time, "save", true)?;
        check(
            drawer_core::world_views::Preferences::load(app.storage.as_ref().unwrap().root())?
                .occlusion[0]
                .enabled,
            "Cutaway workspace not saved",
        )?;
        app.execute_command(&ctx, "scene-cameras");
        settle(&mut app, &mut time);
        app.stage_camera_library(&ctx, Edit::Rename("Stale".into()))?;
        app.documents[0].current_frame = 1;
        drain(&mut app, &mut time)?;
        check(
            !app.cameras_pending() && app.documents[0].snapshot() == frozen,
            "Frame change retained camera preview",
        )?;
        app.stage_camera_library(&ctx, Edit::Rename("Closing".into()))?;
        app.request_close(app.active_tab.ok_or("Missing camera tab")?);
        if app.pending_close.is_some() {
            app.accept_close();
        }
        settle(&mut app, &mut time);
        check(
            !app.scene_cameras.open && !app.cameras_pending(),
            "Closed document retained camera worker",
        )?;
        Ok(())
    })();
    result.and(std::fs::remove_dir_all(dir).map_err(|e| e.to_string()))
}
