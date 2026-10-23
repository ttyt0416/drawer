//! Local connected-component dust repair and a separate selection-only path.
use crate::{
    color_precision::Sample,
    document::{DEFAULT_DOCUMENT_BUDGET, Document},
    scene::Point,
    sizing::DocumentState,
};
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Mode {
    #[default]
    Transparency,
    WhiteBackground,
    Surrounding,
    Foreground,
}
impl Mode {
    pub const ALL: [Self; 4] = [
        Self::Transparency,
        Self::WhiteBackground,
        Self::Surrounding,
        Self::Foreground,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Transparency => "Remove dust from transparency",
            Self::WhiteBackground => "Remove dust from white background",
            Self::Surrounding => "Fill transparent gaps with surrounding color",
            Self::Foreground => "Fill transparent gaps with foreground color",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Dust {
    pub mode: Mode,
    pub max_area: u32,
    pub threshold: f64,
    pub foreground: [f64; 4],
}
impl Default for Dust {
    fn default() -> Self {
        Self {
            mode: Mode::Transparency,
            max_area: 4,
            threshold: 0.1,
            foreground: [0., 0., 0., 1.],
        }
    }
}
impl Dust {
    pub fn valid(self) -> bool {
        self.max_area <= 4096
            && (0. ..=1.).contains(&self.threshold)
            && self.foreground.iter().all(|v| (0. ..=1.).contains(v))
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum Operation {
    Descreen {
        radius: u8,
        amount: f64,
        line_threshold: f64,
        #[serde(default)]
        coverage: bool,
    },
    Gray,
    Dust(Dust),
}
impl Operation {
    pub const COMMANDS: [&'static str; 3] = ["remove-tone", "grayscale-filter", "cleanup-dust"];
    pub fn defaults() -> [Self; 3] {
        [
            Self::Descreen {
                radius: 1,
                amount: 1.,
                line_threshold: 0.02,
                coverage: false,
            },
            Self::Gray,
            Self::Dust(Default::default()),
        ]
    }
    pub fn valid(self) -> bool {
        match self {
            Self::Descreen {
                radius,
                amount,
                line_threshold,
                ..
            } => {
                radius <= 8 && (0. ..=1.).contains(&amount) && (0. ..=1.).contains(&line_threshold)
            }
            Self::Gray => true,
            Self::Dust(s) => s.valid(),
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Descreen { .. } => "Remove tone / descreen",
            Self::Gray => "Grayscale pixels",
            Self::Dust(_) => "Dust and transparent gaps",
        }
    }
    pub fn identity(self) -> bool {
        match self {
            Self::Descreen { radius, amount, .. } => radius == 0 || amount == 0.,
            Self::Gray => false,
            Self::Dust(s) => s.max_area == 0,
        }
    }
    pub fn preserves_alpha(self) -> bool {
        !matches!(self,Self::Dust(s)if s.mode!=Mode::WhiteBackground)
            && !matches!(self, Self::Descreen { coverage: true, .. })
    }
    pub fn cost(self) -> usize {
        match self {
            Self::Descreen { radius, .. } => (2 * radius as usize + 1).pow(2) + 8,
            Self::Gray => 1,
            Self::Dust(_) => 12,
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SelectionKind {
    #[default]
    Transparency,
    WhiteBackground,
    Hole,
}
impl SelectionKind {
    pub const ALL: [Self; 3] = [Self::Transparency, Self::WhiteBackground, Self::Hole];
    pub const COMMANDS: [&'static str; 3] = [
        "select-dust-transparency",
        "select-dust-white",
        "select-transparent-hole",
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Transparency => "Select dust on transparency",
            Self::WhiteBackground => "Select dust on white background",
            Self::Hole => "Select transparent hole",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Selection {
    pub kind: SelectionKind,
    pub max_area: u32,
    pub threshold: f64,
}
impl Default for Selection {
    fn default() -> Self {
        Self {
            kind: Default::default(),
            max_area: 4,
            threshold: 0.1,
        }
    }
}
impl Selection {
    pub fn dust(self) -> Dust {
        Dust {
            mode: match self.kind {
                SelectionKind::Transparency => Mode::Transparency,
                SelectionKind::WhiteBackground => Mode::WhiteBackground,
                SelectionKind::Hole => Mode::Surrounding,
            },
            max_area: self.max_area,
            threshold: self.threshold,
            ..Default::default()
        }
    }
    pub fn valid(self) -> bool {
        self.dust().valid()
    }
}
fn luma(p: Sample) -> f64 {
    let [r, g, b] = p.rgb();
    r * 0.2126 + g * 0.7152 + b * 0.0722
}
fn inside(d: &Document, xy: [i64; 2]) -> bool {
    xy[0] >= 0 && xy[1] >= 0 && xy[0] < d.width as i64 && xy[1] < d.height as i64
}
/// Classify full input components before clipping output to the current selection.
pub fn candidates(
    d: &Document,
    layer: usize,
    s: Dust,
    mut tick: impl FnMut(usize, usize) -> Result<(), String>,
) -> Result<Vec<Option<Sample>>, String> {
    if !s.valid() {
        return Err("Dust area0–4096, threshold/foreground0–1 required".into());
    }
    let src = &d.layers[layer];
    let e = src.extent(d.width, d.height);
    let count = src.pixels.len();
    let width = e.width as usize;
    let mut eligible = vec![false; count];
    for (i, value) in eligible.iter_mut().enumerate() {
        if i.is_multiple_of(128) {
            tick(i, count * 3)?;
        }
        let p = src.sample_at(i);
        let xy = [
            e.offset[0] + (i % width) as i64,
            e.offset[1] + (i / width) as i64,
        ];
        *value = inside(d, xy)
            && match s.mode {
                Mode::Transparency => p.alpha() > 0. && p.alpha() >= s.threshold,
                Mode::WhiteBackground => {
                    p.alpha() > 0. && (1. - luma(p)) * p.alpha() >= s.threshold
                }
                Mode::Surrounding | Mode::Foreground => p.alpha() < 1. - s.threshold,
            };
    }
    let mut visited = vec![false; count];
    let mut output = vec![None; count];
    let mut queue = Vec::new();
    let mut processed = 0;
    if s.max_area == 0 {
        return Ok(output);
    }
    for start in 0..count {
        if start.is_multiple_of(128) {
            tick(count + processed, count * 3)?;
        }
        if visited[start] || !eligible[start] {
            continue;
        }
        queue.push(start);
        visited[start] = true;
        let mut members = Vec::new();
        let mut area = 0;
        let mut border = false;
        let mut sum = [0.; 4];
        let mut neighbors = 0;
        let mut white_border = true;
        while let Some(i) = queue.pop() {
            if processed.is_multiple_of(128) {
                tick(count + processed, count * 3)?;
            }
            processed += 1;
            area += 1;
            if members.len() <= s.max_area as usize {
                members.push(i);
            }
            let x = i % width;
            let y = i / width;
            let xy = [e.offset[0] + x as i64, e.offset[1] + y as i64];
            border |= x == 0
                || x + 1 == width
                || y == 0
                || y + 1 == e.height as usize
                || xy[0] == 0
                || xy[1] == 0
                || xy[0] + 1 == d.width as i64
                || xy[1] + 1 == d.height as i64;
            for (dx, dy) in [
                (-1, -1),
                (0, -1),
                (1, -1),
                (-1, 0),
                (1, 0),
                (-1, 1),
                (0, 1),
                (1, 1),
            ] {
                let nx = x as i64 + dx;
                let ny = y as i64 + dy;
                if nx < 0 || ny < 0 || nx >= width as i64 || ny >= e.height as i64 {
                    continue;
                }
                let n = ny as usize * width + nx as usize;
                if eligible[n] {
                    if !visited[n] {
                        visited[n] = true;
                        queue.push(n);
                    }
                } else {
                    let p = src.sample_at(n);
                    let a = p.alpha();
                    white_border &= a >= 1. - s.threshold && luma(p) >= 1. - s.threshold;
                    if a > 0. {
                        for (k, v) in p.rgb().into_iter().enumerate() {
                            sum[k] += v * a;
                        }
                        sum[3] += a;
                        neighbors += 1;
                    }
                }
            }
        }
        if border || area > s.max_area as usize || s.mode == Mode::WhiteBackground && !white_border
        {
            continue;
        }
        let replacement = match s.mode {
            Mode::Transparency => Sample::default(),
            Mode::WhiteBackground => Sample::new([1.; 4])?,
            Mode::Foreground => Sample::new(s.foreground)?,
            Mode::Surrounding => {
                if sum[3] == 0. || neighbors == 0 {
                    continue;
                }
                Sample::new([
                    sum[0] / sum[3],
                    sum[1] / sum[3],
                    sum[2] / sum[3],
                    sum[3] / neighbors as f64,
                ])?
            }
        };
        for i in members {
            output[i] = Some(if s.mode == Mode::WhiteBackground {
                src.sample_at(i).with_rgb([1.; 3])?
            } else {
                replacement
            });
        }
    }
    tick(count * 3, count * 3)?;
    Ok(output)
}
pub fn sample(
    d: &Document,
    layer: usize,
    xy: [i64; 2],
    op: Operation,
    options: crate::filter_options::Options,
) -> Result<Sample, String> {
    let before = d.layers[layer].sample(d.width, d.height, xy[0], xy[1]);
    if before.alpha() == 0. && !matches!(op, Operation::Descreen { coverage: true, .. }) {
        return Ok(before);
    }
    match op {
        Operation::Gray => before.with_rgb([luma(before); 3]),
        Operation::Descreen {
            radius,
            amount,
            line_threshold,
            coverage,
        } => {
            let sampler = crate::filter_options::Sampler {
                document: d,
                layer,
                boundary: options.boundary,
            };
            let h = luma(before);
            let line = [[-1, 0], [0, -1], [-1, -1], [1, -1]]
                .into_iter()
                .filter(|v| {
                    let a = sampler.at(xy[0] + v[0], xy[1] + v[1]);
                    let b = sampler.at(xy[0] - v[0], xy[1] - v[1]);
                    let ca = a.alpha() > 0. && (luma(a) - h).abs() <= line_threshold;
                    let cb = b.alpha() > 0. && (luma(b) - h).abs() <= line_threshold;
                    ca && cb
                        || ca && !inside(d, [xy[0] - v[0], xy[1] - v[1]])
                        || cb && !inside(d, [xy[0] + v[0], xy[1] + v[1]])
                })
                .count()
                == 1
                && h < 0.9;
            if line && before.alpha() > 0. {
                return Ok(before);
            }
            let mut sum = [0.; 3];
            let mut total = 0.;
            let r = radius as i64;
            for y in -r..=r {
                for x in -r..=r {
                    let p = sampler.at(xy[0] + x, xy[1] + y);
                    let a = p.alpha();
                    for (k, v) in p.rgb().into_iter().enumerate() {
                        sum[k] += v * a;
                    }
                    total += a;
                }
            }
            if total == 0. {
                return Ok(before);
            }
            let filtered = if coverage {
                Sample::new([
                    sum[0] / total,
                    sum[1] / total,
                    sum[2] / total,
                    total / (2 * r + 1).pow(2) as f64,
                ])?
            } else {
                before.with_rgb(sum.map(|v| v / total))?
            };
            before.mix(filtered, amount)
        }
        Operation::Dust(_) => Err("Dust requires precomputed components".into()),
    }
}
pub fn select(
    d: &Document,
    layer: usize,
    s: Selection,
    budget: usize,
    mut tick: impl FnMut(usize, usize) -> Result<(), String>,
) -> Result<DocumentState, String> {
    tick(0, 1)?;
    if !s.valid() {
        return Err("Invalid dust selection settings".into());
    }
    let src = d.layers.get(layer).ok_or("Select a raster layer")?;
    let count = src.pixels.len();
    let grid = crate::selection_edit::Grid::new(d)?;
    let cells = grid.size[0] * grid.size[1];
    if count > 4 * 1024 * 1024
        || d.memory_bytes() > DEFAULT_DOCUMENT_BUDGET / 8
        || count
            .saturating_mul(12)
            .saturating_add(cells.saturating_mul(16))
            > 64 * 1024 * 1024
        || d.resident_bytes()
            .saturating_add(d.memory_bytes().saturating_mul(4))
            .saturating_add(count.saturating_mul(64))
            .saturating_add(cells.saturating_mul(2))
            > budget.min(DEFAULT_DOCUMENT_BUDGET)
    {
        return Err("Dust selection exceeds source, work or workspace memory limit".into());
    }
    if d.scene
        .selection_mask
        .as_ref()
        .is_some_and(|m| m.runs.is_empty())
    {
        return Ok(d.snapshot());
    }
    let values = candidates(d, layer, s.dust(), &mut tick)?;
    let e = src.extent(d.width, d.height);
    let coverage = |p: Point| {
        let x = p[0].floor() as i64;
        let y = p[1].floor() as i64;
        if !inside(d, [x, y]) {
            return 0;
        }
        let x = x - e.offset[0];
        let y = y - e.offset[1];
        if x < 0 || y < 0 || x >= e.width as i64 || y >= e.height as i64 {
            return 0;
        }
        if values[y as usize * e.width as usize + x as usize].is_some() {
            d.scene.selection_coverage(p)
        } else {
            0
        }
    };
    let selected = grid.sample_with_progress(d, true, coverage, &mut || tick(0, 1))?;
    let mut state = d.snapshot();
    state.scene = grid.scene(d, &selected)?;
    if state.scene == d.scene {
        return Ok(d.snapshot());
    }
    state.scene.last_detail_filter = Some(crate::detail_filter::Settings {
        operation: crate::detail_filter::Operation::SelectCleanup(s),
        selection: true,
        options: Default::default(),
    });
    crate::layer_groups::finish(d, &state)?;
    tick(1, 1)?;
    Ok(state)
}
pub fn active(scene: &crate::scene::Scene) -> bool {
    let extra = |s: crate::detail_filter::Settings| {
        matches!(
            s.operation,
            crate::detail_filter::Operation::SelectCleanup(_)
                | crate::detail_filter::Operation::Pixels(
                    crate::pixel_effects::Operation::Cleanup(_)
                )
        )
    };
    scene.last_detail_filter.is_some_and(extra)
        || scene
            .last_batch_filter
            .as_ref()
            .is_some_and(|r| matches!(r.task,crate::batch_filters::Task::Detail(s)if extra(s)))
}
