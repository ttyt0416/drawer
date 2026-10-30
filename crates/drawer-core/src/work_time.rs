//! Work metadata is independent of drawing state, history and revision.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    io::Write,
    path::{Path, PathBuf},
};

pub const IDLE_MS: u64 = 300_000;
pub const MAX_GAP_MS: u64 = 5_000;
const MAX_MS: u64 = 100 * 366 * 24 * 3600 * 1000;
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Snapshot {
    pub id: String,
    pub sequence: u64,
    pub milliseconds: u64,
    pub paused: bool,
    pub reset_undo: Vec<u64>,
}
impl Snapshot {
    pub fn new(id: String) -> Self {
        Self {
            id,
            sequence: 0,
            milliseconds: 0,
            paused: false,
            reset_undo: vec![],
        }
    }
    pub fn valid(&self) -> bool {
        self.id.len() == 64
            && self
                .id
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            && self.milliseconds <= MAX_MS
            && self.reset_undo.len() <= 16
            && self.reset_undo.iter().all(|n| *n <= MAX_MS)
    }
    fn update(
        &mut self,
        change: impl FnOnce(&mut Self) -> Result<(), String>,
    ) -> Result<(), String> {
        let mut next = self.clone();
        next.sequence = next
            .sequence
            .checked_add(1)
            .ok_or("Work time sequence exhausted")?;
        change(&mut next)?;
        if !next.valid() {
            return Err("Invalid or exhausted work time record".into());
        }
        *self = next;
        Ok(())
    }
    pub fn add(&mut self, ms: u64) -> Result<(), String> {
        if ms == 0 || self.paused {
            return Ok(());
        }
        self.update(|s| {
            s.milliseconds = s.milliseconds.checked_add(ms).ok_or("Work time overflow")?;
            Ok(())
        })
    }
    pub fn pause(&mut self, paused: bool) -> Result<(), String> {
        if paused == self.paused {
            return Ok(());
        }
        self.update(|s| {
            s.paused = paused;
            Ok(())
        })
    }
    pub fn reset(&mut self) -> Result<(), String> {
        self.update(|s| {
            if s.reset_undo.len() == 16 {
                s.reset_undo.remove(0);
            }
            s.reset_undo.push(s.milliseconds);
            s.milliseconds = 0;
            Ok(())
        })
    }
    pub fn undo_reset(&mut self) -> Result<(), String> {
        self.update(|s| {
            let removed = s.reset_undo.pop().ok_or("No time reset to undo")?;
            s.milliseconds = s
                .milliseconds
                .checked_add(removed)
                .ok_or("Work time overflow")?;
            Ok(())
        })
    }
}
fn hex(bytes: impl AsRef<[u8]>) -> String {
    bytes.as_ref().iter().map(|b| format!("{b:02x}")).collect()
}
pub fn identity() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(1);
    // Wall time contributes only to identity, never to duration accounting.
    hex(Sha256::digest(format!(
        "Drawer work identity {:?} {} {}",
        std::time::SystemTime::now(),
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )))
}
pub fn legacy_identity(path: &Path) -> String {
    hex(Sha256::digest(format!(
        "Drawer legacy native work identity {}",
        path.to_string_lossy().to_lowercase()
    )))
}

/// One active owner across every document/view. Call on switches as well as repaint ticks.
#[derive(Default)]
pub struct Clock {
    last: Option<u64>,
    owner: Option<u64>,
    until: u64,
    focused: bool,
}
impl Clock {
    pub fn step(
        &mut self,
        now: u64,
        active: Option<u64>,
        focused: bool,
        editing: bool,
    ) -> Option<(u64, u64)> {
        let previous = self.last.replace(now);
        let gap = previous.map_or(0, |p| now.saturating_sub(p));
        let valid_gap = previous.is_some_and(|p| now >= p) && gap <= MAX_GAP_MS;
        let credit = self
            .owner
            .filter(|_| self.focused && valid_gap)
            .and_then(|id| {
                let start = previous?;
                let ms = now.min(self.until).saturating_sub(start);
                (ms > 0).then_some((id, ms))
            });
        if self.owner != active || !focused || !self.focused || !valid_gap {
            self.until = 0;
        }
        self.owner = active;
        self.focused = focused;
        if focused && active.is_some() && editing {
            self.until = now.saturating_add(IDLE_MS);
        }
        credit
    }
    pub fn running(&self, now: u64) -> bool {
        self.focused && self.owner.is_some() && now < self.until
    }
    pub fn break_activity(&mut self) {
        self.until = 0;
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Record {
    pub format: String,
    pub version: u32,
    pub title: String,
    pub origin: Option<String>,
    pub snapshot: Snapshot,
}
impl Record {
    pub fn new(snapshot: Snapshot, title: String, origin: Option<String>) -> Self {
        Self {
            format: "Drawer work time".into(),
            version: 1,
            title,
            origin,
            snapshot,
        }
    }
    pub fn valid(&self) -> bool {
        self.format == "Drawer work time"
            && self.version == 1
            && self.title.len() <= 4096
            && self.origin.as_ref().is_none_or(|s| s.len() <= 8192)
            && self.snapshot.valid()
    }
    pub fn encode(&self) -> Result<Vec<u8>, String> {
        if !self.valid() {
            return Err("Invalid work time metadata".into());
        }
        let b = serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?;
        if b.len() > 32 * 1024 {
            return Err("Work metadata exceeds32KiB".into());
        }
        Ok(b)
    }
}
fn read(path: &Path) -> Result<Vec<u8>, String> {
    let m = std::fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if !m.is_file() || m.file_type().is_symlink() {
        return Err("Work metadata must be a regular file".into());
    }
    crate::book_io::read_local(path, 32 * 1024)
}
fn decode(b: &[u8]) -> Result<Record, String> {
    let r: Record = serde_json::from_slice(b).map_err(|e| e.to_string())?;
    r.encode()?;
    Ok(r)
}
pub struct Journal {
    root: PathBuf,
    path: PathBuf,
    stamp: Option<[u8; 32]>,
    _lease: std::fs::File,
}
impl Journal {
    pub fn open(root: &Path, record: &mut Record) -> Result<Self, String> {
        record.encode()?;
        if !crate::font_library::local(root) {
            return Err("Work time requires local storage".into());
        }
        std::fs::create_dir_all(root).map_err(|e| e.to_string())?;
        let root = root.canonicalize().map_err(|e| e.to_string())?;
        if !crate::font_library::local(&root) {
            return Err("Work time requires local storage".into());
        }
        let path = root.join(format!("{}.json", record.snapshot.id));
        let lock = root.join(format!("{}.lock", record.snapshot.id));
        if std::fs::symlink_metadata(&lock)
            .is_ok_and(|m| !m.is_file() || m.file_type().is_symlink())
        {
            return Err("Work time lease must be a regular file".into());
        }
        let lease = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .open(lock)
            .map_err(|e| e.to_string())?;
        lease
            .try_lock()
            .map_err(|e| format!("Work time is open in another session: {e}"))?;
        let stamp = if path.try_exists().map_err(|e| e.to_string())? {
            let bytes = read(&path)?;
            let stored = decode(&bytes)?;
            if stored.snapshot.id != record.snapshot.id {
                return Err("Work identity mismatch".into());
            }
            if stored.snapshot.sequence == record.snapshot.sequence
                && stored.snapshot != record.snapshot
            {
                return Err(
                    "Conflicting work time snapshots; preserve both files for recovery".into(),
                );
            }
            if stored.snapshot.sequence > record.snapshot.sequence {
                record.snapshot = stored.snapshot;
            }
            Some(Sha256::digest(bytes).into())
        } else {
            None
        };
        let mut journal = Self {
            root,
            path,
            stamp,
            _lease: lease,
        };
        journal.save(record)?;
        Ok(journal)
    }
    pub fn save(&mut self, record: &Record) -> Result<(), String> {
        if self.path.file_stem().and_then(|s| s.to_str()) != Some(&record.snapshot.id) {
            return Err("Work identity mismatch".into());
        }
        let b = record.encode()?;
        if std::fs::symlink_metadata(&self.path)
            .is_ok_and(|m| !m.is_file() || m.file_type().is_symlink())
        {
            return Err("Work journal must be a regular file".into());
        }
        let actual = if self.path.try_exists().map_err(|e| e.to_string())? {
            Some(Sha256::digest(read(&self.path)?).into())
        } else {
            None
        };
        if self.stamp != actual {
            return Err("Work journal changed outside this session; original kept".into());
        }
        if self.stamp == Some(Sha256::digest(&b).into()) {
            return Ok(());
        }
        let mut stage = crate::animation_export::Stage::new(&self.root)?;
        let mut f = stage.create("work.json")?;
        f.write_all(&b)
            .and_then(|_| f.sync_all())
            .map_err(|e| e.to_string())?;
        drop(f);
        let current = if self.path.try_exists().map_err(|e| e.to_string())? {
            Some(Sha256::digest(read(&self.path)?).into())
        } else {
            None
        };
        if current != self.stamp {
            return Err("Work journal changed before publication; original kept".into());
        }
        std::fs::rename(stage.directory.join("work.json"), &self.path)
            .map_err(|e| e.to_string())?;
        self.stamp = Some(Sha256::digest(b).into());
        Ok(())
    }
}
pub fn catalog(root: &Path) -> Result<Vec<Record>, String> {
    if !crate::font_library::local(root) {
        return Err("Choose local time storage".into());
    }
    if !root.exists() {
        return Ok(vec![]);
    }
    let mut rows = vec![];
    for (n, entry) in std::fs::read_dir(root)
        .map_err(|e| e.to_string())?
        .enumerate()
    {
        if n >= 8192 {
            return Err(
                "Too many local time records; move older journals to an archive folder".into(),
            );
        }
        let p = entry.map_err(|e| e.to_string())?.path();
        if p.extension().is_some_and(|e| e == "json") {
            let r = decode(&read(&p)?)?;
            if p.file_stem().and_then(|s| s.to_str()) != Some(&r.snapshot.id) {
                return Err("Work journal filename identity mismatch".into());
            }
            rows.push(r);
        }
    }
    rows.sort_by(|a, b| {
        a.title
            .cmp(&b.title)
            .then(a.snapshot.id.cmp(&b.snapshot.id))
    });
    Ok(rows)
}
/// New filenames only. The caller handles its drawing/source path protection.
pub fn export(record: &Record, path: &Path) -> Result<(), String> {
    if !crate::font_library::local(path) {
        return Err("Choose a local work time output file".into());
    }
    let b = record.encode()?;
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| e.to_string())?;
    let r = f
        .write_all(&b)
        .and_then(|_| f.sync_all())
        .map_err(|e| e.to_string());
    drop(f);
    if r.is_err() {
        let _ = std::fs::remove_file(path);
    }
    r
}

/// Installed-app filesystem verification; all fixture files stay in the caller's private root.
pub fn verify(root: &Path) -> Result<(), String> {
    let dir = root.join("work-time-core");
    std::fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let result = (|| {
        let check = |b: bool, m: &str| if b { Ok(()) } else { Err(m.to_owned()) };
        let mut d = crate::document::Document::create(
            crate::document::CanvasSpec {
                width: 2,
                height: 2,
                dpi: 300.,
                background: crate::document::Background::Transparent,
            },
            1024 * 1024,
        )
        .map_err(|e| e.to_string())?;
        let state = d.snapshot();
        let dirty = d.dirty;
        let counts = d.history_counts();
        let revision = d.revision;
        let mut r = Record::new(Snapshot::new(identity()), "Untitled recovery".into(), None);
        let mut j = Journal::open(&dir.join("journals"), &mut r)?;
        let mut copy = r.clone();
        check(
            Journal::open(&dir.join("journals"), &mut copy).is_err(),
            "Concurrent time writer allowed",
        )?;
        r.snapshot.add(1234)?;
        j.save(&r)?;
        let old = r.clone();
        r.snapshot.reset()?;
        j.save(&r)?;
        drop(j);
        let mut stale = old.clone();
        let mut j = Journal::open(&dir.join("journals"), &mut stale)?;
        check(
            stale.snapshot.milliseconds == 0 && stale.snapshot.sequence > old.snapshot.sequence,
            "Reset lost on crash recovery",
        )?;
        stale.snapshot.undo_reset()?;
        stale.snapshot.pause(true)?;
        j.save(&stale)?;
        d.work_time = Some(stale.snapshot.clone());
        check(
            d.snapshot() == state
                && d.dirty == dirty
                && d.history_counts() == counts
                && d.revision == revision,
            "Time changed drawing state",
        )?;
        let path = dir.join("time.drawer");
        crate::files::save(&mut d, &path).map_err(|e| e.to_string())?;
        let opened = crate::files::open(&path, 1024 * 1024).map_err(|e| e.to_string())?;
        check(
            opened.work_time == d.work_time && !opened.dirty,
            "Native time snapshot lost",
        )?;
        let mut native = std::fs::read(&path).map_err(|e| e.to_string())?;
        native[8..12].copy_from_slice(&102u32.to_le_bytes());
        std::fs::write(dir.join("wrong.drawer"), native).map_err(|e| e.to_string())?;
        check(
            crate::files::open(&dir.join("wrong.drawer"), 1024 * 1024).is_err(),
            "Legacy native accepted new time metadata",
        )?;
        let rows = catalog(&dir.join("journals"))?;
        check(
            rows == vec![stale.clone()],
            "Local time recovery catalog lost record",
        )?;
        let output = dir.join("time.json");
        export(&stale, &output)?;
        check(
            export(&stale, &output).is_err() && decode(&read(&output)?)? == stale,
            "Time export overwrote existing file",
        )?;
        let saved = std::fs::read(&j.path).map_err(|e| e.to_string())?;
        std::fs::write(&j.path, b"foreign").map_err(|e| e.to_string())?;
        check(
            j.save(&stale).is_err() && std::fs::read(&j.path).unwrap() == b"foreign",
            "Foreign time journal overwritten",
        )?;
        std::fs::write(&j.path, &saved).map_err(|e| e.to_string())?;
        drop(j);
        let mut conflict = stale.clone();
        conflict.snapshot.milliseconds += 1;
        check(
            Journal::open(&dir.join("journals"), &mut conflict).is_err(),
            "Same-sequence conflict not rejected",
        )?;
        check(
            std::fs::read_dir(&dir)
                .map_err(|e| e.to_string())?
                .all(|e| {
                    !e.unwrap()
                        .file_name()
                        .to_string_lossy()
                        .starts_with(".Drawer-")
                }),
            "Time staging residue",
        )?;
        Ok(())
    })();
    // This directory was exclusively created above and contains only these owned fixtures.
    let cleanup = std::fs::remove_dir_all(&dir).map_err(|e| e.to_string());
    result.and(cleanup)
}

#[cfg(test)]
#[path = "work_time_tests.rs"]
mod tests;
