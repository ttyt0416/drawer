//! Independent hold recognition and editable shape proposals; raw stroke remains owned by input.
use crate::{
    document::Document,
    scene::Point,
    shapes::{self, Curve, Kind as Shape, Settings as ShapeSettings, Target},
};
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub enabled: bool,
    pub hold_ms: u32,
    pub motion_pixels: f64,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            enabled: false,
            hold_ms: 600,
            motion_pixels: 3.0,
        }
    }
}
impl Settings {
    pub fn valid(self) -> bool {
        self.hold_ms <= 5000
            && self.motion_pixels.is_finite()
            && (0.0..=64.0).contains(&self.motion_pixels)
    }
}
#[derive(Clone, Debug)]
pub struct Capture {
    pub points: Vec<Point>,
    anchor: Point,
    last_motion: f64,
    consumed: bool,
}
impl Capture {
    pub fn new(p: Point, time: f64) -> Result<Self, String> {
        let mut c = Self {
            points: vec![],
            anchor: p,
            last_motion: time,
            consumed: false,
        };
        c.push(p, time, 0.0)?;
        Ok(c)
    }
    pub fn push(&mut self, p: Point, time: f64, motion: f64) -> Result<(), String> {
        if p.iter().any(|v| !v.is_finite() || v.abs() > 1e9)
            || !time.is_finite()
            || time < 0.0
            || !motion.is_finite()
            || motion < 0.0
        {
            return Err("Invalid shape hold input".into());
        }
        if time < self.last_motion || distance(self.anchor, p) > motion {
            self.last_motion = time;
            self.anchor = p;
            self.consumed = false;
        }
        if self.points.last().is_none_or(|q| distance(*q, p) >= 0.25) {
            if self.points.len() == 4096 {
                return Err("Auto shape raw input limit4096; stroke cancelled".into());
            }
            self.points.push(p);
        }
        Ok(())
    }
    pub fn ready(&self, time: f64, s: Settings) -> bool {
        s.enabled
            && s.valid()
            && !self.consumed
            && self.points.len() > 1
            && time.is_finite()
            && time >= self.last_motion
            && (time - self.last_motion) * 1000.0 + 1e-7 >= s.hold_ms as f64
    }
    pub fn waiting(&self) -> bool {
        self.points.len() > 1 && !self.consumed
    }
    pub fn consume(&mut self) {
        self.consumed = true;
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Circle,
    Ellipse,
    Triangle,
    Rectangle,
    Line,
    Curve,
    Polygon,
}
impl Kind {
    pub const ALL: [Self; 7] = [
        Self::Circle,
        Self::Ellipse,
        Self::Triangle,
        Self::Rectangle,
        Self::Line,
        Self::Curve,
        Self::Polygon,
    ];
}
#[derive(Clone, Debug, PartialEq)]
pub struct Plan {
    pub kind: Kind,
    pub controls: Vec<Point>,
    pub corner: f64,
    pub width: f64,
    pub color: [u8; 4],
}
fn distance(a: Point, b: Point) -> f64 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}
fn bounds(points: &[Point]) -> Result<(Point, Point), String> {
    if !(2..=4096).contains(&points.len())
        || points
            .iter()
            .flatten()
            .any(|v| !v.is_finite() || v.abs() > 1e9)
    {
        return Err("Shape proposal needs2–4096 finite points".into());
    }
    let mut lo = [f64::INFINITY; 2];
    let mut hi = [f64::NEG_INFINITY; 2];
    for p in points {
        for i in 0..2 {
            lo[i] = lo[i].min(p[i]);
            hi[i] = hi[i].max(p[i]);
        }
    }
    if distance(lo, hi) < 0.25 {
        return Err("A point remains a freehand dot".into());
    }
    Ok((lo, hi))
}
fn simplify(points: &[Point], tolerance: f64) -> Vec<Point> {
    let mut keep = std::collections::BTreeSet::from([0, points.len() - 1]);
    let mut stack = vec![(0, points.len() - 1)];
    while let Some((a, b)) = stack.pop() {
        if keep.len() >= 128 {
            break;
        }
        let far = (a + 1..b)
            .map(|i| {
                (
                    crate::line_tools::distance(points[i], points[a], points[b]).0,
                    i,
                )
            })
            .max_by(|a, b| a.0.total_cmp(&b.0));
        if let Some((d, i)) = far
            && d > tolerance
        {
            keep.insert(i);
            stack.push((a, i));
            stack.push((i, b));
        }
    }
    keep.into_iter().map(|i| points[i]).collect()
}
pub fn recognize(points: &[Point]) -> Result<Kind, String> {
    let (lo, hi) = bounds(points)?;
    let span = distance(lo, hi);
    let first = points[0];
    let last = *points.last().unwrap();
    let closed = distance(first, last) <= span * 0.2;
    if !closed {
        let deviation = points
            .iter()
            .map(|p| crate::line_tools::distance(*p, first, last).0)
            .fold(0.0, f64::max);
        return Ok(if deviation <= span * 0.06 {
            Kind::Line
        } else {
            Kind::Curve
        });
    }
    let end = if distance(first, last) < 0.25 {
        points.len() - 1
    } else {
        points.len()
    };
    let reduced = simplify(&points[..end], span * 0.04);
    Ok(match reduced.len() {
        3 => Kind::Triangle,
        4 => Kind::Rectangle,
        _ => {
            let w = hi[0] - lo[0];
            let h = hi[1] - lo[1];
            if (w - h).abs() <= w.max(h) * 0.15 {
                Kind::Circle
            } else {
                Kind::Ellipse
            }
        }
    })
}
impl Plan {
    pub fn from_points(
        points: &[Point],
        kind: Kind,
        width: f64,
        color: [u8; 4],
    ) -> Result<Self, String> {
        let (lo, hi) = bounds(points)?;
        if !width.is_finite() || !(0.05..=4096.0).contains(&width) {
            return Err("Invalid converted stroke width".into());
        }
        let center = [(lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0];
        let span = distance(lo, hi);
        let controls = match kind {
            Kind::Circle => {
                let r = (hi[0] - lo[0]).max(hi[1] - lo[1]).max(0.25) / 2.0;
                vec![
                    [center[0] - r, center[1] - r],
                    [center[0] + r, center[1] + r],
                ]
            }
            Kind::Ellipse | Kind::Rectangle => vec![lo, hi],
            Kind::Triangle => vec![[center[0], lo[1]], hi, [lo[0], hi[1]]],
            Kind::Line => {
                let first = points[0];
                let last = *points.last().unwrap();
                let end = if distance(first, last) < span * 0.1 {
                    *points
                        .iter()
                        .max_by(|a, b| distance(first, **a).total_cmp(&distance(first, **b)))
                        .unwrap()
                } else {
                    last
                };
                vec![first, end]
            }
            Kind::Curve => vec![
                points[0],
                points[(points.len() - 1) / 3],
                points[(points.len() - 1) * 2 / 3],
                *points.last().unwrap(),
            ],
            Kind::Polygon => {
                let end = if distance(points[0], *points.last().unwrap()) < 0.25 {
                    points.len() - 1
                } else {
                    points.len()
                };
                let mut p = simplify(&points[..end], span * 0.04);
                if p.len() < 3 {
                    p = vec![[center[0], lo[1]], hi, [lo[0], hi[1]]];
                }
                p
            }
        };
        Ok(Self {
            kind,
            controls,
            corner: 0.0,
            width,
            color,
        })
    }
    pub fn settings(&self, doc: &Document) -> Result<ShapeSettings, String> {
        if self.controls.len() > 128
            || !self.corner.is_finite()
            || !(0.0..=1e6).contains(&self.corner)
        {
            return Err("Invalid auto-shape control/corner limit".into());
        }
        let mut s = ShapeSettings {
            width: self.width / crate::line_tools::maximum_scale(doc.scene.transform),
            line: self.color,
            corner: self.corner,
            ..Default::default()
        };
        match self.kind {
            Kind::Circle => {
                s.kind = Shape::Ellipse;
                s.constraint = shapes::Constraint::SpecifiedRatio;
                s.ratio = 1.0;
            }
            Kind::Ellipse => s.kind = Shape::Ellipse,
            Kind::Rectangle => s.kind = Shape::Rectangle,
            Kind::Triangle | Kind::Polygon => {
                s.kind = Shape::Polygon;
                s.closed = true;
            }
            Kind::Line => s.kind = Shape::Polyline,
            Kind::Curve => {
                s.kind = Shape::Bezier;
                s.curve = Curve::CubicBezier;
            }
        }
        if !s.valid() {
            return Err("Converted geometry exceeds shape settings limits".into());
        }
        Ok(s)
    }
    pub fn geometry(&self, doc: &Document) -> Result<shapes::Geometry, String> {
        shapes::geometry(self.settings(doc)?, &self.controls)
    }
    pub fn prepare(
        &self,
        doc: &Document,
        target: Target,
    ) -> Result<crate::sizing::DocumentState, String> {
        shapes::prepare(doc, target, &self.controls, self.settings(doc)?)
    }
}
pub fn freehand_vector(
    doc: &Document,
    layer: u64,
    points: &[Point],
    width: f64,
    color: [u8; 4],
) -> Result<crate::sizing::DocumentState, String> {
    if !(1..=4096).contains(&points.len())
        || doc.scene.selection.is_some()
        || doc.scene.selection_mask.is_some()
    {
        return Err(
            "Vector freehand requires1–4096 points and unrestricted vector selection".into(),
        );
    }
    if doc.memory_bytes() > crate::document::DEFAULT_DOCUMENT_BUDGET / 8
        || doc
            .scene
            .lines
            .layers
            .iter()
            .find(|l| l.id == layer)
            .is_none_or(|l| l.hidden || l.locked)
    {
        return Err(
            "Vector freehand target is hidden/locked/missing or exceeds snapshot budget".into(),
        );
    }
    let mut state = doc.snapshot();
    let p = points
        .iter()
        .map(|p| doc.scene.transform.inverse().map(*p))
        .collect::<Vec<_>>();
    state.scene.lines.add_path(
        layer,
        if p.len() == 1 { vec![p[0], p[0]] } else { p },
        false,
        width / crate::line_tools::maximum_scale(doc.scene.transform),
        color,
    )?;
    if !state.scene.valid()
        || serde_json::to_vec(&state.scene)
            .map_err(|e| e.to_string())?
            .len()
            > 4 * 1024 * 1024
    {
        return Err("Freehand vector exceeds page geometry limit".into());
    }
    Ok(state)
}

pub fn freehand_vector_pressure(
    doc: &Document,
    layer: u64,
    points: &[Point],
    width: f64,
    color: [u8; 4],
    device: &[Option<f64>],
    device_enabled: bool,
) -> Result<crate::sizing::DocumentState, String> {
    let mut state = freehand_vector(doc, layer, points, width, color)?;
    attach_device(&mut state, layer, device, device_enabled)?;
    Ok(state)
}
pub fn attach_device(
    state: &mut crate::sizing::DocumentState,
    layer: u64,
    device: &[Option<f64>],
    device_enabled: bool,
) -> Result<(), String> {
    let path = state
        .scene
        .lines
        .layers
        .iter_mut()
        .find(|l| l.id == layer)
        .and_then(|l| l.paths.last_mut())
        .ok_or("Missing vector pressure target")?;
    path.pressure.device = if device.iter().any(Option::is_some) {
        device.to_vec()
    } else {
        vec![]
    };
    path.pressure.device_enabled = path.pressure.device.is_empty() || device_enabled;
    if !state.scene.valid()
        || serde_json::to_vec(&state.scene)
            .map_err(|e| e.to_string())?
            .len()
            > 4 * 1024 * 1024
    {
        return Err("Vector input pressure exceeds geometry limits".into());
    }
    Ok(())
}
