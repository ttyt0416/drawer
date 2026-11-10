use super::*;
pub(super) fn run(root: &std::path::Path) -> Result<(), String> {
    coordinate_recording(root)?;
    drawer_core::auto_actions_verify::verify(root)?;
    let dir = root.join("auto-actions-ui");
    std::fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let result = (|| {
        let ctx = egui::Context::default();
        let data = dir.join("data");
        let mut app = Drawer::new(&ctx, None)
            .with_storage(UserStorage::at(data.clone()).map_err(|e| e.to_string())?);
        app.simple_view = true;
        app.add_document(drawer_core::png_io_verify::fixture()?);
        app.execute_command(&ctx, "auto-actions");
        let mut time = 0.;
        let frame = |app: &mut Drawer, time: &mut f64, events| {
            *time += 0.05;
            input_replay::frame(&ctx, app, *time, events);
        };
        let settle = |app: &mut Drawer, time: &mut f64| {
            for _ in 0..3 {
                frame(app, time, vec![]);
            }
        };
        let click = |app: &mut Drawer, time: &mut f64, key: &str| -> Result<(), String> {
            let r = app
                .auto_actions
                .controls
                .get(key)
                .copied()
                .ok_or_else(|| format!("Missing action control {key}"))?;
            if r.center().y > 690. {
                return Err(format!("Clipped action control {key}"));
            }
            for pressed in [true, false] {
                frame(
                    app,
                    time,
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
            settle(app, time);
            Ok(())
        };
        settle(&mut app, &mut time);
        click(&mut app, &mut time, "new-set")?;
        click(&mut app, &mut time, "new-action")?;
        click(&mut app, &mut time, "record")?;
        if app.auto_actions.recording.is_none() {
            return Err(format!(
                "Record pointer failed: {}",
                app.auto_actions.message
            ));
        }
        let original = app.documents[0].snapshot();
        let original_history = app.documents[0].history_counts();
        let mut panel = std::mem::take(&mut app.auto_actions);
        panel.operation = Operation::Fill {
            layer: 0,
            rgba: [15, 90, 170, 255],
            erase: false,
            alpha_lock: false,
        };
        app.execute_auto_operation(&mut panel)?;
        panel.operation = Operation::RenameRaster {
            layer: 0,
            name: "Action target".into(),
        };
        app.execute_auto_operation(&mut panel)?;
        let id = panel.action.ok_or("No action selected")?;
        if panel.draft.action(id).unwrap().steps.len() != 2 {
            return Err("Commands not recorded".into());
        }
        app.auto_actions = panel;
        app.auto_actions_nonrecordable("zoom-in");
        if !app.auto_actions.message.contains("Not recorded") {
            return Err("Nonrecordable command was silent".into());
        }
        settle(&mut app, &mut time);
        click(&mut app, &mut time, "stop")?;
        click(&mut app, &mut time, "save")?;
        let settings = app.auto_actions.draft.clone();
        let recorded = app.documents[0].snapshot();
        if settings != Settings::load(app.storage.as_ref().unwrap())? {
            return Err("Settings save / restart differs".into());
        }
        if !app.documents[0].undo()
            || !app.documents[0].undo()
            || app.documents[0].snapshot() != original
        {
            return Err("Recorded edits not independently undoable".into());
        }
        let mut panel = std::mem::take(&mut app.auto_actions);
        app.start_auto_action(&mut panel, id, false)?;
        app.auto_actions = panel;
        for _ in 0..200 {
            frame(&mut app, &mut time, vec![]);
            if app.auto_actions.job.is_none() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        if app.auto_actions.prepared.is_none() || app.documents[0].snapshot() != original {
            return Err(format!(
                "Preview changed document / failed: {}",
                app.auto_actions.message
            ));
        }
        let mut panel = std::mem::take(&mut app.auto_actions);
        app.apply_auto_action(&mut panel)?;
        app.auto_actions = panel;
        if app.documents[0].snapshot() != recorded
            || app.documents[0].history_counts().0 != original_history.0 + 1
        {
            return Err("Replay did not group into one Undo".into());
        }
        if !app.documents[0].undo() || app.documents[0].snapshot() != original {
            return Err("Replay one Undo failed".into());
        }
        let mut panel = std::mem::take(&mut app.auto_actions);
        app.start_auto_action(&mut panel, id, true)?;
        panel
            .job
            .as_ref()
            .unwrap()
            .cancel
            .store(true, Ordering::Relaxed);
        app.auto_actions = panel;
        for _ in 0..200 {
            frame(&mut app, &mut time, vec![]);
            if app.auto_actions.job.is_none() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        if app.auto_actions.job.is_some() || app.documents[0].snapshot() != original {
            return Err("Canceled worker edited document".into());
        }
        // Button mode sends one job per click and commits automatically once.
        app.auto_actions.draft.mode = Mode::Buttons;
        settle(&mut app, &mut time);
        click(&mut app, &mut time, "play-button")?;
        for _ in 0..200 {
            frame(&mut app, &mut time, vec![]);
            if app.auto_actions.job.is_none() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        if app.documents[0].snapshot() != recorded
            || app.documents[0].history_counts().0 != original_history.0 + 1
        {
            return Err(format!(
                "Button playback failed: {}",
                app.auto_actions.message
            ));
        }
        let unchanged = app.documents[0].snapshot();
        let history = app.documents[0].history_counts();
        let source = dir.join("sets.json");
        settings.write(&source, false)?;
        let mut panel = std::mem::take(&mut app.auto_actions);
        panel.path = source.display().to_string();
        let before = panel.draft.clone();
        app.read_auto_sets(&mut panel)?;
        if panel.draft != before {
            return Err("Read registered imported action sets".into());
        }
        panel.import = None;
        if panel.draft != before {
            return Err("Import cancel registered sets".into());
        }
        app.read_auto_sets(&mut panel)?;
        app.import_auto_sets(&mut panel)?;
        if panel.draft.sets.len() != 2 || panel.draft.sets[0].id == panel.draft.sets[1].id {
            return Err("Independent set import failed".into());
        }
        panel.changed(before.clone());
        panel.prepared = Some(core::prepare(
            &app.documents[0],
            settings.action(id).unwrap(),
            DEFAULT_DOCUMENT_BUDGET,
            &AtomicBool::new(false),
        )?);
        panel.restore(false);
        if panel.draft != before || panel.prepared.is_some() {
            return Err("Settings-only undo failed".into());
        }
        panel.restore(true);
        app.save_auto_actions(&mut panel)?;
        let saved = panel.draft.clone();
        panel.recording = Some(id);
        app.active = None;
        if app.execute_auto_operation(&mut panel).is_ok() || panel.draft != saved {
            return Err("No-document recording changed settings".into());
        }
        if app.documents[0].snapshot() != unchanged || app.documents[0].history_counts() != history
        {
            return Err("Action settings edits changed document history".into());
        }
        let restart =
            Drawer::new(&ctx, None).with_storage(UserStorage::at(data).map_err(|e| e.to_string())?);
        if restart.auto_actions.draft != saved {
            return Err("Local sets/mode/parameters not restored".into());
        }
        Ok(())
    })();
    result.and(std::fs::remove_dir_all(dir).map_err(|e| e.to_string()))
}

fn coordinate_recording(root: &std::path::Path) -> Result<(), String> {
    let dir = root.join("action-coordinate-recording");
    std::fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let result = (|| {
        let ctx = egui::Context::default();
        let mut a = Drawer::new(&ctx, None)
            .with_storage(UserStorage::at(dir.join("data")).map_err(|e| e.to_string())?);
        a.add_document(drawer_core::png_io_verify::fixture()?);
        a.interface.ppp = 2.;
        let mut p = Panel::default();
        let set = p.draft.add_set("座標".into())?;
        let action = p.draft.add_action(set, "문서 좌표".into())?;
        p.recording = Some(action);
        p.operation = Operation::Scroll {
            document: [7., -3.],
        };
        let before = a.documents[0].layers.clone();
        let history = a.documents[0].history_counts();
        let size = [a.documents[0].width as f64, a.documents[0].height as f64];
        let center = a.documents[0]
            .view
            .screen_to_document([0.; 2], size, [0.; 2], 2.);
        a.execute_auto_operation(&mut p)?;
        let after = a.documents[0]
            .view
            .screen_to_document([0.; 2], size, [0.; 2], 2.);
        if a.documents[0].layers != before
            || a.documents[0].history_counts() != history
            || (after[0] - center[0] - 7.).abs() > 1e-6
            || (after[1] - center[1] + 3.).abs() > 1e-6
            || p.draft.action(action).unwrap().steps.len() != 1
        {
            return Err(
                "Recorded scroll changed pixels/history or lost document coordinates".into(),
            );
        }
        p.operation = Operation::Transform {
            layer: 0,
            vector: false,
            settings: drawer_core::vector_scope::Transform {
                translation: [1., 0.],
                ..Default::default()
            },
            filter: drawer_core::interpolation::Filter::Nearest,
        };
        a.execute_auto_operation(&mut p)?;
        if a.documents[0].history_counts().0 != history.0 + 1
            || p.draft.action(action).unwrap().steps.len() != 2
        {
            return Err("Transform recording/history".into());
        }
        p.draft.save(a.storage.as_ref().unwrap())?;
        if Settings::load(a.storage.as_ref().unwrap())? != p.draft {
            return Err("Coordinate action restart".into());
        }
        drop(a);
        Ok(())
    })();
    drawer_core::asset_catalog::cleanup_verification(root, &dir)?;
    result
}
