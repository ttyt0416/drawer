//! Small affine world-space math; retains shear from nonuniform parent transforms.
use serde::{Deserialize, Serialize};
pub type V = [f64; 3];
pub fn add(a: V, b: V) -> V {
    std::array::from_fn(|i| a[i] + b[i])
}
pub fn sub(a: V, b: V) -> V {
    std::array::from_fn(|i| a[i] - b[i])
}
pub fn mul(a: V, k: f64) -> V {
    a.map(|n| n * k)
}
pub fn dot(a: V, b: V) -> f64 {
    a.into_iter().zip(b).map(|(a, b)| a * b).sum()
}
pub fn cross(a: V, b: V) -> V {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
pub fn unit(a: V) -> V {
    mul(a, 1. / dot(a, a).sqrt())
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Matrix(pub [[f64; 4]; 3]);
impl Default for Matrix {
    fn default() -> Self {
        Self([[1., 0., 0., 0.], [0., 1., 0., 0.], [0., 0., 1., 0.]])
    }
}
impl Matrix {
    pub fn vector(self, v: V) -> V {
        self.0.map(|r| dot([r[0], r[1], r[2]], v))
    }
    pub fn map(self, v: V) -> V {
        add(self.vector(v), self.origin())
    }
    pub fn origin(self) -> V {
        self.0.map(|r| r[3])
    }
    pub fn axis(self, n: usize) -> V {
        unit(self.0.map(|r| r[n]))
    }
    pub fn translation(v: V) -> Self {
        let mut m = Self::default();
        for (r, v) in m.0.iter_mut().zip(v) {
            r[3] = v;
        }
        m
    }
    pub fn scale(v: V) -> Self {
        let mut m = Self::default();
        for (i, v) in v.into_iter().enumerate() {
            m.0[i][i] = v;
        }
        m
    }
    /// Apply self, then next. All twelve affine components are retained.
    pub fn then(self, next: Self) -> Self {
        let mut m = Self([[0.; 4]; 3]);
        for i in 0..3 {
            for j in 0..3 {
                m.0[i][j] = (0..3).map(|k| next.0[i][k] * self.0[k][j]).sum();
            }
            m.0[i][3] = next.map(self.origin())[i];
        }
        m
    }
    pub fn determinant(self) -> f64 {
        dot(
            self.0[0][..3].try_into().unwrap(),
            cross(
                self.0[1][..3].try_into().unwrap(),
                self.0[2][..3].try_into().unwrap(),
            ),
        )
    }
    pub fn valid(self) -> bool {
        self.0
            .iter()
            .flatten()
            .all(|n| n.is_finite() && n.abs() <= 1e9)
            && self.determinant().abs() > 1e-18
    }
    pub fn inverse(self) -> Self {
        let c: [V; 3] = std::array::from_fn(|i| self.0.map(|r| r[i]));
        let d = self.determinant();
        let rows = [
            mul(cross(c[1], c[2]), 1. / d),
            mul(cross(c[2], c[0]), 1. / d),
            mul(cross(c[0], c[1]), 1. / d),
        ];
        let mut m = Self(rows.map(|r| [r[0], r[1], r[2], 0.]));
        let p = mul(m.vector(self.origin()), -1.);
        for (r, v) in m.0.iter_mut().zip(p) {
            r[3] = v;
        }
        m
    }
    pub fn rotation(axis: V, degrees: f64) -> Self {
        let [x, y, z] = unit(axis);
        let (s, c) = degrees.to_radians().sin_cos();
        let t = 1. - c;
        Self([
            [t * x * x + c, t * x * y - s * z, t * x * z + s * y, 0.],
            [t * x * y + s * z, t * y * y + c, t * y * z - s * x, 0.],
            [t * x * z - s * y, t * y * z + s * x, t * z * z + c, 0.],
        ])
    }
    pub fn euler(v: V) -> Self {
        (0..3).fold(Self::default(), |m, i| {
            m.then(Self::rotation(Self::default().axis(i), v[i]))
        })
    }
    pub fn around(self, p: V) -> Self {
        Self::translation(mul(p, -1.))
            .then(self)
            .then(Self::translation(p))
    }
    pub fn basis(self) -> Self {
        let mut b = Self::default();
        for i in 0..3 {
            let a = self.axis(i);
            for (j, v) in a.into_iter().enumerate() {
                b.0[j][i] = v;
            }
        }
        b
    }
}
