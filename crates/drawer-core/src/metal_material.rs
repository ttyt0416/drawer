//! Independent metallic foil material. Texture luminance supplies a height field;
//! the optional fallback is an analytic pattern owned by this project.
use crate::{material_layer::Image, scene::Point};
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Metal {
    pub color: [u8; 3],
    pub strength: f64,
    pub gloss: f64,
    pub direction: f64,
    pub texture: Option<Image>,
}
impl Default for Metal {
    fn default() -> Self {
        Self {
            color: [212, 175, 55],
            strength: 1.,
            gloss: 0.6,
            direction: 315.,
            texture: None,
        }
    }
}
impl Metal {
    pub fn valid(&self, missing: bool) -> bool {
        (0. ..=1.).contains(&self.strength)
            && (0. ..=1.).contains(&self.gloss)
            && (0. ..=360.).contains(&self.direction)
            && self.texture.as_ref().is_none_or(|i| i.valid(missing))
    }
    pub fn fallback(&self) -> bool {
        self.texture.as_ref().is_none_or(|i| i.pixels.is_empty())
    }
    pub fn pixel(&self, p: Point, at: impl Fn(Point) -> Option<[u8; 4]>) -> [u8; 4] {
        if !self.valid(true) {
            return [0; 4];
        }
        let fallback = self.fallback();
        let base = if fallback {
            [self.color[0], self.color[1], self.color[2], 255]
        } else {
            at(p).unwrap_or([0; 4])
        };
        if base[3] == 0 {
            return [0; 4];
        }
        if self.strength == 0. {
            return base;
        }
        let luma = |v: [u8; 4]| {
            (0.2126 * v[0] as f64 + 0.7152 * v[1] as f64 + 0.0722 * v[2] as f64) / 255.
        };
        let center = luma(base);
        let height = |q: Point| {
            if fallback {
                0.5 + 0.22 * (q[0] * 0.71).sin() * (q[1] * 0.53).cos()
                    + 0.08 * ((q[0] + q[1]) * 1.63).sin()
            } else {
                at(q).filter(|v| v[3] > 0).map_or(center, luma)
            }
        };
        let dx = (height([p[0] + 1., p[1]]) - height([p[0] - 1., p[1]])) * 2.;
        let dy = (height([p[0], p[1] + 1.]) - height([p[0], p[1] - 1.])) * 2.;
        let (ly, lx) = self.direction.to_radians().sin_cos();
        let diffuse = ((0.75 - dx * lx - dy * ly) / (1. + dx * dx + dy * dy).sqrt()).clamp(0., 1.);
        let specular = diffuse.powf(2. + self.gloss * 50.) * self.gloss;
        let mut output = base;
        for c in 0..3 {
            let tint = self.color[c] as f64 / 255.;
            let metal = (tint * (0.3 + 0.7 * diffuse) + specular * (1. - tint * 0.5)).clamp(0., 1.);
            output[c] = ((base[c] as f64 / 255. * (1. - self.strength) + metal * self.strength)
                * 255.)
                .round() as u8;
        }
        output
    }
}
