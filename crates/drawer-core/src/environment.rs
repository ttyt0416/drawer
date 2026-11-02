//! Local equirectangular panorama snapshots and low-frequency diffuse illumination.
use crate::{
    document::Document,
    scene::Point,
    world_math::unit,
    world3d::{self, Matrix, V},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
};
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum Encoding {
    Srgb,
    IccToSrgb,
    Gamma(f64),
    LinearHdr,
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub yaw: f64,
    pub background: bool,
    pub illumination: bool,
    pub exposure: f64,
    pub strength: f64,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            yaw: 0.,
            background: true,
            illumination: true,
            exposure: 0.,
            strength: 1.,
        }
    }
}
impl Settings {
    pub fn valid(self) -> bool {
        self.yaw.is_finite()
            && self.yaw.abs() <= 36000.
            && self.exposure.is_finite()
            && (-16. ..=16.).contains(&self.exposure)
            && self.strength.is_finite()
            && (0. ..=64.).contains(&self.strength)
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Data {
    pub version: u32,
    pub source: String,
    pub encoding: Encoding,
    pub width: u16,
    pub height: u16,
    pub rgb32: String,
    pub settings: Settings,
}
impl Data {
    pub fn pixels(&self) -> Result<Vec<[f32; 3]>, String> {
        if self.version != 1
            || self.source.is_empty()
            || self.source.len() > 512
            || self.source.chars().any(char::is_control)
            || self.width != self.height.saturating_mul(2)
            || self.height == 0
            || self.height > 256
            || !self.settings.valid()
            || matches!(self.encoding,Encoding::Gamma(g) if !g.is_finite()||!(0.01..=10.).contains(&g))
        {
            return Err("Invalid embedded panorama metadata".into());
        }
        let n = self.width as usize * self.height as usize;
        if self.rgb32.len() != n * 16 {
            return Err("Embedded panorama length differs".into());
        }
        let bytes = STANDARD.decode(&self.rgb32).map_err(|e| e.to_string())?;
        if bytes.len() != n * 12 {
            return Err("Embedded panorama pixels differ".into());
        }
        let mut out = Vec::with_capacity(n);
        for p in bytes.as_chunks::<12>().0 {
            let color = std::array::from_fn(|i| {
                f32::from_le_bytes(p[i * 4..i * 4 + 4].try_into().unwrap())
            });
            if color.iter().any(|v| !v.is_finite() || *v < 0.) {
                return Err("Panorama radiance must be finite and nonnegative".into());
            }
            out.push(color);
        }
        Ok(out)
    }
    pub fn bytes(&self) -> usize {
        self.rgb32.len() + self.width as usize * self.height as usize * 12 + self.source.len() + 128
    }
    pub fn from_pixels(
        source: String,
        encoding: Encoding,
        width: u16,
        height: u16,
        pixels: &[[f32; 3]],
        settings: Settings,
    ) -> Result<Self, String> {
        if pixels.len() != width as usize * height as usize {
            return Err("Panorama dimensions differ".into());
        }
        let rgb32 = STANDARD.encode(
            pixels
                .iter()
                .flatten()
                .flat_map(|v| v.to_le_bytes())
                .collect::<Vec<_>>(),
        );
        let data = Self {
            version: 1,
            source,
            encoding,
            width,
            height,
            rgb32,
            settings,
        };
        data.pixels()?;
        Ok(data)
    }
}
fn linear(v: f64) -> f64 {
    if v <= 0.04045 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}
fn encode(v: f64) -> f64 {
    let v = v.clamp(0., 1.);
    if v <= 0.0031308 {
        12.92 * v
    } else {
        1.055 * v.powf(1. / 2.4) - 0.055
    }
}
pub fn load(
    path: &Path,
    settings: Settings,
    budget: usize,
    cancel: &AtomicBool,
) -> Result<(Data, Vec<String>), String> {
    use image::ImageDecoder;
    crate::book_io::check_cancel(cancel)?;
    if !settings.valid() {
        return Err("Invalid panorama settings".into());
    }
    let path = crate::model_import::local_path(path)?;
    let ext = path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let bytes = crate::book_io::read_local(&path, (16 * 1024 * 1024).min(budget / 8))?;
    let format = image::guess_format(&bytes).map_err(|e| e.to_string())?;
    if !matches!(
        (ext.as_str(), format),
        ("png", image::ImageFormat::Png)
            | ("jpg" | "jpeg", image::ImageFormat::Jpeg)
            | ("hdr", image::ImageFormat::Hdr)
    ) {
        return Err("Choose a matching PNG/JPEG or Radiance .hdr panorama".into());
    }
    let mut reader =
        image::ImageReader::with_format(std::io::Cursor::new(bytes.as_slice()), format);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(2048);
    limits.max_image_height = Some(1024);
    limits.max_alloc = Some(64 * 1024 * 1024);
    reader.limits(limits);
    let mut decoder = reader.into_decoder().map_err(|e| e.to_string())?;
    let (w, h) = decoder.dimensions();
    if w == 0
        || h == 0
        || w != h.saturating_mul(2)
        || h > 1024
        || w as usize * h as usize * 64 + bytes.len() * 8 > budget
    {
        return Err("Panorama needs a2:1 equirectangular image within2048x1024 and the decode memory budget".into());
    }
    let mut warnings = vec![];
    let (pixels, encoding) = if format == image::ImageFormat::Hdr {
        let image = image::DynamicImage::from_decoder(decoder)
            .map_err(|e| e.to_string())?
            .into_rgb32f();
        warnings.push("Radiance HDR is stored as linear RGB32F; source HDR exposure/gamma header annotations are not applied. Background exposure is explicit and display-clipped; lighting retains radiance above1".into());
        (
            image.pixels().map(|p| p.0).collect::<Vec<_>>(),
            Encoding::LinearHdr,
        )
    } else {
        let color = decoder.color_type();
        let gray = matches!(
            color,
            image::ColorType::L8
                | image::ColorType::La8
                | image::ColorType::L16
                | image::ColorType::La16
        );
        let profile = decoder.icc_profile().map_err(|e| e.to_string())?;
        let high = matches!(
            color,
            image::ColorType::L16
                | image::ColorType::La16
                | image::ColorType::Rgb16
                | image::ColorType::Rgba16
        );
        let mut image = image::DynamicImage::from_decoder(decoder)
            .map_err(|e| e.to_string())?
            .into_rgba8();
        if image.pixels().any(|p| p[3] != 255) {
            return Err(
                "Panorama requires opaque pixels; flatten transparency before import".into(),
            );
        }
        let mut encoding = Encoding::Srgb;
        if let Some(profile) = profile {
            crate::image_import::convert_profile(&mut image, &profile, gray, cancel)
                .map_err(|e| e.to_string())?;
            encoding = Encoding::IccToSrgb;
        } else if format == image::ImageFormat::Png {
            let r = png::Decoder::new(std::io::Cursor::new(&bytes))
                .read_info()
                .map_err(|e| e.to_string())?;
            let info = r.info();
            if info.srgb.is_none()
                && let Some(gamma) = info.gamma()
            {
                let g = gamma.into_value() as f64;
                if !(0.01..=10.).contains(&g) {
                    return Err("PNG gamma outside supported range".into());
                }
                encoding = Encoding::Gamma(g);
            }
        }
        if high {
            warnings.push("LDR16-bit input uses the existing color-managed8-bit image bridge before linear panorama storage".into());
        }
        warnings.push(format!("LDR transfer: {encoding:?}; untagged PNG/JPEG is interpreted as sRGB. Panorama orientation follows stored2:1 pixels, without EXIF rotation"));
        let pixels = image
            .pixels()
            .map(|p| {
                std::array::from_fn(|i| {
                    let v = p[i] as f64 / 255.;
                    (if let Encoding::Gamma(g) = encoding {
                        v.powf(1. / g)
                    } else {
                        linear(v)
                    }) as f32
                })
            })
            .collect();
        (pixels, encoding)
    };
    crate::book_io::check_cancel(cancel)?;
    let height = h.min(256);
    let width = height * 2;
    let mut small = Vec::with_capacity((width * height) as usize);
    // Average in linear light; exact integer footprints also preserve HDR energy.
    for y in 0..height {
        crate::book_io::check_cancel(cancel)?;
        for x in 0..width {
            let mut sum = [0f64; 3];
            let mut count = 0.;
            for sy in y * h / height..((y + 1) * h / height) {
                for sx in x * w / width..((x + 1) * w / width) {
                    let p = pixels[(sy * w + sx) as usize];
                    if p.iter().any(|n| !n.is_finite() || *n < 0.) {
                        return Err("Invalid/excessive panorama radiance".into());
                    }
                    for i in 0..3 {
                        sum[i] += p[i] as f64;
                    }
                    count += 1.;
                }
            }
            small.push(sum.map(|v| (v / count) as f32));
        }
    }
    if height != h {
        warnings.push(format!("Panorama reduced in linear light from{w}x{h} to{width}x{height} for local document capacity"));
    }
    let source = path
        .file_name()
        .unwrap()
        .to_str()
        .ok_or("Use a UTF-8 panorama filename")?
        .to_owned();
    Ok((
        Data::from_pixels(
            source,
            encoding,
            width as u16,
            height as u16,
            &small,
            settings,
        )?,
        warnings,
    ))
}
pub(crate) struct Sampler {
    pub settings: Settings,
    width: usize,
    height: usize,
    pixels: Vec<[f32; 3]>,
    rotation: Matrix,
}
impl Sampler {
    pub fn new(data: &Data) -> Result<Self, String> {
        Ok(Self {
            settings: data.settings,
            width: data.width as usize,
            height: data.height as usize,
            pixels: data.pixels()?,
            rotation: Matrix::rotation([0., 1., 0.], -data.settings.yaw),
        })
    }
    fn sample_uv(&self, u: f64, v: f64) -> V {
        let x = (u.rem_euclid(1.) * self.width as f64).floor() as usize;
        let y = (v.clamp(0., 1.) * self.height as f64).floor() as usize;
        self.pixels[y.min(self.height - 1) * self.width + x.min(self.width - 1)].map(|v| v as f64)
    }
    pub fn radiance(&self, direction: V) -> V {
        let d = unit(self.rotation.vector(direction));
        self.sample_uv(
            d[0].atan2(-d[2]) / std::f64::consts::TAU + 0.5,
            d[1].clamp(-1., 1.).acos() / std::f64::consts::PI,
        )
    }
    pub fn background(&self, d: &Document, point: Point) -> crate::color_precision::Sample {
        if !self.settings.background {
            return Default::default();
        }
        let (camera, lens) = world3d::default_camera(d);
        if !camera.page_bounds.contains(point) {
            return Default::default();
        }
        let (_, direction) = lens.ray(&camera, point);
        let rgb = self
            .radiance(direction)
            .map(|v| encode(v * 2f64.powf(self.settings.exposure)));
        crate::color_precision::Sample::new([rgb[0], rgb[1], rgb[2], 1.]).unwrap()
    }
    pub fn light(&self) -> Light {
        let mut coefficients = [[0.; 3]; 9];
        let w = self.width.max(64);
        let h = self.height.max(32);
        for y in 0..h {
            let phi = (y as f64 + 0.5) / h as f64 * std::f64::consts::PI;
            let solid = std::f64::consts::TAU / w as f64
                * ((y as f64 / h as f64 * std::f64::consts::PI).cos()
                    - ((y + 1) as f64 / h as f64 * std::f64::consts::PI).cos());
            for x in 0..w {
                let theta = ((x as f64 + 0.5) / w as f64 - 0.5) * std::f64::consts::TAU;
                let dir = [theta.sin() * phi.sin(), phi.cos(), -theta.cos() * phi.sin()];
                let color =
                    self.sample_uv((x as f64 + 0.5) / w as f64, (y as f64 + 0.5) / h as f64);
                for (c, basis) in coefficients.iter_mut().zip(basis(dir)) {
                    for i in 0..3 {
                        c[i] += color[i] * basis * solid;
                    }
                }
            }
        }
        Light {
            coefficients,
            rotation: self.rotation,
            strength: if self.settings.illumination {
                self.settings.strength
            } else {
                0.
            },
        }
    }
}
fn basis([x, y, z]: V) -> [f64; 9] {
    [
        0.28209479177387814,
        0.4886025119029199 * y,
        0.4886025119029199 * z,
        0.4886025119029199 * x,
        1.0925484305920792 * x * y,
        1.0925484305920792 * y * z,
        0.31539156525252005 * (3. * z * z - 1.),
        1.0925484305920792 * x * z,
        0.5462742152960396 * (x * x - y * y),
    ]
}
pub(crate) struct Light {
    coefficients: [V; 9],
    rotation: Matrix,
    strength: f64,
}
impl Light {
    pub fn diffuse(&self, normal: V) -> V {
        let b = basis(unit(self.rotation.vector(normal)));
        std::array::from_fn(|i| {
            self.coefficients
                .iter()
                .zip(b)
                .enumerate()
                .map(|(j, (c, b))| {
                    c[i] * b
                        * if j == 0 {
                            1.
                        } else if j < 4 {
                            2. / 3.
                        } else {
                            0.25
                        }
                })
                .sum::<f64>()
                .max(0.)
                * self.strength
        })
    }
}
pub enum Edit {
    Load { path: PathBuf, settings: Settings },
    Settings(Settings),
    Remove,
}
pub struct Prepared {
    inner: world3d::Prepared,
    frame: u32,
    no_op: bool,
    pub thumbnail: Vec<u8>,
}
impl Prepared {
    pub fn warnings(&self) -> &[String] {
        &self.inner.warnings
    }
    pub fn matches(&self, d: &Document) -> bool {
        self.inner.matches(d) && self.frame == d.current_frame
    }
    pub fn scene(&self) -> &crate::scene::Scene {
        self.inner.scene()
    }
    pub fn commit(self, d: &mut Document) -> Result<bool, String> {
        if !self.matches(d) {
            return Err("Environment result became stale".into());
        }
        if self.no_op {
            return Ok(false);
        }
        self.inner.commit(d)
    }
}
pub fn prepare(
    d: &Document,
    edit: Edit,
    budget: usize,
    cancel: &AtomicBool,
) -> Result<Prepared, String> {
    crate::book_io::check_cancel(cancel)?;
    if d.memory_bytes()
        .saturating_mul(3)
        .saturating_add(4 * 1024 * 1024)
        > budget
    {
        return Err("Environment snapshot memory budget exceeded".into());
    }
    let triangles = d
        .scene
        .world3d
        .surfaces
        .values()
        .map(|s| s.triangles.len())
        .sum::<usize>();
    if d.memory_bytes()
        .saturating_mul(3)
        .saturating_add(triangles.saturating_mul(512))
        .saturating_add(4 * 1024 * 1024)
        > budget
    {
        return Err("Environment preview acceleration exceeds memory budget".into());
    }
    let mut state = d.snapshot();
    let mut warnings = vec![];
    match edit {
        Edit::Load { path, settings } => {
            let (data, w) = load(
                &path,
                settings,
                budget.saturating_sub(d.memory_bytes().saturating_mul(3)),
                cancel,
            )?;
            state.scene.world3d.environment = Some(data);
            warnings = w;
        }
        Edit::Settings(settings) => {
            if !settings.valid() {
                return Err("Invalid environment values".into());
            }
            state
                .scene
                .world3d
                .environment
                .as_mut()
                .ok_or("Load a local panorama first")?
                .settings = settings;
        }
        Edit::Remove => state.scene.world3d.environment = None,
    }
    if let Some(env) = &state.scene.world3d.environment {
        let (camera, mut lens) = world3d::default_camera(d);
        if d.scene.camera.is_some() && d.scene.world3d.lens.is_none() {
            lens.projection = world3d::Projection::Orthographic;
            warnings.push("Legacy orthographic camera projection retained".into());
        }
        state.scene.camera = Some(camera);
        state.scene.world3d.lens = Some(lens);
        if env.settings.illumination
            && env.settings.strength > 0.
            && state.scene.world3d.lighting.as_ref().is_none_or(|s| {
                matches!(
                    s.style,
                    crate::lighting::Style::Wire | crate::lighting::Style::Unlit
                )
            })
        {
            let mut settings =
                state
                    .scene
                    .world3d
                    .lighting
                    .clone()
                    .unwrap_or_else(|| crate::lighting::Settings {
                        lights: vec![],
                        ambient: 0.,
                        shadows: false,
                        ..Default::default()
                    });
            settings.style = crate::lighting::Style::Smooth;
            state.scene.world3d.lighting = Some(settings);
            warnings.push("Image illumination adopts Smooth rendering; existing lamps/ambient are retained. Unlit/Wire ignore image illumination".into());
        }
        warnings.push("Diffuse environment light uses9 spherical-harmonic terms; it has no local occlusion, specular reflection or environment-cast shadows. Background is an opaque3D backdrop above earlier2D content, below the World render group".into());
    }
    if state.memory_bytes().saturating_mul(3) > budget {
        return Err("Embedded panorama and Undo exceed memory budget".into());
    }
    let no_op = state == d.snapshot();
    let inner = world3d::prepared(
        d,
        state,
        "Local panorama / diffuse environment settings".into(),
        vec![],
        warnings,
    )?;
    let mut state = d.snapshot();
    state.scene = inner.scene().clone();
    let mut frozen = crate::layer_merge::preview(&state, d.current_frame)?;
    frozen.keyframe_content = d.keyframe_content;
    crate::files::native_scene(&frozen)
        .map_err(|e| format!("Panorama exceeds native scene capacity: {e}"))?;
    let renderer = crate::render::Renderer::new(&frozen);
    if let Some(e) = renderer.error() {
        return Err(e.into());
    }
    let mut thumbnail = vec![];
    for y in 0..128 {
        crate::book_io::check_cancel(cancel)?;
        for x in 0..128 {
            thumbnail.extend(
                renderer
                    .display_sample([
                        (x as f64 + 0.5) * d.width as f64 / 128.,
                        (y as f64 + 0.5) * d.height as f64 / 128.,
                    ])?
                    .rgba8(),
            );
        }
    }
    Ok(Prepared {
        inner,
        frame: d.current_frame,
        no_op,
        thumbnail,
    })
}
