//! Independent local committed-frame journal. A manifest is the publication boundary.
use crate::{
    animation::{Cell, Exposure, Fps, Timeline},
    document::{Background, CanvasSpec, Document},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    io::Write,
    path::{Path, PathBuf},
    sync::{Arc, atomic::AtomicBool},
};
const MAX_INDEX: usize = 4 * 1024 * 1024;
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Frame {
    pub id: u64,
    pub file: String,
    pub width: u32,
    pub height: u32,
    pub revision: u64,
    pub active_ms: u64,
    pub bytes: u64,
    pub hash: [u8; 32],
    pub pixels: [u8; 32],
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Data {
    pub format: String,
    pub version: u32,
    pub limit: u64,
    pub active_ms: u64,
    pub next: u64,
    pub frames: Vec<Frame>,
}
impl Data {
    pub fn bytes(&self) -> u64 {
        self.frames.iter().map(|f| f.bytes).sum()
    }
    fn valid(&self) -> bool {
        self.format == "Drawer timelapse"
            && self.version == 1
            && self.limit > 0
            && self.limit <= 2 * 1024 * 1024 * 1024
            && self.next > 0
            && self.frames.len() <= 10000
            && self.frames.iter().all(|f| {
                f.id > 0
                    && f.id < self.next
                    && f.file == format!("frame-{:06}.png", f.id)
                    && (1..=32768).contains(&f.width)
                    && (1..=32768).contains(&f.height)
                    && f.active_ms <= self.active_ms
                    && f.bytes > 0
                    && f.bytes <= 64 * 1024 * 1024
            })
            && self
                .frames
                .windows(2)
                .all(|a| a[0].id < a[1].id && a[0].active_ms <= a[1].active_ms)
            && self.bytes() <= self.limit
    }
    fn encode(&self) -> Result<Vec<u8>, String> {
        if !self.valid() {
            return Err("Invalid timelapse journal".into());
        }
        let b = serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?;
        if b.len() > MAX_INDEX || self.bytes().saturating_add(b.len() as u64) > self.limit {
            return Err(
                "Timelapse size limit reached; increase the limit or start a new recording".into(),
            );
        }
        Ok(b)
    }
}
pub struct Store {
    pub root: PathBuf,
    pub data: Data,
    stamp: [u8; 32],
}
fn local_root(path: &Path) -> Result<PathBuf, String> {
    if !crate::font_library::local(path) {
        return Err("Choose a local recording folder".into());
    }
    let p = path.canonicalize().map_err(|e| e.to_string())?;
    if !crate::font_library::local(&p) || !p.is_dir() {
        return Err("Choose a local recording directory".into());
    }
    Ok(p)
}
fn read(path: &Path, limit: usize) -> Result<Vec<u8>, String> {
    let m = std::fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if !m.is_file() || m.file_type().is_symlink() {
        return Err("Timelapse files must be regular local files".into());
    }
    crate::book_io::read_local(path, limit)
}
impl Store {
    pub fn create(path: &Path, limit: u64) -> Result<Self, String> {
        if !crate::font_library::local(path) {
            return Err("Choose a new local recording folder".into());
        }
        let data = Data {
            format: "Drawer timelapse".into(),
            version: 1,
            limit,
            active_ms: 0,
            next: 1,
            frames: vec![],
        };
        let bytes = data.encode()?;
        std::fs::create_dir(path).map_err(|e| format!("Choose a new recording folder: {e}"))?;
        let result = (|| {
            let root = local_root(path)?;
            let mut f = std::fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(root.join("timelapse.json"))
                .map_err(|e| e.to_string())?;
            f.write_all(&bytes)
                .and_then(|_| f.sync_all())
                .map_err(|e| e.to_string())?;
            Ok(Self {
                root,
                data,
                stamp: Sha256::digest(bytes).into(),
            })
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(path.join("timelapse.json"));
            let _ = std::fs::remove_dir(path);
        }
        result
    }
    pub fn open(path: &Path, cancel: &AtomicBool) -> Result<Self, String> {
        let root = local_root(path)?;
        let bytes = read(&root.join("timelapse.json"), MAX_INDEX)?;
        let data: Data = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
        data.encode()?;
        for f in &data.frames {
            crate::book_io::check_cancel(cancel)?;
            let b = read(&root.join(&f.file), 64 * 1024 * 1024)?;
            if b.len() as u64 != f.bytes || <[u8; 32]>::from(Sha256::digest(&b)) != f.hash {
                return Err(format!("Recorded frame changed or is missing: {}", f.file));
            }
        }
        Ok(Self {
            root,
            data,
            stamp: Sha256::digest(bytes).into(),
        })
    }
    fn fresh(&self) -> Result<(), String> {
        if <[u8; 32]>::from(Sha256::digest(read(
            &self.root.join("timelapse.json"),
            MAX_INDEX,
        )?)) != self.stamp
        {
            return Err(
                "Recording manifest changed outside this session; reopen it before recording"
                    .into(),
            );
        }
        Ok(())
    }
    fn lease(&self) -> Result<std::fs::File, String> {
        let path = self.root.join(".timelapse.lock");
        if std::fs::symlink_metadata(&path)
            .is_ok_and(|m| !m.is_file() || m.file_type().is_symlink())
        {
            return Err("Recording lease must be a regular local file".into());
        }
        let f = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)
            .map_err(|e| e.to_string())?;
        f.try_lock()
            .map_err(|e| format!("Recording folder is busy: {e}"))?;
        Ok(f)
    }
    fn commit(&mut self, data: Data) -> Result<(), String> {
        let _lease = self.lease()?;
        self.commit_locked(data)
    }
    fn commit_locked(&mut self, data: Data) -> Result<(), String> {
        let bytes = data.encode()?;
        self.fresh()?;
        let mut stage = crate::animation_export::Stage::new(&self.root)?;
        let mut f = stage.create("index.json")?;
        f.write_all(&bytes)
            .and_then(|_| f.sync_all())
            .map_err(|e| e.to_string())?;
        drop(f);
        self.fresh()?;
        std::fs::rename(
            stage.directory.join("index.json"),
            self.root.join("timelapse.json"),
        )
        .map_err(|e| e.to_string())?;
        self.stamp = Sha256::digest(bytes).into();
        self.data = data;
        Ok(())
    }
    pub fn checkpoint(&mut self, active_ms: u64) -> Result<(), String> {
        let mut d = self.data.clone();
        d.active_ms = d.active_ms.max(active_ms);
        self.commit(d)
    }
    pub fn set_limit(&mut self, limit: u64) -> Result<(), String> {
        let mut d = self.data.clone();
        d.limit = limit;
        self.commit(d)
    }
    /// Original document state is read only. A failed/canceled append cannot publish an index entry.
    pub fn append(
        &mut self,
        snapshot: &crate::animation_export::Snapshot,
        active_ms: u64,
        cancel: &AtomicBool,
        publish: &mut impl FnMut(&Path, &Path) -> Result<(), String>,
    ) -> Result<bool, String> {
        crate::book_io::check_cancel(cancel)?;
        let _lease = self.lease()?;
        self.fresh()?;
        let raw = crate::animation_render::frame(snapshot, snapshot.options.first, cancel)?;
        let mut h = Sha256::new();
        h.update(snapshot.layout.width.to_le_bytes());
        h.update(snapshot.layout.height.to_le_bytes());
        h.update(&raw);
        let pixels: [u8; 32] = h.finalize().into();
        if self.data.frames.last().is_some_and(|f| f.pixels == pixels) {
            let mut data = self.data.clone();
            data.active_ms = data.active_ms.max(active_ms);
            self.commit_locked(data)?;
            return Ok(false);
        }
        if self.data.frames.len() >= 10000 {
            return Err("Recording reached10000 frames; start a new recording".into());
        }
        let mut stage = crate::animation_export::Stage::new(&self.root)?;
        let private = stage.directory.join("frame.png");
        stage.files.push(private.clone());
        crate::animation_export::write_png(snapshot, &private, &raw)?;
        let encoded = read(&private, 64 * 1024 * 1024)?;
        let hash: [u8; 32] = Sha256::digest(&encoded).into();
        let mut id = self.data.next;
        let mut file = format!("frame-{id:06}.png");
        let mut attempts = 0;
        while std::fs::symlink_metadata(self.root.join(&file)).is_ok() {
            id = id.checked_add(1).ok_or("Frame ID exhausted")?;
            file = format!("frame-{id:06}.png");
            attempts += 1;
            if attempts > 1024 {
                return Err("Too many unlisted frame files; choose a new recording folder".into());
            }
        }
        let mut data = self.data.clone();
        data.next = id.checked_add(1).ok_or("Frame ID exhausted")?;
        data.active_ms = data.active_ms.max(active_ms);
        data.frames.push(Frame {
            id,
            file: file.clone(),
            width: snapshot.layout.width,
            height: snapshot.layout.height,
            revision: snapshot.revision,
            active_ms: data.active_ms,
            bytes: encoded.len() as u64,
            hash,
            pixels,
        });
        data.encode()?;
        crate::book_io::check_cancel(cancel)?;
        self.fresh()?;
        let target = self.root.join(file);
        publish(&private, &target)?;
        let result = (|| {
            crate::book_io::check_cancel(cancel)?;
            self.commit_locked(data)
        })();
        if let Err(e) = result {
            if read(&target, 64 * 1024 * 1024)
                .is_ok_and(|b| <[u8; 32]>::from(Sha256::digest(b)) == hash)
            {
                let _ = std::fs::remove_file(target);
            }
            return Err(e);
        }
        Ok(true)
    }
}
pub fn capture(d: &Document, budget: usize) -> Result<crate::animation_export::Snapshot, String> {
    let mut frozen = crate::layer_merge::preview(&d.snapshot(), d.current_frame)?;
    if frozen.animation.is_none() {
        frozen.commit_state(crate::timeline_editor::create(
            &frozen,
            1,
            Default::default(),
        )?)?;
    }
    let mut s = crate::animation_export::prepare(
        &frozen,
        crate::animation_export::Options {
            first: frozen.current_frame,
            last: frozen.current_frame,
            view: crate::animation_render::View {
                drafts: true,
                icc: false,
                preserve_paper: true,
                ..Default::default()
            },
            ..Default::default()
        },
        budget,
    )?;
    s.revision = d.revision;
    Ok(s)
}
/// Decode only the committed cutoff, independently from the live editor and later recording writes.
pub fn animation(
    root: &Path,
    first: usize,
    last: usize,
    fps: Fps,
    budget: usize,
    cancel: &AtomicBool,
) -> Result<Document, String> {
    let store = Store::open(root, cancel)?;
    if !fps.valid() || first > last || last >= store.data.frames.len() {
        return Err("Choose an existing recorded frame range and a valid rational FPS".into());
    }
    let frames = &store.data.frames[first..=last];
    let width = frames.iter().map(|f| f.width).max().unwrap();
    let height = frames.iter().map(|f| f.height).max().unwrap();
    let mut doc = Document::create(
        CanvasSpec {
            width,
            height,
            dpi: 300.,
            background: Background::Transparent,
        },
        budget / 8,
    )
    .map_err(|e| e.to_string())?;
    let mut cells = vec![];
    let mut used = doc.memory_bytes() * 6;
    for f in frames {
        crate::book_io::check_cancel(cancel)?;
        let b = read(
            &store.root.join(&f.file),
            (64 * 1024 * 1024).min(budget / 4),
        )?;
        if <[u8; 32]>::from(Sha256::digest(&b)) != f.hash {
            return Err("Recorded frame changed during output capture".into());
        }
        let image = crate::book_io::decode(&b, budget.saturating_sub(used) / 2)?;
        if image.width != f.width || image.height != f.height {
            return Err("Recorded frame dimensions changed".into());
        }
        used = used.saturating_add(image.pixels.len() * 24);
        if used > budget {
            return Err("Recorded frame output exceeds memory; choose a smaller range".into());
        }
        cells.push(Cell {
            name: format!("Recorded {}", f.id),
            width: f.width,
            height: f.height,
            offset: [
                ((width - f.width) / 2) as i64,
                ((height - f.height) / 2) as i64,
            ],
            pixels: Arc::new(image.pixels),
        });
    }
    let count = cells.len() as u32;
    let mut state = doc.snapshot();
    state.animation = Some(Timeline {
        fps,
        frames: count,
        tracks: vec![crate::animation::Track {
            name: "Timelapse".into(),
            cells,
            exposures: (0..count)
                .map(|n| Exposure {
                    frame: n,
                    cell: n as usize,
                })
                .collect(),
            end: count,
        }],
        audio: vec![],
    });
    doc.commit_state(state)?;
    doc.title = format!("Timelapse · {} recorded frames", count);
    Ok(doc)
}
/// Pure elapsed clock: pause intervals and wall-clock adjustments never enter the journal.
pub struct Clock {
    base: u64,
    last: f64,
    paused: bool,
}
impl Clock {
    pub fn new(base: u64, now: f64) -> Self {
        Self {
            base,
            last: now,
            paused: false,
        }
    }
    pub fn at(&self, now: f64) -> u64 {
        self.base
            .saturating_add(if self.paused || !now.is_finite() || now < self.last {
                0
            } else {
                ((now - self.last) * 1000.).min(u64::MAX as f64) as u64
            })
    }
    pub fn pause(&mut self, now: f64) {
        self.base = self.at(now);
        self.last = now;
        self.paused = true;
    }
    pub fn resume(&mut self, now: f64) {
        if self.paused {
            self.last = now;
            self.paused = false;
        }
    }
    pub fn paused(&self) -> bool {
        self.paused
    }
}

/// Real filesystem smoke verification, also executed by the installed application.
pub fn verify(
    root: &Path,
    publish: &mut impl FnMut(&Path, &Path) -> Result<(), String>,
) -> Result<(), String> {
    use crate::document::DEFAULT_DOCUMENT_BUDGET as B;
    let dir = root.join("timelapse-verify");
    std::fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let check = |b: bool, m: &str| if b { Ok(()) } else { Err(m.to_owned()) };
    let result = (|| {
        let cancel = AtomicBool::new(false);
        let path = dir.join("record");
        let mut store = Store::create(&path, 1024 * 1024)?;
        let mut d = Document::create(
            CanvasSpec {
                width: 2,
                height: 1,
                dpi: 300.,
                background: Background::Transparent,
            },
            B,
        )
        .map_err(|e| e.to_string())?;
        let original = d.snapshot();
        let counts = d.history_counts();
        let dirty = d.dirty;
        check(
            store.append(&capture(&d, B)?, 0, &cancel, publish)?,
            "Initial committed frame missing",
        )?;
        check(
            !store.append(&capture(&d, B)?, 100, &cancel, publish)? && store.data.frames.len() == 1,
            "Unchanged pixels duplicated",
        )?;
        check(
            d.snapshot() == original && d.history_counts() == counts && d.dirty == dirty,
            "Recording changed drawing/history",
        )?;
        d.edit_pixel(0, 0, 0, [255, 0, 0, 128]);
        store.append(&capture(&d, B)?, 1000, &cancel, publish)?;
        let frozen = d.snapshot();
        let count = store.data.frames.len();
        let index = read(&path.join("timelapse.json"), MAX_INDEX)?;
        d.edit_pixel(0, 1, 0, [0, 0, 255, 255]);
        let c = AtomicBool::new(true);
        check(
            store.append(&capture(&d, B)?, 1500, &c, publish).is_err(),
            "Canceled capture published",
        )?;
        let failed = store.append(&capture(&d, B)?, 1500, &cancel, &mut |_, _| {
            Err("simulated disk full".into())
        });
        check(
            failed.is_err()
                && store.data.frames.len() == count
                && read(&path.join("timelapse.json"), MAX_INDEX)? == index,
            "Write failure changed committed journal",
        )?;
        let canceled = AtomicBool::new(false);
        check(
            store
                .append(&capture(&d, B)?, 1500, &canceled, &mut |a, b| {
                    publish(a, b)?;
                    canceled.store(true, std::sync::atomic::Ordering::Relaxed);
                    Ok(())
                })
                .is_err()
                && store.data.frames.len() == count,
            "Cancel after PNG publication retained uncommitted frame",
        )?;
        check(
            read(&path.join("timelapse.json"), MAX_INDEX)? == index,
            "Cancellation changed index",
        )?;
        check(
            store.set_limit(1).is_err() && store.data.limit == 1024 * 1024,
            "Storage cap damaged existing journal",
        )?;
        let state = crate::sizing::prepare(
            &d.snapshot(),
            crate::sizing::Change::Canvas {
                width: 3,
                height: 2,
                anchor: [0., 0.],
            },
            B,
            &cancel,
        )?;
        d.commit_state(state)?;
        store.append(&capture(&d, B)?, 2000, &cancel, publish)?;
        let mut other = Store::open(&path, &cancel)?;
        d.edit_pixel(0, 2, 1, [0, 200, 0, 255]);
        store.append(&capture(&d, B)?, 2500, &cancel, &mut |a, b| {
            check(
                other.checkpoint(3000).is_err(),
                "Concurrent writer lease was bypassed",
            )?;
            publish(a, b)
        })?;
        check(
            other.checkpoint(4000).is_err(),
            "Stale writer overwrote committed metadata",
        )?;
        store.checkpoint(3000)?;
        let reopened = Store::open(&path, &cancel)?;
        check(reopened.data == store.data, "Restart lost recording data")?;
        let prepared = animation(
            &path,
            0,
            3,
            Fps {
                numerator: 24,
                denominator: 1,
            },
            B,
            &cancel,
        )?;
        check(
            prepared.width == 3
                && prepared.height == 2
                && prepared.animation.as_ref().unwrap().frames == 4,
            "Canvas changes / output frame count",
        )?;
        let snapshot = crate::animation_export::prepare(
            &prepared,
            crate::animation_export::Options {
                first: 0,
                last: 3,
                ..Default::default()
            },
            B,
        )?;
        d.edit_pixel(0, 2, 1, [50, 60, 70, 255]);
        store.append(&capture(&d, B)?, 3500, &cancel, publish)?;
        let report = crate::animation_export::write(
            &snapshot,
            &dir.join("output"),
            false,
            &cancel,
            &std::sync::atomic::AtomicU32::new(0),
        )?;
        check(report.frames == 4, "Output followed later record writes")?;
        let metadata: serde_json::Value =
            serde_json::from_slice(&read(&dir.join("output/animation.json"), MAX_INDEX)?)
                .map_err(|e| e.to_string())?;
        check(
            metadata["fps"]["numerator"] == 24 && metadata["files"].as_array().unwrap().len() == 4,
            "Fixed output timing / range",
        )?;
        check(d.snapshot() != frozen, "Fixture drawing edits missing")?;
        let last = store.data.frames.last().unwrap();
        let f = path.join(&last.file);
        let bytes = read(&f, 64 * 1024 * 1024)?;
        std::fs::write(&f, b"changed").map_err(|e| e.to_string())?;
        check(
            Store::open(&path, &cancel).is_err(),
            "Changed recorded frame accepted",
        )?;
        std::fs::write(&f, bytes).map_err(|e| e.to_string())?;
        let orphan = path.join(format!("frame-{:06}.png", store.data.next));
        std::fs::write(&orphan, b"unlisted preserve").map_err(|e| e.to_string())?;
        d.edit_pixel(0, 2, 1, [10, 20, 30, 255]);
        store.append(&capture(&d, B)?, 4000, &cancel, publish)?;
        check(
            read(&orphan, 100)? == b"unlisted preserve",
            "Unlisted crash residue was overwritten",
        )?;
        check(
            std::fs::read_dir(&path)
                .map_err(|e| e.to_string())?
                .all(|e| {
                    !e.unwrap()
                        .file_name()
                        .to_string_lossy()
                        .starts_with(".Drawer-animation")
                }),
            "Owned incomplete recording files left",
        )?;
        Ok(())
    })();
    let cleanup = std::fs::remove_dir_all(&dir).map_err(|e| e.to_string());
    result.and(cleanup)
}
#[cfg(test)]
#[path = "timelapse_tests.rs"]
mod tests;
