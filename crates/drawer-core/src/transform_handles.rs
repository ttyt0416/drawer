//! Independent document-coordinate transform handles and angle snapping.
use crate::{
    scene::{Affine, Point, Rect},
    vector_scope::Transform,
};
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Snap {
    pub enabled: bool,
    pub degrees: u8,
}
impl Default for Snap {
    fn default() -> Self {
        Self {
            enabled: false,
            degrees: 15,
        }
    }
}
impl Snap {
    pub fn valid(self) -> bool {
        (1..=90).contains(&self.degrees)
    }
    pub fn angle(self, angle: f64) -> Result<f64, String> {
        if !self.valid() || !angle.is_finite() {
            return Err("Angle snapping requires an interval from1 to90 degrees".into());
        }
        Ok((if self.enabled {
            (angle / self.degrees as f64).round() * self.degrees as f64
        } else {
            angle
        })
        .rem_euclid(360.))
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum Record {
    Affine {
        degrees: f64,
        matrix: Affine,
    },
    Quad {
        points: [Point; 4],
        area: Rect,
        perspective: bool,
    },
}
impl Record {
    pub fn valid(self) -> bool {
        match self {
            Self::Affine { degrees, matrix } => {
                degrees.is_finite()
                    && degrees.abs() <= 36000.
                    && matrix.0.iter().all(|v| v.is_finite() && v.abs() <= 1e14)
                    && (matrix.0[0] * matrix.0[3] - matrix.0[1] * matrix.0[2]).abs() > 0.
            }
            Self::Quad {
                points,
                area,
                perspective,
            } => {
                area.valid()
                    && (0..2).all(|n| area.max[n] > area.min[n])
                    && if perspective {
                        crate::material_mapping::Warp::Perspective(points).valid()
                    } else {
                        crate::material_mapping::Warp::Distort(points).valid()
                    }
            }
        }
    }
}
pub fn corners(r: Rect) -> [Point; 4] {
    [r.min, [r.max[0], r.min[1]], r.max, [r.min[0], r.max[1]]]
}
fn midpoint(a: Point, b: Point) -> Point {
    [0, 1].map(|n| (a[n] + b[n]) * 0.5)
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Handle {
    Move,
    Corner(usize),
    Edge(usize),
    Rotate,
}
pub fn pick(bounds: Rect, t: Transform, p: Point, radius: f64) -> Result<Handle, String> {
    let matrix = t.matrix()?;
    let q = corners(bounds).map(|p| matrix.map(p));
    let near = |a: Point| (a[0] - p[0]).hypot(a[1] - p[1]) <= radius;
    for (n, a) in q.iter().enumerate() {
        if near(*a) {
            return Ok(Handle::Corner(n));
        }
    }
    for n in 0..4 {
        if near(midpoint(q[n], q[(n + 1) % 4])) {
            return Ok(Handle::Edge(n));
        }
    }
    Ok(if bounds.contains(matrix.inverse().map(p)) {
        Handle::Move
    } else {
        Handle::Rotate
    })
}
#[derive(Clone, Copy)]
pub struct Drag {
    pub handle: Handle,
    pub bounds: Rect,
    pub source: Transform,
    pub start: Point,
}
impl Drag {
    pub fn update(self, p: Point, aspect: bool, snap: Snap) -> Result<Transform, String> {
        if !self.bounds.valid()
            || (0..2).any(|n| self.bounds.max[n] - self.bounds.min[n] < 1e-12)
            || p.iter()
                .chain(&self.start)
                .any(|v| !v.is_finite() || v.abs() > 1e9)
        {
            return Err("Invalid transform box/pointer".into());
        }
        self.source.matrix()?;
        if p == self.start {
            return Ok(self.source);
        }
        let mut out = self.source;
        match self.handle {
            Handle::Move => {
                out.translation = [0, 1].map(|n| self.source.translation[n] + p[n] - self.start[n]);
            }
            Handle::Rotate => {
                let center = [0, 1].map(|n| self.source.pivot[n] + self.source.translation[n]);
                let angle = |v: Point| (v[1] - center[1]).atan2(v[0] - center[0]);
                out.degrees =
                    snap.angle(self.source.degrees + (angle(p) - angle(self.start)).to_degrees())?;
            }
            handle => {
                let q = corners(self.bounds);
                let (a, b) = match handle {
                    Handle::Corner(n) if n < 4 => (q[(n + 2) % 4], q[n]),
                    Handle::Edge(n) if n < 4 => (
                        midpoint(q[(n + 2) % 4], q[(n + 3) % 4]),
                        midpoint(q[n], q[(n + 1) % 4]),
                    ),
                    _ => return Err("Invalid transform handle".into()),
                };
                let anchor = self.source.matrix()?.map(a);
                let handle_point = self.source.matrix()?.map(b);
                let p = [0, 1].map(|n| p[n] + handle_point[n] - self.start[n]);
                let v = [p[0] - anchor[0], p[1] - anchor[1]];
                let (s, c) = self.source.degrees.to_radians().sin_cos();
                let local = [c * v[0] + s * v[1], -s * v[0] + c * v[1]];
                let extent = [b[0] - a[0], b[1] - a[1]];
                match handle {
                    Handle::Corner(_) if aspect => {
                        let original = [0, 1].map(|n| extent[n] * self.source.scale[n]);
                        let factor = (local[0] * original[0] + local[1] * original[1])
                            / (original[0] * original[0] + original[1] * original[1]);
                        out.scale = self.source.scale.map(|v| v * factor);
                    }
                    Handle::Corner(_) => {
                        out.scale = [0, 1].map(|n| local[n] / extent[n]);
                    }
                    Handle::Edge(n) => {
                        let axis = usize::from(n % 2 == 1);
                        let axis = 1 - axis;
                        out.scale[axis] = local[axis] / extent[axis];
                    }
                    _ => unreachable!(),
                }
                let v = [0, 1].map(|n| (a[n] - out.pivot[n]) * out.scale[n]);
                let mapped = [
                    c * v[0] - s * v[1] + out.pivot[0],
                    s * v[0] + c * v[1] + out.pivot[1],
                ];
                out.translation = [0, 1].map(|n| anchor[n] - mapped[n]);
            }
        }
        out.matrix()?;
        Ok(out)
    }
}
