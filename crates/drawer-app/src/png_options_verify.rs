use super::*;
pub(super) fn run(root: &std::path::Path) -> Result<(), String> {
    drawer_core::png_io_verify::verify(root)?;
    let dir = root.join("png-options-ui");
    std::fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let result = (|| {
        let ctx = egui::Context::default();
        let mut app = Drawer::new(&ctx, None)
            .with_storage(UserStorage::at(dir.join("data")).map_err(|e| e.to_string())?);
        app.simple_view = true;
        app.add_document(drawer_core::png_io_verify::fixture()?);
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
        let click = |a: &mut Drawer, t: &mut f64, key: &str| -> Result<(), String> {
            let r = a
                .png_options
                .controls
                .get(key)
                .copied()
                .ok_or_else(|| format!("Missing PNG option {key}"))?;
            if r.center().y > 690. {
                return Err(format!("Clipped PNG option {key}"));
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
        let before = app.documents[0].snapshot();
        let path = dir.join("output.png");
        app.begin_png_export()?;
        app.png_options.path = path.display().to_string();
        settle(&mut app, &mut time);
        click(&mut app, &mut time, "sixteen")?;
        click(&mut app, &mut time, "alpha")?;
        click(&mut app, &mut time, "export")?;
        if app.png_options.stamp.is_some() || !path.is_file() {
            return Err(format!(
                "PNG export did not finish: {}",
                app.png_options.error
            ));
        }
        let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
        if bytes[24] != 16 {
            return Err("PNG pointer selected wrong depth".into());
        }
        let opened =
            drawer_core::files::open(&path, DEFAULT_DOCUMENT_BUDGET).map_err(|e| e.to_string())?;
        if opened.layers[0]
            .precise
            .as_ref()
            .is_none_or(|p| p.iter().any(|p| p[3] != 65535))
        {
            return Err("PNG pointer opaque export retained transparency".into());
        }
        app.begin_png_export()?;
        app.png_options.path = path.display().to_string();
        settle(&mut app, &mut time);
        click(&mut app, &mut time, "export")?;
        if app.png_options.stamp.is_none()
            || app.png_options.error.is_empty()
            || std::fs::read(&path).map_err(|e| e.to_string())? != bytes
        {
            return Err("PNG replaced existing output without confirmation".into());
        }
        click(&mut app, &mut time, "replace")?;
        click(&mut app, &mut time, "export")?;
        if std::fs::read(&path).map_err(|e| e.to_string())?[24] != 8 {
            return Err("PNG confirmed replacement failed".into());
        }
        let canceled = dir.join("cancel.png");
        app.begin_png_export()?;
        app.png_options.path = canceled.display().to_string();
        settle(&mut app, &mut time);
        click(&mut app, &mut time, "cancel")?;
        if canceled.exists() || app.documents[0].snapshot() != before {
            return Err("PNG cancel/export modified original document".into());
        }
        app.begin_png_export()?;
        app.png_options.path = canceled.display().to_string();
        app.documents[0].revision += 1;
        settle(&mut app, &mut time);
        if app.png_options.stamp.is_some() || canceled.exists() {
            return Err("PNG stale document options retained".into());
        }
        let source = dir.join("sixteen-source.png");
        let pixels = drawer_core::png_io_verify::png16(&source)?;
        let source_bytes = std::fs::read(&source).map_err(|e| e.to_string())?;
        app.begin_import(&ctx, &source);
        let end = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while app.import_dialog.as_ref().is_some_and(|d| d.working()) {
            frame(&mut app, &mut time, vec![]);
            if std::time::Instant::now() > end {
                return Err("PNG import worker timeout".into());
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        settle(&mut app, &mut time);
        app.accept_import()?;
        let imported = &app.documents[app.active.ok_or("No imported PNG page")?];
        if imported.color.depth != Depth::Sixteen
            || imported.layers[0].precise.as_ref() != Some(&pixels)
        {
            return Err("Import dialog reduced PNG sixteen-bit detail".into());
        }
        if app.export_target(app.active.unwrap(), &source).is_ok()
            || std::fs::read(&source).map_err(|e| e.to_string())? != source_bytes
        {
            return Err("PNG imported source not protected".into());
        }
        Ok(())
    })();
    result.and(std::fs::remove_dir_all(dir).map_err(|e| e.to_string()))
}
