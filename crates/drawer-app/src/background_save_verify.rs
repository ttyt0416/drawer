use super::*;
fn check(b: bool, s: &str) -> Result<(), String> {
    if b { Ok(()) } else { Err(s.into()) }
}
pub(crate) fn run(root: &Path) -> Result<(), String> {
    std::fs::create_dir_all(root).map_err(|e| e.to_string())?;
    let dir = root.join("background-save-ui");
    std::fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let result = (|| -> Result<(), String> {
        drawer_core::recovery::generations_verify::run(&dir)?;
        let ctx = egui::Context::default();
        let storage = UserStorage::at(dir.join("data")).map_err(|e| e.to_string())?;
        let mut app = Drawer::new(&ctx, None).with_storage(storage.clone());
        app.create_canvas(CanvasSpec {
            width: 8,
            height: 8,
            dpi: 300.,
            background: drawer_core::document::Background::Transparent,
        })
        .map_err(|e| e.to_string())?;
        input_replay::frame(&ctx, &mut app, 0.1, vec![]);
        let path = dir.join("normal.drawer");
        app.save_to(0, &path)?;
        app.documents[0].edit_pixel(0, 0, 0, [1, 2, 3, 255]);
        let frozen = app.documents[0].snapshot();
        let (tx, rx) = std::sync::mpsc::channel();
        app.queue_native_save(0, &path, Some(rx))?;
        app.documents[0].edit_pixel(0, 1, 0, [4, 5, 6, 255]);
        let later = app.documents[0].snapshot();
        check(
            app.save_in_background(0, &path).is_err() && app.save_to(0, &path).is_err(),
            "Overlapping save was allowed",
        )?;
        tx.send(()).map_err(|e| e.to_string())?;
        app.drain_background();
        check(
            app.documents[0].dirty
                && app.documents[0].snapshot() == later
                && drawer_core::files::open(&path, DEFAULT_DOCUMENT_BUDGET)
                    .map_err(|e| e.to_string())?
                    .snapshot()
                    == frozen,
            "Background save marked later edits clean or wrote a mutable revision",
        )?;
        app.save_in_background(0, &path)?;
        app.drain_background();
        check(
            !app.documents[0].dirty
                && drawer_core::files::open(&path, DEFAULT_DOCUMENT_BUDGET)
                    .map_err(|e| e.to_string())?
                    .snapshot()
                    == later,
            "Newer save did not publish latest state",
        )?;
        let normal = std::fs::read(&path).map_err(|e| e.to_string())?;
        app.documents[0].edit_pixel(0, 2, 0, [7, 8, 9, 255]);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            let lock = std::fs::OpenOptions::new()
                .read(true)
                .share_mode(0)
                .open(&path)
                .map_err(|e| e.to_string())?;
            app.queue_native_save(0, &path, None)?;
            app.drain_background();
            drop(lock);
            check(
                app.documents[0].dirty
                    && app.status.as_ref().is_some_and(|s| s.contains("failed"))
                    && std::fs::read(&path).map_err(|e| e.to_string())? == normal,
                "Background failure changed normal file/dirty state",
            )?;
        }
        app.resources.current.recovery_commands = 1;
        app.resources.current.recovery_seconds = 3600;
        app.resources.current.recovery_keep = 2;
        app.resources
            .current
            .save(&storage)
            .map_err(|e| e.to_string())?;
        app.init_recovery(storage.root());
        app.recovery.last = Instant::now() - Duration::from_secs(2);
        let captured = app.documents[0].snapshot();
        input_replay::frame(&ctx, &mut app, 0.2, vec![]);
        check(
            app.background.busy(),
            "Command interval did not queue a real worker",
        )?;
        app.documents[0].edit_pixel(0, 3, 0, [10, 11, 12, 255]);
        let latest = app.documents[0].snapshot();
        app.drain_background();
        check(
            app.documents[0].dirty && app.documents[0].snapshot() == latest,
            "Recovery changed current edits",
        )?;
        app.quiesce_recovery();
        let old = core::scan(storage.root())?;
        check(
            old.iter()
                .any(|c| !c.project && c.open().is_ok_and(|d| d.snapshot() == captured)),
            "Recovery did not retain captured command revision",
        )?;
        app.resume_recovery();
        app.quiesce_recovery();
        let all = core::scan(storage.root())?;
        check(
            all.iter()
                .any(|c| !c.project && c.open().is_ok_and(|d| d.snapshot() == latest)),
            "Restart/retry missed later edits",
        )?;
        check(
            std::fs::read(&path).map_err(|e| e.to_string())? == normal,
            "Recovery overwrote native source",
        )?;
        let other = dir.join("other.drawer");
        let mut d = Document::create(
            CanvasSpec {
                width: 2,
                height: 2,
                dpi: 300.,
                background: drawer_core::document::Background::Transparent,
            },
            DEFAULT_DOCUMENT_BUDGET,
        )
        .map_err(|e| e.to_string())?;
        drawer_core::files::save(&mut d, &other).map_err(|e| e.to_string())?;
        app.set_project_structure_work(drawer_core::story_text::Work::new(
            "Background project".into(),
            vec![path.clone(), other],
        )?)?;
        app.init_recovery(storage.root());
        app.checkpoint_recovery()?;
        app.quiesce_recovery();
        let candidate = core::scan(storage.root())?
            .into_iter()
            .find(|c| c.project)
            .ok_or("Background project candidate missing")?;
        let work = candidate.open_project()?;
        let guard_ctx = egui::Context::default();
        let mut guard = Drawer::new(&guard_ctx, None);
        guard.set_project_structure_work(work.clone())?;
        guard.open_path(&work.pages[0])?;
        let protected = std::fs::read(&work.pages[0]).map_err(|e| e.to_string())?;
        check(
            guard.save_in_background(0, &work.pages[0]).is_err()
                && std::fs::read(&work.pages[0]).map_err(|e| e.to_string())? == protected,
            "Individual background save overwrote a published project generation",
        )?;
        drop(guard);
        let ctx2 = egui::Context::default();
        let mut recovered = Drawer::new(&ctx2, None)
            .with_storage(UserStorage::at(dir.join("recovered-data")).map_err(|e| e.to_string())?);
        recovered.open_recovered_project(work.clone())?;
        check(
            recovered.project_needs_recovery()
                && recovered.project_uses_recovery_session(&candidate.session),
            "Recovered project lost unsaved/backing identity",
        )?;
        recovered.open_path(&work.pages[0])?;
        check(
            recovered.documents[0].path.is_none() && recovered.documents[0].dirty,
            "Recovered lazy page retained overwrite destination",
        )?;
        recovered.recovery.open = true;
        recovered.recovery.candidates = vec![candidate.clone()];
        recovered.recovery.confirm = Some(candidate.session.clone());
        for n in 1..=3 {
            input_replay::frame(&ctx2, &mut recovered, f64::from(n) * 0.1, vec![]);
        }
        let p = recovered
            .recovery
            .accept
            .ok_or("Project recovery discard confirmation absent")?
            .center();
        input_replay::frame(
            &ctx2,
            &mut recovered,
            0.4,
            vec![
                egui::Event::PointerMoved(p),
                egui::Event::PointerButton {
                    pos: p,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: Default::default(),
                },
                egui::Event::PointerButton {
                    pos: p,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: Default::default(),
                },
            ],
        );
        check(
            candidate.session.exists()
                && recovered
                    .recovery
                    .error
                    .as_ref()
                    .is_some_and(|s| s.contains("backing recovery session")),
            "Discard removed lazy recovered project backing files",
        )?;
        drop(recovered);
        drop(app);
        Ok(())
    })();
    std::fs::remove_dir_all(dir).map_err(|e| e.to_string())?;
    result
}
#[cfg(test)]
mod tests {
    #[test]
    fn captured_revision_serialization_failures_command_recovery_and_project_backing() {
        let root =
            std::env::temp_dir().join(format!("drawer-background-save-ui-{}", std::process::id()));
        std::fs::create_dir(&root).unwrap();
        let r = super::run(&root);
        std::fs::remove_dir_all(root).unwrap();
        r.unwrap();
    }
}
