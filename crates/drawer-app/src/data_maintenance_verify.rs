use super::*;
pub(super) fn run(root: &Path) -> Result<(), String> {
    core::verify::run(root)?;
    let dir = root.join("data-maintenance-ui");
    std::fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let result = (|| {
        let ctx = egui::Context::default();
        let storage = UserStorage::at(dir.join("사용자")).map_err(|e| e.to_string())?;
        let anchor = storage.anchor().to_owned();
        let asset = storage.root().join("materials/source.png");
        let mut d = drawer_core::png_io_verify::fixture()?;
        drawer_core::png_export::export(&d, &asset, 64 * 1024 * 1024).map_err(|e| e.to_string())?;
        let original = storage.root().join("documents/원본.drawer");
        std::fs::create_dir_all(original.parent().ok_or("Document folder")?)
            .map_err(|e| e.to_string())?;
        drawer_core::files::save(&mut d, &original).map_err(|e| e.to_string())?;
        let bytes = std::fs::read(&original).map_err(|e| e.to_string())?;
        let (state, _) = drawer_core::file_objects::create(&d, &asset, "연결")?;
        d.commit_state(state)?;
        let mut app = Drawer::new(&ctx, None).with_storage(storage);
        app.simple_view = true;
        app.add_document(d);
        let layers = app.documents[0].layers.clone();
        let history = app.documents[0].history_counts();
        let old_path = app.documents[0].path.clone();
        app.workspaces
            .store
            .current
            .bindings
            .retain(|b| b.key != "F9");
        app.workspaces
            .store
            .current
            .bindings
            .push(drawer_core::workspaces::Binding {
                key: "F9".into(),
                ctrl: false,
                shift: false,
                alt: false,
                command: "zoom-in".into(),
            });
        app.documents[0].view.angle = 77.;
        app.data_maintenance.open = true;
        app.data_maintenance.destination = dir.join("새 사용자").display().to_string();
        app.data_maintenance.acknowledged = true;
        let mut t = 0.;
        let frame = |a: &mut Drawer, t: &mut f64, events| {
            *t += 0.05;
            input_replay::frame(&ctx, a, *t, events);
        };
        for _ in 0..3 {
            frame(&mut app, &mut t, vec![]);
        }
        let pos = app
            .data_maintenance
            .controls
            .get("move")
            .ok_or("Missing data move button")?
            .center();
        for pressed in [true, false] {
            frame(
                &mut app,
                &mut t,
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
        if !app.data_maintenance.busy() {
            return Err(format!(
                "Move did not start: {}",
                app.data_maintenance.message
            ));
        }
        app.execute_command(&ctx, "view-reset");
        if app.data_maintenance.commands.len() != 1 || app.documents[0].view.angle != 77. {
            return Err("New command did not wait for data move".into());
        }
        frame(
            &mut app,
            &mut t,
            [true, false]
                .into_iter()
                .map(|pressed| egui::Event::Key {
                    key: egui::Key::F9,
                    physical_key: None,
                    pressed,
                    repeat: false,
                    modifiers: Default::default(),
                })
                .collect(),
        );
        for _ in 0..8000 {
            frame(&mut app, &mut t, vec![]);
            if !app.data_maintenance.busy() && app.data_maintenance.commands.is_empty() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        if app.data_maintenance.busy()
            || !app.data_maintenance.message.starts_with("Moved ")
            || app.documents[0].layers != layers
            || app.documents[0].history_counts() != history
            || app.documents[0].path != old_path
            || app.documents[0].view.angle != 0.
            || app.documents[0].view.zoom <= 1.
            || !app.workspaces.input_idle()
            || std::fs::read(&original).map_err(|e| e.to_string())? != bytes
        {
            return Err(format!(
                "Data move/live document/queue: {}",
                app.data_maintenance.message
            ));
        }
        let new = app
            .storage
            .as_ref()
            .ok_or("New storage missing")?
            .root()
            .to_owned();
        if !app.documents[0].scene.file_objects.entries[0]
            .path
            .starts_with(new.join("materials"))
            || !drawer_core::file_browser::same(
                UserStorage::at(anchor).map_err(|e| e.to_string())?.root(),
                &new,
            )
        {
            return Err("Live asset link/restart root mismatch".into());
        }
        let storage = app.storage.as_ref().ok_or("Moved storage")?;
        let anchor = storage.anchor();
        let pointer = anchor.parent().ok_or("Bootstrap parent")?.join(format!(
            ".{}.drawer-location.json",
            anchor.file_name().unwrap().to_string_lossy()
        ));
        if !protected_storage_target(storage, &pointer)
            || !protected_storage_target(storage, anchor)
        {
            return Err("Browser could mutate the bootstrap pointer".into());
        }
        let bundle = dir.join("full-backup");
        app.save_to(0, &dir.join("current-app.drawer"))?;
        app.data_maintenance.snapshot_operation = Some(SnapshotOperation::Create {
            target: bundle.clone(),
            documents: vec![original.clone()],
        });
        app.data_maintenance.pending = true;
        for _ in 0..8000 {
            frame(&mut app, &mut t, vec![]);
            if !app.data_maintenance.busy() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        if !bundle.join("bundle.json").is_file()
            || !app
                .data_maintenance
                .message
                .starts_with("Verified local backup:")
        {
            return Err(format!(
                "Full backup worker: {}",
                app.data_maintenance.message
            ));
        }
        let before_restore = app
            .storage
            .as_ref()
            .ok_or("Current storage")?
            .root()
            .to_owned();
        std::fs::write(
            before_restore.join("settings/before-restore.guard"),
            b"keep current",
        )
        .map_err(|e| e.to_string())?;
        let restored = dir.join("full-restored");
        app.data_maintenance.snapshot_operation = Some(SnapshotOperation::Restore {
            bundle,
            target: restored.clone(),
        });
        app.data_maintenance.pending = true;
        for _ in 0..8000 {
            frame(&mut app, &mut t, vec![]);
            if app.data_maintenance.restart_required || !app.data_maintenance.busy() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        if !app.data_maintenance.restart_required
            || !drawer_core::file_browser::same(
                UserStorage::at(
                    app.storage
                        .as_ref()
                        .ok_or("Current storage")?
                        .anchor()
                        .into(),
                )
                .map_err(|e| e.to_string())?
                .root(),
                &restored,
            )
            || std::fs::read(
                dir.join("full-restored-before-restore/userdata/settings/before-restore.guard"),
            )
            .map_err(|e| e.to_string())?
                != b"keep current"
            || std::fs::read(restored.join("restored-documents/원본.drawer"))
                .map_err(|e| e.to_string())?
                != bytes
            || std::fs::read(&original).map_err(|e| e.to_string())? != bytes
        {
            return Err(format!(
                "Full restore/preserved settings/document/restart gate: {}",
                app.data_maintenance.message
            ));
        }
        app.data_maintenance.commands.push_back("zoom-in".into());
        frame(&mut app, &mut t, vec![]);
        if app.data_maintenance.commands.len() != 1 || app.documents[0].layers != layers {
            return Err("Restore restart gate allowed a drawing command".into());
        }
        drop(app);
        Ok(())
    })();
    drawer_core::asset_catalog::cleanup_verification(root, &dir)?;
    result
}
#[cfg(test)]
mod tests {
    #[test]
    fn local_data_pointer_move_deferred_command_source_and_live_drawing() {
        let t = crate::tests::TestDirectory::new();
        super::run(&t.0).unwrap();
    }
}
