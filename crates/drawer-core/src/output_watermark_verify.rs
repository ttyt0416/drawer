use super::*;
use crate::document::{Background, CanvasSpec, DEFAULT_DOCUMENT_BUDGET as B, Document};
pub fn run(root: &std::path::Path) -> Result<(), String> {
    let dir = root.join("output-watermark-check");
    fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let result = (|| {
        let c = AtomicBool::new(false);
        let d = Document::create(
            CanvasSpec {
                width: 17,
                height: 13,
                dpi: 299.75,
                background: Background::Color([40, 90, 130]),
            },
            B,
        )
        .map_err(|e| e.to_string())?;
        let original = d.snapshot();
        let history = d.history_counts();
        let base = crate::raster_io::prepare(&d, B, &c)?;
        let mut s = Settings {
            enabled: true,
            source: Source::Image,
            embed: true,
            image: Some(Image {
                width: 2,
                height: 2,
                pixels: vec![
                    [255, 30, 10, 128],
                    [20, 250, 30, 255],
                    [0; 4],
                    [50, 80, 255, 90],
                ],
            }),
            anchor: Anchor::Center,
            margin: [0.; 2],
            ..Default::default()
        };
        for mode in Mode::ALL {
            for rotation in [0., 37., 90.] {
                s.mode = mode;
                s.rotation = rotation;
                s.repeat = true;
                s.pattern = Pattern::Staggered;
                s.spacing = [1.; 2];
                s.scale = [2., 1.];
                let mut a = base.clone();
                apply(&mut a, &s, false, B, &c)?;
                let mut b = base.clone();
                apply(&mut b, &s, false, B, &c)?;
                if a.pixels != b.pixels || (mode == Mode::Normal && a.pixels == base.pixels) {
                    return Err("Blend/repeat/rotation determinism failure".into());
                }
            }
        }
        let path = dir.join("local.png");
        let image = s.image.as_ref().unwrap();
        image::RgbaImage::from_raw(
            image.width,
            image.height,
            image.pixels.iter().flatten().copied().collect(),
        )
        .unwrap()
        .save_with_format(&path, image::ImageFormat::Png)
        .map_err(|e| e.to_string())?;
        let source_bytes = fs::read(&path).map_err(|e| e.to_string())?;
        s.path = Some(path.clone());
        s.embed = false;
        s.reload(B)?;
        let frozen = super::frozen(&s, B, &c)?;
        let mut small = base.resized(7, 5, 72.5, B, &c)?;
        apply(&mut small, &s, false, B, &c)?;
        let mut same = base.resized(7, 5, 72.5, B, &c)?;
        apply(&mut same, &frozen, false, B, &c)?;
        if small.pixels != same.pixels
            || fs::read(&path).map_err(|e| e.to_string())? != source_bytes
        {
            return Err("Reload/frozen image or source preservation failed".into());
        }
        fs::remove_file(&path).map_err(|e| e.to_string())?;
        let mut retained = base.clone();
        apply(&mut retained, &frozen, false, B, &c)?;
        s.enabled = false;
        s.path = Some(dir.join("missing.png"));
        s.embed = false;
        s.noise = false;
        let remembered = s.clone();
        let mut out = base.clone();
        apply(&mut out, &s, false, B, &c)?;
        if out.pixels != base.pixels || s != remembered {
            return Err("Disabled watermark applied or overwrote manual values".into());
        }
        s.enabled = true;
        s.opacity = 0.;
        apply(&mut out, &s, false, B, &c)?;
        s.opacity = 0.5;
        if apply(&mut out, &s, false, B, &c).is_ok() || out.pixels != base.pixels {
            return Err("Missing image mutated output".into());
        }
        s.enabled = false;
        s.noise = true;
        s.strength = 0.25;
        s.include_cover = false;
        apply(&mut out, &s, true, B, &c)?;
        if out.pixels != base.pixels {
            return Err("Cover exclusion failed".into());
        }
        let storage =
            crate::storage::UserStorage::at(dir.join("user")).map_err(|e| e.to_string())?;
        s.image = None;
        s.save(&storage)?;
        let restored = Settings::load(&storage)?;
        if restored != s {
            return Err("Watermark preset roundtrip".into());
        }
        let mut again = base.clone();
        apply(&mut out, &s, false, B, &c)?;
        apply(&mut again, &restored, false, B, &c)?;
        if out.pixels != again.pixels || out.pixels == base.pixels {
            return Err("Noise restart/frame determinism".into());
        }
        let before = out.pixels.clone();
        c.store(true, std::sync::atomic::Ordering::Relaxed);
        if apply(&mut out, &s, false, B, &c).is_ok() || out.pixels != before {
            return Err("Canceled output was partially modified".into());
        }
        c.store(false, std::sync::atomic::Ordering::Relaxed);
        s.noise = false;
        s.enabled = true;
        s.source = Source::Text;
        s.text = "한글 © Drawer".into();
        s.font = crate::text_layout::Font::Ubuntu;
        s.scale = [0.5; 2];
        s.repeat = false;
        s.path = None;
        let mut text = base.clone();
        apply(&mut text, &s, false, B, &c)?;
        s.scale = [90., 10.];
        s.rotation = 123.;
        let saved = s.clone();
        s.reset_transform();
        if s.opacity != saved.opacity || s.text != saved.text || s.scale != [1.; 2] {
            return Err("Transform reset changed unrelated options".into());
        }
        let timeline = crate::animation_export_verify::fixture();
        let mut animation = crate::animation_export::prepare(
            &timeline,
            crate::animation_export::Options {
                first: 0,
                last: 2,
                output_watermark: true,
                ..Default::default()
            },
            B,
        )?;
        let mut preset = Settings {
            enabled: true,
            source: Source::Image,
            embed: true,
            image: Some(Image {
                width: 1,
                height: 1,
                pixels: vec![[255, 50, 10, 128]],
            }),
            anchor: Anchor::Center,
            margin: [0.; 2],
            noise: true,
            strength: 0.05,
            ..Default::default()
        };
        animation.set_output_watermark(&preset)?;
        let first = crate::animation_render::frame(&animation, 0, &c)?;
        preset.image.as_mut().unwrap().pixels[0] = [0; 4];
        preset.seed += 99;
        if crate::animation_render::frame(&animation, 0, &c)? != first {
            return Err("Captured animation preset changed after editing source settings".into());
        }
        let mut same = animation.clone();
        same.options.first = 2;
        same.options.last = 2;
        let frame = crate::animation_render::frame(&same, 2, &c)?;
        if frame != first {
            return Err("Watermark/noise pattern flickered between identical frames".into());
        }
        let transparent = vec![0u8; 4 * 4];
        let mut bytes = transparent.clone();
        let noise = Settings {
            noise: true,
            strength: 0.25,
            ..Default::default()
        };
        super::frame(&mut bytes, [2, 2], &noise, None, B, &c)?;
        if bytes != transparent {
            return Err("Noise filled transparent alpha".into());
        }
        let manual = crate::raster_io::Options {
            format: crate::raster_io::Format::Jpeg,
            alpha: true,
            depth: crate::color_precision::Depth::Sixteen,
            compression: crate::raster_io::Compression::Lzw,
            ..Default::default()
        };
        let applied = manual.applied();
        if applied.alpha
            || applied.depth != crate::color_precision::Depth::Eight
            || !manual.alpha
            || manual.depth != crate::color_precision::Depth::Sixteen
        {
            return Err("Inactive codec inputs lost".into());
        }
        if d.snapshot() != original || d.history_counts() != history {
            return Err("Watermark changed document/history".into());
        }
        Ok(())
    })();
    result.and(fs::remove_dir_all(dir).map_err(|e| e.to_string()))
}
#[cfg(test)]
mod tests {
    #[test]
    fn output_watermark_modes_geometry_disabled_values_cancel_and_restart() {
        let t = crate::test_support::TempDir::new();
        super::run(&t.0).unwrap();
    }
}
