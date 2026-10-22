//! Local statistical transfer and user-defined tonal lighting; no model or network input.
use crate::{
    color_precision::Sample, document::Document, references::LocalImage, sizing::DocumentState,
};
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum Correction {
    Distribution {
        matrix: [[f64; 3]; 3],
        bias: [f64; 3],
        strength: f64,
    },
    Lighting {
        light: [f64; 3],
        shadow: [f64; 3],
        strength: f64,
    },
}
impl Correction {
    pub fn lighting() -> Self {
        Self::Lighting {
            light: [0.5; 3],
            shadow: [0.5; 3],
            strength: 1.,
        }
    }
    pub fn strength(&mut self) -> &mut f64 {
        match self {
            Self::Distribution { strength, .. } | Self::Lighting { strength, .. } => strength,
        }
    }
    pub fn valid(self) -> bool {
        match self {
            Self::Distribution {
                matrix,
                bias,
                strength,
            } => {
                (0. ..=1.).contains(&strength)
                    && matrix
                        .iter()
                        .flatten()
                        .chain(bias.iter())
                        .all(|v| v.is_finite() && v.abs() <= 4096.)
            }
            Self::Lighting {
                light,
                shadow,
                strength,
            } => {
                (0. ..=1.).contains(&strength)
                    && light
                        .iter()
                        .chain(shadow.iter())
                        .all(|v| (0. ..=1.).contains(v))
            }
        }
    }
    pub fn identity(self) -> bool {
        match self {
            Self::Distribution { strength, .. } => strength == 0.,
            Self::Lighting {
                light,
                shadow,
                strength,
            } => strength == 0. || light == [0.5; 3] && shadow == [0.5; 3],
        }
    }
    pub fn rgb(self, rgb: [f64; 3]) -> [f64; 3] {
        if self.identity() {
            return rgb;
        }
        let (mapped, strength): ([f64; 3], f64) = match self {
            Self::Distribution {
                matrix,
                bias,
                strength,
            } => (
                std::array::from_fn(|i| {
                    (0..3).map(|j| matrix[i][j] * rgb[j]).sum::<f64>() + bias[i]
                }),
                strength,
            ),
            Self::Lighting {
                light,
                shadow,
                strength,
            } => {
                let l = (rgb[0] * 0.2126 + rgb[1] * 0.7152 + rgb[2] * 0.0722).clamp(0., 1.);
                (
                    std::array::from_fn(|i| rgb[i] * 2. * (shadow[i] * (1. - l) + light[i] * l)),
                    strength,
                )
            }
        };
        std::array::from_fn(|i| {
            (rgb[i] + strength * (mapped[i].clamp(0., 1.) - rgb[i])).clamp(0., 1.)
        })
    }
}
#[derive(Default)]
struct Stats {
    weight: f64,
    mean: [f64; 3],
    sum: [[f64; 3]; 3],
}
impl Stats {
    fn add(&mut self, s: Sample) {
        let weight = s.alpha();
        if weight == 0. {
            return;
        }
        let rgb = s.rgb();
        let delta: [f64; 3] = std::array::from_fn(|i| rgb[i] - self.mean[i]);
        self.weight += weight;
        for (i, d) in delta.iter().enumerate() {
            self.mean[i] += d * weight / self.weight;
        }
        for (i, row) in self.sum.iter_mut().enumerate() {
            for (j, v) in row.iter_mut().enumerate() {
                *v += weight * delta[i] * (rgb[j] - self.mean[j]);
            }
        }
    }
    fn covariance(&self) -> Result<[[f64; 3]; 3], String> {
        if self.weight == 0. {
            return Err(
                "No visible color samples; choose a nontransparent reference/backdrop".into(),
            );
        }
        Ok(self.sum.map(|row| row.map(|v| v / self.weight)))
    }
}
// Jacobi decomposition of a symmetric 3x3 covariance, including singular gray/flat inputs.
fn root(mut a: [[f64; 3]; 3], inverse: bool) -> [[f64; 3]; 3] {
    let mut vectors = [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]];
    for _ in 0..24 {
        for (p, q) in [(0, 1), (0, 2), (1, 2)] {
            if a[p][q].abs() < 1e-15 {
                continue;
            }
            let angle = 0.5 * (2. * a[p][q]).atan2(a[q][q] - a[p][p]);
            let (s, c) = angle.sin_cos();
            let pp = c * c * a[p][p] - 2. * s * c * a[p][q] + s * s * a[q][q];
            let qq = s * s * a[p][p] + 2. * s * c * a[p][q] + c * c * a[q][q];
            for k in [0, 1, 2] {
                if k != p && k != q {
                    let ap = a[k][p];
                    let aq = a[k][q];
                    a[k][p] = c * ap - s * aq;
                    a[p][k] = a[k][p];
                    a[k][q] = s * ap + c * aq;
                    a[q][k] = a[k][q];
                }
            }
            a[p][p] = pp;
            a[q][q] = qq;
            a[p][q] = 0.;
            a[q][p] = 0.;
            for row in &mut vectors {
                let vp = row[p];
                let vq = row[q];
                row[p] = c * vp - s * vq;
                row[q] = s * vp + c * vq;
            }
        }
    }
    let values: [f64; 3] = std::array::from_fn(|i| {
        if inverse {
            if a[i][i] > 1e-8 {
                1. / a[i][i].sqrt()
            } else {
                0.
            }
        } else {
            a[i][i].max(0.).sqrt()
        }
    });
    std::array::from_fn(|i| {
        std::array::from_fn(|j| {
            (0..3)
                .map(|k| vectors[i][k] * values[k] * vectors[j][k])
                .sum()
        })
    })
}
pub fn distribution(
    d: &Document,
    reference: &LocalImage,
    strength: f64,
    mut tick: impl FnMut(usize, usize) -> Result<(), String>,
) -> Result<Correction, String> {
    if !reference.valid()
        || reference.pixels.len() > 4 * 1024 * 1024
        || !(0. ..=1.).contains(&strength)
        || d.memory_bytes() > 32 * 1024 * 1024
    {
        return Err("Reference/color snapshot exceeds limits or invalid strength".into());
    }
    tick(0, 100)?;
    let mut target = Stats::default();
    for (i, p) in reference.pixels.iter().enumerate() {
        if i.is_multiple_of(128) {
            tick(i * 40 / reference.pixels.len(), 100)?;
        }
        target.add(Sample::from8(*p));
    }
    let target_root = root(target.covariance()?, false);
    let renderer =
        crate::float_render::Renderer::with_effect_progress(d, None, &mut |_, _| tick(40, 100))?;
    let space = crate::color_profile::WorkingSpace::new(&d.color.profile)?;
    let nodes = d.scene.paper.root_nodes(d.layers.len());
    let limit =
        (4 * 1024 * 1024 / (d.layers.len() + d.scene.paper.folders.len()).max(1)).min(65536);
    let nx = (d.width as usize)
        .min((limit as f64).sqrt() as usize)
        .max(1);
    let ny = (d.height as usize).min(limit / nx).max(1);
    let mut source = Stats::default();
    for y in 0..ny {
        for x in 0..nx {
            let i = y * nx + x;
            if i.is_multiple_of(128) {
                tick(40 + i * 55 / (nx * ny), 100)?;
            }
            let point = [
                ((x as f64 + 0.5) * d.width as f64 / nx as f64).floor() as i64,
                ((y as f64 + 0.5) * d.height as f64 / ny as f64).floor() as i64,
            ];
            source.add(space.to_srgb(renderer.nodes_sample(&nodes, point)?)?);
        }
    }
    let inverse = root(source.covariance()?, true);
    let matrix = std::array::from_fn(|i| {
        std::array::from_fn(|j| (0..3).map(|k| target_root[i][k] * inverse[k][j]).sum())
    });
    let bias = std::array::from_fn(|i| {
        target.mean[i] - (0..3).map(|j| matrix[i][j] * source.mean[j]).sum::<f64>()
    });
    let value = Correction::Distribution {
        matrix,
        bias,
        strength,
    };
    if !value.valid() {
        return Err("Color distribution is numerically unstable; use another reference".into());
    }
    tick(100, 100)?;
    Ok(value)
}
/// New root correction affects the measured raster backdrop; page objects/timeline remain separate.
pub fn prepare(d: &Document, value: Correction) -> Result<DocumentState, String> {
    if !value.valid() {
        return Err("Invalid local color correction".into());
    }
    if value.identity() {
        return Ok(d.snapshot());
    }
    crate::adjustments::prepare(
        d,
        crate::adjustments::Destination::New {
            name: match value {
                Correction::Distribution { .. } => "Local color distribution",
                Correction::Lighting { .. } => "Local light and shadow",
            }
            .into(),
            parent: None,
        },
        crate::layer_effects::Adjustment {
            local: Some(value),
            ..Default::default()
        },
    )
    .map(|p| p.state)
}
pub fn active(scene: &crate::scene::Scene) -> bool {
    scene
        .layer_effects
        .rasters
        .values()
        .any(|e| e.adjustment.is_some_and(|a| a.local.is_some()))
}
