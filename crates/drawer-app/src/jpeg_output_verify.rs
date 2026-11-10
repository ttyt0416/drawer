use super::*;
pub(super) fn run(root: &std::path::Path) -> Result<(), String> {
    drawer_core::jpeg_output::verify::run(root)?;
    let dir = root.join("jpeg-output-ui");
    std::fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let result = (|| {
        let ctx = egui::Context::default();
        let mut a = Drawer::new(&ctx, None);
        a.simple_view = true;
        a.add_document(drawer_core::jpeg_output::verify::fixture()?);
        let before = a.documents[0].snapshot();
        let identity = (
            a.documents[0].path.clone(),
            a.documents[0].dirty,
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
                if a.raster_export.encoded.is_some() && a.raster_export.job.is_none() {
                    return Ok(());
                }
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            Err(format!(
                "Actual JPEG preview timeout: {}",
                a.raster_export.message
            ))
        };
        let click = |a: &mut Drawer, t: &mut f64, key: &str| -> Result<(), String> {
            for _ in 0..3 {
                frame(a, t, vec![]);
            }
            let r = *a
                .raster_export
                .controls
                .get(key)
                .ok_or("Missing JPEG control")?;
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
        let out = dir.join("reviewed.jpg");
        a.raster_export.path = out.display().to_string();
        wait(&mut a, &mut t)?;
        click(&mut a, &mut t, "confirm")?;
        if !a.raster_export.confirmed {
            return Err("JPEG loss confirmation failed".into());
        }
        let old = a.raster_export.encoded.as_ref().unwrap().bytes().to_vec();
        click(&mut a, &mut t, "420")?;
        if a.raster_export.confirmed {
            return Err("JPEG sampling change retained old confirmation".into());
        }
        wait(&mut a, &mut t)?;
        let encoded = a.raster_export.encoded.as_ref().unwrap();
        let bytes = encoded.bytes().to_vec();
        if drawer_core::jpeg_output::verify::header(&bytes)?.0 != 34
            || bytes == old
            || a.raster_export.preview.is_none()
        {
            return Err("Pointer sampling did not reencode actual preview".into());
        }
        click(&mut a, &mut t, "confirm")?;
        click(&mut a, &mut t, "export")?;
        if !out.exists()
            || std::fs::read(&out).map_err(|e| e.to_string())? != bytes
            || a.raster_export.open
        {
            return Err(format!(
                "Cached JPEG publication failed: {}",
                a.raster_export.message
            ));
        }
        a.begin_raster_export()?;
        let canceled = dir.join("cancel.jpg");
        a.raster_export.path = canceled.display().to_string();
        wait(&mut a, &mut t)?;
        click(&mut a, &mut t, "cancel")?;
        if canceled.exists()
            || a.documents[0].snapshot() != before
            || identity
                != (
                    a.documents[0].path.clone(),
                    a.documents[0].dirty,
                    a.documents[0].history_counts(),
                )
        {
            return Err("JPEG preview/export/cancel changed original".into());
        }
        Ok(())
    })();
    drawer_core::asset_catalog::cleanup_verification(root, &dir)?;
    result
}
#[cfg(test)]
mod tests {
    #[test]
    fn actual_jpeg_pointer_sampling_reencode_confirm_cached_write_and_cancel() {
        let t = crate::tests::TestDirectory::new();
        super::run(&t.0).unwrap();
    }
}
