use super::*;
pub(super) fn run(root: &std::path::Path) -> Result<(), String> {
    let dir = root.join("model-formats-ui");
    std::fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let result = (|| {
        for ext in drawer_core::model_source::EXTENSIONS {
            let source = drawer_core::model_formats_verify::fixture(&dir.join(ext), ext)?;
            let original = std::fs::read(&source).map_err(|e| e.to_string())?;
            let ctx = egui::Context::default();
            let mut app = Drawer::new(&ctx, None).with_storage(
                UserStorage::at(dir.join(format!("data-{ext}"))).map_err(|e| e.to_string())?,
            );
            app.simple_view = true;
            app.add_document(drawer_core::model_library_verify::blank()?);
            let mut time = 0.;
            let frame = |a: &mut Drawer, t: &mut f64, events| {
                *t += 0.05;
                input_replay::frame(&ctx, a, *t, events);
            };
            let settle = |a: &mut Drawer, t: &mut f64| {
                for _ in 0..3 {
                    frame(a, t, vec![]);
                }
            };
            let click =
                |a: &mut Drawer, t: &mut f64, key: &str, world: bool| -> Result<(), String> {
                    let r = if world {
                        a.world3d.controls.get(key).copied()
                    } else {
                        a.world_models.controls.get(key).copied()
                    }
                    .ok_or_else(|| format!("Missing {ext} import control {key}"))?;
                    if r.center().y > 690. {
                        return Err(format!("Clipped {ext} import control {key}"));
                    }
                    for pressed in [true, false] {
                        frame(
                            a,
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
                    settle(a, t);
                    Ok(())
                };
            let drain = |a: &mut Drawer, t: &mut f64| -> Result<(), String> {
                let end = std::time::Instant::now() + std::time::Duration::from_secs(30);
                while a.models_busy() {
                    frame(a, t, vec![]);
                    if std::time::Instant::now() > end {
                        return Err("Format worker timeout".into());
                    }
                    std::thread::sleep(std::time::Duration::from_millis(2));
                }
                settle(a, t);
                Ok(())
            };
            app.execute_command(&ctx, "model-library");
            settle(&mut app, &mut time);
            click(&mut app, &mut time, "formats", false)?;
            click(&mut app, &mut time, "formats", false)?;
            app.world_models.path = source.display().to_string();
            app.world_models.fit = false;
            let before = app.documents[0].snapshot();
            click(&mut app, &mut time, "import", false)?;
            drain(&mut app, &mut time)?;
            if !app.has_world_preview() || app.documents[0].snapshot() != before {
                return Err(format!("{ext} preview failed: {}", app.world_models.notice));
            }
            click(&mut app, &mut time, "source-terms", true)?;
            if app.documents[0].snapshot() != before {
                return Err("Opening usage terms committed model".into());
            }
            click(&mut app, &mut time, "source-terms", true)?;
            click(&mut app, &mut time, "model-review", true)?;
            click(&mut app, &mut time, "apply", true)?;
            let source_data = app.documents[0]
                .scene
                .world3d
                .surfaces
                .get(&1)
                .and_then(|s| s.imported.as_ref())
                .and_then(|a| a.source.as_ref())
                .ok_or("UI adoption lost provenance")?;
            if source_data.notices.is_empty() {
                return Err("UI adoption lost usage metadata".into());
            }
            app.documents[0].undo();
            if app.documents[0].snapshot() != before {
                return Err("Format UI Undo incomplete".into());
            }
            if std::fs::read(source).map_err(|e| e.to_string())? != original {
                return Err("Format UI changed original file".into());
            }
        }
        Ok(())
    })();
    result.and(std::fs::remove_dir_all(&dir).map_err(|e| e.to_string()))
}
