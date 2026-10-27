//! Floating composition of raster trees and ordered editable page sources.
#[path = "float_appearance.rs"]
mod display;
#[path = "float_sources.rs"]
mod sources;
#[path = "float_surfaces.rs"]
mod surfaces;
use crate::{
    color_precision::{Depth, Dither, Pixels, Sample},
    color_profile::WorkingSpace,
    document::Document,
    layer_properties::{self as p, Target},
    layer_search::Expression,
    paper::{Node, Paper},
    scene::Point,
};
pub(crate) struct Plan<'a> {
    cels: Vec<(u64, Renderer<'a>)>,
    export_page: bool,
    page_parts: Vec<(u64, Renderer<'a>)>,
    comic_clips: bool,
    space: WorkingSpace,
    roots: Vec<Node>,
    geometry: Box<crate::render::Renderer<'a>>,
    order: Vec<crate::render_order::Entry>,
    caches: std::collections::BTreeMap<Target, display::Cache>,
    surfaces: std::collections::BTreeMap<u64, crate::surface_effects::Image>,
}
enum PlanRef<'a> {
    Owned(Box<Plan<'a>>),
    Borrowed(&'a Plan<'a>),
}
#[derive(Default)]
pub(crate) struct Sampling<'a> {
    pub pre_effects: bool,
    pub references: Option<&'a crate::eyedropper::References>,
    pub fill: Option<(usize, crate::bucket::Settings)>,
    pub graph: Option<&'a crate::paper::Graph>,
}
pub struct Renderer<'a> {
    document: &'a Document,
    stroke: Option<&'a crate::painting::Stroke>,
    plan: PlanRef<'a>,
    sampling: Sampling<'a>,
}
impl<'a> Plan<'a> {
    pub(crate) fn display(&self, sample: Sample) -> Result<Sample, String> {
        self.space.to_srgb(sample)
    }
}
impl<'a> Renderer<'a> {
    pub fn new(d: &'a Document) -> Result<Self, String> {
        Self::preview(d, None)
    }
    pub fn preview(
        d: &'a Document,
        stroke: Option<&'a crate::painting::Stroke>,
    ) -> Result<Self, String> {
        Self::with_effect_progress(d, stroke, &mut |_, _| Ok(()))
    }
    pub fn with_effect_progress(
        d: &'a Document,
        stroke: Option<&'a crate::painting::Stroke>,
        tick: &mut impl FnMut(usize, usize) -> Result<(), String>,
    ) -> Result<Self, String> {
        Self::mode(d, stroke, tick, false)
    }
    pub fn for_export(d: &'a Document) -> Result<Self, String> {
        Self::mode(d, None, &mut |_, _| Ok(()), true)
    }
    fn mode(
        d: &'a Document,
        stroke: Option<&'a crate::painting::Stroke>,
        tick: &mut impl FnMut(usize, usize) -> Result<(), String>,
        export: bool,
    ) -> Result<Self, String> {
        if !d.scene.valid()
            || !d.scene.paper.valid_for(d.layers.len())
            || !d.color.valid_layers(&d.layers)
            || !(1..=256).contains(&d.layers.len())
            || d.layers.iter().any(|l| !l.valid(d.width, d.height))
            || d.animation.as_ref().is_some_and(|a| !a.valid())
        {
            return Err("Invalid float render document".into());
        }
        let mut renderer = Self {
            document: d,
            stroke,
            sampling: Default::default(),
            plan: PlanRef::Owned(Box::new(Plan {
                cels: d
                    .scene
                    .cels
                    .definitions
                    .iter()
                    .filter_map(|p| {
                        (if export {
                            p.export_document()
                        } else {
                            p.document()
                        })
                        .map(|d| {
                            (if export {
                                Renderer::for_export(d)
                            } else {
                                Renderer::new(d)
                            })
                            .map(|r| (p.object, r))
                        })
                    })
                    .collect::<Result<Vec<_>, _>>()?,
                export_page: export,
                page_parts: d
                    .scene
                    .page_parts
                    .entries
                    .iter()
                    .filter_map(|p| {
                        (if export {
                            p.export_document()
                        } else {
                            p.document()
                        })
                        .map(|d| {
                            (if export {
                                Renderer::for_export(d)
                            } else {
                                Renderer::new(d)
                            })
                            .map(|r| (p.object, r))
                        })
                    })
                    .collect::<Result<Vec<_>, _>>()?,
                comic_clips: crate::comics::has_clips(&d.scene),
                geometry: Box::new(crate::render::Renderer::geometry_preview(d)),
                order: crate::render_order::normalized(&d.scene),
                space: WorkingSpace::new(&d.color.profile)?,
                roots: d.scene.paper.root_nodes(d.layers.len()),
                caches: Default::default(),
                surfaces: Default::default(),
            })),
        };
        if let PlanRef::Owned(plan) = &renderer.plan
            && let Some(error) = plan.geometry.error()
        {
            return Err(error.into());
        }
        renderer.build_display_caches()?;
        renderer.build_surface_caches(tick)?;
        Ok(renderer)
    }
    pub fn effect_check(
        d: &'a Document,
        tick: &mut impl FnMut(usize, usize) -> Result<(), String>,
    ) -> Result<(), String> {
        Self::with_effect_progress(d, None, tick).map(|_| ())
    }
    pub fn display_source(
        &self,
        key: crate::layer_search::Key,
        point: Point,
    ) -> Result<Sample, String> {
        self.plan().space.to_srgb(self.source_pixel(key, point)?)
    }
    pub fn display_sample(&self, point: Point) -> Result<Sample, String> {
        if self.plan().export_page {
            let overlay = self
                .document
                .scene
                .comics
                .page
                .output_pixel(self.document, point);
            if overlay[3] != 0 {
                return Ok(Sample::from8(overlay));
            }
        }
        self.plan().space.to_srgb(self.pixel(point)?)
    }
    fn plan(&self) -> &Plan<'a> {
        match &self.plan {
            PlanRef::Owned(p) => p,
            PlanRef::Borrowed(p) => p,
        }
    }
    pub(crate) fn prepare_plan(
        d: &'a Document,
        stroke: Option<&'a crate::painting::Stroke>,
    ) -> Result<Box<Plan<'a>>, String> {
        let renderer = Self::preview(d, stroke)?;
        match renderer.plan {
            PlanRef::Owned(p) => Ok(p),
            PlanRef::Borrowed(_) => unreachable!(),
        }
    }
    pub(crate) fn prepare_export_plan(d: &'a Document) -> Result<Box<Plan<'a>>, String> {
        let renderer = Self::for_export(d)?;
        match renderer.plan {
            PlanRef::Owned(p) => Ok(p),
            PlanRef::Borrowed(_) => unreachable!(),
        }
    }
    pub(crate) fn prepared(
        d: &'a Document,
        stroke: Option<&'a crate::painting::Stroke>,
        plan: &'a Plan<'a>,
        sampling: Sampling<'a>,
    ) -> Self {
        Self {
            document: d,
            stroke,
            plan: PlanRef::Borrowed(plan),
            sampling,
        }
    }
    fn graph(&self) -> &crate::paper::Graph {
        self.sampling.graph.unwrap_or(&self.document.scene.paper)
    }
    fn raster_allowed(&self, n: usize) -> bool {
        self.sampling.references.is_none_or(|r| r.raster(n))
            && self.sampling.fill.is_none_or(|(active, s)| {
                crate::fill_reference::raster(self.document, n, active, s)
            })
    }
    fn insert_cache(&mut self, target: Target, cache: display::Cache) -> Result<(), String> {
        match &mut self.plan {
            PlanRef::Owned(p) => {
                p.caches.insert(target, cache);
                Ok(())
            }
            PlanRef::Borrowed(_) => Err("Prepared rendering data is immutable".into()),
        }
    }
    fn target(&self, node: Node) -> Target {
        match node {
            Node::Raster(n) => p::raster(self.document, n),
            Node::Folder(n) => Target::Folder(n as u16),
        }
    }
    fn paper(&self, node: Node) -> Paper {
        match node {
            Node::Raster(n) => self.graph().layers.get(&n).copied().unwrap_or_default(),
            Node::Folder(n) => self.graph().folders[n].paper,
        }
    }
    fn weight(&self, node: Node, original: Point) -> f64 {
        let t = self.target(node);
        let properties = p::properties(self.document, t);
        if properties.hidden {
            0.
        } else {
            properties.opacity as f64 / 100.
                * self.document.scene.layer_effects.mask(t, original) as f64
        }
    }
    fn shade(&self, paper: Paper, sample: Sample, original: Point) -> Result<Sample, String> {
        let point = paper.space.inverse().map(original);
        let factor = paper
            .pattern
            .factor([point[0] - paper.origin[0], point[1] - paper.origin[1]]);
        if factor == 1. {
            return Ok(sample);
        }
        let source = self.plan().space.to_srgb(sample)?;
        self.plan()
            .space
            .from_srgb(source.with_rgb(source.rgb().map(|v| v * factor))?)
    }
    fn raster_raw(&self, n: usize, point: [i64; 2], original: Point) -> Result<Sample, String> {
        let d = self.document;
        let t = p::raster(d, n);
        if let Some(cache) = d
            .scene
            .file_objects
            .pixel(crate::layer_search::Key::Layer(t), original)
        {
            return self.plan().space.from_srgb(Sample::from8(cache));
        }
        Ok(self
            .stroke
            .and_then(|s| s.float_pixel(d, n, point[0], point[1]))
            .unwrap_or_else(|| d.layers[n].sample(d.width, d.height, point[0], point[1])))
    }
    fn raster(&self, n: usize, point: [i64; 2], original: Point) -> Result<Sample, String> {
        if self.plan().comic_clips
            && crate::comics::coverage(
                self.document,
                crate::layer_search::Key::Layer(p::raster(self.document, n)),
                original,
            ) == 0.
        {
            return Ok(Sample::default());
        }
        let d = self.document;
        let t = p::raster(d, n);
        if !self.raster_allowed(n) {
            return Ok(Sample::default());
        }
        let raw = self.raster_raw(n, point, original)?;
        if d.scene
            .file_objects
            .get(crate::layer_search::Key::Layer(t))
            .is_some()
        {
            return Ok(raw);
        }
        self.display(raw, t, point.map(|v| v as f64 + 0.5), original)
    }
    fn expression(&self, raw: Sample, t: Target) -> Result<Sample, String> {
        let d = self.document;
        let expression = crate::appearance::expression(d, t);
        let style = d.scene.appearance.get(t);
        if expression != Expression::Color {
            let settings = style
                .and_then(|s| s.expression_settings)
                .unwrap_or_default();
            self.plan()
                .space
                .from_srgb(self.plan().space.to_srgb(raw)?.expression(
                    expression,
                    settings.brightness as f64 / 255.,
                    settings.alpha as f64 / 255.,
                )?)
        } else {
            Ok(raw)
        }
    }
    fn display(
        &self,
        raw: Sample,
        t: Target,
        point: Point,
        original: Point,
    ) -> Result<Sample, String> {
        if let Some(sample) = self.cached_display(t, point) {
            return Ok(sample);
        }
        let sample = self.expression(raw, t)?;
        let style = self.document.scene.appearance.get(t);
        if let Some(style) = style.filter(|s| s.active())
            && let Some(color) = style.color
        {
            let colored = sample.with_rgb(color.map(|v| v as f64 / 255.))?;
            return self.plan().space.mix(
                sample,
                colored,
                style.weight(original),
                crate::color_mix::Space::Standard,
                0.,
            );
        }
        Ok(sample)
    }
    fn source(
        &self,
        node: Node,
        point: [i64; 2],
        original: Point,
        depth: usize,
    ) -> Result<Sample, String> {
        if depth > 16 {
            return Err("Float folder nesting limit".into());
        }
        let d = self.document;
        let paper = self.paper(node);
        if let Node::Folder(n) = node
            && self.graph().folders[n].through
            && self
                .document
                .scene
                .file_objects
                .get(crate::layer_search::Key::Layer(self.target(node)))
                .is_none()
        {
            return Ok(Sample::default());
        }
        if let Node::Raster(n) = node
            && self
                .document
                .scene
                .render_order
                .deferred(self.document.scene.layer_ids.id(n))
        {
            return Ok(Sample::default());
        }
        if paper.hidden {
            return Ok(Sample::default());
        }
        if let Some(cache) = self
            .document
            .scene
            .file_objects
            .pixel(crate::layer_search::Key::Layer(self.target(node)), original)
        {
            return self
                .shade(paper, Sample::from8(cache), original)?
                .coverage(self.weight(node, original));
        }
        if !self.sampling.pre_effects
            && let Node::Raster(n) = node
            && let Some(cache) = self.plan().surfaces.get(&d.scene.layer_ids.id(n))
        {
            return cache
                .sample(point)
                .coverage(p::properties(d, self.target(node)).opacity as f64 / 100.);
        }
        let raw = match node {
            Node::Raster(n) => {
                if self
                    .document
                    .scene
                    .render_order
                    .deferred(self.document.scene.layer_ids.id(n))
                {
                    Sample::default()
                } else {
                    self.raster(n, point, original)?
                }
            }
            Node::Folder(n) => self.nodes(
                &self.graph().folders[n].children,
                Sample::default(),
                point,
                original,
                depth + 1,
            )?,
        };
        self.shade(paper, raw, original)?
            .coverage(self.weight(node, original))
    }
    fn correction(
        &self,
        node: Node,
        back: Sample,
        original: Point,
    ) -> Result<Option<Sample>, String> {
        let target = self.target(node);
        let Some(adjustment) = crate::layer_effects::adjustment(self.document, target) else {
            return Ok(None);
        };
        if self.paper(node).hidden || adjustment.identity() {
            return Ok(Some(back));
        }
        let corrected = self.plan().space.adjusted(back, adjustment)?;
        let rgb = corrected.rgb();
        let corrected = Sample::new([rgb[0], rgb[1], rgb[2], 1.])?;
        Ok(Some(self.plan().space.atop(
            back,
            corrected,
            crate::layer_blend::mode(self.document, target),
            self.weight(node, original),
        )?))
    }
    fn put(
        &self,
        back: Sample,
        source: Sample,
        node: Node,
        original: Point,
    ) -> Result<Sample, String> {
        let mode = crate::layer_blend::mode(self.document, self.target(node));
        let paper = self.paper(node);
        if !mode.is_normal() {
            return self.plan().space.compose(back, source, mode, 1.);
        }
        self.plan().space.brush(
            back,
            source,
            paper.blend,
            1.,
            paper
                .space
                .inverse()
                .map(original)
                .map(|v| v.floor() as i64),
            paper.seed,
        )
    }
    fn clipped(
        &self,
        base: Sample,
        top: Sample,
        node: Node,
        original: Point,
    ) -> Result<Sample, String> {
        let mode = crate::layer_blend::mode(self.document, self.target(node));
        let paper = self.paper(node);
        if !mode.is_normal() || paper.blend == crate::brush_composite::Mode::Normal {
            return self.plan().space.atop(base, top, mode, 1.);
        }
        if base.alpha() == 0. {
            return Ok(base);
        }
        let b = base.rgb();
        let opaque = Sample::new([b[0], b[1], b[2], 1.])?;
        let result = self.plan().space.brush(
            opaque,
            top,
            paper.blend,
            1.,
            paper
                .space
                .inverse()
                .map(original)
                .map(|v| v.floor() as i64),
            paper.seed,
        )?;
        let rgb = result.rgb();
        Sample::new([rgb[0], rgb[1], rgb[2], base.alpha()])
    }
    fn nodes(
        &self,
        nodes: &[Node],
        mut back: Sample,
        point: [i64; 2],
        original: Point,
        depth: usize,
    ) -> Result<Sample, String> {
        if depth > 16 {
            return Err("Float folder nesting limit".into());
        }
        let d = self.document;
        let mut i = 0;
        while i < nodes.len() {
            let node = nodes[i];
            let key = self.target(node);
            if d.scene.clipping.binding(key).is_some() {
                i += 1;
                continue;
            }
            if let Node::Raster(n) = node
                && !self.raster_allowed(n)
            {
                i += 1;
                continue;
            }
            if let Some(corrected) = self.correction(node, back, original)? {
                back = corrected;
                i += 1;
                continue;
            }
            if let Node::Folder(n) = node
                && self.graph().folders[n].through
                && d.scene
                    .file_objects
                    .get(crate::layer_search::Key::Layer(key))
                    .is_none()
            {
                if !self.paper(node).hidden {
                    let result = self.nodes(
                        &self.graph().folders[n].children,
                        back,
                        point,
                        original,
                        depth + 1,
                    )?;
                    back = self.plan().space.mix(
                        back,
                        result,
                        self.weight(node, original),
                        crate::color_mix::Space::Standard,
                        0.,
                    )?;
                }
                i += 1;
                continue;
            }
            let mut group = self.source(node, point, original, depth)?;
            let mut next = i + 1;
            while next < nodes.len() && d.scene.clipping.binding(self.target(nodes[next])).is_some()
            {
                let top = nodes[next];
                if d.scene.clipping.binding(self.target(top)) == Some(key) {
                    group = if let Some(corrected) = self.correction(top, group, original)? {
                        corrected
                    } else if !self.sampling.pre_effects
                        && let Node::Raster(n) = top
                        && self.plan().surfaces.contains_key(&d.scene.layer_ids.id(n))
                    {
                        let base_alpha = self.pre_effect_alpha(node, point, original, depth)?;
                        let value = self.source(top, point, original, depth)?;
                        let rgb = value.rgb();
                        let ink = Sample::new([
                            rgb[0],
                            rgb[1],
                            rgb[2],
                            if base_alpha > 0. {
                                (value.alpha() / base_alpha).min(1.)
                            } else {
                                0.
                            },
                        ])?;
                        let combined = self.clipped(group, ink, top, original)?;
                        // Preserve the clipping run's alpha in its source, while allowing
                        // genuinely expanded effect coverage to appear outside the base.
                        if value.alpha() > combined.alpha() {
                            let halo = value.coverage(
                                (value.alpha() - combined.alpha())
                                    / ((1. - combined.alpha()) * value.alpha()).max(1e-12),
                            )?;
                            self.plan()
                                .space
                                .compose(halo, combined, Default::default(), 1.)?
                        } else {
                            combined
                        }
                    } else {
                        let mut value = self.source(top, point, original, depth)?;
                        if !self.sampling.pre_effects && !self.plan().surfaces.is_empty() {
                            let base_alpha = self.pre_effect_alpha(node, point, original, depth)?;
                            value = value.coverage(if group.alpha() > 0. {
                                base_alpha / group.alpha()
                            } else {
                                0.
                            })?;
                        }
                        self.clipped(group, value, top, original)?
                    };
                }
                next += 1;
            }
            back = self.put(back, group, node, original)?;
            i = next;
        }
        Ok(back)
    }
    pub fn pixel(&self, point: Point) -> Result<Sample, String> {
        let d = self.document;
        if point.iter().any(|v| !v.is_finite()) {
            return Err("Nonfinite float render point".into());
        }
        if point[0] < 0.
            || point[1] < 0.
            || point[0] >= d.width as f64
            || point[1] >= d.height as f64
        {
            return Ok(Sample::default());
        }
        let pixel = point.map(|v| v.floor() as i64);
        let original = d
            .scene
            .transform
            .inverse()
            .map(pixel.map(|v| v as f64 + 0.5));
        let raster = self.nodes(&self.plan().roots, Sample::default(), pixel, original, 0)?;
        let raster = self.timeline(raster, pixel, original)?;
        self.ordered(raster, point, d.scene.transform.inverse().map(point))
    }
    pub(crate) fn nodes_sample(&self, nodes: &[Node], point: [i64; 2]) -> Result<Sample, String> {
        let original = self
            .document
            .scene
            .transform
            .inverse()
            .map(point.map(|v| v as f64 + 0.5));
        self.nodes(nodes, Sample::default(), point, original, 0)
    }
    pub(crate) fn intrinsic_raster(&self, n: usize, point: [i64; 2]) -> Result<Sample, String> {
        let node = Node::Raster(n);
        let original = self
            .document
            .scene
            .transform
            .inverse()
            .map(point.map(|v| v as f64 + 0.5));
        self.shade(self.paper(node), self.raster(n, point, original)?, original)?
            .coverage(
                self.document
                    .scene
                    .layer_effects
                    .mask(self.target(node), original) as f64,
            )
    }
    pub(crate) fn convert_expression(
        &self,
        sample: Sample,
        mode: Expression,
    ) -> Result<Sample, String> {
        if mode == Expression::Color {
            return Ok(sample);
        }
        self.plan()
            .space
            .from_srgb(self.plan().space.to_srgb(sample)?.expression(
                mode,
                128. / 255.,
                128. / 255.,
            )?)
    }
    pub fn output(&self, depth: Depth, dither: Dither, budget: usize) -> Result<Pixels, String> {
        let d = self.document;
        let count = (d.width as usize).saturating_mul(d.height as usize);
        if count.saturating_mul(depth.bytes()) > budget {
            return Err("Float output memory budget".into());
        }
        let mut low = Vec::new();
        let mut high = Vec::new();
        if depth == Depth::Eight {
            low.try_reserve_exact(count).map_err(|e| e.to_string())?;
        } else {
            high.try_reserve_exact(count).map_err(|e| e.to_string())?;
        }
        for y in 0..d.height {
            for x in 0..d.width {
                let value = self.pixel([x as f64 + 0.5, y as f64 + 0.5])?.store(
                    depth,
                    dither,
                    [x as i64, y as i64],
                );
                if depth == Depth::Eight {
                    low.push(value.map(|v| v as u8));
                } else {
                    high.push(value);
                }
            }
        }
        Ok(if depth == Depth::Eight {
            Pixels::Eight(low)
        } else {
            Pixels::Sixteen(high)
        })
    }
}
