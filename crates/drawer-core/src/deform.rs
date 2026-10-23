//! Baked raster quadrilaterals and bounded, sequential liquify brush dabs.
use crate::{
    color_precision::{Depth, Sample},
    document::{DEFAULT_DOCUMENT_BUDGET, Document, PlacedRaster, RasterLayer},
    layer_properties as p,
    layer_search::Key,
    material_mapping::Warp,
    scene::{Point, Rect},
    sizing::DocumentState,
};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Push,
    Expand,
    Pinch,
    Left,
    Right,
    Clockwise,
    Anticlockwise,
}
impl Mode {
    pub const ALL: [Self; 7] = [
        Self::Push,
        Self::Expand,
        Self::Pinch,
        Self::Left,
        Self::Right,
        Self::Clockwise,
        Self::Anticlockwise,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Push => "Push",
            Self::Expand => "Expand",
            Self::Pinch => "Pinch",
            Self::Left => "Push Left",
            Self::Right => "Push Right",
            Self::Clockwise => "Twirl Clockwise",
            Self::Anticlockwise => "Twirl Anti-Clockwise",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Dab {
    pub mode: Mode,
    pub center: Point,
    pub delta: Point,
    pub radius: f64,
    pub strength: f64,
}
impl Dab {
    pub fn valid(self) -> bool {
        self.center.iter().all(|v| v.is_finite() && v.abs() <= 1e6)
            && self.delta.iter().all(|v| v.is_finite() && v.abs() <= 4096.)
            && self.radius.is_finite()
            && (0.5..=1024.).contains(&self.radius)
            && self.strength.is_finite()
            && (0. ..=1.).contains(&self.strength)
    }
    /// Destination to source, in screen coordinates (positive y points down).
    pub fn inverse(self, point: Point) -> Point {
        let v = [point[0] - self.center[0], point[1] - self.center[1]];
        let r2 = (v[0] * v[0] + v[1] * v[1]) / (self.radius * self.radius);
        if r2 >= 1. || self.strength == 0. {
            return point;
        }
        let f = (1. - r2).powi(2) * self.strength;
        let q = match self.mode {
            Mode::Push | Mode::Left | Mode::Right => {
                let len = self.delta[0].hypot(self.delta[1]);
                let scale = if len > 0. {
                    (self.radius * 0.25 / len).min(1.)
                } else {
                    1.
                };
                let delta = match self.mode {
                    Mode::Left => [self.delta[1], -self.delta[0]],
                    Mode::Right => [-self.delta[1], self.delta[0]],
                    _ => self.delta,
                };
                return [
                    point[0] - delta[0] * f * scale,
                    point[1] - delta[1] * f * scale,
                ];
            }
            Mode::Expand => v.map(|v| v * (-f).exp()),
            Mode::Pinch => v.map(|v| v * f.exp()),
            Mode::Clockwise | Mode::Anticlockwise => {
                let angle = if self.mode == Mode::Clockwise { -f } else { f };
                let (s, c) = angle.sin_cos();
                [c * v[0] - s * v[1], s * v[0] + c * v[1]]
            }
        };
        [self.center[0] + q[0], self.center[1] + q[1]]
    }
}
pub enum Change<'a> {
    Quad(&'a Warp),
    Mesh(&'a crate::mesh_deform::Mesh),
    Liquify { dabs: &'a [Dab], only_refer: bool },
}
#[derive(Clone, Copy)]
enum Mapping<'a> {
    Quad(&'a Warp),
    Mesh(&'a crate::mesh_deform::Mesh),
}
impl Mapping<'_> {
    fn points(self) -> Vec<Point> {
        match self {
            Self::Quad(w) => w.points(),
            Self::Mesh(m) => m.points.clone(),
        }
    }
    fn identity(self) -> bool {
        match self {
            Self::Quad(w) => w.points() == crate::material_mapping::UNIT,
            Self::Mesh(m) => m.identity(),
        }
    }
    fn cost(self) -> usize {
        match self {
            Self::Quad(_) => 1,
            Self::Mesh(m) => 2 * (m.columns as usize - 1) * (m.rows as usize - 1),
        }
    }
    fn inverse(self, area: Rect, p: Point) -> Option<Point> {
        match self {
            Self::Quad(w) => inverse_quad(w, area, p),
            Self::Mesh(m) => m.inverse(normalized(area, p)).map(|p| document(area, p)),
        }
    }
}
pub fn corners(r: Rect) -> [Point; 4] {
    [r.min, [r.max[0], r.min[1]], r.max, [r.min[0], r.max[1]]]
}
fn bounds(points: impl IntoIterator<Item = Point>) -> Rect {
    let mut r = Rect {
        min: [f64::INFINITY; 2],
        max: [f64::NEG_INFINITY; 2],
    };
    for point in points {
        for (n, v) in point.into_iter().enumerate() {
            r.min[n] = r.min[n].min(v);
            r.max[n] = r.max[n].max(v);
        }
    }
    r
}
pub fn targets(d: &Document, layers: &[usize]) -> Result<Vec<usize>, String> {
    if layers.iter().any(|n| *n >= d.layers.len()) {
        return Err("Raster layer is missing".into());
    }
    let keys = layers
        .iter()
        .map(|n| Key::Layer(p::raster(d, *n)))
        .collect::<Vec<_>>();
    let keys = crate::layer_groups::expanded(d, &keys, crate::layer_groups::Operation::Transform)?;
    crate::layer_groups::preflight(d, &keys, crate::layer_groups::Operation::Transform)?;
    keys.into_iter()
        .map(|key| {
            let Key::Layer(p::Target::Raster(id)) = key else {
                return Err(
                    "Deformation requires raster layers; convert linked non-raster members first"
                        .into(),
                );
            };
            let n = crate::layers::index(d, id).ok_or("Raster layer changed")?;
            p::paintable(d, n)?;
            if crate::clipping::alpha_locked(d, n) {
                return Err("Unlock layer transparency before moving its pixels".into());
            }
            Ok(n)
        })
        .collect()
}
pub fn area(d: &Document, layers: &[usize], selection: bool) -> Result<Option<Rect>, String> {
    let layers = targets(d, layers)?;
    let source = bounds(layers.iter().flat_map(|n| {
        let e = d.layers[*n].extent(d.width, d.height);
        corners(Rect {
            min: e.offset.map(|v| v as f64),
            max: [
                (e.offset[0] + e.width as i64) as f64,
                (e.offset[1] + e.height as i64) as f64,
            ],
        })
    }));
    if !selection {
        return Ok(Some(source));
    }
    if d.scene
        .selection_mask
        .as_ref()
        .is_some_and(|m| m.runs.is_empty())
    {
        return Ok(None);
    }
    let selected = if let Some(mask) = &d.scene.selection_mask {
        bounds(
            mask.runs
                .iter()
                .flat_map(|r| {
                    corners(Rect {
                        min: [r.x as f64, r.y as f64],
                        max: [r.x as f64 + r.length as f64, r.y as f64 + 1.],
                    })
                })
                .map(|p| d.scene.selection_point(p)),
        )
    } else if let Some(r) = d.scene.selection {
        bounds(corners(r).map(|p| d.scene.selection_point(p)))
    } else {
        return Ok(Some(source));
    };
    let r = Rect {
        min: [0, 1].map(|n| source.min[n].max(selected.min[n])),
        max: [0, 1].map(|n| source.max[n].min(selected.max[n])),
    };
    Ok((0..2).all(|n| r.max[n] > r.min[n]).then_some(r))
}
fn normalized(area: Rect, p: Point) -> Point {
    [0, 1].map(|n| (p[n] - area.min[n]) / (area.max[n] - area.min[n]))
}
fn document(area: Rect, p: Point) -> Point {
    [0, 1].map(|n| area.min[n] + p[n] * (area.max[n] - area.min[n]))
}
pub fn inverse_quad(w: &Warp, area: Rect, p: Point) -> Option<Point> {
    w.inverse(normalized(area, p)).map(|p| document(area, p))
}
fn extent(r: Rect) -> Result<PlacedRaster, String> {
    let lo = r.min.map(f64::floor);
    let hi = r.max.map(f64::ceil);
    if lo
        .iter()
        .chain(&hi)
        .any(|v| !v.is_finite() || v.abs() > 1e6)
        || (0..2).any(|n| !(1. ..=32768.).contains(&(hi[n] - lo[n])))
        || (hi[0] - lo[0]) * (hi[1] - lo[1]) > 4. * 1024. * 1024.
    {
        return Err("Deformation extent exceeds4M pixels,32768 axis or coordinate limit".into());
    }
    Ok(PlacedRaster {
        offset: lo.map(|v| v as i64),
        width: (hi[0] - lo[0]) as u32,
        height: (hi[1] - lo[1]) as u32,
    })
}
struct Pass<'a> {
    d: &'a Document,
    source: &'a RasterLayer,
    area: Rect,
    extent: PlacedRaster,
    selection: bool,
}
impl Pass<'_> {
    fn render(
        &self,
        quad: Option<Mapping<'_>>,
        dab: Option<Dab>,
        only_refer: bool,
        tick: &mut impl FnMut() -> Result<(), String>,
    ) -> Result<RasterLayer, String> {
        let Self {
            d,
            source,
            area,
            extent,
            selection,
        } = *self;
        let count = extent.width as usize * extent.height as usize;
        let mut out = source.clone();
        out.pixels = Vec::with_capacity(count);
        out.precise = (d.color.depth == Depth::Sixteen).then(|| Vec::with_capacity(count));
        out.placement = Some(extent);
        for y in 0..extent.height {
            for x in 0..extent.width {
                if x.is_multiple_of(128) {
                    tick()?;
                }
                let pos = [extent.offset[0] + x as i64, extent.offset[1] + y as i64];
                let point = pos.map(|v| v as f64 + 0.5);
                let coverage = if selection {
                    d.scene.selection_coverage(point) as f64 / 255.
                } else {
                    1.
                };
                let input = if let Some(w) = quad {
                    w.inverse(area, point)
                } else {
                    Some(dab.unwrap().inverse(point))
                };
                let unchanged = input == Some(point) || quad.is_none() && coverage == 0.;
                if unchanged {
                    let i = source.pixel_index(d.width, d.height, pos[0], pos[1]);
                    out.pixels.push(i.map_or([0; 4], |i| source.pixels[i]));
                    if let Some(high) = &mut out.precise {
                        high.push(i.map_or([0; 4], |i| source.sample_at(i).rgba16()));
                    }
                    continue;
                }
                let at = |x: i64, y: i64| {
                    let p = [x as f64 + 0.5, y as f64 + 0.5];
                    let a = if selection && (quad.is_some() || only_refer) {
                        d.scene.selection_coverage(p) as f64 / 255.
                    } else {
                        1.
                    };
                    if only_refer && (0..2).any(|n| p[n] < area.min[n] || p[n] >= area.max[n]) {
                        Sample::default()
                    } else {
                        source.sample(d.width, d.height, x, y).coverage(a).unwrap()
                    }
                };
                let moved = input.map_or_else(Sample::default, |p| {
                    crate::interpolation::sample_float(
                        p.map(|v| v - 0.5),
                        [p; 4],
                        crate::interpolation::Filter::Bilinear,
                        at,
                    )
                });
                if quad.is_some() && coverage == 0. && moved.alpha() == 0. {
                    let i = source.pixel_index(d.width, d.height, pos[0], pos[1]);
                    out.pixels.push(i.map_or([0; 4], |i| source.pixels[i]));
                    if let Some(high) = &mut out.precise {
                        high.push(i.map_or([0; 4], |i| source.sample_at(i).rgba16()));
                    }
                    continue;
                }
                let before = source.sample(d.width, d.height, pos[0], pos[1]);
                let result = if quad.is_some() {
                    before.coverage(1. - coverage)?.over(
                        moved,
                        crate::layer_blend::Mode::Normal,
                        1.,
                    )?
                } else if coverage == 1. {
                    moved
                } else {
                    before.mix(moved, coverage)?
                };
                let stored = result.store(d.color.depth, d.color.dither, pos);
                if let Some(high) = &mut out.precise {
                    high.push(stored);
                    out.pixels.push(Sample::from16(stored).rgba8());
                } else {
                    out.pixels.push(stored.map(|v| v as u8));
                }
            }
        }
        Ok(out)
    }
}
pub fn prepare(
    d: &Document,
    layers: &[usize],
    selection: bool,
    change: Change<'_>,
) -> Result<DocumentState, String> {
    prepare_with_progress(d, layers, selection, change, &mut || Ok(()))
}
pub fn prepare_with_progress(
    d: &Document,
    layers: &[usize],
    selection: bool,
    change: Change<'_>,
    tick: &mut impl FnMut() -> Result<(), String>,
) -> Result<DocumentState, String> {
    let record = if let Change::Quad(w) = &change {
        if let Some(area) = area(d, layers, selection)? {
            match w {
                Warp::Distort(points) => Some(crate::transform_handles::Record::Quad {
                    points: *points,
                    area,
                    perspective: false,
                }),
                Warp::Perspective(points) => Some(crate::transform_handles::Record::Quad {
                    points: *points,
                    area,
                    perspective: true,
                }),
                _ => None,
            }
        } else {
            None
        }
    } else {
        None
    };
    let mut state = prepare_inner(d, layers, selection, change, tick)?;
    if state != d.snapshot() && record.is_some() {
        state.scene.last_transform = record;
    }
    if state.memory_bytes().saturating_add(d.memory_bytes())
        > crate::document::DEFAULT_DOCUMENT_BUDGET / 4
    {
        return Err("Deformation receipt/output exceeds Undo memory budget".into());
    }
    Ok(state)
}
fn prepare_inner(
    d: &Document,
    layers: &[usize],
    selection: bool,
    change: Change<'_>,
    tick: &mut impl FnMut() -> Result<(), String>,
) -> Result<DocumentState, String> {
    tick()?;
    let layers = targets(d, layers)?;
    let (warp, dabs, only_refer) = match change {
        Change::Quad(w) => {
            if !matches!(w, Warp::Distort(_) | Warp::Perspective(_)) || !w.valid() {
                return Err("Quadrilateral is reversed, folded, degenerate or crosses a perspective horizon".into());
            }
            (Some(Mapping::Quad(w)), &[][..], false)
        }
        Change::Mesh(m) => {
            if !m.valid() {
                return Err(
                    "Mesh is folded, overlapping, degenerate or outside the grid limit".into(),
                );
            }
            (Some(Mapping::Mesh(m)), &[][..], false)
        }
        Change::Liquify { dabs, only_refer } => {
            if dabs.len() > 512 || dabs.iter().any(|d| !d.valid()) {
                return Err("Liquify dab/radius/strength limit".into());
            }
            (None, dabs, only_refer)
        }
    };
    let Some(area) = area(d, &layers, selection)? else {
        return Ok(d.snapshot());
    };
    if warp.is_some_and(|w| w.identity())
        || warp.is_none()
            && dabs.iter().all(|b| {
                b.strength == 0.
                    || matches!(b.mode, Mode::Push | Mode::Left | Mode::Right) && b.delta == [0.; 2]
            })
    {
        return Ok(d.snapshot());
    }
    let mapped = if let Some(w) = warp {
        bounds(w.points().into_iter().map(|p| document(area, p)))
    } else {
        bounds(dabs.iter().flat_map(|b| {
            [
                [b.center[0] - b.radius, b.center[1] - b.radius],
                [b.center[0] + b.radius, b.center[1] + b.radius],
            ]
        }))
    };
    let grid = if warp.is_some()
        && selection
        && (d.scene.selection.is_some() || d.scene.selection_mask.is_some())
    {
        Some(crate::selection_edit::Grid::with_bounds(
            d,
            &corners(mapped),
        )?)
    } else {
        None
    };
    let mut plans = vec![];
    let mut work = grid.as_ref().map_or(0, |g| {
        g.size[0]
            .saturating_mul(g.size[1])
            .saturating_mul(16)
            .saturating_mul(warp.unwrap().cost())
    });
    let mut bytes = 0usize;
    for n in &layers {
        let old = d.layers[*n].extent(d.width, d.height);
        let old_rect = Rect {
            min: old.offset.map(|v| v as f64),
            max: [
                (old.offset[0] + old.width as i64) as f64,
                (old.offset[1] + old.height as i64) as f64,
            ],
        };
        let e = extent(bounds(corners(old_rect).into_iter().chain(corners(mapped))))?;
        let count = e.width as usize * e.height as usize;
        work = work
            .saturating_add(count.saturating_mul(warp.map_or(dabs.len().max(1), Mapping::cost)));
        bytes = bytes.saturating_add(count.saturating_mul(d.color.depth.bytes()));
        plans.push((*n, e));
    }
    if work > 32 * 1024 * 1024
        || d.resident_bytes()
            .saturating_add(d.memory_bytes().saturating_mul(3))
            .saturating_add(bytes.saturating_mul(3))
            > DEFAULT_DOCUMENT_BUDGET
    {
        return Err("Deformation processing32M or workspace memory limit; reduce layer size, cage density, selected extent or liquify dabs".into());
    }
    let mut state = d.snapshot();
    for (n, extent) in plans {
        if let Some(w) = warp {
            state.layers[n] = Pass {
                d,
                source: &d.layers[n],
                area,
                extent,
                selection,
            }
            .render(Some(w), None, false, tick)?;
        } else {
            for dab in dabs {
                if dab.strength > 0. {
                    state.layers[n] = Pass {
                        d,
                        source: &state.layers[n],
                        area,
                        extent,
                        selection,
                    }
                    .render(None, Some(*dab), only_refer, tick)?;
                }
            }
        }
    }
    if let (Some(w), Some(grid)) = (warp, grid) {
        let values = grid.sample_with_progress(
            d,
            false,
            |p| {
                w.inverse(area, p)
                    .map_or(0, |p| d.scene.selection_coverage(p))
            },
            tick,
        )?;
        state.scene = grid.scene(d, &values)?;
    }
    tick()?;
    crate::layer_groups::finish(d, &state)?;
    Ok(state)
}
