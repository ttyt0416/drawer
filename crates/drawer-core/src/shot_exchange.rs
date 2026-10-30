//! Drawer shots v1. Typed metadata, exact rational FPS and local raster cells.
use crate::{
    animation::{Cell, Exposure, Fps, Timeline, Track},
    book_io,
    document::{Background, CanvasSpec, Document},
    page_batch::Stamp,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::{Arc, atomic::AtomicBool},
};
const LIMIT: usize = 4 * 1024 * 1024;
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Image {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    pub frame: u32,
    pub path: PathBuf,
    pub offset: [i64; 2],
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Shot {
    pub id: u64,
    pub name: String,
    pub start: u32,
    pub end: u32,
    pub fps: Fps,
    pub dialogue: String,
    pub cells: Vec<Image>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct File {
    pub format: String,
    pub version: u32,
    pub width: u32,
    pub height: u32,
    pub dpi: f64,
    pub shots: Vec<Shot>,
}
impl Shot {
    fn valid(&self) -> bool {
        self.id > 0
            && !self.name.trim().is_empty()
            && self.name.chars().count() <= 256
            && !self.name.chars().any(char::is_control)
            && self.start < self.end
            && self.end <= crate::animation::MAX_FRAMES
            && self.fps.valid()
            && self.dialogue.len() <= 32768
            && !self
                .dialogue
                .chars()
                .any(|c| c.is_control() && !['\n', '\r', '\t'].contains(&c))
            && !self.cells.is_empty()
            && self.cells.len() <= 10000
            && self.cells[0].frame == self.start
            && self.cells.windows(2).all(|a| a[0].frame < a[1].frame)
            && self.cells.iter().all(|c| {
                c.frame >= self.start
                    && c.frame < self.end
                    && c.name.len() <= 4096
                    && !c.name.contains('\0')
                    && c.path.as_os_str().len() <= 4096
                    && !c.path.as_os_str().is_empty()
                    && !c.path.to_string_lossy().contains('\0')
                    && c.offset.iter().all(|v| v.unsigned_abs() <= 1_000_000_000)
            })
    }
}
impl File {
    pub fn valid(&self) -> bool {
        self.format == "Drawer shots"
            && self.version == 1
            && (1..=32768).contains(&self.width)
            && (1..=32768).contains(&self.height)
            && (1. ..=4800.).contains(&self.dpi)
            && !self.shots.is_empty()
            && self.shots.len() <= 32
            && self.shots.iter().all(Shot::valid)
            && self
                .shots
                .iter()
                .map(|s| s.id)
                .collect::<BTreeSet<_>>()
                .len()
                == self.shots.len()
            && self.shots.iter().all(|s| s.fps == self.shots[0].fps)
            && self.shots.iter().map(|s| s.cells.len()).sum::<usize>() <= 10000
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Bound {
    pub shot: Shot,
    pub track: u64,
    pub cells: Vec<u64>,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Data {
    pub entries: Vec<Bound>,
}
impl Data {
    pub fn is_default(&self) -> bool {
        self.entries.is_empty()
    }
    pub fn valid(&self) -> bool {
        self.entries.len() <= 32
            && self.entries.iter().all(|b| {
                b.shot.valid()
                    && b.track > 0
                    && b.cells.len() == b.shot.cells.len()
                    && b.cells.iter().all(|id| *id > 0)
            })
            && self
                .entries
                .iter()
                .map(|b| b.shot.id)
                .collect::<BTreeSet<_>>()
                .len()
                == self.entries.len()
    }
    pub fn bytes(&self) -> usize {
        if self.is_default() {
            0
        } else {
            self.entries
                .iter()
                .map(|b| {
                    256 + b.shot.name.len()
                        + b.shot.dialogue.len()
                        + b.shot
                            .cells
                            .iter()
                            .map(|c| 64 + c.name.len() + c.path.as_os_str().len() * 2)
                            .sum::<usize>()
                        + b.cells.len() * 8
                })
                .sum()
        }
    }
}
pub fn valid_document(d: &Document) -> bool {
    d.scene.shots.valid()
        && d.scene.shots.entries.iter().all(|b| {
            d.animation.as_ref().is_some_and(|a| {
                d.scene
                    .catalog
                    .tracks
                    .index(b.track, a.tracks.len())
                    .is_some_and(|n| {
                        let ids = d
                            .scene
                            .catalog
                            .cells
                            .get(&b.track)
                            .cloned()
                            .unwrap_or_default();
                        b.cells
                            .iter()
                            .all(|id| ids.index(*id, a.tracks[n].cells.len()).is_some())
                    })
            })
        })
}
pub(crate) fn prune(state: &mut crate::sizing::DocumentState) {
    let Some(a) = &state.animation else {
        state.scene.shots.entries.clear();
        return;
    };
    state.scene.shots.entries.retain(|b| {
        state
            .scene
            .catalog
            .tracks
            .index(b.track, a.tracks.len())
            .is_some_and(|n| {
                let ids = state
                    .scene
                    .catalog
                    .cells
                    .get(&b.track)
                    .cloned()
                    .unwrap_or_default();
                b.cells
                    .iter()
                    .all(|id| ids.index(*id, a.tracks[n].cells.len()).is_some())
            })
    });
}
fn local(base: &Path, path: &Path) -> Result<PathBuf, String> {
    if path.is_absolute() {
        if !crate::font_library::local(path) {
            return Err("Shot cells must be local drive files".into());
        }
    } else if path.components().any(|c| {
        !matches!(
            c,
            std::path::Component::Normal(_) | std::path::Component::CurDir
        )
    }) {
        return Err("Shot cell path escapes the exchange folder".into());
    }
    let p = base.join(path);
    if !crate::font_library::local(&p) {
        return Err("Only local shot cell paths are supported".into());
    }
    let p = p.canonicalize().map_err(|e| e.to_string())?;
    if !crate::font_library::local(&p) {
        return Err("Shot cell resolved outside local drive".into());
    }
    Ok(p)
}
pub struct Prepared {
    pub document: Document,
    pub sources: Vec<Stamp>,
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
pub fn prepare(path: &Path, budget: usize, cancel: &AtomicBool) -> Result<Prepared, String> {
    if !path
        .extension()
        .is_some_and(|x| x.eq_ignore_ascii_case("drawer-shots") || x.eq_ignore_ascii_case("csv"))
    {
        return Err(
            "Only Drawer shots v1 JSON/CSV is supported; no verified external proprietary adapter"
                .into(),
        );
    }
    if path.is_absolute() && !crate::font_library::local(path) {
        return Err("Choose a local shot file".into());
    }
    let path = path.canonicalize().map_err(|e| e.to_string())?;
    let bytes = book_io::read_local(&path, LIMIT.min(budget / 2))?;
    let file = if path
        .extension()
        .is_some_and(|x| x.eq_ignore_ascii_case("csv"))
    {
        from_csv(std::str::from_utf8(&bytes).map_err(|e| e.to_string())?)?
    } else {
        serde_json::from_slice::<File>(&bytes).map_err(|e| e.to_string())?
    };
    if !file.valid() {
        return Err("Invalid shot IDs/ranges/cells or FPS. One imported document requires the same exact rational FPS for all shots".into());
    }
    let mut document = Document::create(
        CanvasSpec {
            width: file.width,
            height: file.height,
            dpi: file.dpi,
            background: Background::Transparent,
        },
        budget / 4,
    )
    .map_err(|e| e.to_string())?;
    let mut state = document.snapshot();
    let mut timeline = Timeline {
        fps: file.shots[0].fps,
        frames: file.shots.iter().map(|s| s.end).max().unwrap(),
        tracks: vec![],
        audio: vec![],
    };
    let mut sources = vec![Stamp {
        path: path.clone(),
        hash: Sha256::digest(&bytes).into(),
    }];
    let mut used = document.resident_bytes() * 2 + bytes.len();
    for mut shot in file.shots {
        book_io::check_cancel(cancel)?;
        let mut cells = vec![];
        let mut exposures = vec![];
        let mut seen = BTreeMap::new();
        let mut bindings = vec![];
        for source in &mut shot.cells {
            let p = local(path.parent().unwrap(), &source.path)?;
            source.path = p.clone();
            if source.name.is_empty() {
                source.name = p.file_name().unwrap().to_string_lossy().into_owned();
            }
            let key = (p.clone(), source.offset, source.name.clone());
            let cell = if let Some(n) = seen.get(&key) {
                *n
            } else {
                let data = book_io::read_local(
                    &p,
                    (64 * 1024 * 1024).min(budget.saturating_sub(used) / 3),
                )?;
                let hash = Sha256::digest(&data).into();
                let input = crate::image_import::ImageSource::from_encoded(
                    p.clone(),
                    data,
                    budget.saturating_sub(used) / 2,
                    cancel,
                )
                .map_err(|e| e.to_string())?;
                if input.pages.len() != 1 {
                    return Err("Shot cell must contain one raster image".into());
                }
                let image = input
                    .decode(0, true, budget.saturating_sub(used) / 2, cancel)
                    .map_err(|e| e.to_string())?
                    .image;
                used = used.saturating_add(image.memory_bytes() * 2);
                if used > budget {
                    return Err("Shot cell memory budget exceeded".into());
                }
                let n = cells.len();
                cells.push(Cell {
                    name: if source.name.is_empty() {
                        p.file_name().unwrap().to_string_lossy().into_owned()
                    } else {
                        source.name.clone()
                    },
                    width: image.width,
                    height: image.height,
                    offset: source.offset,
                    pixels: Arc::new(image.pixels),
                });
                seen.insert(key, n);
                sources.push(Stamp { path: p, hash });
                n
            };
            exposures.push(Exposure {
                frame: source.frame,
                cell,
            });
            bindings.push(cell as u64 + 1);
        }
        let id = timeline.tracks.len() as u64 + 1;
        let mut ids = crate::layer_catalog::Ids::default();
        ids.initialize(cells.len())?;
        state.scene.catalog.cells.insert(id, ids);
        timeline.tracks.push(Track {
            name: shot.name.clone(),
            cells,
            exposures,
            end: shot.end,
        });
        state.scene.shots.entries.push(Bound {
            shot,
            track: id,
            cells: bindings,
        });
    }
    state
        .scene
        .catalog
        .tracks
        .initialize(timeline.tracks.len())?;
    state.animation = Some(timeline);
    book_io::check_cancel(cancel)?;
    document.commit_state(state)?;
    document.title = path.file_stem().unwrap().to_string_lossy().into_owned();
    document.current_frame = document.scene.shots.entries[0].shot.start;
    if document.resident_bytes() > budget || !valid_document(&document) {
        return Err("Shot batch exceeds native memory/reference limits".into());
    }
    Ok(Prepared { document, sources })
}
pub fn from_tracks(d: &Document) -> Result<crate::sizing::DocumentState, String> {
    let a = d
        .animation
        .as_ref()
        .ok_or("Create or import an animation first")?;
    if !a.valid() || a.tracks.is_empty() {
        return Err("Animation needs at least one valid track".into());
    }
    let mut state = d.snapshot();
    state.scene.catalog.tracks.initialize(a.tracks.len())?;
    let old = std::mem::take(&mut state.scene.shots.entries);
    let mut next = old
        .iter()
        .map(|b| b.shot.id)
        .max()
        .unwrap_or(0)
        .checked_add(1);
    for (n, t) in a.tracks.iter().enumerate() {
        if t.exposures.is_empty() {
            continue;
        }
        let track = state.scene.catalog.tracks.id(n);
        let ids = state.scene.catalog.cells.entry(track).or_default();
        ids.initialize(t.cells.len())?;
        let previous = old.iter().find(|b| b.track == track);
        let id = if let Some(b) = previous {
            b.shot.id
        } else {
            let id = next.ok_or("Shot ID space exhausted")?;
            next = id.checked_add(1);
            id
        };
        let cells = t
            .exposures
            .iter()
            .map(|e| {
                let id = ids.id(e.cell);
                let path = previous
                    .and_then(|b| {
                        b.cells
                            .iter()
                            .position(|v| *v == id)
                            .map(|n| b.shot.cells[n].path.clone())
                    })
                    .unwrap_or_else(|| format!("embedded-track-{track}-cell-{id}.png").into());
                Image {
                    name: t.cells[e.cell].name.clone(),
                    frame: e.frame,
                    path,
                    offset: t.cells[e.cell].offset,
                }
            })
            .collect();
        state.scene.shots.entries.push(Bound {
            shot: Shot {
                id,
                name: t.name.clone(),
                start: t.exposures[0].frame,
                end: t.end,
                fps: a.fps,
                dialogue: previous
                    .map(|b| b.shot.dialogue.clone())
                    .unwrap_or_default(),
                cells,
            },
            track,
            cells: t.exposures.iter().map(|e| ids.id(e.cell)).collect(),
        });
    }
    if !state.scene.shots.valid() {
        return Err("Animation track metadata exceeds shot limits".into());
    }
    Ok(state)
}
pub fn properties(
    d: &Document,
    rows: &[(u64, String, String)],
) -> Result<crate::sizing::DocumentState, String> {
    let mut state = d.snapshot();
    if rows.len() != state.scene.shots.entries.len()
        || rows.iter().map(|r| r.0).collect::<BTreeSet<_>>().len() != rows.len()
    {
        return Err("Shot property IDs changed".into());
    }
    let timeline = state.animation.as_mut().ok_or("Shot timeline is missing")?;
    for (id, name, dialogue) in rows {
        let b = state
            .scene
            .shots
            .entries
            .iter_mut()
            .find(|b| b.shot.id == *id)
            .ok_or("Shot ID removed")?;
        b.shot.name = name.clone();
        b.shot.dialogue = dialogue.clone();
        let n = state
            .scene
            .catalog
            .tracks
            .index(b.track, timeline.tracks.len())
            .ok_or("Shot track removed")?;
        timeline.tracks[n].name = name.clone();
    }
    if !state.scene.shots.valid() {
        return Err("Invalid shot name/dialogue".into());
    }
    Ok(state)
}
const HEADER: &str = "format,version,width,height,dpi,shot_id,name,start,end,fps_num,fps_den,dialogue,cell_frame,cell_path,x,y,cell_name";
pub(crate) fn quote(s: &str) -> String {
    if s.contains([',', '\"', '\n', '\r']) {
        format!("\"{}\"", s.replace('\"', "\"\""))
    } else {
        s.into()
    }
}
pub fn csv(file: &File) -> Result<Vec<u8>, String> {
    if !file.valid() {
        return Err("Invalid shot sheet".into());
    }
    let mut text = String::from(HEADER);
    text.push_str("\r\n");
    for s in &file.shots {
        for c in &s.cells {
            let row = vec![
                file.format.clone(),
                file.version.to_string(),
                file.width.to_string(),
                file.height.to_string(),
                file.dpi.to_string(),
                s.id.to_string(),
                s.name.clone(),
                s.start.to_string(),
                s.end.to_string(),
                s.fps.numerator.to_string(),
                s.fps.denominator.to_string(),
                s.dialogue.clone(),
                c.frame.to_string(),
                c.path.to_string_lossy().into_owned(),
                c.offset[0].to_string(),
                c.offset[1].to_string(),
                c.name.clone(),
            ];
            text.push_str(&row.iter().map(|s| quote(s)).collect::<Vec<_>>().join(","));
            text.push_str("\r\n");
        }
    }
    if text.len() > LIMIT {
        return Err("Shot CSV limit4MiB".into());
    }
    Ok(text.into_bytes())
}
pub(crate) fn rows(text: &str) -> Result<Vec<Vec<String>>, String> {
    csv_rows(text, 17, 10001)
}
pub(crate) fn csv_rows(
    text: &str,
    max_columns: usize,
    max_rows: usize,
) -> Result<Vec<Vec<String>>, String> {
    if text.len() > LIMIT {
        return Err("Shot CSV limit4MiB".into());
    }
    let mut rows = vec![];
    let mut row = vec![];
    let mut field = String::new();
    let mut quoted = false;
    let mut ended = false;
    let mut it = text.trim_start_matches('\u{feff}').chars().peekable();
    while let Some(c) = it.next() {
        if quoted {
            if c == '\"' {
                if it.peek() == Some(&'\"') {
                    it.next();
                    field.push('\"');
                } else {
                    quoted = false;
                    ended = true;
                }
            } else {
                field.push(c);
            }
            continue;
        }
        match c {
            '\"' if field.is_empty() && !ended => quoted = true,
            ',' => {
                row.push(std::mem::take(&mut field));
                ended = false;
            }
            '\n' | '\r' => {
                if c == '\r' && it.peek() == Some(&'\n') {
                    it.next();
                }
                row.push(std::mem::take(&mut field));
                rows.push(std::mem::take(&mut row));
                ended = false;
            }
            _ if ended || c == '\"' => return Err("Malformed quoted shot CSV".into()),
            _ => field.push(c),
        }
        if rows.len() > max_rows || row.len() > max_columns {
            return Err("Shot CSV row/column limit".into());
        }
    }
    if quoted {
        return Err("Unterminated shot CSV quote".into());
    }
    if !row.is_empty() || !field.is_empty() {
        row.push(field);
        rows.push(row);
    }
    Ok(rows)
}
pub fn from_csv(text: &str) -> Result<File, String> {
    let rows = rows(text)?;
    if rows.first().map(|r| r.join(",")) != Some(HEADER.into()) || rows.len() < 2 {
        return Err("Only Drawer shots v1 CSV has a verified adapter; header mismatch".into());
    }
    let number = |s: &str| {
        s.parse::<u32>()
            .map_err(|_| "Invalid/negative shot frame or numeric field".to_string())
    };
    let mut file = File {
        format: String::new(),
        version: 0,
        width: 0,
        height: 0,
        dpi: 0.,
        shots: vec![],
    };
    let mut ids = BTreeMap::new();
    for r in rows.into_iter().skip(1) {
        if r.len() != 17 {
            return Err("Shot CSV requires17 columns".into());
        }
        let common = (
            r[0].clone(),
            number(&r[1])?,
            number(&r[2])?,
            number(&r[3])?,
            r[4].parse::<f64>().map_err(|e| e.to_string())?,
        );
        if file.shots.is_empty() {
            file.format = common.0;
            file.version = common.1;
            file.width = common.2;
            file.height = common.3;
            file.dpi = common.4;
        } else if (
            file.format.clone(),
            file.version,
            file.width,
            file.height,
            file.dpi,
        ) != common
        {
            return Err("Contradictory shot CSV metadata".into());
        }
        let id = r[5].parse::<u64>().map_err(|e| e.to_string())?;
        let shot = Shot {
            id,
            name: r[6].clone(),
            start: number(&r[7])?,
            end: number(&r[8])?,
            fps: Fps {
                numerator: number(&r[9])?,
                denominator: number(&r[10])?,
            },
            dialogue: r[11].clone(),
            cells: vec![],
        };
        let image = Image {
            name: r[16].clone(),
            frame: number(&r[12])?,
            path: r[13].clone().into(),
            offset: [
                r[14].parse().map_err(|_| "Invalid cell X")?,
                r[15].parse().map_err(|_| "Invalid cell Y")?,
            ],
        };
        if let Some(n) = ids.get(&id).copied() {
            let s: &mut Shot = &mut file.shots[n];
            let mut base = s.clone();
            base.cells.clear();
            if base != shot {
                return Err("Duplicate shot ID with contradictory metadata".into());
            }
            s.cells.push(image);
        } else {
            ids.insert(id, file.shots.len());
            file.shots.push(Shot {
                cells: vec![image],
                ..shot
            });
        }
    }
    if !file.valid() {
        return Err("Invalid shot CSV IDs/ranges/cells/FPS".into());
    }
    Ok(file)
}
pub fn export(
    d: &Document,
    path: &Path,
    budget: usize,
    cancel: &AtomicBool,
    writer: &mut impl FnMut(&Path, &Path) -> Result<(), String>,
) -> Result<(), String> {
    if d.scene.shots.is_default() || !valid_document(d) {
        return Err("Import a valid shot batch before exporting its metadata".into());
    }
    let csv_output = path
        .extension()
        .is_some_and(|x| x.eq_ignore_ascii_case("csv"));
    if !csv_output
        && !path
            .extension()
            .is_some_and(|x| x.eq_ignore_ascii_case("drawer-shots"))
    {
        return Err("Use .drawer-shots JSON or own-format .csv".into());
    }
    let parent = path
        .parent()
        .ok_or("Choose an exchange folder")?
        .canonicalize()
        .map_err(|e| e.to_string())?;
    let mut owned = crate::page_resources::prepare(std::slice::from_ref(d), &parent, cancel)?;
    let timeline = d.animation.as_ref().unwrap();
    let mut file = File {
        format: "Drawer shots".into(),
        version: 1,
        width: d.width,
        height: d.height,
        dpi: d.dpi,
        shots: vec![],
    };
    let mut used = d.resident_bytes();
    for b in &d.scene.shots.entries {
        book_io::check_cancel(cancel)?;
        let n = d
            .scene
            .catalog
            .tracks
            .index(b.track, timeline.tracks.len())
            .unwrap();
        let t = &timeline.tracks[n];
        if t.exposures.is_empty() {
            return Err("Assign an exposure before exporting this shot".into());
        }
        let mut shot = b.shot.clone();
        shot.name = t.name.clone();
        shot.start = t.exposures[0].frame;
        shot.end = t.end;
        shot.fps = timeline.fps;
        shot.cells.clear();
        let mut paths = BTreeMap::new();
        for e in &t.exposures {
            let c = &t.cells[e.cell];
            let path = if let Some(p) = paths.get(&e.cell) {
                PathBuf::clone(p)
            } else {
                use image::ImageEncoder;
                let raw = c.pixels.iter().flatten().copied().collect::<Vec<_>>();
                if used.saturating_add(raw.len() * 2) > budget {
                    return Err("Shot PNG export memory limit".into());
                }
                let mut png = vec![];
                image::codecs::png::PngEncoder::new(&mut png)
                    .write_image(&raw, c.width, c.height, image::ExtendedColorType::Rgba8)
                    .map_err(|e| e.to_string())?;
                used = used.max(d.resident_bytes() + raw.len() + png.len());
                let p = owned.blob(&format!("shot-{}-cell-{}.png", shot.id, e.cell + 1), &png)?;
                let p = p
                    .strip_prefix(&parent)
                    .map_err(|e| e.to_string())?
                    .to_path_buf();
                paths.insert(e.cell, p.clone());
                p
            };
            shot.cells.push(Image {
                name: c.name.clone(),
                frame: e.frame,
                path,
                offset: c.offset,
            });
        }
        file.shots.push(shot);
    }
    if !file.valid() {
        return Err("Edited native shot metadata/ranges are not exportable".into());
    }
    let bytes = if csv_output {
        csv(&file)?
    } else {
        serde_json::to_vec_pretty(&file).map_err(|e| e.to_string())?
    };
    if bytes.len() > LIMIT {
        return Err("Shot exchange limit4MiB".into());
    }
    book_io::publish(path, &bytes, cancel, writer)?;
    owned.keep();
    Ok(())
}
