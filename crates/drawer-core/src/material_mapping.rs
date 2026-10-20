//! Coordinate-only image tiling and invertible normalized material deformations.
use crate::scene::{Point, Rect};
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Tiling {
    #[default]
    Repeat,
    Reverse,
    Flip,
}
impl Tiling {
    pub const ALL: [Self; 3] = [Self::Repeat, Self::Reverse, Self::Flip];
    pub fn label(self) -> &'static str {
        match self {
            Self::Repeat => "Repeat",
            Self::Reverse => "Reverse",
            Self::Flip => "Flip",
        }
    }
    pub fn pixel(self, p: Point, size: [u32; 2], repeat: [bool; 2]) -> Option<[usize; 2]> {
        let mut q = [0; 2];
        let mut tile = [0i64; 2];
        for n in 0..2 {
            let v = p[n].floor();
            if !v.is_finite() || v.abs() > 1e12 || size[n] == 0 {
                return None;
            }
            let v = v as i64;
            let size = i64::from(size[n]);
            if repeat[n] {
                tile[n] = v.div_euclid(size);
                q[n] = v.rem_euclid(size) as usize;
            } else if (0..size).contains(&v) {
                q[n] = v as usize;
            } else {
                return None;
            }
        }
        for n in 0..2 {
            let flip = match self {
                Self::Repeat => false,
                Self::Reverse => tile[n].rem_euclid(2) == 1,
                Self::Flip => (tile[0] + tile[1]).rem_euclid(2) == 1,
            };
            if flip {
                q[n] = size[n] as usize - 1 - q[n];
            }
        }
        Some(q)
    }
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum Warp {
    #[default]
    None,
    Distort([Point; 4]),
    Perspective([Point; 4]),
    Mesh {
        columns: u8,
        rows: u8,
        points: Vec<Point>,
    },
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Mapping {
    pub tiling: Tiling,
    pub warp: Warp,
}
impl Mapping {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
    pub fn valid(&self, area: Rect) -> bool {
        (matches!(self.warp, Warp::None) || (0..2).all(|n| area.max[n] - area.min[n] > 1e-9))
            && self.warp.valid()
    }
    pub fn bytes(&self) -> usize {
        if let Warp::Mesh { points, .. } = &self.warp {
            points.len() * 16
        } else {
            0
        }
    }
}
pub const UNIT: [Point; 4] = [[0., 0.], [1., 0.], [1., 1.], [0., 1.]];
fn cross(a: Point, b: Point) -> f64 {
    a[0] * b[1] - a[1] * b[0]
}
fn sub(a: Point, b: Point) -> Point {
    [a[0] - b[0], a[1] - b[1]]
}
fn quad_valid(q: &[Point; 4]) -> bool {
    q.iter().flatten().all(|v| v.is_finite() && v.abs() <= 64.)
        && (0..4).all(|n| {
            cross(
                sub(q[(n + 1) % 4], q[n]),
                sub(q[(n + 2) % 4], q[(n + 1) % 4]),
            ) > 1e-9
        })
}
fn bilinear(q: &[Point; 4], [u, v]: Point) -> Point {
    [0, 1].map(|n| {
        q[0][n] * (1. - u) * (1. - v)
            + q[1][n] * u * (1. - v)
            + q[2][n] * u * v
            + q[3][n] * (1. - u) * v
    })
}
fn inverse_bilinear(q: &[Point; 4], p: Point) -> Option<Point> {
    let mut uv = [0.5; 2];
    for _ in 0..16 {
        let [u, v] = uv;
        let value = bilinear(q, uv);
        let error = sub(value, p);
        if error[0].abs().max(error[1].abs()) < 1e-10 {
            return (uv.iter().all(|v| (-1e-8..=1. + 1e-8).contains(v)))
                .then(|| uv.map(|v| v.clamp(0., 1.)));
        }
        let du = [0, 1].map(|n| (q[1][n] - q[0][n]) * (1. - v) + (q[2][n] - q[3][n]) * v);
        let dv = [0, 1].map(|n| (q[3][n] - q[0][n]) * (1. - u) + (q[2][n] - q[1][n]) * u);
        let determinant = cross(du, dv);
        if determinant.abs() < 1e-12 {
            return None;
        }
        uv[0] -= cross(error, dv) / determinant;
        uv[1] -= cross(du, error) / determinant;
        if uv.iter().any(|v| !v.is_finite() || v.abs() > 1e6) {
            return None;
        }
    }
    None
}
fn homography(q: &[Point; 4]) -> Option<[f64; 9]> {
    let a = sub(q[1], q[2]);
    let b = sub(q[3], q[2]);
    let c = [0, 1].map(|n| q[0][n] - q[1][n] + q[2][n] - q[3][n]);
    let determinant = cross(a, b);
    if determinant.abs() < 1e-12 {
        return None;
    }
    let g = cross(c, b) / determinant;
    let h = cross(a, c) / determinant;
    if [1., 1. + g, 1. + h, 1. + g + h].iter().any(|v| *v <= 1e-9) {
        return None;
    }
    Some([
        q[1][0] - q[0][0] + g * q[1][0],
        q[3][0] - q[0][0] + h * q[3][0],
        q[0][0],
        q[1][1] - q[0][1] + g * q[1][1],
        q[3][1] - q[0][1] + h * q[3][1],
        q[0][1],
        g,
        h,
        1.,
    ])
}
fn project(m: [f64; 9], p: Point) -> Option<Point> {
    let w = m[6] * p[0] + m[7] * p[1] + m[8];
    if w.abs() < 1e-12 {
        return None;
    }
    Some([
        (m[0] * p[0] + m[1] * p[1] + m[2]) / w,
        (m[3] * p[0] + m[4] * p[1] + m[5]) / w,
    ])
}
fn inverse(m: [f64; 9]) -> Option<[f64; 9]> {
    let a = [
        m[4] * m[8] - m[5] * m[7],
        m[2] * m[7] - m[1] * m[8],
        m[1] * m[5] - m[2] * m[4],
        m[5] * m[6] - m[3] * m[8],
        m[0] * m[8] - m[2] * m[6],
        m[2] * m[3] - m[0] * m[5],
        m[3] * m[7] - m[4] * m[6],
        m[1] * m[6] - m[0] * m[7],
        m[0] * m[4] - m[1] * m[3],
    ];
    let det = m[0] * a[0] + m[1] * a[3] + m[2] * a[6];
    (det.abs() > 1e-12).then(|| a.map(|v| v / det))
}
fn cell(points: &[Point], columns: usize, x: usize, y: usize) -> [Point; 4] {
    let at = y * columns + x;
    [
        points[at],
        points[at + 1],
        points[at + columns + 1],
        points[at + columns],
    ]
}
impl Warp {
    pub fn valid(&self) -> bool {
        match self {
            Self::None => true,
            Self::Distort(q) => quad_valid(q),
            Self::Perspective(q) => quad_valid(q) && homography(q).and_then(inverse).is_some(),
            Self::Mesh {
                columns,
                rows,
                points,
            } => {
                let (c, r) = (*columns as usize, *rows as usize);
                (2..=5).contains(&c)
                    && (2..=5).contains(&r)
                    && points.len() == c * r
                    && (0..r - 1).all(|y| (0..c - 1).all(|x| quad_valid(&cell(points, c, x, y))))
                    && (0..r).all(|y| {
                        (0..c - 1).all(|x| points[y * c + x][0] < points[y * c + x + 1][0])
                    })
                    && (0..c).all(|x| {
                        (0..r - 1).all(|y| points[y * c + x][1] < points[(y + 1) * c + x][1])
                    })
            }
        }
    }
    pub fn points(&self) -> Vec<Point> {
        match self {
            Self::None => UNIT.to_vec(),
            Self::Distort(q) | Self::Perspective(q) => q.to_vec(),
            Self::Mesh { points, .. } => points.clone(),
        }
    }
    pub fn forward(&self, p: Point) -> Option<Point> {
        match self {
            Self::None => Some(p),
            Self::Distort(q) => Some(bilinear(q, p)),
            Self::Perspective(q) => project(homography(q)?, p),
            Self::Mesh {
                columns,
                rows,
                points,
            } => {
                let (c, r) = (*columns as usize, *rows as usize);
                if c < 2 || r < 2 || points.len() != c * r {
                    return None;
                }
                let uv = [p[0] * (c - 1) as f64, p[1] * (r - 1) as f64];
                let x = (uv[0].floor().max(0.) as usize).min(c - 2);
                let y = (uv[1].floor().max(0.) as usize).min(r - 2);
                Some(bilinear(
                    &cell(points, c, x, y),
                    [uv[0] - x as f64, uv[1] - y as f64],
                ))
            }
        }
    }
    pub fn inverse(&self, p: Point) -> Option<Point> {
        let inside = |p: Point| p.iter().all(|v| (-1e-8..=1. + 1e-8).contains(v));
        match self {
            Self::None => Some(p),
            Self::Distort(q) => inverse_bilinear(q, p),
            Self::Perspective(q) => {
                let uv = project(inverse(homography(q)?)?, p)?;
                inside(uv).then_some(uv)
            }
            Self::Mesh {
                columns,
                rows,
                points,
            } => {
                let (c, r) = (*columns as usize, *rows as usize);
                if c < 2 || r < 2 || points.len() != c * r {
                    return None;
                }
                for y in 0..r - 1 {
                    for x in 0..c - 1 {
                        if let Some(uv) = inverse_bilinear(&cell(points, c, x, y), p) {
                            return Some([
                                (x as f64 + uv[0]) / (c - 1) as f64,
                                (y as f64 + uv[1]) / (r - 1) as f64,
                            ]);
                        }
                    }
                }
                None
            }
        }
    }
}
pub fn requires_v63(s: &crate::scene::Scene) -> bool {
    s.objects
        .iter()
        .filter_map(|o| {
            if let crate::scene::PageObject::Material(m) = o {
                Some(m.as_ref())
            } else {
                None
            }
        })
        .chain(s.file_objects.entries.iter().map(|l| &l.cache))
        .any(|m| !m.mapping.is_default())
}
