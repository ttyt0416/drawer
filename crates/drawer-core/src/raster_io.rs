//! Explicit still-image interchange; all options affect only an independent export.
use crate::{
    color_precision::{Depth, Sample},
    document::Document,
};
use image::ImageEncoder;
use std::{
    fs,
    io::{Cursor, Write},
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
};
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Format {
    #[default]
    Jpeg,
    Bmp,
    Tga,
    Tiff,
    Webp,
}
impl Format {
    pub const ALL: [Self; 5] = [Self::Jpeg, Self::Bmp, Self::Tga, Self::Tiff, Self::Webp];
    pub fn extension(self) -> &'static str {
        match self {
            Self::Jpeg => "jpg",
            Self::Bmp => "bmp",
            Self::Tga => "tga",
            Self::Tiff => "tif",
            Self::Webp => "webp",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Jpeg => "JPEG (lossy RGB8)",
            Self::Bmp => "BMP (RGB/RGBA8)",
            Self::Tga => "TGA (RGB/RGBA8)",
            Self::Tiff => "TIFF (RGB/RGBA8/16)",
            Self::Webp => "WebP (lossless RGB/RGBA8)",
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Compression {
    None,
    Lzw,
    #[default]
    Deflate,
    Packbits,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Options {
    pub format: Format,
    pub depth: Depth,
    pub alpha: bool,
    pub background: [u8; 3],
    pub quality: u8,
    pub rle: bool,
    pub compression: Compression,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            format: Format::Jpeg,
            depth: Depth::Eight,
            alpha: false,
            background: [255; 3],
            quality: 90,
            rle: true,
            compression: Compression::Deflate,
        }
    }
}
impl Options {
    pub fn valid(self) -> bool {
        (1..=100).contains(&self.quality)
            && (self.format != Format::Jpeg || !self.alpha)
            && (self.format == Format::Tiff || self.depth == Depth::Eight)
    }
}
pub struct Prepared {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<[u16; 4]>,
    pub source_depth: Depth,
    source: u64,
    revision: u64,
    frame: u32,
}
fn cancel(c: &AtomicBool) -> Result<(), String> {
    if c.load(Ordering::Relaxed) {
        Err("Raster export canceled; original and destination unchanged".into())
    } else {
        Ok(())
    }
}
pub fn prepare(d: &Document, budget: usize, c: &AtomicBool) -> Result<Prepared, String> {
    let count = d.width as usize * d.height as usize;
    if count > 4 * 1024 * 1024
        || count
            .saturating_mul(48)
            .saturating_add(d.memory_bytes().saturating_mul(4))
            > budget
        || !d.color.valid_layers(&d.layers)
        || !d.scene.valid()
    {
        return Err("Invalid document or still-image export work/memory limit".into());
    }
    let renderer = crate::render::Renderer::for_export(d)?;
    let mut pixels = Vec::with_capacity(count);
    for y in 0..d.height {
        cancel(c)?;
        for x in 0..d.width {
            let point = [x as f64 + 0.5, y as f64 + 0.5];
            let mut value = renderer.display_sample(point)?;
            let page = &d.scene.comics.page;
            if page.include_guides || page.include_number {
                value = value.over(
                    Sample::from8(page.output_pixel(d, point)),
                    crate::layer_blend::Mode::Normal,
                    1.,
                )?;
            }
            pixels.push(value.rgba16());
        }
    }
    Ok(Prepared {
        width: d.width,
        height: d.height,
        pixels,
        source_depth: d.color.depth,
        source: d.id,
        revision: d.revision,
        frame: d.current_frame,
    })
}
impl Prepared {
    pub fn check(&self, d: &Document) -> Result<(), String> {
        if (self.source, self.revision, self.frame) == (d.id, d.revision, d.current_frame) {
            Ok(())
        } else {
            Err("Document/frame changed; prepare raster export again".into())
        }
    }
    pub fn losses(&self, o: Options) -> Vec<String> {
        let mut s=vec!["Exported pixels are sRGB; out-of-range RGB is clipped. Source objects, metadata, monitor calibration and Undo history are not exported or modified.".into()];
        if o.format == Format::Jpeg {
            s.push(format!("JPEG is lossy RGB8, quality {}. Alpha is composited on the selected background; preview does not simulate JPEG artifacts.",o.quality));
        } else if !o.alpha {
            s.push("Alpha is composited on the selected background".into());
        }
        if self.source_depth == Depth::Sixteen && o.depth == Depth::Eight {
            s.push("16-bit source channels are quantized to 8-bit only in this file".into());
        }
        if self.source_depth == Depth::Eight && o.depth == Depth::Sixteen {
            s.push("8 to16 bit expands numbers without adding detail".into());
        }
        if o.format == Format::Tga {
            s.push("TGA has no embedded ICC; these pixels are sRGB".into());
        }
        if o.format == Format::Webp {
            s.push("Lossless WebP only; fully transparent hidden RGB may be normalized".into());
        }
        s
    }
    pub fn sample(&self, n: usize, o: Options) -> Result<Sample, String> {
        let p = Sample::from16(*self.pixels.get(n).ok_or("Export sample index")?);
        if o.alpha {
            Ok(p)
        } else {
            Sample::from8([o.background[0], o.background[1], o.background[2], 255]).over(
                p,
                crate::layer_blend::Mode::Normal,
                1.,
            )
        }
    }
    pub fn encode(&self, o: Options, budget: usize, c: &AtomicBool) -> Result<Vec<u8>, String> {
        if !o.valid()
            || self.pixels.len() != self.width as usize * self.height as usize
            || self.pixels.len().saturating_mul(40) > budget
        {
            return Err("Invalid export options or codec memory reserve".into());
        }
        cancel(c)?;
        let channels = if o.alpha { 4 } else { 3 };
        let mut low = Vec::with_capacity(self.pixels.len() * channels);
        let mut high = Vec::new();
        if o.depth == Depth::Sixteen {
            high.reserve(self.pixels.len() * channels);
        }
        for n in 0..self.pixels.len() {
            if n.is_multiple_of(4096) {
                cancel(c)?;
            }
            let p = self.sample(n, o)?;
            if o.depth == Depth::Sixteen {
                high.extend(&p.rgba16()[..channels]);
            } else {
                low.extend(&p.rgba8()[..channels]);
            }
        }
        let mut bytes = vec![];
        let icc = || -> Result<Vec<u8>, String> {
            let mut p = moxcms::ColorProfile::new_srgb()
                .encode()
                .map_err(|e| e.to_string())?;
            for (s, n) in p[24..36]
                .as_chunks_mut::<2>()
                .0
                .iter_mut()
                .zip([2000u16, 1, 1, 0, 0, 0])
            {
                s.copy_from_slice(&n.to_be_bytes());
            }
            Ok(p)
        };
        let color = if o.alpha {
            image::ExtendedColorType::Rgba8
        } else {
            image::ExtendedColorType::Rgb8
        };
        match o.format {
            Format::Jpeg => {
                let mut e =
                    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, o.quality);
                e.set_icc_profile(icc()?).map_err(|e| e.to_string())?;
                e.encode(&low, self.width, self.height, color)
                    .map_err(|e| e.to_string())?;
            }
            Format::Bmp => image::codecs::bmp::BmpEncoder::new(&mut bytes)
                .encode(&low, self.width, self.height, color)
                .map_err(|e| e.to_string())?,
            Format::Tga => {
                let e = image::codecs::tga::TgaEncoder::new(&mut bytes);
                let e = if o.rle { e } else { e.disable_rle() };
                e.encode(&low, self.width, self.height, color)
                    .map_err(|e| e.to_string())?;
            }
            Format::Webp => {
                let mut e = image_webp::WebPEncoder::new(&mut bytes);
                e.set_icc_profile(icc()?);
                e.encode(
                    &low,
                    self.width,
                    self.height,
                    if o.alpha {
                        image_webp::ColorType::Rgba8
                    } else {
                        image_webp::ColorType::Rgb8
                    },
                )
                .map_err(|e| e.to_string())?;
            }
            Format::Tiff => {
                use tiff::encoder::{Compression as C, TiffEncoder, colortype};
                let compression = match o.compression {
                    Compression::None => C::Uncompressed,
                    Compression::Lzw => C::Lzw,
                    Compression::Deflate => C::Deflate(Default::default()),
                    Compression::Packbits => C::Packbits,
                };
                let mut encoder = TiffEncoder::new(Cursor::new(&mut bytes))
                    .map_err(|e| e.to_string())?
                    .with_compression(compression);
                let icc = icc()?;
                macro_rules! write_tiff {
                    ($ty:ty,$data:expr) => {{
                        let mut image = encoder
                            .new_image::<$ty>(self.width, self.height)
                            .map_err(|e| e.to_string())?;
                        if o.alpha {
                            image
                                .encoder()
                                .write_tag(tiff::tags::Tag::ExtraSamples, &[2u16][..])
                                .map_err(|e| e.to_string())?;
                        }
                        image
                            .encoder()
                            .write_tag(tiff::tags::Tag::IccProfile, icc.as_slice())
                            .map_err(|e| e.to_string())?;
                        image.write_data($data).map_err(|e| e.to_string())?;
                    }};
                }
                match (o.depth, o.alpha) {
                    (Depth::Eight, false) => write_tiff!(colortype::RGB8, &low),
                    (Depth::Eight, true) => write_tiff!(colortype::RGBA8, &low),
                    (Depth::Sixteen, false) => write_tiff!(colortype::RGB16, &high),
                    (Depth::Sixteen, true) => write_tiff!(colortype::RGBA16, &high),
                }
            }
        }
        cancel(c)?;
        if bytes.len() > 64 * 1024 * 1024 || bytes.len() > budget / 2 {
            return Err("Encoded still image exceeds output memory limit".into());
        }
        Ok(bytes)
    }
    #[allow(clippy::too_many_arguments)]
    pub fn write(
        &self,
        d: &Document,
        path: &Path,
        o: Options,
        confirmed: bool,
        replace: bool,
        budget: usize,
        c: &AtomicBool,
    ) -> Result<(), String> {
        crate::exchange_guidance::reject_private(path)?;
        self.check(d)?;
        if !confirmed {
            return Err("Confirm the listed export losses".into());
        }
        let ext = path
            .extension()
            .unwrap_or_default()
            .to_string_lossy()
            .to_ascii_lowercase();
        if ext != o.format.extension()
            && !(o.format == Format::Jpeg && ext == "jpeg"
                || o.format == Format::Tiff && ext == "tiff")
        {
            return Err(format!("Choose a .{} filename", o.format.extension()));
        }
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."))
            .canonicalize()
            .map_err(|e| e.to_string())?;
        if !crate::font_library::local(&parent) {
            return Err("Choose a local export folder".into());
        }
        let path = parent.join(path.file_name().ok_or("Missing export filename")?);
        let check = || -> Result<(), String> {
            if let Ok(m) = fs::symlink_metadata(&path) {
                if !m.is_file() || m.file_type().is_symlink() {
                    return Err("Choose a regular export file".into());
                }
                if !replace {
                    return Err("Confirm replacement of this exported file".into());
                }
            }
            let target = path.canonicalize().unwrap_or_else(|_| path.clone());
            for source in d.path.iter().chain(d.source_path.iter()) {
                let resolved = source
                    .canonicalize()
                    .or_else(|_| {
                        source
                            .parent()
                            .unwrap_or(Path::new("."))
                            .canonicalize()
                            .map(|p| p.join(source.file_name().unwrap_or_default()))
                    })
                    .map_err(|e| e.to_string())?;
                if resolved
                    .to_string_lossy()
                    .eq_ignore_ascii_case(&target.to_string_lossy())
                {
                    return Err(
                        "Preserve the original source; choose another output filename".into(),
                    );
                }
            }
            Ok(())
        };
        check()?;
        let bytes = self.encode(o, budget, c)?;
        let mut name = path.file_name().unwrap().to_owned();
        name.push(".raster-write.tmp");
        let temp = path.with_file_name(name);
        let mut file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temp)
            .map_err(|e| e.to_string())?;
        let result = (|| {
            for chunk in bytes.chunks(65536) {
                cancel(c)?;
                file.write_all(chunk).map_err(|e| e.to_string())?;
            }
            file.sync_all().map_err(|e| e.to_string())?;
            drop(file);
            cancel(c)?;
            check()?;
            if replace {
                fs::rename(&temp, &path).map_err(|e| e.to_string())
            } else {
                fs::hard_link(&temp, &path).map_err(|e| e.to_string())
            }
        })();
        let _ = fs::remove_file(temp);
        result
    }
}
pub(crate) fn validate_tga(b: &[u8]) -> Result<(), String> {
    if b.len() < 18
        || b[1] != 0
        || !matches!(b[2], 2 | 3 | 10 | 11)
        || !matches!((b[2], b[16]), (2 | 10, 24 | 32) | (3 | 11, 8))
        || b[17] & 0xc0 != 0
        || !matches!(b[17] & 15, 0 | 8)
        || (b[16] != 32 && b[17] & 15 != 0)
        || b.len() < 18 + b[0] as usize
    {
        return Err("Verified TGA import supports unmapped gray8/RGB24/RGBA32, raw/RLE, without interleaving".into());
    }
    Ok(())
}
#[path = "raster_io_verify.rs"]
pub mod verify;
