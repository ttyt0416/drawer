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
    Multiply,
    Screen,
    Overlay,
}
impl Mode {
    pub const ALL: [Self; 4] = [Self::Normal, Self::Multiply, Self::Screen, Self::Overlay];
    pub fn is_normal(&self) -> bool {
        *self == Self::Normal
    }
    pub fn channel(self, back: f64, source: f64) -> f64 {
        match self {
            Self::Normal => source,
            Self::Multiply => back * source,
            Self::Screen => back + source - back * source,
            Self::Overlay => {
                if back <= 0.5 {
                    2. * back * source
                } else {
                    1. - 2. * (1. - back) * (1. - source)
                }
            }
        }
    }
    pub(crate) fn over(self, back: &mut Color, source: Color) {
        let ab = back.alpha as f64;
        let asrc = source.alpha as f64;
        if asrc <= 0. {
            return;
        }
        for i in 0..3 {
            let cb = if ab > 0. {
                back.rgb[i] as f64 / ab / 255.
            } else {
                0.
            };
            let cs = source.rgb[i] as f64 / asrc / 255.;
            back.rgb[i] = ((1. - asrc) * back.rgb[i] as f64
                + (1. - ab) * source.rgb[i] as f64
                + asrc * ab * 255. * self.channel(cb, cs)) as f32;
        }
        back.alpha = (asrc + ab * (1. - asrc)) as f32;
    }
    pub(crate) fn atop(self, back: &mut Color, source: Color) {
        let ab = back.alpha as f64;
        let asrc = source.alpha as f64;
        if ab <= 0. || asrc <= 0. {
            return;
        }
        for i in 0..3 {
            let cb = back.rgb[i] as f64 / ab / 255.;
            let cs = source.rgb[i] as f64 / asrc / 255.;
            back.rgb[i] =
                ((1. - asrc) * back.rgb[i] as f64 + asrc * ab * 255. * self.channel(cb, cs)) as f32;
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
pub fn mode(d: &Document, t: Target) -> Mode {
    properties(d, t).blend
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
