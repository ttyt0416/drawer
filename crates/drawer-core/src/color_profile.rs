//! Embedded RGB ICC boundaries. Alpha bypasses the CMS; no integer transform stages.
use crate::color_precision::Sample;
use moxcms::{
    ColorProfile, DataColorSpace, Layout, ParsingOptions, TransformF64Executor, TransformOptions,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum RgbProfile {
    #[default]
    Srgb,
    AssumedSrgb,
    Embedded {
        name: String,
        bytes: Vec<u8>,
    },
}
impl RgbProfile {
    pub fn label(&self) -> &str {
        match self {
            Self::Srgb => "sRGB",
            Self::AssumedSrgb => "No RGB profile: interpreted as sRGB",
            Self::Embedded { name, .. } => name,
        }
    }
    pub fn memory_bytes(&self) -> usize {
        match self {
            Self::Embedded { name, bytes } => name.len() + bytes.len(),
            _ => 0,
        }
    }
    pub fn embedded(name: String, bytes: Vec<u8>) -> Result<Self, String> {
        let profile = Self::Embedded { name, bytes };
        profile.parse()?;
        Ok(profile)
    }
    pub fn valid(&self) -> bool {
        self.parse().is_ok()
    }
    fn parse(&self) -> Result<ColorProfile, String> {
        match self {
            Self::Srgb | Self::AssumedSrgb => Ok(ColorProfile::new_srgb()),
            Self::Embedded { name, bytes } => {
                if name.trim().is_empty()
                    || name.chars().count() > 128
                    || name.chars().any(char::is_control)
                    || bytes.len() > 4 * 1024 * 1024
                {
                    return Err("Invalid RGB ICC name or size".into());
                }
                let profile = ColorProfile::new_from_slice_with_options(
                    bytes,
                    ParsingOptions {
                        max_profile_size: 4 * 1024 * 1024,
                        max_allowed_clut_size: 4 * 1024 * 1024,
                        max_allowed_trc_size: 1_000_000,
                    },
                )
                .map_err(|e| format!("Invalid RGB ICC: {e}"))?;
                if profile.color_space != DataColorSpace::Rgb {
                    return Err("This is not an RGB document profile; use an explicit input/output color adapter".into());
                }
                Ok(profile)
            }
        }
    }
    pub fn equivalent(&self, other: &Self) -> bool {
        self.srgb() && other.srgb()
            || match (self, other) {
                (Self::Embedded { bytes: a, .. }, Self::Embedded { bytes: b, .. }) => a == b,
                _ => false,
            }
    }
    fn srgb(&self) -> bool {
        matches!(self, Self::Srgb | Self::AssumedSrgb)
    }
}
pub struct Transform {
    executor: Option<Arc<TransformF64Executor>>,
}
impl Transform {
    pub fn new(from: &RgbProfile, to: &RgbProfile) -> Result<Self, String> {
        let source = from.parse()?;
        let target = to.parse()?;
        if from.equivalent(to) {
            return Ok(Self { executor: None });
        }
        let executor = source
            .create_transform_f64(
                Layout::Rgb,
                &target,
                Layout::Rgb,
                TransformOptions {
                    rendering_intent: moxcms::RenderingIntent::RelativeColorimetric,
                    prefer_fixed_point: false,
                    allow_extended_range_rgb_xyz: true,
                    ..Default::default()
                },
            )
            .map_err(|e| format!("Unsupported RGB ICC transform: {e}"))?;
        Ok(Self {
            executor: Some(executor),
        })
    }
    pub fn map(&self, sample: Sample) -> Result<Sample, String> {
        if sample.alpha() == 0. {
            return Ok(Sample::default());
        }
        let Some(executor) = &self.executor else {
            return Ok(sample);
        };
        let mut rgb = [0.; 3];
        executor
            .transform(&sample.rgb(), &mut rgb)
            .map_err(|e| e.to_string())?;
        sample.with_rgb(rgb)
    }
}
pub struct WorkingSpace {
    to_srgb: Transform,
    from_srgb: Transform,
}
impl WorkingSpace {
    pub fn average(
        &self,
        samples: impl IntoIterator<Item = (Sample, f64)>,
        space: crate::color_mix::Space,
    ) -> Result<Sample, String> {
        let mut sum = [0.; 3];
        let (mut alpha, mut weight) = (0., 0.);
        for (p, w) in samples {
            if !w.is_finite() || w < 0. {
                return Err("Invalid pigment sample weight".into());
            }
            if w == 0. {
                continue;
            }
            let p = self.to_srgb(p)?;
            let a = p.alpha() * w;
            let c = crate::color_mix::coordinates_float(space, p.rgb());
            for n in 0..3 {
                sum[n] += c[n] * a;
            }
            alpha += a;
            weight += w;
        }
        if alpha == 0. || weight == 0. {
            return Ok(Sample::default());
        }
        let rgb = if space == crate::color_mix::Space::Standard {
            sum.map(|v| v / alpha)
        } else {
            crate::color_mix::rgb_float(space, sum.map(|v| v / alpha))
        };
        self.from_srgb(Sample::new([rgb[0], rgb[1], rgb[2], alpha / weight])?)
    }
    pub fn new(profile: &RgbProfile) -> Result<Self, String> {
        Ok(Self {
            to_srgb: Transform::new(profile, &RgbProfile::Srgb)?,
            from_srgb: Transform::new(&RgbProfile::Srgb, profile)?,
        })
    }
    pub fn to_srgb(&self, value: Sample) -> Result<Sample, String> {
        self.to_srgb.map(value)
    }
    pub fn from_srgb(&self, value: Sample) -> Result<Sample, String> {
        self.from_srgb.map(value)
    }
    pub fn compose(
        &self,
        back: Sample,
        source: Sample,
        mode: crate::layer_blend::Mode,
        opacity: f64,
    ) -> Result<Sample, String> {
        if !opacity.is_finite() {
            return Err("Nonfinite layer opacity".into());
        }
        let a = source.alpha() * opacity.clamp(0., 1.);
        if a == 0. {
            return Ok(back);
        }
        if back.alpha() == 0. || mode.is_normal() && a == 1. {
            return Ok(source.coverage(opacity)?.clamped());
        }
        if mode.is_normal() && back.rgb() == source.rgb() {
            let rgb = back.rgb();
            return Ok(
                Sample::new([rgb[0], rgb[1], rgb[2], a + back.alpha() * (1. - a)])?.clamped(),
            );
        }
        let b = self.to_srgb(back)?;
        let s = self.to_srgb(source)?;
        Ok(self.from_srgb(b.over(s, mode, opacity)?)?.clamped())
    }
    pub fn atop(
        &self,
        back: Sample,
        source: Sample,
        mode: crate::layer_blend::Mode,
        opacity: f64,
    ) -> Result<Sample, String> {
        if !opacity.is_finite() {
            return Err("Nonfinite clipping opacity".into());
        }
        if opacity <= 0.
            || source.alpha() == 0.
            || back.alpha() == 0.
            || mode.is_normal() && back.rgb() == source.rgb()
        {
            return Ok(back);
        }
        Ok(self
            .from_srgb(
                self.to_srgb(back)?
                    .atop(self.to_srgb(source)?, mode, opacity)?,
            )?
            .clamped())
    }
    pub fn brush(
        &self,
        back: Sample,
        source: Sample,
        mode: crate::brush_composite::Mode,
        opacity: f64,
        point: [i64; 2],
        seed: u32,
    ) -> Result<Sample, String> {
        use crate::brush_composite::Mode as B;
        if mode == B::Normal {
            return self.compose(back, source, crate::layer_blend::Mode::Normal, opacity);
        }
        if mode == B::Background {
            return self.compose(
                source.coverage(opacity)?,
                back,
                crate::layer_blend::Mode::Normal,
                1.,
            );
        }
        if !matches!(mode, B::BlackBurn | B::WhiteBurn) {
            return Ok(back.brush(source, mode, opacity, point, seed)?.clamped());
        }
        Ok(self
            .from_srgb(self.to_srgb(back)?.brush(
                self.to_srgb(source)?,
                mode,
                opacity,
                point,
                seed,
            )?)?
            .clamped())
    }
}

impl WorkingSpace {
    pub fn mix(
        &self,
        back: Sample,
        source: Sample,
        weight: f64,
        space: crate::color_mix::Space,
        brightness: f64,
    ) -> Result<Sample, String> {
        if !weight.is_finite() {
            return Err("Nonfinite mixing weight".into());
        }
        if weight <= 0. {
            return Ok(back);
        }
        if weight >= 1. {
            return Ok(source);
        }
        if back.rgb() == source.rgb() {
            let rgb = back.rgb();
            return Sample::new([
                rgb[0],
                rgb[1],
                rgb[2],
                back.alpha() * (1. - weight) + source.alpha() * weight,
            ]);
        }
        Ok(self
            .from_srgb(self.to_srgb(back)?.mix_space(
                self.to_srgb(source)?,
                weight,
                space,
                brightness,
            )?)?
            .clamped())
    }
    pub fn tonal(
        &self,
        base: Sample,
        settings: crate::tonal::Settings,
        amount: f64,
    ) -> Result<Sample, String> {
        let source = self.to_srgb(base)?;
        let adjusted = source.tonal(settings, amount)?;
        if adjusted == source {
            Ok(base)
        } else {
            Ok(self.from_srgb(adjusted)?.clamped())
        }
    }
}
