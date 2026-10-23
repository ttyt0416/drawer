//! Validated triangular grids and vertex-anchored rigid pin blending.
use crate::scene::{Point, Rect};
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Mesh {
    pub columns: u8,
    pub rows: u8,
    pub points: Vec<Point>,
    /// Empty for a strict mesh; puppet triangles may overlap and are ordered back to front.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub order: Vec<[i64; 2]>,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Pin {
    pub vertex: usize,
    pub target: Point,
    pub degrees: f64,
    pub locked: bool,
    pub depth: i32,
    pub keep_angle: bool,
    pub reference_degrees: f64,
}
fn sub(a: Point, b: Point) -> Point {
    [a[0] - b[0], a[1] - b[1]]
}
fn cross(a: Point, b: Point) -> f64 {
    a[0] * b[1] - a[1] * b[0]
}
fn regular(columns: u8, rows: u8) -> Vec<Point> {
    (0..rows)
        .flat_map(|y| {
            (0..columns).map(move |x| {
                [
                    x as f64 / (columns - 1) as f64,
                    y as f64 / (rows - 1) as f64,
                ]
            })
        })
        .collect()
}
fn overlap(a: [Point; 3], b: [Point; 3]) -> bool {
    // Separating axis test with strict interior overlap; shared edges/vertices are allowed.
    a.iter()
        .zip(a.iter().cycle().skip(1))
        .take(3)
        .chain(b.iter().zip(b.iter().cycle().skip(1)).take(3))
        .all(|(p, q)| {
            let edge = sub(*q, *p);
            let axis = [-edge[1], edge[0]];
            let project = |t: [Point; 3]| {
                let values = t.map(|p| p[0] * axis[0] + p[1] * axis[1]);
                [
                    values.into_iter().fold(f64::INFINITY, f64::min),
                    values.into_iter().fold(f64::NEG_INFINITY, f64::max),
                ]
            };
            let x = project(a);
            let y = project(b);
            x[1].min(y[1]) - x[0].max(y[0]) > 1e-10 * axis[0].hypot(axis[1])
        })
}
impl Mesh {
    pub fn new(columns: u8, rows: u8) -> Result<Self, String> {
        if !(2..=5).contains(&columns) || !(2..=5).contains(&rows) {
            return Err("Mesh requires2–5 rows and columns".into());
        }
        Ok(Self {
            columns,
            rows,
            points: regular(columns, rows),
            order: vec![],
        })
    }
    pub fn triangles(&self) -> Vec<[usize; 3]> {
        let c = self.columns as usize;
        let r = self.rows as usize;
        (0..r.saturating_sub(1))
            .flat_map(|y| {
                (0..c.saturating_sub(1)).flat_map(move |x| {
                    let a = y * c + x;
                    [[a, a + 1, a + c + 1], [a, a + c + 1, a + c]]
                })
            })
            .collect()
    }
    pub fn valid(&self) -> bool {
        if !(2..=5).contains(&self.columns)
            || !(2..=5).contains(&self.rows)
            || self.points.len() != self.columns as usize * self.rows as usize
            || self
                .points
                .iter()
                .flatten()
                .any(|v| !v.is_finite() || v.abs() > 64.)
        {
            return false;
        }
        let triangles = self
            .triangles()
            .into_iter()
            .map(|t| t.map(|n| self.points[n]))
            .collect::<Vec<_>>();
        let ordered = !self.order.is_empty();
        if ordered
            && (self.order.len() != triangles.len()
                || self.order.iter().any(|p| {
                    p[0] < i32::MIN as i64 || p[0] > i32::MAX as i64 || !(0..25).contains(&p[1])
                }))
        {
            return false;
        }
        if triangles.iter().any(|t| {
            let a = cross(sub(t[1], t[0]), sub(t[2], t[0]));
            if ordered { a.abs() <= 1e-9 } else { a <= 1e-9 }
        }) {
            return false;
        }
        ordered
            || (0..triangles.len())
                .all(|n| (n + 1..triangles.len()).all(|m| !overlap(triangles[n], triangles[m])))
    }
    pub fn identity(&self) -> bool {
        (2..=5).contains(&self.columns)
            && (2..=5).contains(&self.rows)
            && self.points == regular(self.columns, self.rows)
    }
    pub fn inverse(&self, p: Point) -> Option<Point> {
        if !(2..=5).contains(&self.columns)
            || !(2..=5).contains(&self.rows)
            || self.points.len() != self.columns as usize * self.rows as usize
            || p.iter().any(|v| !v.is_finite())
        {
            return None;
        }
        let c = self.columns as usize;
        let r = self.rows as usize;
        // Index arithmetic avoids per-pixel heap allocation of the triangle list.
        let mut best: Option<([i64; 3], Point)> = None;
        for y in 0..r - 1 {
            for x in 0..c - 1 {
                let a = y * c + x;
                for (side, t) in [[a, a + 1, a + c + 1], [a, a + c + 1, a + c]]
                    .into_iter()
                    .enumerate()
                {
                    let q = t.map(|n| self.points[n]);
                    let ab = sub(q[1], q[0]);
                    let ac = sub(q[2], q[0]);
                    let v = sub(p, q[0]);
                    let den = cross(ab, ac);
                    if if self.order.is_empty() {
                        den <= 1e-9
                    } else {
                        den.abs() <= 1e-9
                    } {
                        continue;
                    }
                    let b = cross(v, ac) / den;
                    let c_weight = cross(ab, v) / den;
                    let a_weight = 1. - b - c_weight;
                    if [a_weight, b, c_weight]
                        .iter()
                        .all(|v| *v >= -1e-9 && *v <= 1. + 1e-9)
                    {
                        let source = t.map(|n| {
                            [
                                (n % c) as f64 / (c - 1) as f64,
                                (n / c) as f64 / (r - 1) as f64,
                            ]
                        });
                        let point = [0, 1].map(|k| {
                            source[0][k] * a_weight + source[1][k] * b + source[2][k] * c_weight
                        });
                        if self.order.is_empty() {
                            return Some(point);
                        }
                        let n = (y * (c - 1) + x) * 2 + side;
                        let rank = self.order.get(n)?;
                        let rank = [rank[0], rank[1], n as i64];
                        if best.as_ref().is_none_or(|(old, _)| rank > *old) {
                            best = Some((rank, point));
                        }
                    }
                }
            }
        }
        best.map(|v| v.1)
    }
    pub fn move_selected(&mut self, selected: &[usize], delta: Point) -> Result<(), String> {
        if selected.is_empty()
            || delta.iter().any(|v| !v.is_finite())
            || selected
                .iter()
                .enumerate()
                .any(|(n, v)| *v >= self.points.len() || selected[..n].contains(v))
        {
            return Err("Choose unique valid mesh vertices and a finite offset".into());
        }
        let mut next = self.clone();
        for n in selected {
            next.points[*n] = [0, 1].map(|k| self.points[*n][k] + delta[k]);
        }
        if next
            .points
            .iter()
            .flatten()
            .any(|v| !v.is_finite() || v.abs() > 64.)
        {
            return Err("Mesh point range exceeded".into());
        }
        *self = next;
        Ok(())
    }
    pub fn puppet(columns: u8, rows: u8, pins: &[Pin], area: Rect) -> Result<Self, String> {
        let mut mesh = Self::new(columns, rows)?;
        let source = mesh.points.clone();
        if pins.len() > source.len()
            || pins.iter().enumerate().any(|(n, p)| {
                p.vertex >= source.len()
                    || !p.degrees.is_finite()
                    || p.degrees.abs() > 36000.
                    || !p.reference_degrees.is_finite()
                    || p.reference_degrees.abs() > 36000.
                    || p.target.iter().any(|v| !v.is_finite() || v.abs() > 64.)
                    || pins[..n].iter().any(|q| q.vertex == p.vertex)
            })
            || area
                .min
                .iter()
                .chain(&area.max)
                .any(|v| !v.is_finite() || v.abs() > 1e6)
            || (0..2).any(|k| area.max[k] - area.min[k] <= 1e-9)
        {
            return Err(
                "Puppet pin is duplicated, outside the grid or has invalid target/rotation/bounds"
                    .into(),
            );
        }
        let angle = |p: &Pin| {
            if p.locked {
                0.
            } else if p.keep_angle {
                p.reference_degrees
            } else {
                p.degrees
            }
        };
        if pins
            .iter()
            .all(|p| (p.locked || p.target == source[p.vertex]) && angle(p) == 0.)
        {
            return Ok(mesh);
        }
        let to_doc = |p: Point| [0, 1].map(|k| area.min[k] + p[k] * (area.max[k] - area.min[k]));
        for (n, point) in source.iter().enumerate() {
            if let Some(pin) = pins.iter().find(|p| p.vertex == n) {
                mesh.points[n] = if pin.locked { source[n] } else { pin.target };
                continue;
            }
            let p = to_doc(*point);
            let mut sum = [0.; 2];
            let mut total = 0.;
            for pin in pins {
                let anchor = to_doc(source[pin.vertex]);
                let v = sub(p, anchor);
                let weight = if pins.len() == 1 {
                    1.
                } else {
                    1. / (v[0] * v[0] + v[1] * v[1]).max(1e-12)
                };
                let target = if pin.locked {
                    anchor
                } else {
                    to_doc(pin.target)
                };
                let (s, c) = crate::texture::rotation(angle(pin));
                let moved = [
                    target[0] + v[0] * c - v[1] * s,
                    target[1] + v[0] * s + v[1] * c,
                ];
                for k in 0..2 {
                    sum[k] += moved[k] * weight;
                }
                total += weight;
            }
            mesh.points[n] =
                [0, 1].map(|k| (sum[k] / total - area.min[k]) / (area.max[k] - area.min[k]));
        }
        // A triangle belongs to the closest pin in the immutable source cage.
        // Larger depth is front; equal depth uses pin creation order, then triangle order.
        mesh.order = mesh
            .triangles()
            .into_iter()
            .map(|t| {
                let center =
                    [0, 1].map(|k| t.iter().map(|n| to_doc(source[*n])[k]).sum::<f64>() / 3.);
                let (n, p) = pins
                    .iter()
                    .enumerate()
                    .min_by(|(na, a), (nb, b)| {
                        let distance = |p: &Pin| {
                            let p = to_doc(source[p.vertex]);
                            (p[0] - center[0]).powi(2) + (p[1] - center[1]).powi(2)
                        };
                        distance(a).total_cmp(&distance(b)).then_with(|| nb.cmp(na))
                    })
                    .expect("Nonidentity puppet has pins");
                [p.depth as i64, n as i64]
            })
            .collect();
        Ok(mesh)
    }
}
