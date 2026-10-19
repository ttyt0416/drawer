//! Shared composition data for masks and adjustment layers; editors are separate features.
use crate::{document::Document, layer_properties::Target, scene::Point};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Mask {
    pub coverage: crate::selection::Mask,
    pub offset: Point,
    pub enabled: bool,
    pub linked: bool,
    pub inverted: bool,
}
impl Mask {
    pub fn factor(&self, p: Point) -> f32 {
        if !self.enabled {
            return 1.;
        }
        let a = self
            .coverage
            .coverage([p[0] - self.offset[0], p[1] - self.offset[1]]) as f32
            / 255.;
        if self.inverted { 1. - a } else { a }
    }
    fn valid(&self) -> bool {
        self.coverage.valid() && self.offset.iter().all(|n| n.is_finite() && n.abs() <= 1e9)
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ColorModel {
    #[default]
    Hsl,
    Hsv,
}
fn legacy_color_model() -> ColorModel {
    ColorModel::Hsv
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Adjustment {
    #[serde(default = "legacy_color_model")]
    pub color_model: ColorModel,
    pub brightness: f64,
    pub contrast: f64,
    pub hue: f64,
    pub saturation: f64,
}
impl Default for Adjustment {
    fn default() -> Self {
        Self {
            brightness: 0.,
            contrast: 0.,
            hue: 0.,
            saturation: 0.,
            color_model: ColorModel::Hsl,
        }
    }
}
impl Adjustment {
    pub fn identity(self) -> bool {
        self.brightness == 0. && self.contrast == 0. && self.hue == 0. && self.saturation == 0.
    }
    pub fn valid(self) -> bool {
        [self.brightness, self.contrast, self.saturation]
            .iter()
            .all(|n| (-1. ..=1.).contains(n))
            && (-180. ..=180.).contains(&self.hue)
    }
    pub fn pixel(self, mut p: [u8; 4]) -> [u8; 4] {
        if self.identity() || p[3] == 0 {
            return p;
        }
        let contrast = if self.contrast > 0. {
            1. / (1. - self.contrast * 0.99)
        } else {
            1. + self.contrast
        };
        for c in &mut p[..3] {
            *c = (((*c as f64 / 255. - 0.5) * contrast + 0.5 + self.brightness).clamp(0., 1.)
                * 255.)
                .round() as u8;
        }
        if self.hue != 0. || self.saturation != 0. {
            let [h, s, v] = match self.color_model {
                ColorModel::Hsl => crate::color_selector::hsl(p, 0.),
                ColorModel::Hsv => crate::color_selector::hsv(p, 0.),
            };
            let values = [
                (h + self.hue).rem_euclid(360.),
                (s * (1. + self.saturation)).clamp(0., 1.),
                v,
            ];
            p = match self.color_model {
                ColorModel::Hsl => crate::color_selector::from_hsl(values, p[3]),
                ColorModel::Hsv => crate::color_selector::from_hsv(values, p[3]),
            }
            .unwrap_or(p);
        }
        p
    }
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Effect {
    pub mask: Option<Mask>,
    pub adjustment: Option<Adjustment>,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Data {
    pub rasters: BTreeMap<u64, Effect>,
    pub folders: BTreeMap<u16, Effect>,
    pub vectors: BTreeMap<u64, Effect>,
}
impl Data {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
    pub fn valid(&self) -> bool {
        self.rasters.len() <= 256
            && self.folders.len() <= 256
            && self.vectors.len() <= 32
            && self
                .rasters
                .keys()
                .chain(self.vectors.keys())
                .all(|n| *n > 0)
            && self.folders.keys().all(|n| *n < 256)
            && self
                .folders
                .values()
                .chain(self.vectors.values())
                .all(|e| e.adjustment.is_none())
            && self
                .rasters
                .values()
                .chain(self.folders.values())
                .chain(self.vectors.values())
                .all(|e| {
                    e.mask.as_ref().is_none_or(Mask::valid)
                        && e.adjustment.is_none_or(Adjustment::valid)
                })
            && self
                .rasters
                .values()
                .chain(self.folders.values())
                .chain(self.vectors.values())
                .filter_map(|e| e.mask.as_ref())
                .map(|m| m.coverage.runs.len())
                .sum::<usize>()
                <= 32768
    }
    pub fn bytes(&self) -> usize {
        self.rasters
            .values()
            .chain(self.folders.values())
            .chain(self.vectors.values())
            .map(|e| {
                128 + e.mask.as_ref().map_or(0, |m| {
                    m.coverage.runs.len() * std::mem::size_of::<crate::selection::Run>()
                })
            })
            .sum()
    }
    pub fn get(&self, t: Target) -> Option<&Effect> {
        match t {
            Target::Raster(n) => self.rasters.get(&n),
            Target::Folder(n) => self.folders.get(&n),
            Target::Vector(n) => self.vectors.get(&n),
            _ => None,
        }
    }
    pub fn remove(&mut self, t: Target) {
        match t {
            Target::Raster(n) => {
                self.rasters.remove(&n);
            }
            Target::Folder(n) => {
                self.folders.remove(&n);
            }
            Target::Vector(n) => {
                self.vectors.remove(&n);
            }
            _ => {}
        }
    }
    pub fn mask(&self, t: Target, p: Point) -> f32 {
        self.get(t)
            .and_then(|e| e.mask.as_ref())
            .map_or(1., |m| m.factor(p))
    }
    pub fn graph_active(&self) -> bool {
        !self.rasters.is_empty() || !self.folders.is_empty()
    }
}
pub fn adjustment(d: &Document, t: Target) -> Option<Adjustment> {
    d.scene.layer_effects.get(t).and_then(|e| e.adjustment)
}

pub fn shift(
    scene: &mut crate::scene::Scene,
    t: crate::line_tools::Target,
    delta: Point,
) -> Result<(), String> {
    crate::appearance::shift(scene, t, delta);
    let inv = scene.transform.inverse();
    let shift = [
        inv.0[0] * delta[0] + inv.0[2] * delta[1],
        inv.0[1] * delta[0] + inv.0[3] * delta[1],
    ];
    let effect = match t {
        crate::line_tools::Target::Raster(n) => {
            scene.layer_effects.rasters.get_mut(&scene.layer_ids.id(n))
        }
        crate::line_tools::Target::Vector(n) => scene.layer_effects.vectors.get_mut(&n),
        _ => None,
    };
    if let Some(mask) = effect.and_then(|e| e.mask.as_mut()).filter(|m| m.linked) {
        let next = [mask.offset[0] + shift[0], mask.offset[1] + shift[1]];
        if next.iter().any(|n| !n.is_finite() || n.abs() > 1e9) {
            return Err("Linked mask position limit".into());
        }
        mask.offset = next;
    }
    Ok(())
}
