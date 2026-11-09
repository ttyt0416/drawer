//! One refusal policy shared by document/image entry points. No private-format parser.
use std::{
    io::{self, Read},
    path::Path,
};
pub const GUIDE: &str = "SAI / SAI2 / CLIP direct document reading and writing are unsupported. Keep editable work in .drawer; export PSD / PNG from the original application for local exchange. Renaming a file does not convert its contents.";
pub fn reject_private(path: &Path) -> Result<(), String> {
    if path.extension().is_some_and(|e| {
        ["sai", "sai2", "clip"]
            .iter()
            .any(|s| e.eq_ignore_ascii_case(s))
    }) {
        Err(GUIDE.into())
    } else {
        Ok(())
    }
}
pub fn probe(path: &Path) -> Result<(), String> {
    reject_private(path)?;
    let absolute = if path.is_absolute() {
        path.to_owned()
    } else {
        std::env::current_dir()
            .map_err(|e| e.to_string())?
            .join(path)
    };
    if !crate::font_library::local(&absolute) {
        return Err("Choose a local file on this computer".into());
    }
    let absolute = absolute.canonicalize().map_err(|e| e.to_string())?;
    if !crate::font_library::local(&absolute) {
        return Err("Choose a local file on this computer".into());
    }
    let metadata = std::fs::metadata(&absolute).map_err(|e| e.to_string())?;
    if !metadata.is_file() {
        return Err("Choose a regular local document/image file".into());
    }
    let mut head = Vec::with_capacity(32);
    std::fs::File::open(&absolute)
        .map_err(|e| e.to_string())?
        .take(32)
        .read_to_end(&mut head)
        .map_err(|e| e.to_string())?;
    if head.starts_with(b"DRAWER\r\n")
        || head.starts_with(b"8BPS")
        || image::guess_format(&head).is_ok()
        || path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("tga"))
    {
        Ok(())
    } else {
        Err(GUIDE.into())
    }
}
pub fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}
pub fn verify(root: &Path) -> Result<(), String> {
    use crate::document::{Background, CanvasSpec, DEFAULT_DOCUMENT_BUDGET as B, Document};
    use std::sync::atomic::AtomicBool;
    let dir = root.join("unsupported-documents-core");
    std::fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let result = (|| {
        let mut d = Document::create(
            CanvasSpec {
                width: 2,
                height: 2,
                dpi: 300.,
                background: Background::Transparent,
            },
            B,
        )
        .map_err(|e| e.to_string())?;
        let before = d.snapshot();
        let history = d.history_counts();
        for name in [
            "private.sai",
            "private.SAI2",
            "private.clip",
            "renamed.png",
            "renamed.drawer",
            "renamed.psd",
            "renamed.psb",
            "renamed.tga",
        ] {
            let path = dir.join(name);
            let bytes = b"Unrecognized private-document payload, never treated as image pixels";
            std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
            let err = crate::files::open_report(&path, B)
                .err()
                .ok_or("Unsupported document opened")?
                .to_string();
            let image = crate::image_import::ImageSource::read(&path, B, &AtomicBool::new(false))
                .err()
                .ok_or("Unsupported image imported")?
                .to_string();
            if !err.contains(GUIDE) || !image.contains(GUIDE) {
                return Err(format!(
                    "Different unsupported-format guidance for {name}: {err}; {image}"
                ));
            }
            if reject_private(&path).is_err() {
                let err = crate::files::save(&mut d, &path)
                    .err()
                    .ok_or("Private document save allowed")?
                    .to_string();
                if !err.contains(GUIDE) {
                    return Err("Private save guidance differs".into());
                }
            }
            if std::fs::read(&path).map_err(|e| e.to_string())? != bytes {
                return Err("Unsupported input was changed".into());
            }
            std::fs::remove_file(path).map_err(|e| e.to_string())?;
        }
        let png = dir.join("valid.png");
        image::save_buffer(&png, &[20u8, 40, 60, 255], 1, 1, image::ColorType::Rgba8)
            .map_err(|e| e.to_string())?;
        let bytes = std::fs::read(&png).map_err(|e| e.to_string())?;
        let disguised = dir.join("valid-image.clip");
        std::fs::write(&disguised, &bytes).map_err(|e| e.to_string())?;
        if probe(&disguised).is_ok()
            || crate::image_import::ImageSource::read(&disguised, B, &AtomicBool::new(false))
                .is_ok()
            || d.snapshot() != before
            || d.history_counts() != history
            || std::fs::read(&disguised).unwrap() != bytes
        {
            return Err("Suffix disguise / original state changed".into());
        }
        std::fs::remove_file(png).map_err(|e| e.to_string())?;
        std::fs::remove_file(disguised).map_err(|e| e.to_string())?;
        Ok(())
    })();
    result.and(std::fs::remove_dir(dir).map_err(|e| e.to_string()))
}
#[cfg(test)]
mod tests {
    #[test]
    fn unsupported_extensions_disguises_read_write_and_original_preservation() {
        let t = crate::test_support::TempDir::new();
        super::verify(&t.0).unwrap();
    }
}
