//! Independent tone-band dodge/burn, preserving alpha and pixels outside the band.
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Mode {
    #[default]
    Off,
    Dodge,
    Burn,
}
impl Mode {
    pub const ALL: [Self; 3] = [Self::Off, Self::Dodge, Self::Burn];
    pub fn label(self) -> &'static str {
        match self {
            Self::Off => "Color brush",
            Self::Dodge => "Dodge (lighten)",
            Self::Burn => "Burn (darken)",
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Band {
    Shadows,
    #[default]
    Midtones,
    Highlights,
}
impl Band {
    pub const ALL: [Self; 3] = [Self::Shadows, Self::Midtones, Self::Highlights];
    pub fn label(self) -> &'static str {
        match self {
            Self::Shadows => "Shadows (0–84)",
            Self::Midtones => "Midtones (85–169)",
            Self::Highlights => "Highlights (170–255)",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub mode: Mode,
    pub band: Band,
    pub strength: f64,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            mode: Mode::Off,
            band: Band::Midtones,
            strength: 0.5,
        }
    }
}
impl Settings {
    pub fn valid(self) -> bool {
        (0.0..=1.0).contains(&self.strength)
    }
    pub fn apply(self, base: [u8; 4], amount: f64) -> [u8; 4] {
        let l = (54 * base[0] as u32 + 183 * base[1] as u32 + 19 * base[2] as u32) / 256;
        let band = if l < 85 {
            Band::Shadows
        } else if l < 170 {
            Band::Midtones
        } else {
            Band::Highlights
        };
        if self.mode == Mode::Off || base[3] == 0 || band != self.band {
            return base;
        }
        let weight = self.strength * amount.clamp(0.0, 1.0);
        let mut result = base;
        for c in &mut result[..3] {
            let v = *c as f64;
            *c = match self.mode {
                Mode::Dodge => (v + (255.0 - v) * weight).round() as u8,
                Mode::Burn => (v * (1.0 - weight)).round() as u8,
                Mode::Off => *c,
            };
        }
        result
    }
}
