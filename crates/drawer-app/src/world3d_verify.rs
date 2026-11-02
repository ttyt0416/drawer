use super::*;
pub(super) fn run(root: &std::path::Path) -> Result<(), String> {
    drawer_core::world3d_verify::verify(root)?;
    let dir = root.join("world3d-ui-verify");
    std::fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let result = (|| {
        let check = |b: bool, s: &str| if b { Ok(()) } else { Err(s.to_owned()) };
        let ctx = egui::Context::default();
        let mut app = Drawer::new(&ctx, None)
            .with_storage(UserStorage::at(dir.join("user")).map_err(|e| e.to_string())?);
        app.simple_view = true;
        app.create_canvas(CanvasSpec {
            width: 64,
            height: 64,
            dpi: 300.,
            background: Background::Transparent,
        })
        .map_err(|e| e.to_string())?;
        app.execute_command(&ctx, "world3d");
        check(app.world3d.open, "3D command did not open")?;
        let mut time = 0.;
        let frame = |app: &mut Drawer, t: &mut f64, events| {
            *t += 0.05;
            input_replay::frame(&ctx, app, *t, events);
        };
        let click = |app: &mut Drawer, t: &mut f64, key: &str| -> Result<(), String> {
            let r = *app
                .world3d
                .controls
                .get(key)
                .ok_or_else(|| format!("Missing3D control{key}"))?;
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
            for _ in 0..2 {
                frame(app, t, vec![]);
            }
            Ok(())
        };
        for _ in 0..3 {
            frame(&mut app, &mut time, vec![]);
        }
        let original = app.documents[0].snapshot();
        click(&mut app, &mut time, "reference")?;
        check(
            app.world3d.staged.is_some() && app.documents[0].snapshot() == original,
            "Pointer reference preview changed original / failed",
        )?;
        click(&mut app, &mut time, "apply")?;
        check(
            app.documents[0].scene.worlds.len() == 3,
            "Reference apply failed",
        )?;
        let before = app.documents[0].snapshot();
        let history = app.documents[0].history_counts();
        let dirty = app.documents[0].dirty;
        app.world3d.preferences.pivot = Pivot::FirstSelected;
        app.world3d.preferences.axes = Axes::Object;
        app.world3d.preferences.step = 7.5;
        frame(&mut app, &mut time, vec![]);
        check(
            app.documents[0].snapshot() == before
                && app.documents[0].history_counts() == history
                && app.documents[0].dirty == dirty,
            "Pivot / axis options changed document",
        )?;
        click(&mut app, &mut time, "save")?;
        let prefs = app.world3d.preferences;
        let restarted = Drawer::new(&ctx, None)
            .with_storage(UserStorage::at(dir.join("user")).map_err(|e| e.to_string())?);
        check(
            restarted.world3d.preferences == prefs,
            "Local3D defaults not restored",
        )?;
        drop(restarted);
        app.world3d.ids = vec![1];
        app.world3d.preferences = Preferences::default();
        app.world3d.axis = 0;
        for _ in 0..2 {
            frame(&mut app, &mut time, vec![]);
        }
        let pad = app.world3d.controls["pad"];
        let from = pad.center() + egui::vec2(-100., 55.);
        let to = from + egui::vec2(70., 0.);
        frame(
            &mut app,
            &mut time,
            vec![
                egui::Event::PointerMoved(from),
                egui::Event::PointerButton {
                    pos: from,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: Default::default(),
                },
            ],
        );
        frame(
            &mut app,
            &mut time,
            vec![egui::Event::PointerMoved(from + egui::vec2(20., 0.))],
        );
        frame(&mut app, &mut time, vec![egui::Event::PointerMoved(to)]);
        check(
            app.world3d.drag.is_some()
                && app.world3d.staged.is_some()
                && app.documents[0].snapshot() == before,
            "Gizmo drag did not stage / changed original",
        )?;
        frame(
            &mut app,
            &mut time,
            vec![egui::Event::PointerButton {
                pos: to,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: Default::default(),
            }],
        );
        for _ in 0..2 {
            frame(&mut app, &mut time, vec![]);
        }
        check(
            app.world3d.drag.is_none() && app.world3d.staged.is_some(),
            "Release lost3D preview",
        )?;
        let released_preview = app.world3d.staged.as_ref().unwrap().scene().clone();
        frame(
            &mut app,
            &mut time,
            vec![
                egui::Event::PointerMoved(from),
                egui::Event::PointerButton {
                    pos: from,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: Default::default(),
                },
            ],
        );
        frame(&mut app, &mut time, vec![egui::Event::PointerMoved(to)]);
        frame(
            &mut app,
            &mut time,
            vec![egui::Event::PointerButton {
                pos: to,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: Default::default(),
            }],
        );
        check(
            app.world3d.drag.is_none()
                && app
                    .world3d
                    .staged
                    .as_ref()
                    .is_some_and(|s| s.scene() == &released_preview)
                && app.documents[0].snapshot() == before,
            "Second drag replaced a pending preview",
        )?;
        for _ in 0..2 {
            frame(&mut app, &mut time, vec![]);
        }
        click(&mut app, &mut time, "apply")?;
        check(
            app.documents[0].snapshot() != before
                && app.documents[0].history_counts().0 == history.0 + 1,
            "Gizmo apply did not use one Undo",
        )?;
        app.documents[0].undo();
        check(app.documents[0].snapshot() == before, "Gizmo Undo failed")?;
        for _ in 0..2 {
            frame(&mut app, &mut time, vec![]);
        }
        app.world3d.value = [0.5, 0., 0.];
        click(&mut app, &mut time, "transform")?;
        check(
            app.world3d.staged.is_some(),
            &format!(
                "Numeric transform preview missing: {}; selection {:?}; button {:?}",
                app.world3d.notice,
                app.world3d.ids,
                app.world3d.controls.get("transform")
            ),
        )?;
        frame(
            &mut app,
            &mut time,
            vec![egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Default::default(),
            }],
        );
        frame(
            &mut app,
            &mut time,
            vec![egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed: false,
                repeat: false,
                modifiers: Default::default(),
            }],
        );
        check(
            app.world3d.staged.is_none() && app.documents[0].snapshot() == before,
            "Esc changed original / retained preview",
        )?;
        app.world3d.preview(
            &app.documents[0],
            Edit::Transform {
                ids: vec![2],
                options: Preferences {
                    mode: Mode::Rotate,
                    ..Default::default()
                },
                value: [0., 0., 30.],
            },
            DEFAULT_DOCUMENT_BUDGET,
        );
        app.documents[0].edit_pixel(0, 0, 0, [1, 2, 3, 255]);
        let changed = app.documents[0].snapshot();
        frame(&mut app, &mut time, vec![]);
        check(
            app.world3d.staged.is_none() && app.documents[0].snapshot() == changed,
            "Stale3D preview replaced edits",
        )?;
        app.world3d
            .preview(&app.documents[0], Edit::Reference, DEFAULT_DOCUMENT_BUDGET);
        app.request_close(app.active_tab.ok_or("3D document tab missing")?);
        if app.pending_close.is_some() {
            app.accept_close();
        }
        frame(&mut app, &mut time, vec![]);
        check(
            app.documents.is_empty() && app.world3d.staged.is_none() && app.world3d.drag.is_none(),
            "Closed document retained3D snapshot",
        )?;
        Ok(())
    })();
    let clean = std::fs::remove_dir_all(&dir).map_err(|e| e.to_string());
    result.and(clean)
}
