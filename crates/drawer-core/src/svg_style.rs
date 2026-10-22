//! Self-contained gradient and clipping data for imported vector outlines.
use crate::scene::{Affine, Point};
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Rule {
    #[default]
    EvenOdd,
    NonZero,
}
pub fn contains(points: &[Point], p: Point, rule: Rule) -> bool {
    if rule == Rule::EvenOdd {
        return crate::shapes::contains(points, p);
    }
    let mut winding = 0i32;
    for i in 0..points.len() {
        let a = points[i];
        let b = points[(i + 1) % points.len()];
        let cross = (b[0] - a[0]) * (p[1] - a[1]) - (p[0] - a[0]) * (b[1] - a[1]);
        if a[1] <= p[1] && b[1] > p[1] && cross > 0. {
            winding += 1;
        } else if a[1] > p[1] && b[1] <= p[1] && cross < 0. {
            winding -= 1;
        }
    }
    winding != 0
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum Clip {
    Path { points: Vec<Point>, rule: Rule },
    Union(Vec<Clip>),
    Intersection(Vec<Clip>),
}
impl Clip {
    fn valid_at(&self, depth: usize, count: &mut usize) -> bool {
        if depth > 16 {
            return false;
        }
        match self {
            Self::Path { points, .. } => {
                *count += points.len();
                (3..=4096).contains(&points.len())
                    && *count <= 8192
                    && points.iter().copied().all(crate::line_tools::point_valid)
            }
            Self::Union(v) | Self::Intersection(v) => {
                v.len() <= 128 && v.iter().all(|c| c.valid_at(depth + 1, count))
            }
        }
    }
    pub fn hit(&self, p: Point) -> bool {
        match self {
            Self::Path { points, rule } => contains(points, p, *rule),
            Self::Union(v) => v.iter().any(|c| c.hit(p)),
            Self::Intersection(v) => v.iter().all(|c| c.hit(p)),
        }
    }
    pub fn transform(&mut self, matrix: Affine) {
        match self {
            Self::Path { points, .. } => {
                for p in points {
                    *p = matrix.map(*p);
                }
            }
            Self::Union(v) | Self::Intersection(v) => {
                for c in v {
                    c.transform(matrix);
                }
            }
        }
    }
    fn bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + match self {
                Self::Path { points, .. } => points.len() * 16,
                Self::Union(v) | Self::Intersection(v) => v.iter().map(Self::bytes).sum(),
            }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Spread {
    Pad,
    Repeat,
    Reflect,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum Kind {
    Linear {
        start: Point,
        end: Point,
    },
    Radial {
        center: Point,
        focus: Point,
        radius: f64,
        focal_radius: f64,
    },
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Stop {
    pub offset: f64,
    pub color: [u8; 4],
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Gradient {
    pub frame: Affine,
    pub kind: Kind,
    pub spread: Spread,
    pub stops: Vec<Stop>,
}
impl Gradient {
    fn valid(&self) -> bool {
        self.frame.valid()
            && (1..=256).contains(&self.stops.len())
            && self
                .stops
                .iter()
                .all(|s| s.offset.is_finite() && (0. ..=1.).contains(&s.offset))
            && self.stops.windows(2).all(|w| w[0].offset <= w[1].offset)
            && match self.kind {
                Kind::Linear { start, end } => {
                    [start, end].into_iter().all(crate::line_tools::point_valid)
                }
                Kind::Radial {
                    center,
                    focus,
                    radius,
                    focal_radius,
                } => {
                    [center, focus]
                        .into_iter()
                        .all(crate::line_tools::point_valid)
                        && radius.is_finite()
                        && (1e-9..=1e9).contains(&radius)
                        && focal_radius.is_finite()
                        && (0. ..=radius).contains(&focal_radius)
                }
            }
    }
    pub fn color(&self, p: Point) -> [u8; 4] {
        let p = self.frame.map(p);
        let t = match self.kind {
            Kind::Linear { start, end } => {
                let d = [end[0] - start[0], end[1] - start[1]];
                let length = d[0] * d[0] + d[1] * d[1];
                if length < 1e-20 {
                    1.
                } else {
                    ((p[0] - start[0]) * d[0] + (p[1] - start[1]) * d[1]) / length
                }
            }
            Kind::Radial {
                center,
                focus,
                radius,
                focal_radius,
            } => {
                let d = [center[0] - focus[0], center[1] - focus[1]];
                let q = [p[0] - focus[0], p[1] - focus[1]];
                let r = radius - focal_radius;
                let a = d[0] * d[0] + d[1] * d[1] - r * r;
                let b = -2. * (q[0] * d[0] + q[1] * d[1] + focal_radius * r);
                let c = q[0] * q[0] + q[1] * q[1] - focal_radius * focal_radius;
                if a.abs() < 1e-20 {
                    if b.abs() < 1e-20 { 0. } else { -c / b }
                } else {
                    let disc = (b * b - 4. * a * c).max(0.).sqrt();
                    let x = (-b - disc) / (2. * a);
                    let y = (-b + disc) / (2. * a);
                    if x >= 0. && y >= 0. {
                        x.max(y)
                    } else {
                        x.max(y).max(0.)
                    }
                }
            }
        };
        let t = match self.spread {
            Spread::Pad => t.clamp(0., 1.),
            Spread::Repeat => t.rem_euclid(1.),
            Spread::Reflect => {
                let v = t.rem_euclid(2.);
                if v > 1. { 2. - v } else { v }
            }
        };
        let n = self.stops.partition_point(|s| s.offset <= t);
        if n == 0 {
            return self.stops[0].color;
        }
        if n == self.stops.len() {
            return self.stops[n - 1].color;
        }
        let a = &self.stops[n - 1];
        let b = &self.stops[n];
        let f = (t - a.offset) / (b.offset - a.offset);
        let alpha = a.color[3] as f64 * (1. - f) + b.color[3] as f64 * f;
        let mut out = [0; 4];
        out[3] = alpha.round() as u8;
        if alpha > 0. {
            for (i, channel) in out.iter_mut().enumerate().take(3) {
                *channel = (a.color[i] as f64 * (1. - f) + b.color[i] as f64 * f)
                    .round()
                    .clamp(0., 255.) as u8;
            }
        }
        out
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Style {
    pub rule: Rule,
    pub gradient: Option<Gradient>,
    pub clip: Option<Clip>,
}
impl Style {
    pub fn valid(&self) -> bool {
        self.gradient.as_ref().is_none_or(Gradient::valid)
            && self.clip.as_ref().is_none_or(|c| c.valid_at(0, &mut 0))
    }
    pub fn bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + self
                .gradient
                .as_ref()
                .map_or(0, |g| g.stops.len() * std::mem::size_of::<Stop>())
            + self.clip.as_ref().map_or(0, Clip::bytes)
    }
    pub fn transform(&mut self, matrix: Affine) {
        if let Some(g) = &mut self.gradient {
            g.frame = matrix.inverse().then(g.frame);
        }
        if let Some(c) = &mut self.clip {
            c.transform(matrix);
        }
    }
}
pub fn requires_v68(scene: &crate::scene::Scene) -> bool {
    scene
        .lines
        .layers
        .iter()
        .flat_map(|l| &l.paths)
        .any(|p| p.svg.is_some())
}
