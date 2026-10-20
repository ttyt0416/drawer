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
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub device_positions: Vec<f64>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub control_positions: Vec<f64>,
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
            device_positions: vec![],
            control_positions: vec![],
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
fn positions_valid(p: &[f64], count: usize) -> bool {
    p.is_empty()
        || p.len() == count
            && p.iter().all(|t| t.is_finite() && (0. ..=1.).contains(t))
            && p.windows(2).all(|w| w[0] < w[1])
}
fn located(values: impl Fn(usize) -> f64, count: usize, positions: &[f64], t: f64) -> f64 {
    if positions.is_empty() {
        return interpolate(values, count, t);
    }
    if t <= positions[0] {
        return values(0);
    }
    let n = positions
        .partition_point(|v| *v < t)
        .saturating_sub(1)
        .min(count - 1);
    if n + 1 == count {
        return values(n);
    }
    let f = ((t - positions[n]) / (positions[n + 1] - positions[n])).clamp(0., 1.);
    values(n) * (1. - f) + values(n + 1) * f
}
impl Profile {
    pub fn is_default(&self) -> bool {
        self.device_enabled
            && self.device.is_empty()
            && self.controls.is_empty()
            && self.device_positions.is_empty()
            && self.control_positions.is_empty()
    }
    pub fn valid(&self) -> bool {
        self.device.len() <= 8192
            && self.controls.len() <= 8192
            && positions_valid(&self.device_positions, self.device.len())
            && positions_valid(&self.control_positions, self.controls.len())
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
        self.device.len() * std::mem::size_of::<Option<f64>>()
            + (self.controls.len() + self.device_positions.len() + self.control_positions.len()) * 8
    }
    pub fn device_at(&self, t: f64) -> f64 {
        located(
            |n| self.device[n].unwrap_or(1.),
            self.device.len(),
            &self.device_positions,
            t,
        )
    }
    pub fn control_at(&self, t: f64) -> f64 {
        located(
            |n| self.controls[n],
            self.controls.len(),
            &self.control_positions,
            t,
        )
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
        if self.controls.len() != count || !self.control_positions.is_empty() {
            self.controls = (0..count)
                .map(|n| {
                    self.control_at(if count == 1 {
                        0.
                    } else {
                        n as f64 / (count - 1) as f64
                    })
                })
                .collect();
            self.control_positions.clear();
        }
        self.controls[n] = (value * 100.).round() / 100.;
        if self.controls.iter().all(|v| *v == 1.) {
            self.controls.clear();
        }
        Ok(())
    }
    pub fn edit_node_controls(
        &mut self,
        parameters: &[f64],
        edits: &[(usize, f64)],
        closing_period: Option<f64>,
    ) -> Result<(), String> {
        if !(2..=4096).contains(&parameters.len())
            || parameters[0] != 0.
            || !positions_valid(parameters, parameters.len())
            || edits
                .iter()
                .any(|(n, v)| *n >= parameters.len() || !v.is_finite() || !(0. ..=2.).contains(v))
            || closing_period.is_some_and(|p| !p.is_finite() || p <= 1.)
        {
            return Err("Invalid node pressure anchors".into());
        }
        let mut positions = parameters.to_vec();
        let extra = *positions.last().unwrap() < 1.;
        if extra {
            positions.push(1.);
        }
        let mut controls = positions
            .iter()
            .map(|t| self.control_at(*t))
            .collect::<Vec<_>>();
        for (n, value) in edits {
            controls[*n] = (*value * 100.).round() / 100.;
        }
        if extra && let Some(period) = closing_period {
            let n = parameters.len() - 1;
            let f = (1. - parameters[n]) / (period - parameters[n]);
            controls[n + 1] = controls[n] * (1. - f) + controls[0] * f;
        }
        let uniform = positions
            .iter()
            .enumerate()
            .all(|(n, t)| (*t - n as f64 / (positions.len() - 1) as f64).abs() < 1e-12);
        self.controls = controls;
        self.control_positions = if uniform { vec![] } else { positions };
        if self.controls.iter().all(|v| *v == 1.) {
            self.controls.clear();
            self.control_positions.clear();
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
    pub(crate) fn pressure_values(&self, parameter: f64) -> (f64, f64) {
        let count = self.points.len().saturating_sub(1).max(1) as f64;
        let period = (count + 1.) / count;
        let t = if self.closed {
            parameter.rem_euclid(period)
        } else {
            parameter.clamp(0., 1.)
        };
        if t <= 1. {
            return (self.pressure.device_at(t), self.pressure.control_at(t));
        }
        let f = ((t - 1.) * count).clamp(0., 1.);
        (
            self.pressure.device_at(1.) * (1. - f) + self.pressure.device_at(0.) * f,
            self.pressure.control_at(1.) * (1. - f) + self.pressure.control_at(0.) * f,
        )
    }
    pub fn maximum_width(&self) -> f64 {
        self.width * self.pressure.maximum()
    }
    pub fn width_at(&self, t: f64) -> f64 {
        self.width * self.pressure.factor(t)
    }
    pub fn pressure_segments(&self) -> impl Iterator<Item = (Point, Point, f64, f64)> + '_ {
        self.parameter_segments()
            .map(|(_, _, a, b, wa, wb)| (a, b, wa, wb))
    }
    pub(crate) fn parameter_segments(
        &self,
    ) -> impl Iterator<Item = (f64, f64, Point, Point, f64, f64)> + '_ {
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
                        let (device, control) = self.pressure_values((n as f64 + t) / count as f64);
                        self.width
                            * control
                            * if self.pressure.device_enabled {
                                device
                            } else {
                                1.
                            }
                    } else {
                        self.width_at((n as f64 + t) / count as f64)
                    }
                };
                (
                    (n as f64 + t0) / count as f64,
                    (n as f64 + t1) / count as f64,
                    lerp(a, b, t0),
                    lerp(a, b, t1),
                    width(t0),
                    width(t1),
                )
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
    pub(crate) fn fragment_pressure_at(&self, parameters: &[f64]) -> Result<Profile, String> {
        fragment_pressure(self, parameters)
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
fn fragment_pressure(path: &Path, parameters: &[f64]) -> Result<Profile, String> {
    if path.pressure.is_default() {
        return Ok(Profile::default());
    }
    if parameters.len() == path.points.len()
        && parameters.iter().enumerate().all(|(n, t)| {
            (*t - n as f64 / path.points.len().saturating_sub(1).max(1) as f64).abs() < 1e-12
        })
    {
        return Ok(path.pressure.clone());
    }
    if parameters.len() < 2
        || parameters.len() > 4096
        || parameters.iter().any(|t| !t.is_finite() || *t < 0.)
        || parameters.windows(2).any(|w| w[0] > w[1])
    {
        return Err("Invalid fragment source parameters".into());
    }
    let count = path.points.len().saturating_sub(1).max(1) as f64;
    let period = if path.closed {
        (count + 1.) / count
    } else {
        1.
    };
    let mut knots = vec![0., 1., period];
    for (length, positions) in [
        (path.pressure.device.len(), &path.pressure.device_positions),
        (
            path.pressure.controls.len(),
            &path.pressure.control_positions,
        ),
    ] {
        if positions.is_empty() {
            knots.extend((0..length).map(|n| n as f64 / length.saturating_sub(1).max(1) as f64));
        } else {
            knots.extend(positions);
        }
    }
    let base = knots.clone();
    if path.closed {
        for n in 1..=2 {
            knots.extend(base.iter().map(|t| t + n as f64 * period));
        }
    }
    knots.sort_by(f64::total_cmp);
    knots.dedup_by(|a, b| (*a - *b).abs() < 1e-12);
    let mut samples = vec![(0., parameters[0])];
    let edges = parameters.len() - 1;
    for (n, w) in parameters.windows(2).enumerate() {
        if w[1] > w[0] {
            let first = knots.partition_point(|t| *t <= w[0] + 1e-12);
            for t in knots[first..].iter().take_while(|t| **t < w[1] - 1e-12) {
                samples.push(((n as f64 + (t - w[0]) / (w[1] - w[0])) / edges as f64, *t));
            }
        }
        samples.push(((n + 1) as f64 / edges as f64, w[1]));
        if samples.len() > 8192 {
            return Err("Fragment pressure sample limit; no paths changed".into());
        }
    }
    let positions = samples.iter().map(|(t, _)| *t).collect::<Vec<_>>();
    let mut result = Profile {
        device_enabled: path.pressure.device_enabled,
        ..Default::default()
    };
    if !path.pressure.device.is_empty() {
        result.device = samples
            .iter()
            .map(|(_, t)| {
                let local = if path.closed {
                    t.rem_euclid(period)
                } else {
                    *t
                };
                let original = if path.pressure.device_positions.is_empty() {
                    let x = local * path.pressure.device.len().saturating_sub(1) as f64;
                    ((x - x.round()).abs() < 1e-10 && local <= 1.).then_some(x.round() as usize)
                } else {
                    path.pressure
                        .device_positions
                        .iter()
                        .position(|p| (*p - local).abs() < 1e-10)
                };
                original
                    .filter(|n| *n < path.pressure.device.len())
                    .map_or_else(
                        || Some(path.pressure_values(*t).0),
                        |n| path.pressure.device[n],
                    )
            })
            .collect();
        result.device_positions = positions.clone();
    }
    if !path.pressure.controls.is_empty() {
        result.controls = samples
            .iter()
            .map(|(_, t)| path.pressure_values(*t).1)
            .collect();
        result.control_positions = positions;
    }
    if !result.valid() {
        return Err("Invalid fragment pressure".into());
    }
    Ok(result)
}
pub fn requires_v65(scene: &crate::scene::Scene) -> bool {
    scene.lines.layers.iter().flat_map(|l| &l.paths).any(|p| {
        !p.pressure.device_positions.is_empty() || !p.pressure.control_positions.is_empty()
    })
}
pub fn requires_v64(scene: &crate::scene::Scene) -> bool {
    scene
        .lines
        .layers
        .iter()
        .flat_map(|l| &l.paths)
        .any(|p| p.width == 0. || !p.pressure.is_default())
}
