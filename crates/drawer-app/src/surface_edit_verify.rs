use super::*;
pub(super) fn run(root: &std::path::Path) -> Result<(), String> {
    drawer_core::surface_edit_verify::verify(root)?;
    let dir = root.join("surface-ui");
    std::fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let result = (|| {
        let check = |b: bool, s: &str| if b { Ok(()) } else { Err(s.to_owned()) };
        let ctx = egui::Context::default();
        let mut app = Drawer::new(&ctx, None)
            .with_storage(UserStorage::at(dir.join("data")).map_err(|e| e.to_string())?);
        app.simple_view = true;
        app.add_document(drawer_core::surface_edit_verify::fixture(
            drawer_core::model_library::Shape::Plane,
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
            for _ in 0..16 {
                let r = *app
                    .model_controls
                    .controls
                    .get(key)
                    .ok_or_else(|| format!("Missing surface control{key}"))?;
                let visible = *app
                    .model_controls
                    .controls
                    .get("surface-scroll")
                    .ok_or("No surface viewport")?;
                if visible.shrink(5.).contains(r.center()) {
                    break;
                }
                let delta = (r
                    .center()
                    .y
                    .clamp(visible.top() + 10., visible.bottom() - 10.)
                    - r.center().y)
                    .clamp(-240., 240.);
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
                .model_controls
                .controls
                .get(key)
                .ok_or("Missing surface button")?;
            let visible = *app
                .model_controls
                .controls
                .get("surface-scroll")
                .ok_or("No surface viewport")?;
            check(
                visible.contains(r.center()),
                &format!("Clipped surface button{key}:{r:?} viewport{visible:?}"),
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
                    return Err("Surface worker timeout".into());
                }
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            settle(app, t);
            Ok(())
        };
        app.execute_command(&ctx, "model-controls");
        settle(&mut app, &mut time);
        click(&mut app, &mut time, "surface-expand")?;
        app.model_controls.draft.divisions = 2;
        settle(&mut app, &mut time);
        let original = app.documents[0].snapshot();
        click(&mut app, &mut time, "surface-divisions")?;
        drain(&mut app, &mut time)?;
        check(
            app.model_controls.staged.is_some() && app.documents[0].snapshot() == original,
            &format!("Subdivision preview missing:{}", app.model_controls.notice),
        )?;
        click(&mut app, &mut time, "apply")?;
        check(
            app.documents[0].scene.world3d.surfaces[&1]
                .tools
                .primitive
                .as_ref()
                .unwrap()
                .detail
                == 2,
            "Subdivision pointer apply failed",
        )?;
        app.documents[0].undo();
        settle(&mut app, &mut time);
        check(
            app.documents[0].snapshot() == original,
            "Subdivision pointer Undo failed",
        )?;
        click(&mut app, &mut time, "surface-uv-generate")?;
        drain(&mut app, &mut time)?;
        click(&mut app, &mut time, "apply")?;
        let path = dir.join("color.png");
        drawer_core::surface_edit_verify::image(
            &path,
            &[
                255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255,
            ],
            2,
            2,
        )?;
        app.model_controls.draft.map_path = path.display().to_string();
        settle(&mut app, &mut time);
        let before = app.documents[0].snapshot();
        click(&mut app, &mut time, "surface-diffuse")?;
        drain(&mut app, &mut time)?;
        check(
            app.model_controls.staged.is_some() && app.documents[0].snapshot() == before,
            &format!("Texture preview missing:{}", app.model_controls.notice),
        )?;
        click(&mut app, &mut time, "cancel")?;
        check(
            app.documents[0].snapshot() == before,
            "Texture cancel changed source",
        )?;
        click(&mut app, &mut time, "surface-diffuse")?;
        drain(&mut app, &mut time)?;
        click(&mut app, &mut time, "apply")?;
        check(
            app.documents[0].scene.world3d.surfaces[&1].tools.materials[&0]
                .diffuse
                .is_some(),
            "Texture pointer apply missing",
        )?;
        app.model_controls.draft.uv.offset = [0.5, 0.];
        settle(&mut app, &mut time);
        click(&mut app, &mut time, "surface-uv")?;
        drain(&mut app, &mut time)?;
        click(&mut app, &mut time, "apply")?;
        check(
            app.documents[0].scene.world3d.surfaces[&1].tools.materials[&0]
                .uv
                .offset
                == [0.5, 0.],
            "UV pointer apply missing",
        )?;
        let normal = dir.join("normal.png");
        drawer_core::surface_edit_verify::image(&normal, &[128, 128, 255, 255], 1, 1)?;
        app.model_controls.draft.map_path = normal.display().to_string();
        settle(&mut app, &mut time);
        click(&mut app, &mut time, "surface-normal")?;
        drain(&mut app, &mut time)?;
        click(&mut app, &mut time, "apply")?;
        check(
            app.documents[0].scene.world3d.surfaces[&1].tools.materials[&0]
                .normal
                .is_some(),
            "Normal pointer apply missing",
        )?;
        let snapshot = app.documents[0].snapshot();
        app.model_controls.draft.export_folder = dir.join("textures").display().to_string();
        app.model_controls.draft.conflict = drawer_core::surface_edit::Conflict::Rename;
        settle(&mut app, &mut time);
        click(&mut app, &mut time, "surface-export")?;
        drain(&mut app, &mut time)?;
        check(
            !app.model_edit_pending()
                && app.documents[0].snapshot() == snapshot
                && dir.join("textures/texture-color.png").is_file()
                && dir.join("textures/texture-normal.png").is_file(),
            &format!("Texture export failed:{}", app.model_controls.notice),
        )?;
        std::fs::remove_file(&path).map_err(|e| e.to_string())?;
        click(&mut app, &mut time, "surface-reload")?;
        drain(&mut app, &mut time)?;
        check(
            !app.model_edit_pending() && app.documents[0].snapshot() == snapshot,
            "Missing reload changed document",
        )?;
        app.stage_model_controls(
            &ctx,
            Edit::Surface {
                model: 1,
                change: drawer_core::surface_edit::Change::Display(
                    drawer_core::surface_edit::Display::Wire,
                ),
            },
        )?;
        app.documents[0].current_frame += 1;
        settle(&mut app, &mut time);
        check(!app.model_edit_pending(), "Stale surface frame retained")?;
        app.stage_model_controls(
            &ctx,
            Edit::Surface {
                model: 1,
                change: drawer_core::surface_edit::Change::Display(
                    drawer_core::surface_edit::Display::Wire,
                ),
            },
        )?;
        app.active = None;
        settle(&mut app, &mut time);
        check(
            !app.model_edit_pending() && !app.model_controls.open,
            "Surface close failed to join job",
        )?;
        Ok(())
    })();
    result.and(std::fs::remove_dir_all(dir).map_err(|e| e.to_string()))
}
