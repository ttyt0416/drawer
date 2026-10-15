//! Raster folder composition and nondestructive paper recipes.
use crate::{
    document::{Document, RasterLayer},
    scene::Point,
    texture::Pattern,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Paper {
    pub pattern: Pattern,
    pub origin: Point,
}
impl Paper {
    fn valid(self) -> bool {
        self.pattern.valid() && self.origin.iter().all(|v| v.is_finite() && v.abs() <= 1e12)
    }
    pub fn shade(self, color: [u8; 4], original: Point) -> [u8; 4] {
        self.pattern.shade(
            color,
            [original[0] - self.origin[0], original[1] - self.origin[1]],
        )
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Node {
    Raster(usize),
    Folder(usize),
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Folder {
    pub name: String,
    pub children: Vec<Node>,
    pub paper: Paper,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Graph {
    pub layers: BTreeMap<usize, Paper>,
    pub folders: Vec<Folder>,
    pub roots: Vec<Node>,
}
#[derive(Clone, Copy, Default)]
pub(crate) struct Color {
    pub rgb: [f32; 3],
    pub alpha: f32,
}
impl Color {
    fn from(color: [u8; 4]) -> Self {
        let alpha = color[3] as f32 / 255.0;
        Self {
            rgb: [
                color[0] as f32 * alpha,
                color[1] as f32 * alpha,
                color[2] as f32 * alpha,
            ],
            alpha,
        }
    }
    fn over(&mut self, top: Self) {
        for channel in 0..3 {
            self.rgb[channel] = top.rgb[channel] + self.rgb[channel] * (1.0 - top.alpha);
        }
        self.alpha = top.alpha + self.alpha * (1.0 - top.alpha);
    }
    pub(crate) fn rgba(self) -> [u8; 4] {
        if self.alpha == 0.0 {
            [0; 4]
        } else {
            [
                (self.rgb[0] / self.alpha).round() as u8,
                (self.rgb[1] / self.alpha).round() as u8,
                (self.rgb[2] / self.alpha).round() as u8,
                (self.alpha * 255.0).round() as u8,
            ]
        }
    }
    pub(crate) fn add(&mut self, pixel: [u8; 4]) {
        self.over(Self::from(pixel));
    }
}
impl Graph {
    pub fn is_empty(&self) -> bool {
        self.layers.is_empty() && self.folders.is_empty() && self.roots.is_empty()
    }
    pub fn valid_for(&self, count: usize) -> bool {
        if count > 256
            || self.layers.len() > 256
            || self.folders.len() > 256
            || self.roots.len() > 512
            || self.layers.iter().any(|(n, p)| *n >= count || !p.valid())
            || self.folders.iter().any(|f| {
                f.name.trim().is_empty()
                    || f.name.chars().count() > 128
                    || f.name.chars().any(char::is_control)
                    || f.children.len() > 512
                    || !f.paper.valid()
            })
        {
            return false;
        }
        let mut folders = [0u8; 256];
        let mut layers = [false; 256];
        for root in &self.roots {
            if !self.visit(*root, count, 0, &mut folders, &mut layers) {
                return false;
            }
        }
        self.folders
            .iter()
            .enumerate()
            .all(|(n, f)| folders[n] == 2 || f.children.is_empty())
    }
    fn visit(
        &self,
        node: Node,
        count: usize,
        depth: usize,
        folders: &mut [u8; 256],
        layers: &mut [bool; 256],
    ) -> bool {
        if depth > 16 {
            return false;
        }
        match node {
            Node::Raster(n) => {
                if n >= count || layers[n] {
                    false
                } else {
                    layers[n] = true;
                    true
                }
            }
            Node::Folder(n) => {
                let Some(folder) = self.folders.get(n) else {
                    return false;
                };
                if folders[n] != 0 {
                    return false;
                }
                folders[n] = 1;
                if !folder
                    .children
                    .iter()
                    .all(|child| self.visit(*child, count, depth + 1, folders, layers))
                {
                    return false;
                }
                folders[n] = 2;
                true
            }
        }
    }
    pub fn group_all(&mut self, count: usize, name: &str) -> Result<usize, String> {
        if !self.valid_for(count) || self.folders.len() >= 256 {
            return Err("Invalid folder structure or folder limit".into());
        }
        let mut next = self.clone();
        let mut seen = [false; 256];
        for root in &next.roots {
            next.mark_layers(*root, &mut seen, 0);
        }
        let mut children = next.roots.clone();
        for (n, seen) in seen.iter().enumerate().take(count) {
            if !seen {
                children.push(Node::Raster(n));
            }
        }
        let index = next.folders.len();
        next.folders.push(Folder {
            name: name.trim().into(),
            children,
            paper: Default::default(),
        });
        next.roots = vec![Node::Folder(index)];
        if !next.valid_for(count) {
            return Err("Folder name, depth or reference limit".into());
        }
        *self = next;
        Ok(index)
    }
    fn mark_layers(&self, node: Node, seen: &mut [bool; 256], depth: usize) {
        if depth > 16 {
            return;
        }
        match node {
            Node::Raster(n) => {
                if let Some(v) = seen.get_mut(n) {
                    *v = true;
                }
            }
            Node::Folder(n) => {
                if let Some(f) = self.folders.get(n) {
                    for c in &f.children {
                        self.mark_layers(*c, seen, depth + 1);
                    }
                }
            }
        }
    }
    pub fn root_nodes(&self, count: usize) -> Vec<Node> {
        let mut seen = [false; 256];
        for root in &self.roots {
            self.mark_layers(*root, &mut seen, 0);
        }
        let mut roots = self.roots.clone();
        for (n, seen) in seen.iter().enumerate().take(count) {
            if !seen {
                roots.push(Node::Raster(n));
            }
        }
        roots
    }
    pub fn shift_layer(&mut self, index: usize, delta: Point) {
        if let Some(p) = self.layers.get_mut(&index) {
            p.origin[0] += delta[0];
            p.origin[1] += delta[1];
        }
    }
    pub fn missing(&self) -> Vec<String> {
        self.layers
            .values()
            .map(|p| p.pattern)
            .chain(self.folders.iter().map(|f| f.paper.pattern))
            .filter(|p| p.missing())
            .map(Pattern::label)
            .collect()
    }
    pub fn size_bytes(&self) -> usize {
        self.layers.len() * std::mem::size_of::<(usize, Paper)>()
            + self.roots.len() * std::mem::size_of::<Node>()
            + self
                .folders
                .iter()
                .map(|f| {
                    std::mem::size_of::<Folder>()
                        + f.name.len()
                        + f.children.len() * std::mem::size_of::<Node>()
                })
                .sum::<usize>()
    }
    pub(crate) fn composite(
        &self,
        doc: &Document,
        x: u32,
        y: u32,
        pixel: &mut impl FnMut(usize, &RasterLayer) -> [u8; 4],
    ) -> Color {
        if self.layers.is_empty() && self.roots.is_empty() {
            let mut color = Color::default();
            for (n, layer) in doc.layers.iter().enumerate() {
                color.add(pixel(n, layer));
            }
            return color;
        }
        let original = doc
            .scene
            .transform
            .inverse()
            .map([x as f64 + 0.5, y as f64 + 0.5]);
        if self.roots.is_empty() {
            let mut color = Color::default();
            for (n, layer) in doc.layers.iter().enumerate() {
                let c = pixel(n, layer);
                color.add(self.layers.get(&n).map_or(c, |p| p.shade(c, original)));
            }
            return color;
        }
        let mut seen = [false; 256];
        let mut color = Color::default();
        for root in &self.roots {
            self.append(doc, (*root, 0), original, &mut color, &mut seen, pixel);
        }
        for (n, layer) in doc.layers.iter().enumerate() {
            if !seen[n] {
                let c = pixel(n, layer);
                color.add(self.layers.get(&n).map_or(c, |p| p.shade(c, original)));
            }
        }
        color
    }
    fn append(
        &self,
        doc: &Document,
        location: (Node, usize),
        original: Point,
        color: &mut Color,
        seen: &mut [bool; 256],
        pixel: &mut impl FnMut(usize, &RasterLayer) -> [u8; 4],
    ) {
        let (node, depth) = location;
        if depth > 16 {
            return;
        }
        match node {
            Node::Raster(n) => {
                if n >= 256 || seen[n] {
                    return;
                }
                let Some(layer) = doc.layers.get(n) else {
                    return;
                };
                seen[n] = true;
                let c = pixel(n, layer);
                color.add(self.layers.get(&n).map_or(c, |p| p.shade(c, original)));
            }
            Node::Folder(n) => {
                if let Some(folder) = self.folders.get(n) {
                    if folder.paper.pattern.effective() {
                        let mut group = Color::default();
                        for c in &folder.children {
                            self.append(doc, (*c, depth + 1), original, &mut group, seen, pixel);
                        }
                        color.add(folder.paper.shade(group.rgba(), original));
                    } else {
                        for c in &folder.children {
                            self.append(doc, (*c, depth + 1), original, color, seen, pixel);
                        }
                    }
                }
            }
        }
    }
    pub fn node_pixel(&self, doc: &Document, node: Node, point: [i64; 2]) -> [u8; 4] {
        let original = doc
            .scene
            .transform
            .inverse()
            .map([point[0] as f64 + 0.5, point[1] as f64 + 0.5]);
        let mut color = Color::default();
        let mut seen = [false; 256];
        self.append(
            doc,
            (node, 0),
            original,
            &mut color,
            &mut seen,
            &mut |_, layer| layer.pixel(doc.width, doc.height, point[0], point[1]),
        );
        color.rgba()
    }
    pub fn layer_pixel(
        &self,
        doc: &Document,
        index: usize,
        point: [i64; 2],
        color: [u8; 4],
    ) -> [u8; 4] {
        let original = doc
            .scene
            .transform
            .inverse()
            .map([point[0] as f64 + 0.5, point[1] as f64 + 0.5]);
        self.layers
            .get(&index)
            .map_or(color, |p| p.shade(color, original))
    }
}

impl Graph {
    pub fn raster_indices(&self, node: Node) -> Vec<usize> {
        let mut seen = [false; 256];
        self.mark_layers(node, &mut seen, 0);
        seen.iter()
            .enumerate()
            .filter_map(|(n, v)| v.then_some(n))
            .collect()
    }
    /// Render a subtree, including content outside the canvas. No document mutation.
    pub fn render_node(
        &self,
        doc: &Document,
        node: Node,
        budget: usize,
    ) -> Result<RasterLayer, String> {
        if !self.valid_for(doc.layers.len()) {
            return Err("Invalid folder graph".into());
        }
        let indices = self.raster_indices(node);
        if indices.is_empty() {
            return Err("The folder contains no raster layers".into());
        }
        let mut min = [i64::MAX; 2];
        let mut max = [i64::MIN; 2];
        for n in &indices {
            let layer = doc.layers.get(*n).ok_or("Missing raster layer")?;
            if !layer.valid(doc.width, doc.height) {
                return Err("Invalid raster layer".into());
            }
            let e = layer.extent(doc.width, doc.height);
            for a in 0..2 {
                min[a] = min[a].min(e.offset[a]);
                max[a] = max[a].max(e.offset[a] + [e.width, e.height][a] as i64);
            }
        }
        let size = [max[0] - min[0], max[1] - min[1]];
        if size
            .iter()
            .any(|n| *n < 1 || *n > crate::document::MAX_DIMENSION as i64)
        {
            return Err("Merged extent exceeds the raster dimension limit".into());
        }
        let count = (size[0] as usize)
            .checked_mul(size[1] as usize)
            .ok_or("Raster extent overflow")?;
        if count > budget / 4 {
            return Err("Merged raster exceeds the memory budget".into());
        }
        let mut pixels = Vec::new();
        pixels
            .try_reserve_exact(count)
            .map_err(|_| "Not enough raster memory")?;
        for y in min[1]..max[1] {
            for x in min[0]..max[0] {
                let c = match node {
                    Node::Raster(n) => self.layer_pixel(
                        doc,
                        n,
                        [x, y],
                        doc.layers[n].pixel(doc.width, doc.height, x, y),
                    ),
                    Node::Folder(_) => self.node_pixel(doc, node, [x, y]),
                };
                pixels.push(c);
            }
        }
        let name = match node {
            Node::Raster(n) => doc.layers[n].name.clone(),
            Node::Folder(n) => self.folders[n].name.clone(),
        };
        Ok(RasterLayer {
            locked: false,
            name,
            pixels,
            placement: Some(crate::document::PlacedRaster {
                width: size[0] as u32,
                height: size[1] as u32,
                offset: min,
            }),
        })
    }
}
/// Bake one paper effect or a folder subtree as a single undoable document command.
pub fn bake(doc: &mut Document, node: Node) -> Result<bool, String> {
    if !doc.scene.paper.valid_for(doc.layers.len()) {
        return Err("Invalid folder graph".into());
    }
    let graph = &doc.scene.paper;
    if !graph
        .root_nodes(doc.layers.len())
        .iter()
        .any(|r| contains(graph, *r, node, 0))
    {
        return Err("Select a visible raster layer or folder".into());
    }
    let indices = graph.raster_indices(node);
    if indices.iter().any(|n| doc.layers[*n].locked) {
        return Err("Unlock all affected layers before baking".into());
    }
    if let Node::Raster(n) = node
        && !graph.layers.contains_key(&n)
    {
        return Ok(false);
    }
    let available = crate::document::DEFAULT_DOCUMENT_BUDGET
        .saturating_sub(doc.resident_bytes().saturating_add(doc.memory_bytes() * 2));
    let raster = graph.render_node(
        doc,
        node,
        available.min(crate::document::DEFAULT_DOCUMENT_BUDGET / 8),
    )?;
    let mut after = doc.snapshot();
    match node {
        Node::Raster(n) => {
            after.layers[n] = raster;
            after.scene.paper.layers.remove(&n);
            after.scene.mouse_strokes.retain(|s| s.layer != n);
        }
        Node::Folder(folder) => {
            let first = *indices.first().ok_or("Empty folder")?;
            let mut map = [0usize; 256];
            let mut layers = Vec::new();
            for (n, layer) in after.layers.into_iter().enumerate() {
                map[n] = layers.len();
                if n == first {
                    layers.push(raster.clone());
                } else if !indices.contains(&n) {
                    layers.push(layer);
                }
            }
            let replacement = map[first];
            let mut g = after.scene.paper;
            let roots = g.root_nodes(doc.layers.len());
            let consumed = (0..g.folders.len())
                .filter(|n| contains(&g, Node::Folder(folder), Node::Folder(*n), 0))
                .collect::<Vec<_>>();
            let remap = |n: Node| match n {
                Node::Folder(f) if f == folder => Node::Raster(replacement),
                Node::Raster(r) => Node::Raster(map[r]),
                n => n,
            };
            g.roots = roots.into_iter().map(remap).collect();
            for (n, f) in g.folders.iter_mut().enumerate() {
                if consumed.contains(&n) {
                    f.children.clear();
                    f.paper = Default::default();
                } else {
                    f.children = f.children.iter().copied().map(remap).collect();
                }
            }
            g.layers = g
                .layers
                .into_iter()
                .filter(|(n, _)| !indices.contains(n))
                .map(|(n, p)| (map[n], p))
                .collect();
            after
                .scene
                .mouse_strokes
                .retain(|s| !indices.contains(&s.layer));
            for s in &mut after.scene.mouse_strokes {
                s.layer = map[s.layer];
            }
            after.layers = layers;
            after.scene.paper = g;
        }
    }
    doc.commit_state(after)
}
fn contains(g: &Graph, node: Node, target: Node, depth: usize) -> bool {
    if node == target {
        return true;
    }
    if depth > 16 {
        return false;
    }
    match node {
        Node::Raster(_) => false,
        Node::Folder(n) => g.folders.get(n).is_some_and(|f| {
            f.children
                .iter()
                .any(|c| contains(g, *c, target, depth + 1))
        }),
    }
}

impl Graph {
    pub fn preview_pixel(&self, doc: &Document, x: u32, y: u32) -> [u8; 4] {
        self.composite(doc, x, y, &mut |_, l| {
            l.pixel(doc.width, doc.height, x as i64, y as i64)
        })
        .rgba()
    }
}

pub fn verify() -> Result<(), String> {
    use crate::document::{Background, CanvasSpec};
    let mut d = Document::create(
        CanvasSpec {
            width: 8,
            height: 4,
            dpi: 300.0,
            background: Background::Color([220, 140, 60]),
        },
        1024,
    )
    .map_err(|e| e.to_string())?;
    d.layers[0].pixels.fill([220, 140, 60, 96]);
    d.layers.push(RasterLayer {
        locked: false,
        name: "Top".into(),
        pixels: vec![[40, 80, 240, 128]; 32],
        placement: None,
    });
    let raw = d.snapshot();
    let original = d.composite_pixel(2, 1);
    let pattern = Pattern {
        source: 1,
        strength: 0.8,
        ..Default::default()
    };
    let folder = d.scene.paper.group_all(d.layers.len(), "Owned paper")?;
    d.scene.paper.folders[folder].paper.pattern = pattern;
    let expected = pattern.shade(original, [2.5, 1.5]);
    if d.composite_pixel(2, 1) != expected {
        return Err("Folder paper was not applied once to the composed group".into());
    }
    let before = d.snapshot();
    let pixels = (0..32)
        .map(|n| d.composite_pixel(n % 8, n / 8))
        .collect::<Vec<_>>();
    bake(&mut d, Node::Folder(folder))?;
    if d.layers.len() != 1
        || d.layers[0].pixels != pixels
        || !d.undo()
        || d.snapshot() != before
        || !d.redo()
    {
        return Err("Paper folder bake/undo/redo mismatch".into());
    }
    if raw.layers[0].pixels[0] != [220, 140, 60, 96] {
        return Err("Paper modified original input".into());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        document::{Background, CanvasSpec},
        test_support::TempDir,
    };
    fn doc() -> Document {
        Document::create(
            CanvasSpec {
                width: 8,
                height: 4,
                dpi: 300.0,
                background: Background::Color([200, 100, 50]),
            },
            1024,
        )
        .unwrap()
    }
    #[test]
    fn folder_applies_once_and_bakes_one_undo() {
        verify().unwrap();
    }
    #[test]
    fn nested_partial_alpha_bake_keeps_offcanvas_and_remaps_other_layers() {
        let mut d = doc();
        d.layers[0].pixels.fill([220, 100, 20, 77]);
        d.layers[0].placement = Some(crate::document::PlacedRaster {
            width: 8,
            height: 4,
            offset: [-2, 1],
        });
        d.layers.push(RasterLayer {
            locked: false,
            name: "Above".into(),
            pixels: vec![[30, 90, 220, 128]; 32],
            placement: None,
        });
        let first = d.scene.paper.group_all(2, "Inner").unwrap();
        d.scene.paper.folders[first].paper.pattern = Pattern {
            source: 2,
            strength: 0.7,
            ..Default::default()
        };
        let outer = d.scene.paper.group_all(2, "Outer").unwrap();
        d.scene.paper.folders[outer].paper.pattern = Pattern {
            source: 1,
            strength: 0.3,
            ..Default::default()
        };
        d.layers.push(RasterLayer {
            locked: false,
            name: "Unreferenced top".into(),
            pixels: vec![[255, 20, 80, 30]; 32],
            placement: None,
        });
        d.scene.paper.layers.insert(
            2,
            Paper {
                pattern: Pattern {
                    source: 3,
                    ..Default::default()
                },
                ..Default::default()
            },
        );
        let before = d.snapshot();
        let outside = d.scene.paper.node_pixel(&d, Node::Folder(outer), [-2, 2]);
        let displayed = (0..32)
            .map(|n| d.composite_pixel(n % 8, n / 8))
            .collect::<Vec<_>>();
        bake(&mut d, Node::Folder(outer)).unwrap();
        assert_eq!(d.layers.len(), 2);
        assert_eq!(d.layers[0].extent(8, 4).offset, [-2, 0]);
        assert_eq!(d.layers[0].pixel(8, 4, -2, 2), outside);
        assert_eq!(
            displayed,
            (0..32)
                .map(|n| d.composite_pixel(n % 8, n / 8))
                .collect::<Vec<_>>()
        );
        assert!(d.scene.paper.layers.contains_key(&1));
        assert!(d.scene.paper.valid_for(2));
        assert!(d.undo());
        assert!(d.snapshot() == before);
        assert!(d.redo());
    }
    #[test]
    fn paper_moves_with_layer_view_is_independent_and_locked_bake_is_atomic() {
        let mut d = doc();
        d.scene.paper.layers.insert(
            0,
            Paper {
                pattern: Pattern {
                    source: 1,
                    strength: 1.0,
                    ..Default::default()
                },
                ..Default::default()
            },
        );
        let pixel = d.composite_pixel(2, 1);
        let raw = d.layers[0].pixels.clone();
        d.view.rotate(37.0);
        d.view.flipped = true;
        assert_eq!(d.composite_pixel(2, 1), pixel);
        let mut movement =
            crate::precision::Nudge::begin(&d, 0, crate::precision::Target::Layer).unwrap();
        movement.step(&mut d, [2, 1]).unwrap();
        assert_eq!(d.composite_pixel(4, 2), pixel);
        assert_eq!(d.layers[0].pixels, raw);
        d.layers[0].locked = true;
        let before = d.snapshot();
        assert!(bake(&mut d, Node::Raster(0)).is_err());
        assert!(d.snapshot() == before);
        d.layers[0].locked = false;
        let before = d.snapshot();
        let shown = d.composite_pixel(4, 2);
        bake(&mut d, Node::Raster(0)).unwrap();
        assert_eq!(d.composite_pixel(4, 2), shown);
        assert!(d.undo());
        assert!(d.snapshot() == before);
    }
    #[test]
    fn native_self_contained_missing_fallback_and_legacy_default() {
        let root = TempDir::new();
        let path = root.0.join("paper.drawer");
        let mut d = doc();
        let i = d.scene.paper.group_all(1, "한지").unwrap();
        d.scene.paper.folders[i].paper.pattern = Pattern {
            source: 2,
            scale: 1.3,
            strength: 0.9,
            offset: [-2.0, 3.0],
            ..Default::default()
        };
        crate::files::save(&mut d, &path).unwrap();
        let a = crate::files::open(&path, 100000).unwrap();
        assert!(a.snapshot() == d.snapshot());
        assert_eq!(a.composite_pixel(2, 1), d.composite_pixel(2, 1));
        d.scene.paper.folders[i].paper.pattern.source = 999;
        assert_eq!(d.composite_pixel(2, 1), [200, 100, 50, 255]);
        assert_eq!(d.scene.paper.missing().len(), 1);
        crate::files::save(&mut d, &path).unwrap();
        let a = crate::files::open(&path, 100000).unwrap();
        assert_eq!(a.scene.paper.missing(), d.scene.paper.missing());
        let mut old = serde_json::to_value(crate::scene::Scene::default()).unwrap();
        old.as_object_mut().unwrap().remove("paper");
        let scene: crate::scene::Scene = serde_json::from_value(old).unwrap();
        assert!(scene.paper.is_empty());
    }
    #[test]
    fn malformed_graph_rejected_without_mutation_or_recursion() {
        let mut d = doc();
        let before = d.snapshot();
        let mut scene = d.scene.clone();
        scene.paper.roots = vec![Node::Raster(0), Node::Raster(0)];
        assert!(d.edit_scene(scene).is_err());
        let mut graph = Graph::default();
        graph.folders.push(Folder {
            name: "Cycle".into(),
            children: vec![Node::Folder(0)],
            paper: Default::default(),
        });
        graph.roots.push(Node::Folder(0));
        assert!(!graph.valid_for(1));
        let mut graph = Graph::default();
        for _ in 0..16 {
            graph.group_all(1, "Nested").unwrap();
        }
        let before_graph = graph.clone();
        assert!(graph.group_all(1, "Too deep").is_err());
        assert_eq!(graph, before_graph);
        let mut scene = d.scene.clone();
        scene.paper.layers.insert(3, Default::default());
        assert!(d.edit_scene(scene).is_err());
        assert!(d.snapshot() == before);
    }
    #[test]
    fn actual_flip_rotation_keep_paper_aligned_and_zero_strength_is_identity() {
        use crate::sizing::{Change, prepare};
        use std::sync::atomic::AtomicBool;
        let mut d = doc();
        let p = Pattern {
            source: 1,
            strength: 0.9,
            ..Default::default()
        };
        d.scene.paper.layers.insert(
            0,
            Paper {
                pattern: p,
                ..Default::default()
            },
        );
        let original = d.snapshot();
        let pixels = (0..32)
            .map(|n| d.composite_pixel(n % 8, n / 8))
            .collect::<Vec<_>>();
        for change in [Change::FlipHorizontal, Change::Rotate90 { clockwise: true }] {
            let next = prepare(&original, change, 1_000_000, &AtomicBool::new(false)).unwrap();
            d.commit_state(next).unwrap();
            for y in 0..d.height {
                for x in 0..d.width {
                    let old = match change {
                        Change::FlipHorizontal => [7 - x, y],
                        _ => [y, 3 - x],
                    };
                    assert_eq!(
                        d.composite_pixel(x, y),
                        pixels[(old[1] * 8 + old[0]) as usize]
                    );
                }
            }
            assert!(d.undo());
            assert!(d.snapshot() == original);
        }
        d.scene.paper.layers.get_mut(&0).unwrap().pattern.strength = 0.0;
        assert_eq!(d.composite_pixel(2, 1), [200, 100, 50, 255]);
    }
    #[test]
    fn huge_merge_extent_and_budget_reject_without_partial_history() {
        let mut d = doc();
        d.layers.push(RasterLayer {
            locked: false,
            name: "Far away".into(),
            pixels: vec![[255; 4]; 32],
            placement: Some(crate::document::PlacedRaster {
                width: 8,
                height: 4,
                offset: [1_000_000, 0],
            }),
        });
        let n = d.scene.paper.group_all(2, "Wide").unwrap();
        let before = d.snapshot();
        assert!(bake(&mut d, Node::Folder(n)).is_err());
        assert!(d.snapshot() == before);
        assert_eq!(d.history_counts(), (0, 0));
        assert!(d.scene.paper.render_node(&d, Node::Raster(0), 1).is_err());
    }
}
