//! Document color ownership and loss-preview state. Integration is kept separate from view proofing.
use crate::{
    color_precision::{Depth, Dither, Sample},
    color_profile::{RgbProfile, Transform},
    document::{CanvasSpec, CreateError, DEFAULT_DOCUMENT_BUDGET, Document, RasterLayer},
    sizing::DocumentState,
};
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Calculation {
    LegacyEight,
    #[default]
    Float64,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub calculation: Calculation,
    pub depth: Depth,
    pub dither: Dither,
    pub profile: RgbProfile,
}
impl Config {
    pub fn legacy() -> Self {
        Self {
            calculation: Calculation::LegacyEight,
            depth: Depth::Eight,
            dither: Default::default(),
            profile: RgbProfile::AssumedSrgb,
        }
    }
    pub fn memory_bytes(&self) -> usize {
        self.profile.memory_bytes()
    }
    pub fn valid(&self) -> bool {
        if self.calculation == Calculation::LegacyEight {
            *self == Self::legacy()
        } else {
            self.profile.valid()
        }
    }
    pub fn valid_layers(&self, layers: &[RasterLayer]) -> bool {
        self.valid()
            && layers.iter().all(|l| {
                l.color_valid()
                    && (self.depth == Depth::Sixteen || l.precise.is_none())
                    && (self.calculation == Calculation::LegacyEight
                        || l.precise.is_some()
                        || l.pixels.iter().all(|p| p[3] != 0 || p[..3] == [0; 3]))
            })
    }
}
impl Document {
    pub fn create_with_color(
        spec: CanvasSpec,
        memory_budget: usize,
        color: Config,
    ) -> Result<Self, CreateError> {
        if !color.valid() {
            return Err(CreateError::LayerDefaults(
                "Invalid RGB document color configuration".into(),
            ));
        }
        let bytes = (spec.width as usize)
            .saturating_mul(spec.height as usize)
            .saturating_mul(if color.depth == Depth::Sixteen { 12 } else { 4 })
            .saturating_add(color.memory_bytes());
        if bytes > memory_budget {
            return Err(CreateError::ResourceLimit);
        }
        let mut d = Self::create(spec, memory_budget)?;
        if color.depth == Depth::Sixteen {
            let pixels = d.layers[0]
                .pixels
                .iter()
                .map(|p| Sample::from8(*p).rgba16())
                .collect();
            d.layers[0].precise = Some(pixels);
        }
        d.color = color;
        Ok(d)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProfileChange {
    AssignNumbers,
    ConvertPixels,
}
/// Store one completed operation; callers keep all earlier stages in floating point.
pub(crate) fn set_samples(
    layer: &mut RasterLayer,
    config: &Config,
    extent: crate::document::PlacedRaster,
    samples: impl IntoIterator<Item = Sample>,
) -> Result<(), String> {
    let count = extent.width as usize * extent.height as usize;
    if !extent.valid() || count > 4 * 1024 * 1024 {
        return Err("Color storage extent exceeds 4Mi samples".into());
    }
    let mut low = Vec::with_capacity(count);
    let mut high = (config.depth == Depth::Sixteen).then(|| Vec::with_capacity(count));
    for (n, sample) in samples.into_iter().enumerate() {
        if n >= count {
            return Err("Too many color samples".into());
        }
        let value = sample.store(
            config.depth,
            config.dither,
            [
                extent.offset[0] + (n % extent.width as usize) as i64,
                extent.offset[1] + (n / extent.width as usize) as i64,
            ],
        );
        if let Some(high) = &mut high {
            high.push(value);
            low.push(Sample::from16(value).rgba8());
        } else {
            low.push(value.map(|v| v as u8));
        }
    }
    if low.len() != count {
        return Err("Missing color samples".into());
    }
    layer.pixels = low;
    layer.precise = high;
    Ok(())
}
pub(crate) fn map_layer(
    layer: &mut RasterLayer,
    config: &Config,
    width: u32,
    height: u32,
    mut operation: impl FnMut(Sample) -> Result<Sample, String>,
) -> Result<(), String> {
    if layer.pixels.len() > 4 * 1024 * 1024 {
        return Err("Color mapping exceeds 4Mi samples".into());
    }
    let extent = layer.extent(width, height);
    let w = extent.width as usize;
    let mut low = Vec::with_capacity(layer.pixels.len());
    let mut high = (config.depth == Depth::Sixteen).then(|| Vec::with_capacity(layer.pixels.len()));
    for n in 0..layer.pixels.len() {
        let sample = operation(layer.sample_at(n))?;
        let value = sample.store(
            config.depth,
            config.dither,
            [
                extent.offset[0] + (n % w) as i64,
                extent.offset[1] + (n / w) as i64,
            ],
        );
        if let Some(high) = &mut high {
            high.push(value);
            low.push(Sample::from16(value).rgba8());
        } else {
            low.push(value.map(|v| v as u8));
        }
    }
    layer.pixels = low;
    layer.precise = high;
    Ok(())
}
pub(crate) fn edit_pixel8(
    d: &mut Document,
    layer: usize,
    x: u32,
    y: u32,
    after: [u8; 4],
) -> Result<bool, String> {
    crate::layer_properties::paintable(d, layer)?;
    if x >= d.width || y >= d.height {
        return Ok(false);
    }
    let raster = d.layers.get(layer).ok_or("Missing raster")?;
    let n = raster
        .pixel_index(d.width, d.height, x as i64, y as i64)
        .ok_or("Outside stored raster")?;
    let original = raster.sample_at(n);
    let source = if crate::clipping::alpha_locked(d, layer) {
        if original.alpha() == 0. {
            return Ok(false);
        }
        Sample::new([
            after[0] as f64 / 255.,
            after[1] as f64 / 255.,
            after[2] as f64 / 255.,
            original.alpha(),
        ])?
    } else {
        Sample::from8(after)
    };
    let value = source.store(d.color.depth, d.color.dither, [x as i64, y as i64]);
    let mut state = d.snapshot();
    let target = &mut state.layers[layer];
    if d.color.depth == Depth::Sixteen {
        let mut precise = target.precise.clone().unwrap_or_else(|| {
            target
                .pixels
                .iter()
                .map(|p| Sample::from8(*p).rgba16())
                .collect()
        });
        if precise[n] == value {
            return Ok(false);
        }
        precise[n] = value;
        target.precise = Some(precise);
        target.pixels[n] = Sample::from16(value).rgba8();
    } else {
        let value = value.map(|v| v as u8);
        if target.pixels[n] == value {
            return Ok(false);
        }
        target.pixels[n] = value;
    }
    d.commit_state(state)
}
fn requires_rasterization(d: &Document) -> bool {
    if !d.scene.surface_effects.rasters.is_empty() {
        return true;
    }
    d.scene
        .layer_effects
        .rasters
        .values()
        .any(|e| e.adjustment.is_some())
        || !d.scene.lines.layers.is_empty()
        || !d.scene.objects.is_empty()
        || !d.scene.gradients.layers.is_empty()
        || d.animation.is_some()
        || !d.scene.worlds.is_empty()
        || d.scene
            .appearance
            .rasters
            .values()
            .chain(d.scene.appearance.vectors.values())
            .any(|s| s.enabled && (s.color.is_some() || s.border > 0))
}
/// Frozen loss preview for stored raster channels. Other editable source types need their own adapters.
pub fn prepare(
    d: &Document,
    config: Config,
    change: ProfileChange,
) -> Result<DocumentState, String> {
    if !config.valid()
        || !d.color.valid_layers(&d.layers)
        || d.memory_bytes() > DEFAULT_DOCUMENT_BUDGET / 8
    {
        return Err("Invalid color state or snapshot budget".into());
    }
    if config == d.color {
        return Ok(d.snapshot());
    }
    let profile_changed = !config.profile.equivalent(&d.color.profile);
    if profile_changed && change == ProfileChange::ConvertPixels && requires_rasterization(d) {
        return Err(
            "Enable current-frame rasterization to convert editable non-raster sources, or rasterize them first"
                .into(),
        );
    }
    let transform = if profile_changed && change == ProfileChange::ConvertPixels {
        Some(Transform::new(&d.color.profile, &config.profile)?)
    } else {
        None
    };
    let mut state = d.snapshot();
    for layer in &mut state.layers {
        let count = layer.pixels.len();
        if count > 4 * 1024 * 1024 {
            return Err("Precision conversion exceeds 4Mi samples per layer".into());
        }
        let extent = layer.extent(d.width, d.height);
        let width = extent.width as usize;
        let mut bytes = Vec::with_capacity(count);
        let mut high = (config.depth == Depth::Sixteen).then(|| Vec::with_capacity(count));
        for n in 0..count {
            let source = layer.sample_at(n);
            let source = if let Some(t) = &transform {
                t.map(source)?
            } else {
                source
            };
            let stored = source.store(
                config.depth,
                config.dither,
                [
                    extent.offset[0] + (n % width) as i64,
                    extent.offset[1] + (n / width) as i64,
                ],
            );
            if let Some(high) = &mut high {
                high.push(stored);
                bytes.push(Sample::from16(stored).rgba8());
            } else {
                bytes.push(stored.map(|v| v as u8));
            }
        }
        layer.pixels = bytes;
        layer.precise = high;
    }
    state.color = config;
    if state.memory_bytes().saturating_add(d.memory_bytes()) > DEFAULT_DOCUMENT_BUDGET / 4
        || !state.color.valid_layers(&state.layers)
    {
        return Err("Precision conversion exceeds Undo memory or color limits".into());
    }
    crate::layer_properties::validate_edit(d, &state)?;
    crate::clipping::validate_state(d, &state)?;
    Ok(state)
}
/// An explicit flattening adapter keeps the original document untouched until one final commit.
pub fn prepare_with_sources(
    d: &Document,
    config: Config,
    change: ProfileChange,
    rasterize_sources: bool,
) -> Result<DocumentState, String> {
    let non_raster = requires_rasterization(d);
    if !rasterize_sources
        || !non_raster
        || change == ProfileChange::AssignNumbers
        || config.profile.equivalent(&d.color.profile)
    {
        return prepare(d, config, change);
    }
    if !config.valid() || !d.color.valid_layers(&d.layers) {
        return Err("Invalid RGB conversion configuration".into());
    }
    let mut state = crate::layer_merge::prepare(
        d,
        crate::layer_merge::Operation::Flatten,
        Default::default(),
    )?
    .state;
    let renderer = crate::float_render::Renderer::new(d)?;
    let transform = Transform::new(&d.color.profile, &config.profile)?;
    let count = d.width as usize * d.height as usize;
    let mut samples = Vec::with_capacity(count);
    for y in 0..d.height {
        for x in 0..d.width {
            samples.push(transform.map(renderer.pixel([x as f64 + 0.5, y as f64 + 0.5])?)?);
        }
    }
    set_samples(
        &mut state.layers[0],
        &config,
        crate::document::PlacedRaster {
            width: d.width,
            height: d.height,
            offset: [0; 2],
        },
        samples,
    )?;
    state.color = config;
    if state.memory_bytes().saturating_add(d.memory_bytes()) > DEFAULT_DOCUMENT_BUDGET / 4
        || !state.color.valid_layers(&state.layers)
    {
        return Err("RGB flatten conversion exceeds Undo budget".into());
    }
    crate::layer_properties::validate_edit(d, &state)?;
    crate::clipping::validate_state(d, &state)?;
    Ok(state)
}
const MAGIC: &[u8; 8] = b"DCOLOR01";
fn invalid(s: impl ToString) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, s.to_string())
}
fn u32_in(r: &mut impl Read) -> std::io::Result<u32> {
    let mut b = [0; 4];
    r.read_exact(&mut b)?;
    Ok(u32::from_le_bytes(b))
}
fn byte(r: &mut impl Read) -> std::io::Result<u8> {
    let mut b = [0];
    r.read_exact(&mut b)?;
    Ok(b[0])
}
pub(crate) fn write(d: &Document, w: &mut impl Write) -> std::io::Result<()> {
    if d.color == Config::legacy() && d.layers.iter().all(|l| l.precise.is_none()) {
        return Ok(());
    }
    if !d.color.valid_layers(&d.layers) {
        return Err(invalid("Invalid precision storage"));
    }
    w.write_all(MAGIC)?;
    w.write_all(&[
        match d.color.depth {
            Depth::Eight => 8,
            Depth::Sixteen => 16,
        },
        u8::from(d.color.dither.enabled),
    ])?;
    w.write_all(&d.color.dither.seed.to_le_bytes())?;
    w.write_all(&[match d.color.calculation {
        Calculation::LegacyEight => 0,
        Calculation::Float64 => 1,
    }])?;
    match &d.color.profile {
        RgbProfile::Srgb => w.write_all(&[0])?,
        RgbProfile::AssumedSrgb => w.write_all(&[1])?,
        RgbProfile::Embedded { name, bytes } => {
            w.write_all(&[2])?;
            w.write_all(&(name.len() as u32).to_le_bytes())?;
            w.write_all(name.as_bytes())?;
            w.write_all(&(bytes.len() as u32).to_le_bytes())?;
            w.write_all(bytes)?;
        }
    }
    for l in &d.layers {
        w.write_all(&[u8::from(l.precise.is_some())])?;
        if let Some(pixels) = &l.precise {
            for chunk in pixels.chunks(1024) {
                let mut buffer = [0; 8192];
                for (p, out) in chunk.iter().zip(buffer.as_chunks_mut::<8>().0) {
                    for (v, b) in p.iter().zip(out.as_chunks_mut::<2>().0) {
                        b.copy_from_slice(&v.to_le_bytes());
                    }
                }
                w.write_all(&buffer[..chunk.len() * 8])?;
            }
        }
    }
    Ok(())
}
pub(crate) fn read(d: &mut Document, r: &mut impl Read, budget: usize) -> std::io::Result<()> {
    let mut first = [0];
    if r.read(&mut first)? == 0 {
        return Ok(());
    }
    let mut magic = [0; 8];
    magic[0] = first[0];
    r.read_exact(&mut magic[1..])?;
    if &magic != MAGIC {
        return Err(invalid("Invalid precision footer"));
    }
    let depth = match byte(r)? {
        8 => Depth::Eight,
        16 => Depth::Sixteen,
        _ => return Err(invalid("Invalid channel depth")),
    };
    let enabled = match byte(r)? {
        0 => false,
        1 => true,
        _ => return Err(invalid("Invalid dither switch")),
    };
    let seed = u32_in(r)?;
    let calculation = match byte(r)? {
        0 => Calculation::LegacyEight,
        1 => Calculation::Float64,
        _ => return Err(invalid("Invalid color calculation tag")),
    };
    let profile = match byte(r)? {
        0 => RgbProfile::Srgb,
        1 => RgbProfile::AssumedSrgb,
        2 => {
            let n = u32_in(r)? as usize;
            if !(1..=512).contains(&n) {
                return Err(invalid("Invalid ICC name length"));
            }
            let mut name = vec![0; n];
            r.read_exact(&mut name)?;
            let name = String::from_utf8(name).map_err(invalid)?;
            let n = u32_in(r)? as usize;
            if n > 4 * 1024 * 1024 || d.memory_bytes().saturating_add(n) > budget {
                return Err(invalid("ICC byte budget"));
            }
            let mut bytes = vec![0; n];
            r.read_exact(&mut bytes)?;
            RgbProfile::embedded(name, bytes).map_err(invalid)?
        }
        _ => return Err(invalid("Invalid RGB profile tag")),
    };
    let mut used = d.memory_bytes().saturating_add(profile.memory_bytes());
    for l in &mut d.layers {
        match byte(r)? {
            0 => {}
            1 => {
                let n = l.pixels.len();
                used = used.saturating_add(n.saturating_mul(8));
                if depth != Depth::Sixteen || used > budget {
                    return Err(invalid("Precision payload exceeds budget/depth"));
                }
                let mut pixels = Vec::new();
                pixels.try_reserve_exact(n).map_err(invalid)?;
                for _ in 0..n {
                    let mut p = [0; 4];
                    for v in &mut p {
                        let mut b = [0; 2];
                        r.read_exact(&mut b)?;
                        *v = u16::from_le_bytes(b);
                    }
                    pixels.push(p);
                }
                l.precise = Some(pixels);
            }
            _ => return Err(invalid("Invalid precision layer tag")),
        }
    }
    d.color = Config {
        calculation,
        depth,
        dither: Dither { enabled, seed },
        profile,
    };
    if !d.color.valid_layers(&d.layers) {
        return Err(invalid("Invalid precision cache or transparent RGB"));
    }
    Ok(())
}
