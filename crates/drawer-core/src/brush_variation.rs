//! Seeded HSV variation and independent secondary procedural tip masks.
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Scope {
    #[default]
    Stroke,
    Dab,
}
impl Scope {
    pub const ALL: [Self; 2] = [Self::Stroke, Self::Dab];
    pub fn label(self) -> &'static str {
        match self {
            Self::Stroke => "One color per stroke",
            Self::Dab => "One color per dab",
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Mask {
    #[default]
    Round,
    Square,
    Grain,
    Soft,
}
impl Mask {
    pub const ALL: [Self; 4] = [Self::Round, Self::Square, Self::Grain, Self::Soft];
    pub fn label(self) -> &'static str {
        match self {
            Self::Round => "Owned round",
            Self::Square => "Owned square",
            Self::Grain => "Owned grain",
            Self::Soft => "Owned soft",
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Blend {
    #[default]
    Multiply,
    Add,
    Subtract,
    Minimum,
    Maximum,
    Difference,
    Normal,
    Screen,
    Overlay,
    ColorDodge,
    ColorBurn,
    LinearBurn,
    HardMix,
    HeightLinear,
    ApplyRgb,
}
impl Blend {
    pub const ALL: [Self; 15] = [
        Self::Normal,
        Self::Multiply,
        Self::Add,
        Self::Subtract,
        Self::Minimum,
        Self::Maximum,
        Self::Screen,
        Self::Overlay,
        Self::ColorDodge,
        Self::ColorBurn,
        Self::LinearBurn,
        Self::HardMix,
        Self::HeightLinear,
        Self::ApplyRgb,
        Self::Difference,
    ];
    pub fn channel(self, a: f64, b: f64) -> f64 {
        let a = a.clamp(0.0, 1.0);
        let b = b.clamp(0.0, 1.0);
        match self {
            Self::Normal | Self::ApplyRgb => b,
            Self::Multiply => a * b,
            Self::Add => (a + b).min(1.0),
            Self::Subtract => (a - b).max(0.0),
            Self::Minimum => a.min(b),
            Self::Maximum => a.max(b),
            Self::Difference => (a - b).abs(),
            Self::Screen => a + b - a * b,
            Self::Overlay => {
                if a <= 0.5 {
                    2.0 * a * b
                } else {
                    1.0 - 2.0 * (1.0 - a) * (1.0 - b)
                }
            }
            Self::ColorDodge => {
                if a == 0.0 {
                    0.0
                } else if b == 1.0 {
                    1.0
                } else {
                    (a / (1.0 - b)).min(1.0)
                }
            }
            Self::ColorBurn => {
                if a == 1.0 {
                    1.0
                } else if b == 0.0 {
                    0.0
                } else {
                    1.0 - ((1.0 - a) / b).min(1.0)
                }
            }
            Self::LinearBurn => (a + b - 1.0).max(0.0),
            Self::HardMix => {
                if a + b >= 1.0 {
                    1.0
                } else {
                    0.0
                }
            }
            Self::HeightLinear => (a + b) * 0.5,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Multiply => "Multiply",
            Self::Add => "Add (Glow)",
            Self::Subtract => "Subtract secondary",
            Self::Minimum => "Darken",
            Self::Maximum => "Lighten",
            Self::Difference => "Coverage difference",
            Self::Normal => "Normal",
            Self::Screen => "Screen",
            Self::Overlay => "Overlay",
            Self::ColorDodge => "Color dodge",
            Self::ColorBurn => "Color burn",
            Self::LinearBurn => "Linear burn",
            Self::HardMix => "Hard mix",
            Self::HeightLinear => "Height (Linear)",
            Self::ApplyRgb => "Apply RGB value",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Controls {
    #[serde(default)]
    pub flip: [bool; 2],
    #[serde(default)]
    pub texture: Option<crate::tip_texture::Settings>,
    pub rgb: bool,
    pub color: [u8; 4],
    pub linked_size: bool,
    pub size: f64,
}
impl Default for Controls {
    fn default() -> Self {
        Self {
            flip: [false; 2],
            texture: None,
            rgb: false,
            color: [0, 0, 0, 255],
            linked_size: true,
            size: 8.0,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Dual {
    #[serde(default)]
    pub controls: Controls,
    pub enabled: bool,
    pub mask: Mask,
    pub blend: Blend,
    pub scale: f64,
    pub angle: f64,
    pub offset: [f64; 2],
    pub opacity: f64,
}
impl Default for Dual {
    fn default() -> Self {
        Self {
            controls: Default::default(),
            enabled: false,
            mask: Default::default(),
            blend: Default::default(),
            scale: 0.5,
            angle: 0.0,
            offset: [0.0; 2],
            opacity: 1.0,
        }
    }
}
impl Dual {
    pub fn valid(self) -> bool {
        self.controls.texture.is_none_or(|s| s.valid())
            && (0.1..=4096.0).contains(&self.controls.size)
            && (0.1..=1.0).contains(&self.scale)
            && self.angle.is_finite()
            && self.angle.abs() <= 180.0
            && self.offset.iter().all(|v| (-1.0..=1.0).contains(v))
            && (0.0..=1.0).contains(&self.opacity)
    }
    pub fn active(self) -> bool {
        self.enabled && self.opacity > 0.0
    }
    pub fn effective_scale(self, primary_size: f64) -> f64 {
        if self.controls.linked_size {
            self.scale
        } else {
            self.controls.size / primary_size.max(1e-9)
        }
    }
    pub fn extra_radius(self, primary_size: f64) -> f64 {
        if self.active() {
            self.offset[0].abs().max(self.offset[1].abs())
                + std::f64::consts::SQRT_2 * self.effective_scale(primary_size)
        } else {
            0.0
        }
    }
    pub fn local(self, q: [f64; 2], primary_size: f64) -> [f64; 2] {
        let scale = self.effective_scale(primary_size);
        let (s, c) = self.angle.to_radians().sin_cos();
        let d = [q[0] - self.offset[0], q[1] - self.offset[1]];
        let mut p = [
            (d[0] * c + d[1] * s) / scale,
            (-d[0] * s + d[1] * c) / scale,
        ];
        for (n, v) in p.iter_mut().enumerate() {
            if self.controls.flip[n] {
                *v = -*v;
            }
        }
        p
    }
    pub fn mask(
        self,
        q: [f64; 2],
        minor: f64,
        point: [f64; 2],
        seed: u32,
        primary_size: f64,
    ) -> f64 {
        let scale = self.effective_scale(primary_size);
        let p = self.local(q, primary_size);
        let distance = if self.mask == Mask::Square {
            p[0].abs().max(p[1].abs())
        } else {
            p[0].hypot(p[1])
        };
        let edge = ((1.0 - distance) * minor * scale + 0.5).clamp(0.0, 1.0);
        edge * match self.mask {
            Mask::Soft => (1.0 - distance.powi(2)).clamp(0.0, 1.0).powi(3),
            Mask::Grain => {
                let point = if self.controls.flip.iter().any(|v| *v) {
                    p.map(|v| v * 64.0)
                } else {
                    point
                };
                0.25 + 0.75
                    * random(
                        seed,
                        (point[0].floor() as i64 as u64).wrapping_mul(0x9e3779b97f4a7c15)
                            ^ (point[1].floor() as i64 as u64).rotate_left(31),
                    )
            }
            _ => 1.0,
        }
    }
    fn secondary(self, mask: f64) -> f64 {
        let alpha = self.controls.color[3] as f64 / 255.0;
        let density = if self.blend == Blend::ApplyRgb {
            (self.controls.color[0] as f64 * 54.0
                + self.controls.color[1] as f64 * 183.0
                + self.controls.color[2] as f64 * 19.0)
                / (256.0 * 255.0)
        } else {
            1.0
        };
        mask * alpha * density
    }
    fn combined(self, a: f64, b: f64) -> f64 {
        let value = match self.blend {
            Blend::Normal => a + b - a * b,
            Blend::ApplyRgb => a * b,
            _ => self.blend.channel(a, b),
        };
        (a + (value - a) * self.opacity).clamp(0.0, 1.0)
    }
    pub fn combine(self, a: f64, b: f64) -> f64 {
        if !self.active() {
            a
        } else {
            self.combined(a, self.secondary(b))
        }
    }
    /// Alpha-only uses the primary ink. RGB mode combines two independent inks;
    /// the returned coverage already includes their alpha, so ink alpha is 255.
    pub fn ink(self, a: f64, b: f64, primary: [u8; 4], rgb: bool) -> (f64, [u8; 4]) {
        if !self.active() {
            return (a, primary);
        }
        let b = self.secondary(b);
        if !rgb {
            return (self.combined(a, b), primary);
        }
        let a = a * primary[3] as f64 / 255.0;
        let alpha = self.combined(a, b);
        let source = b * self.opacity;
        let union = a + source * (1.0 - a);
        if union <= 0.0 {
            return (0.0, [0; 4]);
        }
        let color = std::array::from_fn::<_, 3, _>(|n| {
            let base = primary[n] as f64 / 255.0;
            let ink = self.controls.color[n] as f64 / 255.0;
            let result = (source * (1.0 - a) * ink
                + source * a * self.blend.channel(base, ink)
                + (1.0 - source) * a * base)
                / union;
            (result.clamp(0.0, 1.0) * 255.0).round() as u8
        });
        (alpha, [color[0], color[1], color[2], 255])
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub hue: f64,
    pub saturation: f64,
    pub value: f64,
    pub seed: u32,
    pub scope: Scope,
    pub dual: Dual,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            hue: 0.0,
            saturation: 0.0,
            value: 0.0,
            seed: 0,
            scope: Default::default(),
            dual: Default::default(),
        }
    }
}
fn random(seed: u32, index: u64) -> f64 {
    let mut x = (seed as u64).wrapping_add(index.wrapping_add(1).wrapping_mul(0x9e3779b97f4a7c15));
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d049bb133111eb);
    ((x ^ (x >> 31)) >> 11) as f64 / (1u64 << 53) as f64
}
fn hsv(p: [u8; 4]) -> [f64; 3] {
    let [r, g, b] = [p[0], p[1], p[2]].map(|v| v as f64 / 255.0);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let delta = max - min;
    let h = if delta <= 1e-12 {
        0.0
    } else if max == r {
        60.0 * ((g - b) / delta).rem_euclid(6.0)
    } else if max == g {
        60.0 * ((b - r) / delta + 2.0)
    } else {
        60.0 * ((r - g) / delta + 4.0)
    };
    [h, if max > 0.0 { delta / max } else { 0.0 }, max]
}
fn rgb([h, s, v]: [f64; 3], alpha: u8) -> [u8; 4] {
    let h = h.rem_euclid(360.0) / 60.0;
    let c = v * s;
    let x = c * (1.0 - (h.rem_euclid(2.0) - 1.0).abs());
    let m = v - c;
    let rgb = match h as u8 {
        0 => [c, x, 0.0],
        1 => [x, c, 0.0],
        2 => [0.0, c, x],
        3 => [0.0, x, c],
        4 => [x, 0.0, c],
        _ => [c, 0.0, x],
    }
    .map(|n| ((n + m).clamp(0.0, 1.0) * 255.0).round() as u8);
    [rgb[0], rgb[1], rgb[2], alpha]
}
impl Settings {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
    pub fn valid(self) -> bool {
        (0.0..=180.0).contains(&self.hue)
            && (0.0..=1.0).contains(&self.saturation)
            && (0.0..=1.0).contains(&self.value)
            && self.dual.valid()
    }
    pub fn color(self, p: [u8; 4], dab: u64) -> [u8; 4] {
        if self.hue == 0.0 && self.saturation == 0.0 && self.value == 0.0 {
            return p;
        }
        let index = if self.scope == Scope::Stroke { 0 } else { dab };
        let mut c = hsv(p);
        for n in 0..3 {
            c[n] += (random(self.seed, index.wrapping_mul(3).wrapping_add(n as u64)) * 2.0 - 1.0)
                * [self.hue, self.saturation, self.value][n];
        }
        c[1] = c[1].clamp(0.0, 1.0);
        c[2] = c[2].clamp(0.0, 1.0);
        rgb(c, p[3])
    }
}

fn document() -> Result<crate::document::Document, String> {
    crate::document::Document::create(
        crate::document::CanvasSpec {
            width: 64,
            height: 24,
            dpi: 300.0,
            background: crate::document::Background::Transparent,
        },
        65536,
    )
    .map_err(|e| e.to_string())
}
fn style(variation: Settings) -> crate::painting::Style {
    crate::painting::Style {
        clone_stamp: Default::default(),
        variation,
        pattern: Default::default(),
        dynamics: Default::default(),
        sequence: Default::default(),
        filters: Default::default(),
        profile: Default::default(),
        texture: Default::default(),
        material: Default::default(),
        shape: crate::brush_cursor::Shape {
            size: 6.0,
            aspect: 1.0,
            angle: 0.0,
        },
        tip: crate::assets::BrushTip::Pen,
        color: [40, 90, 200, 180],
        opacity: 1.0,
        hardness: 1.0,
        spacing: 0.5,
        smoothing: 0.0,
        pressure: crate::pressure::Mapping {
            size: false,
            opacity: true,
            ..Default::default()
        },
    }
}
fn paint(d: &mut crate::document::Document, s: crate::painting::Style) -> Result<(), String> {
    let mut st = crate::painting::Stroke::begin(d, 0, s)?;
    for x in [6.5, 18.5, 30.5, 42.5, 54.5] {
        st.direct_dab(d, [x, 12.5], [x, 12.5], 1.0)?;
    }
    st.commit(d)?;
    Ok(())
}
pub fn verify() -> Result<(), String> {
    let base = [40, 90, 200, 180];
    for n in 0..20 {
        if Settings::default().color(base, n) != base {
            return Err("Zero variation changed base RGBA".into());
        }
    }
    let settings = Settings {
        hue: 90.0,
        saturation: 0.4,
        value: 0.3,
        seed: 951,
        scope: Scope::Dab,
        ..Default::default()
    };
    let mut d = document()?;
    let before = d.snapshot();
    let s = style(settings);
    let mut pending = crate::painting::Stroke::begin(&d, 0, s)?;
    pending.sample(&d, [12.5, 12.5])?;
    drop(pending);
    if d.snapshot() != before {
        return Err("Variation preview changed source".into());
    }
    paint(&mut d, s)?;
    let after = d.snapshot();
    if d.history_counts() != (1, 0) || !d.undo() || d.snapshot() != before {
        return Err("Variation did not undo atomically".into());
    }
    paint(&mut d, s)?;
    if d.snapshot() != after || !d.undo() || !d.redo() || d.snapshot() != after {
        return Err("Variation seed or redo changed".into());
    }
    for (n, x) in [6, 18, 30, 42, 54].into_iter().enumerate() {
        if d.composite_pixel(x, 12) != settings.color(base, n as u64) {
            return Err("Actual dab did not use frozen seeded color/alpha".into());
        }
    }
    let mut plain = document()?;
    paint(&mut plain, style(Settings::default()))?;
    for blend in Blend::ALL {
        let mut d = document()?;
        let s = style(Settings {
            dual: Dual {
                enabled: true,
                blend,
                opacity: 0.0,
                offset: [1.0, -1.0],
                mask: Mask::Grain,
                ..Default::default()
            },
            ..Default::default()
        });
        paint(&mut d, s)?;
        if d.layers[0].pixels != plain.layers[0].pixels {
            return Err("Zero-alpha secondary tip had an effect".into());
        }
    }
    let mut empty = document()?;
    let mut s = style(settings);
    s.color[3] = 0;
    s.variation.dual = Dual {
        enabled: true,
        blend: Blend::Add,
        ..Default::default()
    };
    paint(&mut empty, s)?;
    if empty.layers[0].pixels.iter().any(|p| p[3] != 0) || empty.history_counts() != (0, 0) {
        return Err("Transparent foreground manufactured ink".into());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn seeded_rgba_dabs_zero_variation_transparency_cancel_and_history() {
        verify().unwrap();
    }
    #[test]
    fn hsv_round_trip_scopes_channels_and_invalid_bounds() {
        for p in [
            [255, 0, 0, 255],
            [0, 255, 0, 128],
            [0, 0, 255, 0],
            [0, 0, 0, 255],
            [255; 4],
            [60, 120, 200, 240],
        ] {
            assert_eq!(rgb(hsv(p), p[3]), p);
        }
        let base = [60, 120, 200, 178];
        for settings in [
            Settings {
                hue: 180.0,
                ..Default::default()
            },
            Settings {
                saturation: 1.0,
                ..Default::default()
            },
            Settings {
                value: 1.0,
                ..Default::default()
            },
        ] {
            let c = settings.color(base, 0);
            assert_ne!(c, base);
            assert_eq!(c[3], base[3]);
        }
        let s = Settings {
            hue: 180.0,
            scope: Scope::Stroke,
            ..Default::default()
        };
        assert_eq!(s.color(base, 0), s.color(base, 7));
        let s = Settings {
            scope: Scope::Dab,
            ..s
        };
        assert_ne!(s.color(base, 0), s.color(base, 7));
        assert_ne!(s.color(base, 0), Settings { seed: 22, ..s }.color(base, 0));
        for hue in [f64::NAN, -0.1, 180.1] {
            assert!(
                !Settings {
                    hue,
                    ..Default::default()
                }
                .valid()
            );
        }
        assert!(
            !Dual {
                offset: [f64::INFINITY, 0.0],
                ..Default::default()
            }
            .valid()
        );
        assert!(
            !Dual {
                opacity: 1.1,
                ..Default::default()
            }
            .valid()
        );
    }
    #[test]
    fn mask_blends_match_independent_numeric_expectations() {
        for (blend, expected) in [
            (Blend::Multiply, 0.54),
            (Blend::Add, 0.8),
            (Blend::Subtract, 0.3),
            (Blend::Minimum, 0.6),
            (Blend::Maximum, 0.7),
            (Blend::Difference, 0.4),
        ] {
            let d = Dual {
                enabled: true,
                blend,
                opacity: 0.5,
                ..Default::default()
            };
            assert!((d.combine(0.6, 0.8) - expected).abs() < 1e-12, "{blend:?}");
            assert_eq!(Dual { opacity: 0.0, ..d }.combine(0.6, 0.8), 0.6);
        }
    }
    #[test]
    fn all_six_blends_change_pixels_and_secondary_mask_can_extend_primary() {
        let mut images = Vec::new();
        for blend in [
            Blend::Multiply,
            Blend::Add,
            Blend::Subtract,
            Blend::Minimum,
            Blend::Maximum,
            Blend::Difference,
        ] {
            let mut d = document().unwrap();
            let mut s = style(Settings {
                dual: Dual {
                    enabled: true,
                    blend,
                    mask: Mask::Grain,
                    scale: 0.9,
                    offset: [0.5, 0.0],
                    opacity: 0.8,
                    ..Default::default()
                },
                ..Default::default()
            });
            s.tip = crate::assets::BrushTip::Pencil;
            s.shape.size = 12.0;
            paint(&mut d, s).unwrap();
            let pixels = d.layers.remove(0).pixels;
            assert!(!images.contains(&pixels), "{blend:?}");
            images.push(pixels);
        }
        let mut d = document().unwrap();
        let mut s = style(Settings {
            dual: Dual {
                enabled: true,
                blend: Blend::Add,
                mask: Mask::Square,
                scale: 1.0,
                offset: [1.0, 0.0],
                ..Default::default()
            },
            ..Default::default()
        });
        s.shape.size = 8.0;
        let mut st = crate::painting::Stroke::begin(&d, 0, s).unwrap();
        st.direct_dab(&d, [12.5, 12.5], [12.5, 12.5], 1.0).unwrap();
        st.commit(&mut d).unwrap();
        assert_eq!(d.composite_pixel(19, 12), s.color);
    }
    #[test]
    fn preset_native_round_trip_and_missing_legacy_fields() {
        use crate::{
            assets::BrushTip,
            brush_presets::Preset,
            mouse_pressure::{Replay, Sample},
            workspaces::Snapshot,
        };
        let root = crate::test_support::TempDir::new();
        let mut snap = Snapshot {
            properties: Some(Default::default()),
            ..Default::default()
        };
        snap.input.variations[0] = Settings {
            hue: 35.0,
            saturation: 0.3,
            value: 0.2,
            seed: 331,
            scope: Scope::Dab,
            dual: Dual {
                enabled: true,
                mask: Mask::Grain,
                blend: Blend::Add,
                ..Default::default()
            },
        };
        let p = Preset::capture(&snap, BrushTip::Pen, "Variation").unwrap();
        let path = root.0.join("variation.json");
        p.save(&path, false).unwrap();
        let p = Preset::read(&path).unwrap();
        assert_eq!(
            p.apply(&Snapshot {
                properties: Some(Default::default()),
                ..Default::default()
            })
            .unwrap()
            .input
            .variations[0],
            snap.input.variations[0]
        );
        let mut d = document().unwrap();
        let s = style(p.variation);
        let mut r = Replay::begin(&d, 0, s, Default::default())
            .unwrap()
            .configure_input(Default::default(), true)
            .unwrap();
        for point in [[6.5, 12.5], [54.5, 12.5]] {
            r.push_input(
                Sample {
                    point,
                    micros: None,
                },
                1.0,
                0.0,
            )
            .unwrap();
        }
        r.render(&d, true)
            .unwrap()
            .commit_mouse(&mut d, r.metadata())
            .unwrap();
        let before = d.snapshot();
        let path = root.0.join("variation.drawer");
        crate::files::save(&mut d, &path).unwrap();
        let reopened = crate::files::open(&path, 65536).unwrap();
        assert!(reopened.snapshot() == before);
        assert_eq!(reopened.scene.mouse_strokes[0].style.variation, p.variation);
        let mut old = serde_json::to_value(s).unwrap();
        old.as_object_mut().unwrap().remove("variation");
        assert!(
            serde_json::from_value::<crate::painting::Style>(old)
                .unwrap()
                .variation
                .is_default()
        );
        let mut old = serde_json::to_value(snap.input).unwrap();
        old.as_object_mut().unwrap().remove("variations");
        assert_eq!(
            serde_json::from_value::<crate::tool_input::Settings>(old)
                .unwrap()
                .variations,
            [Settings::default(); 7]
        );
    }
}
