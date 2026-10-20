//! Read-only image snapshots. Imported pixels use the application's sRGB RGBA8 space.
use crate::{document::MAX_DIMENSION, references::LocalImage};
use image::{DynamicImage, ImageDecoder, ImageFormat, metadata::Orientation};
use moxcms::{ColorProfile, DataColorSpace, Layout};
use std::{
    fs,
    io::{self, Cursor, Read},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use tiff::{
    decoder::{Decoder, DecodingResult},
    tags::Tag,
};

const MAX_ENCODED_BYTES: usize = 64 * 1024 * 1024;
const MAX_PROFILE_BYTES: usize = 1024 * 1024;
const MAX_PAGES: usize = 256;
fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}
fn checked(cancel: &AtomicBool) -> io::Result<()> {
    if cancel.load(Ordering::Relaxed) {
        Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "Image import canceled",
        ))
    } else {
        Ok(())
    }
}
fn dimensions(width: u32, height: u32, budget: usize) -> io::Result<usize> {
    let bytes = (width as usize)
        .checked_mul(height as usize)
        .and_then(|n| n.checked_mul(4));
    if !(1..=MAX_DIMENSION).contains(&width)
        || !(1..=MAX_DIMENSION).contains(&height)
        || bytes.is_none_or(|n| n > budget)
    {
        return Err(invalid(
            "Image dimensions exceed the document or memory limit",
        ));
    }
    Ok(bytes.unwrap())
}
fn image_limits(budget: usize) -> image::Limits {
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_DIMENSION);
    limits.max_image_height = Some(MAX_DIMENSION);
    limits.max_alloc = Some(budget as u64);
    limits
}
#[derive(Clone, Debug)]
pub struct PageInfo {
    pub width: u32,
    pub height: u32,
}
#[derive(Clone)]
pub struct ImageSource {
    pub path: PathBuf,
    pub pages: Vec<PageInfo>,
    bytes: Arc<Vec<u8>>,
    format: ImageFormat,
}
pub struct ImportedPage {
    pub image: LocalImage,
    pub orientation: Orientation,
    pub profile_applied: bool,
}
impl ImageSource {
    pub fn encoded_snapshot(&self) -> Arc<Vec<u8>> {
        self.bytes.clone()
    }
    pub fn read(path: &Path, budget: usize, cancel: &AtomicBool) -> io::Result<Self> {
        checked(cancel)?;
        let path = path.canonicalize()?;
        let mut file = fs::File::open(&path)?;
        let limit = MAX_ENCODED_BYTES.min(budget / 2);
        if !file.metadata()?.is_file() || file.metadata()?.len() > limit as u64 {
            return Err(invalid(
                "Encoded image exceeds the import memory limit (maximum 64 MiB)",
            ));
        }
        let mut bytes = Vec::new();
        file.by_ref()
            .take(limit as u64 + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() > limit {
            return Err(invalid("Image grew beyond the import memory limit"));
        }
        Self::from_encoded(path, bytes, budget, cancel)
    }
    pub(crate) fn from_encoded(
        path: PathBuf,
        bytes: Vec<u8>,
        budget: usize,
        cancel: &AtomicBool,
    ) -> io::Result<Self> {
        if bytes.len() > MAX_ENCODED_BYTES.min(budget / 2) {
            return Err(invalid("Encoded image exceeds the import memory limit"));
        }
        checked(cancel)?;
        let format = image::guess_format(&bytes).map_err(|e| invalid(e.to_string()))?;
        let mut pages = Vec::new();
        match format {
            ImageFormat::Png | ImageFormat::Jpeg | ImageFormat::Bmp => {
                if format == ImageFormat::Png
                    && image::codecs::png::PngDecoder::with_limits(
                        Cursor::new(&bytes),
                        image_limits(budget),
                    )
                    .map_err(|e| invalid(e.to_string()))?
                    .is_apng()
                    .map_err(|e| invalid(e.to_string()))?
                {
                    return Err(invalid(
                        "Animated PNG frames are not yet supported. Choose a still image or a TIFF page.",
                    ));
                }
                let mut reader = image::ImageReader::with_format(Cursor::new(&bytes), format);
                reader.limits(image_limits(budget));
                let decoder = reader.into_decoder().map_err(|e| invalid(e.to_string()))?;
                let (width, height) = decoder.dimensions();
                dimensions(width, height, budget)?;
                pages.push(PageInfo { width, height });
            }
            ImageFormat::Tiff => {
                let mut decoder = tiff_decoder(&bytes, budget)?;
                loop {
                    checked(cancel)?;
                    if pages.len() == MAX_PAGES {
                        return Err(invalid("TIFF contains more than 256 pages"));
                    }
                    let (width, height) =
                        decoder.dimensions().map_err(|e| invalid(e.to_string()))?;
                    dimensions(width, height, budget)?;
                    pages.push(PageInfo { width, height });
                    if !decoder.more_images() {
                        break;
                    }
                    decoder.next_image().map_err(|e| invalid(e.to_string()))?;
                }
            }
            _ => {
                return Err(invalid(
                    "Import supports PNG, JPEG, TIFF and BMP. Other formats are not yet supported.",
                ));
            }
        }
        Ok(Self {
            path,
            pages,
            bytes: Arc::new(bytes),
            format,
        })
    }
    pub fn decode(
        &self,
        page: usize,
        orient: bool,
        budget: usize,
        cancel: &AtomicBool,
    ) -> io::Result<ImportedPage> {
        checked(cancel)?;
        let info = self
            .pages
            .get(page)
            .ok_or_else(|| invalid("Choose a valid image page"))?;
        let count = dimensions(info.width, info.height, budget)?;
        // Reserve space for source copies, decoder/conversion/orientation buffers and the new document.
        let peak = count
            .checked_mul(5)
            .and_then(|n| n.checked_add(self.bytes.len() * 2))
            .and_then(|n| n.checked_add(MAX_PROFILE_BYTES * 2));
        if peak.is_none_or(|n| n > budget) {
            return Err(invalid(
                "Not enough memory for image decoding and the independent document",
            ));
        }
        let (mut rgba, orientation, profile, gray) = if self.format == ImageFormat::Tiff {
            decode_tiff(&self.bytes, page, budget, cancel)?
        } else {
            let mut reader =
                image::ImageReader::with_format(Cursor::new(self.bytes.as_slice()), self.format);
            reader.limits(image_limits(budget));
            let mut decoder = reader.into_decoder().map_err(|e| invalid(e.to_string()))?;
            let gray = matches!(
                decoder.color_type(),
                image::ColorType::L8
                    | image::ColorType::La8
                    | image::ColorType::L16
                    | image::ColorType::La16
            );
            let orientation = decoder.orientation().map_err(|e| invalid(e.to_string()))?;
            let profile = decoder.icc_profile().map_err(|e| invalid(e.to_string()))?;
            let rgba = DynamicImage::from_decoder(decoder)
                .map_err(|e| invalid(e.to_string()))?
                .into_rgba8();
            (rgba, orientation, profile, gray)
        };
        checked(cancel)?;
        let profile_applied = if let Some(profile) = profile {
            convert_profile(&mut rgba, &profile, gray, cancel)?;
            true
        } else {
            false
        };
        let mut image = DynamicImage::ImageRgba8(rgba);
        if orient {
            image.apply_orientation(orientation);
        }
        checked(cancel)?;
        let rgba = image.into_rgba8();
        let image = LocalImage::from_rgba(
            rgba.width() as usize,
            rgba.height() as usize,
            rgba.as_raw(),
            budget,
        )?;
        Ok(ImportedPage {
            image,
            orientation,
            profile_applied,
        })
    }
}
fn tiff_decoder(bytes: &[u8], budget: usize) -> io::Result<Decoder<Cursor<&[u8]>>> {
    let mut limits = tiff::decoder::Limits::default();
    limits.decoding_buffer_size = budget;
    limits.intermediate_buffer_size = budget / 4;
    limits.ifd_value_size = MAX_PROFILE_BYTES;
    Decoder::new(Cursor::new(bytes))
        .map(|d| d.with_limits(limits))
        .map_err(|e| invalid(e.to_string()))
}
type Decoded = (image::RgbaImage, Orientation, Option<Vec<u8>>, bool);
fn decode_tiff(
    bytes: &[u8],
    page: usize,
    budget: usize,
    cancel: &AtomicBool,
) -> io::Result<Decoded> {
    let mut decoder = tiff_decoder(bytes, budget)?;
    decoder
        .seek_to_image(page)
        .map_err(|e| invalid(e.to_string()))?;
    let (width, height) = decoder.dimensions().map_err(|e| invalid(e.to_string()))?;
    dimensions(width, height, budget)?;
    let color = decoder.colortype().map_err(|e| invalid(e.to_string()))?;
    let extra = decoder
        .find_tag_unsigned_vec::<u16>(Tag::ExtraSamples)
        .map_err(|e| invalid(e.to_string()))?;
    let photometric = decoder
        .find_tag_unsigned::<u16>(Tag::PhotometricInterpretation)
        .map_err(|e| invalid(e.to_string()))?;
    let (channels, depth, gray, alpha) = match color {
        tiff::ColorType::Gray(n @ (8 | 16)) => (1, n, true, false),
        tiff::ColorType::GrayA(n @ (8 | 16)) => (2, n, true, true),
        tiff::ColorType::RGB(n @ (8 | 16)) => (3, n, false, false),
        tiff::ColorType::RGBA(n @ (8 | 16)) => (4, n, false, true),
        tiff::ColorType::Multiband {
            bit_depth: n @ (8 | 16),
            num_samples: 2,
        } if photometric == Some(1)
            && extra
                .as_ref()
                .is_some_and(|v| v.len() == 1 && matches!(v[0], 1 | 2)) =>
        {
            (2, n, true, true)
        }
        _ => {
            return Err(invalid(
                "TIFF import supports unsigned Gray/Gray+alpha/RGB/RGBA at 8 or 16 bits. This color layout is unsupported.",
            ));
        }
    };
    if decoder
        .find_tag_unsigned::<u16>(Tag::SamplesPerPixel)
        .map_err(|e| invalid(e.to_string()))?
        .is_some_and(|n| n as usize != channels)
    {
        return Err(invalid(
            "TIFF with additional non-color channels is not yet supported",
        ));
    }
    let orientation = decoder
        .find_tag_unsigned::<u8>(Tag::Orientation)
        .map_err(|e| invalid(e.to_string()))?
        .map_or(Ok(Orientation::NoTransforms), |n| {
            Orientation::from_exif(n).ok_or_else(|| invalid("Invalid TIFF orientation"))
        })?;
    let profile = if decoder
        .find_tag(Tag::IccProfile)
        .map_err(|e| invalid(e.to_string()))?
        .is_some()
    {
        Some(
            decoder
                .get_tag_u8_vec(Tag::IccProfile)
                .map_err(|e| invalid(e.to_string()))?,
        )
    } else {
        None
    };
    let associated = alpha && extra.as_ref().is_some_and(|v| v.first() == Some(&1));
    let layout = decoder
        .image_buffer_layout()
        .map_err(|e| invalid(e.to_string()))?;
    let row = layout
        .row_stride
        .ok_or_else(|| invalid("Unsupported TIFF row layout"))?
        .get()
        / (depth as usize / 8);
    let plane = layout.plane_stride.map(|n| n.get() / (depth as usize / 8));
    checked(cancel)?;
    let decoded = decoder.read_image().map_err(|e| invalid(e.to_string()))?;
    let mut rgba = image::RgbaImage::new(width, height);
    for y in 0..height as usize {
        checked(cancel)?;
        for x in 0..width as usize {
            let sample = |c: usize| -> io::Result<u8> {
                let index = if layout.planes > 1 {
                    c * plane.ok_or_else(|| invalid("Unsupported TIFF plane layout"))? + y * row + x
                } else {
                    y * row + x * channels + c
                };
                match &decoded {
                    DecodingResult::U8(v) => v
                        .get(index)
                        .copied()
                        .ok_or_else(|| invalid("Truncated TIFF pixels")),
                    DecodingResult::U16(v) => v
                        .get(index)
                        .map(|n| ((*n as u32 + 128) / 257) as u8)
                        .ok_or_else(|| invalid("Truncated TIFF pixels")),
                    _ => Err(invalid(
                        "Signed or floating-point TIFF samples are not yet supported",
                    )),
                }
            };
            let r = sample(0)?;
            let mut pixel = [
                r,
                if gray { r } else { sample(1)? },
                if gray { r } else { sample(2)? },
                if alpha { sample(channels - 1)? } else { 255 },
            ];
            if associated {
                for c in 0..3 {
                    pixel[c] = if pixel[3] == 0 {
                        0
                    } else {
                        ((pixel[c] as u32 * 255 + pixel[3] as u32 / 2) / pixel[3] as u32).min(255)
                            as u8
                    };
                }
            }
            rgba.put_pixel(x as u32, y as u32, image::Rgba(pixel));
        }
    }
    Ok((rgba, orientation, profile, gray))
}
fn convert_profile(
    rgba: &mut image::RgbaImage,
    bytes: &[u8],
    gray: bool,
    cancel: &AtomicBool,
) -> io::Result<()> {
    if bytes.len() > MAX_PROFILE_BYTES {
        return Err(invalid("ICC profile exceeds 1 MiB"));
    }
    let profile = ColorProfile::new_from_slice(bytes)
        .map_err(|e| invalid(format!("Invalid ICC profile: {e}")))?;
    let layout = match (profile.color_space, gray) {
        (DataColorSpace::Rgb, false) => Layout::Rgba,
        (DataColorSpace::Gray, true) => Layout::Gray,
        _ => {
            return Err(invalid(
                "The embedded ICC profile does not match a supported RGB/Gray input. Import canceled to avoid incorrect colors.",
            ));
        }
    };
    let transform = profile
        .create_transform_8bit(
            layout,
            &ColorProfile::new_srgb(),
            Layout::Rgba,
            Default::default(),
        )
        .map_err(|e| invalid(format!("Unsupported ICC profile: {e}")))?;
    let stride = rgba.width() as usize * 4;
    let mut input = Vec::with_capacity(stride);
    let mut output = vec![0; stride];
    for row in rgba.as_mut().chunks_exact_mut(stride) {
        checked(cancel)?;
        input.clear();
        if gray {
            input.extend(row.as_chunks::<4>().0.iter().map(|p| p[0]));
        } else {
            input.extend_from_slice(row);
        }
        transform
            .transform(&input, &mut output)
            .map_err(|e| invalid(e.to_string()))?;
        for (original, converted) in row
            .as_chunks_mut::<4>()
            .0
            .iter_mut()
            .zip(output.as_chunks::<4>().0)
        {
            original[..3].copy_from_slice(&converted[..3]);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{document::DEFAULT_DOCUMENT_BUDGET, test_support::TempDir};
    use image::ImageEncoder;
    const BUDGET: usize = DEFAULT_DOCUMENT_BUDGET;
    fn decode(path: &Path, page: usize, orient: bool) -> ImportedPage {
        let cancel = AtomicBool::new(false);
        ImageSource::read(path, BUDGET, &cancel)
            .unwrap()
            .decode(page, orient, BUDGET, &cancel)
            .unwrap()
    }
    #[test]
    fn bmp_device_image_decodes_with_bounds_and_preserves_original_bytes() {
        let root = TempDir::new();
        let path = root.0.join("scan.bmp");
        let mut file = fs::File::create(&path).unwrap();
        image::codecs::bmp::BmpEncoder::new(&mut file)
            .encode(
                &[10, 20, 30, 40, 50, 60],
                2,
                1,
                image::ExtendedColorType::Rgb8,
            )
            .unwrap();
        drop(file);
        let before = fs::read(&path).unwrap();
        let image = decode(&path, 0, true).image;
        assert_eq!(image.pixels, vec![[10, 20, 30, 255], [40, 50, 60, 255]]);
        assert_eq!(fs::read(&path).unwrap(), before);
        let mut huge = before;
        huge[18..22].copy_from_slice(&100000i32.to_le_bytes());
        fs::write(&path, huge).unwrap();
        assert!(ImageSource::read(&path, BUDGET, &AtomicBool::new(false)).is_err());
    }
    #[test]
    fn png_alpha_profile_conversion_source_snapshot_and_native_roundtrip() {
        let root = TempDir::new();
        let path = root.0.join("profile.png");
        let input = [180, 80, 120, 128, 20, 40, 60, 0];
        let mut file = fs::File::create(&path).unwrap();
        let mut encoder = image::codecs::png::PngEncoder::new(&mut file);
        encoder
            .set_icc_profile(ColorProfile::new_display_p3().encode().unwrap())
            .unwrap();
        encoder
            .write_image(&input, 2, 1, image::ExtendedColorType::Rgba8)
            .unwrap();
        drop(file);
        let before = fs::read(&path).unwrap();
        let cancel = AtomicBool::new(false);
        let source = ImageSource::read(&path, BUDGET, &cancel).unwrap();
        let imported = source.decode(0, true, BUDGET, &cancel).unwrap();
        assert!(imported.profile_applied);
        assert!(imported.image.pixels[0][0] > input[0]);
        assert!(imported.image.pixels[0][1] < input[1]);
        assert_eq!(imported.image.pixels[0][3], 128);
        assert_eq!(imported.image.pixels[1][3], 0);
        let mut doc = imported.image.document(BUDGET).unwrap();
        assert!(doc.path.is_none() && doc.source_path.is_none() && doc.dirty);
        let saved = root.0.join("independent.drawer");
        crate::files::save(&mut doc, &saved).unwrap();
        assert_eq!(
            crate::files::open(&saved, BUDGET).unwrap().snapshot(),
            doc.snapshot()
        );
        assert_eq!(fs::read(&path).unwrap(), before);
        fs::write(&path, "changed externally").unwrap();
        assert_eq!(
            source.decode(0, true, BUDGET, &cancel).unwrap().image,
            imported.image
        );
    }
    #[test]
    fn unprofiled_png_keeps_hidden_rgb_and_bad_icc_is_not_silently_ignored() {
        let root = TempDir::new();
        let path = root.0.join("plain.png");
        image::save_buffer(
            &path,
            &[11, 22, 33, 0, 44, 55, 66, 128],
            2,
            1,
            image::ColorType::Rgba8,
        )
        .unwrap();
        let imported = decode(&path, 0, true);
        assert!(!imported.profile_applied);
        assert_eq!(imported.image.pixels, [[11, 22, 33, 0], [44, 55, 66, 128]]);
        let mut bytes = Vec::new();
        let mut encoder = image::codecs::png::PngEncoder::new(&mut bytes);
        encoder.set_icc_profile(vec![0; 128]).unwrap();
        encoder
            .write_image(&[1, 2, 3, 255], 1, 1, image::ExtendedColorType::Rgba8)
            .unwrap();
        fs::write(&path, bytes).unwrap();
        let cancel = AtomicBool::new(false);
        let source = ImageSource::read(&path, BUDGET, &cancel).unwrap();
        assert!(
            source
                .decode(0, true, BUDGET, &cancel)
                .err()
                .unwrap()
                .to_string()
                .contains("ICC")
        );
    }
    #[test]
    fn jpeg_orientation_choice_matches_preview_and_preserves_original_bytes() {
        let root = TempDir::new();
        let path = root.0.join("oriented.jpg");
        let pixels = [
            255, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 0, 255, 0, 255, 0, 255, 255,
        ];
        let mut jpeg = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 100)
            .encode(&pixels, 2, 3, image::ExtendedColorType::Rgb8)
            .unwrap();
        // Minimal little-endian TIFF EXIF with Orientation = 6, in an APP1 segment.
        let mut exif =
            b"Exif\0\0II\x2a\0\x08\0\0\0\x01\0\x12\x01\x03\0\x01\0\0\0\x06\0\0\0\0\0\0\0".to_vec();
        let mut data = jpeg[..2].to_vec();
        data.extend_from_slice(&[0xff, 0xe1]);
        data.extend_from_slice(&((exif.len() + 2) as u16).to_be_bytes());
        data.append(&mut exif);
        data.extend_from_slice(&jpeg[2..]);
        fs::write(&path, &data).unwrap();
        let raw = decode(&path, 0, false);
        let turned = decode(&path, 0, true);
        assert_eq!(raw.orientation, Orientation::Rotate90);
        assert_eq!((raw.image.width, raw.image.height), (2, 3));
        assert_eq!((turned.image.width, turned.image.height), (3, 2));
        let p = &raw.image.pixels;
        assert_eq!(turned.image.pixels, [p[4], p[2], p[0], p[5], p[3], p[1]]);
        assert_eq!(fs::read(&path).unwrap(), data);
    }
    #[test]
    fn tiff_page_selection_16bit_orientation_alpha_and_source_preservation() {
        use tiff::encoder::{TiffEncoder, colortype};
        let root = TempDir::new();
        let path = root.0.join("pages.tiff");
        let mut file = fs::File::create(&path).unwrap();
        let mut encoder = TiffEncoder::new(&mut file).unwrap();
        encoder
            .write_image::<colortype::RGB8>(2, 1, &[255, 0, 0, 0, 255, 0])
            .unwrap();
        let mut page = encoder.new_image::<colortype::RGBA16>(1, 2).unwrap();
        page.encoder()
            .write_tag(Tag::ExtraSamples, &[2u16][..])
            .unwrap();
        page.encoder().write_tag(Tag::Orientation, 6u16).unwrap();
        page.write_data(&[2570, 5140, 7710, 32896, 10280, 12850, 15420, 0])
            .unwrap();
        drop(file);
        let before = fs::read(&path).unwrap();
        let cancel = AtomicBool::new(false);
        let source = ImageSource::read(&path, BUDGET, &cancel).unwrap();
        assert_eq!(source.pages.len(), 2);
        assert_eq!(
            decode(&path, 0, true).image.pixels,
            [[255, 0, 0, 255], [0, 255, 0, 255]]
        );
        let page = decode(&path, 1, true);
        assert_eq!((page.image.width, page.image.height), (2, 1));
        assert_eq!(page.image.pixels, [[40, 50, 60, 0], [10, 20, 30, 128]]);
        assert_eq!(fs::read(&path).unwrap(), before);
        assert!(source.decode(2, true, BUDGET, &cancel).is_err());
    }
    #[test]
    fn size_corruption_missing_and_cancellation_are_bounded() {
        let root = TempDir::new();
        let path = root.0.join("large.png");
        image::save_buffer(
            &path,
            &vec![0; 32768 * 4],
            32768,
            1,
            image::ColorType::Rgba8,
        )
        .unwrap();
        let cancel = AtomicBool::new(false);
        let source = ImageSource::read(&path, BUDGET, &cancel).unwrap();
        assert_eq!(
            source.decode(0, true, BUDGET, &cancel).unwrap().image.width,
            32768
        );
        assert!(source.decode(0, true, 65536, &cancel).is_err());
        cancel.store(true, Ordering::Relaxed);
        assert_eq!(
            source
                .decode(0, true, BUDGET, &cancel)
                .err()
                .unwrap()
                .kind(),
            io::ErrorKind::Interrupted
        );
        assert!(ImageSource::read(&path, BUDGET, &cancel).is_err());
        cancel.store(false, Ordering::Relaxed);
        image::save_buffer(
            &path,
            &vec![0; 32769 * 4],
            32769,
            1,
            image::ColorType::Rgba8,
        )
        .unwrap();
        assert!(ImageSource::read(&path, BUDGET, &cancel).is_err());
        fs::write(&path, "damaged").unwrap();
        assert!(ImageSource::read(&path, BUDGET, &cancel).is_err());
        assert!(ImageSource::read(&root.0.join("missing.png"), BUDGET, &cancel).is_err());
    }
    #[test]
    fn tiff_gray_rgb_8_16_and_associated_alpha_variants_have_verified_samples() {
        use tiff::{
            encoder::{TiffEncoder, colortype},
            tags::ExtraSamples,
        };
        let root = TempDir::new();
        let path = root.0.join("variants.tiff");
        {
            let mut encoder = TiffEncoder::new(fs::File::create(&path).unwrap()).unwrap();
            let mut gray = encoder.new_image::<colortype::Gray8>(2, 1).unwrap();
            gray.encoder()
                .write_tag(
                    Tag::IccProfile,
                    ColorProfile::new_gray_with_gamma(1.0)
                        .encode()
                        .unwrap()
                        .as_slice(),
                )
                .unwrap();
            gray.write_data(&[64, 128]).unwrap();
            encoder
                .write_image::<colortype::Gray16>(1, 1, &[25700])
                .unwrap();
            let mut alpha = encoder.new_image::<colortype::Gray8>(1, 2).unwrap();
            alpha
                .extra_samples(&[ExtraSamples::UnassociatedAlpha])
                .unwrap();
            alpha.write_data(&[60, 128, 100, 0]).unwrap();
            let mut alpha = encoder.new_image::<colortype::Gray16>(1, 1).unwrap();
            alpha
                .extra_samples(&[ExtraSamples::UnassociatedAlpha])
                .unwrap();
            alpha.write_data(&[7710, 32896]).unwrap();
            encoder
                .write_image::<colortype::RGB16>(1, 1, &[2570, 5140, 7710])
                .unwrap();
            let mut rgba = encoder.new_image::<colortype::RGBA8>(1, 1).unwrap();
            rgba.encoder()
                .write_tag(Tag::ExtraSamples, &[1u16][..])
                .unwrap();
            rgba.write_data(&[10, 20, 30, 128]).unwrap();
            encoder
                .write_image::<colortype::RGB32Float>(1, 1, &[0.1, 0.2, 0.3])
                .unwrap();
        }
        let gray = decode(&path, 0, true);
        assert!(gray.profile_applied);
        assert!(gray.image.pixels[0][0] > 64);
        assert_eq!(gray.image.pixels[0][3], 255);
        for (page, expected) in [
            (1, vec![[100, 100, 100, 255]]),
            (2, vec![[60, 60, 60, 128], [100, 100, 100, 0]]),
            (3, vec![[30, 30, 30, 128]]),
            (4, vec![[10, 20, 30, 255]]),
            (5, vec![[20, 40, 60, 128]]),
        ] {
            assert_eq!(decode(&path, page, true).image.pixels, expected);
        }
        let cancel = AtomicBool::new(false);
        let source = ImageSource::read(&path, BUDGET, &cancel).unwrap();
        assert!(source.decode(6, true, BUDGET, &cancel).is_err());
    }
}
