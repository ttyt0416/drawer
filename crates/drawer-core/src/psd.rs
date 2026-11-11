//! Compatibility entry points use the verified PSD/PSB exchange engine.
use crate::document::Document;
#[cfg(test)]
use crate::{document::RasterLayer, render::Renderer};
#[cfg(test)]
use std::fs;
use std::{io, path::Path};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Choice {
    Cancel,
    RenderedLayers,
    RenderedImage,
}
fn error(s: impl ToString) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, s.to_string())
}
pub fn layers_supported(d: &Document) -> bool {
    (d.scene.include_drafts_in_output || !crate::layer_roles::has_drafts(d))
        && !crate::layer_blend::graph_active(d)
        && d.scene.render_order.is_default()
        && d.animation.is_none()
        && d.scene.gradients.layers.is_empty()
        && d.scene.objects.is_empty()
        && d.scene.lines.layers.is_empty()
        && d.scene.worlds.is_empty()
        && d.scene
            .paper
            .layers
            .values()
            .all(|p| !p.hidden && p.blend == crate::brush_composite::Mode::Normal)
        && d.scene
            .paper
            .folders
            .iter()
            .all(|f| !f.paper.hidden && f.paper.blend == crate::brush_composite::Mode::Normal)
}
pub const LOSS: &str = "Editable text (Unicode, fonts, layout and transforms) is rasterized; this exporter has no verified editable-text adapter. PSD stores rendered RGB8 sRGB pixels; 16-bit document precision is reduced only in this output. Paper recipes, file-layer caches and editable folder effects are baked; folders become raster substitutes. Brush history and other Drawer metadata are not included. Layer blend settings are baked in rendered-image export; editable blend-mode metadata is not preserved. Keep the .drawer original. Merged-preview import does not restore PSD layers and uses 300 DPI.";
pub fn text_loss(d: &Document) -> Option<String> {
    let count = d
        .scene
        .objects
        .iter()
        .filter(|o| matches!(o, crate::scene::PageObject::Text { .. }))
        .count();
    (count>0).then(||format!("{count} editable text object(s): raster substitutes only. Unicode, fonts, layout generations and text transforms remain editable only in the .drawer original."))
}
pub fn export(
    d: &Document,
    path: &Path,
    choice: Choice,
    replace: bool,
    budget: usize,
) -> io::Result<bool> {
    if choice == Choice::Cancel {
        return Ok(false);
    }
    if choice == Choice::RenderedLayers && !layers_supported(d) {
        return Err(error(
            "This legacy rendered-layer choice cannot preserve the composition; use PSD/PSB exchange review or the whole composite",
        ));
    }
    let cancel = std::sync::atomic::AtomicBool::new(false);
    let plan = crate::psd_exchange::prepare(d, budget, &cancel).map_err(error)?;
    plan.write(
        d,
        path,
        crate::psd_exchange::Format::Psd,
        choice == Choice::RenderedImage,
        true,
        replace,
        budget,
        &cancel,
    )
    .map_err(error)?;
    Ok(true)
}
pub fn open(path: &Path, budget: usize) -> io::Result<Document> {
    let (source, _) = crate::psd_exchange::open_report(path, budget).map_err(error)?;
    let pixels =
        crate::psd_exchange::render(&source, budget, &std::sync::atomic::AtomicBool::new(false))
            .map_err(error)?;
    let mut d = Document::create(
        crate::document::CanvasSpec {
            width: source.width,
            height: source.height,
            dpi: source.dpi,
            background: crate::document::Background::Transparent,
        },
        budget,
    )
    .map_err(error)?;
    d.layers[0].pixels = pixels;
    d.layers[0].name = "PSD merged preview".into();
    d.title = source.title;
    d.source_path = source.source_path;
    Ok(d)
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        document::{Background, CanvasSpec},
        paper::{Node, Paper},
        test_support::TempDir,
        texture::Pattern,
    };
    fn fixture() -> Document {
        let mut d = Document::create(
            CanvasSpec {
                width: 9,
                height: 5,
                dpi: 300.0,
                background: Background::Transparent,
            },
            1024,
        )
        .unwrap();
        for (n, p) in d.layers[0].pixels.iter_mut().enumerate() {
            *p = [220, 40 + n as u8 * 3, 90, 20 + n as u8 * 3];
        }
        d.layers[0].name = "밑그림".into();
        d.scene.paper.layers.insert(
            0,
            Paper {
                pattern: Pattern {
                    source: 1,
                    strength: 0.7,
                    ..Default::default()
                },
                ..Default::default()
            },
        );
        let n = d.scene.paper.group_all(1, "한지 폴더").unwrap();
        d.scene.paper.folders[n].paper.pattern = Pattern {
            source: 2,
            strength: 0.6,
            ..Default::default()
        };
        d.layers.push(RasterLayer {
            tiled: None,
            precise: None,
            locked: true,
            name: "Top blue".into(),
            pixels: vec![[30, 90, 220, 70]; 45],
            placement: Some(crate::document::PlacedRaster {
                width: 9,
                height: 5,
                offset: [1, -1],
            }),
        });
        d
    }
    #[test]
    fn rendered_psd_cancel_layers_whole_reopen_and_source_are_unchanged() {
        let root = TempDir::new();
        let path = root.0.join("out.psd");
        let d = fixture();
        let state = d.snapshot();
        let history = d.history_counts();
        let dirty = d.dirty;
        assert!(!export(&d, &path, Choice::Cancel, false, 1).unwrap());
        assert!(!path.exists());
        for choice in [Choice::RenderedLayers, Choice::RenderedImage] {
            export(&d, &path, choice, true, 100000).unwrap();
            let opened = crate::files::open(&path, 100000).unwrap();
            let expected = Renderer::new(&d).output(d.dpi, 100000).unwrap().2;
            assert!(
                crate::psd_exchange::difference(
                    &Renderer::new(&opened).output(opened.dpi, 100000).unwrap().2,
                    &expected
                ) <= 1
            );
            assert!(d.snapshot() == state);
            assert_eq!(d.history_counts(), history);
            assert_eq!(d.dirty, dirty);
        }
        let bytes = fs::read(&path).unwrap();
        assert!(export(&d, &path, Choice::RenderedLayers, false, 100000).is_err());
        assert!(export(&d, &path, Choice::RenderedLayers, true, 1).is_err());
        assert_eq!(fs::read(&path).unwrap(), bytes);
        let mut imported = open(&path, 100000).unwrap();
        imported.scene.paper.group_all(1, "New folder").unwrap();
        assert!(export(&imported, &path, Choice::RenderedLayers, true, 100000).is_err());
        assert_eq!(fs::read(&path).unwrap(), bytes);
        fs::write(root.0.join("out.psd.psd-exchange.tmp"), "foreign temporary").unwrap();
        assert!(export(&d, &path, Choice::RenderedLayers, true, 100000).is_err());
        assert_eq!(fs::read(&path).unwrap(), bytes);
        assert_eq!(
            fs::read_to_string(root.0.join("out.psd.psd-exchange.tmp")).unwrap(),
            "foreign temporary"
        );
    }
    #[test]
    fn page_objects_require_explicit_whole_image_and_invalid_psd_is_bounded() {
        let root = TempDir::new();
        let path = root.0.join("out.psd");
        let mut d = fixture();
        d.scene.objects.push(crate::scene::PageObject::Line {
            start: [0.0; 2],
            end: [8.0, 4.0],
            width_px: 2.0,
            color: [255, 255, 0, 255],
        });
        assert!(export(&d, &path, Choice::RenderedLayers, false, 100000).is_err());
        assert!(!path.exists());
        export(&d, &path, Choice::RenderedImage, false, 100000).unwrap();
        assert!(
            crate::psd_exchange::difference(
                &open(&path, 100000).unwrap().layers[0].pixels,
                &Renderer::new(&d).output(d.dpi, 100000).unwrap().2
            ) <= 1
        );
        let mut bytes = fs::read(&path).unwrap();
        bytes[26..30].copy_from_slice(&u32::MAX.to_be_bytes());
        fs::write(&path, &bytes).unwrap();
        assert!(open(&path, 100000).is_err());
        bytes.truncate(12);
        fs::write(&path, &bytes).unwrap();
        assert!(open(&path, 100000).is_err());
    }
    #[test]
    fn psd_decoder_fixture() {
        let root = TempDir::new();
        let folder = std::env::var_os("DRAWER_PSD_VERIFY_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| root.0.clone());
        let mut d = fixture();
        export(
            &d,
            &folder.join("roots.psd"),
            Choice::RenderedLayers,
            false,
            100000,
        )
        .unwrap();
        crate::png_export::export(&d, &folder.join("roots-reference.png"), 100000).unwrap();
        let n = d.scene.paper.group_all(2, "Nested paper").unwrap();
        d.scene.paper.folders[n].paper.pattern = Pattern {
            source: 1,
            strength: 0.2,
            ..Default::default()
        };
        export(
            &d,
            &folder.join("nested.psd"),
            Choice::RenderedLayers,
            false,
            100000,
        )
        .unwrap();
        crate::png_export::export(&d, &folder.join("reference.png"), 100000).unwrap();
        d.layers[1].locked = false;
        let before = Renderer::new(&d).output(d.dpi, 100000).unwrap().2;
        crate::paper::bake(&mut d, Node::Folder(n)).unwrap();
        assert_eq!(Renderer::new(&d).output(d.dpi, 100000).unwrap().2, before);
        export(
            &d,
            &folder.join("baked.psd"),
            Choice::RenderedLayers,
            false,
            100000,
        )
        .unwrap();
    }
}
