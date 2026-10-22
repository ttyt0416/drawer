//! Independent inverse maps, with native-depth premultiplied bilinear sampling.
use crate::{
    color_precision::Sample,
    document::{Document, PlacedRaster},
    scene::Point,
};
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum Operation {
    Fisheye {
        strength: f64,
        radius: f64,
        center: Option<Point>,
    },
    Panorama {
        field_of_view: f64,
        center: Option<Point>,
    },
    Wave {
        amplitude: Point,
        period: Point,
        phase: f64,
    },
    Swirl {
        angle: f64,
        radius: f64,
        center: Option<Point>,
    },
}
impl Operation {
    pub fn defaults() -> [Self; 4] {
        [
            Self::Fisheye {
                strength: 0.5,
                radius: 64.,
                center: None,
            },
            Self::Panorama {
                field_of_view: 90.,
                center: None,
            },
            Self::Wave {
                amplitude: [4., 0.],
                period: [16.; 2],
                phase: 0.,
            },
            Self::Swirl {
                angle: 90.,
                radius: 64.,
                center: None,
            },
        ]
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Fisheye { .. } => "Fisheye",
            Self::Panorama { .. } => "Panorama",
            Self::Wave { .. } => "Wave",
            Self::Swirl { .. } => "Swirl",
        }
    }
    pub fn valid(self) -> bool {
        let center =
            |c: Option<Point>| c.is_none_or(|p| p.iter().all(|v| v.is_finite() && v.abs() <= 1e6));
        let radius = |r: f64| (0. ..=32768.).contains(&r);
        match self {
            Self::Fisheye {
                strength,
                radius: r,
                center: c,
            } => (-1. ..=1.).contains(&strength) && radius(r) && center(c),
            Self::Panorama {
                field_of_view,
                center: c,
            } => (0. ..=179.).contains(&field_of_view) && center(c),
            Self::Wave {
                amplitude,
                period,
                phase,
            } => {
                amplitude.iter().all(|v| (0. ..=1024.).contains(v))
                    && period.iter().all(|v| (0.25..=32768.).contains(v))
                    && (-36000. ..=36000.).contains(&phase)
            }
            Self::Swirl {
                angle,
                radius: r,
                center: c,
            } => (-720. ..=720.).contains(&angle) && radius(r) && center(c),
        }
    }
    pub fn identity(self) -> bool {
        match self {
            Self::Fisheye {
                strength, radius, ..
            } => strength == 0. || radius == 0.,
            Self::Panorama { field_of_view, .. } => field_of_view < 1e-6,
            Self::Wave { amplitude, .. } => amplitude == [0.; 2],
            Self::Swirl { angle, radius, .. } => angle == 0. || radius == 0.,
        }
    }
    pub fn map(self, p: Point, extent: PlacedRaster) -> Point {
        if self.identity() {
            return p;
        }
        let auto = [
            extent.offset[0] as f64 + extent.width as f64 / 2.,
            extent.offset[1] as f64 + extent.height as f64 / 2.,
        ];
        match self {
            Self::Fisheye {
                strength,
                radius,
                center,
            } => {
                let c = center.unwrap_or(auto);
                let v = [p[0] - c[0], p[1] - c[1]];
                let r = v[0].hypot(v[1]) / radius;
                if r >= 1. {
                    return p;
                }
                let scale = 1. + strength * (r * r - 1.);
                [c[0] + v[0] * scale, c[1] + v[1] * scale]
            }
            Self::Panorama {
                field_of_view,
                center,
            } => {
                let c = center.unwrap_or(auto);
                let half = field_of_view.to_radians() / 2.;
                let theta = (p[0] - c[0]) / (extent.width as f64 / 2.) * half;
                if theta.abs() >= std::f64::consts::FRAC_PI_2 {
                    return [f64::INFINITY; 2];
                }
                [
                    c[0] + theta.tan() / half.tan() * extent.width as f64 / 2.,
                    c[1] + (p[1] - c[1]) / theta.cos(),
                ]
            }
            Self::Wave {
                amplitude,
                period,
                phase,
            } => {
                let q = [
                    p[0] - extent.offset[0] as f64,
                    p[1] - extent.offset[1] as f64,
                ];
                let phase = phase.rem_euclid(360.).to_radians();
                [
                    p[0] + amplitude[0] * (std::f64::consts::TAU * q[1] / period[0] + phase).sin(),
                    p[1] + amplitude[1] * (std::f64::consts::TAU * q[0] / period[1] + phase).sin(),
                ]
            }
            Self::Swirl {
                angle,
                radius,
                center,
            } => {
                let c = center.unwrap_or(auto);
                let v = [p[0] - c[0], p[1] - c[1]];
                let r = v[0].hypot(v[1]) / radius;
                if r >= 1. {
                    return p;
                }
                let a = -angle.to_radians() * (1. - r).powi(2);
                let (s, cos) = a.sin_cos();
                [c[0] + v[0] * cos - v[1] * s, c[1] + v[0] * s + v[1] * cos]
            }
        }
    }
}
pub fn sample(d: &Document, layer: usize, xy: [i64; 2], op: Operation) -> Result<Sample, String> {
    if !op.valid() {
        return Err("Invalid distortion parameters".into());
    }
    let src = &d.layers[layer];
    let p = op
        .map(xy.map(|v| v as f64 + 0.5), src.extent(d.width, d.height))
        .map(|v| v - 0.5);
    Ok(crate::interpolation::sample_float(
        p,
        [
            [p[0] - 0.5, p[1] - 0.5],
            [p[0] + 0.5, p[1] - 0.5],
            [p[0] + 0.5, p[1] + 0.5],
            [p[0] - 0.5, p[1] + 0.5],
        ],
        crate::interpolation::Filter::Bilinear,
        |x, y| src.sample(d.width, d.height, x, y),
    ))
}
