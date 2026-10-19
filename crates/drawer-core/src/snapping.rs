//! Screen-distance capture with fixed ruler → guide → grid priority.
use crate::{
    document::Document,
    scene::{Affine, Point},
};
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Modifier {
    #[default]
    Control,
    Shift,
    Alt,
}
impl Modifier {
    pub const ALL: [Self; 3] = [Self::Control, Self::Shift, Self::Alt];
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub rulers: bool,
    pub guides: bool,
    pub grid: bool,
    #[serde(default)]
    pub objects: bool,
    #[serde(default)]
    pub control_points: bool,
    #[serde(default)]
    pub canvas_edges: bool,
    pub screen_distance: f64,
    pub modifier: Modifier,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            rulers: true,
            guides: false,
            grid: false,
            objects: false,
            control_points: false,
            canvas_edges: false,
            screen_distance: 8.0,
            modifier: Modifier::Control,
        }
    }
}
impl Settings {
    pub fn valid(self) -> bool {
        (0.1..=128.0).contains(&self.screen_distance)
    }
}
#[derive(Clone)]
pub(crate) struct Lower {
    geometry: crate::object_snap::Geometry,
    guides: Vec<[Point; 2]>,
    grid: Option<(crate::aids::Grid, Affine)>,
    distance: f64,
    work: usize,
    failed: bool,
}
impl Lower {
    pub fn new(doc: &Document, s: Settings, document_per_screen: f64) -> Result<Self, String> {
        if !s.valid()
            || !document_per_screen.is_finite()
            || !(1e-9..=1e6).contains(&document_per_screen)
            || !doc.scene.valid()
        {
            return Err("Invalid snapping settings, page or view scale".into());
        }
        let guides = if s.guides {
            doc.scene
                .guides
                .iter()
                .map(|g| g.map(|p| doc.scene.transform.map(p)))
                .collect()
        } else {
            vec![]
        };
        let result = Self {
            geometry: crate::object_snap::Geometry::new(doc, s, &[])?,
            guides,
            grid: s.grid.then_some((doc.scene.grid, doc.scene.transform)),
            distance: s.screen_distance * document_per_screen,
            work: 0,
            failed: false,
        };
        if result
            .guides
            .iter()
            .flatten()
            .flatten()
            .any(|v| !v.is_finite() || v.abs() > 1e12)
        {
            return Err("Guide snapping coordinate limit".into());
        }
        Ok(result)
    }
    pub fn bytes(&self) -> usize {
        self.guides.len() * 32 + self.geometry.bytes()
    }
    pub fn failed(&self) -> bool {
        self.failed
    }
    pub fn snap(&mut self, p: Point) -> (Point, bool) {
        self.work = self
            .work
            .saturating_add(self.guides.len() + self.geometry.work() + 1);
        if self.work > 64 * 1024 * 1024 {
            self.failed = true;
            return (p, false);
        }
        let mut best = None;
        for [a, b] in &self.guides {
            let d = [b[0] - a[0], b[1] - a[1]];
            let length = d[0].hypot(d[1]);
            if length < 1e-9 {
                continue;
            }
            let unit = d.map(|v| v / length);
            let at = (p[0] - a[0]) * unit[0] + (p[1] - a[1]) * unit[1];
            let q = [a[0] + at * unit[0], a[1] + at * unit[1]];
            let distance = (p[0] - q[0]).hypot(p[1] - q[1]);
            if distance <= self.distance && best.is_none_or(|(_, old)| distance < old - 1e-9) {
                best = Some((q, distance));
            }
        }
        if let Some((q, _)) = best {
            return (q, true);
        }
        if let Some((q, _)) = self.geometry.snap(p, self.distance) {
            return (q, true);
        }
        if let Some((grid, transform)) = self.grid {
            let original = transform.inverse().map(p);
            let q = std::array::from_fn(|k| {
                grid.origin[k]
                    + ((original[k] - grid.origin[k]) / grid.spacing[k]).round() * grid.spacing[k]
            });
            let q = transform.map(q);
            if q.iter().all(|v| v.is_finite() && v.abs() <= 1e12)
                && (p[0] - q[0]).hypot(p[1] - q[1]) <= self.distance
            {
                return (q, true);
            }
        }
        (p, false)
    }
}
