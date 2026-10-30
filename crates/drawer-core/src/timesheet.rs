//! Drawer-owned, versioned cel interchange; immutable PNG sources are shared across exposures.
use crate::{
    animation::{Cell, Exposure, Fps, Timeline, Track},
    document::{Background, CanvasSpec, Document},
    layer_properties::{self as props, Properties, Target},
    page_batch::Stamp,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Write,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU32, Ordering},
    },
};
pub const LIMIT: usize = 4 * 1024 * 1024;
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Naming {
    #[default]
    Cel,
    Serial,
    FolderCel,
    FolderSerial,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Options {
    pub naming: Naming,
    pub drafts: bool,
    pub common: bool,
    pub overflow: bool,
    pub unassigned: bool,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            naming: Default::default(),
            drafts: false,
            common: false,
            overflow: false,
            unassigned: true,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Bitmap {
    pub id: u64,
    pub name: String,
    pub path: String,
    pub width: u32,
    pub height: u32,
    pub origin: [i64; 2],
    pub hash: [u8; 32],
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Slot {
    pub id: u64,
    pub order: usize,
    pub name: String,
    pub source: u64,
    pub offset: [i64; 2],
    pub properties: Properties,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Key {
    pub frame: u32,
    pub cell: u64,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Lane {
    pub id: u64,
    pub order: usize,
    pub name: String,
    pub end: u32,
    pub properties: Properties,
    pub cells: Vec<Slot>,
    pub exposures: Vec<Key>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Sheet {
    pub format: String,
    pub version: u32,
    pub width: u32,
    pub height: u32,
    pub dpi: f64,
    pub fps: Fps,
    pub frames: u32,
    pub captured_frame: u32,
    pub revision: u64,
    pub options: Options,
    pub images: Vec<Bitmap>,
    pub common: Option<Bitmap>,
    pub tracks: Vec<Lane>,
}
fn name(s: &str) -> bool {
    !s.trim().is_empty() && s.len() <= 4096 && !s.chars().any(char::is_control)
}
fn plain_png(s: &str) -> bool {
    let p = Path::new(s);
    p.components().count() == 1
        && p.components()
            .all(|c| matches!(c, std::path::Component::Normal(_)))
        && s.len() <= 220
        && !s.contains(['\\', ':', '\0'])
        && p.extension().is_some_and(|e| e.eq_ignore_ascii_case("png"))
}
impl Sheet {
    pub fn normalize(&mut self) {
        self.tracks.sort_by_key(|t| t.order);
        self.images.sort_by_key(|i| i.id);
        for t in &mut self.tracks {
            t.cells.sort_by_key(|c| c.order);
            t.exposures.sort_by_key(|e| e.frame);
        }
    }
    pub fn valid(&self) -> bool {
        let image = |i: &Bitmap| {
            name(&i.name)
                && plain_png(&i.path)
                && (1..=32768).contains(&i.width)
                && (1..=32768).contains(&i.height)
                && i.origin.iter().all(|n| n.unsigned_abs() <= 1_000_000_000)
        };
        let paths = self
            .images
            .iter()
            .chain(self.common.iter())
            .map(|i| i.path.to_lowercase())
            .collect::<BTreeSet<_>>();
        self.format == "Drawer timesheet"
            && self.version == 1
            && (1..=32768).contains(&self.width)
            && (1..=32768).contains(&self.height)
            && (1. ..=4800.).contains(&self.dpi)
            && self.fps.valid()
            && (1..=10000).contains(&self.frames)
            && self.captured_frame < self.frames
            && self.images.len() <= 512
            && self.images.iter().all(|i| i.id > 0 && image(i))
            && self
                .images
                .iter()
                .map(|i| i.id)
                .collect::<BTreeSet<_>>()
                .len()
                == self.images.len()
            && self.options.common == self.common.is_some()
            && self.common.as_ref().is_none_or(|i| i.id == 0 && image(i))
            && paths.len() == self.images.len() + usize::from(self.common.is_some())
            && !self.tracks.is_empty()
            && self.tracks.len() <= 32
            && self
                .tracks
                .iter()
                .map(|t| t.id)
                .collect::<BTreeSet<_>>()
                .len()
                == self.tracks.len()
            && self.tracks.iter().map(|t| t.cells.len()).sum::<usize>() <= 10000
            && self.tracks.iter().enumerate().all(|(n, t)| {
                t.id > 0
                    && t.order == n
                    && name(&t.name)
                    && t.end <= self.frames
                    && t.properties.opacity <= 100
                    && t.cells.len() <= 10000
                    && t.cells.iter().map(|c| c.id).collect::<BTreeSet<_>>().len() == t.cells.len()
                    && t.cells.iter().enumerate().all(|(n, c)| {
                        c.id > 0
                            && c.order == n
                            && name(&c.name)
                            && c.properties.opacity <= 100
                            && self.images.iter().any(|i| i.id == c.source)
                            && c.offset.iter().all(|n| n.unsigned_abs() <= 1_000_000_000)
                    })
                    && t.exposures.len() <= 10000
                    && t.exposures.windows(2).all(|p| p[0].frame < p[1].frame)
                    && t.exposures
                        .iter()
                        .all(|e| e.frame < t.end && t.cells.iter().any(|c| c.id == e.cell))
            })
    }
    pub fn json(&self) -> Result<Vec<u8>, String> {
        if !self.valid() {
            return Err("Invalid Drawer timesheet structure".into());
        }
        let b = serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?;
        if b.len() > LIMIT {
            return Err("Timesheet exceeds4MiB".into());
        }
        Ok(b)
    }
    pub fn decode(bytes: &[u8], csv: bool) -> Result<Self, String> {
        if bytes.len() > LIMIT {
            return Err("Timesheet exceeds4MiB".into());
        }
        let mut s: Self = if csv {
            crate::timesheet_csv::decode(std::str::from_utf8(bytes).map_err(|e| e.to_string())?)?
        } else {
            serde_json::from_slice(bytes).map_err(|_|"Only Drawer timesheet v1 JSON / CSV is supported; no verified proprietary adapter")?
        };
        s.normalize();
        if !s.valid() {
            return Err("Invalid or unsupported Drawer timesheet version, IDs, FPS, frame range or local filename".into());
        }
        Ok(s)
    }
}
pub struct Snapshot {
    document: Box<Document>,
    pub options: Options,
    pub revision: u64,
    budget: usize,
}
pub fn capture(d: &Document, o: Options, budget: usize) -> Result<Snapshot, String> {
    let a = d
        .animation
        .as_ref()
        .filter(|a| a.valid() && !a.tracks.is_empty())
        .ok_or("Choose an animation with tracks")?;
    if !d.scene.valid()
        || !crate::cels::valid_document(d)
        || d.memory_bytes().saturating_mul(6) > budget
        || a.tracks.iter().map(|t| t.cells.len()).sum::<usize>() > 10000
    {
        return Err("Cel snapshot exceeds structure / memory limits".into());
    }
    Ok(Snapshot {
        document: Box::new(crate::layer_merge::preview(&d.snapshot(), d.current_frame)?),
        options: o,
        revision: d.revision,
        budget,
    })
}
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Source {
    Global(u64),
    Legacy(usize, usize),
}
struct Work {
    source: Source,
    name: String,
    folder: String,
}
fn safe(s: &str) -> String {
    use unicode_normalization::UnicodeNormalization;
    let mut s = s
        .nfc()
        .map(|c| {
            if c.is_control() || "<>:\"/\\|?*".contains(c) {
                '_'
            } else {
                c
            }
        })
        .take(48)
        .collect::<String>()
        .trim()
        .trim_end_matches('.')
        .to_owned();
    if s.is_empty() {
        s = "cel".into();
    }
    let upper = s.split('.').next().unwrap_or("").to_uppercase();
    if [
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
        "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ]
    .contains(&upper.as_str())
    {
        s.insert(0, '_');
    }
    s
}
fn filename(w: &Work, o: Options, id: u64, used: &mut BTreeSet<String>) -> String {
    let serial = format!("{id:06}");
    let mut stem = match o.naming {
        Naming::Cel => safe(&w.name),
        Naming::Serial => serial,
        Naming::FolderCel => format!("{}-{}", safe(&w.folder), safe(&w.name)),
        Naming::FolderSerial => format!("{}-{serial}", safe(&w.folder)),
    };
    while stem.len() > 180 {
        stem.pop();
    }
    let mut p = format!("{stem}.png");
    let mut attempt = 0;
    while used.contains(&p.to_lowercase()) {
        p = format!("{stem}-source-{id}-{attempt}.png");
        attempt += 1;
    }
    used.insert(p.to_lowercase());
    p
}
fn rendered(
    mut d: Document,
    o: Options,
    budget: usize,
) -> Result<crate::animation_export::Snapshot, String> {
    if d.animation.is_none() {
        d.commit_state(crate::timeline_editor::create(&d, 1, Default::default())?)?;
    }
    crate::animation_export::prepare(
        &d,
        crate::animation_export::Options {
            first: d.current_frame,
            last: d.current_frame,
            view: crate::animation_render::View {
                area: if o.overflow {
                    crate::animation_render::Area::Overflow
                } else {
                    crate::animation_render::Area::Canvas
                },
                camera: false,
                preserve_paper: true,
                drafts: o.drafts,
                icc: false,
                ..Default::default()
            },
            ..Default::default()
        },
        budget,
    )
}
fn image(
    snapshot: &crate::animation_export::Snapshot,
    path: &Path,
    id: u64,
    name: String,
    cancel: &AtomicBool,
) -> Result<Bitmap, String> {
    let pixels = crate::animation_render::frame(snapshot, snapshot.options.first, cancel)?;
    crate::animation_export::write_png(snapshot, path, &pixels)?;
    let bytes = crate::book_io::read_local(path, 64 * 1024 * 1024)?;
    Ok(Bitmap {
        id,
        name,
        path: path.file_name().unwrap().to_string_lossy().into_owned(),
        width: snapshot.layout.width,
        height: snapshot.layout.height,
        origin: snapshot.layout.origin,
        hash: Sha256::digest(bytes).into(),
    })
}
/// A new bundle directory is published only after all PNGs and both sheets validate.
pub fn export(
    s: &Snapshot,
    destination: &Path,
    cancel: &AtomicBool,
    progress: &AtomicU32,
    publish: &mut impl FnMut(&Path, &Path) -> Result<(), String>,
) -> Result<Sheet, String> {
    export_with(s, destination, cancel, progress, publish, |_| {})
}
fn export_with(
    s: &Snapshot,
    destination: &Path,
    cancel: &AtomicBool,
    progress: &AtomicU32,
    publish: &mut impl FnMut(&Path, &Path) -> Result<(), String>,
    mut after: impl FnMut(u32),
) -> Result<Sheet, String> {
    crate::book_io::check_cancel(cancel)?;
    let parent = destination
        .parent()
        .ok_or("Choose a new bundle directory")?
        .canonicalize()
        .map_err(|e| e.to_string())?;
    if !crate::font_library::local(&parent) {
        return Err("Choose a local bundle directory".into());
    }
    let destination = parent.join(destination.file_name().ok_or("Choose a bundle name")?);
    if std::fs::symlink_metadata(&destination).is_ok() {
        return Err("Choose a new bundle folder; existing folders are preserved".into());
    }
    let d = &s.document;
    let a = d.animation.as_ref().unwrap();
    let mut sheet = Sheet {
        format: "Drawer timesheet".into(),
        version: 1,
        width: d.width,
        height: d.height,
        dpi: d.dpi,
        fps: a.fps,
        frames: a.frames,
        captured_frame: d.current_frame,
        revision: s.revision,
        options: s.options,
        images: vec![],
        common: None,
        tracks: vec![],
    };
    let mut jobs: Vec<Work> = vec![];
    let mut ids = BTreeMap::new();
    for (t, lane) in a.tracks.iter().enumerate() {
        let track = d.scene.catalog.tracks.id(t);
        let mut cells = vec![];
        let mut slots = BTreeMap::new();
        for (c, cell) in lane.cells.iter().enumerate() {
            if !s.options.unassigned && !lane.exposures.iter().any(|e| e.cell == c) {
                continue;
            }
            let source = d
                .scene
                .cels
                .source(track, d.scene.catalog.cell_id(track, c))
                .map_or(Source::Legacy(t, c), Source::Global);
            let id = *ids.entry(source).or_insert_with(|| {
                jobs.push(Work {
                    source,
                    name: match source {
                        Source::Global(id) => d.scene.cels.get(id).unwrap().name.clone(),
                        _ => cell.name.clone(),
                    },
                    folder: lane.name.clone(),
                });
                jobs.len() as u64
            });
            let id_slot = c as u64 + 1;
            slots.insert(c, id_slot);
            cells.push(Slot {
                id: id_slot,
                order: cells.len(),
                name: cell.name.clone(),
                source: id,
                offset: cell.offset,
                properties: props::properties(
                    d,
                    Target::Cell {
                        track: t as u16,
                        cell: c as u16,
                    },
                ),
            });
        }
        sheet.tracks.push(Lane {
            id: track,
            order: t,
            name: lane.name.clone(),
            end: lane.end,
            properties: props::inherited(d, Target::Track(t as u16)),
            cells,
            exposures: lane
                .exposures
                .iter()
                .map(|e| Key {
                    frame: e.frame,
                    cell: slots[&e.cell],
                })
                .collect(),
        });
    }
    if s.options.unassigned {
        for p in &d.scene.cels.definitions {
            let src = Source::Global(p.object);
            if let std::collections::btree_map::Entry::Vacant(e) = ids.entry(src) {
                jobs.push(Work {
                    source: src,
                    name: p.name.clone(),
                    folder: "Unassigned".into(),
                });
                e.insert(jobs.len() as u64);
            }
        }
    }
    if jobs.len() > 512 {
        return Err("Exchange supports up to512 distinct editable cel sources".into());
    }
    let mut stage = crate::animation_export::Stage::new(&parent)?;
    let mut used = BTreeSet::from(["common-background.png".into()]);
    let mut bytes = 0u64;
    for (n, job) in jobs.iter().enumerate() {
        crate::book_io::check_cancel(cancel)?;
        let child = match job.source {
            Source::Global(id) => d.scene.cels.get(id).unwrap().open()?,
            Source::Legacy(t, c) => {
                crate::cels::raster_document(d, &a.tracks[t].cells[c], s.budget)?
            }
        };
        let capture = rendered(child, s.options, s.budget)?;
        let path = stage
            .directory
            .join(filename(job, s.options, n as u64 + 1, &mut used));
        stage.files.push(path.clone());
        sheet.images.push(image(
            &capture,
            &path,
            n as u64 + 1,
            job.name.clone(),
            cancel,
        )?);
        bytes = bytes.saturating_add(std::fs::metadata(&path).map_err(|e| e.to_string())?.len());
        if bytes > 2 * 1024 * 1024 * 1024 {
            return Err("Cel bundle exceeds2GiB".into());
        }
        progress.store(n as u32 + 1, Ordering::Relaxed);
        after(n as u32 + 1);
    }
    if s.options.common {
        let mut state = d.snapshot();
        for t in 0..a.tracks.len() {
            let target = Target::Track(t as u16);
            let p = Properties {
                hidden: true,
                ..props::properties(d, target)
            };
            props::set_state(d, &mut state, target, p)?;
        }
        let common = crate::layer_merge::preview(&state, d.current_frame)?;
        let capture = rendered(common, s.options, s.budget)?;
        let path = stage.directory.join("common-background.png");
        stage.files.push(path.clone());
        sheet.common = Some(image(
            &capture,
            &path,
            0,
            "Common background".into(),
            cancel,
        )?);
        bytes = bytes.saturating_add(std::fs::metadata(&path).map_err(|e| e.to_string())?.len());
        if bytes > 2 * 1024 * 1024 * 1024 {
            return Err("Cel bundle including common background exceeds2GiB".into());
        }
    }
    sheet.normalize();
    let json = sheet.json()?;
    let csv = crate::timesheet_csv::encode(&sheet)?;
    if Sheet::decode(&json, false)? != Sheet::decode(&csv, true)? {
        return Err("JSON/CSV roundtrip verification failed".into());
    }
    for (name, bytes) in [("timesheet.json", json), ("timesheet.csv", csv)] {
        let mut f = stage.create(name)?;
        f.write_all(&bytes)
            .and_then(|_| f.sync_all())
            .map_err(|e| e.to_string())?;
    }
    crate::book_io::check_cancel(cancel)?;
    publish(&stage.directory, &destination)?;
    Ok(sheet)
}
pub struct Prepared {
    pub document: Document,
    pub sources: Vec<Stamp>,
    pub sheet: Sheet,
}
impl Prepared {
    pub fn fresh(&self) -> Result<(), String> {
        crate::page_batch::Prepared {
            pages: vec![],
            sources: self.sources.clone(),
            rejected: vec![],
        }
        .fresh()
    }
}
fn decode_image(
    root: &Path,
    i: &Bitmap,
    budget: usize,
    cancel: &AtomicBool,
) -> Result<(Document, Stamp), String> {
    crate::book_io::check_cancel(cancel)?;
    let p = root.join(&i.path);
    let m = std::fs::symlink_metadata(&p).map_err(|e| format!("Missing cel {}: {e}", i.path))?;
    if !m.is_file() || m.file_type().is_symlink() {
        return Err("Cel must be a regular local PNG".into());
    }
    let bytes = crate::book_io::read_local(&p, (64 * 1024 * 1024).min(budget / 4))?;
    let hash = Sha256::digest(&bytes).into();
    if hash != i.hash {
        return Err(format!("Cel changed or is corrupt: {}", i.path));
    }
    let png = png::Decoder::new(std::io::Cursor::new(&bytes))
        .read_info()
        .map_err(|e| e.to_string())?;
    if png.info().animation_control.is_some() {
        return Err("Exchange requires static PNG cells, not APNG".into());
    }
    drop(png);
    let source =
        crate::image_import::ImageSource::from_encoded(p.clone(), bytes, budget / 2, cancel)
            .map_err(|e| e.to_string())?;
    if source.pages.len() != 1 {
        return Err("Exchange cel must be one static PNG".into());
    }
    let image = source
        .decode(0, true, budget / 2, cancel)
        .map_err(|e| e.to_string())?
        .image;
    if image.width != i.width || image.height != i.height {
        return Err("Cel dimensions do not match the timesheet".into());
    }
    Ok((
        image.document(budget / 2).map_err(|e| e.to_string())?,
        Stamp { path: p, hash },
    ))
}
pub fn prepare(path: &Path, budget: usize, cancel: &AtomicBool) -> Result<Prepared, String> {
    crate::book_io::check_cancel(cancel)?;
    if !crate::font_library::local(path)
        || !path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("json") || e.eq_ignore_ascii_case("csv"))
    {
        return Err("Only local Drawer timesheet v1 JSON / CSV is supported; no verified proprietary adapter".into());
    }
    let path = path.canonicalize().map_err(|e| e.to_string())?;
    if !crate::font_library::local(&path) {
        return Err("Choose a local timesheet".into());
    }
    let bytes = crate::book_io::read_local(&path, LIMIT.min(budget / 4))?;
    let sheet = Sheet::decode(
        &bytes,
        path.extension().unwrap().eq_ignore_ascii_case("csv"),
    )?;
    let mut d = Document::create(
        CanvasSpec {
            width: sheet.width,
            height: sheet.height,
            dpi: sheet.dpi,
            background: Background::Transparent,
        },
        budget / 8,
    )
    .map_err(|e| e.to_string())?;
    let mut sources = vec![Stamp {
        path: path.clone(),
        hash: Sha256::digest(&bytes).into(),
    }];
    let root = path.parent().unwrap();
    let mut state = d.snapshot();
    let mut loaded = BTreeMap::new();
    let mut used = d.memory_bytes() * 4 + bytes.len();
    for i in &sheet.images {
        let (child, stamp) = decode_image(root, i, budget.saturating_sub(used), cancel)?;
        used = used.saturating_add(child.memory_bytes() * 6);
        if used > budget {
            return Err("Cel import exceeds the available memory budget".into());
        }
        let source = crate::cels::insert(
            &mut state,
            &child,
            &i.name.chars().take(64).collect::<String>(),
            budget.saturating_sub(used / 2),
        )?;
        let pixels = Arc::new(child.layers[0].pixels.clone());
        loaded.insert(i.id, (source, pixels, i));
        sources.push(stamp);
    }
    let mut timeline = Timeline {
        fps: sheet.fps,
        frames: sheet.frames,
        tracks: vec![],
        audio: vec![],
    };
    state.scene.catalog.tracks.initialize(sheet.tracks.len())?;
    for (t, lane) in sheet.tracks.iter().enumerate() {
        let track = state.scene.catalog.tracks.id(t);
        let mut cells = vec![];
        let mut slots = BTreeMap::new();
        let mut ids = crate::layer_catalog::Ids::default();
        ids.initialize(lane.cells.len())?;
        for (c, cell) in lane.cells.iter().enumerate() {
            let (source, pixels, i) = &loaded[&cell.source];
            let offset = [0, 1].map(|n| cell.offset[n].saturating_add(i.origin[n]));
            if offset.iter().any(|n| n.unsigned_abs() > 1_000_000_000) {
                return Err("Imported cel offset exceeds document limits".into());
            }
            cells.push(Cell {
                name: cell.name.clone(),
                width: i.width,
                height: i.height,
                offset,
                pixels: pixels.clone(),
            });
            slots.insert(cell.id, c);
            state.scene.cels.links.push(crate::cels::Link {
                track,
                cell: ids.id(c),
                source: *source,
            });
            state.scene.layer_properties.set(
                Target::Cell {
                    track: t as u16,
                    cell: c as u16,
                },
                cell.properties,
            );
        }
        state.scene.catalog.cells.insert(track, ids);
        state
            .scene
            .layer_properties
            .tracks
            .insert(t as u16, lane.properties);
        timeline.tracks.push(Track {
            name: lane.name.clone(),
            cells,
            exposures: lane
                .exposures
                .iter()
                .map(|e| Exposure {
                    frame: e.frame,
                    cell: slots[&e.cell],
                })
                .collect(),
            end: lane.end,
        });
    }
    if let Some(i) = &sheet.common {
        let (child, stamp) = decode_image(root, i, budget.saturating_sub(used), cancel)?;
        used += child.memory_bytes() * 4;
        if used > budget {
            return Err("Common background import exceeds memory budget".into());
        }
        state.layers[0] = child.layers[0].clone();
        state.layers[0].name = "Common background".into();
        state.layers[0].placement = Some(crate::document::PlacedRaster {
            width: i.width,
            height: i.height,
            offset: i.origin,
        });
        sources.push(stamp);
    }
    state.animation = Some(timeline);
    crate::book_io::check_cancel(cancel)?;
    d.commit_state(state)?;
    d.title = path
        .parent()
        .and_then(Path::file_name)
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    d.current_frame = sheet.captured_frame;
    if !crate::cels::valid_document(&d) || d.resident_bytes() > budget {
        return Err("Imported cel references exceed native / memory limits".into());
    }
    Ok(Prepared {
        document: d,
        sources,
        sheet,
    })
}
pub fn encode_options(o: Options) -> Result<Vec<u8>, String> {
    serde_json::to_vec_pretty(
        &serde_json::json!({"format":"Drawer timesheet preset","version":1,"options":o}),
    )
    .map_err(|e| e.to_string())
}
pub fn decode_options(b: &[u8]) -> Result<Options, String> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Preset {
        format: String,
        version: u32,
        options: Options,
    }
    if b.len() > 65536 {
        return Err("Timesheet preset exceeds64KiB".into());
    }
    let p: Preset = serde_json::from_slice(b).map_err(|e| e.to_string())?;
    if p.format != "Drawer timesheet preset" || p.version != 1 {
        return Err("Unsupported timesheet preset".into());
    }
    Ok(p.options)
}
pub fn save_options(path: &Path, o: Options) -> Result<(), String> {
    if !crate::font_library::local(path)
        || !path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("json"))
    {
        return Err("Choose a local JSON preset".into());
    }
    let parent = path
        .parent()
        .ok_or("Choose preset folder")?
        .canonicalize()
        .map_err(|e| e.to_string())?;
    let old = crate::animation_export::existing(path)?;
    let mut stage = crate::animation_export::Stage::new(&parent)?;
    let mut f = stage.create("preset.json")?;
    f.write_all(&encode_options(o)?)
        .and_then(|_| f.sync_all())
        .map_err(|e| e.to_string())?;
    drop(f);
    if crate::animation_export::existing(path)? != old {
        return Err("Preset changed while saving".into());
    }
    std::fs::rename(stage.directory.join("preset.json"), path).map_err(|e| e.to_string())
}

#[cfg(test)]
#[path = "timesheet_tests.rs"]
mod tests;
