//! Explicit still-image interchange; all options affect only an independent export.
use crate::{
    color_precision::{Depth, Sample},
    document::Document,
};
use std::{
    fs,
    io::{Cursor, Write},
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
};
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Format {
    Png,
    #[default]
    Jpeg,
    Bmp,
    Tga,
    Tiff,
    Webp,
}
impl Format {
    pub const ALL: [Self; 6] = [
        Self::Png,
        Self::Jpeg,
        Self::Bmp,
        Self::Tga,
        Self::Tiff,
        Self::Webp,
    ];
    pub fn extension(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Jpeg => "jpg",
            Self::Bmp => "bmp",
            Self::Tga => "tga",
            Self::Tiff => "tif",
            Self::Webp => "webp",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Png => "PNG (lossless RGB/RGBA8/16)",
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
    pub chroma_quality: u8,
    pub sampling: crate::jpeg_output::Sampling,
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
            chroma_quality: 90,
            sampling: Default::default(),
            rle: true,
            compression: Compression::Deflate,
        }
    }
}
impl Options {
    pub fn valid(self) -> bool {
        (1..=100).contains(&self.quality)
            && (1..=100).contains(&self.chroma_quality)
            && (self.format != Format::Jpeg || !self.alpha)
            && (matches!(self.format, Format::Tiff | Format::Png) || self.depth == Depth::Eight)
            && (self.format != Format::Png
                || matches!(self.compression, Compression::None | Compression::Deflate))
    }
}
#[derive(Clone)]
pub struct Prepared {
    source_losses: crate::export_loss::Source,
    color_losses: Vec<String>,
    pub(crate) icc: Option<std::sync::Arc<Vec<u8>>>,
    pub(crate) display: Option<std::sync::Arc<crate::color_profile::Transform>>,
    pub(crate) matte: Option<Sample>,
    pub(crate) output_label: String,
    pub(crate) color_notices: Vec<String>,
    pub dpi: f64,
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<[u16; 4]>,
    pub source_depth: Depth,
    source: u64,
    revision: u64,
    frame: u32,
}
pub struct Encoded {
    source: (u64, u64, u32),
    options: Options,
    bytes: Vec<u8>,
    pub size: [usize; 2],
    pub pixels: Vec<[u8; 4]>,
}
impl Encoded {
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
    pub fn options(&self) -> Options {
        self.options
    }
}
fn cancel(c: &AtomicBool) -> Result<(), String> {
    if c.load(Ordering::Relaxed) {
        Err("Raster export canceled; original and destination unchanged".into())
    } else {
        Ok(())
    }
}
pub fn prepare(d: &Document, budget: usize, c: &AtomicBool) -> Result<Prepared, String> {
    prepare_inner(d, budget, c, None)
}
pub fn prepare_managed(
    d: &Document,
    budget: usize,
    c: &AtomicBool,
    settings: &crate::export_color::Settings,
    o: Options,
) -> Result<Prepared, String> {
    let stage = crate::export_color::Stage::new(d, settings, o.format)?;
    let matte = stage.matte(o.background)?;
    let mut p = prepare_inner(d, budget, c, Some(&stage))?;
    p.matte = Some(matte);
    Ok(p)
}
fn prepare_inner(
    d: &Document,
    budget: usize,
    c: &AtomicBool,
    stage: Option<&crate::export_color::Stage>,
) -> Result<Prepared, String> {
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
    let space = crate::color_profile::WorkingSpace::new(&d.color.profile)?;
    let mut clipped = 0usize;
    for y in 0..d.height {
        cancel(c)?;
        for x in 0..d.width {
            let point = [x as f64 + 0.5, y as f64 + 0.5];
            let mut value = if stage.is_some() {
                renderer.sample(point)?
            } else {
                renderer.display_sample(point)?
            };
            let page = &d.scene.comics.page;
            if page.include_guides || page.include_number {
                let guide = Sample::from8(page.output_pixel(d, point));
                value = value.over(
                    if stage.is_some() {
                        space.from_srgb(guide)?
                    } else {
                        guide
                    },
                    crate::layer_blend::Mode::Normal,
                    1.,
                )?;
            }
            if let Some(stage) = stage {
                value = stage.map(value)?;
            }
            if value.rgb().into_iter().any(|v| !(0.0..=1.0).contains(&v)) {
                clipped += 1;
            }
            pixels.push(value.rgba16());
        }
    }
    let mut color_notices = stage.map_or_else(Vec::new, |s| s.notices.clone());
    if clipped > 0 {
        color_notices.push(format!(
            "{clipped} output samples exceed the RGB gamut and are clipped only in this file"
        ));
    }
    let mut color_losses = stage.map_or_else(
        || {
            if d.color
                .profile
                .equivalent(&crate::color_profile::RgbProfile::Srgb)
            {
                vec![]
            } else {
                vec![
                    "Color space: working RGB is converted to sRGB only in the exported composite"
                        .into(),
                ]
            }
        },
        |s| s.losses.clone(),
    );
    color_losses.extend(
        color_notices
            .iter()
            .filter(|s| s.contains("are clipped"))
            .cloned(),
    );
    Ok(Prepared {
        source_losses: crate::export_loss::Source::new(d)?,
        color_losses,
        icc: if let Some(s) = stage {
            s.icc.clone()
        } else {
            Some(std::sync::Arc::new(crate::export_color::profile_bytes(
                &crate::color_profile::RgbProfile::Srgb,
            )?))
        },
        display: stage.map(|s| s.display.clone()),
        matte: None,
        output_label: stage.map_or_else(|| "sRGB".into(), |s| s.label.clone()),
        color_notices,
        dpi: d.dpi,
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
    pub fn resized(
        &self,
        width: u32,
        height: u32,
        dpi: f64,
        budget: usize,
        c: &AtomicBool,
    ) -> Result<Self, String> {
        let count = (width as usize).saturating_mul(height as usize);
        if width == 0
            || height == 0
            || width > 32768
            || height > 32768
            || count > 4 * 1024 * 1024
            || !dpi.is_finite()
            || !(1.0..=4800.).contains(&dpi)
            || count.saturating_add(self.pixels.len()).saturating_mul(16) > budget
        {
            return Err("Output dimensions/DPI or resize memory limit".into());
        }
        cancel(c)?;
        if (width, height) == (self.width, self.height) {
            let mut same = self.clone();
            same.dpi = dpi;
            return Ok(same);
        }
        let mut pixels = Vec::with_capacity(count);
        let step = [
            self.width as f64 / width as f64,
            self.height as f64 / height as f64,
        ];
        for y in 0..height {
            cancel(c)?;
            for x in 0..width {
                let point = [
                    (x as f64 + 0.5) * step[0] - 0.5,
                    (y as f64 + 0.5) * step[1] - 0.5,
                ];
                let v = crate::interpolation::sample_float(
                    point,
                    crate::interpolation::box_footprint(point, step),
                    crate::interpolation::Filter::Bilinear,
                    |x, y| {
                        let x = x.clamp(0, self.width as i64 - 1) as usize;
                        let y = y.clamp(0, self.height as i64 - 1) as usize;
                        Sample::from16(self.pixels[y * self.width as usize + x])
                    },
                );
                pixels.push(v.rgba16());
            }
        }
        Ok(Self {
            source_losses: self.source_losses.clone(),
            color_losses: self.color_losses.clone(),
            icc: self.icc.clone(),
            display: self.display.clone(),
            matte: self.matte,
            output_label: self.output_label.clone(),
            color_notices: self.color_notices.clone(),
            width,
            height,
            dpi,
            pixels,
            source_depth: self.source_depth,
            source: self.source,
            revision: self.revision,
            frame: self.frame,
        })
    }
    pub fn check(&self, d: &Document) -> Result<(), String> {
        if (self.source, self.revision, self.frame) == (d.id, d.revision, d.current_frame) {
            Ok(())
        } else {
            Err("Document/frame changed; prepare raster export again".into())
        }
    }
    pub fn losses(&self, o: Options) -> Vec<String> {
        let mut losses = self.source_losses.losses();
        losses.extend(self.color_losses.clone());
        if self.source_depth == Depth::Sixteen && o.depth == Depth::Eight {
            losses.push(
                "Bit depth: 16-bit source channels are quantized to 8-bit in this file".into(),
            );
        }
        if !o.alpha && self.pixels.iter().any(|p| p[3] < 65535) {
            losses.push(
                "Alpha: transparency is composited onto the selected opaque background".into(),
            );
        }
        if o.format == Format::Jpeg {
            losses.push(format!(
                "Compression: JPEG luma/chroma quality {}/{} and {} use lossy RGB8 encoding",
                o.quality,
                o.chroma_quality,
                o.sampling.label()
            ));
        }
        losses
    }
    pub fn notices(&self, o: Options) -> Vec<String> {
        let mut s = vec![format!(
            "Exported pixels use {}; source objects, metadata, monitor calibration and Undo history are not modified.",
            self.output_label
        )];
        s.extend(self.color_notices.clone());
        if o.format == Format::Jpeg {
            s.push(format!("JPEG baseline RGB8: luma quality {}, chroma quality {}, sampling {}. Alpha is composited before encoding. The compression preview decodes the exact output bytes.",o.quality,o.chroma_quality,o.sampling.label()));
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
            s.push(format!(
                "TGA has no embedded ICC; numeric pixels use {}",
                self.output_label
            ));
        }
        if o.format == Format::Webp {
            s.push("Lossless WebP only; fully transparent hidden RGB may be normalized".into());
        }
        if matches!(o.format, Format::Png | Format::Jpeg | Format::Tiff) {
            let actual = match o.format {
                Format::Png => (self.dpi / 0.0254).round() * 0.0254,
                Format::Jpeg => self.dpi.round(),
                _ => (self.dpi * 10000.).round() / 10000.,
            };
            if (actual - self.dpi).abs() > 1e-8 {
                s.push(format!("Resolution metadata approximates {:.8}DPI as {:.8}DPI; source DPI is unchanged",self.dpi,actual));
            }
        } else {
            s.push(
                "This output adapter does not store DPI metadata; pixel dimensions remain exact"
                    .into(),
            );
        }
        s
    }
    pub fn sample(&self, n: usize, o: Options) -> Result<Sample, String> {
        let p = Sample::from16(*self.pixels.get(n).ok_or("Export sample index")?);
        if o.alpha {
            Ok(p)
        } else {
            self.matte
                .unwrap_or_else(|| {
                    Sample::from8([o.background[0], o.background[1], o.background[2], 255])
                })
                .over(p, crate::layer_blend::Mode::Normal, 1.)
        }
    }
    pub fn preview_sample(&self, n: usize, o: Options) -> Result<Sample, String> {
        let p = self.sample(n, o)?;
        let p = if o.depth == Depth::Eight {
            Sample::from8(p.rgba8())
        } else {
            Sample::from16(p.rgba16())
        };
        self.display.as_ref().map_or(Ok(p), |t| t.map(p))
    }
    pub fn encode(&self, o: Options, budget: usize, c: &AtomicBool) -> Result<Vec<u8>, String> {
        if !o.valid()
            || !self.dpi.is_finite()
            || !(1.0..=4800.0).contains(&self.dpi)
            || self.pixels.len() != self.width as usize * self.height as usize
            || self.pixels.len().saturating_mul(40) > budget
        {
            return Err("Invalid export options or codec memory reserve".into());
        }
        cancel(c)?;
        let channels = if o.alpha { 4 } else { 3 };
        let mut low = Vec::with_capacity(self.pixels.len() * channels);
        let mut high: Vec<u16> = Vec::new();
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
        let color = if o.alpha {
            image::ExtendedColorType::Rgba8
        } else {
            image::ExtendedColorType::Rgb8
        };
        match o.format {
            Format::Png => {
                let mut info = png::Info::with_size(self.width, self.height);
                info.color_type = if o.alpha {
                    png::ColorType::Rgba
                } else {
                    png::ColorType::Rgb
                };
                info.bit_depth = if o.depth == Depth::Sixteen {
                    png::BitDepth::Sixteen
                } else {
                    png::BitDepth::Eight
                };
                info.icc_profile = self
                    .icc
                    .as_ref()
                    .map(|b| std::borrow::Cow::Owned(b.as_ref().clone()));
                let ppm = (self.dpi / 0.0254).round() as u32;
                info.pixel_dims = Some(png::PixelDimensions {
                    xppu: ppm,
                    yppu: ppm,
                    unit: png::Unit::Meter,
                });
                let mut encoder =
                    png::Encoder::with_info(&mut bytes, info).map_err(|e| e.to_string())?;
                encoder.set_compression(if o.compression == Compression::None {
                    png::Compression::NoCompression
                } else {
                    png::Compression::Balanced
                });
                let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
                let mut precise = Vec::new();
                let data = if o.depth == Depth::Sixteen {
                    precise.reserve(high.len() * 2);
                    for v in &high {
                        precise.extend(v.to_be_bytes());
                    }
                    &precise
                } else {
                    &low
                };
                writer.write_image_data(data).map_err(|e| e.to_string())?;
                writer.finish().map_err(|e| e.to_string())?;
            }
            Format::Jpeg => {
                bytes = crate::jpeg_output::encode(
                    crate::jpeg_output::Image {
                        rgb: &low,
                        width: self.width,
                        height: self.height,
                        dpi: self.dpi,
                        icc: self.icc.as_deref().map(Vec::as_slice),
                    },
                    crate::jpeg_output::Settings {
                        luma: o.quality,
                        chroma: o.chroma_quality,
                        sampling: o.sampling,
                    },
                    budget,
                    c,
                )?;
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
                if let Some(icc) = &self.icc {
                    e.set_icc_profile(icc.as_ref().clone());
                }
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
                let icc = self.icc.as_deref();
                macro_rules! write_tiff {
                    ($ty:ty,$data:expr) => {{
                        let mut image = encoder
                            .new_image::<$ty>(self.width, self.height)
                            .map_err(|e| e.to_string())?;
                        image.resolution(
                            tiff::tags::ResolutionUnit::Inch,
                            tiff::encoder::Rational {
                                n: (self.dpi * 10000.).round() as u32,
                                d: 10000,
                            },
                        );
                        if o.alpha {
                            image
                                .encoder()
                                .write_tag(tiff::tags::Tag::ExtraSamples, &[2u16][..])
                                .map_err(|e| e.to_string())?;
                        }
                        if let Some(icc) = icc {
                            image
                                .encoder()
                                .write_tag(tiff::tags::Tag::IccProfile, icc.as_slice())
                                .map_err(|e| e.to_string())?;
                        }
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
    pub fn encoded_preview(
        &self,
        o: Options,
        budget: usize,
        c: &AtomicBool,
    ) -> Result<Encoded, String> {
        if o.format != Format::Jpeg {
            return Err("Actual compression preview is for JPEG; lossless adapters use their exact prepared channel preview".into());
        }
        let bytes = self.encode(o, budget, c)?;
        let (size, pixels) = self.preview_bytes(&bytes, o, [300, 100], budget, c)?;
        Ok(Encoded {
            source: (self.source, self.revision, self.frame),
            options: o,
            bytes,
            size,
            pixels,
        })
    }
    pub fn preview_bytes(
        &self,
        bytes: &[u8],
        o: Options,
        max: [u32; 2],
        budget: usize,
        c: &AtomicBool,
    ) -> Result<([usize; 2], Vec<[u8; 4]>), String> {
        if max.contains(&0) || max.iter().any(|n| *n > 1024) {
            return Err("Invalid preview dimensions".into());
        }
        cancel(c)?;
        let decoded = if o.format == Format::Jpeg {
            if bytes.len() > 64 * 1024 * 1024
                || self
                    .pixels
                    .len()
                    .saturating_mul(24)
                    .saturating_add(bytes.len())
                    > budget
            {
                return Err("JPEG readback exceeds current preview memory".into());
            }
            let mut reader =
                image::ImageReader::with_format(Cursor::new(bytes), image::ImageFormat::Jpeg);
            let mut limits = image::Limits::default();
            limits.max_image_width = Some(self.width);
            limits.max_image_height = Some(self.height);
            limits.max_alloc = Some((budget / 2) as u64);
            reader.limits(limits);
            let image = reader.decode().map_err(|e| e.to_string())?.into_rgba8();
            if image.dimensions() != (self.width, self.height) {
                return Err("JPEG readback dimensions differ from output plan".into());
            }
            Some(image)
        } else {
            None
        };
        let step = self
            .width
            .div_ceil(max[0])
            .max(self.height.div_ceil(max[1]))
            .max(1);
        let size = [
            self.width.div_ceil(step) as usize,
            self.height.div_ceil(step) as usize,
        ];
        let mut pixels = Vec::with_capacity(size[0] * size[1]);
        for y in 0..size[1] {
            cancel(c)?;
            for x in 0..size[0] {
                let n = y * step as usize * self.width as usize + x * step as usize;
                let value = if let Some(image) = &decoded {
                    let raw = Sample::from8(image.get_pixel(x as u32 * step, y as u32 * step).0);
                    self.display.as_ref().map_or(Ok(raw), |t| t.map(raw))?
                } else {
                    self.preview_sample(n, o)?
                };
                pixels.push(value.rgba8());
            }
        }
        cancel(c)?;
        Ok((size, pixels))
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
        self.write_selected(d, path, o, confirmed, replace, budget, c, None)
    }
    #[allow(clippy::too_many_arguments)]
    pub fn write_encoded(
        &self,
        d: &Document,
        path: &Path,
        encoded: &Encoded,
        confirmed: bool,
        replace: bool,
        budget: usize,
        c: &AtomicBool,
    ) -> Result<(), String> {
        if encoded.source != (self.source, self.revision, self.frame) {
            return Err("Encoded preview belongs to another source snapshot".into());
        }
        self.write_selected(
            d,
            path,
            encoded.options,
            confirmed,
            replace,
            budget,
            c,
            Some(encoded),
        )
    }
    #[allow(clippy::too_many_arguments)]
    fn write_selected(
        &self,
        d: &Document,
        path: &Path,
        o: Options,
        confirmed: bool,
        replace: bool,
        budget: usize,
        c: &AtomicBool,
        encoded: Option<&Encoded>,
    ) -> Result<(), String> {
        crate::exchange_guidance::reject_private(path)?;
        self.check(d)?;
        if !confirmed && !self.losses(o).is_empty() {
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
        cancel(c)?;
        let owned;
        let bytes = if let Some(encoded) = encoded {
            if encoded.bytes.len() > budget / 2 {
                return Err("Encoded preview exceeds current workspace memory".into());
            }
            &encoded.bytes
        } else {
            owned = self.encode(o, budget, c)?;
            &owned
        };
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
