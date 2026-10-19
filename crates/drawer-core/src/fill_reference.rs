//! Read-only fill reference policy. Tags belong to the page; filters to the tool.
use crate::{
    bucket::{Reference, Settings},
    document::Document,
    scene::PageObject,
};
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Role {
    #[default]
    Drawing,
    Text,
    Balloon,
    Paper,
}
impl Role {
    pub const ALL: [Self; 4] = [Self::Drawing, Self::Text, Self::Balloon, Self::Paper];
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tags {
    pub role: Role,
    pub draft: bool,
    pub reference: bool,
}
impl Tags {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Filter {
    pub text: bool,
    pub balloon: bool,
    pub draft: bool,
    pub editing: bool,
    pub paper: bool,
    pub locked: bool,
}
impl Filter {
    pub fn accepts(self, t: Tags, locked: bool) -> bool {
        !(self.text && t.role == Role::Text
            || self.balloon && t.role == Role::Balloon
            || self.paper && t.role == Role::Paper
            || self.draft && t.draft
            || self.locked && locked)
    }
}
pub fn selected(bits: [u64; 4], n: usize) -> bool {
    n < 256 && bits[n / 64] & (1u64 << (n % 64)) != 0
}
pub fn select(bits: &mut [u64; 4], n: usize, on: bool) {
    if n < 256 {
        if on {
            bits[n / 64] |= 1u64 << (n % 64);
        } else {
            bits[n / 64] &= !(1u64 << (n % 64));
        }
    }
}
pub(crate) fn in_folder(doc: &Document, folder: usize, layer: usize, depth: usize) -> bool {
    depth < 16
        && doc.scene.paper.folders.get(folder).is_some_and(|f| {
            f.children.iter().any(|n| match *n {
                crate::paper::Node::Raster(i) => i == layer,
                crate::paper::Node::Folder(i) => in_folder(doc, i, layer, depth + 1),
            })
        })
}
pub fn raster(doc: &Document, n: usize, active: usize, s: Settings) -> bool {
    let Some(layer) = doc.layers.get(n) else {
        return false;
    };
    let target = crate::layer_properties::raster(doc, n);
    let tags = crate::layer_roles::tags(doc, target);
    let scope = match s.reference {
        Reference::Active => n == active,
        Reference::Layer(i) => n == i as usize,
        Reference::Composite => true,
        Reference::Marked => tags.reference,
        Reference::Selected => selected(s.selected, n),
        Reference::Folder(i) => in_folder(doc, i as usize, n, 0),
    };
    scope
        && (s.include_hidden || doc.scene.paper.raster_visible(n, doc.layers.len()))
        && !(s.exclude.editing && n == active)
        && s.exclude.accepts(
            tags,
            layer.locked || crate::layer_properties::inherited(doc, target).locked,
        )
}
pub fn vector(layer: &crate::line_tools::Layer, index: usize, s: Settings) -> bool {
    let scope = match s.reference {
        Reference::Composite => true,
        Reference::Marked => layer.fill.reference,
        Reference::Selected => selected(s.selected_vectors, index),
        _ => false,
    };
    scope && (s.include_hidden || !layer.hidden) && s.exclude.accepts(layer.fill, layer.locked)
}
pub fn object(object: &PageObject, s: Settings) -> bool {
    s.reference == Reference::Composite
        && !(s.exclude.text && matches!(object, PageObject::Text { .. }))
}
pub fn validate(doc: &Document, active: usize, s: Settings) -> Result<(), String> {
    match s.reference {
        Reference::Active if active >= doc.layers.len() => {
            Err("Missing active fill reference".into())
        }
        Reference::Layer(n) if n as usize >= doc.layers.len() => {
            Err("Missing fill reference layer".into())
        }
        Reference::Folder(n) if n as usize >= doc.scene.paper.folders.len() => {
            Err("Missing fill reference folder".into())
        }
        _ => Ok(()),
    }
}
#[derive(Clone, Copy)]
pub enum Target {
    Raster(usize),
    Vector(u64),
}
pub fn apply_tags(doc: &mut Document, edits: &[(Target, Tags)]) -> Result<bool, String> {
    let mut scene = doc.scene.clone();
    for (target, tags) in edits {
        match target {
            Target::Raster(n) => {
                if *n >= doc.layers.len() {
                    return Err("Missing layer for fill roles".into());
                }
                scene.paper.layers.entry(*n).or_default().fill = *tags;
            }
            Target::Vector(id) => {
                scene
                    .lines
                    .layers
                    .iter_mut()
                    .find(|l| l.id == *id)
                    .ok_or("Missing vector for fill roles")?
                    .fill = *tags
            }
        }
    }
    if serde_json::to_vec(&scene).map_err(|e| e.to_string())?.len() > 4 * 1024 * 1024 {
        return Err("Fill roles exceed scene serialization limit".into());
    }
    doc.edit_scene(scene)
}
pub(crate) fn graph(doc: &Document, active: usize, s: Settings) -> crate::paper::Graph {
    let mut graph = doc.scene.paper.clone();
    for n in 0..doc.layers.len() {
        if !raster(doc, n, active, s) {
            graph.layers.entry(n).or_default().hidden = true;
        }
    }
    for i in 0..graph.folders.len() {
        let included =
            (0..doc.layers.len()).any(|n| in_folder(doc, i, n, 0) && raster(doc, n, active, s));
        if !included {
            graph.folders[i].paper.hidden = true;
        }
    }
    graph
}
pub(crate) struct Sampler<'a> {
    doc: &'a Document,
    active: usize,
    settings: Settings,
    raw: Option<usize>,
    renderer: Option<crate::render::Renderer<'a>>,
}
impl<'a> Sampler<'a> {
    pub fn new(doc: &'a Document, active: usize, s: Settings) -> Result<Self, String> {
        if !s.valid() {
            return Err("Invalid fill reference settings".into());
        }
        validate(doc, active, s)?;
        let count = doc.width as usize * doc.height as usize;
        if count > 4 * 1024 * 1024
            || count
                .saturating_mul(12)
                .saturating_add(doc.resident_bytes())
                .saturating_add(doc.memory_bytes().saturating_mul(2))
                > crate::document::DEFAULT_DOCUMENT_BUDGET
        {
            return Err("Fill reference exceeds work/memory limit".into());
        }
        let raw = match s.reference {
            Reference::Active => Some(active),
            Reference::Layer(n) => Some(n as usize),
            _ => None,
        };
        if let Some(n) = raw
            && !doc.layers[n].valid(doc.width, doc.height)
        {
            return Err("Invalid raster fill reference".into());
        }
        let complexity = doc
            .scene
            .gradients
            .layers
            .iter()
            .map(|l| l.areas.len() * 3 + 1)
            .sum::<usize>()
            + doc.layers.len()
            + doc.scene.objects.len()
            + doc
                .scene
                .lines
                .layers
                .iter()
                .flat_map(|l| &l.paths)
                .map(|p| p.points.len())
                .sum::<usize>()
            + 1;
        if raw.is_none() && count.saturating_mul(complexity) > 64 * 1024 * 1024 {
            return Err("Fill reference exceeds sampling work limit".into());
        }
        Ok(Self {
            doc,
            active,
            settings: s,
            raw,
            renderer: raw
                .is_none()
                .then(|| crate::render::Renderer::fill(doc, active, s))
                .transpose()?,
        })
    }
    pub fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        if let Some(n) = self.raw {
            if raster(self.doc, n, self.active, self.settings) {
                self.doc.layers[n].pixel(self.doc.width, self.doc.height, x as i64, y as i64)
            } else {
                [0; 4]
            }
        } else {
            self.renderer
                .as_ref()
                .unwrap()
                .pixel([x as f64 + 0.5, y as f64 + 0.5])
        }
    }
}
