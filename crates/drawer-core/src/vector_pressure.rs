//! Independent device samples and editable 0..2 control multipliers.
use crate::{line_tools::Path, scene::Point};
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Profile {
    #[serde(default = "device_enabled_default")]
    pub device_enabled: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub device: Vec<Option<f64>>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub controls: Vec<f64>,
}
fn device_enabled_default() -> bool {
    true
}
impl Default for Profile {
    fn default() -> Self {
        Self {
            device_enabled: true,
            device: vec![],
            controls: vec![],
        }
    }
}
fn interpolate(values: impl Fn(usize) -> f64, count: usize, t: f64) -> f64 {
    if count == 0 {
        return 1.;
    }
    if count == 1 {
        return values(0);
    }
    let x = t.clamp(0., 1.) * (count - 1) as f64;
    let n = (x.floor() as usize).min(count - 2);
    let f = x - n as f64;
    values(n) * (1. - f) + values(n + 1) * f
}
impl Profile {
    pub fn is_default(&self) -> bool {
        self.device_enabled && self.device.is_empty() && self.controls.is_empty()
    }
    pub fn valid(&self) -> bool {
        self.device.len() <= 8192
            && self.controls.len() <= 8192
            && self
                .device
                .iter()
                .flatten()
                .all(|v| v.is_finite() && (0. ..=1.).contains(v))
            && self
                .controls
                .iter()
                .all(|v| v.is_finite() && (0. ..=2.).contains(v))
    }
    pub fn bytes(&self) -> usize {
        self.device.len() * std::mem::size_of::<Option<f64>>() + self.controls.len() * 8
    }
    pub fn device_at(&self, t: f64) -> f64 {
        interpolate(|n| self.device[n].unwrap_or(1.), self.device.len(), t)
    }
    pub fn control_at(&self, t: f64) -> f64 {
        interpolate(|n| self.controls[n], self.controls.len(), t)
    }
    pub fn factor(&self, t: f64) -> f64 {
        if self.device_enabled {
            self.device_at(t) * self.control_at(t)
        } else {
            self.control_at(t)
        }
    }
    pub fn set_control(&mut self, n: usize, count: usize, value: f64) -> Result<(), String> {
        if count == 0
            || count > 4096
            || n >= count
            || !value.is_finite()
            || !(0. ..=2.).contains(&value)
        {
            return Err("Vector pressure requires a live control and a 0–2 multiplier".into());
        }
        if self.controls.len() != count {
            self.controls = (0..count)
                .map(|n| {
                    self.control_at(if count == 1 {
                        0.
                    } else {
                        n as f64 / (count - 1) as f64
                    })
                })
                .collect();
        }
        self.controls[n] = (value * 100.).round() / 100.;
        if self.controls.iter().all(|v| *v == 1.) {
            self.controls.clear();
        }
        Ok(())
    }
    pub fn maximum(&self) -> f64 {
        let a = if !self.device_enabled || self.device.is_empty() {
            1.
        } else {
            self.device
                .iter()
                .map(|v| v.unwrap_or(1.))
                .fold(0., f64::max)
        };
        let b = if self.controls.is_empty() {
            1.
        } else {
            self.controls.iter().copied().fold(0., f64::max)
        };
        a * b
    }
}
fn lerp(a: Point, b: Point, t: f64) -> Point {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]
}
impl Path {
    pub fn maximum_width(&self) -> f64 {
        self.width * self.pressure.maximum()
    }
    pub fn width_at(&self, t: f64) -> f64 {
        self.width * self.pressure.factor(t)
    }
    pub fn pressure_segments(&self) -> impl Iterator<Item = (Point, Point, f64, f64)> + '_ {
        let count = self.points.len().saturating_sub(1).max(1);
        let steps = if self.pressure.is_default() {
            1
        } else {
            64usize
                .max(self.pressure.device.len())
                .max(self.pressure.controls.len())
                .div_ceil(count)
                .max(1)
        };
        self.segments().enumerate().flat_map(move |(n, (a, b))| {
            (0..steps).map(move |part| {
                let t0 = part as f64 / steps as f64;
                let t1 = (part + 1) as f64 / steps as f64;
                let width = |t: f64| {
                    if n == self.points.len() - 1 {
                        self.width_at(1.) * (1. - t) + self.width_at(0.) * t
                    } else {
                        self.width_at((n as f64 + t) / count as f64)
                    }
                };
                (lerp(a, b, t0), lerp(a, b, t1), width(t0), width(t1))
            })
        })
    }
    pub fn stroke_hit(&self, p: Point) -> bool {
        if !self.stroke || self.width <= 0. {
            return false;
        }
        if self.pressure.is_default() {
            return self
                .segments()
                .any(|(a, b)| crate::line_tools::distance(p, a, b).0 <= self.width / 2.);
        }
        self.pressure_segments()
            .any(|(a, b, wa, wb)| taper_hit(p, a, b, wa, wb))
    }
    /// Preserve the sampled profile when an older intersection eraser emits a
    /// polyline fragment. The search follows source order across closed wraps.
    pub(crate) fn fragment_pressure(&self, points: &[Point]) -> Profile {
        fragment_pressure(self, points)
    }
}
/// Coverage of the union of disks whose radius varies linearly along an edge.
pub(crate) fn taper_hit(p: Point, a: Point, b: Point, wa: f64, wb: f64) -> bool {
    let r0 = wa / 2.;
    let dr = (wb - wa) / 2.;
    if r0 <= 0. && wb <= 0. {
        return false;
    }
    let q = [p[0] - a[0], p[1] - a[1]];
    let v = [b[0] - a[0], b[1] - a[1]];
    let aa = v[0] * v[0] + v[1] * v[1] - dr * dr;
    let bb = -2. * (q[0] * v[0] + q[1] * v[1] + r0 * dr);
    let t = if aa > 1e-18 {
        (-bb / (2. * aa)).clamp(0., 1.)
    } else if q[0] * q[0] + q[1] * q[1] - r0 * r0
        < (p[0] - b[0]).powi(2) + (p[1] - b[1]).powi(2) - (wb / 2.).powi(2)
    {
        0.
    } else {
        1.
    };
    let r = r0 + dr * t;
    r > 0. && (q[0] - v[0] * t).powi(2) + (q[1] - v[1] * t).powi(2) <= r * r + 1e-12
}
fn fragment_pressure(path: &Path, points: &[Point]) -> Profile {
    if path.pressure.is_default() {
        return Profile::default();
    }
    let edges = path.segments().collect::<Vec<_>>();
    let count = path.points.len().saturating_sub(1).max(1);
    let mut cursor = 0usize;
    let mut device = vec![];
    let mut controls = vec![];
    for point in points {
        let mut best = (f64::INFINITY, 0usize, 0.);
        for delta in 0..edges.len() {
            let n = (cursor + delta) % edges.len();
            let (a, b) = edges[n];
            let (distance, near) = crate::line_tools::distance(*point, a, b);
            let length = (b[0] - a[0]).hypot(b[1] - a[1]);
            let t = if length > 0. {
                (near[0] - a[0]).hypot(near[1] - a[1]) / length
            } else {
                0.
            };
            if distance < best.0 {
                best = (distance, n, t);
                if distance < 1e-8 {
                    break;
                }
            }
        }
        cursor = best.1;
        let t = ((best.1 as f64 + best.2) / count as f64).clamp(0., 1.);
        device.push(Some(path.pressure.device_at(t)));
        controls.push(path.pressure.control_at(t));
    }
    if path.pressure.device.is_empty() {
        device.clear();
    }
    if path.pressure.controls.is_empty() {
        controls.clear();
    }
    Profile {
        device_enabled: path.pressure.device_enabled,
        device,
        controls,
    }
}
pub fn requires_v64(scene: &crate::scene::Scene) -> bool {
    scene
        .lines
        .layers
        .iter()
        .flat_map(|l| &l.paths)
        .any(|p| p.width == 0. || !p.pressure.is_default())
}
