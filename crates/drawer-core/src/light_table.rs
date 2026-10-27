//! Saved reference objects with their own history; drawing pixels and history are independent.
use crate::{
    document::Document, keyframes::Pose, layer_search::Key, references::LocalImage, scene::Point,
    timeline_view::ColorMode,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    io::{self, Read, Write},
    path::{Path, PathBuf},
    sync::Arc,
};
pub const BUDGET: usize = 64 * 1024 * 1024;
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Image {
    pub id: u64,
    pub width: u32,
    pub height: u32,
    pub hash: [u8; 32],
    #[serde(skip)]
    pub pixels: Arc<Vec<[u8; 4]>>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Source {
    Layer(Key),
    File { path: PathBuf, image: u64 },
    Copy { title: String, image: u64 },
    Onion { track: Option<u64>, frame: u32 },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Scope {
    Common,
    Cell(u64),
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Reference {
    pub id: u64,
    pub name: String,
    pub scope: Scope,
    pub source: Source,
    pub pose: Pose,
    pub mode: ColorMode,
    pub color: [u8; 3],
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Data {
    pub next: u64,
    pub image_next: u64,
    pub images: Vec<Image>,
    pub references: Vec<Reference>,
    pub opacity: u8,
}
impl Default for Data {
    fn default() -> Self {
        Self {
            next: 1,
            image_next: 1,
            images: vec![],
            references: vec![],
            opacity: 30,
        }
    }
}
fn label(s: &str) -> bool {
    !s.trim().is_empty() && s.len() <= 256 && !s.chars().any(char::is_control)
}
fn local_path(path: &Path) -> bool {
    use std::path::{Component, Prefix};
    if !path.is_absolute() {
        return false;
    }
    match path.components().next() {
        Some(Component::Prefix(p)) => matches!(p.kind(), Prefix::Disk(_) | Prefix::VerbatimDisk(_)),
        _ => !path.to_string_lossy().starts_with("//"),
    }
}
fn image_id(s: &Source) -> Option<u64> {
    match s {
        Source::File { image, .. } | Source::Copy { image, .. } => Some(*image),
        _ => None,
    }
}
impl Data {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
    pub fn valid(&self, missing: bool) -> bool {
        let mut ids = BTreeSet::new();
        let mut images = BTreeSet::new();
        self.next > 0
            && self.image_next > 0
            && self.opacity <= 100
            && self.references.len() <= 128
            && self.images.len() <= 32
            && self.image_bytes() <= BUDGET
            && self.images.iter().all(|i| {
                i.id > 0
                    && i.id < self.image_next
                    && images.insert(i.id)
                    && (1..=16384).contains(&i.width)
                    && (1..=16384).contains(&i.height)
                    && (missing && i.pixels.is_empty()
                        || i.pixels.len() == i.width as usize * i.height as usize)
            })
            && self.references.iter().all(|r| {
                r.id > 0
                    && r.id < self.next
                    && ids.insert(r.id)
                    && label(&r.name)
                    && r.pose.valid()
                    && match r.scope {
                        Scope::Common => true,
                        Scope::Cell(id) => id > 0,
                    }
                    && match &r.source {
                        Source::Layer(k) => crate::layer_groups::canonical(*k),
                        Source::File { path, image } => {
                            local_path(path)
                                && path.as_os_str().len() <= 32768
                                && images.contains(image)
                        }
                        Source::Copy { title, image } => label(title) && images.contains(image),
                        Source::Onion { track, frame } => {
                            track.is_none_or(|id| id > 0) && *frame < 10000
                        }
                    }
            })
    }
    pub fn image_bytes(&self) -> usize {
        self.images.iter().fold(0usize, |total, i| {
            total.saturating_add(
                (i.width as usize)
                    .saturating_mul(i.height as usize)
                    .saturating_mul(4),
            )
        })
    }
    pub fn hashes(&self) -> bool {
        self.images
            .iter()
            .all(|i| Sha256::digest(i.pixels.as_slice().as_flattened()).as_slice() == i.hash)
    }
    pub fn get(&self, id: u64) -> Option<&Reference> {
        self.references.iter().find(|r| r.id == id)
    }
    pub fn get_mut(&mut self, id: u64) -> Option<&mut Reference> {
        self.references.iter_mut().find(|r| r.id == id)
    }
    fn intern(&mut self, image: LocalImage) -> Result<u64, String> {
        let hash: [u8; 32] = Sha256::digest(image.pixels.as_slice().as_flattened()).into();
        if let Some(i) = self
            .images
            .iter()
            .find(|i| i.width == image.width && i.height == image.height && i.hash == hash)
        {
            return Ok(i.id);
        }
        let id = self.image_next;
        self.image_next = id.checked_add(1).ok_or("Image ID limit")?;
        self.images.push(Image {
            id,
            width: image.width,
            height: image.height,
            hash,
            pixels: Arc::new(image.pixels),
        });
        Ok(id)
    }
    pub fn prune_images(&mut self) {
        let used = self
            .references
            .iter()
            .filter_map(|r| image_id(&r.source))
            .collect::<BTreeSet<_>>();
        self.images.retain(|i| used.contains(&i.id));
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pin {
    pub source: u64,
    pub child: Option<u64>,
}
#[derive(Clone, Debug)]
pub struct Session {
    pub data: Data,
    pub pin: Option<Pin>,
    pub visible: bool,
    current: u64,
    next: u64,
    saved: u64,
    instance: u64,
    undo: Vec<(u64, Data)>,
    redo: Vec<(u64, Data)>,
}
impl Default for Session {
    fn default() -> Self {
        Self::loaded(Data::default())
    }
}
impl Session {
    pub fn loaded(data: Data) -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        Self {
            instance: NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            data,
            pin: None,
            visible: false,
            current: 0,
            next: 0,
            saved: 0,
            undo: vec![],
            redo: vec![],
        }
    }
    pub fn instance(&self) -> u64 {
        self.instance
    }
    pub fn stamp(&self) -> u64 {
        self.current
    }
    pub fn dirty(&self) -> bool {
        self.current != self.saved
    }
    pub fn counts(&self) -> (usize, usize) {
        (self.undo.len(), self.redo.len())
    }
    pub fn mark_saved(&mut self) {
        self.saved = self.current;
    }
    pub fn bytes(&self) -> usize {
        let mut pointers = BTreeSet::new();
        std::iter::once(&self.data)
            .chain(self.undo.iter().chain(&self.redo).map(|(_, d)| d))
            .map(|d| {
                d.references
                    .iter()
                    .map(|r| {
                        std::mem::size_of::<Reference>()
                            + r.name.len()
                            + match &r.source {
                                Source::File { path, .. } => path.as_os_str().len() * 2,
                                Source::Copy { title, .. } => title.len(),
                                _ => 0,
                            }
                    })
                    .sum::<usize>()
                    + d.images
                        .iter()
                        .map(|i| {
                            if pointers.insert(Arc::as_ptr(&i.pixels) as usize) {
                                i.pixels.len() * 4
                            } else {
                                0
                            }
                        })
                        .sum::<usize>()
            })
            .sum()
    }
    fn commit(&mut self, mut data: Data) -> Result<bool, String> {
        data.prune_images();
        if !data.valid(false) || !data.hashes() {
            return Err("Invalid light-table references, images or memory limit".into());
        }
        if self.data == data {
            return Ok(false);
        }
        let mut trial = self.clone();
        trial.redo.clear();
        trial
            .undo
            .push((trial.current, std::mem::replace(&mut trial.data, data)));
        trial.next = trial
            .next
            .checked_add(1)
            .ok_or("Light-table history ID limit")?;
        trial.current = trial.next;
        while trial.undo.len() > 64 || trial.bytes() > BUDGET {
            if trial.undo.len() <= 1 {
                return Err("Light-table reference/history budget exceeded".into());
            }
            trial.undo.remove(0);
        }
        *self = trial;
        Ok(true)
    }
    fn step(&mut self, redo: bool) -> bool {
        let from = if redo { &mut self.redo } else { &mut self.undo };
        let Some((id, data)) = from.pop() else {
            return false;
        };
        let old = (self.current, std::mem::replace(&mut self.data, data));
        self.current = id;
        if redo {
            self.undo.push(old)
        } else {
            self.redo.push(old)
        }
        true
    }
}
impl Document {
    pub fn edit_light_table(&mut self, data: Data) -> Result<bool, String> {
        let mut next = self.light_table.clone();
        let changed = next.commit(data)?;
        if self
            .resident_bytes()
            .saturating_sub(self.light_table.bytes())
            .saturating_add(next.bytes())
            > crate::document::DEFAULT_DOCUMENT_BUDGET
        {
            return Err("Document/reference history budget exceeded".into());
        }
        if changed {
            self.light_table = next;
            self.refresh_dirty();
        }
        Ok(changed)
    }
    pub fn undo_light_table(&mut self) -> bool {
        let changed = self.light_table.step(false);
        if changed {
            self.refresh_dirty();
        }
        changed
    }
    pub fn redo_light_table(&mut self) -> bool {
        let changed = self.light_table.step(true);
        if changed {
            self.refresh_dirty();
        }
        changed
    }
}
pub enum Add {
    Layer(Key),
    File(PathBuf),
    Copy {
        origin: u64,
        title: String,
        image: LocalImage,
    },
    Onion {
        track: Option<u64>,
        frame: u32,
    },
}
pub fn self_reference(d: &Document, source: &Source, cell: Option<u64>) -> bool {
    match source {
        Source::Layer(Key::Cell { track, cell: slot }) => {
            cell.is_some_and(|id| d.scene.cels.source(*track, *slot) == Some(id))
        }
        Source::Layer(Key::Track(track)) => cell.is_some_and(|id| {
            d.animation
                .as_ref()
                .and_then(|a| {
                    d.scene
                        .catalog
                        .tracks
                        .index(*track, a.tracks.len())
                        .and_then(|n| {
                            a.indexed_visible_cells(d.current_frame)
                                .find(|(t, _, _)| *t == n)
                                .map(|(_, i, _)| d.scene.catalog.cell_id(*track, i))
                        })
                })
                .and_then(|slot| d.scene.cels.source(*track, slot))
                == Some(id)
        }),
        Source::Onion { frame, track } => {
            cell.is_some()
                && *frame == d.current_frame
                && track.is_none_or(|id| self_reference(d, &Source::Layer(Key::Track(id)), cell))
        }
        _ => false,
    }
}
pub fn same_file(a: &Path, b: &Path) -> bool {
    let a = a.canonicalize().unwrap_or_else(|_| a.to_path_buf());
    let b = b.canonicalize().unwrap_or_else(|_| b.to_path_buf());
    a.to_string_lossy()
        .eq_ignore_ascii_case(&b.to_string_lossy())
}
pub fn save_destination(d: &Document, path: &Path) -> Result<(), String> {
    if d.light_table
        .data
        .references
        .iter()
        .any(|r| matches!(&r.source,Source::File{path:p,..}if same_file(p,path)))
    {
        return Err("A reference file cannot also be this drawing's save destination".into());
    }
    Ok(())
}
pub fn capture(d: &Document) -> Result<LocalImage, String> {
    if d.width as usize * d.height as usize * 4 > BUDGET {
        return Err("Reference copy exceeds64MiB".into());
    }
    let r = crate::render::Renderer::new(d);
    if let Some(e) = r.error() {
        return Err(e.into());
    }
    let mut pixels = Vec::new();
    pixels
        .try_reserve_exact(d.width as usize * d.height as usize)
        .map_err(|e| e.to_string())?;
    for y in 0..d.height {
        for x in 0..d.width {
            pixels.push(r.display_sample([x as f64 + 0.5, y as f64 + 0.5])?.rgba8());
        }
    }
    Ok(LocalImage {
        width: d.width,
        height: d.height,
        pixels,
    })
}
fn load(d: &Document, path: &Path) -> Result<(PathBuf, LocalImage), String> {
    if !local_path(path) {
        return Err("Choose an absolute local file path".into());
    }
    let path = path.canonicalize().map_err(|e| e.to_string())?;
    if d.path
        .iter()
        .chain(&d.source_path)
        .any(|p| same_file(p, &path))
    {
        return Err("Choose a reference from another document".into());
    }
    if !path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("png") || e.eq_ignore_ascii_case("drawer"))
    {
        return Err("Reference files: local PNG or .drawer".into());
    }
    let source = crate::files::open(&path, BUDGET).map_err(|e| e.to_string())?;
    Ok((path, capture(&source)?))
}
pub fn add(d: &Document, name: String, scope: Scope, input: Add) -> Result<(Data, u64), String> {
    if !label(&name) {
        return Err("Use a printable reference name of1..256 bytes".into());
    }
    if let Scope::Cell(id) = scope
        && d.scene.cels.get(id).is_none()
    {
        return Err("Choose an existing editable cel for this scope".into());
    }
    let mut data = d.light_table.data.clone();
    let source = match input {
        Add::Layer(k) => {
            if !crate::layer_groups::live(d)?.contains_key(&k) {
                return Err("Reference layer is missing".into());
            }
            Source::Layer(k)
        }
        Add::File(path) => {
            let (path, image) = load(d, &path)?;
            Source::File {
                path,
                image: data.intern(image)?,
            }
        }
        Add::Copy {
            origin,
            title,
            image,
        } => {
            if origin == d.id || d.light_table.pin.is_some_and(|p| p.child == Some(origin)) {
                return Err("Copy a different canvas, not the editing cel itself".into());
            }
            Source::Copy {
                title,
                image: data.intern(image)?,
            }
        }
        Add::Onion { track, frame } => {
            let a = d.animation.as_ref().ok_or("Choose an animation")?;
            if frame >= a.frames
                || track
                    .is_some_and(|id| d.scene.catalog.tracks.index(id, a.tracks.len()).is_none())
            {
                return Err("Reference frame or track is missing".into());
            }
            Source::Onion { track, frame }
        }
    };
    let cell = match scope {
        Scope::Cell(id) => Some(id),
        _ => d.light_table.pin.map(|p| p.source),
    };
    if self_reference(d, &source, cell) {
        return Err("A cel cannot reference itself; choose another layer or frame".into());
    }
    let id = data.next;
    data.next = id.checked_add(1).ok_or("Reference ID limit")?;
    data.references.push(Reference {
        id,
        name,
        scope,
        source,
        pose: Pose::default(),
        mode: ColorMode::Original,
        color: [100, 140, 240],
    });
    if !data.valid(false) {
        return Err("Reference limits:128 references,32 images,64MiB".into());
    }
    Ok((data, id))
}
pub fn refresh(d: &Document, id: u64) -> Result<Data, String> {
    let mut data = d.light_table.data.clone();
    let r = data.get(id).ok_or("Reference removed")?;
    let Source::File { path, .. } = &r.source else {
        return Ok(data);
    };
    let (path, image) = load(d, path)?;
    let image = data.intern(image)?;
    data.get_mut(id).unwrap().source = Source::File { path, image };
    data.prune_images();
    Ok(data)
}
pub fn pin(d: &mut Document, source: u64, child: Option<u64>) -> Result<(), String> {
    if d.scene.cels.get(source).is_none() {
        return Err("The editing cel was removed; select another cel or release the pin".into());
    }
    for link in d.scene.cels.links.iter().filter(|l| l.source == source) {
        let (t, c, _) =
            crate::cels::slot(d, link.track, link.cell).ok_or("Editing cel slot missing")?;
        crate::cels::editable(
            d,
            crate::layer_properties::Target::Cell {
                track: t as u16,
                cell: c as u16,
            },
        )?;
    }
    d.light_table.pin = Some(Pin { source, child });
    Ok(())
}
pub fn pin_valid(d: &Document) -> bool {
    d.light_table.pin.is_none_or(|p| {
        if d.scene.cels.get(p.source).is_none() {
            return false;
        }
        d.scene
            .cels
            .links
            .iter()
            .filter(|l| l.source == p.source)
            .all(|l| {
                crate::cels::slot(d, l.track, l.cell).is_some_and(|(t, c, _)| {
                    crate::cels::editable(
                        d,
                        crate::layer_properties::Target::Cell {
                            track: t as u16,
                            cell: c as u16,
                        },
                    )
                    .is_ok()
                })
            })
    })
}
pub fn pivot(pose: Pose, point: Point) -> Pose {
    let pivot = pose.matrix().inverse().map(point);
    Pose {
        pivot,
        offset: [point[0] - pivot[0], point[1] - pivot[1]],
        ..pose
    }
}
pub fn drag(pose: Pose, start: Point, end: Point, rotate: bool) -> Pose {
    if rotate {
        let center = [
            pose.pivot[0] + pose.offset[0],
            pose.pivot[1] + pose.offset[1],
        ];
        let a = (start[1] - center[1]).atan2(start[0] - center[0]);
        let b = (end[1] - center[1]).atan2(end[0] - center[0]);
        Pose {
            degrees: (pose.degrees + (b - a).to_degrees()).clamp(-36000., 36000.),
            ..pose
        }
    } else {
        Pose {
            offset: [
                pose.offset[0] + end[0] - start[0],
                pose.offset[1] + end[1] - start[1],
            ],
            ..pose
        }
    }
}
pub struct Preview {
    pub image: LocalImage,
    pub notices: Vec<String>,
}
enum Render<'a> {
    Image(&'a Image),
    Layer(Key),
    Onion {
        document: Box<Document>,
        track: Option<u64>,
    },
}
pub fn preview(d: &Document, data: &Data, cell: Option<u64>, side: u32) -> Result<Preview, String> {
    if !data.valid(false) || side == 0 || side > 1024 {
        return Err("Invalid reference preview or size".into());
    }
    let scale = (d.width.max(d.height) as f64 / side as f64).max(1.);
    let width = (d.width as f64 / scale).ceil() as u32;
    let height = (d.height as f64 / scale).ceil() as u32;
    if data.opacity == 0 {
        return Ok(Preview {
            image: LocalImage {
                width,
                height,
                pixels: vec![[0; 4]; width as usize * height as usize],
            },
            notices: vec![],
        });
    }
    let snapshots = data
        .references
        .iter()
        .filter(|r| {
            matches!(r.source, Source::Onion { .. })
                && r.pose.opacity > 0.
                && (r.scope == Scope::Common || r.scope == cell.map_or(Scope::Common, Scope::Cell))
        })
        .count();
    if snapshots > 16 || d.memory_bytes().saturating_mul(snapshots) > BUDGET {
        return Err("Reference snapshot budget exceeded; reduce onion registrations".into());
    }
    let mut sources = vec![];
    let mut notices = vec![];
    for r in &data.references {
        if r.pose.opacity == 0. {
            continue;
        }
        if !matches!(r.scope, Scope::Common) && r.scope != cell.map_or(Scope::Common, Scope::Cell) {
            continue;
        }
        if self_reference(d, &r.source, cell) {
            notices.push(format!("{}: reference is the editing cel", r.name));
            continue;
        }
        let source = match &r.source {
            Source::File { path, image } => {
                if !path.is_file() {
                    notices.push(format!(
                        "{}: source file missing; showing saved copy",
                        r.name
                    ));
                }
                Render::Image(
                    data.images
                        .iter()
                        .find(|i| i.id == *image)
                        .ok_or("Reference image missing")?,
                )
            }
            Source::Copy { image, .. } => Render::Image(
                data.images
                    .iter()
                    .find(|i| i.id == *image)
                    .ok_or("Reference image missing")?,
            ),
            Source::Layer(key) => {
                if !crate::layer_groups::live(d)?.contains_key(key) {
                    notices.push(format!("{}: source layer removed", r.name));
                    continue;
                }
                Render::Layer(*key)
            }
            Source::Onion { track, frame } => {
                if d.animation.as_ref().is_none_or(|a| {
                    *frame >= a.frames
                        || track.is_some_and(|id| {
                            d.scene.catalog.tracks.index(id, a.tracks.len()).is_none()
                        })
                }) {
                    notices.push(format!("{}: source frame or track missing", r.name));
                    continue;
                }
                let copy = crate::layer_merge::preview(&d.snapshot(), *frame)?;
                Render::Onion {
                    document: Box::new(copy),
                    track: *track,
                }
            }
        };
        sources.push((r, source));
    }
    if (width as usize * height as usize).saturating_mul(sources.len()) > 16 * 1024 * 1024 {
        return Err("Reference sample budget exceeded; hide/remove references".into());
    }
    let base = crate::float_render::Renderer::new(d)?;
    let onion = sources
        .iter()
        .map(|(_, s)| {
            if let Render::Onion { document, .. } = s {
                crate::float_render::Renderer::new(document).map(Some)
            } else {
                Ok(None)
            }
        })
        .collect::<Result<Vec<_>, String>>()?;
    let mut pixels = vec![[0; 4]; width as usize * height as usize];
    for ((r, source), renderer) in sources.iter().zip(&onion) {
        let inverse = r.pose.matrix().inverse();
        let tint = crate::timeline_view::Onion {
            opacity: 100,
            mode: r.mode,
            colors: [r.color; 2],
            ..Default::default()
        };
        for y in 0..height {
            for x in 0..width {
                let p = inverse.map([
                    (x as f64 + 0.5) * d.width as f64 / width as f64,
                    (y as f64 + 0.5) * d.height as f64 / height as f64,
                ]);
                let raw = match source {
                    Render::Image(i) => {
                        let px = p[0].floor() as i64;
                        let py = p[1].floor() as i64;
                        if px < 0 || py < 0 || px >= i.width as i64 || py >= i.height as i64 {
                            [0; 4]
                        } else {
                            i.pixels[py as usize * i.width as usize + px as usize]
                        }
                    }
                    Render::Layer(key) => base.display_source(*key, p)?.rgba8(),
                    Render::Onion { track, .. } => {
                        if let Some(track) = track {
                            renderer
                                .as_ref()
                                .unwrap()
                                .display_source(Key::Track(*track), p)?
                                .rgba8()
                        } else {
                            renderer.as_ref().unwrap().display_sample(p)?.rgba8()
                        }
                    }
                };
                let rgba = tint.tint(raw, 0, 1);
                let sample = crate::color_precision::Sample::from8(rgba)
                    .coverage(r.pose.opacity * data.opacity as f64 / 100.)?;
                let n = y as usize * width as usize + x as usize;
                pixels[n] = crate::color_precision::Sample::from8(pixels[n])
                    .over(sample, crate::layer_blend::Mode::Normal, 1.)?
                    .rgba8();
            }
        }
    }
    Ok(Preview {
        image: LocalImage {
            width,
            height,
            pixels,
        },
        notices,
    })
}
fn invalid(e: impl ToString) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, e.to_string())
}
pub(crate) fn write(d: &Document, w: &mut impl Write) -> io::Result<()> {
    for i in &d.light_table.data.images {
        w.write_all(i.pixels.as_slice().as_flattened())?;
    }
    Ok(())
}
pub(crate) fn read(d: &mut Document, r: &mut impl Read, budget: usize) -> io::Result<()> {
    let bytes = d.light_table.data.image_bytes();
    if bytes
        > d.memory_bytes()
            .checked_sub(d.light_table.bytes())
            .map_or(0, |used| budget.saturating_sub(used))
    {
        return Err(invalid("Light-table image payload budget"));
    }
    for i in &mut d.light_table.data.images {
        let count = i.width as usize * i.height as usize;
        let mut pixels = vec![];
        pixels.try_reserve_exact(count).map_err(invalid)?;
        pixels.resize(count, [0; 4]);
        r.read_exact(pixels.as_mut_slice().as_flattened_mut())?;
        if Sha256::digest(pixels.as_slice().as_flattened()).as_slice() != i.hash {
            return Err(invalid("Corrupt reference image payload"));
        }
        i.pixels = Arc::new(pixels);
    }
    if !d.light_table.data.valid(false) {
        return Err(invalid("Invalid light-table references"));
    }
    Ok(())
}
