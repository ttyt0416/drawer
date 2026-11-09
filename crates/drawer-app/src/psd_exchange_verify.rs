use super::*;
pub(super) fn run(root: &std::path::Path) -> Result<(), String> {
    core::verify::verify(root)?;
    let dir = root.join("psd-exchange-ui");
    std::fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let result = (|| {
        let ctx = egui::Context::default();
        let mut app = Drawer::new(&ctx, None);
        app.simple_view = true;
        app.add_document(core::verify::fixture()?);
        let original = app.documents[0].snapshot();
        let history = app.documents[0].history_counts();
        let path = dir.join("ui.psd");
        app.begin_psd_exchange(path.clone())?;
        let mut time = 0.;
        let frame = |app: &mut Drawer, t: &mut f64, events| {
            *t += 0.05;
            input_replay::frame(&ctx, app, *t, events);
        };
        let click = |app: &mut Drawer, t: &mut f64, key: &str| -> Result<(), String> {
            for _ in 0..3 {
                frame(app, t, vec![]);
            }
            let r = app
                .textures
                .export
                .as_ref()
                .and_then(|e| e.controls.get(key))
                .copied()
                .ok_or_else(|| format!("Missing PSD control {key}"))?;
            if r.center().y > 690. {
                return Err(format!("Clipped PSD control {key}"));
            }
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
            for _ in 0..3 {
                frame(app, t, vec![]);
            }
            Ok(())
        };
        for _ in 0..300 {
            frame(&mut app, &mut time, vec![]);
            if app
                .textures
                .export
                .as_ref()
                .is_some_and(|e| e.preparing.is_none())
            {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        let e = app.textures.export.as_ref().ok_or("No PSD preview")?;
        if e.plan.is_none() || path.exists() {
            return Err(format!("Preview failed or wrote a file: {}", e.message));
        }
        click(&mut app, &mut time, "export")?;
        if path.exists() {
            return Err("Unconfirmed export button wrote a file".into());
        }
        click(&mut app, &mut time, "confirm")?;
        if !app.textures.export.as_ref().is_some_and(|e| e.confirmed) {
            return Err("Pointer confirmation did not change the checkbox".into());
        }
        click(&mut app, &mut time, "export")?;
        for _ in 0..300 {
            frame(&mut app, &mut time, vec![]);
            if app.textures.export.is_none() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        if !path.exists() || app.textures.export.is_some() {
            return Err(format!(
                "Confirmed pointer export failed: {:?}",
                app.textures.export.as_ref().map(|e| &e.message)
            ));
        }
        let (d, w) = drawer_core::files::open_report(&path, DEFAULT_DOCUMENT_BUDGET)
            .map_err(|e| e.to_string())?;
        if d.layers.len() != 3 || !w.is_empty() {
            return Err("Open did not preserve PSD structure".into());
        }
        let canceled = dir.join("cancel.psb");
        app.begin_psd_exchange(canceled.clone())?;
        frame(&mut app, &mut time, vec![]);
        click(&mut app, &mut time, "cancel")?;
        for _ in 0..300 {
            frame(&mut app, &mut time, vec![]);
            if app.textures.export.is_none() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        if canceled.exists() || app.textures.export.is_some() {
            return Err("Canceled PSD job was published / retained".into());
        }
        app.begin_psd_exchange(dir.join("stale.psd"))?;
        for _ in 0..300 {
            frame(&mut app, &mut time, vec![]);
            if app
                .textures
                .export
                .as_ref()
                .is_some_and(|e| e.plan.is_some())
            {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        let mut e = app.textures.export.take().ok_or("Missing stale preview")?;
        e.confirmed = true;
        app.documents[0].revision += 1;
        if app.write_psd_exchange(&mut e).is_ok() {
            return Err("Stale PSD preview exported".into());
        }
        app.documents[0].revision -= 1;
        if app.documents[0].snapshot() != original
            || app.documents[0].history_counts() != history
            || app.documents[0].path.is_some()
        {
            return Err("PSD export changed source state / history / path".into());
        }
        Ok(())
    })();
    result.and(std::fs::remove_dir_all(dir).map_err(|e| e.to_string()))
}
