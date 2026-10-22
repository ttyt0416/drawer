//! Independent, ordered, editable raster surface effects. All distances are document pixels.
use crate::{
    color_precision::Sample,
    document::{DEFAULT_DOCUMENT_BUDGET, Document, PlacedRaster},
    layer_effects::Mask,
    scene::{Affine, Point, Rect},
    sizing::DocumentState,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Light {
    pub x: f64,
    pub y: f64,
}
impl Default for Light {
    fn default() -> Self {
        Self { x: -0.6, y: -0.8 }
    }
}
impl<'de> Deserialize<'de> for Light {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(d)?;
        let pair = value
            .get("x")
            .and_then(|v| v.as_f64())
            .zip(value.get("y").and_then(|v| v.as_f64()));
        Ok(pair
            .filter(|(x, y)| (-1. ..=1.).contains(x) && (-1. ..=1.).contains(y))
            .map_or_else(Self::default, |(x, y)| Self { x, y }))
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum Operation {
    Shadow {
        #[serde(default)]
        blur: u32,
        #[serde(default)]
        direction: f64,
        #[serde(default)]
        distance: f64,
        #[serde(default)]
        color: [u8; 3],
        #[serde(default)]
        opacity: f64,
    },
    Glow {
        #[serde(default)]
        intensity: f64,
        #[serde(default)]
        spread: u32,
        #[serde(default)]
        blur: u32,
        #[serde(default)]
        color: [u8; 3],
    },
    Emboss {
        #[serde(default)]
        shadow: f64,
        #[serde(default)]
        gloss: f64,
        #[serde(default)]
        height: f64,
        #[serde(default)]
        smoothing: u32,
        #[serde(default)]
        light: Light,
    },
    Crystal {
        #[serde(default = "unit_cell")]
        cell: u32,
        #[serde(default)]
        irregularity: f64,
        #[serde(default)]
        offset: Point,
        #[serde(default)]
        tiling: bool,
        #[serde(default)]
        seed: u64,
    },
}
fn unit_cell() -> u32 {
    1
}

impl Operation {
    pub fn defaults() -> [Self; 4] {
        [
            Self::Shadow {
                blur: 4,
                direction: 45.,
                distance: 8.,
                color: [0; 3],
                opacity: 0.6,
            },
            Self::Glow {
                intensity: 0.7,
                spread: 4,
                blur: 4,
                color: [255, 240, 180],
            },
            Self::Emboss {
                shadow: 0.7,
                gloss: 0.4,
                height: 6.,
                smoothing: 2,
                light: Light::default(),
            },
            Self::Crystal {
                cell: 12,
                irregularity: 0.6,
                offset: [0.; 2],
                tiling: false,
                seed: 1,
            },
        ]
    }
    pub fn index(self) -> usize {
        match self {
            Self::Shadow { .. } => 0,
            Self::Glow { .. } => 1,
            Self::Emboss { .. } => 2,
            Self::Crystal { .. } => 3,
        }
    }
    pub fn label(self) -> &'static str {
        [
            "Drop shadow",
            "Glow",
            "Emboss / raised surface",
            "Crystallize",
        ][self.index()]
    }
    pub fn valid(self) -> bool {
        match self {
            Self::Shadow {
                blur,
                direction,
                distance,
                opacity,
                ..
            } => {
                blur <= 32
                    && (-36000. ..=36000.).contains(&direction)
                    && (0. ..=256.).contains(&distance)
                    && (0. ..=1.).contains(&opacity)
            }
            Self::Glow {
                intensity,
                spread,
                blur,
                ..
            } => (0. ..=4.).contains(&intensity) && spread <= 32 && blur <= 32,
            Self::Emboss {
                shadow,
                gloss,
                height,
                smoothing,
                light,
            } => {
                (0. ..=2.).contains(&shadow)
                    && (0. ..=2.).contains(&gloss)
                    && (0. ..=32.).contains(&height)
                    && smoothing <= 32
                    && (-1. ..=1.).contains(&light.x)
                    && (-1. ..=1.).contains(&light.y)
            }
            Self::Crystal {
                cell,
                irregularity,
                offset,
                ..
            } => {
                (1..=256).contains(&cell)
                    && (0. ..=1.).contains(&irregularity)
                    && offset.iter().all(|v| v.is_finite() && v.abs() <= 1e6)
            }
        }
    }
    pub fn identity(self) -> bool {
        match self {
            Self::Shadow { opacity, .. } => opacity == 0.,
            Self::Glow { intensity, .. } => intensity == 0.,
            Self::Emboss {
                height,
                shadow,
                gloss,
                ..
            } => height == 0. || shadow == 0. && gloss == 0.,
            Self::Crystal {
                cell, irregularity, ..
            } => cell == 1 && irregularity == 0.,
        }
    }
    fn padding(self) -> i64 {
        match self {
            Self::Shadow { blur, distance, .. } => distance.ceil() as i64 + blur as i64 * 3 + 2,
            Self::Glow { spread, blur, .. } => spread as i64 + blur as i64 * 3,
            _ => 0,
        }
    }
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Scope {
    pub rect: Option<Rect>,
    pub mask: Option<crate::selection::Mask>,
    pub space: Affine,
    pub translation: Point,
}
impl Scope {
    pub fn capture(d: &Document) -> Self {
        Self {
            rect: d.scene.selection,
            mask: d.scene.selection_mask.clone(),
            space: d.scene.transform,
            translation: d.scene.selection_translation,
        }
    }
    fn weight(&self, p: Point) -> f64 {
        let q = self
            .space
            .inverse()
            .map([p[0] - self.translation[0], p[1] - self.translation[1]]);
        self.mask.as_ref().map_or_else(
            || f64::from(self.rect.is_none_or(|r| r.contains(q))),
            |m| m.coverage(q) as f64 / 255.,
        )
    }
    fn valid(&self) -> bool {
        self.space.valid()
            && self.rect.is_none_or(|r| r.valid())
            && self.mask.as_ref().is_none_or(crate::selection::Mask::valid)
            && self
                .translation
                .iter()
                .all(|v| v.is_finite() && v.abs() <= 1e9)
    }
    fn bytes(&self) -> usize {
        128 + self.mask.as_ref().map_or(0, |m| {
            m.runs.len() * std::mem::size_of::<crate::selection::Run>()
        })
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub operation: Operation,
    #[serde(default)]
    pub enabled: bool,
    pub scope: Option<Scope>,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Stack {
    pub effects: Vec<Entry>,
    /// This mask clips the final output; the layer's ordinary mask clips the source.
    pub output_mask: Option<Mask>,
}
impl Stack {
    pub fn valid(&self) -> bool {
        self.effects.len() <= 16
            && self
                .effects
                .iter()
                .all(|e| e.operation.valid() && e.scope.as_ref().is_none_or(Scope::valid))
            && self.output_mask.as_ref().is_none_or(Mask::valid)
            && self.bytes() <= 1024 * 1024
    }
    pub fn active(&self) -> bool {
        self.effects
            .iter()
            .any(|e| e.enabled && !e.operation.identity())
            || self.output_mask.as_ref().is_some_and(|m| m.enabled)
    }
    fn bytes(&self) -> usize {
        self.effects
            .iter()
            .map(|e| 192 + e.scope.as_ref().map_or(0, Scope::bytes))
            .sum::<usize>()
            + self.output_mask.as_ref().map_or(0, |m| {
                128 + m.coverage.runs.len() * std::mem::size_of::<crate::selection::Run>()
            })
    }
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Data {
    /// Stable source-layer identity -> ordered editable effects (no cross-source recursion).
    pub rasters: BTreeMap<u64, Stack>,
    /// Baked results retain their recipe independently of the preserved source.
    pub baked: BTreeMap<u64, Stack>,
}
impl Data {
    pub fn is_default(&self) -> bool {
        self.rasters.is_empty() && self.baked.is_empty()
    }
    pub fn valid(&self) -> bool {
        self.rasters.len() + self.baked.len() <= 256
            && self
                .rasters
                .iter()
                .chain(&self.baked)
                .all(|(id, s)| *id > 0 && s.valid())
            && self.bytes() <= 2 * 1024 * 1024
    }
    pub fn bytes(&self) -> usize {
        self.rasters
            .values()
            .chain(self.baked.values())
            .map(Stack::bytes)
            .sum()
    }
    pub fn remove(&mut self, id: u64) {
        self.rasters.remove(&id);
        self.baked.remove(&id);
    }
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Presets {
    pub values: [Option<Operation>; 4],
}
impl Presets {
    pub fn valid(&self) -> bool {
        self.values
            .iter()
            .enumerate()
            .all(|(i, s)| s.is_none_or(|s| s.valid() && s.index() == i))
    }
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
    pub fn get(&self, index: usize) -> Operation {
        self.values[index]
            .filter(|s| s.valid() && s.index() == index)
            .unwrap_or(Operation::defaults()[index])
    }
}

pub struct Image {
    pub extent: PlacedRaster,
    pub pixels: Vec<[f32; 4]>,
}
impl Image {
    pub fn sample(&self, p: [i64; 2]) -> Sample {
        let x = p[0] - self.extent.offset[0];
        let y = p[1] - self.extent.offset[1];
        if x < 0 || y < 0 || x >= self.extent.width as i64 || y >= self.extent.height as i64 {
            Sample::default()
        } else {
            Sample::new(
                self.pixels[y as usize * self.extent.width as usize + x as usize].map(f64::from),
            )
            .unwrap()
        }
    }
}
pub fn extent(source: PlacedRaster, stack: &Stack) -> Result<PlacedRaster, String> {
    if !stack.valid() {
        return Err("Invalid surface effect stack".into());
    }
    let pad: i64 = stack
        .effects
        .iter()
        .filter(|e| e.enabled && !e.operation.identity())
        .map(|e| e.operation.padding())
        .sum();
    let out = PlacedRaster {
        offset: source.offset.map(|v| v - pad),
        width: source.width.saturating_add((pad * 2) as u32),
        height: source.height.saturating_add((pad * 2) as u32),
    };
    let n = out.width as usize * out.height as usize;
    if !out.valid()
        || n > 1024 * 1024
        || n.saturating_mul(stack.effects.len().max(1) * 80) > 64 * 1024 * 1024
    {
        return Err(
            "Surface effects exceed extent/work budget (1M pixels / 64M operations)".into(),
        );
    }
    Ok(out)
}
pub fn render(
    source: PlacedRaster,
    stack: &Stack,
    budget: usize,
    mut pixel: impl FnMut([i64; 2]) -> Result<Sample, String>,
    mut tick: impl FnMut(usize, usize) -> Result<(), String>,
) -> Result<Image, String> {
    render_with_appearance(source, stack, budget, &mut pixel, None, &mut tick)
}
pub(crate) fn render_with_appearance(
    source: PlacedRaster,
    stack: &Stack,
    budget: usize,
    mut pixel: impl FnMut([i64; 2]) -> Result<Sample, String>,
    style: Option<&crate::appearance::Style>,
    mut tick: impl FnMut(usize, usize) -> Result<(), String>,
) -> Result<Image, String> {
    let extent = extent(source, stack)?;
    let n = extent.width as usize * extent.height as usize;
    if n.saturating_mul(96) > budget {
        return Err("Surface effects exceed memory budget".into());
    }
    tick(0, n * (stack.effects.len() + 1))?;
    let mut image = Image {
        extent,
        pixels: Vec::with_capacity(n),
    };
    for i in 0..n {
        if i % 128 == 0 {
            tick(i, n * (stack.effects.len() + 1))?;
        }
        let p = point(extent, i);
        image.pixels.push(pixel(p)?.components().map(|v| v as f32));
    }
    if let Some(style) = style.filter(|s| s.enabled && (s.border > 0 || s.rim > 0.)) {
        image.pixels = crate::surface_kernels::appearance(&image, style, |done| {
            tick(done, n * (stack.effects.len() + 1))
        })?;
    }
    for (j, e) in stack.effects.iter().enumerate() {
        if !e.enabled || e.operation.identity() {
            continue;
        }
        let after = crate::surface_kernels::apply(&image, source, e.operation, |done| {
            tick(n * (j + 1) + done, n * (stack.effects.len() + 1))
        })?;
        for (i, p) in after.into_iter().enumerate() {
            if i % 128 == 0 {
                tick(n * (j + 1) + i, n * (stack.effects.len() + 1))?;
            }
            let weight = e
                .scope
                .as_ref()
                .map_or(1., |s| s.weight(point(extent, i).map(|v| v as f64 + 0.5)));
            if weight > 0. {
                image.pixels[i] = Sample::new(image.pixels[i].map(f64::from))?
                    .mix(Sample::new(p.map(f64::from))?, weight)?
                    .components()
                    .map(|v| v as f32);
            }
        }
    }
    if let Some(m) = &stack.output_mask {
        for i in 0..n {
            if i % 128 == 0 {
                tick(i, n)?;
            }
            image.pixels[i][3] *= m.factor(point(extent, i).map(|v| v as f64 + 0.5));
        }
    }
    tick(n * (stack.effects.len() + 1), n * (stack.effects.len() + 1))?;
    Ok(image)
}
pub(crate) fn point(e: PlacedRaster, i: usize) -> [i64; 2] {
    [
        e.offset[0] + (i % e.width as usize) as i64,
        e.offset[1] + (i / e.width as usize) as i64,
    ]
}
pub fn prepare(
    d: &Document,
    layer: usize,
    stack: Stack,
    mut tick: impl FnMut(usize, usize) -> Result<(), String>,
) -> Result<DocumentState, String> {
    crate::layer_properties::paintable(d, layer)?;
    if !stack.valid() || d.memory_bytes() > DEFAULT_DOCUMENT_BUDGET / 8 {
        return Err("Invalid effect stack or source memory limit".into());
    }
    let id = d.scene.layer_ids.id(layer);
    let mut state = d.snapshot();
    if stack == Stack::default() {
        state.scene.surface_effects.rasters.remove(&id);
    } else {
        state.scene.surface_effects.rasters.insert(id, stack);
    }
    crate::layer_groups::finish(d, &state)?;
    let preview = crate::layer_merge::preview(&state, d.current_frame)?;
    crate::float_render::Renderer::effect_check(&preview, &mut tick)?;
    Ok(state)
}
pub fn valid_live(d: &Document) -> bool {
    d.scene
        .surface_effects
        .rasters
        .keys()
        .chain(d.scene.surface_effects.baked.keys())
        .all(|id| crate::layers::index(d, *id).is_some())
}
pub(crate) fn source_extent(d: &Document, layer: usize) -> PlacedRaster {
    let mut e = d.layers[layer].extent(d.width, d.height);
    if let Some(s) = d
        .scene
        .appearance
        .get(crate::layer_properties::raster(d, layer))
        .filter(|s| s.enabled)
    {
        let r = s.border as i64;
        e.offset = e.offset.map(|v| v - r);
        e.width = e.width.saturating_add((r * 2) as u32);
        e.height = e.height.saturating_add((r * 2) as u32);
    }
    e
}
pub fn capture_output_mask(d: &Document) -> Result<Mask, String> {
    if d.scene.selection.is_none() && d.scene.selection_mask.is_none() {
        return Err("Create an area selection for the output mask".into());
    }
    if d.width as usize * d.height as usize > 1024 * 1024 {
        return Err("Output mask snapshot exceeds1M pixels".into());
    }
    let mut runs = Vec::new();
    for y in 0..d.height {
        let mut x = 0;
        while x < d.width {
            let start = x;
            let alpha = d.scene.selection_coverage([x as f64 + 0.5, y as f64 + 0.5]);
            x += 1;
            while x < d.width
                && d.scene.selection_coverage([x as f64 + 0.5, y as f64 + 0.5]) == alpha
            {
                x += 1;
            }
            if alpha > 0 {
                runs.push(crate::selection::Run {
                    x: start as i32,
                    y: y as i32,
                    length: x - start,
                    alpha,
                });
                if runs.len() > 32768 {
                    return Err("Output mask run limit".into());
                }
            }
        }
    }
    Ok(Mask {
        space: Affine::default(),
        coverage: crate::selection::Mask { runs },
        offset: [0.; 2],
        enabled: true,
        linked: false,
        inverted: false,
    })
}
/// Keep the original layer intact and hidden; the replacement occupies its former tree position.
pub fn rasterize(
    d: &Document,
    layer: usize,
    stack: Stack,
    mut tick: impl FnMut(usize, usize) -> Result<(), String>,
) -> Result<DocumentState, String> {
    let state = prepare(d, layer, stack.clone(), &mut tick)?;
    let working = crate::layer_merge::preview(&state, d.current_frame)?;
    let source = crate::layer_properties::raster(&working, layer);
    let base = working.scene.clipping.binding(source).unwrap_or(source);
    let run = crate::layer_properties::targets(&working)
        .into_iter()
        .filter(|t| *t == base || working.scene.clipping.binding(*t) == Some(base))
        .collect::<Vec<_>>();
    if run.len() > 1 {
        return rasterize_run(d, &working, layer, stack, run, &mut tick);
    }
    let out = extent(source_extent(d, layer), &stack)?;
    let n = out.width as usize * out.height as usize;
    if state
        .memory_bytes()
        .saturating_mul(3)
        .saturating_add(n * 12)
        > DEFAULT_DOCUMENT_BUDGET / 4
    {
        return Err("Effect raster exceeds Undo memory budget".into());
    }
    let renderer = crate::float_render::Renderer::with_effect_progress(&working, None, &mut tick)?;
    let mut samples = Vec::with_capacity(n);
    for i in 0..n {
        if i % 128 == 0 {
            tick(i, n)?;
        }
        samples.push(renderer.surface_sample(layer, point(out, i))?);
    }
    drop(renderer);
    let mut raster = crate::document::RasterLayer {
        precise: None,
        locked: false,
        name: "Surface effect result".into(),
        pixels: vec![[0; 4]; n],
        placement: Some(out),
    };
    crate::color_storage::set_samples(&mut raster, &working.color, out, samples)?;
    let parent = crate::layers::parent(&working.scene.paper, layer);
    let created = crate::layers::prepare(
        &working,
        crate::layers::Command::Create {
            kind: crate::layers::Kind::Raster,
            name: "Surface effect result".into(),
            parent,
        },
    )?;
    let mut result = created.state;
    let index = result.layers.len() - 1;
    let id = result.scene.layer_ids.id(index);
    let old_id = result.scene.layer_ids.id(layer);
    result.layers[index] = raster;
    let list = parent.map_or(&mut result.scene.paper.roots, |p| {
        &mut result.scene.paper.folders[p].children
    });
    list.retain(|node| *node != crate::paper::Node::Raster(index));
    let position = list
        .iter()
        .position(|node| *node == crate::paper::Node::Raster(layer))
        .ok_or("Missing original layer position")?;
    list.insert(position + 1, crate::paper::Node::Raster(index));
    let props = crate::layer_properties::properties(d, source);
    result
        .scene
        .layer_properties
        .set(crate::layer_properties::Target::Raster(id), props);
    result.scene.layer_properties.set(
        source,
        crate::layer_properties::Properties {
            hidden: true,
            ..props
        },
    );
    if let Some(paper) = result.scene.paper.layers.get(&layer).copied() {
        result.scene.paper.layers.insert(
            index,
            crate::paper::Paper {
                pattern: Default::default(),
                ..paper
            },
        );
    }
    result.scene.paper.layers.entry(layer).or_default().hidden = true;
    result.scene.surface_effects.remove(old_id);
    if let Some(original) = d.scene.surface_effects.rasters.get(&old_id) {
        result
            .scene
            .surface_effects
            .rasters
            .insert(old_id, original.clone());
    }
    result.scene.surface_effects.baked.insert(id, stack);
    crate::layer_groups::finish(d, &result)?;
    tick(n, n)?;
    Ok(result)
}
fn rasterize_run(
    d: &Document,
    working: &Document,
    layer: usize,
    stack: Stack,
    run: Vec<crate::layer_properties::Target>,
    tick: &mut impl FnMut(usize, usize) -> Result<(), String>,
) -> Result<DocumentState, String> {
    use crate::{
        layer_properties::{self as p, Target},
        layers,
        paper::Node,
    };
    tick(0, 1)?;
    let base = working
        .scene
        .clipping
        .binding(p::raster(working, layer))
        .unwrap_or(p::raster(working, layer));
    let base_paper = match crate::clipping::node(working, base).ok_or("Missing clipping base")? {
        Node::Raster(n) => working
            .scene
            .paper
            .layers
            .get(&n)
            .copied()
            .unwrap_or_default(),
        Node::Folder(n) => working.scene.paper.folders[n].paper,
    };
    if base_paper.blend != crate::brush_composite::Mode::Normal {
        return Err("This clipping base uses a backdrop-dependent paper blend. Keep the live effects, or use Flatten page to include its backdrop.".into());
    }
    let nodes = run
        .iter()
        .filter_map(|t| crate::clipping::node(working, *t))
        .collect::<Vec<_>>();
    let display = crate::appearance::Caches::new(working, None)?;
    let extents =
        crate::file_objects::render_extents(working, &working.scene.paper, &nodes, &display)?;
    let mut min = [i64::MAX; 2];
    let mut max = [i64::MIN; 2];
    for e in extents {
        for k in 0..2 {
            min[k] = min[k].min(e.offset[k]);
            max[k] = max[k].max(e.offset[k] + [e.width, e.height][k] as i64);
        }
    }
    drop(display);
    let size = [max[0] - min[0], max[1] - min[1]];
    if size.iter().any(|n| !(1..=32768).contains(n)) {
        return Err("Clipping-run output extent limit".into());
    }
    let n = size[0] as usize * size[1] as usize;
    if n > 1024 * 1024
        || n.saturating_mul(working.layers.len()).saturating_mul(8) > 64 * 1024 * 1024
        || working
            .memory_bytes()
            .saturating_mul(4)
            .saturating_add(n * 96)
            > DEFAULT_DOCUMENT_BUDGET / 2
    {
        return Err("Clipping-run raster memory/work budget".into());
    }
    let extent = PlacedRaster {
        offset: min,
        width: size[0] as u32,
        height: size[1] as u32,
    };
    let renderer = crate::float_render::Renderer::with_effect_progress(working, None, tick)?;
    let mut samples = Vec::with_capacity(n);
    for i in 0..n {
        if i % 128 == 0 {
            tick(i, n)?;
        }
        samples.push(renderer.nodes_sample(&nodes, point(extent, i))?);
    }
    drop(renderer);
    let mut raster = crate::document::RasterLayer {
        precise: None,
        locked: false,
        name: "Surface effect clipping run".into(),
        pixels: vec![[0; 4]; n],
        placement: Some(extent),
    };
    crate::color_storage::set_samples(&mut raster, &working.color, extent, samples)?;
    let parent = layers::parent(&working.scene.paper, layer);
    let created = layers::prepare(
        d,
        layers::Command::Create {
            kind: layers::Kind::Raster,
            name: "Surface effect clipping run".into(),
            parent,
        },
    )?;
    let mut state = created.state;
    let index = state.layers.len() - 1;
    let id = state.scene.layer_ids.id(index);
    state.layers[index] = raster;
    let list = parent.map_or(&mut state.scene.paper.roots, |p| {
        &mut state.scene.paper.folders[p].children
    });
    list.retain(|n| *n != Node::Raster(index));
    let position = list
        .iter()
        .rposition(|n| nodes.contains(n))
        .ok_or("Clipping run position missing")?;
    list.insert(position + 1, Node::Raster(index));
    let base = working
        .scene
        .clipping
        .binding(p::raster(working, layer))
        .unwrap_or(p::raster(working, layer));
    let mut output_props = p::properties(working, base);
    output_props.opacity = 100;
    output_props.hidden = false;
    for target in run {
        let mut props = p::properties(d, target);
        props.hidden = true;
        p::set_state(d, &mut state, target, props)?;
    }
    state
        .scene
        .layer_properties
        .set(Target::Raster(id), output_props);
    state.scene.surface_effects.baked.insert(id, stack);
    crate::layer_groups::finish(d, &state)?;
    tick(2, 2)?;
    Ok(state)
}
