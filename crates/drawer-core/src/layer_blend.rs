//! Encoded RGB channel blending with premultiplied source-over alpha.
use crate::{
    document::Document,
    layer_properties::{Target, properties},
    paper::Color,
};
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Mode {
    #[default]
    Normal,
    Darken,
    Multiply,
    ColorBurn,
    LinearBurn,
    Subtract,
    DarkerColor,
    Lighten,
    Screen,
    ColorDodge,
    GlowDodge,
    Add,
    AddGlow,
    LighterColor,
    Overlay,
    SoftLight,
    HardLight,
    VividLight,
    LinearLight,
    PinLight,
    HardMix,
    Difference,
    Exclusion,
    Hue,
    Saturation,
    Color,
    Brightness,
    Divide,
}
impl Mode {
    pub const ALL: [Self; 28] = [
        Self::Normal,
        Self::Darken,
        Self::Multiply,
        Self::ColorBurn,
        Self::LinearBurn,
        Self::Subtract,
        Self::DarkerColor,
        Self::Lighten,
        Self::Screen,
        Self::ColorDodge,
        Self::GlowDodge,
        Self::Add,
        Self::AddGlow,
        Self::LighterColor,
        Self::Overlay,
        Self::SoftLight,
        Self::HardLight,
        Self::VividLight,
        Self::LinearLight,
        Self::PinLight,
        Self::HardMix,
        Self::Difference,
        Self::Exclusion,
        Self::Hue,
        Self::Saturation,
        Self::Color,
        Self::Brightness,
        Self::Divide,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Normal => "Normal",
            Self::Darken => "Darken",
            Self::Multiply => "Multiply",
            Self::ColorBurn => "Color burn",
            Self::LinearBurn => "Linear burn",
            Self::Subtract => "Subtract",
            Self::DarkerColor => "Darker color",
            Self::Lighten => "Lighten",
            Self::Screen => "Screen",
            Self::ColorDodge => "Color dodge",
            Self::GlowDodge => "Glow dodge",
            Self::Add => "Add",
            Self::AddGlow => "Add (Glow)",
            Self::LighterColor => "Lighter color",
            Self::Overlay => "Overlay",
            Self::SoftLight => "Soft light",
            Self::HardLight => "Hard light",
            Self::VividLight => "Vivid light",
            Self::LinearLight => "Linear light",
            Self::PinLight => "Pin light",
            Self::HardMix => "Hard mix",
            Self::Difference => "Difference",
            Self::Exclusion => "Exclusion",
            Self::Hue => "Hue",
            Self::Saturation => "Saturation",
            Self::Color => "Color",
            Self::Brightness => "Brightness",
            Self::Divide => "Divide",
        }
    }
    pub fn is_normal(&self) -> bool {
        *self == Self::Normal
    }
    pub fn channel(self, back: f64, source: f64) -> f64 {
        let back = unit(back);
        let source = unit(source);
        match self {
            Self::Normal => source,
            Self::Darken => back.min(source),
            Self::Multiply => back * source,
            Self::ColorBurn => burn(back, source),
            Self::LinearBurn => (back + source - 1.).max(0.),
            Self::Subtract => (back - source).max(0.),
            Self::Lighten => back.max(source),
            Self::Screen => back + source - back * source,
            Self::ColorDodge => dodge(back, source),
            // Fixed quadratic glow definition; does not claim a vendor's internal implementation.
            Self::GlowDodge => back.max(if source == 0. {
                0.
            } else if back == 1. {
                1.
            } else {
                (source * source / (1. - back)).min(1.)
            }),
            Self::Add => (back + source).min(1.),
            // Delayed clipping distinguishes this from ordinary clipped addition.
            Self::AddGlow => back + source,
            Self::Overlay => {
                if back <= 0.5 {
                    2. * back * source
                } else {
                    1. - 2. * (1. - back) * (1. - source)
                }
            }
            Self::SoftLight => {
                if source <= 0.5 {
                    back - (1. - 2. * source) * back * (1. - back)
                } else {
                    let d = if back <= 0.25 {
                        ((16. * back - 12.) * back + 4.) * back
                    } else {
                        back.sqrt()
                    };
                    back + (2. * source - 1.) * (d - back)
                }
            }
            Self::HardLight => {
                if source <= 0.5 {
                    2. * back * source
                } else {
                    1. - 2. * (1. - back) * (1. - source)
                }
            }
            Self::VividLight => {
                if source <= 0.5 {
                    burn(back, 2. * source)
                } else {
                    dodge(back, 2. * source - 1.)
                }
            }
            Self::LinearLight => (back + 2. * source - 1.).clamp(0., 1.),
            Self::PinLight => {
                if source <= 0.5 {
                    back.min(2. * source)
                } else {
                    back.max(2. * source - 1.)
                }
            }
            Self::HardMix => {
                if Self::VividLight.channel(back, source) < 0.5 {
                    0.
                } else {
                    1.
                }
            }
            Self::Difference => (back - source).abs(),
            Self::Exclusion => back + source - 2. * back * source,
            Self::Divide => {
                if source == 0. {
                    1.
                } else {
                    (back / source).min(1.)
                }
            }
            Self::DarkerColor
            | Self::LighterColor
            | Self::Hue
            | Self::Saturation
            | Self::Color
            | Self::Brightness => self.rgb([back; 3], [source; 3])[0],
        }
    }
    pub fn rgb(self, back: [f64; 3], source: [f64; 3]) -> [f64; 3] {
        let b = back.map(unit);
        let s = source.map(unit);
        match self {
            Self::DarkerColor => {
                if lum(b) <= lum(s) {
                    b
                } else {
                    s
                }
            }
            Self::LighterColor => {
                if lum(b) >= lum(s) {
                    b
                } else {
                    s
                }
            }
            Self::Hue => set_lum(set_sat(s, sat(b)), lum(b)),
            Self::Saturation => set_lum(set_sat(b, sat(s)), lum(b)),
            Self::Color => set_lum(s, lum(b)),
            Self::Brightness => set_lum(b, lum(s)),
            _ => [0, 1, 2].map(|i| self.channel(b[i], s[i])),
        }
    }
    pub(crate) fn over(self, back: &mut Color, source: Color) {
        let ab = back.alpha as f64;
        let asrc = source.alpha as f64;
        if asrc <= 0. {
            return;
        }
        let cb = [0, 1, 2].map(|i| {
            if ab > 0. {
                back.rgb[i] as f64 / ab / 255.
            } else {
                0.
            }
        });
        let cs = source.rgb.map(|v| v as f64 / asrc / 255.);
        let blended = self.rgb(cb.map(recover), cs.map(recover));
        let alpha = asrc + ab * (1. - asrc);
        for (i, b) in blended.into_iter().enumerate() {
            back.rgb[i] = ((1. - asrc) * back.rgb[i] as f64
                + (1. - ab) * source.rgb[i] as f64
                + asrc * ab * 255. * b)
                .clamp(0., 255. * alpha) as f32;
        }
        back.alpha = alpha as f32;
    }
    pub(crate) fn atop(self, back: &mut Color, source: Color) {
        let ab = back.alpha as f64;
        let asrc = source.alpha as f64;
        if ab <= 0. || asrc <= 0. {
            return;
        }
        let cb = back.rgb.map(|v| v as f64 / ab / 255.);
        let cs = source.rgb.map(|v| v as f64 / asrc / 255.);
        let blended = self.rgb(cb.map(recover), cs.map(recover));
        for (i, b) in blended.into_iter().enumerate() {
            back.rgb[i] = ((1. - asrc) * back.rgb[i] as f64 + asrc * ab * 255. * b)
                .clamp(0., 255. * ab) as f32;
        }
    }
    pub fn compose(self, back: [u8; 4], source: [u8; 4], opacity: f64) -> [u8; 4] {
        let mut b = Color::default();
        b.add(back);
        let mut s = Color::default();
        s.add(source);
        let w = if opacity.is_finite() {
            opacity.clamp(0., 1.) as f32
        } else {
            0.
        };
        s.rgb.iter_mut().for_each(|c| *c *= w);
        s.alpha *= w;
        self.over(&mut b, s);
        b.rgba()
    }
}
fn recover(v: f64) -> f64 {
    if v.abs() < 2e-7 {
        0.
    } else if (v - 1.).abs() < 2e-7 {
        1.
    } else {
        v
    }
}
fn unit(v: f64) -> f64 {
    if v.is_finite() { v.clamp(0., 1.) } else { 0. }
}
fn dodge(b: f64, s: f64) -> f64 {
    if b == 0. {
        0.
    } else if s == 1. {
        1.
    } else {
        (b / (1. - s)).min(1.)
    }
}
fn burn(b: f64, s: f64) -> f64 {
    if b == 1. {
        1.
    } else if s == 0. {
        0.
    } else {
        1. - ((1. - b) / s).min(1.)
    }
}
fn lum(c: [f64; 3]) -> f64 {
    0.3 * c[0] + 0.59 * c[1] + 0.11 * c[2]
}
fn sat(c: [f64; 3]) -> f64 {
    c.into_iter().fold(f64::NEG_INFINITY, f64::max) - c.into_iter().fold(f64::INFINITY, f64::min)
}
fn set_sat(c: [f64; 3], s: f64) -> [f64; 3] {
    let min = c.into_iter().fold(f64::INFINITY, f64::min);
    let max = c.into_iter().fold(f64::NEG_INFINITY, f64::max);
    if max > min {
        c.map(|v| (v - min) * s / (max - min))
    } else {
        [0.; 3]
    }
}
fn set_lum(c: [f64; 3], value: f64) -> [f64; 3] {
    let delta = value - lum(c);
    let mut c = c.map(|v| v + delta);
    let min = c.into_iter().fold(f64::INFINITY, f64::min);
    let max = c.into_iter().fold(f64::NEG_INFINITY, f64::max);
    if min < 0. {
        c = c.map(|v| value + (v - value) * value / (value - min));
    }
    if max > 1. {
        c = c.map(|v| value + (v - value) * (1. - value) / (max - value));
    }
    c.map(unit)
}
pub fn mode(d: &Document, t: Target) -> Mode {
    properties(d, t).blend
}
pub fn exchange_notice(d: &Document) -> Option<String> {
    let targets = crate::layer_properties::targets(d);
    let modes = Mode::ALL
        .into_iter()
        .filter(|m| !m.is_normal() && targets.iter().any(|t| mode(d, *t) == *m))
        .map(Mode::label)
        .collect::<Vec<_>>();
    (!modes.is_empty()).then(||format!("Editable layer blending ({}) is preserved in .drawer. PSD export bakes it into Rendered whole image; layer-mode settings are not exchanged.",modes.join(", ")))
}
pub fn graph_active(d: &Document) -> bool {
    d.scene
        .layer_properties
        .rasters
        .values()
        .chain(d.scene.layer_properties.folders.values())
        .any(|p| !p.blend.is_normal())
}
pub fn vectors_active(d: &Document) -> bool {
    d.scene
        .layer_properties
        .vectors
        .values()
        .any(|p| !p.blend.is_normal())
}
