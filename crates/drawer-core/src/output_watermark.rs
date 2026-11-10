//! Output-only, coordinate-stable local watermark presets. No document commands.
use crate::{color_precision::Sample, layer_blend::Mode, raster_io::Prepared};
use serde::{Deserialize, Serialize};
use std::{fs, io::Write, path::PathBuf, sync::atomic::AtomicBool};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<[u8; 4]>,
}
impl Image {
    fn valid(&self) -> bool {
        self.width > 0
            && self.height > 0
            && self.pixels.len() <= 1024 * 1024
            && self.pixels.len() == self.width as usize * self.height as usize
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Source {
    #[default]
    Text,
    Image,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Anchor {
    TopLeft,
    Top,
    TopRight,
    Left,
    Center,
    Right,
    BottomLeft,
    Bottom,
    #[default]
    BottomRight,
}
impl Anchor {
    pub const ALL: [Self; 9] = [
        Self::TopLeft,
        Self::Top,
        Self::TopRight,
        Self::Left,
        Self::Center,
        Self::Right,
        Self::BottomLeft,
        Self::Bottom,
        Self::BottomRight,
    ];
    fn ratio(self) -> [f64; 2] {
        let n = Self::ALL.iter().position(|v| *v == self).unwrap();
        [(n % 3) as f64 / 2., (n / 3) as f64 / 2.]
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Pattern {
    #[default]
    Grid,
    Staggered,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Direction {
    Horizontal,
    Vertical,
    #[default]
    Both,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    pub version: u32,
    pub enabled: bool,
    pub source: Source,
    pub text: String,
    pub size: f64,
    pub color: [u8; 4],
    pub font: crate::text_layout::Font,
    pub path: Option<PathBuf>,
    pub embed: bool,
    pub image: Option<Image>,
    pub mode: Mode,
    pub opacity: f64,
    pub rotation: f64,
    pub scale: [f64; 2],
    pub lock: bool,
    pub position: [f64; 2],
    pub anchor: Anchor,
    pub margin: [f64; 2],
    pub repeat: bool,
    pub pattern: Pattern,
    pub direction: Direction,
    pub spacing: [f64; 2],
    pub include_cover: bool,
    pub first_is_cover: bool,
    pub noise: bool,
    pub strength: f64,
    pub seed: u64,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            version: 1,
            enabled: false,
            source: Source::Text,
            text: "© Drawer".into(),
            size: 24.,
            color: [255; 4],
            font: crate::text_layout::Font::Ubuntu,
            path: None,
            embed: false,
            image: None,
            mode: Mode::Normal,
            opacity: 0.5,
            rotation: 0.,
            scale: [1.; 2],
            lock: true,
            position: [0.; 2],
            anchor: Anchor::BottomRight,
            margin: [16.; 2],
            repeat: false,
            pattern: Pattern::Grid,
            direction: Direction::Both,
            spacing: [32.; 2],
            include_cover: true,
            first_is_cover: false,
            noise: false,
            strength: 0.02,
            seed: 1,
        }
    }
}
impl Settings {
    pub fn valid(&self) -> bool {
        self.version == 1
            && self.text.len() <= 4096
            && self.font.valid()
            && self
                .path
                .as_ref()
                .is_none_or(|p| crate::file_browser::lexical(p).is_ok())
            && self.image.as_ref().is_none_or(Image::valid)
            && self.size.is_finite()
            && (1.0..=512.).contains(&self.size)
            && self.opacity.is_finite()
            && (0.0..=1.).contains(&self.opacity)
            && self.rotation.is_finite()
            && (-360.0..=360.).contains(&self.rotation)
            && self
                .scale
                .iter()
                .all(|v| v.is_finite() && (0.01..=100.).contains(v))
            && self
                .position
                .iter()
                .all(|v| v.is_finite() && v.abs() <= 1e6)
            && self
                .margin
                .iter()
                .chain(&self.spacing)
                .all(|v| v.is_finite() && (0.0..=1e6).contains(v))
            && self.strength.is_finite()
            && (0.0..=0.25).contains(&self.strength)
    }
    pub fn reset_transform(&mut self) {
        self.rotation = 0.;
        self.scale = [1.; 2];
        self.position = [0.; 2];
        self.margin = [16.; 2];
        self.anchor = Anchor::BottomRight;
    }
    pub fn reload(&mut self, budget: usize) -> Result<(), String> {
        let path = crate::file_browser::checked(
            self.path.as_ref().ok_or("Select a local PNG watermark")?,
        )?;
        let v = crate::animation_render::Watermark::read(&path, budget)?;
        let image = Image {
            width: v.width,
            height: v.height,
            pixels: v.pixels,
        };
        if !image.valid() {
            return Err("Watermark image exceeds1M pixels".into());
        }
        self.image = Some(image);
        Ok(())
    }
    pub fn load(storage: &crate::storage::UserStorage) -> Result<Self, String> {
        let p = storage.root().join("settings/output-watermark.json");
        if !p.exists() {
            return Ok(Self::default());
        }
        let bytes = crate::asset_catalog::bytes(&p, 24 * 1024 * 1024)?;
        let s: Self = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
        if !s.valid() {
            return Err("Invalid watermark preset; original preserved".into());
        }
        Ok(s)
    }
    pub fn save(&self, storage: &crate::storage::UserStorage) -> Result<(), String> {
        if !self.valid() {
            return Err("Invalid watermark preset".into());
        }
        let mut saved = self.clone();
        if saved.embed && saved.source == Source::Image && saved.image.is_none() {
            saved.reload(crate::document::DEFAULT_DOCUMENT_BUDGET)?;
        }
        if !saved.embed {
            saved.image = None;
        }
        let bytes = serde_json::to_vec_pretty(&saved).map_err(|e| e.to_string())?;
        if bytes.len() > 24 * 1024 * 1024 {
            return Err("Watermark preset size limit".into());
        }
        let p = storage.root().join("settings/output-watermark.json");
        let temp = p.with_extension("json.write.tmp");
        let mut f = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)
            .map_err(|e| e.to_string())?;
        let result = (|| {
            f.write_all(&bytes).map_err(|e| e.to_string())?;
            f.sync_all().map_err(|e| e.to_string())?;
            drop(f);
            fs::rename(&temp, &p).map_err(|e| e.to_string())
        })();
        if temp.exists() {
            let _ = fs::remove_file(temp);
        }
        result
    }
}
struct Stamp {
    image: Image,
    size: [f64; 2],
}
fn stamp(s: &Settings, budget: usize, c: &AtomicBool) -> Result<Stamp, String> {
    let image = match s.source {
        Source::Image => {
            let mut captured = s.clone();
            if !s.embed || s.image.is_none() {
                captured.reload(budget)?;
            }
            captured.image.ok_or("Missing watermark image")?
        }
        Source::Text => {
            if s.text.trim().is_empty() {
                return Err("Enter watermark text".into());
            }
            let style = crate::text_layout::Style {
                font: s.font,
                wrap_width: 4096.,
                ..Default::default()
            };
            let b = style.bounds([0., s.size], &s.text, s.size);
            let width = (b.max[0] - b.min[0]).ceil().max(1.) as u32 + 2;
            let height = (b.max[1] - b.min[1]).ceil().max(1.) as u32 + 2;
            if u64::from(width) * u64::from(height) > 1024 * 1024 {
                return Err("Text watermark exceeds1M pixels".into());
            }
            let mut d = crate::document::Document::create(
                crate::document::CanvasSpec {
                    width,
                    height,
                    dpi: 96.,
                    background: crate::document::Background::Transparent,
                },
                budget,
            )
            .map_err(|e| e.to_string())?;

            d.scene.objects.push(crate::scene::PageObject::Text {
                baseline: [1. - b.min[0], s.size + 1. - b.min[1]],
                text: s.text.clone(),
                size_px: s.size,
                color: s.color,
                layout: Some(Box::new(style)),
            });
            let r = crate::render::Renderer::for_export(&d)?;
            let mut pixels = Vec::with_capacity(width as usize * height as usize);
            for y in 0..height {
                crate::asset_catalog::canceled(c)?;
                for x in 0..width {
                    pixels.push(r.display_sample([x as f64 + 0.5, y as f64 + 0.5])?.rgba8());
                }
            }
            Image {
                width,
                height,
                pixels,
            }
        }
    };
    if !image.valid() || image.pixels.len().saturating_mul(24) > budget {
        return Err("Watermark memory limit".into());
    }
    Ok(Stamp {
        size: [
            image.width as f64 * s.scale[0],
            image.height as f64 * s.scale[1],
        ],
        image,
    })
}
/// Fixed seed and output pixel coordinates for every page/frame; no frame-dependent flicker.
pub fn apply(
    p: &mut Prepared,
    s: &Settings,
    cover: bool,
    budget: usize,
    c: &AtomicBool,
) -> Result<(), String> {
    if !s.enabled && !s.noise {
        return Ok(());
    }
    p.pixels = composite(
        &p.pixels,
        [p.width, p.height],
        s,
        cover,
        p.from_srgb.as_deref(),
        budget,
        c,
    )?;
    p.color_notices.push("Output-only watermark/noise; fixed output-coordinate pattern across pages and time-lapse frames. Source pixels/history untouched.".into());
    Ok(())
}
/// Capture external pixels and font outlines once before rendering any animation frame.
pub fn frozen(s: &Settings, budget: usize, c: &AtomicBool) -> Result<Settings, String> {
    if !s.valid() {
        return Err("Invalid watermark preset".into());
    }
    let mut out = s.clone();
    if s.enabled && s.opacity > 0. {
        out.image = Some(stamp(s, budget, c)?.image);
        out.source = Source::Image;
        out.embed = true;
    }
    Ok(out)
}
pub(crate) fn frame(
    bytes: &mut Vec<u8>,
    size: [u32; 2],
    s: &Settings,
    profile: Option<&crate::color_profile::RgbProfile>,
    budget: usize,
    c: &AtomicBool,
) -> Result<(), String> {
    if !s.enabled && !s.noise {
        return Ok(());
    }
    let count = size[0] as usize * size[1] as usize;
    if bytes.len() != count * 4 || count.saturating_mul(32) > budget {
        return Err("Watermark frame memory limit".into());
    }
    let pixels = bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|v| v.map(|c| c as u16 * 257))
        .collect::<Vec<_>>();
    let transform = profile
        .map(|p| crate::color_profile::Transform::new(&crate::color_profile::RgbProfile::Srgb, p))
        .transpose()?;
    let result = composite(&pixels, size, s, false, transform.as_ref(), budget, c)?;
    *bytes = result
        .into_iter()
        .flat_map(|p| Sample::from16(p).rgba8())
        .collect();
    Ok(())
}
fn composite(
    pixels: &[[u16; 4]],
    dimensions: [u32; 2],
    s: &Settings,
    cover: bool,
    from_srgb: Option<&crate::color_profile::Transform>,
    budget: usize,
    c: &AtomicBool,
) -> Result<Vec<[u16; 4]>, String> {
    if !s.enabled && !s.noise {
        return Ok(pixels.to_vec());
    }
    if !s.valid() {
        return Err("Invalid watermark settings".into());
    }
    if cover && !s.include_cover {
        return Ok(pixels.to_vec());
    }
    let stamp = if s.enabled && s.opacity > 0. {
        Some(stamp(s, budget, c)?)
    } else {
        None
    };
    let count = pixels.len();
    if count.saturating_mul(16) > budget {
        return Err("Watermark output copy budget".into());
    }
    let mut result = Vec::with_capacity(count);
    let (sin, cos) = s.rotation.to_radians().sin_cos();
    let ratio = s.anchor.ratio();
    let size = stamp.as_ref().map_or([0.; 2], |v| v.size);
    let bound = [
        size[0] * cos.abs() + size[1] * sin.abs(),
        size[0] * sin.abs() + size[1] * cos.abs(),
    ];
    let anchor = [
        s.margin[0] + (dimensions[0] as f64 - 2. * s.margin[0]) * ratio[0] + s.position[0],
        s.margin[1] + (dimensions[1] as f64 - 2. * s.margin[1]) * ratio[1] + s.position[1],
    ];
    let center = [
        anchor[0] + (0.5 - ratio[0]) * bound[0],
        anchor[1] + (0.5 - ratio[1]) * bound[1],
    ];
    let step = [
        (size[0] + s.spacing[0]).max(0.01),
        (size[1] + s.spacing[1]).max(0.01),
    ];
    for y in 0..dimensions[1] {
        crate::asset_catalog::canceled(c)?;
        for x in 0..dimensions[0] {
            let n = y as usize * dimensions[0] as usize + x as usize;
            let mut back = Sample::from16(pixels[n]);
            if let Some(stamp) = &stamp {
                let delta = [x as f64 + 0.5 - center[0], y as f64 + 0.5 - center[1]];
                let mut q = [
                    cos * delta[0] + sin * delta[1],
                    -sin * delta[0] + cos * delta[1],
                ];
                if s.repeat {
                    let row = (q[1] / step[1] + 0.5).floor();
                    if s.direction != Direction::Horizontal {
                        q[1] -= row * step[1];
                    }
                    if s.pattern == Pattern::Staggered
                        && row.rem_euclid(2.) == 1.
                        && s.direction == Direction::Both
                    {
                        q[0] -= step[0] / 2.;
                    }
                    if s.direction != Direction::Vertical {
                        q[0] -= (q[0] / step[0] + 0.5).floor() * step[0];
                    }
                }
                let q = [
                    q[0] / s.scale[0] + stamp.image.width as f64 / 2. - 0.5,
                    q[1] / s.scale[1] + stamp.image.height as f64 / 2. - 0.5,
                ];
                let sample = crate::interpolation::sample_float(
                    q,
                    crate::interpolation::box_footprint(q, [1. / s.scale[0], 1. / s.scale[1]]),
                    crate::interpolation::Filter::Bilinear,
                    |x, y| {
                        if x < 0
                            || y < 0
                            || x >= stamp.image.width as i64
                            || y >= stamp.image.height as i64
                        {
                            Sample::default()
                        } else {
                            Sample::from8(
                                stamp.image.pixels
                                    [y as usize * stamp.image.width as usize + x as usize],
                            )
                        }
                    },
                );
                let sample = if let Some(t) = from_srgb {
                    t.map(sample)?
                } else {
                    sample
                };
                back = back.over(sample, s.mode, s.opacity)?;
            }
            let mut rgba = back.rgba16();
            if s.noise && s.strength > 0. && rgba[3] > 0 {
                let mut v = s.seed
                    ^ (x as u64).wrapping_mul(0x9e3779b185ebca87)
                    ^ (y as u64).wrapping_mul(0xc2b2ae3d27d4eb4f);
                v ^= v >> 29;
                v = v.wrapping_mul(0x94d049bb133111eb);
                v ^= v >> 31;
                let noise =
                    ((v >> 11) as f64 / (1u64 << 53) as f64 * 2. - 1.) * s.strength * 65535.;
                for ch in &mut rgba[..3] {
                    *ch = (*ch as f64 + noise).round().clamp(0., 65535.) as u16;
                }
            }
            result.push(rgba);
        }
    }
    Ok(result)
}
#[path = "output_watermark_verify.rs"]
pub mod verify;
