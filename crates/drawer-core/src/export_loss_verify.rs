use crate::{
    color_precision::Depth,
    document::{Background, CanvasSpec, DEFAULT_DOCUMENT_BUDGET as B, Document},
    raster_io::{self, Format, Options},
};
use std::{fs, path::Path, sync::atomic::AtomicBool};
pub fn run(root: &Path) -> Result<(), String> {
    let dir = root.join("export-loss-core");
    fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let result = (|| {
        let cancel = AtomicBool::new(false);
        let mut d = Document::create(
            CanvasSpec {
                width: 2,
                height: 1,
                dpi: 300.,
                background: Background::Color([20, 30, 40]),
            },
            B,
        )
        .map_err(|e| e.to_string())?;
        let source = dir.join("source.drawer");
        crate::files::save(&mut d, &source).map_err(|e| e.to_string())?;
        let bytes = fs::read(&source).map_err(|e| e.to_string())?;
        let identity = (
            d.path.clone(),
            d.source_path.clone(),
            d.dirty,
            d.revision,
            d.history_counts(),
        );
        let before = d.snapshot();
        let o = Options {
            format: Format::Tiff,
            alpha: true,
            ..Default::default()
        };
        let p = raster_io::prepare(&d, B, &cancel)?;
        if !p.losses(o).is_empty() {
            return Err(format!(
                "Unnecessary warning for one opaque RGB8 raster: {:?}",
                p.losses(o)
            ));
        }
        let path = dir.join("lossless.tif");
        p.write(&d, &path, o, false, false, B, &cancel)?;
        if !path.exists() {
            return Err("Zero-loss output incorrectly required loss confirmation".into());
        }
        let mut effect = d.clone();
        effect.scene.appearance.rasters.insert(
            effect.scene.layer_ids.id(0),
            crate::appearance::Style {
                enabled: true,
                border: 1,
                ..Default::default()
            },
        );
        if !super::Source::new(&effect)?
            .losses()
            .iter()
            .any(|s| s.starts_with("Layer flattening:"))
        {
            return Err("Single-raster editable appearance was treated as lossless".into());
        }
        let jpeg = Options::default();
        if !p.losses(jpeg).iter().any(|s| s.starts_with("Compression:"))
            || p.losses(jpeg).iter().any(|s| s.starts_with("Alpha:"))
        {
            return Err("Lossy codec/opaque alpha report incorrect".into());
        }
        let rejected = dir.join("unconfirmed.jpg");
        if p.write(&d, &rejected, jpeg, false, false, B, &cancel)
            .is_ok()
            || rejected.exists()
        {
            return Err("Lossy output bypassed loss review".into());
        }
        let canceled = dir.join("canceled.tif");
        if p.write(&d, &canceled, o, false, false, B, &AtomicBool::new(true))
            .is_ok()
            || canceled.exists()
        {
            return Err("Canceled output created a file".into());
        }
        if d.snapshot() != before
            || identity
                != (
                    d.path.clone(),
                    d.source_path.clone(),
                    d.dirty,
                    d.revision,
                    d.history_counts(),
                )
            || fs::read(&source).map_err(|e| e.to_string())? != bytes
        {
            return Err("Export changed saved source identity/dirty/history/bytes".into());
        }
        d.dirty = true;
        let dirty = d.dirty;
        let p = raster_io::prepare(&d, B, &cancel)?;
        p.write(&d, &dir.join("dirty.tif"), o, false, false, B, &cancel)?;
        if d.dirty != dirty
            || d.path.as_ref() != Some(&source.canonicalize().map_err(|e| e.to_string())?)
        {
            return Err("Export reset dirty/source path".into());
        }
        d.layers[0].pixels[0] = [20, 30, 40, 128];
        d.layers.push(d.layers[0].clone());
        let p = raster_io::prepare(&d, B, &cancel)?;
        let loss = p.losses(jpeg);
        if !loss.iter().any(|s| s.starts_with("Layer flattening:"))
            || !loss.iter().any(|s| s.starts_with("Alpha:"))
        {
            return Err("Layers/alpha loss omitted".into());
        }
        d.scene.objects.push(crate::scene::PageObject::Text {
            layout: None,
            baseline: [0., 0.],
            text: "Editable".into(),
            size_px: 8.,
            color: [0, 0, 0, 255],
        });
        if !super::Source::new(&d)?
            .losses()
            .iter()
            .any(|s| s.starts_with("Text:"))
        {
            return Err("Editable text loss omitted".into());
        }
        let d16 = Document::create_with_color(
            CanvasSpec {
                width: 1,
                height: 1,
                dpi: 300.,
                background: Background::Transparent,
            },
            B,
            crate::color_storage::Config {
                depth: Depth::Sixteen,
                ..Default::default()
            },
        )
        .map_err(|e| e.to_string())?;
        let p = raster_io::prepare(&d16, B, &cancel)?;
        if !p.losses(o).iter().any(|s| s.starts_with("Bit depth:"))
            || p.losses(Options {
                depth: Depth::Sixteen,
                ..o
            })
            .iter()
            .any(|s| s.starts_with("Bit depth:"))
        {
            return Err("Bit-depth report did not follow actual output settings".into());
        }
        let mut wide = d16;
        // Alter gamma through the CMS model to create a real different working space.
        let mut profile = moxcms::ColorProfile::new_srgb();
        profile.red_trc = Some(moxcms::ToneReprCurve::Parametric(vec![1.]));
        profile.green_trc = profile.red_trc.clone();
        profile.blue_trc = profile.red_trc.clone();
        wide.color.profile = crate::color_profile::RgbProfile::embedded(
            "Linear".into(),
            profile.encode().map_err(|e| e.to_string())?,
        )?;
        let p = raster_io::prepare(&wide, B, &cancel)?;
        if !p.losses(o).iter().any(|s| s.starts_with("Color space:")) {
            return Err("Different working-space loss omitted".into());
        }
        Ok(())
    })();
    crate::asset_catalog::cleanup_verification(root, &dir)?;
    result
}
#[cfg(test)]
mod tests {
    #[test]
    fn content_dependent_loss_review_zero_loss_and_source_preservation() {
        let t = crate::test_support::TempDir::new();
        super::run(&t.0).unwrap();
    }
}
