//! Shared document-edge sampling and persisted, operation-specific filter options.
use crate::{
    color_precision::Sample,
    document::{Document, PlacedRaster},
    scene::Point,
};
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Boundary {
    #[default]
    Transparent,
    Edge,
    Wrap,
    Mirror,
}
impl Boundary {
    pub const ALL: [Self; 4] = [Self::Transparent, Self::Edge, Self::Wrap, Self::Mirror];
    pub fn label(self) -> &'static str {
        match self {
            Self::Transparent => "Transparent",
            Self::Edge => "Repeat edge pixels",
            Self::Wrap => "Wrap around",
            Self::Mirror => "Mirror repeat",
        }
    }
    pub fn coordinate(self, v: i64, size: u32) -> Option<i64> {
        let n = size as i64;
        if n == 0 {
            return None;
        }
        Some(match self {
            Self::Transparent => {
                if !(0..n).contains(&v) {
                    return None;
                }
                v
            }
            Self::Edge => v.clamp(0, n - 1),
            Self::Wrap => v.rem_euclid(n),
            Self::Mirror => {
                let q = v.rem_euclid(2 * n);
                if q < n { q } else { 2 * n - 1 - q }
            }
        })
    }
}
pub struct Sampler<'a> {
    pub document: &'a Document,
    pub layer: usize,
    pub boundary: Boundary,
}
impl Sampler<'_> {
    pub fn at(&self, x: i64, y: i64) -> Sample {
        let d = self.document;
        match (
            self.boundary.coordinate(x, d.width),
            self.boundary.coordinate(y, d.height),
        ) {
            (Some(x), Some(y)) => d.layers[self.layer].sample(d.width, d.height, x, y),
            _ => Sample::default(),
        }
    }
    pub fn bilinear(&self, p: Point) -> Sample {
        if p.iter().any(|v| !v.is_finite()) {
            return Sample::default();
        }
        let size = [self.document.width, self.document.height];
        let p = [0, 1].map(|k| match self.boundary {
            Boundary::Transparent => p[k].clamp(-1., size[k] as f64),
            Boundary::Edge => p[k].clamp(0., size[k].saturating_sub(1) as f64),
            Boundary::Wrap => p[k].rem_euclid(size[k] as f64),
            Boundary::Mirror => p[k].rem_euclid(size[k] as f64 * 2.),
        });
        crate::interpolation::sample_float(
            p,
            [p; 4],
            crate::interpolation::Filter::Bilinear,
            |x, y| self.at(x, y),
        )
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Waveform {
    #[default]
    Sine,
    Triangle,
    Rectangular,
}
impl Waveform {
    pub fn value(self, cycles: f64) -> f64 {
        let t = cycles.rem_euclid(1.);
        match self {
            Self::Sine => (t * std::f64::consts::TAU).sin(),
            Self::Triangle => {
                1. - 4.
                    * (t - 0.25)
                        .rem_euclid(1.)
                        .abs()
                        .min(1. - (t - 0.25).rem_euclid(1.))
            }
            Self::Rectangular => {
                if t < 0.5 {
                    1.
                } else {
                    -1.
                }
            }
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Area {
    #[default]
    Specify,
    EntireSelection,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Abstraction {
    #[default]
    ColorAndLines,
    ColorOnly,
    LinesOnly,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Aberration {
    #[default]
    Offset,
    Lateral,
    Radial,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Film {
    #[default]
    None,
    Sepia,
    Slant,
    All,
    SepiaSlant,
    SepiaAll,
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Options {
    pub boundary: Boundary,
    pub waveform: Waveform,
    pub area: Area,
    /// Ellipse width/height ratio for explicitly bounded effects.
    pub shape: f64,
    pub spherize: bool,
    pub abstraction: Abstraction,
    pub aberration: Aberration,
    pub angle: f64,
    pub center: Option<Point>,
    pub film: Film,
    pub minimum_line: bool,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            boundary: Boundary::Transparent,
            waveform: Waveform::Sine,
            area: Area::Specify,
            shape: 1.,
            spherize: false,
            abstraction: Abstraction::ColorAndLines,
            aberration: Aberration::Offset,
            angle: 0.,
            center: None,
            film: Film::None,
            minimum_line: false,
        }
    }
}
impl Options {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
    pub fn valid(self) -> bool {
        (0.1..=10.).contains(&self.shape)
            && self.angle.is_finite()
            && self.angle.abs() <= 36000.
            && self
                .center
                .is_none_or(|p| p.iter().all(|v| v.is_finite() && v.abs() <= 1e6))
    }
    pub fn region(self, d: &Document, e: PlacedRaster) -> PlacedRaster {
        if self.area == Area::Specify {
            return e;
        }
        let mut rect = crate::scene::Rect {
            min: [f64::INFINITY; 2],
            max: [f64::NEG_INFINITY; 2],
        };
        let mut include = |p: Point| {
            let q = d.scene.selection_point(p);
            for (k, v) in q.into_iter().enumerate() {
                rect.min[k] = rect.min[k].min(v);
                rect.max[k] = rect.max[k].max(v);
            }
        };
        if let Some(mask) = &d.scene.selection_mask {
            for r in &mask.runs {
                for p in [
                    [r.x as f64, r.y as f64],
                    [r.x as f64 + r.length as f64, r.y as f64],
                    [r.x as f64, r.y as f64 + 1.],
                    [r.x as f64 + r.length as f64, r.y as f64 + 1.],
                ] {
                    include(p);
                }
            }
        } else if let Some(r) = d.scene.selection {
            for p in crate::deform::corners(r) {
                include(p);
            }
        } else {
            return PlacedRaster {
                offset: [0; 2],
                width: d.width,
                height: d.height,
            };
        }
        if rect.min.iter().chain(&rect.max).any(|v| !v.is_finite()) {
            return e;
        }
        let min = rect.min.map(f64::floor);
        let max = rect.max.map(f64::ceil);
        PlacedRaster {
            offset: min.map(|v| v as i64),
            width: (max[0] - min[0]).clamp(1., 32768.) as u32,
            height: (max[1] - min[1]).clamp(1., 32768.) as u32,
        }
    }
    pub fn ellipse(self, p: Point, center: Point) -> Point {
        [center[0] + (p[0] - center[0]) / self.shape, p[1]]
    }
}
pub fn active(scene: &crate::scene::Scene) -> bool {
    let detail = |s: crate::detail_filter::Settings| !s.options.is_default();
    scene.last_detail_filter.is_some_and(detail)
        || scene
            .last_spatial_blur
            .is_some_and(|s| !s.options_default())
        || scene
            .last_batch_filter
            .as_ref()
            .is_some_and(|r| match r.task {
                crate::batch_filters::Task::Detail(s) => detail(s),
                crate::batch_filters::Task::Blur(s) => !s.options_default(),
                _ => false,
            })
}
