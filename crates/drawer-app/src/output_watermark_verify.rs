use super::*;
pub(super) fn run(root: &Path) -> Result<(), String> {
    let dir = root.join("watermark-output-ui");
    std::fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let result = (|| {
        let ctx = egui::Context::default();
        let storage = UserStorage::at(dir.join("user")).map_err(|e| e.to_string())?;
        let mut settings = drawer_core::output_watermark::Settings {
            enabled: true,
            text: "©".into(),
            size: 12.,
            scale: [0.5; 2],
            anchor: drawer_core::output_watermark::Anchor::Center,
            margin: [0.; 2],
            noise: true,
            strength: 0.05,
            ..Default::default()
        };
        settings.save(&storage)?;
        let preset = std::fs::read(storage.root().join("settings/output-watermark.json"))
            .map_err(|e| e.to_string())?;
        let mut app = Drawer::new(&ctx, None).with_storage(storage);
        app.simple_view = true;
        app.add_document(drawer_core::raster_io::verify::fixture()?);
        let original = app.documents[0].snapshot();
        let history = app.documents[0].history_counts();
        app.begin_output_batch()?;
        if app.output_batch.settings.watermark != settings {
            return Err("Saved watermark was not loaded in output UI".into());
        }
        app.output_batch.folder = dir.display().to_string();
        app.output_batch.settings.pattern = "review-{index}".into();
        let mut t = 0.;
        let frame = |a: &mut Drawer, t: &mut f64, e| {
            *t += 0.05;
            input_replay::frame(&ctx, a, *t, e);
        };
        let click = |a: &mut Drawer, t: &mut f64, key: &str| -> Result<(), String> {
            for _ in 0..3 {
                frame(a, t, vec![]);
            }
            let pos = a
                .output_batch
                .controls
                .get(key)
                .ok_or("Missing output control")?
                .center();
            for pressed in [true, false] {
                frame(
                    a,
                    t,
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
            Ok(())
        };
        click(&mut app, &mut t, "prepare")?;
        for _ in 0..500 {
            frame(&mut app, &mut t, vec![]);
            if app.output_batch.job.is_none() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        let plan =
            app.output_batch.plan.as_ref().ok_or_else(|| {
                format!("Watermarked preview failed: {}", app.output_batch.message)
            })?;
        let reviewed = plan.files[0].bytes.clone();
        let path = plan.files[0].path.clone();
        if !plan.files[0]
            .notices
            .iter()
            .any(|v| v.contains("Output-only watermark"))
        {
            return Err("Watermark not routed to output plan".into());
        }
        app.output_batch.reviewed = true;
        click(&mut app, &mut t, "write")?;
        for _ in 0..500 {
            frame(&mut app, &mut t, vec![]);
            if app.output_batch.job.is_none() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        if std::fs::read(&path).map_err(|e| e.to_string())? != reviewed {
            return Err("Watermark written bytes differ from review".into());
        }
        click(&mut app, &mut t, "close")?;
        app.begin_output_batch()?;
        app.output_batch.settings.watermark.opacity = 0.17;
        click(&mut app, &mut t, "close")?;
        if std::fs::read(
            app.storage
                .as_ref()
                .unwrap()
                .root()
                .join("settings/output-watermark.json"),
        )
        .map_err(|e| e.to_string())?
            != preset
        {
            return Err("Canceled preset overwrote saved preferences".into());
        }
        if app.documents[0].snapshot() != original || app.documents[0].history_counts() != history {
            return Err("Watermark UI changed document".into());
        }
        settings.enabled = false;
        let remembered = settings.clone();
        let _ = ctx.run_ui(Default::default(), |ui| {
            crate::output_watermark::editor(ui, &mut settings, &mut String::new());
        });
        if settings != remembered {
            return Err("Disabled controls changed preserved inputs".into());
        }
        Ok(())
    })();
    result.and(drawer_core::asset_catalog::cleanup_verification(root, &dir))
}
#[cfg(test)]
mod tests {
    #[test]
    fn watermarked_output_actual_pointer_preset_cancel_and_source_isolation() {
        let t = crate::tests::TestDirectory::new();
        super::run(&t.0).unwrap();
    }
}
