use super::*;
use std::path::Path;
fn check(b: bool, s: &str) -> Result<(), String> {
    if b { Ok(()) } else { Err(s.into()) }
}
pub(crate) fn run(root: &Path) -> Result<(), String> {
    std::fs::create_dir_all(root).map_err(|e| e.to_string())?;
    drawer_core::saved_revision::verify::run(root)?;
    let dir = root.join("save-status-ui");
    std::fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let result = (|| {
        let ctx = egui::Context::default();
        let storage = UserStorage::at(dir.join("data")).map_err(|e| e.to_string())?;
        let mut app = Drawer::new(&ctx, None).with_storage(storage.clone());
        app.add_document(drawer_core::recovery::verify::fixture()?);
        let path = dir.join("current.drawer");
        app.save_to(0, &path)?;
        let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
        app.documents[0].edit_pixel(0, 0, 0, [60, 80, 100, 255]);
        app.add_document(drawer_core::recovery::verify::fixture()?);
        let other = app.documents[1].snapshot();
        let other_id = app.documents[1].id;
        let other_history = app.documents[1].history_counts();
        app.select_tab(0);
        app.init_recovery(storage.root());
        app.checkpoint_recovery()?;
        let mut time = 0.;
        let step = |app: &mut Drawer, time: &mut f64, events| {
            *time += 0.05;
            input_replay::frame(&ctx, app, *time, events);
        };
        let settle = |app: &mut Drawer, time: &mut f64| {
            for _ in 0..3 {
                step(app, time, vec![]);
            }
        };
        let click = |app: &mut Drawer, time: &mut f64, r: egui::Rect| {
            for pressed in [true, false] {
                step(
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
        };
        settle(&mut app, &mut time);
        let before = (
            app.documents[0].snapshot(),
            app.documents[0].history_counts(),
            app.documents[0].path.clone(),
            app.documents[0].view,
            app.documents[0].dirty,
        );
        app.request_close(0);
        settle(&mut app, &mut time);
        let r = app.save_status.close[2].ok_or("Close Cancel button missing")?;
        click(&mut app, &mut time, r);
        check(
            app.pending_close.is_none()
                && app.documents.len() == 2
                && (
                    app.documents[0].snapshot(),
                    app.documents[0].history_counts(),
                    app.documents[0].path.clone(),
                    app.documents[0].view,
                    app.documents[0].dirty,
                ) == before
                && std::fs::read(&path).map_err(|e| e.to_string())? == bytes,
            "Close cancel changed drawing/selection/path/view/history/file",
        )?;
        let checkpoint = app.documents[0].begin_save_checkpoint()?;
        let mut frozen = app.documents[0].current_copy();
        app.documents[0].edit_pixel(0, 2, 0, [160, 180, 200, 255]);
        drawer_core::files::save(&mut frozen, &path).map_err(|e| e.to_string())?;
        check(
            !app.documents[0]
                .complete_save_checkpoint(checkpoint, frozen.path.as_ref().unwrap())?
                && app.documents[0].dirty,
            "Background completion removed dirty marker after a newer stroke",
        )?;
        check(
            app.documents[0].undo()
                && !app.documents[0].dirty
                && app.documents[0].redo()
                && app.documents[0].dirty,
            "Saved-state Undo/Redo dirty marker",
        )?;
        let before = app.documents[0].snapshot();
        let saved = std::fs::read(&path).map_err(|e| e.to_string())?;
        let temp = dir.join("current.drawer.drawer-write.tmp");
        std::fs::write(&temp, b"foreign save").map_err(|e| e.to_string())?;
        app.request_close(0);
        settle(&mut app, &mut time);
        let r = app.save_status.close[0].ok_or("Close Save button missing")?;
        click(&mut app, &mut time, r);
        check(
            app.pending_close.is_some()
                && app.documents.len() == 2
                && app.documents[0].snapshot() == before
                && app.documents[0].dirty
                && std::fs::read(&path).map_err(|e| e.to_string())? == saved,
            "Failed save closed or changed a document",
        )?;
        std::fs::remove_file(&temp).map_err(|e| e.to_string())?;
        app.status = None;
        settle(&mut app, &mut time);
        let r = app.save_status.close[0].ok_or("Save retry button missing")?;
        click(&mut app, &mut time, r);
        check(
            app.documents.len() == 1
                && app.documents[0].id == other_id
                && drawer_core::files::open(&path, DEFAULT_DOCUMENT_BUDGET)
                    .map_err(|e| e.to_string())?
                    .snapshot()
                    == before,
            "Confirmed Save did not write latest state and close only that document",
        )?;
        app.add_document(drawer_core::recovery::verify::fixture()?);
        app.checkpoint_recovery()?;
        let last = app.tabs.len() - 1;
        app.request_close(last);
        settle(&mut app, &mut time);
        let r = app.save_status.close[1].ok_or("Close Discard button missing")?;
        click(&mut app, &mut time, r);
        check(
            app.documents.len() == 1
                && app.documents[0].id == other_id
                && app.documents[0].snapshot() == other
                && app.documents[0].history_counts() == other_history,
            "Discard affected another document/history",
        )?;
        app.quiesce_recovery();
        let candidates = drawer_core::recovery::scan(storage.root())?;
        check(
            candidates.len() == 1 && candidates[0].open()?.snapshot() == other,
            "Saved/discarded documents reappeared as unresolved recovery",
        )?;
        drawer_core::recovery::discard(&candidates[0].session)?;
        Ok(())
    })();
    std::fs::remove_dir_all(&dir).map_err(|e| e.to_string())?;
    result
}
#[cfg(test)]
mod tests {
    #[test]
    fn pointer_close_cancel_failed_save_retry_discard_dirty_checkpoint_and_recovery_retirement() {
        let root = crate::tests::TestDirectory::new();
        super::run(&root.0.join("fresh")).unwrap();
    }
}
