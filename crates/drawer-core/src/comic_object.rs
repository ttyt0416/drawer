//! Independent, parameter-owned comic objects. Coordinates are local to each affine frame.
use crate::{
    layer_search::Key,
    scene::{Affine, Point, Rect, Scene},
};
use serde::{Deserialize, Serialize};
#[path = "comic_lines.rs"]
mod line_grid;
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum BalloonShape {
    #[default]
    Ellipse,
    Rectangle,
    Cloud,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Tail {
    #[default]
    Speech,
    Thought,
    None,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Balloon {
    pub rect: Rect,
    pub shape: BalloonShape,
    pub tail: Tail,
    pub tip: Point,
    pub attachment: f64,
    pub tail_width: f64,
    pub fill: [u8; 4],
    pub ink: [u8; 4],
    pub width: f64,
    pub text: Option<u64>,
    pub padding: f64,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Frame {
    pub polygon: Vec<Point>,
    pub gap: f64,
    pub width: f64,
    pub ink: [u8; 4],
    pub children: Vec<Key>,
    pub family: u64,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Dot {
    #[default]
    Circle,
    Square,
    Lozenge,
    Line,
    Cross,
    Ellipse,
    Noise,
}
impl Dot {
    pub const ALL: [Self; 7] = [
        Self::Circle,
        Self::Square,
        Self::Lozenge,
        Self::Line,
        Self::Cross,
        Self::Ellipse,
        Self::Noise,
    ];
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tone {
    pub rect: Rect,
    pub dot: Dot,
    pub lpi: f64,
    pub density: f64,
    pub end_density: f64,
    pub gradient: bool,
    pub angle: f64,
    pub phase: Point,
    pub color: [u8; 4],
    pub monochrome: bool,
    pub seed: u32,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Lines {
    #[default]
    Parallel,
    Radial,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Effect {
    pub rect: Rect,
    pub kind: Lines,
    pub center: Point,
    pub angle: f64,
    pub inner: f64,
    pub count: u32,
    pub min_width: f64,
    pub max_width: f64,
    pub seed: u32,
    pub ink: [u8; 4],
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Kind {
    Balloon(Balloon),
    Frame(Frame),
    Tone(Tone),
    Effect(Effect),
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Object {
    pub name: String,
    pub transform: Affine,
    pub kind: Kind,
}
fn rect(r: Rect) -> bool {
    r.valid()
        && (0..2).all(|n| r.max[n] > r.min[n])
        && r.min.iter().chain(&r.max).all(|v| v.abs() <= 1e6)
}
fn point(p: Point) -> bool {
    p.iter().all(|v| v.is_finite() && v.abs() <= 1e6)
}
pub fn polygon_bounds(points: &[Point]) -> Rect {
    let mut r = Rect {
        min: [f64::INFINITY; 2],
        max: [f64::NEG_INFINITY; 2],
    };
    for p in points {
        for (n, v) in p.iter().enumerate() {
            r.min[n] = r.min[n].min(*v);
            r.max[n] = r.max[n].max(*v);
        }
    }
    r
}
pub fn distance(p: Point, a: Point, b: Point) -> f64 {
    let v = [b[0] - a[0], b[1] - a[1]];
    let t = if v[0] * v[0] + v[1] * v[1] <= 1e-20 {
        0.
    } else {
        ((p[0] - a[0]) * v[0] + (p[1] - a[1]) * v[1]) / (v[0] * v[0] + v[1] * v[1])
    }
    .clamp(0., 1.);
    (p[0] - a[0] - t * v[0]).hypot(p[1] - a[1] - t * v[1])
}
pub fn edge_distance(poly: &[Point], p: Point) -> f64 {
    poly.iter()
        .copied()
        .zip(poly.iter().copied().cycle().skip(1))
        .take(poly.len())
        .map(|(a, b)| distance(p, a, b))
        .fold(f64::INFINITY, f64::min)
}
pub fn contains(poly: &[Point], p: Point) -> bool {
    crate::render::inside(poly, p) || edge_distance(poly, p) < 1e-8
}
pub fn area(poly: &[Point]) -> f64 {
    poly.iter()
        .zip(poly.iter().cycle().skip(1))
        .take(poly.len())
        .map(|(a, b)| a[0] * b[1] - b[0] * a[1])
        .sum::<f64>()
        / 2.
}
impl Object {
    pub fn valid(&self) -> bool {
        !self.name.trim().is_empty()
            && self.name.chars().count() <= 64
            && !self.name.chars().any(char::is_control)
            && self.transform.valid()
            && match &self.kind {
                Kind::Balloon(b) => {
                    rect(b.rect)
                        && point(b.tip)
                        && (-360. ..=360.).contains(&b.attachment)
                        && (0. ..=1e4).contains(&b.tail_width)
                        && (0. ..=128.).contains(&b.width)
                        && (0. ..=4096.).contains(&b.padding)
                        && b.text.is_none_or(|id| id > 0)
                }
                Kind::Frame(f) => {
                    crate::comic_split::simple(&f.polygon)
                        && f.polygon.len() >= 3
                        && f.polygon.len() <= 128
                        && f.polygon.iter().copied().all(point)
                        && area(&f.polygon).abs() > 1e-6
                        && (0. ..=4096.).contains(&f.gap)
                        && (0. ..=128.).contains(&f.width)
                        && f.children.len() <= 256
                        && f.children
                            .iter()
                            .enumerate()
                            .all(|(i, k)| !f.children[..i].contains(k))
                }
                Kind::Tone(t) => {
                    rect(t.rect)
                        && (1. ..=4000.).contains(&t.lpi)
                        && (0. ..=1.).contains(&t.density)
                        && (0. ..=1.).contains(&t.end_density)
                        && (-360. ..=360.).contains(&t.angle)
                        && point(t.phase)
                }
                Kind::Effect(e) => {
                    rect(e.rect)
                        && point(e.center)
                        && (-360. ..=360.).contains(&e.angle)
                        && (0. ..=1e6).contains(&e.inner)
                        && (1..=2048).contains(&e.count)
                        && (0.1..=128.).contains(&e.min_width)
                        && (e.min_width..=128.).contains(&e.max_width)
                }
            }
    }
    pub fn local_bounds(&self) -> Rect {
        match &self.kind {
            Kind::Balloon(b) => {
                let mut r = b.rect;
                if b.tail != Tail::None {
                    for (n, v) in b.tip.iter().enumerate() {
                        r.min[n] = r.min[n].min(*v);
                        r.max[n] = r.max[n].max(*v);
                    }
                }
                r
            }
            Kind::Frame(f) => polygon_bounds(&f.polygon),
            Kind::Tone(t) => t.rect,
            Kind::Effect(e) => e.rect,
        }
    }
    pub fn bytes(&self) -> usize {
        self.name.len()
            + 128
            + match &self.kind {
                Kind::Frame(f) => f.polygon.len() * 16 + f.children.len() * 32,
                _ => 256,
            }
    }
    pub fn bounds(&self) -> Rect {
        let mut r = self.local_bounds();
        let padding = match &self.kind {
            Kind::Balloon(b) => {
                b.width / 2.
                    + if b.shape == BalloonShape::Cloud {
                        (b.rect.max[0] - b.rect.min[0]).max(b.rect.max[1] - b.rect.min[1]) * 0.04
                    } else {
                        0.
                    }
            }
            Kind::Frame(f) => f.width / 2.,
            _ => 0.,
        };
        for n in 0..2 {
            r.min[n] -= padding;
            r.max[n] += padding;
        }

        polygon_bounds(
            &[r.min, [r.max[0], r.min[1]], r.max, [r.min[0], r.max[1]]]
                .map(|p| self.transform.map(p)),
        )
    }
    pub fn resolved(&self, scene: &Scene) -> Self {
        let mut out = self.clone();
        if let Kind::Balloon(b) = &mut out.kind
            && let Some(id) = b.text
            && let Some(n) = scene.catalog.objects.index(id, scene.objects.len())
            && let Some(s) = crate::text_layout::Settings::from_object(&scene.objects[n])
            && let Ok(layout) = crate::text_layout::layout(s.baseline, &s.text, s.size, &s.style)
        {
            let points: Vec<_> = layout
                .paths
                .iter()
                .flatten()
                .map(|p| self.transform.inverse().map(*p))
                .collect();
            if !points.is_empty() {
                let r = polygon_bounds(&points);
                for n in 0..2 {
                    b.rect.min[n] = b.rect.min[n].min(r.min[n] - b.padding);
                    b.rect.max[n] = b.rect.max[n].max(r.max[n] + b.padding);
                }
            }
        }
        out
    }
}
fn hash(mut n: u32) -> u32 {
    n ^= n >> 16;
    n = n.wrapping_mul(0x7feb352d);
    n ^= n >> 15;
    n = n.wrapping_mul(0x846ca68b);
    n ^ (n >> 16)
}
fn random(seed: u32, n: u32) -> f64 {
    hash(seed ^ n.wrapping_mul(0x9e3779b9)) as f64 / u32::MAX as f64
}
fn score(dot: Dot, x: f64, y: f64) -> f64 {
    match dot {
        Dot::Circle => x * x + y * y,
        Dot::Square => x.abs().max(y.abs()),
        Dot::Lozenge => x.abs() + y.abs(),
        Dot::Line => y.abs(),
        Dot::Cross => x.abs().min(y.abs()),
        Dot::Ellipse => (x * 0.7).powi(2) + (y * 1.3).powi(2),
        Dot::Noise => 0.,
    }
}
static SCORES: std::sync::LazyLock<Vec<Vec<f64>>> = std::sync::LazyLock::new(|| {
    Dot::ALL
        .iter()
        .map(|s| {
            let mut v: Vec<_> = (0..4096)
                .map(|i| {
                    score(
                        *s,
                        (i % 64) as f64 / 64. - 0.5 + 1. / 128.,
                        (i / 64) as f64 / 64. - 0.5 + 1. / 128.,
                    )
                })
                .collect();
            v.sort_by(f64::total_cmp);
            v
        })
        .collect()
});
fn tone(t: &Tone, p: Point, full: Affine, dpi: f64) -> [u8; 4] {
    if !t.rect.contains(p) {
        return [0; 4];
    }
    if !t.monochrome {
        return t.color;
    }
    let mut density = t.density;
    if t.gradient {
        density += (t.end_density - t.density)
            * ((p[1] - t.rect.min[1]) / (t.rect.max[1] - t.rect.min[1])).clamp(0., 1.);
    }
    density *= 1.
        - (0.2126 * t.color[0] as f64 + 0.7152 * t.color[1] as f64 + 0.0722 * t.color[2] as f64)
            / 255.;
    if density <= 0. {
        return [0; 4];
    }
    if density >= 1. {
        return [0, 0, 0, t.color[3]];
    }
    let q = full.map(p);
    let origin = full.map([0.; 2]);
    let v = full.vector([1., 0.]);
    let angle = t.angle.to_radians() + v[1].atan2(v[0]);
    let (s, c) = angle.sin_cos();
    let phase = full.vector(t.phase);
    let x = q[0] - origin[0] - phase[0];
    let y = q[1] - origin[1] - phase[1];
    let period = dpi / t.lpi;
    let (x, y) = (
        ((c * x + s * y) / period).rem_euclid(1.) - 0.5,
        ((-s * x + c * y) / period).rem_euclid(1.) - 0.5,
    );
    let ink = if t.dot == Dot::Noise {
        random(
            t.seed,
            ((x + 0.5) * 64.).floor() as u32 + 64 * ((y + 0.5) * 64.).floor() as u32,
        ) < density
    } else {
        score(t.dot, x, y) <= SCORES[t.dot as usize][((density * 4096.) as usize).min(4095)]
    };
    if ink { [0, 0, 0, t.color[3]] } else { [0; 4] }
}
fn bubble_inside(b: &Balloon, p: Point) -> bool {
    let center = [
        (b.rect.min[0] + b.rect.max[0]) / 2.,
        (b.rect.min[1] + b.rect.max[1]) / 2.,
    ];
    let half = [
        (b.rect.max[0] - b.rect.min[0]) / 2.,
        (b.rect.max[1] - b.rect.min[1]) / 2.,
    ];
    let q = [(p[0] - center[0]) / half[0], (p[1] - center[1]) / half[1]];
    let base = match b.shape {
        BalloonShape::Rectangle => q[0].abs() <= 1. && q[1].abs() <= 1.,
        BalloonShape::Ellipse => q[0] * q[0] + q[1] * q[1] <= 1.,
        BalloonShape::Cloud => q[0].hypot(q[1]) <= 1. + 0.08 * (q[1].atan2(q[0]) * 10.).cos(),
    };
    if base {
        return true;
    }
    let a = b.attachment.to_radians();
    let attach = [center[0] + a.cos() * half[0], center[1] + a.sin() * half[1]];
    match b.tail {
        Tail::None => false,
        Tail::Speech => {
            let w = b.tail_width / 2.;
            contains(
                &[
                    [attach[0] - a.sin() * w, attach[1] + a.cos() * w],
                    [attach[0] + a.sin() * w, attach[1] - a.cos() * w],
                    b.tip,
                ],
                p,
            )
        }
        Tail::Thought => (1..=3).any(|i| {
            let t = i as f64 / 4.;
            let c = [
                attach[0] + (b.tip[0] - attach[0]) * t,
                attach[1] + (b.tip[1] - attach[1]) * t,
            ];
            (p[0] - c[0]).hypot(p[1] - c[1]) <= b.tail_width * (1. - t) / 2.
        }),
    }
}
#[derive(Clone)]
pub struct Geometry {
    object: Object,
    lines: Vec<(Point, Point, f64)>,
    grid: Option<line_grid::Grid>,
}
impl Geometry {
    pub fn new(object: &Object, scene: &Scene) -> Self {
        if !object.valid() {
            return Self {
                object: object.clone(),
                lines: vec![],
                grid: None,
            };
        }
        let object = object.resolved(scene);
        let mut lines = vec![];
        if let Kind::Effect(e) = &object.kind {
            let center = e.center;
            let radius = (e.rect.max[0] - e.rect.min[0]).hypot(e.rect.max[1] - e.rect.min[1]) * 2.;
            let (s, c) = e.angle.to_radians().sin_cos();
            for i in 0..e.count {
                let width = e.min_width + (e.max_width - e.min_width) * random(e.seed, i * 3);
                let (a, b) = if e.kind == Lines::Radial {
                    let angle = (i as f64 + random(e.seed, i * 3 + 1) * 0.8)
                        * std::f64::consts::TAU
                        / e.count as f64
                        + e.angle.to_radians();
                    let v = [angle.cos(), angle.sin()];
                    (
                        [center[0] + v[0] * e.inner, center[1] + v[1] * e.inner],
                        [center[0] + v[0] * radius, center[1] + v[1] * radius],
                    )
                } else {
                    let offset = (i as f64 + random(e.seed, i * 3 + 1)) * radius / e.count as f64
                        - radius / 2.;
                    (
                        [
                            center[0] - c * radius - s * offset,
                            center[1] - s * radius + c * offset,
                        ],
                        [
                            center[0] + c * radius - s * offset,
                            center[1] + s * radius + c * offset,
                        ],
                    )
                };
                lines.push((a, b, width));
            }
        }
        let grid = if let Kind::Effect(e) = &object.kind {
            Some(line_grid::Grid::new(e.rect, &lines))
        } else {
            None
        };
        Self {
            object,
            lines,
            grid,
        }
    }
    pub fn pixel(&self, original: Point, scene_transform: Affine, dpi: f64) -> [u8; 4] {
        let p = self.object.transform.inverse().map(original);
        match &self.object.kind {
            Kind::Balloon(b) => {
                let inside = bubble_inside(b, p);
                let radius = b.width / 2.;
                if radius > 0.
                    && (0..8).any(|i| {
                        let a = i as f64 * std::f64::consts::TAU / 8.;
                        bubble_inside(b, [p[0] + radius * a.cos(), p[1] + radius * a.sin()])
                            != inside
                    })
                {
                    b.ink
                } else if inside {
                    b.fill
                } else {
                    [0; 4]
                }
            }
            Kind::Frame(f) => {
                if f.width > 0. && edge_distance(&f.polygon, p) <= f.width / 2. {
                    f.ink
                } else {
                    [0; 4]
                }
            }
            Kind::Tone(t) => tone(t, p, self.object.transform.then(scene_transform), dpi),
            Kind::Effect(e) => {
                if e.rect.contains(p)
                    && self.grid.as_ref().is_some_and(|g| {
                        g.candidates(p).iter().any(|n| {
                            let (a, b, w) = self.lines[*n];
                            distance(p, a, b) <= w / 2.
                        })
                    })
                {
                    e.ink
                } else {
                    [0; 4]
                }
            }
        }
    }
}
