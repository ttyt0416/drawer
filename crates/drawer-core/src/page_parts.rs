//! Editable native-page containers; nested source cameras and timelines keep their scope.
use crate::{
    document::{Background, CanvasSpec, DEFAULT_DOCUMENT_BUDGET, Document},
    scene::{Affine, PageObject, Rect, Scene},
    sizing::DocumentState,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    cell::Cell,
    io::{self, Read, Write},
    sync::Arc,
};
const MAX_SOURCE: usize = 64 * 1024 * 1024;
const MAX_DEPTH: u32 = 4;
pub(crate) fn nested_depth(s: &Scene) -> u32 {
    s.page_parts.depth().max(s.cels.depth())
}
thread_local! {static DEPTH:Cell<u32>=const{Cell::new(0)};}
struct Depth;
impl Depth {
    fn enter() -> Result<Self, String> {
        DEPTH.with(|n| {
            if n.get() >= MAX_DEPTH {
                return Err("Nested page limit4".into());
            }
            n.set(n.get() + 1);
            Ok(Self)
        })
    }
}
impl Drop for Depth {
    fn drop(&mut self) {
        DEPTH.with(|n| n.set(n.get() - 1));
    }
}
#[derive(Clone)]
struct Source {
    bytes: Arc<Vec<u8>>,
    document: Document,
    export: Option<Box<Document>>,
}
impl std::fmt::Debug for Source {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NativePage")
            .field("bytes", &self.bytes.len())
            .finish()
    }
}
impl PartialEq for Source {
    fn eq(&self, o: &Self) -> bool {
        self.bytes == o.bytes
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Part {
    pub object: u64,
    pub original_page: u64,
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub dpi: f64,
    pub frame: u32,
    pub depth: u32,
    length: usize,
    hash: [u8; 32],
    #[serde(skip)]
    source: Option<Arc<Source>>,
}
impl Part {
    pub(crate) fn valid_source(&self, missing: bool) -> bool {
        self.object > 0
            && self.original_page > 0
            && !self.name.trim().is_empty()
            && self.name.len() <= 256
            && !self.name.chars().any(char::is_control)
            && (1..=32768).contains(&self.width)
            && (1..=32768).contains(&self.height)
            && self.dpi.is_finite()
            && self.dpi > 0.
            && (1..=MAX_DEPTH).contains(&self.depth)
            && self.length > 0
            && self.length <= MAX_SOURCE
            && (missing || self.document().is_some())
    }
    pub(crate) fn write_source(&self, w: &mut impl Write) -> io::Result<()> {
        let source = self
            .source
            .as_ref()
            .ok_or_else(|| invalid("Missing native source"))?;
        w.write_all(&(source.bytes.len() as u64).to_le_bytes())?;
        w.write_all(&source.bytes)
    }
    pub(crate) fn read_source(&self, r: &mut impl Read, budget: usize) -> io::Result<Self> {
        let mut size = [0; 8];
        r.read_exact(&mut size)?;
        let len = u64::from_le_bytes(size);
        if len != self.length as u64 || len > MAX_SOURCE as u64 || len > budget as u64 / 2 {
            return Err(invalid("Native source allocation limit"));
        }
        let mut bytes = vec![0; len as usize];
        r.read_exact(&mut bytes)?;
        self.clone().hydrate_payload(bytes, budget).map_err(invalid)
    }
    pub(crate) fn payload_bytes(&self) -> usize {
        self.source.as_ref().map_or(0, |s| {
            s.bytes.len()
                + s.document.memory_bytes()
                + s.export.as_ref().map_or(0, |d| d.memory_bytes())
        })
    }
    pub fn document(&self) -> Option<&Document> {
        self.source.as_ref().map(|s| &s.document)
    }
    pub fn source_hash(&self) -> [u8; 32] {
        self.hash
    }
    pub fn export_document(&self) -> Option<&Document> {
        self.source
            .as_ref()
            .map(|s| s.export.as_deref().unwrap_or(&s.document))
    }
    pub(crate) fn output_policy(
        &mut self,
        view: crate::animation_render::View,
    ) -> Result<(), String> {
        if let Some(source) = &self.source {
            let mut source = (**source).clone();
            let mut document =
                crate::layer_merge::preview(&source.document.snapshot(), self.frame)?;
            crate::animation_render::apply(&mut document, view, false)?;
            source.export = Some(Box::new(document));
            self.source = Some(Arc::new(source));
        }
        Ok(())
    }
    pub fn open(&self) -> Result<Document, String> {
        let d = self.document().ok_or("Missing embedded page")?;
        let mut opened = crate::layer_merge::preview(&d.snapshot(), self.frame)?;
        opened.title = self.name.clone();
        Ok(opened)
    }
    pub(crate) fn hydrate_payload(mut self, bytes: Vec<u8>, budget: usize) -> Result<Self, String> {
        let _depth = Depth::enter()?;
        if bytes.len() != self.length
            || bytes.len() > MAX_SOURCE
            || <[u8; 32]>::from(Sha256::digest(&bytes)) != self.hash
        {
            return Err("Embedded page payload length/hash mismatch".into());
        }
        let mut d = crate::files::read_native_snapshot(
            std::path::Path::new("embedded-page.drawer"),
            &bytes,
            budget.saturating_sub(bytes.len()),
        )
        .map_err(|e| e.to_string())?;
        if d.width != self.width
            || d.height != self.height
            || d.dpi != self.dpi
            || self.depth != 1 + nested_depth(&d.scene)
            || self.frame >= d.animation.as_ref().map_or(1, |a| a.frames)
        {
            return Err("Embedded page dimensions, frame or nesting mismatch".into());
        }
        d.current_frame = self.frame;
        d.path = None;
        d.source_path = None;
        d.title = self.name.clone();
        let export = crate::layer_roles::output_view(&d)?;
        self.source = Some(Arc::new(Source {
            bytes: Arc::new(bytes),
            document: d,
            export,
        }));
        Ok(self)
    }
    pub(crate) fn create(
        object: u64,
        original_page: u64,
        name: String,
        d: &Document,
        budget: usize,
    ) -> Result<Self, String> {
        let bytes = crate::files::native_bytes(d).map_err(|e| e.to_string())?;
        let p = Self {
            object,
            original_page,
            name,
            width: d.width,
            height: d.height,
            dpi: d.dpi,
            frame: d.current_frame,
            depth: 1 + nested_depth(&d.scene),
            length: bytes.len(),
            hash: Sha256::digest(&bytes).into(),
            source: None,
        };
        if p.depth > MAX_DEPTH {
            return Err("Nested page limit4".into());
        }
        p.hydrate_payload(bytes, budget)
    }
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Data {
    pub entries: Vec<Part>,
}
impl Data {
    pub fn is_default(&self) -> bool {
        self.entries.is_empty()
    }
    pub fn get(&self, id: u64) -> Option<&Part> {
        self.entries.iter().find(|p| p.object == id)
    }
    pub fn depth(&self) -> u32 {
        self.entries.iter().map(|p| p.depth).max().unwrap_or(0)
    }
    pub fn payload_bytes(&self) -> usize {
        self.entries
            .iter()
            .filter_map(|p| p.source.as_ref())
            .map(|s| {
                s.bytes
                    .len()
                    .saturating_add(s.document.memory_bytes())
                    .saturating_add(s.export.as_ref().map_or(0, |d| d.memory_bytes()))
            })
            .sum()
    }
    pub fn bytes(&self) -> usize {
        if self.is_default() {
            0
        } else {
            self.entries
                .iter()
                .map(|p| std::mem::size_of::<Part>() + p.name.len())
                .sum::<usize>()
                .saturating_add(self.payload_bytes())
        }
    }
    pub fn valid(&self, s: &Scene, missing: bool) -> bool {
        self.entries.len()<=128 && self.entries.iter().enumerate().all(|(n,p)|p.object>0 && p.original_page>0 && !p.name.trim().is_empty() && p.name.len()<=256 && !p.name.chars().any(char::is_control) && (1..=32768).contains(&p.width) && (1..=32768).contains(&p.height) && p.dpi.is_finite() && p.dpi>0. && (1..=MAX_DEPTH).contains(&p.depth) && p.length>0 && p.length<=MAX_SOURCE && !self.entries[..n].iter().any(|q|q.object==p.object) && s.catalog.objects.index(p.object,s.objects.len()).is_some_and(|i|matches!(&s.objects[i],PageObject::Material(m)if matches!(m.source,crate::material_layer::Source::Solid([0,0,0,0])))) && (missing || p.document().is_some()))
    }
}
fn invalid(e: impl ToString) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, e.to_string())
}
pub(crate) fn write(s: &Scene, w: &mut impl Write) -> io::Result<()> {
    for p in &s.page_parts.entries {
        let source = p
            .source
            .as_ref()
            .ok_or_else(|| invalid("Missing embedded page"))?;
        w.write_all(&(source.bytes.len() as u64).to_le_bytes())?;
        w.write_all(&source.bytes)?;
    }
    Ok(())
}
pub(crate) fn read(d: &mut Document, r: &mut impl Read, budget: usize) -> io::Result<()> {
    for n in 0..d.scene.page_parts.entries.len() {
        let mut size = [0; 8];
        r.read_exact(&mut size)?;
        let len = u64::from_le_bytes(size);
        let p = d.scene.page_parts.entries[n].clone();
        let remaining = budget.saturating_sub(d.memory_bytes());
        if len != p.length as u64 || len > MAX_SOURCE as u64 || len > remaining as u64 / 2 {
            return Err(invalid("Embedded page allocation limit"));
        }
        let mut bytes = vec![0; len as usize];
        r.read_exact(&mut bytes)?;
        d.scene.page_parts.entries[n] = p.hydrate_payload(bytes, remaining).map_err(invalid)?;
        if d.memory_bytes() > budget {
            return Err(invalid("Embedded page memory limit"));
        }
    }
    Ok(())
}
pub(crate) fn prune(state: &mut DocumentState) {
    let ids = &state.scene.catalog.objects;
    let objects = &state.scene.objects;
    state.scene.page_parts.entries.retain(|p| {
        ids.index(p.object, objects.len())
            .is_some_and(|i| matches!(objects[i], PageObject::Material(_)))
    });
}
pub(crate) fn duplicate(
    s: &mut Scene,
    original: &Scene,
    map: &std::collections::BTreeMap<crate::layer_search::Key, crate::layer_search::Key>,
) {
    use crate::layer_search::Key;
    for p in &original.page_parts.entries {
        if let Some(Key::Object(id)) = map.get(&Key::Object(p.object)) {
            let mut p = p.clone();
            p.object = *id;
            p.name = format!("{} copy", p.name.chars().take(59).collect::<String>());
            s.page_parts.entries.push(p);
        }
    }
}
pub fn merge(
    pages: &[(u64, &Document)],
    physical: bool,
    budget: usize,
) -> Result<Document, String> {
    if pages.len() < 2 || pages.len() > 16 {
        return Err("Choose 2..16 pages to combine".into());
    }
    let dpi = pages[0].1.dpi;
    let sizes = pages
        .iter()
        .map(|(_, d)| {
            let scale = if physical { dpi / d.dpi } else { 1. };
            (
                (d.width as f64 * scale).round().max(1.) as u32,
                (d.height as f64 * scale).round().max(1.) as u32,
                scale,
            )
        })
        .collect::<Vec<_>>();
    let width = sizes
        .iter()
        .try_fold(0u32, |n, s| n.checked_add(s.0))
        .ok_or("Combined page width overflow")?;
    let height = sizes.iter().map(|s| s.1).max().unwrap();
    let mut d = Document::create_with_color(
        CanvasSpec {
            width,
            height,
            dpi,
            background: Background::Transparent,
        },
        budget,
        pages[0].1.color.clone(),
    )
    .map_err(|e| e.to_string())?;
    d.scene.comics.page = pages[0].1.scene.comics.page.clone();
    d.scene.comics.page.trim_mm = [0.; 2];
    d.scene.comics.page.bleed_mm = 0.;
    d.scene.comics.page.safe_mm = 0.;
    let mut x = 0.;
    for ((page_id, source), (w, _, scale)) in pages.iter().zip(sizes) {
        if source.scene.page_parts.depth() >= MAX_DEPTH {
            return Err("Nested page limit4".into());
        }
        let before = d.scene.clone();
        d.scene.objects.push(PageObject::Material(Box::new(
            crate::material_layer::Layer {
                source: crate::material_layer::Source::Solid([0; 4]),
                area: Rect {
                    min: [0., 0.],
                    max: [source.width as f64, source.height as f64],
                },
                transform: Affine([scale, 0., 0., scale, x, 0.]),
                repeat: [false; 2],
                mapping: Default::default(),
            },
        )));
        crate::layer_catalog::reconcile_scene(&before, &mut d.scene)?;
        let id = d.scene.catalog.objects.id(d.scene.objects.len() - 1);
        let name = format!(
            "Page#{page_id} · {}",
            source.title.chars().take(40).collect::<String>()
        );
        let p = Part::create(
            id,
            *page_id,
            name.clone(),
            source,
            budget.saturating_sub(d.memory_bytes()),
        )?;
        d.scene.page_parts.entries.push(p);
        d.scene.catalog.object_info.entry(id).or_default().name =
            Some(name.chars().take(64).collect());
        x += w as f64;
        if d.memory_bytes() > budget {
            return Err("Combined editable pages exceed memory budget".into());
        }
    }
    if !d.scene.valid() {
        return Err("Invalid combined page".into());
    }
    d.title = "Combined pages".into();
    Ok(d)
}
pub fn split(d: &Document, x: u32, budget: usize) -> Result<[Document; 2], String> {
    if x == 0 || x >= d.width {
        return Err("Split must be inside the page width".into());
    }
    let source = d.snapshot();
    let cancel = std::sync::atomic::AtomicBool::new(false);
    let mut out = vec![];
    for (left, width) in [(0, x), (x, d.width - x)] {
        let state = crate::sizing::prepare(
            &source,
            crate::sizing::Change::Crop {
                x: left,
                y: 0,
                width,
                height: d.height,
            },
            budget,
            &cancel,
        )?;
        let mut page = crate::layer_merge::preview(&state, d.current_frame)?;
        page.title = if out.is_empty() {
            "Left page"
        } else {
            "Right page"
        }
        .into();
        page.scene.comics.page.trim_mm = [0.; 2];
        page.scene.comics.page.bleed_mm = 0.;
        page.scene.comics.page.safe_mm = 0.;
        out.push(page);
    }
    let [a, b]: [Document; 2] = out.try_into().map_err(|_| "Split page count")?;
    Ok([a, b])
}
pub fn replace(
    d: &Document,
    id: u64,
    source: &Document,
    budget: usize,
) -> Result<DocumentState, String> {
    let mut state = d.snapshot();
    let old = state
        .scene
        .page_parts
        .get(id)
        .ok_or("Choose a contained page")?
        .clone();
    if source.width != old.width || source.height != old.height || source.dpi != old.dpi {
        return Err(
            "Keep the contained page size/DPI when applying; split or resize outside first".into(),
        );
    }
    let part = Part::create(
        id,
        old.original_page,
        old.name,
        source,
        budget.saturating_sub(d.resident_bytes()),
    )?;
    *state
        .scene
        .page_parts
        .entries
        .iter_mut()
        .find(|p| p.object == id)
        .unwrap() = part;
    if !state.scene.valid() || state.memory_bytes().saturating_add(d.resident_bytes()) > budget {
        return Err("Contained page update exceeds Undo budget".into());
    }
    Ok(state)
}
pub const DEFAULT_BUDGET: usize = DEFAULT_DOCUMENT_BUDGET;
