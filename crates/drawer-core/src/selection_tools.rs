//! Page-owned selection commands. Shapes live in preserved page coordinates.
use crate::{
    document::Document,
    scene::{Affine, Point, Rect, Scene},
    selection::{Mask, Run},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::atomic::{AtomicU64, Ordering},
};
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Shape {
    pub rect: Option<Rect>,
    pub mask: Option<Mask>,
    pub translation: Point,
}
impl Shape {
    fn capture(s: &Scene) -> Self {
        Self {
            rect: s.selection,
            mask: s.selection_mask.clone(),
            translation: s.selection_translation,
        }
    }
    fn put(&self, s: &mut Scene) {
        s.selection = self.rect;
        s.selection_mask = self.mask.clone();
        s.selection_translation = self.translation;
    }
    fn valid(&self) -> bool {
        self.rect.is_none_or(Rect::valid)
            && self.mask.as_ref().is_none_or(Mask::valid)
            && self
                .translation
                .iter()
                .all(|v| v.is_finite() && v.abs() <= 1e9)
    }
    fn bytes(&self) -> usize {
        self.mask
            .as_ref()
            .map_or(0, |m| m.runs.len() * std::mem::size_of::<Run>())
            + 64
    }
    fn restricted(&self) -> bool {
        self.rect.is_some() || self.mask.is_some()
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Named {
    pub id: u64,
    pub name: String,
    pub shape: Shape,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Data {
    pub page: Option<String>,
    pub next_id: u64,
    pub last: Option<Shape>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_cleared: Option<Shape>,
    pub saved: Vec<Named>,
}
impl Default for Data {
    fn default() -> Self {
        Self {
            page: None,
            next_id: 1,
            last: None,
            last_cleared: None,
            saved: vec![],
        }
    }
}
pub(crate) fn page_id() -> String {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    format!(
        "{:x}-{:x}-{:x}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos(),
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}
impl Data {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
    pub fn valid(&self) -> bool {
        self.next_id > 0
            && self.saved.len() <= 16
            && self.page.as_ref().is_none_or(|s| {
                !s.is_empty() && s.len() <= 96 && s.is_ascii() && !s.chars().any(char::is_control)
            })
            && (self.page.is_some()
                || (self.last.is_none() && self.last_cleared.is_none() && self.saved.is_empty()))
            && self.last.as_ref().is_none_or(Shape::valid)
            && self.last_cleared.as_ref().is_none_or(Shape::valid)
            && self.saved.iter().all(|n| {
                n.id > 0
                    && n.id < self.next_id
                    && n.name == n.name.trim()
                    && !n.name.is_empty()
                    && n.name.chars().count() <= 64
                    && !n.name.chars().any(char::is_control)
                    && n.shape.valid()
            })
            && self
                .saved
                .iter()
                .map(|n| n.id)
                .collect::<BTreeSet<_>>()
                .len()
                == self.saved.len()
    }
    pub fn size_bytes(&self) -> usize {
        self.page.as_ref().map_or(0, String::len)
            + self.last.as_ref().map_or(0, Shape::bytes)
            + self.last_cleared.as_ref().map_or(0, Shape::bytes)
            + self
                .saved
                .iter()
                .map(|n| n.name.len() + n.shape.bytes() + 16)
                .sum::<usize>()
    }
    pub fn transform_translation(&mut self, t: Affine) {
        let origin = t.map([0.0; 2]);
        for s in self
            .last
            .iter_mut()
            .chain(self.last_cleared.iter_mut())
            .chain(self.saved.iter_mut().map(|n| &mut n.shape))
        {
            let p = t.map(s.translation);
            s.translation = [p[0] - origin[0], p[1] - origin[1]];
        }
    }
    pub fn fork_page(&mut self) {
        if !self.is_default() {
            self.page = Some(page_id());
        }
    }
}
fn commit(doc: &mut Document, scene: Scene) -> Result<bool, String> {
    if !scene.valid()
        || serde_json::to_vec(&scene).map_err(|e| e.to_string())?.len() > 4 * 1024 * 1024
    {
        return Err("Selection exceeds page geometry/serialization budget".into());
    }
    doc.edit_scene(scene)
}
/// Build a page-owned selection for a preview without modifying pixels/history.
pub fn prepare_shape(doc: &Document, shape: Shape) -> Result<Scene, String> {
    let old = Shape::capture(&doc.scene);
    if shape == old {
        return Ok(doc.scene.clone());
    }
    if !shape.valid() {
        return Err("Invalid selection shape".into());
    }
    let mut scene = doc.scene.clone();
    if scene.selections.page.is_none() {
        scene.selections.page = Some(page_id());
    }
    scene.selections.last = if shape.restricted() {
        Some(shape.clone())
    } else if old.restricted() {
        Some(old.clone())
    } else {
        scene.selections.last.clone()
    };
    if !shape.restricted() && old.restricted() {
        scene.selections.last_cleared = Some(old);
    }
    shape.put(&mut scene);
    if !scene.valid()
        || serde_json::to_vec(&scene).map_err(|e| e.to_string())?.len() > 4 * 1024 * 1024
    {
        return Err("Selection exceeds page geometry/serialization budget".into());
    }
    Ok(scene)
}
fn selected(doc: &mut Document, shape: Shape) -> Result<bool, String> {
    let scene = prepare_shape(doc, shape)?;
    doc.edit_scene(scene)
}
pub fn clear(doc: &mut Document) -> Result<bool, String> {
    selected(doc, Shape::default())
}
pub fn reselect(doc: &mut Document) -> Result<bool, String> {
    let last = doc
        .scene
        .selections
        .last_cleared
        .as_ref()
        .or(doc.scene.selections.last.as_ref())
        .cloned()
        .ok_or("No previous selection on this page")?;
    selected(doc, last)
}
pub fn save_named(doc: &mut Document, name: &str) -> Result<bool, String> {
    let mut scene = doc.scene.clone();
    if scene.selections.page.is_none() {
        scene.selections.page = Some(page_id());
    }
    let shape = Shape::capture(&scene);
    let name = name.trim();
    if let Some(n) = scene.selections.saved.iter_mut().find(|n| n.name == name) {
        n.shape = shape;
    } else {
        let id = scene.selections.next_id;
        scene.selections.next_id = id.checked_add(1).ok_or("Selection ID limit")?;
        scene.selections.saved.push(Named {
            id,
            name: name.into(),
            shape,
        });
    }
    commit(doc, scene)
}
pub fn load_named(doc: &mut Document, id: u64) -> Result<bool, String> {
    let shape = doc
        .scene
        .selections
        .saved
        .iter()
        .find(|n| n.id == id)
        .ok_or("Named page selection is missing")?
        .shape
        .clone();
    selected(doc, shape)
}
pub fn delete_named(doc: &mut Document, id: u64) -> Result<bool, String> {
    let mut scene = doc.scene.clone();
    let i = scene
        .selections
        .saved
        .iter()
        .position(|n| n.id == id)
        .ok_or("Named page selection is missing")?;
    scene.selections.saved.remove(i);
    commit(doc, scene)
}
pub fn rectangle(doc: &mut Document, a: Point, b: Point) -> Result<bool, String> {
    if a.iter().chain(&b).any(|v| !v.is_finite() || v.abs() > 1e9) || a[0] == b[0] || a[1] == b[1] {
        return Err("Selection rectangle needs finite, nonzero width/height".into());
    }
    lasso(doc, &[a, [b[0], a[1]], b, [a[0], b[1]]])
}
pub fn lasso(doc: &mut Document, points: &[Point]) -> Result<bool, String> {
    let scene = prepare_lasso(doc, points)?;
    doc.edit_scene(scene)
}
pub fn prepare_lasso(doc: &Document, points: &[Point]) -> Result<Scene, String> {
    if points.len() < 3
        || points.len() > 4096
        || points
            .iter()
            .flatten()
            .any(|v| !v.is_finite() || v.abs() > 1e9)
    {
        return Err("Lasso needs 3–4096 finite page points".into());
    }
    let first = points[0];
    let Some(second) = points.iter().find(|p| **p != first) else {
        return Err("Empty lasso; previous selection preserved".into());
    };
    if !points.iter().any(|p| {
        ((p[0] - first[0]) * (second[1] - first[1]) - (p[1] - first[1]) * (second[0] - first[0]))
            .abs()
            > 1e-9
    }) {
        return Err("Lasso has no area; previous selection preserved".into());
    }
    let inverse = doc.scene.transform.inverse();
    let points = points.iter().map(|p| inverse.map(*p)).collect::<Vec<_>>();
    let mask = rasterize(doc, &points)?;
    prepare_shape(
        doc,
        Shape {
            mask: Some(mask),
            ..Default::default()
        },
    )
}
fn canvas(doc: &Document) -> [Point; 4] {
    let inverse = doc.scene.transform.inverse();
    [
        [0.0, 0.0],
        [doc.width as f64, 0.0],
        [doc.width as f64, doc.height as f64],
        [0.0, doc.height as f64],
    ]
    .map(|p| inverse.map(p))
}
fn pairs(points: &[Point], y: f64) -> Vec<[f64; 2]> {
    let mut xs = Vec::new();
    for n in 0..points.len() {
        let a = points[n];
        let b = points[(n + 1) % points.len()];
        if (a[1] > y) != (b[1] > y) {
            xs.push(a[0] + (y - a[1]) * (b[0] - a[0]) / (b[1] - a[1]));
        }
    }
    xs.sort_by(f64::total_cmp);
    xs.as_chunks::<2>().0.to_vec()
}
fn push(mask: &mut Mask, x: i32, y: i32, length: u32, alpha: u8) -> Result<(), String> {
    if alpha == 0 {
        return Ok(());
    }
    let mut x = x;
    let mut length = length;
    while length > 0 {
        let n = length.min(32768);
        if let Some(last) = mask.runs.last_mut()
            && last.y == y
            && last.alpha == alpha
            && last.x as i64 + last.length as i64 == x as i64
            && last.length + n <= 32768
        {
            last.length += n;
        } else {
            if mask.runs.len() == 32768 {
                return Err("Selection exceeds 32768 coverage runs".into());
            }
            mask.runs.push(Run {
                x,
                y,
                length: n,
                alpha,
            });
        }
        x += n as i32;
        length -= n;
    }
    Ok(())
}
fn event(events: &mut BTreeMap<i32, i32>, a: i32, b: i32, value: i32) {
    if a < b && value > 0 {
        *events.entry(a).or_default() += value;
        *events.entry(b).or_default() -= value;
    }
}
fn interval(events: &mut BTreeMap<i32, i32>, a: f64, b: f64) {
    if a >= b {
        return;
    }
    let start = a.ceil() as i32;
    let end = b.floor() as i32;
    event(events, start, end, 4);
    let lo = a.floor() as i32;
    let hi = b.floor() as i32;
    for x in [Some(lo), (hi != lo).then_some(hi)].into_iter().flatten() {
        if x >= start && x < end {
            continue;
        }
        let hits = [0.125, 0.375, 0.625, 0.875]
            .iter()
            .filter(|s| x as f64 + **s >= a && x as f64 + **s < b)
            .count() as i32;
        event(events, x, x + 1, hits);
    }
}
fn rasterize(doc: &Document, points: &[Point]) -> Result<Mask, String> {
    let clip = canvas(doc);
    if points
        .iter()
        .chain(&clip)
        .flatten()
        .any(|v| !v.is_finite() || v.abs() > 1_000_000.0)
    {
        return Err("Selection exceeds preserved-page coordinate range".into());
    }
    let min = points
        .iter()
        .map(|p| p[1])
        .fold(f64::INFINITY, f64::min)
        .max(clip.iter().map(|p| p[1]).fold(f64::INFINITY, f64::min))
        .floor() as i32;
    let max = points
        .iter()
        .map(|p| p[1])
        .fold(f64::NEG_INFINITY, f64::max)
        .min(clip.iter().map(|p| p[1]).fold(f64::NEG_INFINITY, f64::max))
        .ceil() as i32;
    if (max - min).max(0) as usize * points.len() * 4 > 64 * 1024 * 1024 {
        return Err("Selection scanline work budget exceeded".into());
    }
    let mut mask = Mask::default();
    for y in min..max {
        let mut events = BTreeMap::new();
        for sy in [0.125, 0.375, 0.625, 0.875] {
            let clipped = pairs(&clip, y as f64 + sy);
            for [a, b] in pairs(points, y as f64 + sy) {
                for [c, d] in &clipped {
                    interval(&mut events, a.max(*c), b.min(*d));
                }
            }
        }
        let mut previous = 0;
        let mut count = 0;
        for (x, delta) in events {
            if count > 0 && x > previous {
                if count > 16 {
                    return Err("Invalid lasso coverage".into());
                }
                push(
                    &mut mask,
                    previous,
                    y,
                    (x - previous) as u32,
                    ((count * 255 + 8) / 16) as u8,
                )?;
            }
            count += delta;
            previous = x;
        }
        if count != 0 {
            return Err("Unclosed lasso scanline".into());
        }
    }
    if !mask.valid() {
        return Err("Invalid lasso mask".into());
    }
    Ok(mask)
}
fn current_mask(doc: &Document, full: &Mask) -> Result<Mask, String> {
    let inverse = doc.scene.transform.inverse();
    let zero = inverse.map([0.0; 2]);
    let shifted = inverse.map(doc.scene.selection_translation);
    let delta = [shifted[0] - zero[0], shifted[1] - zero[1]];
    if let Some(mask) = &doc.scene.selection_mask
        && delta.iter().all(|d| (*d - d.round()).abs() < 1e-9)
    {
        let mut mask = mask.clone();
        for r in &mut mask.runs {
            let x = r.x as i64 + delta[0].round() as i64;
            let y = r.y as i64 + delta[1].round() as i64;
            r.x = i32::try_from(x).map_err(|_| "Selection translation range")?;
            r.y = i32::try_from(y).map_err(|_| "Selection translation range")?;
        }
        if !mask.valid() {
            return Err("Selection translation range".into());
        }
        return Ok(mask);
    }
    if let Some(r) = doc.scene.selection
        && doc.scene.selection_mask.is_none()
    {
        return rasterize(
            doc,
            &[r.min, [r.max[0], r.min[1]], r.max, [r.min[0], r.max[1]]]
                .map(|p| [p[0] + delta[0], p[1] + delta[1]]),
        );
    }
    if !Shape::capture(&doc.scene).restricted() {
        return Ok(full.clone());
    }
    if full.runs.iter().map(|r| r.length as usize).sum::<usize>() > 4 * 1024 * 1024 {
        return Err("Fractional selection conversion exceeds pixel work budget".into());
    }
    let mut out = Mask::default();
    for r in &full.runs {
        for x in r.x..r.x + r.length as i32 {
            let p = doc.scene.transform.map([x as f64 + 0.5, r.y as f64 + 0.5]);
            push(&mut out, x, r.y, 1, doc.scene.selection_coverage(p))?;
        }
    }
    Ok(out)
}
pub fn invert(doc: &mut Document) -> Result<bool, String> {
    if !Shape::capture(&doc.scene).restricted() {
        return selected(
            doc,
            Shape {
                mask: Some(Mask::default()),
                ..Default::default()
            },
        );
    }
    let full = rasterize(doc, &canvas(doc))?;
    let old = current_mask(doc, &full)?;
    let mut output = Mask::default();
    for run in &full.runs {
        let end = run.x + run.length as i32;
        let mut cuts = vec![run.x, end];
        let start = old.runs.partition_point(|r| r.y < run.y);
        for r in old.runs[start..].iter().take_while(|r| r.y == run.y) {
            let a = r.x.max(run.x);
            let b = (r.x + r.length as i32).min(end);
            if a < b {
                cuts.extend([a, b]);
            }
        }
        cuts.sort_unstable();
        cuts.dedup();
        for c in cuts.windows(2) {
            let alpha = run
                .alpha
                .saturating_sub(old.coverage([c[0] as f64 + 0.5, run.y as f64 + 0.5]));
            push(&mut output, c[0], run.y, (c[1] - c[0]) as u32, alpha)?;
        }
    }
    selected(
        doc,
        Shape {
            mask: Some(output),
            ..Default::default()
        },
    )
}
impl Document {
    pub fn fork_page(&self) -> Result<Self, String> {
        let state = self.snapshot();
        if state.memory_bytes() > crate::document::DEFAULT_DOCUMENT_BUDGET {
            return Err("Page copy exceeds document memory budget".into());
        }
        let mut doc = Self::create(
            crate::document::CanvasSpec {
                width: self.width,
                height: self.height,
                dpi: self.dpi,
                background: crate::document::Background::Transparent,
            },
            crate::document::DEFAULT_DOCUMENT_BUDGET,
        )
        .map_err(|e| e.to_string())?;
        state.restore(&mut doc);
        doc.scene.selections.fork_page();
        if serde_json::to_vec(&doc.scene)
            .map_err(|e| e.to_string())?
            .len()
            > 4 * 1024 * 1024
        {
            return Err("Copied page exceeds native geometry budget".into());
        }
        doc.current_frame = self.current_frame;
        doc.title = format!("{} copy", self.title);
        doc.view = self.view;
        doc.tool = self.tool;
        Ok(doc)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn doc() -> Document {
        Document::create(
            crate::document::CanvasSpec {
                width: 32,
                height: 24,
                dpi: 300.0,
                background: crate::document::Background::Transparent,
            },
            65536,
        )
        .unwrap()
    }
    #[test]
    fn rectangle_lasso_antialias_inverse_undo_and_native() {
        let mut d = doc();
        let pixels = d.layers.clone();
        rectangle(&mut d, [1.25, 2.5], [10.75, 12.0]).unwrap();
        assert_eq!(d.scene.selection_coverage([1.5, 2.5]), 96);
        assert_eq!(d.scene.selection_coverage([2.5, 3.5]), 255);
        let a = d.scene.selection_mask.clone();
        invert(&mut d).unwrap();
        assert_eq!(d.scene.selection_coverage([1.5, 2.5]), 159);
        invert(&mut d).unwrap();
        assert_eq!(d.scene.selection_mask, a);
        clear(&mut d).unwrap();
        assert!(d.scene.selection_mask.is_none());
        reselect(&mut d).unwrap();
        assert_eq!(d.scene.selection_mask, a);
        let before = d.snapshot();
        assert!(lasso(&mut d, &[[1.0, 1.0]; 3]).is_err());
        assert!(d.snapshot() == before);
        lasso(
            &mut d,
            &[[-4.0, 0.0], [16.0, 15.0], [40.0, 0.0], [16.0, 23.0]],
        )
        .unwrap();
        assert!(d.scene.selection_mask.as_ref().unwrap().valid());
        assert_eq!(d.layers, pixels);
        let after = d.snapshot();
        assert!(d.undo());
        assert!(d.snapshot() == before);
        assert!(d.redo());
        assert!(d.snapshot() == after);
        let root = crate::test_support::TempDir::new();
        let path = root.0.join("selection.drawer");
        crate::files::save(&mut d, &path).unwrap();
        assert!(crate::files::open(&path, 65536).unwrap().snapshot() == after);
    }
    #[test]
    fn unrestricted_empty_full_outside_and_large_scanlines() {
        let mut d = doc();
        invert(&mut d).unwrap();
        assert_eq!(d.scene.selection_coverage([8.5, 8.5]), 0);
        invert(&mut d).unwrap();
        assert_eq!(d.scene.selection_coverage([8.5, 8.5]), 255);
        rectangle(&mut d, [-20.0, -20.0], [-1.0, -1.0]).unwrap();
        assert_eq!(d.scene.selection_coverage([0.5, 0.5]), 0);
        let mut large = Document::create(
            crate::document::CanvasSpec {
                width: 32768,
                height: 1,
                dpi: 300.0,
                background: crate::document::Background::Transparent,
            },
            1024 * 1024,
        )
        .unwrap();
        rectangle(&mut large, [0.0, 0.0], [32768.0, 1.0]).unwrap();
        assert_eq!(large.scene.selection_mask.as_ref().unwrap().runs.len(), 1);
        assert_eq!(large.scene.selection_coverage([32767.5, 0.5]), 255);
    }
}

/// Own two-page/two-cell fixture used by the installed app; no original assets.
pub fn verify_pages(root: &std::path::Path) -> Result<(), String> {
    use crate::{
        animation::{Cell, Exposure, Fps, Timeline, Track},
        document::{Background, CanvasSpec},
    };
    use std::sync::Arc;
    let mut a = Document::create(
        CanvasSpec {
            width: 32,
            height: 24,
            dpi: 300.0,
            background: Background::Transparent,
        },
        65536,
    )
    .map_err(|e| e.to_string())?;
    a.animation = Some(Timeline {
        fps: Fps::default(),
        frames: 2,
        tracks: vec![Track {
            name: "Own cells".into(),
            cells: vec![
                Cell {
                    name: "Red".into(),
                    width: 1,
                    height: 1,
                    offset: [0, 0],
                    pixels: Arc::new(vec![[255, 0, 0, 255]]),
                },
                Cell {
                    name: "Blue".into(),
                    width: 1,
                    height: 1,
                    offset: [0, 0],
                    pixels: Arc::new(vec![[0, 0, 255, 255]]),
                },
            ],
            exposures: vec![
                Exposure { frame: 0, cell: 0 },
                Exposure { frame: 1, cell: 1 },
            ],
            end: 2,
        }],
        audio: vec![],
    });
    rectangle(&mut a, [1.25, 2.5], [16.75, 18.0])?;
    save_named(&mut a, "Page area")?;
    let active = a.scene.selection_mask.clone();
    let history = a.history_counts();
    let animation = a.animation.clone();
    let page = a.scene.selections.page.clone();
    a.current_frame = 1;
    if a.scene.selection_mask != active || a.history_counts() != history {
        return Err("Cell/frame switch changed page selection/history".into());
    }
    let mut b = a.fork_page()?;
    if b.scene.selections.page == page {
        return Err("Copied page reused selection identity".into());
    }
    let original = a.snapshot();
    rectangle(&mut b, [20.0, 4.0], [30.0, 12.0])?;
    save_named(&mut b, "Page area")?;
    let saved = b.scene.selection_mask.clone();
    clear(&mut b)?;
    load_named(&mut b, 1)?;
    if b.scene.selection_mask != saved || a.snapshot() != original {
        return Err("Named page selection crossed pages".into());
    }
    b.current_frame = 0;
    let before = b.scene.clone();
    if crate::render::Renderer::new(&b).pixel([0.5, 0.5]) != [255, 0, 0, 255]
        || crate::render::Renderer::new(&a).pixel([0.5, 0.5]) != [0, 0, 255, 255]
    {
        return Err("Two-cell frame render mismatch".into());
    }
    if b.scene != before || b.animation != animation || a.current_frame != 1 {
        return Err("Reference render/cell switch changed page selection or cell pixels".into());
    }
    let path = root.join("two-page-selection.drawer");
    crate::files::save(&mut b, &path).map_err(|e| e.to_string())?;
    let reopened = crate::files::open(&path, 65536).map_err(|e| e.to_string())?;
    if reopened.snapshot() != b.snapshot() {
        return Err("Page selections/last/names/IDs/cells native roundtrip failed".into());
    }
    Ok(())
}
#[cfg(test)]
mod page_tests {
    use super::*;
    #[test]
    fn pages_cells_named_ids_native_and_capacity_errors() {
        let root = crate::test_support::TempDir::new();
        verify_pages(&root.0).unwrap();
        let mut d = Document::create(
            crate::document::CanvasSpec {
                width: 4,
                height: 4,
                dpi: 300.0,
                background: crate::document::Background::Transparent,
            },
            4096,
        )
        .unwrap();
        rectangle(&mut d, [0.0, 0.0], [2.0, 2.0]).unwrap();
        for i in 0..16 {
            save_named(&mut d, &format!("Area {i}")).unwrap();
        }
        let before = d.snapshot();
        assert!(save_named(&mut d, "Seventeenth").is_err());
        assert!(d.snapshot() == before);
        assert!(save_named(&mut d, " ").is_err());
        assert!(d.snapshot() == before);
        assert!(load_named(&mut d, u64::MAX).is_err());
        assert!(d.snapshot() == before);
        delete_named(&mut d, 1).unwrap();
        assert!(d.undo());
        assert!(d.snapshot() == before);
    }
    #[test]
    fn scanlines_match_independent_pointwise_coverage() {
        let mut d = Document::create(
            crate::document::CanvasSpec {
                width: 16,
                height: 16,
                dpi: 300.0,
                background: crate::document::Background::Transparent,
            },
            65536,
        )
        .unwrap();
        for points in [
            vec![[-2.0, 1.25], [15.75, 3.0], [4.25, 19.0]],
            vec![[0.25, 0.5], [14.75, 14.5], [0.25, 14.5], [14.75, 0.5]],
        ] {
            lasso(&mut d, &points).unwrap();
            for y in 0..16 {
                for x in 0..16 {
                    let mut hits = 0;
                    for sy in [0.125, 0.375, 0.625, 0.875] {
                        for sx in [0.125, 0.375, 0.625, 0.875] {
                            let p = [x as f64 + sx, y as f64 + sy];
                            let mut inside = false;
                            for i in 0..points.len() {
                                let a = points[i];
                                let b = points[(i + 1) % points.len()];
                                if (a[1] > p[1]) != (b[1] > p[1])
                                    && p[0] < a[0] + (p[1] - a[1]) * (b[0] - a[0]) / (b[1] - a[1])
                                {
                                    inside = !inside;
                                }
                            }
                            hits += u32::from(inside);
                        }
                    }
                    assert_eq!(
                        d.scene.selection_coverage([x as f64 + 0.5, y as f64 + 0.5]),
                        ((hits * 255 + 8) / 16) as u8
                    );
                }
            }
            let a = d.scene.selection_mask.clone();
            invert(&mut d).unwrap();
            invert(&mut d).unwrap();
            assert_eq!(d.scene.selection_mask, a);
        }
    }
}

#[cfg(test)]
mod transform_tests {
    use super::*;
    #[test]
    fn saved_selection_translation_follows_page_rotation_and_legacy_defaults() {
        let mut d = Document::create(
            crate::document::CanvasSpec {
                width: 4,
                height: 4,
                dpi: 300.0,
                background: crate::document::Background::Transparent,
            },
            65536,
        )
        .unwrap();
        rectangle(&mut d, [0.0, 0.0], [2.0, 2.0]).unwrap();
        d.scene.selection_translation = [1.0, 0.0];
        save_named(&mut d, "Moved").unwrap();
        let state = crate::sizing::prepare(
            &d.snapshot(),
            crate::sizing::Change::Rotate90 { clockwise: true },
            65536,
            &std::sync::atomic::AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(
            state.scene.selections.saved[0].shape.translation,
            [0.0, 1.0]
        );
        let root = crate::test_support::TempDir::new();
        let mut legacy = Document::create(
            crate::document::CanvasSpec {
                width: 2,
                height: 2,
                dpi: 300.0,
                background: crate::document::Background::Transparent,
            },
            4096,
        )
        .unwrap();
        let path = root.0.join("legacy.drawer");
        crate::files::save(&mut legacy, &path).unwrap();
        let mut bytes = std::fs::read(&path).unwrap();
        bytes[8..12].copy_from_slice(&32u32.to_le_bytes());
        std::fs::write(&path, bytes).unwrap();
        let old = crate::files::open(&path, 4096).unwrap();
        assert!(old.scene.selections.is_default());
    }
}
