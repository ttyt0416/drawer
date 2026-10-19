//! Local editable gradients. Frozen coverage prevents edits from sampling their own output.
use crate::{
    bucket,
    document::{DEFAULT_DOCUMENT_BUDGET, Document},
    scene::{Affine, Point},
    selection::{Mask, Run},
    sizing::DocumentState,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Shape {
    #[default]
    StraightLine,
    Circle,
    Ellipse,
    Cone,
    Freeform,
}
impl Shape {
    pub const ALL: [Self; 5] = [
        Self::StraightLine,
        Self::Circle,
        Self::Ellipse,
        Self::Cone,
        Self::Freeform,
    ];
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Repeat {
    #[default]
    DoNotRepeat,
    Repeat,
    Reverse,
    NotDraw,
}
impl Repeat {
    pub const ALL: [Self; 4] = [
        Self::DoNotRepeat,
        Self::Repeat,
        Self::Reverse,
        Self::NotDraw,
    ];
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Cover {
    #[default]
    Keep,
    Gradient,
    DrawingColor,
}
impl Cover {
    pub const ALL: [Self; 3] = [Self::Keep, Self::Gradient, Self::DrawingColor];
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum VectorFill {
    #[default]
    Keep,
    UpToIntersection,
    WholeLine,
}
impl VectorFill {
    pub const ALL: [Self; 3] = [Self::Keep, Self::UpToIntersection, Self::WholeLine];
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ramp: Option<crate::gradient_ramp::Ramp>,
    pub shape: Shape,
    pub repeat: Repeat,
    pub start: Point,
    pub end: Point,
    pub first: [u8; 4],
    pub last: [u8; 4],
    pub drawing: [u8; 4],
    pub ellipse_ratio: f64,
    pub dithering: bool,
    pub black: Cover,
    pub transparency: Cover,
    pub exclude_color: Option<[u8; 4]>,
    pub vectors: VectorFill,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            ramp: None,
            shape: Shape::StraightLine,
            repeat: Repeat::DoNotRepeat,
            start: [0.5, 0.5],
            end: [128.5, 0.5],
            first: [0, 0, 0, 255],
            last: [255; 4],
            drawing: [0, 0, 0, 255],
            ellipse_ratio: 1.0,
            dithering: false,
            black: Cover::Keep,
            transparency: Cover::Gradient,
            exclude_color: None,
            vectors: VectorFill::Keep,
        }
    }
}
impl Settings {
    pub fn valid(&self) -> bool {
        crate::line_tools::point_valid(self.start)
            && crate::line_tools::point_valid(self.end)
            && (0.01..=100.0).contains(&self.ellipse_ratio)
            && self
                .ramp
                .as_ref()
                .is_none_or(crate::gradient_ramp::Ramp::valid)
    }
    pub fn color(&self, p: Point, anchors: Option<[Point; 2]>) -> [u8; 4] {
        let [a, b] = anchors.unwrap_or([self.start, self.end]);
        let delta = [b[0] - a[0], b[1] - a[1]];
        let length = delta[0].hypot(delta[1]);
        let mut t = if length < 1e-9 {
            0.0
        } else {
            let q = [p[0] - a[0], p[1] - a[1]];
            let along = (q[0] * delta[0] + q[1] * delta[1]) / length;
            let across = (-q[0] * delta[1] + q[1] * delta[0]) / length;
            match self.shape {
                Shape::StraightLine => along / length,
                Shape::Circle => q[0].hypot(q[1]) / length,
                Shape::Ellipse => along.hypot(across / self.ellipse_ratio) / length,
                Shape::Cone => {
                    (q[1].atan2(q[0]) - delta[1].atan2(delta[0])).rem_euclid(std::f64::consts::TAU)
                        / std::f64::consts::TAU
                }
                Shape::Freeform => {
                    let d = q[0].hypot(q[1]);
                    d / (d + (p[0] - b[0]).hypot(p[1] - b[1]))
                }
            }
        };
        t = match self.repeat {
            Repeat::DoNotRepeat => t.clamp(0.0, 1.0),
            Repeat::Repeat => t.rem_euclid(1.0),
            Repeat::Reverse => 1.0 - (t.rem_euclid(2.0) - 1.0).abs(),
            Repeat::NotDraw => {
                if !(0.0..=1.0).contains(&t) {
                    return [0; 4];
                } else {
                    t
                }
            }
        };
        let sample = if let Some(ramp) = &self.ramp {
            ramp.sample(t)
        } else {
            crate::gradient_ramp::mix(
                crate::gradient_ramp::Space::Standard,
                crate::gradient_ramp::Brightness::Off,
                self.first,
                self.last,
                t,
            )
        };
        if sample[3] == 0.0 {
            return [0; 4];
        }
        let bayer = [[0, 8, 2, 10], [12, 4, 14, 6], [3, 11, 1, 9], [15, 7, 13, 5]];
        let noise = if self.dithering {
            (bayer[(p[1].floor() as i64).rem_euclid(4) as usize]
                [(p[0].floor() as i64).rem_euclid(4) as usize] as f64
                + 0.5)
                / 16.0
                - 0.5
        } else {
            0.0
        };
        let mut c = [0; 4];
        for (i, v) in c[..3].iter_mut().enumerate() {
            *v = (sample[i] + noise).round().clamp(0.0, 255.0) as u8;
        }
        c[3] = sample[3].round() as u8;
        c
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Area {
    pub anchors: Option<[Point; 2]>,
    pub paint: Mask,
    pub black: Mask,
    pub transparent: Mask,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Layer {
    pub id: u64,
    pub name: String,
    pub hidden: bool,
    pub settings: Settings,
    pub creation: Affine,
    pub areas: Vec<Area>,
}
impl Layer {
    pub fn pixel(&self, original: Point) -> [u8; 4] {
        if self.hidden {
            return [0; 4];
        }
        let p = self.creation.map(original);
        for a in &self.areas {
            let mut coverage = a.paint.coverage(p);
            let mut color = self.settings.color(p, a.anchors);
            for (mask, policy) in [
                (&a.black, self.settings.black),
                (&a.transparent, self.settings.transparency),
            ] {
                let c = mask.coverage(p);
                if c > coverage && policy != Cover::Keep {
                    coverage = c;
                    if policy == Cover::DrawingColor {
                        color = self.settings.drawing;
                    }
                }
            }
            if coverage > 0 {
                color[3] = ((color[3] as u32 * coverage as u32 + 127) / 255) as u8;
                return color;
            }
        }
        [0; 4]
    }
    fn valid(&self) -> bool {
        self.id > 0
            && !self.name.trim().is_empty()
            && self.name.chars().count() <= 64
            && !self.name.chars().any(char::is_control)
            && self.settings.valid()
            && self.creation.valid()
            && self.areas.len() <= 256
            && self.areas.iter().all(|a| {
                a.paint.valid()
                    && a.black.valid()
                    && a.transparent.valid()
                    && a.anchors
                        .is_none_or(|p| p.into_iter().all(crate::line_tools::point_valid))
            })
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Data {
    pub next_id: u64,
    pub layers: Vec<Layer>,
}
impl Default for Data {
    fn default() -> Self {
        Self {
            next_id: 1,
            layers: vec![],
        }
    }
}
impl Data {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
    pub fn valid(&self) -> bool {
        let mut ids = std::collections::BTreeSet::new();
        self.next_id > 0
            && self.layers.len() <= 32
            && self
                .layers
                .iter()
                .all(|l| l.valid() && l.id < self.next_id && ids.insert(l.id))
            && self.run_count() <= 32768
    }
    pub(crate) fn run_count(&self) -> usize {
        self.layers
            .iter()
            .flat_map(|l| &l.areas)
            .map(|a| a.paint.runs.len() + a.black.runs.len() + a.transparent.runs.len())
            .sum()
    }
    pub fn size_bytes(&self) -> usize {
        self.run_count() * std::mem::size_of::<Run>()
            + self
                .layers
                .iter()
                .map(|l| {
                    std::mem::size_of::<Layer>()
                        + l.name.len()
                        + l.areas.len() * std::mem::size_of::<Area>()
                })
                .sum::<usize>()
    }
}
pub(crate) fn mask(values: &[u8], width: usize) -> Result<Mask, String> {
    let mut runs = vec![];
    for (y, row) in values.chunks(width).enumerate() {
        let mut x = 0;
        while x < width {
            let alpha = row[x];
            let start = x;
            x += 1;
            while x < width && row[x] == alpha {
                x += 1
            }
            if alpha > 0 {
                runs.push(Run {
                    x: start as i32,
                    y: y as i32,
                    length: (x - start) as u32,
                    alpha,
                });
                if runs.len() > 32768 {
                    return Err("Gradient coverage exceeds run limit".into());
                }
            }
        }
    }
    Ok(Mask { runs })
}
pub fn prepare(
    doc: &Document,
    target: usize,
    settings: Settings,
    editable: bool,
    reference: bucket::Settings,
) -> Result<DocumentState, String> {
    if !settings.valid() || !doc.scene.valid() || doc.memory_bytes() > DEFAULT_DOCUMENT_BUDGET / 8 {
        return Err("Invalid gradient or gradient undo budget exceeded".into());
    }
    let count = doc.width as usize * doc.height as usize;
    if count == 0
        || count > 4 * 1024 * 1024
        || count
            .saturating_mul(32)
            .saturating_add(doc.resident_bytes())
            > DEFAULT_DOCUMENT_BUDGET
    {
        return Err("Gradient exceeds canvas/memory work limit".into());
    }
    let (areas, lines) = if settings.shape == Shape::Freeform {
        super::gradient_regions::prepare(doc, target, &settings, reference)?
    } else {
        let mut values = vec![0; count];
        for y in 0..doc.height {
            for x in 0..doc.width {
                values[y as usize * doc.width as usize + x as usize] = doc
                    .scene
                    .selection_coverage([x as f64 + 0.5, y as f64 + 0.5]);
            }
        }
        (
            vec![Area {
                anchors: None,
                paint: mask(&values, doc.width as usize)?,
                black: Mask::default(),
                transparent: Mask::default(),
            }],
            doc.scene.lines.clone(),
        )
    };
    if count.saturating_mul(areas.len().max(1)).saturating_mul(3) > 64 * 1024 * 1024 {
        return Err("Gradient region sampling work limit".into());
    }
    let layer = Layer {
        id: doc.scene.gradients.next_id,
        name: format!("Gradient {}", doc.scene.gradients.next_id),
        hidden: false,
        settings,
        creation: doc.scene.transform,
        areas,
    };
    let mut state = if editable {
        let mut state = doc.snapshot();
        state.scene.gradients.next_id = state
            .scene
            .gradients
            .next_id
            .checked_add(1)
            .ok_or("Gradient ID exhausted")?;
        state.scene.gradients.layers.push(layer);
        state
    } else {
        bucket::paint_pixels(doc, target, false, |x, y| {
            layer.pixel(
                doc.scene
                    .transform
                    .inverse()
                    .map([x as f64 + 0.5, y as f64 + 0.5]),
            )
        })?
    };
    state.scene.lines = lines;
    validate(&state.scene)?;
    Ok(state)
}
pub fn edit(doc: &Document, id: u64, settings: Settings) -> Result<DocumentState, String> {
    if !settings.valid() {
        return Err("Invalid gradient settings".into());
    }
    let mut state = doc.snapshot();
    let layer = state
        .scene
        .gradients
        .layers
        .iter_mut()
        .find(|l| l.id == id)
        .ok_or("Missing gradient layer")?;
    if (layer.settings.shape == Shape::Freeform) != (settings.shape == Shape::Freeform)
        || layer.settings.vectors != settings.vectors
        || layer.settings.exclude_color != settings.exclude_color
    {
        return Err("Changing gradient topology requires creating a new gradient".into());
    }
    if settings.shape == Shape::Freeform {
        for a in &mut layer.areas {
            if let Some(old) = a.anchors {
                let old_delta = [
                    layer.settings.end[0] - layer.settings.start[0],
                    layer.settings.end[1] - layer.settings.start[1],
                ];
                let delta = [
                    settings.end[0] - settings.start[0],
                    settings.end[1] - settings.start[1],
                ];
                a.anchors = Some([
                    [
                        old[0][0] + settings.start[0] - layer.settings.start[0],
                        old[0][1] + settings.start[1] - layer.settings.start[1],
                    ],
                    [
                        old[1][0] + settings.start[0] - layer.settings.start[0] + delta[0]
                            - old_delta[0],
                        old[1][1] + settings.start[1] - layer.settings.start[1] + delta[1]
                            - old_delta[1],
                    ],
                ]);
            }
        }
    }
    layer.settings = settings;
    validate(&state.scene)?;
    Ok(state)
}
pub fn validate(scene: &crate::scene::Scene) -> Result<(), String> {
    if !scene.valid()
        || serde_json::to_vec(scene).map_err(|e| e.to_string())?.len() > 4 * 1024 * 1024
    {
        Err("Gradient metadata limit exceeded".into())
    } else {
        Ok(())
    }
}
