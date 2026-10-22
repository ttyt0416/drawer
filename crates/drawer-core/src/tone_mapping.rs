//! Local RGB curves and luminance gradient maps; alpha and geometry stay unchanged.
use crate::{document::Document, gradient_ramp::Ramp, scene::Point, sizing::DocumentState};
#[derive(Clone, Debug, PartialEq)]
pub struct Curve {
    pub points: Vec<Point>,
    pub smooth: bool,
}
impl Default for Curve {
    fn default() -> Self {
        Self {
            points: vec![[0., 0.], [1., 1.]],
            smooth: true,
        }
    }
}
impl Curve {
    pub fn valid(&self) -> bool {
        (2..=32).contains(&self.points.len())
            && self.points[0][0] == 0.
            && self.points.last().unwrap()[0] == 1.
            && self
                .points
                .iter()
                .flatten()
                .all(|v| v.is_finite() && (0. ..=1.).contains(v))
            && self.points.windows(2).all(|p| p[1][0] - p[0][0] >= 1e-9)
    }
    fn identity(&self) -> bool {
        self.points.iter().all(|p| p[0] == p[1])
    }
    pub fn sample(&self, t: f64) -> Option<f64> {
        (self.valid() && t.is_finite()).then(|| Compiled::new(self).sample(t))
    }
}
struct Compiled {
    points: Vec<Point>,
    slopes: Vec<f64>,
    smooth: bool,
}
impl Compiled {
    fn new(c: &Curve) -> Self {
        let deltas = c
            .points
            .windows(2)
            .map(|w| (w[1][1] - w[0][1]) / (w[1][0] - w[0][0]))
            .collect::<Vec<_>>();
        let mut slopes = vec![0.; c.points.len()];
        slopes[0] = deltas[0];
        *slopes.last_mut().unwrap() = *deltas.last().unwrap();
        for i in 1..slopes.len() - 1 {
            let (a, b) = (deltas[i - 1], deltas[i]);
            if a * b > 0. {
                let h0 = c.points[i][0] - c.points[i - 1][0];
                let h1 = c.points[i + 1][0] - c.points[i][0];
                let w0 = 2. * h1 + h0;
                let w1 = h1 + 2. * h0;
                slopes[i] = (w0 + w1) / (w0 / a + w1 / b);
            }
        }
        for (i, d) in deltas.iter().enumerate() {
            if *d == 0. {
                slopes[i] = 0.;
                slopes[i + 1] = 0.;
            } else {
                let a = slopes[i] / d;
                let b = slopes[i + 1] / d;
                let length = a.hypot(b);
                if length > 3. {
                    slopes[i] *= 3. / length;
                    slopes[i + 1] *= 3. / length;
                }
            }
        }
        Self {
            points: c.points.clone(),
            slopes,
            smooth: c.smooth,
        }
    }
    fn sample(&self, t: f64) -> f64 {
        let t = t.clamp(0., 1.);
        let i = self
            .points
            .partition_point(|p| p[0] <= t)
            .saturating_sub(1)
            .min(self.points.len() - 2);
        let (a, b) = (self.points[i], self.points[i + 1]);
        let h = b[0] - a[0];
        let x = ((t - a[0]) / h).clamp(0., 1.);
        if !self.smooth || self.points.len() == 2 {
            return a[1] + (b[1] - a[1]) * x;
        }
        ((2. * x * x * x - 3. * x * x + 1.) * a[1]
            + (x * x * x - 2. * x * x + x) * h * self.slopes[i]
            + (-2. * x * x * x + 3. * x * x) * b[1]
            + (x * x * x - x * x) * h * self.slopes[i + 1])
            .clamp(a[1].min(b[1]), a[1].max(b[1]))
    }
}
#[derive(Clone, Debug, PartialEq)]
pub enum Operation {
    Curves([Curve; 4]),
    Gradient(Ramp),
}
impl Operation {
    pub fn identity_curves() -> Self {
        Self::Curves(std::array::from_fn(|_| Curve::default()))
    }
}
pub fn prepare(
    d: &Document,
    layer: usize,
    selection: bool,
    op: &Operation,
) -> Result<DocumentState, String> {
    crate::layer_properties::paintable(d, layer)?;
    let source = d.layers.get(layer).ok_or("Choose a raster layer")?;
    if source.pixels.len() > 4 * 1024 * 1024
        || d.memory_bytes() > crate::document::DEFAULT_DOCUMENT_BUDGET / 8
    {
        return Err("Tone mapping snapshot/pixel limit4M".into());
    }
    let (mut curves, mut ramp) = (None, None);
    match op {
        Operation::Curves(c) => {
            if c.iter().any(|c| !c.valid()) {
                return Err("Curves require2–32 finite points, endpoints X=0/1, and strictly increasing input positions; overlapping points cannot be applied".into());
            }
            if c.iter().all(Curve::identity) {
                return Ok(d.snapshot());
            }
            curves = Some(c.each_ref().map(Compiled::new));
        }
        Operation::Gradient(r) => {
            if !r.valid() {
                return Err("Invalid local gradient ramp".into());
            }
            let mut r = r.clone();
            for s in &mut r.stops {
                s.color[3] = 255;
            }
            ramp = Some(r);
        }
    }
    let space = crate::color_profile::WorkingSpace::new(&d.color.profile)?;
    let extent = source.extent(d.width, d.height);
    let mut state = d.snapshot();
    let output = &mut state.layers[layer];
    for (i, p) in source.pixels.iter().enumerate() {
        let original = source.sample_at(i);
        if original.alpha() == 0. {
            continue;
        }
        let pos = [
            extent.offset[0] + (i % extent.width as usize) as i64,
            extent.offset[1] + (i / extent.width as usize) as i64,
        ];
        let weight = if selection {
            d.scene.selection_coverage(pos.map(|v| v as f64 + 0.5)) as f64 / 255.
        } else {
            1.
        };
        if weight == 0. {
            continue;
        }
        let input = space.to_srgb(original)?;
        let rgb = input.rgb();
        let rgb = if let Some(c) = &curves {
            std::array::from_fn(|k| c[k + 1].sample(c[0].sample(rgb[k])))
        } else {
            let rgb = ramp
                .as_ref()
                .unwrap()
                .sample((0.2126 * rgb[0] + 0.7152 * rgb[1] + 0.0722 * rgb[2]).clamp(0., 1.));
            [rgb[0] / 255., rgb[1] / 255., rgb[2] / 255.]
        };
        let mapped = space.from_srgb(input.with_rgb(rgb)?)?;
        let result = if weight == 1. {
            mapped
        } else {
            original.mix(mapped, weight)?
        };
        let mut stored = result.store(d.color.depth, d.color.dither, pos);
        if stored == original.store(d.color.depth, d.color.dither, pos) {
            continue;
        }
        if d.color.depth == crate::color_precision::Depth::Sixteen && output.precise.is_none() {
            output.precise = Some(
                (0..source.pixels.len())
                    .map(|i| source.sample_at(i).rgba16())
                    .collect(),
            );
        }
        if let Some(high) = &mut output.precise {
            stored[3] = original.rgba16()[3];
            high[i] = stored;
            output.pixels[i] = crate::color_precision::Sample::from16(stored).rgba8();
        } else {
            output.pixels[i] = stored.map(|v| v as u8);
        }
        output.pixels[i][3] = p[3];
    }
    crate::layer_groups::finish(d, &state)?;
    Ok(state)
}
