//! Bounded independent ruler families and editable vanishing-point relationships.
use crate::scene::{Affine, Point};
use serde::{Deserialize, Serialize};
fn point(p: Point) -> bool {
    p.iter().all(|v| v.is_finite() && v.abs() <= 1e6)
}
fn sub(a: Point, b: Point) -> Point {
    [a[0] - b[0], a[1] - b[1]]
}
fn add(a: Point, b: Point) -> Point {
    [a[0] + b[0], a[1] + b[1]]
}
fn mul(a: Point, t: f64) -> Point {
    a.map(|v| v * t)
}
fn dot(a: Point, b: Point) -> f64 {
    a[0] * b[0] + a[1] * b[1]
}
fn length(a: Point) -> f64 {
    a[0].hypot(a[1])
}
fn unit(a: Point) -> Option<Point> {
    let r = length(a);
    (r > 1e-9 && r.is_finite()).then(|| mul(a, 1.0 / r))
}
fn line(a: Point, d: Point, p: Point) -> Point {
    add(a, mul(d, dot(sub(p, a), d)))
}
fn rotate(p: Point, c: Point, a: f64) -> Point {
    let (s, cos) = a.sin_cos();
    let q = sub(p, c);
    add(c, [q[0] * cos - q[1] * s, q[0] * s + q[1] * cos])
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Vanishing {
    pub id: u64,
    pub point: Point,
    pub fixed: bool,
    pub infinite: bool,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LinkedGuide {
    pub id: u64,
    pub vanishing: u64,
    pub anchor: Point,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Perspective {
    pub next_id: u64,
    pub center: Point,
    pub radius: f64,
    pub horizon: f64,
    pub horizon_fixed: bool,
    pub fisheye: bool,
    pub points: Vec<Vanishing>,
    pub guides: Vec<LinkedGuide>,
}
impl Default for Perspective {
    fn default() -> Self {
        Self {
            next_id: 3,
            center: [32.0, 32.0],
            radius: 32.0,
            horizon: 32.0,
            horizon_fixed: false,
            fisheye: false,
            points: vec![
                Vanishing {
                    id: 1,
                    point: [0.0, 32.0],
                    fixed: false,
                    infinite: false,
                },
                Vanishing {
                    id: 2,
                    point: [64.0, 32.0],
                    fixed: false,
                    infinite: false,
                },
            ],
            guides: vec![],
        }
    }
}
impl Perspective {
    pub fn valid(&self) -> bool {
        (!self.fisheye || self.points.len() <= 2 && self.points.iter().all(|p| !p.infinite))
            && self.next_id > 0
            && point(self.center)
            && self.radius.is_finite()
            && (0.25..=1e6).contains(&self.radius)
            && self.horizon.is_finite()
            && self.horizon.abs() <= 1e6
            && (1..=16).contains(&self.points.len())
            && self.guides.len() <= 64
            && self.points.iter().all(|v| {
                v.id > 0
                    && v.id < self.next_id
                    && point(v.point)
                    && (!v.infinite || unit(v.point).is_some())
                    && (!self.horizon_fixed
                        || v.infinite
                        || (v.point[1] - self.horizon).abs() < 1e-6)
            })
            && self
                .points
                .iter()
                .enumerate()
                .all(|(i, p)| self.points[..i].iter().all(|v| v.id != p.id))
            && self.guides.iter().all(|g| {
                g.id > 0
                    && g.id < self.next_id
                    && point(g.anchor)
                    && self.points.iter().any(|p| {
                        p.id == g.vanishing && (p.infinite || length(sub(p.point, g.anchor)) > 1e-6)
                    })
            })
            && self.guides.iter().enumerate().all(|(i, g)| {
                self.guides[..i].iter().all(|p| p.id != g.id)
                    && self.points.iter().all(|p| p.id != g.id)
            })
    }
    fn edit(
        &mut self,
        operation: impl FnOnce(&mut Self) -> Result<(), String>,
    ) -> Result<(), String> {
        let mut next = self.clone();
        operation(&mut next)?;
        if !next.valid() {
            return Err("Invalid perspective fields/linked guides or capacity limit".into());
        }
        *self = next;
        Ok(())
    }
    fn id(&mut self) -> Result<u64, String> {
        let id = self.next_id;
        self.next_id = self
            .next_id
            .checked_add(1)
            .ok_or("Vanishing point identity limit")?;
        Ok(id)
    }
    pub fn add_point(&mut self, mut p: Point) -> Result<(), String> {
        self.edit(|s| {
            if s.horizon_fixed {
                p[1] = s.horizon;
            }
            let id = s.id()?;
            s.points.push(Vanishing {
                id,
                point: p,
                fixed: false,
                infinite: false,
            });
            Ok(())
        })
    }
    pub fn remove_point(&mut self, id: u64) -> Result<(), String> {
        self.edit(|s| {
            if !s.points.iter().any(|p| p.id == id) {
                return Err("Vanishing point is missing".into());
            }
            s.points.retain(|p| p.id != id);
            s.guides.retain(|g| g.vanishing != id);
            Ok(())
        })
    }
    pub fn move_point(&mut self, id: u64, mut point: Point) -> Result<(), String> {
        self.edit(|s| {
            let p = s
                .points
                .iter_mut()
                .find(|p| p.id == id)
                .ok_or("Vanishing point is missing")?;
            if p.fixed {
                return Err("Unlock this vanishing point before moving it".into());
            }
            if s.horizon_fixed && !p.infinite {
                point[1] = s.horizon;
            }
            p.point = point;
            Ok(())
        })
    }
    pub fn infinitize(&mut self, id: u64, infinite: bool) -> Result<(), String> {
        if self.fisheye {
            return Err("Use linear perspective to infinitize a point".into());
        }
        self.edit(|s| {
            let p = s
                .points
                .iter_mut()
                .find(|p| p.id == id)
                .ok_or("Vanishing point is missing")?;
            if p.fixed {
                return Err("Unlock this vanishing point before changing its direction".into());
            }
            if p.infinite == infinite {
                return Ok(());
            }
            if infinite {
                let mut d = sub(p.point, s.center);
                if s.horizon_fixed {
                    d[1] = 0.0;
                }
                p.point =
                    unit(d).ok_or("Move the vanishing point away from the projection center")?;
            } else {
                p.point = add(s.center, mul(p.point, s.radius * 2.0));
                if s.horizon_fixed {
                    p.point[1] = s.horizon;
                }
            }
            p.infinite = infinite;
            Ok(())
        })
    }
    pub fn lock_horizon(&mut self, locked: bool) -> Result<(), String> {
        self.edit(|s| {
            if locked
                && s.points
                    .iter()
                    .any(|p| p.fixed && !p.infinite && (p.point[1] - s.horizon).abs() >= 1e-6)
            {
                return Err("A fixed point lies away from this eye height".into());
            }
            s.horizon_fixed = locked;
            if locked {
                for p in &mut s.points {
                    if !p.infinite {
                        p.point[1] = s.horizon;
                    }
                }
            }
            Ok(())
        })
    }
    pub fn add_guide(&mut self, vanishing: u64, anchor: Point) -> Result<(), String> {
        self.edit(|s| {
            let id = s.id()?;
            s.guides.push(LinkedGuide {
                id,
                vanishing,
                anchor,
            });
            Ok(())
        })
    }
    pub fn remove_guide(&mut self, id: u64) {
        self.guides.retain(|g| g.id != id);
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Marks {
    Equal(u16),
    Golden(u8),
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Spec {
    ParallelLine {
        a: Point,
        b: Point,
    },
    ParallelCurve {
        points: Vec<Point>,
    },
    MultipleCurve {
        curves: Vec<Vec<Point>>,
    },
    RadialLine {
        center: Point,
        guide: Point,
    },
    RadialCurve {
        center: Point,
        points: Vec<Point>,
    },
    ConcentricCircle {
        center: Point,
        radius: f64,
    },
    Guide {
        a: Point,
        b: Point,
    },
    Division {
        a: Point,
        b: Point,
        marks: Marks,
        snap_ticks: bool,
    },
    Perspective(Perspective),
}
impl Spec {
    pub fn label(&self) -> &'static str {
        match self {
            Self::ParallelLine { .. } => "Parallel line",
            Self::ParallelCurve { .. } => "Parallel curve",
            Self::MultipleCurve { .. } => "Multiple curve",
            Self::RadialLine { .. } => "Radial line",
            Self::RadialCurve { .. } => "Radial curve",
            Self::ConcentricCircle { .. } => "Concentric circle",
            Self::Guide { .. } => "Guide",
            Self::Division {
                marks: Marks::Equal(_),
                ..
            } => "Equal division",
            Self::Division {
                marks: Marks::Golden(_),
                ..
            } => "Golden ratio",
            Self::Perspective(p) => {
                if p.fisheye {
                    "Fisheye perspective"
                } else {
                    "Editable perspective"
                }
            }
        }
    }
    pub fn valid(&self) -> bool {
        let curve = |p: &Vec<Point>| {
            (2..=32).contains(&p.len())
                && p.iter().all(|p| point(*p))
                && p.windows(2).any(|p| length(sub(p[0], p[1])) > 1e-6)
        };
        match self {
            Self::ParallelLine { a, b } | Self::Guide { a, b } => {
                point(*a) && point(*b) && length(sub(*a, *b)) > 1e-6
            }
            Self::ParallelCurve { points } => curve(points),
            Self::MultipleCurve { curves } => {
                (1..=8).contains(&curves.len()) && curves.iter().all(curve)
            }
            Self::RadialLine { center, guide } => {
                point(*center) && point(*guide) && length(sub(*center, *guide)) > 1e-6
            }
            Self::RadialCurve { center, points } => point(*center) && curve(points),
            Self::ConcentricCircle { center, radius } => {
                point(*center) && radius.is_finite() && (0.25..=1e6).contains(radius)
            }
            Self::Division { a, b, marks, .. } => {
                point(*a)
                    && point(*b)
                    && length(sub(*a, *b)) > 1e-6
                    && match marks {
                        Marks::Equal(n) => (1..=512).contains(n),
                        Marks::Golden(n) => (1..=32).contains(n),
                    }
            }
            Self::Perspective(p) => p.valid(),
        }
    }
    pub fn bytes(&self) -> usize {
        match self {
            Self::ParallelCurve { points } | Self::RadialCurve { points, .. } => points.len() * 16,
            Self::MultipleCurve { curves } => curves.iter().map(|p| p.len() * 16 + 24).sum(),
            Self::Perspective(p) => p.points.len() * 48 + p.guides.len() * 40,
            _ => 128,
        }
    }
    pub fn tick_points(&self) -> Vec<Point> {
        let Self::Division { a, b, marks, .. } = self else {
            return vec![];
        };
        let fractions = match marks {
            Marks::Equal(n) => (0..=*n).map(|i| i as f64 / *n as f64).collect(),
            Marks::Golden(n) => {
                let r = (5.0f64.sqrt() - 1.0) / 2.0;
                let mut p = vec![0.0, 1.0];
                for i in 1..=*n {
                    p.push(r.powi(i as i32));
                    p.push(1.0 - r.powi(i as i32));
                }
                p.sort_by(f64::total_cmp);
                p.dedup_by(|a, b| (*a - *b).abs() < 1e-12);
                p
            }
        };
        fractions
            .into_iter()
            .map(|t| add(*a, mul(sub(*b, *a), t)))
            .collect()
    }
    fn paths(&self) -> Result<Vec<Vec<Point>>, String> {
        Ok(match self {
            Self::ParallelCurve { points } | Self::RadialCurve { points, .. } => {
                vec![curve(points)?]
            }
            Self::MultipleCurve { curves } => {
                curves.iter().map(|p| curve(p)).collect::<Result<_, _>>()?
            }
            _ => vec![],
        })
    }
    pub fn contour(&self) -> Result<Option<Vec<Point>>, String> {
        if !self.valid() {
            return Err("Invalid ruler contour".into());
        }
        match self {
            Self::ConcentricCircle { center, radius } => Ok(Some(circle(*center, *radius, 96))),
            Self::ParallelCurve { points } | Self::RadialCurve { points, .. }
                if length(sub(points[0], *points.last().unwrap())) < 1e-6 =>
            {
                Ok(Some(curve(points)?))
            }
            _ => Ok(None),
        }
    }
    pub fn follow(&self) -> Result<(Vec<Point>, bool), String> {
        self.follow_part(0)
    }
    pub fn follow_part(&self, part: usize) -> Result<(Vec<Point>, bool), String> {
        if !self.valid() {
            return Err("Invalid ruler geometry".into());
        }
        let result = match self {
            Self::ParallelLine { a, b } | Self::Guide { a, b } | Self::Division { a, b, .. } => {
                (vec![*a, *b], false)
            }
            Self::ParallelCurve { points } | Self::RadialCurve { points, .. } => (
                curve(points)?,
                length(sub(points[0], *points.last().unwrap())) < 1e-6,
            ),
            Self::MultipleCurve { curves } => (
                curve(curves.get(part).ok_or("Choose an existing curve")?)?,
                false,
            ),
            Self::RadialLine { center, guide } => (vec![*center, *guide], false),
            Self::ConcentricCircle { center, radius } => (circle(*center, *radius, 96), true),
            Self::Perspective(p) => {
                let g = p
                    .guides
                    .get(part)
                    .ok_or("Add/select a perspective guide to follow first")?;
                let v = p.points.iter().find(|p| p.id == g.vanishing).unwrap();
                if p.fisheye {
                    let other = p
                        .points
                        .iter()
                        .find(|v| v.id != g.vanishing)
                        .map_or(sub(mul(p.center, 2.0), v.point), |v| v.point);
                    let Some((center, radius)) = through_circle(v.point, other, g.anchor) else {
                        return Ok((
                            lens_line(g.anchor, sub(other, v.point), p.center, p.radius)?,
                            false,
                        ));
                    };
                    let (arc, closed) = lens_arc(center, radius, p.center, p.radius)?;
                    return Ok((resample(&arc, 128), closed));
                }
                (
                    vec![
                        g.anchor,
                        if v.infinite {
                            add(g.anchor, mul(v.point, p.radius * 2.0))
                        } else {
                            v.point
                        },
                    ],
                    false,
                )
            }
        };
        Ok((resample(&result.0, 128), result.1))
    }
    pub fn lines(&self, span: f64) -> Result<Vec<[Point; 2]>, String> {
        if !self.valid() {
            return Err("Invalid ruler geometry".into());
        }
        let mut lines = vec![];
        let mut append = |points: &[Point], closed: bool| {
            lines.extend(points.windows(2).map(|p| [p[0], p[1]]));
            if closed {
                lines.push([*points.last().unwrap(), points[0]]);
            }
        };
        match self {
            Self::ParallelLine { a, b } | Self::Guide { a, b } => {
                let d = unit(sub(*b, *a)).unwrap();
                append(&[add(*a, mul(d, -span)), add(*a, mul(d, span))], false);
            }
            Self::ParallelCurve { .. } | Self::MultipleCurve { .. } => {
                for p in self.paths()? {
                    append(&p, false);
                }
            }
            Self::RadialCurve { center, .. } => {
                for n in 0..4 {
                    for p in self.paths()? {
                        let p = p
                            .into_iter()
                            .map(|p| rotate(p, *center, std::f64::consts::FRAC_PI_2 * n as f64))
                            .collect::<Vec<_>>();
                        append(&p, false);
                    }
                }
            }
            Self::RadialLine { center, .. } => {
                for n in 0..12 {
                    let (s, c) = (std::f64::consts::PI * n as f64 / 12.0).sin_cos();
                    append(
                        &[
                            add(*center, [-c * span, -s * span]),
                            add(*center, [c * span, s * span]),
                        ],
                        false,
                    );
                }
            }
            Self::ConcentricCircle { center, radius } => {
                for t in [0.5, 1.0, 1.5] {
                    append(&circle(*center, *radius * t, 96), true);
                }
            }
            Self::Division { a, b, .. } => {
                append(&[*a, *b], false);
                let d = unit(sub(*b, *a)).unwrap();
                let n = [-d[1], d[0]];
                for p in self.tick_points() {
                    append(&[add(p, mul(n, -4.0)), add(p, mul(n, 4.0))], false);
                }
            }
            Self::Perspective(p) => {
                if p.fisheye {
                    append(&circle(p.center, p.radius, 96), true);
                    let poles = p
                        .points
                        .iter()
                        .filter(|v| !v.infinite)
                        .map(|v| v.point)
                        .collect::<Vec<_>>();
                    let a = poles
                        .first()
                        .copied()
                        .unwrap_or(add(p.center, [-p.radius, 0.0]));
                    let b = poles.get(1).copied().unwrap_or(sub(mul(p.center, 2.0), a));
                    let anchors = if p.guides.is_empty() {
                        (-3..=3)
                            .filter(|n| *n != 0)
                            .map(|n| add(p.center, [0.0, p.radius * n as f64 / 4.0]))
                            .collect::<Vec<_>>()
                    } else {
                        p.guides.iter().map(|g| g.anchor).collect()
                    };
                    for anchor in anchors {
                        if let Some((center, radius)) = through_circle(a, b, anchor) {
                            if let Ok((arc, closed)) = lens_arc(center, radius, p.center, p.radius)
                            {
                                append(&arc, closed);
                            }
                        } else if let Ok(path) = lens_line(anchor, sub(b, a), p.center, p.radius) {
                            append(&path, false);
                        }
                    }
                } else {
                    append(
                        &[
                            [p.center[0] - span, p.horizon],
                            [p.center[0] + span, p.horizon],
                        ],
                        false,
                    );
                    for g in &p.guides {
                        let v = p.points.iter().find(|v| v.id == g.vanishing).unwrap();
                        let d = if v.infinite {
                            unit(v.point).unwrap()
                        } else {
                            unit(sub(v.point, g.anchor)).unwrap()
                        };
                        append(
                            &[add(g.anchor, mul(d, -span)), add(g.anchor, mul(d, span))],
                            false,
                        );
                    }
                    if p.guides.is_empty() {
                        for v in &p.points {
                            for n in -3..=3 {
                                let (s, c) = (n as f64 * std::f64::consts::PI / 12.0).sin_cos();
                                let origin = if v.infinite { p.center } else { v.point };
                                append(
                                    &[
                                        add(origin, [-c * span, -s * span]),
                                        add(origin, [c * span, s * span]),
                                    ],
                                    false,
                                );
                            }
                        }
                    }
                }
            }
        }
        Ok(lines)
    }
}
fn curve(p: &[Point]) -> Result<Vec<Point>, String> {
    Ok(crate::shapes::geometry(
        crate::shapes::Settings {
            kind: crate::shapes::Kind::Polyline,
            curve: crate::shapes::Curve::Spline,
            ..Default::default()
        },
        p,
    )?
    .points)
}
fn circle(c: Point, r: f64, n: usize) -> Vec<Point> {
    (0..n)
        .map(|i| {
            let (s, cos) = (std::f64::consts::TAU * i as f64 / n as f64).sin_cos();
            add(c, [r * cos, r * s])
        })
        .collect()
}
fn resample(p: &[Point], limit: usize) -> Vec<Point> {
    if p.len() <= limit {
        return p.to_vec();
    }
    let mut lengths = vec![0.0];
    for q in p.windows(2) {
        lengths.push(lengths.last().unwrap() + length(sub(q[1], q[0])));
    }
    let total = *lengths.last().unwrap();
    if total <= 1e-9 {
        return vec![p[0], *p.last().unwrap()];
    }
    (0..limit)
        .map(|i| {
            let t = total * i as f64 / (limit - 1) as f64;
            let k = lengths.partition_point(|v| *v < t).clamp(1, p.len() - 1);
            let d = lengths[k] - lengths[k - 1];
            let f = if d > 1e-9 {
                (t - lengths[k - 1]) / d
            } else {
                0.0
            };
            add(p[k - 1], mul(sub(p[k], p[k - 1]), f))
        })
        .collect()
}
#[derive(Clone)]
enum Mode {
    Parallel {
        a: Option<Point>,
        d: Point,
    },
    Fixed {
        a: Point,
        d: Point,
    },
    Radial {
        center: Point,
        d: Point,
        anchor: bool,
    },
    Curves {
        paths: Vec<Vec<Point>>,
        kind: u8,
        center: Point,
        selected: Option<(usize, f64, f64)>,
    },
    Ring {
        center: Point,
        radius: Option<f64>,
        last: Point,
    },
    Division {
        a: Point,
        d: Point,
        marks: Vec<Point>,
        snap: bool,
    },
    Perspective {
        data: Perspective,
        anchor: Option<Point>,
        direction: Option<Point>,
        circle: Option<(Point, f64)>,
    },
}
#[derive(Clone)]
pub struct Runtime {
    transform: Affine,
    inverse: Affine,
    mode: Mode,
    work: usize,
    failed: bool,
    distance: f64,
}
impl Runtime {
    pub fn new(spec: &Spec, transform: Affine) -> Result<Self, String> {
        if !spec.valid() || !transform.valid() {
            return Err("Invalid special ruler".into());
        }
        let mode = match spec {
            Spec::ParallelLine { a, b } => Mode::Parallel {
                a: None,
                d: unit(sub(*b, *a)).unwrap(),
            },
            Spec::Guide { a, b } => Mode::Fixed {
                a: *a,
                d: unit(sub(*b, *a)).unwrap(),
            },
            Spec::RadialLine { center, guide } => Mode::Radial {
                center: *center,
                d: unit(sub(*guide, *center)).unwrap(),
                anchor: false,
            },
            Spec::ParallelCurve { .. } | Spec::MultipleCurve { .. } | Spec::RadialCurve { .. } => {
                Mode::Curves {
                    paths: spec.paths()?,
                    kind: match spec {
                        Spec::ParallelCurve { .. } => 0,
                        Spec::RadialCurve { .. } => 2,
                        _ => 1,
                    },
                    center: if let Spec::RadialCurve { center, .. } = spec {
                        *center
                    } else {
                        [0.0; 2]
                    },
                    selected: None,
                }
            }
            Spec::ConcentricCircle { center, .. } => Mode::Ring {
                center: *center,
                radius: None,
                last: *center,
            },
            Spec::Division {
                a, b, snap_ticks, ..
            } => Mode::Division {
                a: *a,
                d: unit(sub(*b, *a)).unwrap(),
                marks: spec.tick_points(),
                snap: *snap_ticks,
            },
            Spec::Perspective(data) => Mode::Perspective {
                data: data.clone(),
                anchor: None,
                direction: None,
                circle: None,
            },
        };
        Ok(Self {
            transform,
            inverse: transform.inverse(),
            mode,
            work: 0,
            failed: false,
            distance: 8.0,
        })
    }
    pub fn bytes(&self) -> usize {
        match &self.mode {
            Mode::Curves { paths, .. } => paths.iter().map(|p| p.capacity() * 16 + 24).sum(),
            Mode::Division { marks, .. } => marks.capacity() * 16,
            Mode::Perspective { data, .. } => {
                data.points.capacity() * std::mem::size_of::<Vanishing>()
                    + data.guides.capacity() * std::mem::size_of::<LinkedGuide>()
            }
            _ => 0,
        }
    }
    pub fn set_distance(&mut self, d: f64) {
        self.distance = d;
    }
    pub fn failed(&self) -> bool {
        self.failed
    }
    pub fn project(&mut self, point: Point) -> Point {
        let p = self.inverse.map(point);
        if p.iter().any(|v| !v.is_finite() || v.abs() > 1e12) {
            self.failed = true;
            return point;
        }
        let result = match &mut self.mode {
            Mode::Parallel { a, d } => {
                let a = *a.get_or_insert(p);
                line(a, *d, p)
            }
            Mode::Fixed { a, d } => line(*a, *d, p),
            Mode::Radial { center, d, anchor } => {
                if !*anchor {
                    *d = unit(sub(p, *center)).unwrap_or(*d);
                    *anchor = true;
                }
                line(*center, *d, p)
            }
            Mode::Ring {
                center,
                radius,
                last,
            } => {
                let r = *radius.get_or_insert(length(sub(p, *center)));
                if let Some(d) = unit(sub(p, *center)) {
                    *last = add(*center, mul(d, r));
                }
                *last
            }
            Mode::Curves {
                paths,
                kind,
                center,
                selected,
            } => {
                if selected.is_none() {
                    let (index, q, d) = nearest(paths, p, &mut self.work);
                    let n = [-d[1], d[0]];
                    let offset = if *kind == 0 { dot(sub(p, q), n) } else { 0.0 };
                    let angle = if *kind == 2 {
                        let r = length(sub(p, *center));
                        let q = radius_point(&paths[index], *center, r);
                        sub(p, *center)[1].atan2(sub(p, *center)[0])
                            - sub(q, *center)[1].atan2(sub(q, *center)[0])
                    } else {
                        0.0
                    };
                    *selected = Some((index, offset, angle));
                }
                let (index, offset, angle) = selected.unwrap();
                let query = if *kind == 2 {
                    rotate(p, *center, -angle)
                } else {
                    p
                };
                let (_, q, d) = nearest(std::slice::from_ref(&paths[index]), query, &mut self.work);
                if *kind == 2 {
                    rotate(q, *center, angle)
                } else {
                    add(q, [-d[1] * offset, d[0] * offset])
                }
            }
            Mode::Division { a, d, marks, snap } => {
                self.work = self.work.saturating_add(marks.len());
                let mut q = line(*a, *d, p);
                if *snap {
                    let mut best = self.distance;
                    for mark in marks {
                        let distance = length(sub(point, self.transform.map(*mark)));
                        if distance < best {
                            best = distance;
                            q = *mark;
                        }
                    }
                }
                q
            }
            Mode::Perspective {
                data,
                anchor,
                direction,
                circle: arc,
            } => {
                if anchor.is_none() {
                    let start = if data.fisheye && length(sub(p, data.center)) > data.radius {
                        add(
                            data.center,
                            mul(unit(sub(p, data.center)).unwrap_or([1.0, 0.0]), data.radius),
                        )
                    } else {
                        p
                    };
                    *anchor = Some(start);
                    if data.fisheye {
                        let finite = data
                            .points
                            .iter()
                            .filter(|v| !v.infinite)
                            .map(|v| v.point)
                            .collect::<Vec<_>>();
                        let a = finite
                            .first()
                            .copied()
                            .unwrap_or(add(data.center, [-data.radius, 0.0]));
                        let b = finite
                            .get(1)
                            .copied()
                            .unwrap_or(sub(mul(data.center, 2.0), a));
                        *arc = through_circle(a, b, start);
                    }
                    start
                } else if data.fisheye {
                    if let Some((c, r)) = *arc {
                        let q = add(c, mul(unit(sub(p, c)).unwrap_or([1.0, 0.0]), r));
                        if length(sub(q, data.center)) <= data.radius + 1e-6 {
                            q
                        } else {
                            circle_intersection(c, r, data.center, data.radius, p)
                                .unwrap_or(*anchor.as_ref().unwrap())
                        }
                    } else {
                        let d = unit(sub(
                            data.points
                                .iter()
                                .find(|v| !v.infinite)
                                .map_or(data.center, |v| v.point),
                            *anchor.as_ref().unwrap(),
                        ))
                        .unwrap_or([1.0, 0.0]);
                        let a = *anchor.as_ref().unwrap();
                        let q = line(a, d, p);
                        let center = sub(a, data.center);
                        let b = dot(center, d);
                        let determinant = (b * b - dot(center, center) + data.radius * data.radius)
                            .max(0.0)
                            .sqrt();
                        let t = dot(sub(q, a), d).clamp(-b - determinant, -b + determinant);
                        add(a, mul(d, t))
                    }
                } else {
                    let a = anchor.unwrap();
                    if direction.is_none()
                        && let Some(motion) = unit(sub(p, a))
                    {
                        let mut best = -1.0;
                        for v in &data.points {
                            self.work = self.work.saturating_add(1);
                            if let Some(d) = if v.infinite {
                                unit(v.point)
                            } else {
                                unit(sub(v.point, a))
                            } {
                                let score = dot(motion, d).abs();
                                if score > best {
                                    best = score;
                                    *direction = Some(d);
                                }
                            }
                        }
                    }
                    direction.map_or(p, |d| line(a, d, p))
                }
            }
        };
        self.failed |= self.work > 64 * 1024 * 1024;
        let q = self.transform.map(result);
        if q.iter().any(|v| !v.is_finite() || v.abs() > 1e12) {
            self.failed = true;
            point
        } else {
            q
        }
    }
}
fn nearest(paths: &[Vec<Point>], p: Point, work: &mut usize) -> (usize, Point, Point) {
    let mut best = (f64::INFINITY, 0, p, [1.0, 0.0]);
    for (i, path) in paths.iter().enumerate() {
        for (j, pair) in path.windows(2).enumerate() {
            *work = work.saturating_add(1);
            let (d, q) = crate::line_tools::distance(p, pair[0], pair[1]);
            if d < best.0 {
                let mut tangent = unit(sub(pair[1], pair[0])).unwrap_or([1.0, 0.0]);
                let other = if length(sub(q, pair[0])) < 1e-8 && j > 0 {
                    unit(sub(path[j], path[j - 1]))
                } else if length(sub(q, pair[1])) < 1e-8 && j + 2 < path.len() {
                    unit(sub(path[j + 2], path[j + 1]))
                } else {
                    None
                };
                if let Some(other) = other {
                    tangent = unit(add(tangent, other)).unwrap_or(tangent);
                }
                best = (d, i, q, tangent);
            }
        }
    }
    (best.1, best.2, best.3)
}
fn radius_point(path: &[Point], center: Point, r: f64) -> Point {
    let mut best = (f64::INFINITY, path[0]);
    for pair in path.windows(2) {
        let a = sub(pair[0], center);
        let d = sub(pair[1], pair[0]);
        let aa = dot(d, d);
        let bb = 2.0 * dot(a, d);
        let cc = dot(a, a) - r * r;
        let determinant = bb * bb - 4.0 * aa * cc;
        for t in [0.0, 1.0] {
            let q = add(pair[0], mul(d, t));
            let error = (length(sub(q, center)) - r).abs();
            if error < best.0 {
                best = (error, q);
            }
        }
        if aa > 1e-12 && determinant >= 0.0 {
            for sign in [-1.0, 1.0] {
                let t = (-bb + sign * determinant.sqrt()) / (2.0 * aa);
                if (0.0..=1.0).contains(&t) {
                    return add(pair[0], mul(d, t));
                }
            }
        }
    }
    best.1
}
fn through_circle(a: Point, b: Point, p: Point) -> Option<(Point, f64)> {
    let b = sub(b, a);
    let p = sub(p, a);
    let determinant = 2.0 * (b[0] * p[1] - b[1] * p[0]);
    if determinant.abs() < 1e-6 {
        return None;
    }
    let bb = dot(b, b);
    let pp = dot(p, p);
    let center = add(
        a,
        [
            (bb * p[1] - pp * b[1]) / determinant,
            (b[0] * pp - p[0] * bb) / determinant,
        ],
    );
    let radius = length(sub(center, a));
    (radius.is_finite() && radius <= 1e9).then_some((center, radius))
}
fn lens_line(
    anchor: Point,
    direction: Point,
    lens: Point,
    radius: f64,
) -> Result<Vec<Point>, String> {
    let d = unit(direction).ok_or("Coincident fisheye poles")?;
    let mid = line(anchor, d, lens);
    let h2 = radius * radius - dot(sub(mid, lens), sub(mid, lens));
    if h2 < 0.0 {
        return Err("Fisheye guide does not cross the lens".into());
    }
    let h = h2.sqrt();
    Ok(vec![add(mid, mul(d, -h)), add(mid, mul(d, h))])
}
fn lens_intersections(c: Point, r: f64, lens: Point, radius: f64) -> Option<[Point; 2]> {
    let v = sub(c, lens);
    let d = length(v);
    if d < 1e-9 || d > r + radius || d < (r - radius).abs() {
        return None;
    }
    // Compute from the small lens instead of subtracting two very large squared radii.
    let a = (radius * radius + (d - r) * (d + r)) / (2.0 * d);
    let h = (radius * radius - a * a).max(0.0).sqrt();
    let u = mul(v, 1.0 / d);
    let base = add(lens, mul(u, a));
    Some([
        add(base, [-u[1] * h, u[0] * h]),
        add(base, [u[1] * h, -u[0] * h]),
    ])
}
fn lens_arc(c: Point, r: f64, lens: Point, radius: f64) -> Result<(Vec<Point>, bool), String> {
    if length(sub(c, lens)) + r <= radius + 1e-8 {
        return Ok((circle(c, r, 96), true));
    }
    let [a, b] =
        lens_intersections(c, r, lens, radius).ok_or("Fisheye guide does not cross the lens")?;
    let angle = |p: Point| (p[1] - c[1]).atan2(p[0] - c[0]);
    let start = angle(a);
    let mut delta = (angle(b) - start).rem_euclid(std::f64::consts::TAU);
    let on_circle = |angle: f64| {
        let (s, co) = angle.sin_cos();
        add(c, [co * r, s * r])
    };
    if length(sub(on_circle(start + delta / 2.0), lens)) > radius + 1e-6 {
        delta -= std::f64::consts::TAU;
    }
    let mut path = (0..=96)
        .map(|i| on_circle(start + delta * i as f64 / 96.0))
        .collect::<Vec<_>>();
    path[0] = a;
    path[96] = b;
    Ok((path, false))
}
fn circle_intersection(c: Point, r: f64, lens: Point, radius: f64, p: Point) -> Option<Point> {
    let [a, b] = lens_intersections(c, r, lens, radius)?;
    Some(if length(sub(a, p)) <= length(sub(b, p)) {
        a
    } else {
        b
    })
}
