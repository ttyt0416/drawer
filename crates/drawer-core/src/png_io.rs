//! Strict PNG container validation and a sixteen-bit import path.
use crate::{
    color_precision::{Depth, Sample},
    image_import::ImportedPage,
    references::LocalImage,
};
use image::{DynamicImage, ImageDecoder, metadata::Orientation};
use std::{
    io::{self, Cursor},
    sync::atomic::AtomicBool,
};
fn invalid(s: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, s.into())
}
#[derive(Clone, Copy)]
pub(crate) struct Info {
    pub bits: u8,
    pub icc: bool,
}
pub(crate) fn validate(bytes: &[u8], cancel: &AtomicBool) -> io::Result<Info> {
    if !bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Err(invalid("Invalid PNG signature"));
    }
    let mut at = 8usize;
    let mut header = None;
    let mut idat = false;
    let mut ended = false;
    let mut icc = false;
    while at < bytes.len() {
        crate::book_io::check_cancel(cancel).map_err(invalid)?;
        if bytes.len() - at < 12 {
            return Err(invalid("Truncated PNG chunk"));
        }
        let len = u32::from_be_bytes(bytes[at..at + 4].try_into().unwrap()) as usize;
        let end = at
            .checked_add(12)
            .and_then(|n| n.checked_add(len))
            .filter(|n| *n <= bytes.len())
            .ok_or_else(|| invalid("PNG chunk length exceeds file"))?;
        let kind = &bytes[at + 4..at + 8];
        if !kind.iter().all(u8::is_ascii_alphabetic) || kind[2].is_ascii_lowercase() {
            return Err(invalid("Invalid PNG chunk type"));
        }
        let crc = u32::from_be_bytes(bytes[end - 4..end].try_into().unwrap());
        if crc32fast::hash(&bytes[at + 4..end - 4]) != crc {
            return Err(invalid(format!(
                "PNG CRC mismatch in {}",
                String::from_utf8_lossy(kind)
            )));
        }
        if header.is_none() && kind != b"IHDR" {
            return Err(invalid("PNG IHDR must be first"));
        }
        match kind {
            b"IHDR" => {
                if at != 8 || len != 13 {
                    return Err(invalid("Duplicate / invalid PNG IHDR"));
                }
                header = Some(bytes[at + 16]);
            }
            b"IDAT" => {
                if ended {
                    return Err(invalid("PNG IDAT chunks must be consecutive"));
                }
                idat = true;
            }
            b"IEND" => {
                if len != 0 || !idat || end != bytes.len() {
                    return Err(invalid("Invalid PNG IEND / trailing data"));
                }
                return Ok(Info {
                    bits: header.unwrap(),
                    icc,
                });
            }
            _ => {
                if idat {
                    ended = true;
                }
                if kind == b"iCCP" {
                    if icc {
                        return Err(invalid("Duplicate PNG ICC profile"));
                    }
                    icc = true;
                }
            }
        }
        at = end;
    }
    Err(invalid("Missing PNG IEND"))
}
pub(crate) fn decode16(
    bytes: &[u8],
    orient: bool,
    budget: usize,
    cancel: &AtomicBool,
) -> io::Result<ImportedPage> {
    let mut reader = image::ImageReader::with_format(Cursor::new(bytes), image::ImageFormat::Png);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(crate::document::MAX_DIMENSION);
    limits.max_image_height = limits.max_image_width;
    limits.max_alloc = Some(budget as u64);
    reader.limits(limits);
    let mut decoder = reader.into_decoder().map_err(|e| invalid(e.to_string()))?;
    let gray = matches!(
        decoder.color_type(),
        image::ColorType::L16 | image::ColorType::La16
    );
    let orientation = decoder.orientation().map_err(|e| invalid(e.to_string()))?;
    let profile = decoder.icc_profile().map_err(|e| invalid(e.to_string()))?;
    let rgba = DynamicImage::from_decoder(decoder)
        .map_err(|e| invalid(e.to_string()))?
        .into_rgba16();
    finish16(rgba, orientation, profile, gray, orient, budget, cancel)
}
pub(crate) fn finish16(
    mut rgba: image::ImageBuffer<image::Rgba<u16>, Vec<u16>>,
    orientation: Orientation,
    profile: Option<Vec<u8>>,
    gray: bool,
    orient: bool,
    budget: usize,
    cancel: &AtomicBool,
) -> io::Result<ImportedPage> {
    if (rgba.width() as usize)
        .saturating_mul(rgba.height() as usize)
        .saturating_mul(32)
        > budget
    {
        return Err(invalid(
            "High precision image conversion exceeds memory budget",
        ));
    }
    if let Some(bytes) = profile
        .as_ref()
        .filter(|bytes| gray || !is_own_srgb_profile(bytes))
    {
        if bytes.len() > 1024 * 1024 {
            return Err(invalid("ICC profile exceeds1MiB"));
        }
        use moxcms::{ColorProfile, DataColorSpace, Layout};
        let p = ColorProfile::new_from_slice(bytes)
            .map_err(|e| invalid(format!("Invalid ICC profile: {e}")))?;
        let layout = match (p.color_space, gray) {
            (DataColorSpace::Rgb, false) => Layout::Rgba,
            (DataColorSpace::Gray, true) => Layout::Gray,
            _ => return Err(invalid("Image ICC profile does not match RGB/Gray input")),
        };
        let transform = p
            .create_transform_16bit(
                layout,
                &ColorProfile::new_srgb(),
                Layout::Rgba,
                Default::default(),
            )
            .map_err(|e| invalid(format!("Unsupported ICC profile: {e}")))?;
        let stride = rgba.width() as usize * 4;
        let mut input = Vec::with_capacity(stride);
        let mut output = vec![0u16; stride];
        for row in rgba.as_mut().chunks_exact_mut(stride) {
            crate::book_io::check_cancel(cancel).map_err(invalid)?;
            input.clear();
            if gray {
                input.extend(row.as_chunks::<4>().0.iter().map(|p| p[0]));
            } else {
                input.extend_from_slice(row);
            }
            transform
                .transform(&input, &mut output)
                .map_err(|e| invalid(e.to_string()))?;
            for (p, c) in row
                .as_chunks_mut::<4>()
                .0
                .iter_mut()
                .zip(output.as_chunks::<4>().0)
            {
                p[..3].copy_from_slice(&c[..3]);
            }
        }
    }
    let mut image = DynamicImage::ImageRgba16(rgba);
    if orient {
        image.apply_orientation(orientation);
    }
    let rgba = image.into_rgba16();
    let mut pixels = Vec::with_capacity(rgba.width() as usize * rgba.height() as usize);
    let mut preview = Vec::with_capacity(pixels.capacity());
    let mut hidden = 0;
    for (i, p) in rgba.as_raw().as_chunks::<4>().0.iter().enumerate() {
        if i % 4096 == 0 {
            crate::book_io::check_cancel(cancel).map_err(invalid)?;
        }
        let mut p = *p;
        if p[3] == 0 && p[..3] != [0; 3] {
            p = [0; 4];
            hidden += 1;
        }
        pixels.push(p);
        preview.push(Sample::from16(p).rgba8());
    }
    Ok(ImportedPage {
        image: LocalImage {
            width: rgba.width(),
            height: rgba.height(),
            pixels: preview,
        },
        orientation,
        profile_applied: profile.is_some(),
        color: Some(crate::paste::ClipboardColor {
            profile: crate::color_profile::RgbProfile::Srgb,
            pixels,
        }),
        bits: 16,
        warnings: if hidden == 0 {
            vec![]
        } else {
            vec![format!(
                "{hidden} fully transparent16-bit pixels: hidden RGB normalized to zero; alpha preserved"
            )]
        },
    })
}
impl ImportedPage {
    pub fn document(self, budget: usize) -> io::Result<crate::document::Document> {
        let Some(color) = self.color else {
            return self.image.document(budget);
        };
        let count = self.image.pixels.len();
        if count.saturating_mul(24) > budget || color.pixels.len() != count {
            return Err(invalid(
                "High precision image document exceeds memory budget",
            ));
        }
        let mut d = crate::document::Document::create_with_color(
            crate::document::CanvasSpec {
                width: self.image.width,
                height: self.image.height,
                dpi: 300.,
                background: crate::document::Background::Transparent,
            },
            budget,
            crate::color_storage::Config {
                depth: Depth::Sixteen,
                profile: color.profile,
                ..Default::default()
            },
        )
        .map_err(|e| invalid(e.to_string()))?;
        d.layers[0].pixels = self.image.pixels;
        d.layers[0].precise = Some(color.pixels);
        Ok(d)
    }
}

/// Own deterministic standard-sRGB ICC is already the destination space. Re-parsing its
/// quantized matrix as a foreign profile causes avoidable16-bit round-trip drift.
pub(crate) fn is_own_srgb_profile(bytes: &[u8]) -> bool {
    static STANDARD: std::sync::OnceLock<Option<Vec<u8>>> = std::sync::OnceLock::new();
    STANDARD
        .get_or_init(|| {
            moxcms::ColorProfile::new_srgb().encode().ok().map(|mut p| {
                for (slot, n) in p[24..36]
                    .as_chunks_mut::<2>()
                    .0
                    .iter_mut()
                    .zip([2000u16, 1, 1, 0, 0, 0])
                {
                    slot.copy_from_slice(&n.to_be_bytes());
                }
                p
            })
        })
        .as_ref()
        .is_some_and(|p| p.as_slice() == bytes)
}
