//! A clipping run is composited at the base's fixed alpha, then placed over its backdrop.
use super::*;
use crate::{clipping, layer_properties::Target};
struct Sample<'a, F> {
    graph: &'a Graph,
    doc: &'a Document,
    original: Point,
    pixel: &'a mut F,
    seen: [bool; 256],
}
fn atop(
    base: &mut Color,
    top: Color,
    paper: Paper,
    original: Point,
    blend: crate::layer_blend::Mode,
) {
    if !blend.is_normal() {
        blend.atop(base, top);
        return;
    }
    if base.alpha <= 0.0 || top.alpha <= 0.0 {
        return;
    }
    if paper.blend == crate::brush_composite::Mode::Normal {
        for i in 0..3 {
            base.rgb[i] = top.rgb[i] * base.alpha + base.rgb[i] * (1.0 - top.alpha);
        }
    } else {
        let mut opaque = base.rgba();
        opaque[3] = 255;
        let result = paper.blend.compose(
            opaque,
            top.rgba(),
            1.0,
            original.map(|n| n.floor() as i64),
            paper.seed,
        );
        for (i, c) in result[..3].iter().enumerate() {
            base.rgb[i] = *c as f32 * base.alpha;
        }
    }
}
impl<F: FnMut(usize, &RasterLayer) -> [u8; 4]> Sample<'_, F> {
    fn paper(&self, n: Node) -> Paper {
        match n {
            Node::Raster(n) => self.graph.layers.get(&n).copied().unwrap_or_default(),
            Node::Folder(n) => self.graph.folders[n].paper,
        }
    }
    fn weight(&self, n: Node) -> f32 {
        let p = match n {
            Node::Raster(n) => self
                .doc
                .scene
                .layer_properties
                .rasters
                .get(&self.doc.scene.layer_ids.id(n)),
            Node::Folder(n) => self.doc.scene.layer_properties.folders.get(&(n as u16)),
        };
        p.map_or(1.0, |p| p.opacity as f32 / 100.0)
    }
    fn source(&mut self, n: Node, depth: usize) -> Color {
        if depth > 16 {
            return Color::default();
        }
        let graph = self.graph;
        let paper = self.paper(n);
        if paper.hidden {
            graph.mark_layers(n, &mut self.seen, 0);
            return Color::default();
        }
        let pixel = match n {
            Node::Raster(n) => {
                self.seen[n] = true;
                let Some(l) = self.doc.layers.get(n) else {
                    return Color::default();
                };
                (self.pixel)(n, l)
            }
            Node::Folder(n) => {
                let folder = &graph.folders[n];
                if folder.through {
                    return Color::default();
                }
                let mut group = Color::default();
                self.nodes(&folder.children, &mut group, depth + 1);
                group.rgba()
            }
        };
        let mut c = Color::from(paper.shade(pixel, self.original));
        c.scale(self.weight(n));
        c
    }
    fn put(&self, dst: &mut Color, source: Color, n: Node) {
        let paper = self.paper(n);
        let mode = crate::layer_blend::mode(self.doc, clipping::node_key(self.doc, n));
        if !mode.is_normal() {
            mode.over(dst, source);
            return;
        }
        if paper.blend == crate::brush_composite::Mode::Normal {
            dst.over(source);
        } else {
            *dst = Color::from(paper.blend.compose(
                dst.rgba(),
                source.rgba(),
                1.0,
                self.original.map(|p| p.floor() as i64),
                paper.seed,
            ));
        }
    }
    fn nodes(&mut self, nodes: &[Node], dst: &mut Color, depth: usize) {
        if depth > 16 {
            return;
        }
        let graph = self.graph;
        let mut i = 0;
        while i < nodes.len() {
            let n = nodes[i];
            let key = clipping::node_key(self.doc, n);
            if self.doc.scene.clipping.binding(key).is_some() {
                graph.mark_layers(n, &mut self.seen, 0);
                i += 1;
                continue;
            }
            if let Node::Folder(index) = n
                && graph.folders[index].through
            {
                let folder = &graph.folders[index];
                if folder.paper.hidden {
                    graph.mark_layers(n, &mut self.seen, 0);
                } else {
                    let before = *dst;
                    self.nodes(&folder.children, dst, depth + 1);
                    *dst = Color::interpolate(before, *dst, self.weight(n));
                }
                i += 1;
                continue;
            }
            let mut group = self.source(n, depth);
            let mut j = i + 1;
            while j < nodes.len()
                && self
                    .doc
                    .scene
                    .clipping
                    .binding(clipping::node_key(self.doc, nodes[j]))
                    .is_some()
            {
                if self
                    .doc
                    .scene
                    .clipping
                    .binding(clipping::node_key(self.doc, nodes[j]))
                    == Some(key)
                {
                    let top = self.source(nodes[j], depth);
                    atop(
                        &mut group,
                        top,
                        self.paper(nodes[j]),
                        self.original,
                        crate::layer_blend::mode(self.doc, clipping::node_key(self.doc, nodes[j])),
                    );
                } else {
                    graph.mark_layers(nodes[j], &mut self.seen, 0);
                }
                j += 1;
            }
            self.put(dst, group, n);
            i = j;
        }
    }
}
impl Graph {
    pub(super) fn clipped_composite(
        &self,
        doc: &Document,
        point: [i64; 2],
        pixel: &mut impl FnMut(usize, &RasterLayer) -> [u8; 4],
    ) -> Color {
        let mut sample = Sample {
            graph: self,
            doc,
            original: doc
                .scene
                .transform
                .inverse()
                .map([point[0] as f64 + 0.5, point[1] as f64 + 0.5]),
            pixel,
            seen: [false; 256],
        };
        let mut color = Color::default();
        sample.nodes(&self.roots, &mut color, 0);
        for n in 0..doc.layers.len() {
            if !sample.seen[n]
                && doc
                    .scene
                    .clipping
                    .binding(crate::layer_properties::raster(doc, n))
                    .is_none()
            {
                let source = sample.source(Node::Raster(n), 0);
                sample.put(&mut color, source, Node::Raster(n));
            }
        }
        color
    }
    /// Read-only contribution for picking and independent layer snapshots, including the base mask.
    pub(crate) fn clipped_node_pixel(
        &self,
        doc: &Document,
        node: Node,
        point: [i64; 2],
    ) -> [u8; 4] {
        let mut pixel =
            |_: usize, l: &RasterLayer| l.pixel(doc.width, doc.height, point[0], point[1]);
        let mut sample = Sample {
            graph: self,
            doc,
            original: doc
                .scene
                .transform
                .inverse()
                .map([point[0] as f64 + 0.5, point[1] as f64 + 0.5]),
            pixel: &mut pixel,
            seen: [false; 256],
        };
        let key = clipping::node_key(doc, node);
        if let Some(base) = doc.scene.clipping.binding(key) {
            if !clipping::active(doc, key) {
                return [0; 4];
            }
            let Some(base) = clipping::node(doc, base) else {
                return [0; 4];
            };
            let alpha = sample.source(base, 0).alpha;
            let mut c = sample.source(node, 0);
            c.scale(alpha);
            c.rgba()
        } else {
            let mut c = Color::default();
            sample.nodes(&[node], &mut c, 0);
            c.rgba()
        }
    }
}
pub(crate) fn vectors(
    doc: &Document,
    point: Point,
    original: Point,
    initial: [u8; 4],
    mut allowed: impl FnMut(usize, &crate::line_tools::Layer) -> bool,
    centerline: bool,
) -> [u8; 4] {
    let source = |n: usize, allowed: &mut dyn FnMut(usize, &crate::line_tools::Layer) -> bool| {
        let l = &doc.scene.lines.layers[n];
        if l.hidden || !allowed(n, l) {
            return Color::default();
        }
        let mut rgba = [0; 4];
        for p in &l.paths {
            let value = if centerline {
                if p.segments().any(|(a, b)| {
                    crate::line_tools::distance(
                        point,
                        doc.scene.transform.map(a),
                        doc.scene.transform.map(b),
                    )
                    .0 <= 0.5
                }) {
                    if p.stroke {
                        p.color
                    } else {
                        p.fill.unwrap_or(p.color)
                    }
                } else {
                    [0; 4]
                }
            } else {
                p.pixel(original)
            };
            rgba = crate::render::over(rgba, value);
        }
        let mut result = Color::from(rgba);
        result.scale(crate::layer_properties::factor(doc, Target::Vector(l.id)) as f32);
        result
    };
    let mut output = Color::from(initial);
    let mut i = 0;
    let layers = &doc.scene.lines.layers;
    while i < layers.len() {
        let key = Target::Vector(layers[i].id);
        if doc.scene.clipping.binding(key).is_some() {
            i += 1;
            continue;
        }
        let mut group = source(i, &mut allowed);
        let mut j = i + 1;
        while j < layers.len()
            && doc
                .scene
                .clipping
                .binding(Target::Vector(layers[j].id))
                .is_some()
        {
            if doc.scene.clipping.binding(Target::Vector(layers[j].id)) == Some(key) {
                let top = source(j, &mut allowed);
                atop(
                    &mut group,
                    top,
                    Paper::default(),
                    original,
                    crate::layer_blend::mode(doc, Target::Vector(layers[j].id)),
                );
            }
            j += 1;
        }
        crate::layer_blend::mode(doc, key).over(&mut output, group);
        i = j;
    }
    output.rgba()
}
