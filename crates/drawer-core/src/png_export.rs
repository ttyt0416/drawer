use crate::{document::Document, render::Renderer};
use image::ImageEncoder;
use std::{
    fs,
    io::{self, BufWriter, Write},
    path::Path,
};

/// Export pixels without changing document association, history, selection or view.
pub fn export(document: &Document, path: &Path, budget: usize) -> io::Result<()> {
    let invalid = |message: &str| io::Error::new(io::ErrorKind::InvalidInput, message);
    if !document.scene.valid()
        || document
            .layers
            .iter()
            .any(|l| !l.valid(document.width, document.height))
    {
        return Err(invalid("Invalid document."));
    }
    if !path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("png"))
    {
        return Err(invalid("Choose a .png destination."));
    }
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
        .canonicalize()?;
    let path = parent.join(
        path.file_name()
            .ok_or_else(|| invalid("Missing filename."))?,
    );
    if fs::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_symlink()) {
        return Err(invalid("Choose a regular destination file."));
    }
    let path = if path.exists() {
        path.canonicalize()?
    } else {
        path
    };
    if document.source_path.as_ref() == Some(&path) || document.path.as_ref() == Some(&path) {
        return Err(invalid(
            "Choose another filename to preserve the original document.",
        ));
    }
    let (width, height, pixels) = Renderer::new(document)
        .output(document.dpi, budget / 2)
        .map_err(|e| invalid(&e))?;
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(pixels.len() * 4)
        .map_err(|_| invalid("Not enough export memory."))?;
    for pixel in pixels {
        bytes.extend_from_slice(&pixel);
    }
    let mut name = path.file_name().unwrap().to_owned();
    name.push(".png-write.tmp");
    let temporary = path.with_file_name(name);
    let file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)?;
    let mut writer = BufWriter::new(file);
    let result = (|| {
        image::codecs::png::PngEncoder::new(&mut writer)
            .write_image(&bytes, width, height, image::ExtendedColorType::Rgba8)
            .map_err(|e| invalid(&e.to_string()))?;
        writer.flush()?;
        writer.get_ref().sync_all()
    })();
    drop(writer);
    if let Err(error) = result.and_then(|_| fs::rename(&temporary, &path)) {
        let _ = fs::remove_file(temporary);
        return Err(error);
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        document::{Background, CanvasSpec, DEFAULT_DOCUMENT_BUDGET},
        sizing::{self, Change},
        test_support::TempDir,
    };
    use std::sync::atomic::AtomicBool;
    #[test]
    fn view_mirror_does_not_change_png_but_actual_transforms_change_corner_markers() {
        let root = TempDir::new();
        let path = root.0.join("result.png");
        let mut doc = Document::create(
            CanvasSpec {
                width: 3,
                height: 2,
                dpi: 300.0,
                background: Background::Transparent,
            },
            24,
        )
        .unwrap();
        let (red, green, blue, yellow) = (
            [255, 0, 0, 255],
            [0, 255, 0, 255],
            [0, 0, 255, 255],
            [255, 255, 0, 255],
        );
        doc.layers[0].pixels = vec![red, [0; 4], green, blue, [0; 4], yellow];
        let original = doc.snapshot();
        let read = || {
            image::ImageReader::open(&path)
                .unwrap()
                .decode()
                .unwrap()
                .into_rgba8()
        };
        export(&doc, &path, 1024).unwrap();
        let before = read();
        doc.view.flipped = true;
        doc.view.rotate(37.0);
        export(&doc, &path, 1024).unwrap();
        assert_eq!(before, read());
        assert_eq!(doc.snapshot(), original);
        assert_eq!(doc.history_counts(), (0, 0));
        for (change, corners) in [
            (Change::FlipHorizontal, [green, red, yellow, blue]),
            (Change::FlipVertical, [blue, yellow, red, green]),
            (
                Change::Rotate90 { clockwise: true },
                [blue, red, yellow, green],
            ),
            (
                Change::Rotate90 { clockwise: false },
                [green, yellow, red, blue],
            ),
        ] {
            let result = sizing::prepare(
                &original,
                change,
                DEFAULT_DOCUMENT_BUDGET,
                &AtomicBool::new(false),
            )
            .unwrap();
            doc.commit_state(result).unwrap();
            let history = doc.history_counts();
            let view = doc.view;
            export(&doc, &path, 1024).unwrap();
            let image = read();
            assert_eq!(
                [
                    image.get_pixel(0, 0).0,
                    image.get_pixel(image.width() - 1, 0).0,
                    image.get_pixel(0, image.height() - 1).0,
                    image.get_pixel(image.width() - 1, image.height() - 1).0
                ],
                corners
            );
            assert_eq!(doc.history_counts(), history);
            assert_eq!(doc.view, view);
            assert!(doc.path.is_none());
            assert!(doc.undo());
            assert_eq!(doc.snapshot(), original);
        }
    }
    #[test]
    fn failed_export_preserves_old_file_original_png_and_document_state() {
        let root = TempDir::new();
        let path = root.0.join("out.png");
        let doc = Document::create(
            CanvasSpec {
                width: 1,
                height: 1,
                dpi: 300.0,
                background: Background::Transparent,
            },
            4,
        )
        .unwrap();
        export(&doc, &path, 1024).unwrap();
        let bytes = fs::read(&path).unwrap();
        let temp = root.0.join("out.png.png-write.tmp");
        fs::write(&temp, "busy").unwrap();
        assert!(export(&doc, &path, 1024).is_err());
        assert_eq!(fs::read(&path).unwrap(), bytes);
        assert_eq!(fs::read_to_string(&temp).unwrap(), "busy");
        fs::remove_file(temp).unwrap();
        assert!(export(&doc, &path, 1).is_err());
        assert_eq!(fs::read(&path).unwrap(), bytes);
        let imported = crate::files::open(&path, 1024).unwrap();
        assert!(export(&imported, &path, 1024).is_err());
        assert_eq!(fs::read(&path).unwrap(), bytes);
    }
}
