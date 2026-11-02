use super::*;
pub(super) fn run(root: &std::path::Path) -> Result<(), String> {
    drawer_core::model_presets_verify::verify(root)?;
    let dir = root.join("model-preset-ui");
    std::fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let result = (|| {
        let check = |b: bool, s: &str| if b { Ok(()) } else { Err(s.to_owned()) };
        let ctx = egui::Context::default();
        let mut app = Drawer::new(&ctx, None)
            .with_storage(UserStorage::at(dir.join("data")).map_err(|e| e.to_string())?);
        app.simple_view = true;
        let mut d = drawer_core::model_library_verify::blank()?;
        for kind in [
            drawer_core::human_rig::Kind::Body,
            drawer_core::human_rig::Kind::Hand,
        ] {
            drawer_core::human_rig::prepare(
                &d,
                drawer_core::human_rig::Edit::Create {
                    rig: drawer_core::human_rig::Rig::new(kind),
                    position: [0.; 3],
                },
                DEFAULT_DOCUMENT_BUDGET,
                &AtomicBool::new(false),
            )?
            .commit(&mut d)?;
        }
        let mut rig = d.scene.world3d.rigs[&1].clone();
        rig.angles[10][0] = 65.;
        rig.shape.shoulders = 1.5;
        drawer_core::human_rig::prepare(
            &d,
            drawer_core::human_rig::Edit::Replace {
                id: 1,
                rig: rig.clone(),
            },
            DEFAULT_DOCUMENT_BUDGET,
            &AtomicBool::new(false),
        )?
        .commit(&mut d)?;
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
        let click = |app: &mut Drawer, t: &mut f64, key: &str, world: bool| -> Result<(), String> {
            let r = if world {
                *app.world3d
                    .controls
                    .get(key)
                    .ok_or("Missing3D preset adoption control")?
            } else {
                *app.model_presets
                    .controls
                    .get(key)
                    .ok_or("Missing preset control")?
            };
            check(r.center().y < 690., "Preset control clipped")?;
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
        app.execute_command(&ctx, "3d-presets");
        settle(&mut app, &mut time);
        check(app.model_presets.open, "Preset command failed")?;
        let before = app.documents[0].snapshot();
        let history = app.documents[0].history_counts();
        let dirty = app.documents[0].dirty;
        click(&mut app, &mut time, "save", false)?;
        check(
            app.model_presets.library.as_ref().unwrap().entries.len() == 1
                && app.documents[0].snapshot() == before
                && app.documents[0].history_counts() == history
                && app.documents[0].dirty == dirty,
            "Capture changed document / did not save",
        )?;
        let mut changed = rig.clone();
        changed.reset_pose();
        drawer_core::human_rig::prepare(
            &app.documents[0],
            drawer_core::human_rig::Edit::Replace {
                id: 1,
                rig: changed,
            },
            DEFAULT_DOCUMENT_BUDGET,
            &AtomicBool::new(false),
        )?
        .commit(&mut app.documents[0])?;
        settle(&mut app, &mut time);
        let before = app.documents[0].snapshot();
        click(&mut app, &mut time, "preview", false)?;
        check(
            app.has_world_preview() && app.documents[0].snapshot() == before,
            &format!("Pose preview failed: {}", app.model_presets.notice),
        )?;
        click(&mut app, &mut time, "discard", true)?;
        check(
            !app.has_world_preview() && app.documents[0].snapshot() == before,
            "Cancel changed pose",
        )?;
        app.begin_model_presets()?;
        settle(&mut app, &mut time);
        click(&mut app, &mut time, "preview", false)?;
        click(&mut app, &mut time, "apply", true)?;
        check(
            app.documents[0].scene.world3d.rigs[&1] == rig,
            "Pose preset Apply failed",
        )?;
        let posed = app.documents[0].snapshot();
        app.documents[0].undo();
        check(
            app.documents[0].snapshot() == before,
            "Preset Apply not one Undo",
        )?;
        app.documents[0].redo();
        check(app.documents[0].snapshot() == posed, "Preset Redo failed")?;
        app.begin_model_presets()?;
        app.model_presets.scope = Scope::Shape;
        app.model_presets.name = "Body shape".into();
        settle(&mut app, &mut time);
        click(&mut app, &mut time, "save", false)?;
        app.model_presets.scope = Scope::Camera;
        app.model_presets.name = "Camera".into();
        click(&mut app, &mut time, "save", false)?;
        check(
            Library::load(&dir.join("data"))?.entries.len() == 3,
            "All preset kinds not stored",
        )?;
        app.model_presets.selected = Some(1);
        app.model_presets.actor = Some(2);
        click(&mut app, &mut time, "preview", false)?;
        check(
            !app.has_world_preview()
                && app.model_presets.notice.contains("Skeleton mismatch")
                && app.documents[0].snapshot() == posed,
            "Wrong skeleton accepted / pose changed",
        )?;
        app.model_presets.selected = Some(3);
        app.model_presets.actor = None;
        let (c, mut l) = drawer_core::world3d::default_camera(&app.documents[0]);
        l.fov = 68.;
        drawer_core::world3d::prepare(
            &app.documents[0],
            drawer_core::world3d::Edit::Camera { camera: c, lens: l },
            DEFAULT_DOCUMENT_BUDGET,
            &AtomicBool::new(false),
        )?
        .commit(&mut app.documents[0])?;
        settle(&mut app, &mut time);
        click(&mut app, &mut time, "preview", false)?;
        check(app.has_world_preview(), "Camera preview missing")?;
        click(&mut app, &mut time, "apply", true)?;
        check(
            app.documents[0].scene.world3d.lens.unwrap().fov == 45.,
            "Camera preset did not apply",
        )?;
        app.begin_model_presets()?;
        app.model_presets.selected = Some(1);
        settle(&mut app, &mut time);
        click(&mut app, &mut time, "preview", false)?;
        app.documents[0].edit_pixel(0, 0, 0, [1, 2, 3, 255]);
        settle(&mut app, &mut time);
        check(
            !app.has_world_preview(),
            "Changed document retained preset preview",
        )?;
        app.begin_model_presets()?;
        let before = app.documents[0].snapshot();
        settle(&mut app, &mut time);
        click(&mut app, &mut time, "delete", false)?;
        check(
            Library::load(&dir.join("data"))?.entries.len() == 2
                && app.documents[0].snapshot() == before,
            "Asset deletion changed drawing / did not persist",
        )?;
        let mut restarted = Drawer::new(&ctx, None)
            .with_storage(UserStorage::at(dir.join("data")).map_err(|e| e.to_string())?);
        restarted.add_document(drawer_core::model_library_verify::blank()?);
        restarted.begin_model_presets()?;
        check(
            restarted
                .model_presets
                .library
                .as_ref()
                .unwrap()
                .entries
                .len()
                == 2,
            "Preset restart failed",
        )?;
        std::fs::write(dir.join("data/settings/model-presets.json"), b"corrupt")
            .map_err(|e| e.to_string())?;
        check(
            restarted.begin_model_presets().is_err()
                && !restarted.model_presets.open
                && restarted.model_presets.library.is_none(),
            "Broken library left stale write access",
        )?;
        Ok(())
    })();
    result.and(std::fs::remove_dir_all(dir).map_err(|e| e.to_string()))
}
