//! Bounded independent shape construction shared by mouse/pen input and local documents.
use crate::{
    document::{DEFAULT_DOCUMENT_BUDGET, Document},
    line_tools::{Path, point_valid},
    scene::{Affine, Point},
    sizing::DocumentState,
};
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Kind {
    #[default]
    Rectangle,
    Ellipse,
    Polygon,
    Polyline,
    Bezier,
}
impl Kind {
    pub const ALL: [Self; 5] = [
        Self::Rectangle,
        Self::Ellipse,
        Self::Polygon,
        Self::Polyline,
        Self::Bezier,
    ];
    pub fn is_box(self) -> bool {
        matches!(self, Self::Rectangle | Self::Ellipse)
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Curve {
    #[default]
    StraightLine,
    Spline,
    QuadraticBezier,
    CubicBezier,
}
impl Curve {
    pub const ALL: [Self; 4] = [
        Self::StraightLine,
        Self::Spline,
        Self::QuadraticBezier,
        Self::CubicBezier,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::StraightLine => "Straight line",
            Self::Spline => "Spline",
            Self::QuadraticBezier => "Quadratic Bezier",
            Self::CubicBezier => "Cubic Bezier",
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Input {
    ByClick,
    #[default]
    ByDrag,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Paint {
    CreateFill,
    #[default]
    CreateLine,
    CreateBoth,
}
impl Paint {
    pub const ALL: [Self; 3] = [Self::CreateFill, Self::CreateLine, Self::CreateBoth];
    pub fn line(self) -> bool {
        self != Self::CreateFill
    }
    pub fn fill(self) -> bool {
        self != Self::CreateLine
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::CreateFill => "Create fill",
            Self::CreateLine => "Create line",
            Self::CreateBoth => "Create both line and fill",
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Constraint {
    #[default]
    Free,
    SpecifiedRatio,
    SpecifyLength,
}
impl Constraint {
    pub const ALL: [Self; 3] = [Self::Free, Self::SpecifiedRatio, Self::SpecifyLength];
    pub fn label(self) -> &'static str {
        match self {
            Self::Free => "Free",
            Self::SpecifiedRatio => "Specified ratio",
            Self::SpecifyLength => "Specify length",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub kind: Kind,
    pub curve: Curve,
    pub input: Input,
    pub paint: Paint,
    pub constraint: Constraint,
    pub ratio: f64,
    pub length: f64,
    pub width: f64,
    pub corner: f64,
    pub center: bool,
    pub closed: bool,
    pub snap: bool,
    pub snap_step: f64,
    pub line: [u8; 4],
    pub fill: [u8; 4],
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            kind: Kind::Rectangle,
            curve: Curve::StraightLine,
            input: Input::ByDrag,
            paint: Paint::CreateLine,
            constraint: Constraint::Free,
            ratio: 1.0,
            length: 64.0,
            width: 2.0,
            corner: 0.0,
            center: false,
            closed: false,
            snap: false,
            snap_step: 1.0,
            line: [0, 0, 0, 255],
            fill: [200, 40, 20, 255],
        }
    }
}
impl Settings {
    pub fn valid(&self) -> bool {
        (0.01..=100.0).contains(&self.ratio)
            && (0.1..=1e6).contains(&self.length)
            && (0.1..=512.0).contains(&self.width)
            && (0.0..=1e6).contains(&self.corner)
            && (0.1..=1e6).contains(&self.snap_step)
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Definition {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nodes: Option<crate::vector_nodes::Data>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub snapped_controls: bool,
    pub settings: Settings,
    pub controls: Vec<Point>,
    pub creation: Affine,
}
fn is_false(v: &bool) -> bool {
    !*v
}
impl Definition {
    pub fn geometry(&self) -> Result<Geometry, String> {
        if let Some(nodes) = &self.nodes {
            return Ok(Geometry {
                points: nodes.sample(&self.controls, self.settings.closed)?.points,
                closed: self.settings.closed,
            });
        }
        let mut s = self.settings;
        if self.snapped_controls {
            s.snap = false;
        }
        geometry(s, &self.controls)
    }
    pub fn valid(&self) -> bool {
        self.settings.valid()
            && self.creation.valid()
            && (2..=if self.nodes.is_some() { 4096 } else { 128 }).contains(&self.controls.len())
            && self.nodes.as_ref().is_none_or(|n| {
                n.valid(&self.controls) && (!self.settings.closed || self.controls.len() >= 3)
            })
            && self.controls.iter().copied().all(point_valid)
    }
    pub fn bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + self.controls.len() * 16
            + self
                .nodes
                .as_ref()
                .map_or(0, crate::vector_nodes::Data::bytes)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Raster(usize),
    Vector(Option<u64>),
}
impl Default for Target {
    fn default() -> Self {
        Self::Raster(0)
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct Geometry {
    pub points: Vec<Point>,
    pub closed: bool,
}
fn lerp(a: Point, b: Point, t: f64) -> Point {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]
}
fn distance(a: Point, b: Point) -> f64 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}
fn quadratic(a: Point, b: Point, c: Point, t: f64) -> Point {
    lerp(lerp(a, b, t), lerp(b, c, t), t)
}
fn cubic(a: Point, b: Point, c: Point, d: Point, t: f64) -> Point {
    lerp(quadratic(a, b, c, t), quadratic(b, c, d, t), t)
}
fn rounded(p: &[Point], closed: bool, r: f64) -> Vec<Point> {
    if r == 0.0 {
        return p.to_vec();
    }
    let mut out = vec![];
    for i in 0..p.len() {
        if !closed && (i == 0 || i + 1 == p.len()) {
            out.push(p[i]);
            continue;
        }
        let a = p[(i + p.len() - 1) % p.len()];
        let b = p[i];
        let c = p[(i + 1) % p.len()];
        let trim = r.min(distance(a, b) / 2.0).min(distance(b, c) / 2.0);
        if trim < 1e-9 {
            out.push(b);
            continue;
        }
        let from = lerp(b, a, trim / distance(a, b));
        let to = lerp(b, c, trim / distance(b, c));
        for n in 0..=8 {
            out.push(quadratic(from, b, to, n as f64 / 8.0));
        }
    }
    out
}
pub fn geometry(s: Settings, input: &[Point]) -> Result<Geometry, String> {
    if !s.valid() || !(2..=128).contains(&input.len()) || !input.iter().copied().all(point_valid) {
        return Err("Shape requires 2–128 finite control points and valid settings".into());
    }
    let mut p = input.to_vec();
    if s.snap {
        for point in &mut p {
            for v in point {
                *v = (*v / s.snap_step).round() * s.snap_step;
            }
        }
    }
    let a = p[0];
    let last = *p.last().unwrap();
    let mut d = [last[0] - a[0], last[1] - a[1]];
    match s.constraint {
        Constraint::Free => {}
        Constraint::SpecifiedRatio => {
            let width = d[0].abs().max(d[1].abs() * s.ratio);
            let height = width / s.ratio;
            d = [
                width * if d[0] < 0.0 { -1.0 } else { 1.0 },
                height * if d[1] < 0.0 { -1.0 } else { 1.0 },
            ];
        }
        Constraint::SpecifyLength => {
            let len = d[0].hypot(d[1]);
            if len < 1e-9 {
                return Err("A fixed length needs a direction".into());
            }
            d = [d[0] * s.length / len, d[1] * s.length / len];
        }
    }
    if s.constraint != Constraint::Free {
        let old = [last[0] - a[0], last[1] - a[1]];
        for v in &mut p {
            for i in 0..2 {
                v[i] = a[i]
                    + if old[i].abs() > 1e-9 {
                        (v[i] - a[i]) * d[i] / old[i]
                    } else {
                        d[i] * (if *v == a { 0.0 } else { 1.0 })
                    };
            }
        }
        *p.last_mut().unwrap() = [a[0] + d[0], a[1] + d[1]];
    }
    let closed = s.kind.is_box() || s.kind == Kind::Polygon || s.closed || s.paint.fill();
    let points = if s.kind.is_box() {
        let b = [a[0] + d[0], a[1] + d[1]];
        let a = if s.center {
            [a[0] - d[0], a[1] - d[1]]
        } else {
            a
        };
        let min = [a[0].min(b[0]), a[1].min(b[1])];
        let max = [a[0].max(b[0]), a[1].max(b[1])];
        if max[0] - min[0] < 1e-9 || max[1] - min[1] < 1e-9 {
            return Err("Shape needs nonzero width and height".into());
        }
        if s.kind == Kind::Ellipse {
            (0..96)
                .map(|n| {
                    let (sin, cos) = (std::f64::consts::TAU * n as f64 / 96.0).sin_cos();
                    [
                        (min[0] + max[0]) / 2.0 + (max[0] - min[0]) * cos / 2.0,
                        (min[1] + max[1]) / 2.0 + (max[1] - min[1]) * sin / 2.0,
                    ]
                })
                .collect()
        } else {
            rounded(
                &[min, [max[0], min[1]], max, [min[0], max[1]]],
                true,
                s.corner.min((max[0] - min[0]).min(max[1] - min[1]) / 2.0),
            )
        }
    } else {
        match s.curve {
            Curve::StraightLine => rounded(&p, closed, s.corner),
            Curve::QuadraticBezier => {
                let control = if p.len() == 2 {
                    lerp(p[0], *p.last().unwrap(), 0.5)
                } else {
                    let n = (p.len() - 2) as f64;
                    p[1..p.len() - 1]
                        .iter()
                        .fold([0.0; 2], |a, b| [a[0] + b[0] / n, a[1] + b[1] / n])
                };
                (0..=64)
                    .map(|n| quadratic(p[0], control, *p.last().unwrap(), n as f64 / 64.0))
                    .collect()
            }
            Curve::CubicBezier => {
                let b = if p.len() == 2 {
                    lerp(p[0], p[1], 1.0 / 3.0)
                } else {
                    p[1]
                };
                let c = if p.len() == 2 {
                    lerp(p[0], p[1], 2.0 / 3.0)
                } else {
                    p[p.len() - 2]
                };
                (0..=96)
                    .map(|n| cubic(p[0], b, c, *p.last().unwrap(), n as f64 / 96.0))
                    .collect()
            }
            Curve::Spline => {
                let segments = if closed { p.len() } else { p.len() - 1 };
                let mut out = vec![];
                for i in 0..segments {
                    let at = |n: isize| {
                        if closed {
                            p[n.rem_euclid(p.len() as isize) as usize]
                        } else {
                            p[n.clamp(0, p.len() as isize - 1) as usize]
                        }
                    };
                    let [a, b, c, d] = [
                        at(i as isize - 1),
                        at(i as isize),
                        at(i as isize + 1),
                        at(i as isize + 2),
                    ];
                    for n in 0..32 {
                        let t = n as f64 / 32.0;
                        out.push(std::array::from_fn(|k| {
                            0.5 * ((2.0 * b[k])
                                + (-a[k] + c[k]) * t
                                + (2.0 * a[k] - 5.0 * b[k] + 4.0 * c[k] - d[k]) * t * t
                                + (-a[k] + 3.0 * b[k] - 3.0 * c[k] + d[k]) * t * t * t)
                        }));
                    }
                }
                if !closed {
                    out.push(*p.last().unwrap())
                }
                out
            }
        }
    };
    if points.len() > 4096
        || points.len() < if closed { 3 } else { 2 }
        || !points.iter().copied().all(point_valid)
        || points.iter().all(|p| distance(*p, points[0]) < 1e-9)
    {
        return Err("Degenerate shape or generated point limit".into());
    }
    if s.paint.fill()
        && !points.windows(3).any(|p| {
            ((p[1][0] - p[0][0]) * (p[2][1] - p[0][1]) - (p[1][1] - p[0][1]) * (p[2][0] - p[0][0]))
                .abs()
                > 1e-9
        })
    {
        return Err("A fill needs a non-collinear closed shape".into());
    }
    Ok(Geometry { points, closed })
}
pub fn contains(points: &[Point], p: Point) -> bool {
    let mut inside = false;
    for (a, b) in points
        .iter()
        .zip(points.iter().cycle().skip(1))
        .take(points.len())
    {
        if (a[1] > p[1]) != (b[1] > p[1])
            && p[0] < (b[0] - a[0]) * (p[1] - a[1]) / (b[1] - a[1]) + a[0]
        {
            inside = !inside
        }
    }
    inside
}
pub fn prepare(
    doc: &Document,
    target: Target,
    controls: &[Point],
    s: Settings,
) -> Result<DocumentState, String> {
    prepare_inner(doc, target, controls, s, false)
}
pub fn prepare_resolved(
    doc: &Document,
    target: Target,
    controls: &[Point],
    s: Settings,
) -> Result<DocumentState, String> {
    prepare_inner(doc, target, controls, s, true)
}
fn prepare_inner(
    doc: &Document,
    target: Target,
    controls: &[Point],
    s: Settings,
    resolved: bool,
) -> Result<DocumentState, String> {
    if !doc.scene.valid() || doc.memory_bytes() > DEFAULT_DOCUMENT_BUDGET / 8 {
        return Err("Invalid shape page or undo budget".into());
    }
    let mut effective = s;
    if resolved {
        effective.snap = false;
    }
    let geometry = geometry(effective, controls)?;
    let inverse = doc.scene.transform.inverse();
    let points: Vec<_> = geometry
        .points
        .into_iter()
        .map(|p| inverse.map(p))
        .collect();
    if !points.iter().copied().all(point_valid) {
        return Err("Shape page coordinate limit".into());
    }
    let path = Path {
        pressure: Default::default(),
        id: 1,
        points,
        closed: geometry.closed,
        width: s.width,
        color: s.line,
        fill: s.paint.fill().then_some(s.fill),
        stroke: s.paint.line(),
        definition: Some(Definition {
            nodes: None,
            settings: s,
            snapped_controls: resolved,
            controls: controls.to_vec(),
            creation: doc.scene.transform,
        }),
    };
    let state = match target {
        Target::Vector(id) => {
            if doc.scene.selection.is_some() || doc.scene.selection_mask.is_some() {
                return Err(
                    "Clear the raster selection before creating an editable vector shape".into(),
                );
            }
            let mut state = doc.snapshot();
            let id = match id {
                Some(id) => id,
                None => state.scene.lines.add_layer("Shape vectors")?,
            };
            let path_id = state.scene.lines.add_path(
                id,
                path.points.clone(),
                path.closed,
                path.width,
                path.color,
            )?;
            let made = state
                .scene
                .lines
                .layers
                .iter_mut()
                .find(|l| l.id == id)
                .unwrap()
                .paths
                .iter_mut()
                .find(|p| p.id == path_id)
                .unwrap();
            *made = Path {
                id: path_id,
                ..path
            };
            state
        }
        Target::Raster(index) => {
            crate::layer_properties::paintable(doc, index)?;
            let raster = doc
                .layers
                .get(index)
                .ok_or("Choose a raster shape target")?;
            if raster.locked || !doc.scene.paper.raster_visible(index, doc.layers.len()) {
                return Err("Shape target is hidden or locked".into());
            }
            let count = doc.width as usize * doc.height as usize;
            if count == 0
                || count > 4 * 1024 * 1024
                || count
                    .saturating_mul(12)
                    .saturating_add(doc.resident_bytes())
                    > DEFAULT_DOCUMENT_BUDGET
            {
                return Err("Raster shape canvas/memory work limit".into());
            }
            let bounds = path.bounds();
            let corners = [
                bounds.min,
                [bounds.max[0], bounds.min[1]],
                bounds.max,
                [bounds.min[0], bounds.max[1]],
            ]
            .map(|p| doc.scene.transform.map(p));
            let mut min = [f64::INFINITY; 2];
            let mut max = [f64::NEG_INFINITY; 2];
            for p in corners {
                for k in 0..2 {
                    min[k] = min[k].min(p[k]);
                    max[k] = max[k].max(p[k]);
                }
            }
            let low = [
                min[0].floor().clamp(0.0, doc.width as f64) as u32,
                min[1].floor().clamp(0.0, doc.height as f64) as u32,
            ];
            let high = [
                max[0].ceil().clamp(0.0, doc.width as f64) as u32,
                max[1].ceil().clamp(0.0, doc.height as f64) as u32,
            ];
            if ((high[0] - low[0]) as usize)
                .saturating_mul((high[1] - low[1]) as usize)
                .saturating_mul(path.points.len())
                .saturating_mul(32)
                > 64 * 1024 * 1024
            {
                return Err("Raster shape sampling work limit".into());
            }
            let mut pixels = vec![[0; 4]; count];
            for y in low[1]..high[1] {
                for x in low[0]..high[0] {
                    let mut sum = [0.0; 4];
                    for sy in 0..4 {
                        for sx in 0..4 {
                            let p = path.pixel(inverse.map([
                                x as f64 + (sx as f64 + 0.5) / 4.0,
                                y as f64 + (sy as f64 + 0.5) / 4.0,
                            ]));
                            for k in 0..3 {
                                sum[k] += p[k] as f64 * p[3] as f64;
                            }
                            sum[3] += p[3] as f64;
                        }
                    }
                    if sum[3] > 0.0 {
                        let mut p = [0; 4];
                        for k in 0..3 {
                            p[k] = (sum[k] / sum[3]).round() as u8;
                        }
                        p[3] = (sum[3] / 16.0
                            * doc
                                .scene
                                .selection_coverage([x as f64 + 0.5, y as f64 + 0.5])
                                as f64
                            / 255.0)
                            .round() as u8;
                        pixels[y as usize * doc.width as usize + x as usize] = p;
                    }
                }
            }
            crate::bucket::paint_pixels(doc, index, false, |x, y| {
                pixels[y as usize * doc.width as usize + x as usize]
            })?
        }
    };
    if !state.scene.valid()
        || serde_json::to_vec(&state.scene)
            .map_err(|e| e.to_string())?
            .len()
            > 4 * 1024 * 1024
    {
        return Err("Shape scene/metadata limit".into());
    }
    Ok(state)
}
