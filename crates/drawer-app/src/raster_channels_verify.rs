use super::*;
pub(super) fn run(root: &std::path::Path) -> Result<(), String> {
    drawer_core::raster_channels::verify::run(root)?;
    let dir = root.join("raster-channels-ui");
    std::fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let result = (|| {
        let ctx = egui::Context::default();
        let mut app = Drawer::new(&ctx, None);
        app.simple_view = true;
        app.add_document(core::verify::fixture()?);
        let before = app.documents[0].snapshot();
        let history = app.documents[0].history_counts();
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
                .raster_export
                .controls
                .get(key)
                .ok_or("Missing raster-channel UI control")?
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
            for _ in 0..3 {
                frame(a, t, vec![]);
            }
            Ok(())
        };
        let wait = |a: &mut Drawer, t: &mut f64| -> Result<(), String> {
            for _ in 0..500 {
                frame(a, t, vec![]);
                if a.raster_export.prepared.is_some() && a.raster_export.job.is_none() {
                    return Ok(());
                }
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            Err("Channel output preparation timeout".into())
        };
        app.begin_raster_export()?;
        app.raster_export.options.format = Format::Png;
        app.raster_export.options.depth = Depth::Sixteen;
        app.raster_export.options.alpha = true;
        app.raster_export.path = dir.join("gray.png").display().to_string();
        wait(&mut app, &mut t)?;
        click(&mut app, &mut t, "jpg")?;
        click(&mut app, &mut t, "png")?;
        wait(&mut app, &mut t)?;
        if app.raster_export.options.depth != Depth::Sixteen || !app.raster_export.options.alpha {
            return Err("Format switching discarded inactive16-bit/alpha input".into());
        }
        click(&mut app, &mut t, "grayscale")?;
        if !app.raster_export.options.grayscale {
            return Err("Grayscale pointer toggle failed".into());
        }
        if app.raster_export.controls.contains_key("confirm") {
            click(&mut app, &mut t, "confirm")?;
        }
        click(&mut app, &mut t, "export")?;
        let bytes = std::fs::read(dir.join("gray.png")).map_err(|e| {
            format!(
                "Gray export missing: {e}; confirmed={}, options={:?}, notice={}",
                app.raster_export.confirmed, app.raster_export.options, app.raster_export.message
            )
        })?;
        if bytes.len() < 33
            || !bytes.starts_with(b"\x89PNG\r\n\x1a\n")
            || bytes[24] != 16
            || bytes[25] != 4
        {
            return Err("UI GrayAlpha16 selection did not reach encoder".into());
        }
        app.begin_raster_export()?;
        app.raster_export.options.format = Format::Bmp;
        app.raster_export.options.alpha = true;
        app.raster_export.path = dir.join("alpha.bmp").display().to_string();
        wait(&mut app, &mut t)?;
        app.raster_export.confirmed = true;
        click(&mut app, &mut t, "export")?;
        let bytes = std::fs::read(dir.join("alpha.bmp"))
            .map_err(|e| format!("BMP export missing: {e}; {}", app.raster_export.message))?;
        if u32::from_le_bytes(bytes[14..18].try_into().unwrap()) != 124 {
            return Err("UI alpha BMP is not V5".into());
        }
        app.begin_raster_export()?;
        app.raster_export.path = dir.join("canceled.png").display().to_string();
        click(&mut app, &mut t, "cancel")?;
        if dir.join("canceled.png").exists()
            || app.documents[0].snapshot() != before
            || app.documents[0].history_counts() != history
        {
            return Err("Channel output/cancel changed live drawing".into());
        }
        Ok(())
    })();
    result.and(drawer_core::asset_catalog::cleanup_verification(root, &dir))
}
#[cfg(test)]
mod tests {
    #[test]
    fn grayscale_alpha16_bmp_v5_pointer_output_and_inactive_values() {
        let t = crate::tests::TestDirectory::new();
        super::run(&t.0).unwrap();
    }
}
