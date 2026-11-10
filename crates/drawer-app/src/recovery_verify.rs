use super::*;
use std::path::Path;
fn check(b: bool, s: &str) -> Result<(), String> {
    if b { Ok(()) } else { Err(s.into()) }
}
pub(crate) fn run(root: &Path) -> Result<(), String> {
    std::fs::create_dir_all(root).map_err(|e| e.to_string())?;
    core::verify::run(root)?;
    let dir = root.join("recovery-ui-fixture");
    std::fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let result = (|| {
        let storage = UserStorage::at(dir.join("data")).map_err(|e| e.to_string())?;
        let mut document = core::verify::fixture()?;
        let source = dir.join("original.drawer");
        drawer_core::files::save(&mut document, &source).map_err(|e| e.to_string())?;
        let bytes = std::fs::read(&source).map_err(|e| e.to_string())?;
        document.edit_pixel(0, 0, 0, [100, 110, 120, 255]);
        let expected = document.snapshot();
        let mut previous = Session::create(storage.root())?;
        previous.checkpoint(&[&document])?;
        drop(previous);
        let ctx = egui::Context::default();
        let mut app = Drawer::new(&ctx, None).with_storage(storage.clone());
        app.add_document(document);
        app.init_recovery(storage.root());
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
        let cancel = app
            .recovery
            .cancel
            .ok_or("Recovery cancel button missing")?;
        click(&mut app, &mut time, cancel);
        check(
            !app.recovery.open
                && app.documents.len() == 1
                && core::scan(storage.root())?.len() == 1,
            "Recovery cancel changed originals or removed snapshots",
        )?;
        app.recovery.open = true;
        settle(&mut app, &mut time);
        let recover = app.recovery.recover.ok_or("Recovery open button missing")?;
        click(&mut app, &mut time, recover);
        check(
            app.documents.len() == 2
                && app.documents[1].snapshot() == expected
                && app.documents[1].path.is_none()
                && app.documents[1].dirty
                && app.documents[1].history_counts() == (0, 0),
            "Recovery pointer did not open a separate unsaved document",
        )?;
        check(
            app.documents[0].snapshot() == expected
                && app.documents[0].history_counts() == (2, 0)
                && std::fs::read(&source).map_err(|e| e.to_string())? == bytes,
            "Recovery changed original drawing/history/file",
        )?;
        let discard = app
            .recovery
            .discard
            .ok_or("Recovery discard review missing")?;
        click(&mut app, &mut time, discard);
        check(
            app.recovery.confirm.is_some() && core::scan(storage.root())?.len() == 1,
            "Discard bypassed confirmation",
        )?;
        let accept = app
            .recovery
            .accept
            .ok_or("Recovery discard confirmation missing")?;
        click(&mut app, &mut time, accept);
        check(
            app.recovery.candidates.is_empty()
                && core::scan(storage.root())?.is_empty()
                && app.documents.len() == 2
                && app.documents[1].snapshot() == expected,
            "Discard removed recovered/current documents",
        )?;
        app.checkpoint_recovery()?;
        app.documents[1].edit_pixel(0, 2, 0, [210, 100, 80, 255]);
        let timed = app.documents[1].snapshot();
        app.recovery.last = std::time::Instant::now() - std::time::Duration::from_secs(61);
        step(&mut app, &mut time, vec![]);
        let session = app
            .recovery
            .session
            .take()
            .ok_or("Current recovery session missing")?;
        let directory = session.directory.clone();
        drop(session);
        let candidates = core::scan(storage.root())?;
        check(
            candidates.len() == 2 && candidates.iter().all(|c| c.error.is_none()),
            "Explicit checkpoint missed dirty open documents",
        )?;
        check(
            candidates
                .iter()
                .any(|c| c.open().is_ok_and(|d| d.snapshot() == timed)),
            "Elapsed interval did not checkpoint the newer dirty edit",
        )?;
        core::discard(&directory)?;
        Ok(())
    })();
    std::fs::remove_dir_all(&dir).map_err(|e| e.to_string())?;
    result
}
#[cfg(test)]
mod tests {
    #[test]
    fn pointer_recovery_cancel_separate_unsaved_open_confirmed_discard_and_checkpoint() {
        let root = crate::tests::TestDirectory::new();
        super::run(&root.0.join("first-run-parent")).unwrap();
    }
}
