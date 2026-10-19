//! Independent pixel-preserving display color, square outline and watercolor edge recipe.
use crate::{
    document::{DEFAULT_DOCUMENT_BUDGET, Document, PlacedRaster},
    layer_properties::{self as p, Target},
    layer_search::{Expression, Key},
    scene::Point,
    selection_tools::Shape,
    sizing::DocumentState,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, VecDeque};
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Style {
    pub enabled: bool,
    pub color: Option<[u8; 3]>,
    pub border: u8,
    pub border_color: [u8; 4],
    pub rim: f64,
    pub scope: Option<Shape>,
}
impl Default for Style {
    fn default() -> Self {
        Self {
            enabled: false,
            color: None,
            border: 0,
            border_color: [0, 0, 0, 255],
            rim: 0.,
            scope: None,
        }
    }
}
impl Style {
    pub fn active(&self) -> bool {
        self.enabled && (self.color.is_some() || self.border > 0 || self.rim > 0.)
    }
    pub fn valid(&self) -> bool {
        self.border <= 32
            && (0. ..=1.).contains(&self.rim)
            && self.scope.as_ref().is_none_or(Shape::valid)
    }
    fn weight(&self, original: Point) -> f64 {
        let Some(s) = &self.scope else {
            return 1.;
        };
        let point = [
            original[0] - s.translation[0],
            original[1] - s.translation[1],
        ];
        if s.rect.is_some_and(|r| !r.contains(point)) {
            return 0.;
        }
        s.mask
            .as_ref()
            .map_or(1., |m| m.coverage(point) as f64 / 255.)
    }
    fn bytes(&self) -> usize {
        128 + self
            .scope
            .as_ref()
            .and_then(|s| s.mask.as_ref())
            .map_or(0, |m| {
                m.runs.len() * std::mem::size_of::<crate::selection::Run>()
            })
    }
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Data {
    pub rasters: BTreeMap<u64, Style>,
    pub vectors: BTreeMap<u64, Style>,
}
impl Data {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
    pub fn valid(&self) -> bool {
        self.rasters.len() <= 256
            && self.vectors.len() <= 32
            && self
                .rasters
                .keys()
                .chain(self.vectors.keys())
                .all(|id| *id > 0)
            && self
                .rasters
                .values()
                .chain(self.vectors.values())
                .all(Style::valid)
            && self
                .rasters
                .values()
                .chain(self.vectors.values())
                .filter_map(|s| s.scope.as_ref())
                .filter_map(|s| s.mask.as_ref())
                .map(|m| m.runs.len())
                .sum::<usize>()
                <= 32768
    }
    pub fn bytes(&self) -> usize {
        self.rasters
            .values()
            .chain(self.vectors.values())
            .map(Style::bytes)
            .sum()
    }
    pub fn get(&self, t: Target) -> Option<&Style> {
        match t {
            Target::Raster(id) => self.rasters.get(&id),
            Target::Vector(id) => self.vectors.get(&id),
            _ => None,
        }
    }
    pub fn set(&mut self, t: Target, s: Style) -> Result<(), String> {
        let map=match t{Target::Raster(_)=>&mut self.rasters,Target::Vector(_)=>&mut self.vectors,_=>return Err("Display recipes support raster and vector layers; folder paper is edited separately".into())};
        let id = match t {
            Target::Raster(id) | Target::Vector(id) => id,
            _ => unreachable!(),
        };
        if s == Style::default() {
            map.remove(&id);
        } else {
            map.insert(id, s);
        }
        Ok(())
    }
    pub fn remove(&mut self, t: Target) {
        match t {
            Target::Raster(id) => {
                self.rasters.remove(&id);
            }
            Target::Vector(id) => {
                self.vectors.remove(&id);
            }
            _ => {}
        }
    }
}
pub fn expression(d: &Document, t: Target) -> Expression {
    d.scene
        .catalog
        .info(crate::layer_catalog::key(d, t))
        .map_or(Expression::Color, |i| i.expression)
}
pub fn active(d: &Document, t: Target) -> bool {
    d.scene.appearance.get(t).is_some_and(Style::active) || expression(d, t) != Expression::Color
}
pub fn color(mut p: [u8; 4], expression: Expression) -> [u8; 4] {
    if expression != Expression::Color && p[3] > 0 {
        let value = ((p[0] as u32 * 54 + p[1] as u32 * 183 + p[2] as u32 * 19 + 128) / 256) as u8;
        let value = if expression == Expression::Mono {
            if value >= 128 { 255 } else { 0 }
        } else {
            value
        };
        p[..3].fill(value);
    }
    p
}
pub fn capture_scope(d: &Document) -> Result<Shape, String> {
    if d.scene.selection.is_none() && d.scene.selection_mask.is_none() || d.scene.selection_empty()
    {
        return Err("Create a nonempty area selection first".into());
    }
    let inv = d.scene.transform.inverse();
    let v = d.scene.selection_translation;
    Ok(Shape {
        rect: d.scene.selection,
        mask: d.scene.selection_mask.clone(),
        translation: [
            inv.0[0] * v[0] + inv.0[2] * v[1],
            inv.0[1] * v[0] + inv.0[3] * v[1],
        ],
    })
}
pub fn prepare(d: &Document, t: Target, s: Style) -> Result<DocumentState, String> {
    prepare_expression(d, t, s, None)
}
pub fn prepare_expression(
    d: &Document,
    t: Target,
    s: Style,
    value: Option<Expression>,
) -> Result<DocumentState, String> {
    if d.scene
        .file_objects
        .get(crate::layer_catalog::key(d, t))
        .is_some()
    {
        return Err("Edit the linked source document, then refresh its cache".into());
    }

    if !s.valid()
        || !p::targets(d).contains(&t)
        || !matches!(t, Target::Raster(_) | Target::Vector(_))
        || crate::layer_effects::adjustment(d, t).is_some()
        || d.memory_bytes() > DEFAULT_DOCUMENT_BUDGET / 8
    {
        return Err("Invalid display recipe, source or snapshot limit".into());
    }
    if p::inherited(d, t).locked {
        return Err("Unlock the layer before changing its display recipe".into());
    }
    let mut state = d.snapshot();
    state.scene.appearance.set(t, s)?;
    if let Some(value) = value {
        let key = crate::layer_catalog::key(d, t);
        let mut info = state.scene.catalog.info(key).cloned().unwrap_or_default();
        info.expression = value;
        if info.kind.is_some_and(|k| {
            matches!(
                k,
                crate::layer_search::Kind::RasterColor
                    | crate::layer_search::Kind::RasterGray
                    | crate::layer_search::Kind::RasterMono
                    | crate::layer_search::Kind::VectorColor
                    | crate::layer_search::Kind::VectorGray
                    | crate::layer_search::Kind::VectorMono
            )
        }) {
            info.kind = None;
        }
        state.scene.catalog.set(key, info);
    }
    let preview = crate::layer_merge::preview(&state, d.current_frame)?;
    Caches::new(&preview, None)?;
    if !state.scene.valid()
        || serde_json::to_vec(&state.scene)
            .map_err(|e| e.to_string())?
            .len()
            > 4 * 1024 * 1024
    {
        return Err("Display recipe exceeds scene/serialization limits".into());
    }
    p::validate_edit(d, &state)?;
    Ok(state)
}
pub(crate) fn shift(scene: &mut crate::scene::Scene, t: crate::line_tools::Target, delta: Point) {
    let inv = scene.transform.inverse();
    let v = [
        inv.0[0] * delta[0] + inv.0[2] * delta[1],
        inv.0[1] * delta[0] + inv.0[3] * delta[1],
    ];
    let style = match t {
        crate::line_tools::Target::Raster(n) => {
            scene.appearance.rasters.get_mut(&scene.layer_ids.id(n))
        }
        crate::line_tools::Target::Vector(id) => scene.appearance.vectors.get_mut(&id),
        _ => None,
    };
    if let Some(shape) = style.and_then(|s| s.scope.as_mut()) {
        for (a, b) in shape.translation.iter_mut().zip(v) {
            *a += b;
        }
    }
}
pub struct Cache {
    pub extent: PlacedRaster,
    pub pixels: Vec<[u8; 4]>,
}
impl Cache {
    pub fn bounds(&self) -> Option<crate::scene::Rect> {
        let mut min = [self.extent.width, self.extent.height];
        let mut max = [0u32; 2];
        for (n, _) in self.pixels.iter().enumerate().filter(|(_, p)| p[3] > 0) {
            let x = n as u32 % self.extent.width;
            let y = n as u32 / self.extent.width;
            min = [min[0].min(x), min[1].min(y)];
            max = [max[0].max(x + 1), max[1].max(y + 1)];
        }
        if min[0] >= max[0] || min[1] >= max[1] {
            None
        } else {
            Some(crate::scene::Rect {
                min: [
                    self.extent.offset[0] as f64 + min[0] as f64,
                    self.extent.offset[1] as f64 + min[1] as f64,
                ],
                max: [
                    self.extent.offset[0] as f64 + max[0] as f64,
                    self.extent.offset[1] as f64 + max[1] as f64,
                ],
            })
        }
    }
    pub fn pixel(&self, point: [i64; 2]) -> [u8; 4] {
        let x = point[0] - self.extent.offset[0];
        let y = point[1] - self.extent.offset[1];
        if x < 0 || y < 0 || x >= self.extent.width as i64 || y >= self.extent.height as i64 {
            [0; 4]
        } else {
            self.pixels[y as usize * self.extent.width as usize + x as usize]
        }
    }
}
pub struct Caches {
    pub rasters: Vec<Option<Cache>>,
    pub vectors: Vec<Option<Cache>>,
}
fn vector_extent(
    scene: &crate::scene::Scene,
    l: &crate::line_tools::Layer,
) -> Result<Option<PlacedRaster>, String> {
    if l.paths.is_empty() {
        return Ok(None);
    }
    let mut min = [f64::INFINITY; 2];
    let mut max = [f64::NEG_INFINITY; 2];
    for path in &l.paths {
        let b = path.bounds();
        for p in [b.min, [b.max[0], b.min[1]], b.max, [b.min[0], b.max[1]]] {
            let p = scene.transform.map(p);
            for i in 0..2 {
                min[i] = min[i].min(p[i]);
                max[i] = max[i].max(p[i]);
            }
        }
    }
    if min
        .iter()
        .chain(&max)
        .any(|v| !v.is_finite() || v.abs() > 1_000_000_000.)
    {
        return Err("Vector display position exceeds limits".into());
    }
    let min = min.map(|v| v.floor() as i64);
    let max = max.map(|v| v.ceil() as i64);
    if (0..2).any(|i| max[i] <= min[i] || max[i] - min[i] > 32768) {
        return Err("Vector display bounds exceed limits".into());
    }
    Ok(Some(PlacedRaster {
        width: (max[0] - min[0]) as u32,
        height: (max[1] - min[1]) as u32,
        offset: min,
    }))
}
pub(crate) fn limits(
    width: u32,
    height: u32,
    layers: &[crate::document::RasterLayer],
    scene: &crate::scene::Scene,
) -> Result<(), String> {
    let mut count = 0usize;
    let mut work = 0usize;
    let mut check = |e: PlacedRaster,
                     style: Option<&Style>,
                     expression: Expression,
                     points: usize|
     -> Result<(), String> {
        if !style.is_some_and(Style::active) && expression == Expression::Color {
            return Ok(());
        }
        let r = style
            .filter(|s| s.active())
            .map_or(0, |s| s.border as usize);
        let w = e.width as usize + 2 * r;
        let h = e.height as usize + 2 * r;
        let n = w.saturating_mul(h);
        count = count.saturating_add(n);
        let mask = style
            .filter(|s| s.active())
            .and_then(|s| s.scope.as_ref())
            .and_then(|s| s.mask.as_ref())
            .map_or(0, |m| usize::BITS - m.runs.len().max(1).leading_zeros());
        work = work.saturating_add(n.saturating_mul(8 + points + mask as usize));
        if w > 32768
            || h > 32768
            || n > 4 * 1024 * 1024
            || count > 16 * 1024 * 1024
            || work > 64 * 1024 * 1024
        {
            return Err("Display recipe exceeds extent, cache or sampling work limit".into());
        }
        Ok(())
    };
    for (n, l) in layers.iter().enumerate() {
        let id = scene.layer_ids.id(n);
        let expression = scene
            .catalog
            .rasters
            .get(&id)
            .map_or(Expression::Color, |i| i.expression);
        check(
            l.extent(width, height),
            scene.appearance.rasters.get(&id),
            expression,
            0,
        )?;
    }
    for l in &scene.lines.layers {
        let style = scene.appearance.vectors.get(&l.id);
        let expression = scene
            .catalog
            .vectors
            .get(&l.id)
            .map_or(Expression::Color, |i| i.expression);
        if !style.is_some_and(Style::active) && expression == Expression::Color {
            continue;
        }
        if let Some(e) = vector_extent(scene, l)? {
            check(
                e,
                style,
                expression,
                l.paths.iter().map(|p| p.points.len()).sum(),
            )?;
        }
    }
    Ok(())
}
impl Caches {
    pub fn new(d: &Document, stroke: Option<&crate::painting::Stroke>) -> Result<Self, String> {
        limits(d.width, d.height, &d.layers, &d.scene)?;
        let mut total = 0usize;
        let mut make = |t: Target,
                        extent: PlacedRaster,
                        mut pixel: Box<dyn FnMut([i64; 2]) -> [u8; 4] + '_>|
         -> Result<Option<Cache>, String> {
            let style = d.scene.appearance.get(t).cloned().unwrap_or_default();
            let expression = expression(d, t);
            if !style.active() && expression == Expression::Color {
                return Ok(None);
            }
            let r = if style.active() {
                style.border as i64
            } else {
                0
            };
            let width = extent.width as i64 + 2 * r;
            let height = extent.height as i64 + 2 * r;
            if !(1..=32768).contains(&width) || !(1..=32768).contains(&height) {
                return Err("Display recipe extent exceeds 32768 pixels".into());
            }
            let n = (width * height) as usize;
            total = total.saturating_add(n);
            if n > 4 * 1024 * 1024 || total > 16 * 1024 * 1024 {
                return Err("Display recipe cache exceeds layer/page pixel budget".into());
            }
            let extent = PlacedRaster {
                width: width as u32,
                height: height as u32,
                offset: [extent.offset[0] - r, extent.offset[1] - r],
            };
            let w = width as usize;
            let h = height as usize;
            let mut pixels = Vec::with_capacity(n);
            for y in 0..h {
                for x in 0..w {
                    pixels.push(color(
                        pixel([extent.offset[0] + x as i64, extent.offset[1] + y as i64]),
                        expression,
                    ));
                }
            }
            if style.active() {
                let alpha = pixels.iter().map(|p| p[3]).collect::<Vec<_>>();
                let horizontal = maxima(&alpha, w, h, style.border as usize, true);
                let expanded = maxima(&horizontal, w, h, style.border as usize, false);
                drop(horizontal);
                let inv = d.scene.transform.inverse();
                for y in 0..h {
                    for x in 0..w {
                        let i = y * w + x;
                        let raw = pixels[i];
                        let mut ink = raw;
                        if ink[3] > 0 {
                            if let Some(c) = style.color {
                                ink[..3].copy_from_slice(&c);
                            }
                            if style.rim > 0. {
                                let neighbor = [
                                    x.checked_sub(1).map(|a| y * w + a),
                                    (x + 1 < w).then_some(y * w + x + 1),
                                    y.checked_sub(1).map(|a| a * w + x),
                                    (y + 1 < h).then_some((y + 1) * w + x),
                                ]
                                .into_iter()
                                .map(|n| n.map_or(0, |n| alpha[n]))
                                .min()
                                .unwrap_or(0);
                                let edge = 1. - neighbor as f64 / ink[3] as f64;
                                for c in &mut ink[..3] {
                                    *c =
                                        (*c as f64 * (1. - style.rim * edge.max(0.))).round() as u8;
                                }
                            }
                        }
                        let mut border = style.border_color;
                        border[3] =
                            ((expanded[i].saturating_sub(alpha[i]) as u32 * border[3] as u32 + 127)
                                / 255) as u8;
                        let after = crate::render::over(border, ink);
                        let original = inv.map([
                            extent.offset[0] as f64 + x as f64 + 0.5,
                            extent.offset[1] as f64 + y as f64 + 0.5,
                        ]);
                        let weight = style.weight(original);
                        pixels[i] = if weight == 0. {
                            raw
                        } else if weight == 1. {
                            after
                        } else {
                            mix(raw, after, weight)
                        };
                    }
                }
            }
            Ok(Some(Cache { extent, pixels }))
        };
        let mut rasters = Vec::with_capacity(d.layers.len());
        for (n, l) in d.layers.iter().enumerate() {
            rasters.push(make(
                p::raster(d, n),
                l.extent(d.width, d.height),
                Box::new(move |point| {
                    stroke
                        .and_then(|s| {
                            if point.iter().all(|v| *v >= 0 && *v <= u32::MAX as i64) {
                                s.pixel(d, n, l, point[0] as u32, point[1] as u32)
                            } else {
                                None
                            }
                        })
                        .unwrap_or_else(|| l.pixel(d.width, d.height, point[0], point[1]))
                }),
            )?);
        }
        let mut vectors = Vec::with_capacity(d.scene.lines.layers.len());
        for l in &d.scene.lines.layers {
            let t = Target::Vector(l.id);
            if !d.scene.appearance.get(t).is_some_and(Style::active)
                && expression(d, t) == Expression::Color
            {
                vectors.push(None);
                continue;
            }
            let Some(bounds) = crate::layer_catalog::bounds(d, Key::Layer(t)) else {
                vectors.push(None);
                continue;
            };
            if bounds
                .min
                .iter()
                .chain(&bounds.max)
                .any(|v| !v.is_finite() || v.abs() > 1_000_000_000.)
            {
                return Err("Vector display position exceeds limits".into());
            }
            let min = bounds.min.map(|v| v.floor() as i64);
            let max = bounds.max.map(|v| v.ceil() as i64);
            if max.iter().zip(min).any(|(b, a)| *b <= a || *b - a > 32768) {
                return Err("Vector display bounds exceed limits".into());
            }
            let extent = PlacedRaster {
                width: (max[0] - min[0]) as u32,
                height: (max[1] - min[1]) as u32,
                offset: min,
            };
            let radius = if d.scene.appearance.get(t).is_some_and(Style::active) {
                d.scene.appearance.get(t).unwrap().border as usize
            } else {
                0
            };
            let work = (extent.width as usize + 2 * radius)
                .saturating_mul(extent.height as usize + 2 * radius)
                .saturating_mul(l.paths.iter().map(|p| p.points.len()).sum::<usize>().max(1));
            if work > 64 * 1024 * 1024 {
                return Err("Vector display sampling exceeds work budget".into());
            }
            vectors.push(make(
                t,
                extent,
                Box::new(move |point| {
                    let original = d
                        .scene
                        .transform
                        .inverse()
                        .map([point[0] as f64 + 0.5, point[1] as f64 + 0.5]);
                    l.paths
                        .iter()
                        .fold([0; 4], |out, p| crate::render::over(out, p.pixel(original)))
                }),
            )?);
        }
        Ok(Self { rasters, vectors })
    }
    pub fn raster(&self, d: &Document, n: usize, point: [i64; 2]) -> [u8; 4] {
        if let Some(c) = d.scene.file_objects.pixel(
            crate::layer_search::Key::Layer(Target::Raster(d.scene.layer_ids.id(n))),
            d.scene
                .transform
                .inverse()
                .map(point.map(|v| v as f64 + 0.5)),
        ) {
            return c;
        }
        self.rasters.get(n).and_then(|c| c.as_ref()).map_or_else(
            || d.layers[n].pixel(d.width, d.height, point[0], point[1]),
            |c| c.pixel(point),
        )
    }
}
fn maxima(values: &[u8], w: usize, h: usize, r: usize, horizontal: bool) -> Vec<u8> {
    if r == 0 {
        return values.to_vec();
    }
    let mut out = vec![0; values.len()];
    let (lines, len) = if horizontal { (h, w) } else { (w, h) };
    let index = |line: usize, i: usize| {
        if horizontal {
            line * w + i
        } else {
            i * w + line
        }
    };
    for line in 0..lines {
        let mut queue = VecDeque::<usize>::new();
        let mut next = 0;
        for i in 0..len {
            let end = i.saturating_add(r).min(len - 1);
            while next <= end {
                while queue
                    .back()
                    .is_some_and(|n| values[index(line, *n)] <= values[index(line, next)])
                {
                    queue.pop_back();
                }
                queue.push_back(next);
                next += 1;
            }
            let start = i.saturating_sub(r);
            while queue.front().is_some_and(|n| *n < start) {
                queue.pop_front();
            }
            out[index(line, i)] = values[index(line, *queue.front().unwrap())];
        }
    }
    out
}
fn mix(a: [u8; 4], b: [u8; 4], w: f64) -> [u8; 4] {
    let alpha = (a[3] as f64 * (1. - w) + b[3] as f64 * w) / 255.;
    if alpha == 0. {
        return [0; 4];
    }
    let mut out = [0; 4];
    for i in 0..3 {
        out[i] = ((a[i] as f64 * a[3] as f64 * (1. - w) + b[i] as f64 * b[3] as f64 * w)
            / (255. * alpha))
            .round() as u8;
    }
    out[3] = (alpha * 255.).round() as u8;
    out
}
