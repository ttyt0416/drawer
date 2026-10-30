//! Global editable cel definitions, independent from track-local exposure slots.
use crate::{
    animation::Cell,
    document::{Background, CanvasSpec, Document},
    layer_search::Key,
    page_parts::Part,
    scene::Scene,
    sizing::DocumentState,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::{self, Read, Write},
    sync::Arc,
};
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Link {
    pub track: u64,
    pub cell: u64,
    pub source: u64,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Label {
    pub id: u64,
    pub name: String,
    pub frame: u32,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Data {
    pub next: u64,
    pub next_label: u64,
    pub definitions: Vec<Part>,
    pub links: Vec<Link>,
    pub labels: Vec<Label>,
}
impl Default for Data {
    fn default() -> Self {
        Self {
            next: 1,
            next_label: 1,
            definitions: vec![],
            links: vec![],
            labels: vec![],
        }
    }
}
impl Data {
    pub(crate) fn same_sources(&self, other: &Self) -> bool {
        self.links == other.links
            && self.definitions.len() == other.definitions.len()
            && self
                .definitions
                .iter()
                .zip(&other.definitions)
                .all(|(a, b)| a.same_content(b))
    }

    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
    pub fn get(&self, id: u64) -> Option<&Part> {
        self.definitions.iter().find(|p| p.object == id)
    }
    pub fn source(&self, track: u64, cell: u64) -> Option<u64> {
        self.links
            .iter()
            .find(|l| l.track == track && l.cell == cell)
            .map(|l| l.source)
    }
    pub fn depth(&self) -> u32 {
        self.definitions.iter().map(|p| p.depth).max().unwrap_or(0)
    }
    pub fn payload_bytes(&self) -> usize {
        self.definitions.iter().map(Part::payload_bytes).sum()
    }
    pub fn bytes(&self) -> usize {
        if self.is_default() {
            0
        } else {
            128 + self.payload_bytes()
                + self
                    .definitions
                    .iter()
                    .map(|p| 256 + p.name.len())
                    .sum::<usize>()
                + self.links.len() * 32
                + self.labels.iter().map(|l| 32 + l.name.len()).sum::<usize>()
        }
    }
    pub fn valid(&self, missing: bool) -> bool {
        self.next > 0
            && self.next_label > 0
            && self.definitions.len() <= 512
            && self.links.len() <= 10000
            && self.labels.len() <= 1024
            && self.definitions.iter().all(|p| {
                p.object < self.next
                    && p.valid_source(missing)
                    && p.document()
                        .is_none_or(|d| d.animation.is_none() && d.scene.cels.is_default())
            })
            && self
                .definitions
                .iter()
                .map(|p| p.object)
                .collect::<BTreeSet<_>>()
                .len()
                == self.definitions.len()
            && self
                .links
                .iter()
                .all(|l| l.track > 0 && l.cell > 0 && self.get(l.source).is_some())
            && self
                .links
                .iter()
                .map(|l| (l.track, l.cell))
                .collect::<BTreeSet<_>>()
                .len()
                == self.links.len()
            && self.labels.iter().all(|l| {
                l.id > 0
                    && l.id < self.next_label
                    && !l.name.trim().is_empty()
                    && l.name.len() <= 256
                    && !l.name.chars().any(char::is_control)
                    && l.frame < crate::animation::MAX_FRAMES
            })
            && self
                .labels
                .iter()
                .map(|l| l.id)
                .collect::<BTreeSet<_>>()
                .len()
                == self.labels.len()
    }
}
pub fn valid_document(d: &Document) -> bool {
    d.scene.cels.valid(false)
        && d.scene.cels.links.iter().all(|l| {
            slot(d, l.track, l.cell).is_some_and(|(_, _, c)| {
                d.scene
                    .cels
                    .get(l.source)
                    .is_some_and(|p| p.width == c.width && p.height == c.height)
            })
        })
        && d.scene.cels.labels.iter().all(|l| {
            !d.scene.timing.labels_linked
                || d.animation.as_ref().is_some_and(|a| l.frame < a.frames)
        })
}
pub fn slot(d: &Document, track: u64, cell: u64) -> Option<(usize, usize, &Cell)> {
    let a = d.animation.as_ref()?;
    let t = d.scene.catalog.tracks.index(track, a.tracks.len())?;
    let c = d
        .scene
        .catalog
        .cells
        .get(&track)
        .cloned()
        .unwrap_or_default()
        .index(cell, a.tracks[t].cells.len())?;
    Some((t, c, &a.tracks[t].cells[c]))
}
pub fn editable(d: &Document, t: crate::layer_properties::Target) -> Result<(), String> {
    let p = crate::layer_properties::inherited(d, t);
    if p.hidden || p.locked || p.pixels_locked || p.position_locked {
        return Err("Show/unlock every affected track and cel before editing".into());
    }
    Ok(())
}
pub fn insert(
    s: &mut DocumentState,
    d: &Document,
    name: &str,
    budget: usize,
) -> Result<u64, String> {
    if d.animation.is_some()
        || !d.scene.cels.is_default()
        || name.trim().is_empty()
        || name.len() > 256
    {
        return Err("A cel needs a static editable document and a bounded name".into());
    }
    if s.scene.cels.definitions.len() >= 512 {
        return Err("Editable cel limit512".into());
    }
    let id = s.scene.cels.next;
    s.scene.cels.next = id.checked_add(1).ok_or("Cel ID exhausted")?;
    let p = Part::create(id, id, name.into(), d, budget)?;
    s.scene.cels.definitions.push(p);
    Ok(id)
}
pub fn raster_document(d: &Document, c: &Cell, budget: usize) -> Result<Document, String> {
    let mut result = Document::create_with_color(
        CanvasSpec {
            width: c.width,
            height: c.height,
            dpi: d.dpi,
            background: Background::Transparent,
        },
        budget,
        d.color.clone(),
    )
    .map_err(|e| e.to_string())?;
    result.layers[0].pixels = c.pixels.as_ref().clone();
    if let Some(precise) = &mut result.layers[0].precise {
        for (dst, src) in precise.iter_mut().zip(c.pixels.iter()) {
            *dst = crate::color_precision::Sample::from8(*src).rgba16();
        }
    }
    result.title = c.name.clone();
    Ok(result)
}
pub fn pixels(d: &Document, budget: usize) -> Result<Arc<Vec<[u8; 4]>>, String> {
    if (d.width as usize)
        .checked_mul(d.height as usize)
        .is_none_or(|n| n * 4 > budget)
    {
        return Err("Cel raster preview exceeds memory budget".into());
    }
    let r = crate::render::Renderer::new(d);
    if let Some(e) = r.error() {
        return Err(e.into());
    }
    let mut pixels = Vec::with_capacity(d.width as usize * d.height as usize);
    for y in 0..d.height {
        for x in 0..d.width {
            pixels.push(r.pixel([x as f64 + 0.5, y as f64 + 0.5]));
        }
    }
    Ok(Arc::new(pixels))
}
pub fn replace(
    d: &Document,
    id: u64,
    child: &Document,
    budget: usize,
) -> Result<DocumentState, String> {
    let p = d.scene.cels.get(id).ok_or("Cel definition removed")?;
    if child.animation.is_some() || !child.scene.cels.is_default() {
        return Err("Cel content must be a static document".into());
    }
    for l in d.scene.cels.links.iter().filter(|l| l.source == id) {
        let (t, c, _) = slot(d, l.track, l.cell).ok_or("Cel slot removed")?;
        editable(d, crate::layer_properties::Target::Track(t as u16))?;
        editable(
            d,
            crate::layer_properties::Target::Cell {
                track: t as u16,
                cell: c as u16,
            },
        )?;
    }
    let part = Part::create(id, id, p.name.clone(), child, budget)?;
    let cache = pixels(child, budget)?;
    let mut s = d.snapshot();
    let n = s
        .scene
        .cels
        .definitions
        .iter()
        .position(|p| p.object == id)
        .unwrap();
    s.scene.cels.definitions[n] = part;
    for l in &s.scene.cels.links {
        if l.source == id {
            let (t, c, _) = slot(d, l.track, l.cell).unwrap();
            let cell = &mut s.animation.as_mut().unwrap().tracks[t].cells[c];
            cell.width = child.width;
            cell.height = child.height;
            cell.pixels = cache.clone();
        }
    }
    crate::layer_groups::finish(d, &s)?;
    Ok(s)
}
/// Import existing raster cells only when first using the cel editor.
pub fn enable(d: &Document, budget: usize) -> Result<DocumentState, String> {
    let mut s = d.snapshot();
    let a = d.animation.as_ref().ok_or("Create an animation first")?;
    s.scene.catalog.tracks.initialize(a.tracks.len())?;
    for (n, t) in a.tracks.iter().enumerate() {
        let track = s.scene.catalog.tracks.id(n);
        s.scene
            .catalog
            .cells
            .entry(track)
            .or_default()
            .initialize(t.cells.len())?;
        for (c, cell) in t.cells.iter().enumerate() {
            let id = s.scene.catalog.cell_id(track, c);
            if s.scene.cels.source(track, id).is_some() {
                continue;
            }
            let child = raster_document(d, cell, budget)?;
            let source = insert(&mut s, &child, &cell.name, budget)?;
            s.scene.cels.links.push(Link {
                track,
                cell: id,
                source,
            });
        }
    }
    crate::layer_groups::finish(d, &s)?;
    Ok(s)
}
pub(crate) fn reconcile(before: &Document, after: &mut DocumentState) -> Result<(), String> {
    // Legacy raster consumers must not silently replace an editable shared cel.
    for l in &before.scene.cels.links {
        if let Some(new) = after
            .scene
            .cels
            .links
            .iter()
            .find(|n| n.track == l.track && n.cell == l.cell && n.source == l.source)
            && before.scene.cels.get(l.source) == after.scene.cels.get(new.source)
            && let Some((_, _, old)) = slot(before, l.track, l.cell)
            && let Some(a) = &after.animation
            && let Some(t) = after.scene.catalog.tracks.index(l.track, a.tracks.len())
            && let Some(c) = after
                .scene
                .catalog
                .cells
                .get(&l.track)
                .cloned()
                .unwrap_or_default()
                .index(l.cell, a.tracks[t].cells.len())
            && (old.width != a.tracks[t].cells[c].width
                || old.height != a.tracks[t].cells[c].height
                || old.pixels != a.tracks[t].cells[c].pixels)
        {
            return Err("Edit this editable cel through Timeline / Open cel, then Apply; direct raster replacement would discard its typed content".into());
        }
    }
    let a = &after.animation;
    let ids = &after.scene.catalog;
    after.scene.cels.links.retain(|l| {
        a.as_ref().is_some_and(|a| {
            ids.tracks.index(l.track, a.tracks.len()).is_some_and(|t| {
                ids.cells
                    .get(&l.track)
                    .cloned()
                    .unwrap_or_default()
                    .index(l.cell, a.tracks[t].cells.len())
                    .is_some()
            })
        })
    });
    if let Some(a) = a {
        if after.scene.timing.labels_linked {
            after.scene.cels.labels.retain(|l| l.frame < a.frames);
        }
    } else {
        after.scene.cels.labels.clear();
    }
    Ok(())
}
pub(crate) fn duplicate(s: &mut Scene, original: &Scene, map: &BTreeMap<Key, Key>) {
    for l in &original.cels.links {
        if let Some(Key::Cell { track, cell }) = map.get(&Key::Cell {
            track: l.track,
            cell: l.cell,
        }) {
            s.cels.links.push(Link {
                track: *track,
                cell: *cell,
                source: l.source,
            });
        }
    }
}
pub(crate) fn write(s: &Scene, w: &mut impl Write) -> io::Result<()> {
    for p in &s.cels.definitions {
        p.write_source(w)?;
    }
    Ok(())
}
pub(crate) fn read(d: &mut Document, r: &mut impl Read, budget: usize) -> io::Result<()> {
    for n in 0..d.scene.cels.definitions.len() {
        let p = d.scene.cels.definitions[n].clone();
        d.scene.cels.definitions[n] = p.read_source(r, budget.saturating_sub(d.memory_bytes()))?;
        if d.memory_bytes() > budget {
            return Err(io::Error::other("Cel source budget"));
        }
    }
    Ok(())
}
