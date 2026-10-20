//! Persistent editable vector selection; area conversion never edits geometry or ink.
use crate::{
    document::Document,
    line_tools::{Layer, Path},
    scene::{Point, Rect, Scene},
    selection_edit::Combine,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Kind {
    #[default]
    Points,
    Strokes,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Control {
    pub path: u64,
    pub point: u16,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct State {
    pub layer: Option<u64>,
    pub kind: Kind,
    pub include_width: bool,
    pub points: Vec<Control>,
    pub paths: Vec<u64>,
}
impl State {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
    pub fn bytes(&self) -> usize {
        self.points.len() * std::mem::size_of::<Control>() + self.paths.len() * 8
    }
    pub fn valid(&self, layers: &[Layer]) -> bool {
        if self.points.len() > 8192
            || self.paths.len() > 128
            || self.points.iter().collect::<BTreeSet<_>>().len() != self.points.len()
            || self.paths.iter().collect::<BTreeSet<_>>().len() != self.paths.len()
        {
            return false;
        }
        let Some(id) = self.layer else {
            return self.points.is_empty() && self.paths.is_empty();
        };
        let Some(layer) = layers.iter().find(|l| l.id == id) else {
            return false;
        };
        (self.kind == Kind::Points && self.paths.is_empty()
            || self.kind == Kind::Strokes && self.points.is_empty())
            && self.points.iter().all(|p| {
                layer
                    .paths
                    .iter()
                    .any(|path| path.id == p.path && (p.point as usize) < path.control_count())
            })
            && self
                .paths
                .iter()
                .all(|id| layer.paths.iter().any(|p| p.id == *id))
    }
    pub fn prune(&mut self, layers: &[Layer]) {
        let Some(layer) = layers.iter().find(|l| Some(l.id) == self.layer) else {
            *self = Self::default();
            return;
        };
        self.points.retain(|p| {
            layer
                .paths
                .iter()
                .any(|path| path.id == p.path && (p.point as usize) < path.control_count())
        });
        self.paths
            .retain(|id| layer.paths.iter().any(|p| p.id == *id));
    }
}
impl Path {
    pub fn control_count(&self) -> usize {
        self.definition
            .as_ref()
            .map_or(self.points.len(), |d| d.controls.len())
    }
    pub fn control_point(&self, n: usize) -> Option<Point> {
        if let Some(d) = &self.definition {
            d.controls.get(n).map(|p| d.creation.inverse().map(*p))
        } else {
            self.points.get(n).copied()
        }
    }
}
fn combine<T: Ord + Copy>(old: &[T], new: Vec<T>, op: Combine) -> Vec<T> {
    let a: BTreeSet<_> = old.iter().copied().collect();
    let b: BTreeSet<_> = new.into_iter().collect();
    match op {
        Combine::Replace => b,
        Combine::Add => a.union(&b).copied().collect(),
        Combine::Subtract => a.difference(&b).copied().collect(),
        Combine::Intersect => a.intersection(&b).copied().collect(),
    }
    .into_iter()
    .collect()
}
fn intersects(a: Point, b: Point, r: Rect) -> bool {
    if r.min[0] >= r.max[0] || r.min[1] >= r.max[1] {
        return false;
    }
    let mut low = 0.0f64;
    let mut high = 1.0f64;
    for k in 0..2 {
        let max = r.max[k].next_down();
        let d = b[k] - a[k];
        if d.abs() < 1e-15 {
            if a[k] < r.min[k] || a[k] > max {
                return false;
            }
        } else {
            let x = (r.min[k] - a[k]) / d;
            let y = (max - a[k]) / d;
            low = low.max(x.min(y));
            high = high.min(x.max(y));
            if low > high {
                return false;
            }
        }
    }
    low <= high
}
fn distance_rect(p: Point, r: Rect) -> f64 {
    (r.min[0] - p[0])
        .max(0.0)
        .max(p[0] - r.max[0])
        .hypot((r.min[1] - p[1]).max(0.0).max(p[1] - r.max[1]))
}
fn touches(a: Point, b: Point, r: Rect, radius: f64) -> bool {
    if r.min[0] >= r.max[0] || r.min[1] >= r.max[1] {
        return false;
    }
    if intersects(a, b, r) {
        return true;
    }
    if radius == 0.0 {
        return false;
    }
    let r = Rect {
        min: r.min,
        max: r.max.map(f64::next_down),
    };
    distance_rect(a, r).min(distance_rect(b, r)).min(
        [r.min, [r.max[0], r.min[1]], r.max, [r.min[0], r.max[1]]]
            .into_iter()
            .map(|p| crate::line_tools::distance(p, a, b).0)
            .fold(f64::INFINITY, f64::min),
    ) <= radius
}
pub fn prepare(
    doc: &Document,
    layer: u64,
    kind: Kind,
    include_width: bool,
    op: Combine,
) -> Result<Scene, String> {
    if !doc.scene.valid() {
        return Err("Invalid vector selection page".into());
    }
    let target = doc
        .scene
        .lines
        .layers
        .iter()
        .find(|l| l.id == layer)
        .ok_or("Choose a vector layer")?;
    if target.hidden || target.locked {
        return Err("Hidden/locked vector layers cannot change editing selection".into());
    }
    if doc.scene.selection.is_none() && doc.scene.selection_mask.is_none() {
        return Err("Create an area selection first; editing selection kept".into());
    }
    if doc.resident_bytes().saturating_add(16 * 1024 * 1024)
        > crate::document::DEFAULT_DOCUMENT_BUDGET
    {
        return Err("Vector selection working memory limit".into());
    }
    let mut points = vec![];
    let mut paths = vec![];
    let mut work = 0usize;
    let rectangles: Vec<_> = if let Some(mask) = &doc.scene.selection_mask {
        mask.runs
            .iter()
            .map(|r| Rect {
                min: [r.x as f64, r.y as f64],
                max: [r.x as f64 + r.length as f64, r.y as f64 + 1.0],
            })
            .collect()
    } else {
        doc.scene.selection.into_iter().collect()
    };
    let inverse = doc.scene.transform.inverse();
    let t = doc.scene.selection_translation;
    let shift = [
        inverse.0[0] * t[0] + inverse.0[2] * t[1],
        inverse.0[1] * t[0] + inverse.0[3] * t[1],
    ];
    for path in &target.paths {
        match kind {
            Kind::Points => {
                for n in 0..path.control_count() {
                    let p = doc.scene.transform.map(
                        path.control_point(n)
                            .ok_or("Invalid editable control point")?,
                    );
                    if p.iter().any(|v| !v.is_finite() || v.abs() > 1e12) {
                        return Err("Vector control point coordinate limit".into());
                    }
                    if doc.scene.selection_coverage(p) > 0 {
                        points.push(Control {
                            path: path.id,
                            point: n as u16,
                        });
                    }
                }
            }
            Kind::Strokes => {
                let radius = if include_width && path.stroke {
                    path.maximum_width() / 2.0
                } else {
                    0.0
                };
                let mut hit = false;
                for (a, b) in path.segments() {
                    let a = [a[0] - shift[0], a[1] - shift[1]];
                    let b = [b[0] - shift[0], b[1] - shift[1]];
                    for rect in &rectangles {
                        work += 1;
                        if work > 64 * 1024 * 1024 {
                            return Err(
                                "Vector/area intersection work limit; selection kept".into()
                            );
                        }
                        if touches(a, b, *rect, radius) {
                            hit = true;
                            break;
                        }
                    }
                    if hit {
                        break;
                    }
                }
                if hit {
                    paths.push(path.id);
                }
            }
        }
    }
    let mut scene = doc.scene.clone();
    let old = &scene.lines.selection;
    let same = old.layer == Some(layer) && old.kind == kind;
    let new = State {
        layer: Some(layer),
        kind,
        include_width,
        points: if kind == Kind::Points {
            combine(if same { &old.points } else { &[] }, points, op)
        } else {
            vec![]
        },
        paths: if kind == Kind::Strokes {
            combine(if same { &old.paths } else { &[] }, paths, op)
        } else {
            vec![]
        },
    };
    scene.lines.selection = new;
    if !scene.valid()
        || serde_json::to_vec(&scene).map_err(|e| e.to_string())?.len() > 4 * 1024 * 1024
    {
        return Err("Vector selection metadata limit".into());
    }
    Ok(scene)
}
pub fn clear(doc: &Document) -> Scene {
    let mut scene = doc.scene.clone();
    scene.lines.selection = State::default();
    scene
}
