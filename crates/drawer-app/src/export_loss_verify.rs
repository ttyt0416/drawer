use super::*;
pub(super) fn run(root: &std::path::Path) -> Result<(), String> {
    drawer_core::export_loss::verify::run(root)?;
    let dir = root.join("export-loss-ui");
    std::fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let result = (|| {
        let ctx = egui::Context::default();
        let mut a = Drawer::new(&ctx, None);
        a.create_canvas(CanvasSpec {
            width: 2,
            height: 1,
            dpi: 300.,
            background: Background::Color([40, 50, 60]),
        })
        .map_err(|e| e.to_string())?;
        let source = dir.join("original.drawer");
        drawer_core::files::save(&mut a.documents[0], &source).map_err(|e| e.to_string())?;
        let original = std::fs::read(&source).map_err(|e| e.to_string())?;
        let before = a.documents[0].snapshot();
        let identity = (
            a.documents[0].path.clone(),
            a.documents[0].dirty,
            a.documents[0].revision,
            a.documents[0].history_counts(),
        );
        let mut t = 0.;
        let frame = |a: &mut Drawer, t: &mut f64, events| {
            *t += 0.05;
            input_replay::frame(&ctx, a, *t, events);
        };
        let wait = |a: &mut Drawer, t: &mut f64| -> Result<(), String> {
            for _ in 0..4000 {
                frame(a, t, vec![]);
                if a.raster_export.job.is_none() {
                    return a
                        .raster_export
                        .prepared
                        .as_ref()
                        .map(|_| ())
                        .ok_or_else(|| a.raster_export.message.clone());
                }
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            Err("Export preview timeout".into())
        };
        let click = |a: &mut Drawer, t: &mut f64, key: &str| -> Result<(), String> {
            for _ in 0..3 {
                frame(a, t, vec![]);
            }
            let r = *a
                .raster_export
                .controls
                .get(key)
                .ok_or("Missing loss review control")?;
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
            Ok(())
        };
        a.begin_raster_export()?;
        a.raster_export.options = Options {
            format: Format::Tiff,
            alpha: true,
            ..Default::default()
        };
        let out = dir.join("no-loss.tif");
        a.raster_export.path = out.display().to_string();
        wait(&mut a, &mut t)?;
        frame(&mut a, &mut t, vec![]);
        if !a
            .raster_export
            .prepared
            .as_ref()
            .unwrap()
            .losses(a.raster_export.options)
            .is_empty()
            || a.raster_export.controls.contains_key("confirm")
        {
            return Err("Zero-loss output showed unnecessary loss confirmation".into());
        }
        click(&mut a, &mut t, "export")?;
        if !out.exists() || a.raster_export.open {
            return Err(format!(
                "Zero-loss output required confirmation: {}",
                a.raster_export.message
            ));
        }
        a.begin_raster_export()?;
        let canceled = dir.join("canceled.jpg");
        a.raster_export.path = canceled.display().to_string();
        wait(&mut a, &mut t)?;
        click(&mut a, &mut t, "export")?;
        if canceled.exists() {
            return Err("Unreviewed lossy output was written".into());
        }
        click(&mut a, &mut t, "cancel")?;
        if canceled.exists()
            || a.raster_export.open
            || a.documents[0].snapshot() != before
            || identity
                != (
                    a.documents[0].path.clone(),
                    a.documents[0].dirty,
                    a.documents[0].revision,
                    a.documents[0].history_counts(),
                )
            || std::fs::read(&source).map_err(|e| e.to_string())? != original
        {
            return Err("Export/cancel changed original path/dirty/history/bytes".into());
        }
        Ok(())
    })();
    drawer_core::asset_catalog::cleanup_verification(root, &dir)?;
    result
}
#[cfg(test)]
mod tests {
    #[test]
    fn zero_loss_no_warning_lossy_guard_and_cancel_preserve_original() {
        let t = crate::tests::TestDirectory::new();
        super::run(&t.0).unwrap();
    }
}
