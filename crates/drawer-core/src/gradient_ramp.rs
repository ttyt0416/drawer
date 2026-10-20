//! Independent multi-stop mixing and local preset data; no product-specific numeric compatibility.
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Space {
    #[default]
    Standard,
    PerceptualA,
    PerceptualB,
    LinearRgb,
}
impl Space {
    pub const ALL: [Self; 4] = [
        Self::Standard,
        Self::PerceptualA,
        Self::PerceptualB,
        Self::LinearRgb,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Standard => "Standard",
            Self::PerceptualA => "Perceptual A (Oklab)",
            Self::PerceptualB => "Perceptual B (CIELAB D65)",
            Self::LinearRgb => "Linear RGB",
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Brightness {
    #[default]
    Off,
    Low,
    Medium,
    High,
    Highest,
}
impl Brightness {
    pub const ALL: [Self; 5] = [
        Self::Off,
        Self::Low,
        Self::Medium,
        Self::High,
        Self::Highest,
    ];
    pub(crate) fn amount(self) -> f64 {
        Self::ALL.iter().position(|x| *x == self).unwrap() as f64 / 4.0
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Curve {
    #[default]
    Linear,
    Smooth,
    EaseIn,
    EaseOut,
}
impl Curve {
    pub const ALL: [Self; 4] = [Self::Linear, Self::Smooth, Self::EaseIn, Self::EaseOut];
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Stop {
    pub position: f64,
    pub color: [u8; 4],
    pub midpoint: f64,
    pub curve: Curve,
}
impl Stop {
    pub fn new(position: f64, color: [u8; 4]) -> Self {
        Self {
            position,
            color,
            midpoint: 0.5,
            curve: Curve::Linear,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Ramp {
    pub stops: Vec<Stop>,
    pub space: Space,
    pub brightness: Brightness,
}
impl Ramp {
    pub fn two(a: [u8; 4], b: [u8; 4]) -> Self {
        Self {
            stops: vec![Stop::new(0.0, a), Stop::new(1.0, b)],
            space: Space::Standard,
            brightness: Brightness::Off,
        }
    }
    pub fn valid(&self) -> bool {
        (2..=32).contains(&self.stops.len())
            && self.stops.iter().all(|s| {
                s.position.is_finite()
                    && (0.0..=1.0).contains(&s.position)
                    && (0.01..=0.99).contains(&s.midpoint)
            })
            && self
                .stops
                .windows(2)
                .all(|s| s[0].position <= s[1].position)
    }
    pub fn sample(&self, t: f64) -> [f64; 4] {
        let i = self.stops.partition_point(|s| s.position <= t);
        if i == 0 {
            return self.stops[0].color.map(f64::from);
        }
        let a = &self.stops[i - 1];
        if i == self.stops.len() || t == a.position {
            return a.color.map(f64::from);
        }
        let b = &self.stops[i];
        let mut w = ((t - a.position) / (b.position - a.position)).clamp(0.0, 1.0);
        w = w.powf(0.5f64.ln() / a.midpoint.ln());
        w = match a.curve {
            Curve::Linear => w,
            Curve::Smooth => w * w * (3.0 - 2.0 * w),
            Curve::EaseIn => w * w,
            Curve::EaseOut => 1.0 - (1.0 - w) * (1.0 - w),
        };
        mix(self.space, self.brightness, a.color, b.color, w)
    }
}
fn luminance(rgb: [f64; 3]) -> f64 {
    rgb[0] * 0.2126 + rgb[1] * 0.7152 + rgb[2] * 0.0722
}
pub(crate) fn mix(
    space: Space,
    brightness: Brightness,
    a: [u8; 4],
    b: [u8; 4],
    w: f64,
) -> [f64; 4] {
    use crate::color_mix::{self as c, Space as Cs};
    let aa = a[3] as f64 * (1.0 - w);
    let ba = b[3] as f64 * w;
    let alpha = aa + ba;
    if alpha == 0.0 {
        return [0.0; 4];
    }
    let rgba = [a[0], a[1], a[2]];
    let rgbb = [b[0], b[1], b[2]];
    let cs = match space {
        Space::Standard | Space::LinearRgb => Cs::Standard,
        Space::PerceptualA => Cs::Perceptual,
        Space::PerceptualB => Cs::LabD65,
    };
    let mut ac = c::coordinates(cs, rgba);
    let mut bc = c::coordinates(cs, rgbb);
    if space == Space::LinearRgb {
        ac = ac.map(c::linear);
        bc = bc.map(c::linear);
    }
    let mixed = std::array::from_fn(|n| (ac[n] * aa + bc[n] * ba) / alpha);
    let mut encoded = if space == Space::LinearRgb {
        mixed.map(c::encoded)
    } else {
        c::rgb_float(cs, mixed)
    };
    if brightness != Brightness::Off {
        let linear = encoded.map(c::linear);
        let current = luminance(linear);
        let desired = (luminance(rgba.map(|v| c::linear(v as f64 / 255.0))) * aa
            + luminance(rgbb.map(|v| c::linear(v as f64 / 255.0))) * ba)
            / alpha;
        let target = current + (desired - current) * brightness.amount();
        encoded = linear
            .map(|v| {
                if target >= current {
                    v + (1.0 - v) * (target - current) / (1.0 - current).max(1e-12)
                } else {
                    v * target / current.max(1e-12)
                }
            })
            .map(c::encoded);
    }
    [
        encoded[0] * 255.0,
        encoded[1] * 255.0,
        encoded[2] * 255.0,
        alpha,
    ]
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Preset {
    pub id: u64,
    pub name: String,
    pub settings: crate::gradient::Settings,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Library {
    pub version: u32,
    pub next_id: u64,
    pub presets: Vec<Preset>,
}
impl Default for Library {
    fn default() -> Self {
        Self {
            version: 1,
            next_id: 1,
            presets: vec![],
        }
    }
}
impl Library {
    pub fn valid(&self) -> bool {
        let mut ids = std::collections::BTreeSet::new();
        let mut names = std::collections::BTreeSet::new();
        self.version == 1
            && self.next_id > 0
            && self.presets.len() <= 32
            && self.presets.iter().all(|p| {
                p.id > 0
                    && p.id < self.next_id
                    && ids.insert(p.id)
                    && !p.name.trim().is_empty()
                    && p.name == p.name.trim()
                    && p.name.chars().count() <= 64
                    && !p.name.chars().any(char::is_control)
                    && names.insert(p.name.to_lowercase())
                    && p.settings.valid()
            })
    }
    pub fn save(
        &mut self,
        id: Option<u64>,
        name: &str,
        settings: crate::gradient::Settings,
    ) -> Result<u64, String> {
        let mut next = self.clone();
        let id = if let Some(id) = id {
            let p = next
                .presets
                .iter_mut()
                .find(|p| p.id == id)
                .ok_or("Missing gradient preset")?;
            p.name = name.trim().into();
            p.settings = settings;
            id
        } else {
            let id = next.next_id;
            next.next_id = id.checked_add(1).ok_or("Gradient preset ID exhausted")?;
            next.presets.push(Preset {
                id,
                name: name.trim().into(),
                settings,
            });
            id
        };
        if !self.valid() || !next.valid() {
            return Err("Invalid/duplicate gradient preset, or local library is full".into());
        }
        *self = next;
        Ok(id)
    }
    pub fn remove(&mut self, id: u64) -> Result<(), String> {
        let mut next = self.clone();
        let index = next
            .presets
            .iter()
            .position(|p| p.id == id)
            .ok_or("Missing gradient preset")?;
        next.presets.remove(index);
        if !next.valid() {
            return Err("Invalid gradient preset library".into());
        }
        *self = next;
        Ok(())
    }
    pub fn export(&self, path: &std::path::Path) -> Result<(), String> {
        if !self.valid() {
            return Err("Invalid gradient preset library".into());
        }
        crate::workspaces::write_json(path, self).map_err(|e| e.to_string())
    }
    pub fn import(path: &std::path::Path) -> Result<Self, String> {
        use std::io::Read;
        let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
        let mut bytes = vec![];
        file.take(256 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        if bytes.len() > 256 * 1024 {
            return Err("Gradient preset file exceeds 256 KiB".into());
        }
        let library: Self = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
        if !library.valid() {
            return Err("Invalid gradient preset file".into());
        }
        Ok(library)
    }
}
#[derive(Default)]
pub struct History {
    undo: Vec<Library>,
    redo: Vec<Library>,
}
impl History {
    pub fn record(&mut self, before: Library, after: &Library) {
        if before != *after {
            self.redo.clear();
            self.undo.push(before);
            if self.undo.len() > 16 {
                self.undo.remove(0);
            }
        }
    }
    pub fn undo(&mut self, current: &mut Library) -> bool {
        let Some(v) = self.undo.pop() else {
            return false;
        };
        self.redo.push(std::mem::replace(current, v));
        true
    }
    pub fn redo(&mut self, current: &mut Library) -> bool {
        let Some(v) = self.redo.pop() else {
            return false;
        };
        self.undo.push(std::mem::replace(current, v));
        true
    }
}
