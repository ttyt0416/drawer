use crate::{
    color_precision::{Depth, Pixels, Sample},
    document::Document,
    render::Renderer,
};
use image::ImageEncoder;
use std::{
    fs,
    io::{self, BufWriter, Write},
    path::Path,
};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Options {
    pub depth: Depth,
    pub transparent: bool,
    pub background: [u8; 3],
}
impl Default for Options {
    fn default() -> Self {
        Self {
            depth: Depth::Eight,
            transparent: true,
            background: [255; 3],
        }
    }
}
/// Export a rendered copy; source association, pixels, history and view stay unchanged.
pub fn export(d: &Document, path: &Path, budget: usize) -> io::Result<()> {
    export_with(d, path, budget, Options::default())
}
pub fn export_with(
    document: &Document,
    path: &Path,
    budget: usize,
    options: Options,
) -> io::Result<()> {
    let invalid = |s: &str| io::Error::new(io::ErrorKind::InvalidInput, s.to_owned());
    if !document.color.valid_layers(&document.layers)
        || !document.scene.paper.valid_for(document.layers.len())
        || !document.scene.valid()
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
    if fs::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_symlink() || !m.is_file()) {
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
    let extra =
        if !document.scene.include_drafts_in_output && crate::layer_roles::has_drafts(document) {
            document.memory_bytes().saturating_mul(2)
        } else {
            0
        };
    let count = (document.width as usize).saturating_mul(document.height as usize);
    if extra > budget / 2
        || count
            .saturating_mul(options.depth.bytes())
            .saturating_mul(3)
            > budget.saturating_sub(extra)
    {
        return Err(invalid("PNG output buffers exceed export memory budget"));
    }
    let floating = document.color != crate::color_storage::Config::legacy();
    let owned = if floating {
        crate::layer_roles::output_view(document).map_err(|e| invalid(&e))?
    } else {
        None
    };
    let d = owned.as_deref().unwrap_or(document);
    let (width, height, pixels) = if floating {
        let r = crate::float_render::Renderer::for_export(d).map_err(|e| invalid(&e))?;
        let space =
            crate::color_profile::WorkingSpace::new(&d.color.profile).map_err(|e| invalid(&e))?;
        let matte = space
            .from_srgb(Sample::from8([
                options.background[0],
                options.background[1],
                options.background[2],
                255,
            ]))
            .map_err(|e| invalid(&e))?;
        let page = &d.scene.comics.page;
        page.guides(d).map_err(|e| invalid(&e))?;
        let mut low = vec![];
        let mut high = vec![];
        if options.depth == Depth::Eight {
            low.try_reserve_exact(count)
                .map_err(|_| invalid("Not enough PNG memory"))?;
        } else {
            high.try_reserve_exact(count)
                .map_err(|_| invalid("Not enough PNG memory"))?;
        }
        for y in 0..d.height {
            for x in 0..d.width {
                let mut value = r
                    .pixel([x as f64 + 0.5, y as f64 + 0.5])
                    .map_err(|e| invalid(&e))?;
                if page.include_guides || page.include_number {
                    let overlay = page.output_pixel(d, [x as f64 + 0.5, y as f64 + 0.5]);
                    if overlay[3] != 0 {
                        value = value
                            .over(
                                space
                                    .from_srgb(Sample::from8(overlay))
                                    .map_err(|e| invalid(&e))?,
                                crate::layer_blend::Mode::Normal,
                                1.,
                            )
                            .map_err(|e| invalid(&e))?;
                    }
                }
                if !options.transparent {
                    value = matte
                        .over(value, crate::layer_blend::Mode::Normal, 1.)
                        .map_err(|e| invalid(&e))?;
                }
                let v = value.store(options.depth, d.color.dither, [x as i64, y as i64]);
                if options.depth == Depth::Eight {
                    low.push(v.map(|v| v as u8));
                } else {
                    high.push(v);
                }
            }
        }
        (
            d.width,
            d.height,
            if options.depth == Depth::Eight {
                Pixels::Eight(low)
            } else {
                Pixels::Sixteen(high)
            },
        )
    } else {
        let (w, h, mut low) = Renderer::for_export(document)
            .map_err(|e| invalid(&e))?
            .output(document.dpi, budget.saturating_sub(extra) / 2)
            .map_err(|e| invalid(&e))?;
        if low
            .len()
            .saturating_mul(options.depth.bytes())
            .saturating_mul(3)
            > budget.saturating_sub(extra)
        {
            return Err(invalid("PNG output size exceeds memory budget"));
        }
        if !options.transparent {
            let matte = Sample::from8([
                options.background[0],
                options.background[1],
                options.background[2],
                255,
            ]);
            if options.depth == Depth::Sixteen {
                let high = low
                    .into_iter()
                    .map(|p| {
                        matte
                            .over(Sample::from8(p), crate::layer_blend::Mode::Normal, 1.)
                            .map(|s| s.rgba16())
                            .map_err(|e| invalid(&e))
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                (w, h, Pixels::Sixteen(high))
            } else {
                for p in &mut low {
                    *p = matte
                        .over(Sample::from8(*p), crate::layer_blend::Mode::Normal, 1.)
                        .map_err(|e| invalid(&e))?
                        .rgba8();
                }
                (w, h, Pixels::Eight(low))
            }
        } else if options.depth == Depth::Sixteen {
            (
                w,
                h,
                Pixels::Sixteen(low.into_iter().map(|p| p.map(|v| v as u16 * 257)).collect()),
            )
        } else {
            (w, h, Pixels::Eight(low))
        }
    };
    let icc = if floating || options.depth == Depth::Sixteen {
        Some(match &document.color.profile {
            crate::color_profile::RgbProfile::Embedded { bytes, .. } => bytes.clone(),
            _ => {
                let mut p = moxcms::ColorProfile::new_srgb()
                    .encode()
                    .map_err(|e| invalid(&e.to_string()))?;
                for (slot, value) in p[24..36]
                    .as_chunks_mut::<2>()
                    .0
                    .iter_mut()
                    .zip([2000u16, 1, 1, 0, 0, 0])
                {
                    slot.copy_from_slice(&value.to_be_bytes());
                }
                p
            }
        })
    } else {
        None
    };
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(pixels.memory_bytes())
        .map_err(|_| invalid("Not enough PNG memory"))?;
    match pixels {
        Pixels::Eight(v) => {
            for p in v {
                bytes.extend(p);
            }
        }
        Pixels::Sixteen(v) => {
            for p in v {
                for n in p {
                    bytes.extend(n.to_ne_bytes());
                }
            }
        }
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
        let mut encoder = image::codecs::png::PngEncoder::new(&mut writer);
        if let Some(icc) = icc {
            encoder
                .set_icc_profile(icc)
                .map_err(|e| invalid(&e.to_string()))?;
        }
        encoder
            .write_image(
                &bytes,
                width,
                height,
                if options.depth == Depth::Sixteen {
                    image::ExtendedColorType::Rgba16
                } else {
                    image::ExtendedColorType::Rgba8
                },
            )
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
    fn generated_srgb_png_bytes_are_stable_across_clock_boundary() {
        let root = TempDir::new();
        let path = root.0.join("stable.png");
        let d = Document::create_with_color(
            CanvasSpec {
                width: 1,
                height: 1,
                dpi: 300.,
                background: Background::Color([120, 60, 30]),
            },
            DEFAULT_DOCUMENT_BUDGET,
            Default::default(),
        )
        .unwrap();
        export(&d, &path, DEFAULT_DOCUMENT_BUDGET).unwrap();
        let first = fs::read(&path).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(1100));
        export(&d, &path, DEFAULT_DOCUMENT_BUDGET).unwrap();
        assert_eq!(first, fs::read(&path).unwrap());
        let info = png::Decoder::new(std::io::BufReader::new(fs::File::open(&path).unwrap()))
            .read_info()
            .unwrap();
        let icc = info.info().icc_profile.as_ref().unwrap();
        assert_eq!(&icc[24..36], &[7, 208, 0, 1, 0, 1, 0, 0, 0, 0, 0, 0]);
        assert!(moxcms::ColorProfile::new_from_slice(icc).is_ok());
    }
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
