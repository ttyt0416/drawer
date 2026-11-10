//! Registered local works. Metadata/history are separate from drawing documents.
use crate::{asset_catalog as assets, file_browser as browser, storage::UserStorage};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
};
const HEADER: &[u8] = b"Drawer work library v1\n";
pub const DB_FILE: &str = "work-library.db";
pub const INDEX_FILE: &str = "work-library.index";
const PREVIOUS: &str = "work-library.previous.db";
const LIMIT: usize = 8 * 1024 * 1024;
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Sort {
    #[default]
    Title,
    Path,
    Modified,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub id: u64,
    pub path: PathBuf,
    pub title: String,
    pub labels: Vec<String>,
    pub favorite: bool,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct State {
    pub entries: Vec<Entry>,
    pub sort: Sort,
    pub descending: bool,
    pub favorites_only: bool,
    pub path_labels: bool,
}
fn text(s: &str, limit: usize) -> bool {
    s.len() <= limit && !s.chars().any(char::is_control)
}
fn same(a: &Path, b: &Path) -> bool {
    if cfg!(windows) {
        a.to_string_lossy()
            .eq_ignore_ascii_case(&b.to_string_lossy())
    } else {
        a == b
    }
}
impl State {
    fn valid(&self, next: u64) -> bool {
        self.entries.len() <= 512
            && self.entries.iter().enumerate().all(|(n, e)| {
                e.id > 0
                    && e.id < next
                    && browser::lexical(&e.path).is_ok()
                    && e.path.to_str().is_some_and(|s| s.len() <= 4096)
                    && text(&e.title, 512)
                    && !e.title.trim().is_empty()
                    && e.labels.len() <= 16
                    && e.labels.iter().all(|s| text(s, 64) && !s.trim().is_empty())
                    && !self.entries[..n]
                        .iter()
                        .any(|v| v.id == e.id || same(&v.path, &e.path))
            })
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Change {
    label: String,
    state: State,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Database {
    version: u32,
    pub generation: u64,
    next_id: u64,
    pub state: State,
    undo: Vec<Change>,
    redo: Vec<Change>,
    #[serde(skip)]
    stamp: Option<String>,
}
impl Default for Database {
    fn default() -> Self {
        Self {
            version: 1,
            generation: 0,
            next_id: 1,
            state: State::default(),
            undo: vec![],
            redo: vec![],
            stamp: None,
        }
    }
}
pub(super) fn managed(s: &UserStorage, name: &str) -> Result<PathBuf, String> {
    assets::checked(
        &browser::folder(s.root())?,
        &Path::new("settings").join(name),
        false,
    )
}
pub(super) fn raw(s: &UserStorage, name: &str, limit: usize) -> Result<Option<Vec<u8>>, String> {
    let p = managed(s, name)?;
    let m = match fs::symlink_metadata(&p) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.to_string()),
    };
    if !m.is_file() || assets::linked(&m) || m.len() > limit as u64 {
        return Err("Local library file type/size limit; original preserved".into());
    }
    let mut b = vec![];
    fs::File::open(p)
        .map_err(|e| e.to_string())?
        .take(limit as u64 + 1)
        .read_to_end(&mut b)
        .map_err(|e| e.to_string())?;
    if b.len() > limit {
        return Err("Library file grew beyond its limit".into());
    }
    Ok(Some(b))
}
pub(super) fn lease(s: &UserStorage) -> Result<fs::File, String> {
    let p = managed(s, "work-library.lock")?;
    let f = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(p)
        .map_err(|e| e.to_string())?;
    f.try_lock()
        .map_err(|e| format!("Library is being changed by another session: {e}"))?;
    Ok(f)
}
pub(super) fn atomic(
    s: &UserStorage,
    name: &str,
    bytes: &[u8],
    header: &[u8],
    c: &AtomicBool,
    before: impl FnOnce() -> Result<(), String>,
) -> Result<(), String> {
    let p = managed(s, name)?;
    let temp = managed(s, &format!("{name}.write.tmp"))?;
    if let Some(b) = raw(s, &format!("{name}.write.tmp"), 32 * 1024 * 1024)? {
        if !b.starts_with(header) {
            return Err("Unrecognized interrupted library staging file preserved".into());
        }
        fs::remove_file(&temp).map_err(|e| e.to_string())?;
    }
    if fs::symlink_metadata(&p).is_ok_and(|m| m.permissions().readonly()) {
        return Err("Library is read-only; previous state retained".into());
    }
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)
        .map_err(|e| e.to_string())?;
    let result = (|| {
        for chunk in bytes.chunks(65536) {
            assets::canceled(c)?;
            f.write_all(chunk).map_err(|e| e.to_string())?;
        }
        f.sync_all().map_err(|e| e.to_string())?;
        drop(f);
        assets::canceled(c)?;
        before()?;
        fs::rename(&temp, &p).map_err(|e| e.to_string())
    })();
    let _ = fs::remove_file(temp);
    result
}
impl Database {
    fn valid(&self) -> bool {
        self.version == 1
            && self.next_id > 0
            && self.state.valid(self.next_id)
            && self.undo.len() <= 32
            && self.redo.len() <= 32
            && self
                .undo
                .iter()
                .chain(&self.redo)
                .all(|v| text(&v.label, 128) && v.state.valid(self.next_id))
    }
    fn decode(b: &[u8]) -> Result<Self, String> {
        if b.len() > LIMIT || !b.starts_with(HEADER) {
            return Err("Damaged/unsupported work library; original retained".into());
        }
        let v: Self = serde_json::from_slice(&b[HEADER.len()..]).map_err(|e| e.to_string())?;
        if !v.valid() {
            return Err("Invalid library version, identities, paths or recovery history".into());
        }
        Ok(v)
    }
    fn encode(&mut self) -> Result<Vec<u8>, String> {
        if !self.valid() {
            return Err("Invalid library update; previous state retained".into());
        }
        loop {
            let mut b = HEADER.to_vec();
            b.extend(serde_json::to_vec(self).map_err(|e| e.to_string())?);
            if b.len() <= LIMIT {
                return Ok(b);
            }
            if !self.undo.is_empty() {
                self.undo.remove(0);
            } else if !self.redo.is_empty() {
                self.redo.remove(0);
            } else {
                return Err("Library metadata size limit".into());
            }
        }
    }
    pub fn load(s: &UserStorage) -> Result<Self, String> {
        match raw(s, DB_FILE, LIMIT)? {
            None => Ok(Self::default()),
            Some(b) => {
                let mut d = Self::decode(&b)?;
                d.stamp = Some(assets::hash(&b));
                Ok(d)
            }
        }
    }
    pub(super) fn check_disk(&self, s: &UserStorage) -> Result<(), String> {
        let current = raw(s, DB_FILE, LIMIT)?.map(|b| assets::hash(&b));
        if current != self.stamp {
            return Err("Library changed on disk; reopen it before changing metadata".into());
        }
        Ok(())
    }
    fn publish(mut self, s: &UserStorage) -> Result<Self, String> {
        let _lease = lease(s)?;
        self.check_disk(s)?;
        let bytes = self.encode()?;
        let c = AtomicBool::new(false);
        if let Some(old) = raw(s, DB_FILE, LIMIT)?
            && Self::decode(&old).is_ok()
        {
            atomic(s, PREVIOUS, &old, HEADER, &c, || self.check_disk(s))?;
        }
        atomic(s, DB_FILE, &bytes, HEADER, &c, || self.check_disk(s))?;
        self.stamp = Some(assets::hash(&bytes));
        Ok(self)
    }
    pub fn edit(
        &self,
        s: &UserStorage,
        label: &str,
        work: impl FnOnce(&mut State, &mut u64) -> Result<(), String>,
    ) -> Result<Self, String> {
        let mut d = self.clone();
        work(&mut d.state, &mut d.next_id)?;
        if d.state == self.state {
            return Ok(self.clone());
        }
        d.generation = d
            .generation
            .checked_add(1)
            .ok_or("Library generation limit")?;
        d.undo.push(Change {
            label: label.into(),
            state: self.state.clone(),
        });
        if d.undo.len() > 32 {
            d.undo.remove(0);
        }
        d.redo.clear();
        d.publish(s)
    }
    pub fn counts(&self) -> (usize, usize) {
        (self.undo.len(), self.redo.len())
    }
    pub fn history(&self, redo: bool) -> Option<&str> {
        if redo {
            self.redo.last()
        } else {
            self.undo.last()
        }
        .map(|v| v.label.as_str())
    }
    pub fn undo(&self, s: &UserStorage, redo: bool) -> Result<Self, String> {
        let mut d = self.clone();
        let change = if redo { d.redo.pop() } else { d.undo.pop() }
            .ok_or("No library metadata change to restore")?;
        let reverse = Change {
            label: change.label,
            state: d.state,
        };
        d.state = change.state;
        if redo {
            d.undo.push(reverse);
        } else {
            d.redo.push(reverse);
        }
        d.generation = d
            .generation
            .checked_add(1)
            .ok_or("Library generation limit")?;
        d.publish(s)
    }
    pub fn entry(&self, id: u64) -> Result<&Entry, String> {
        self.state
            .entries
            .iter()
            .find(|e| e.id == id)
            .ok_or("Select a registered work".into())
    }
    pub fn register(&self, s: &UserStorage, paths: &[PathBuf]) -> Result<Self, String> {
        if paths.is_empty() || paths.len() > 64 {
            return Err("Choose 1..64 local files".into());
        }
        let mut prepared = vec![];
        for p in paths {
            let stamp = browser::Stamp::read(p)?;
            if stamp.directory {
                return Err("Register files, not folders".into());
            }
            prepared.push(stamp.path);
        }
        self.edit(s, "Register works", move |state, next| {
            for path in prepared {
                if state.entries.iter().any(|e| same(&e.path, &path)) {
                    continue;
                }
                let id = *next;
                *next = next.checked_add(1).ok_or("Library identity limit")?;
                let title = path
                    .file_stem()
                    .and_then(|v| v.to_str())
                    .unwrap_or("Untitled")
                    .chars()
                    .take(128)
                    .collect();
                state.entries.push(Entry {
                    id,
                    path,
                    title,
                    labels: vec![],
                    favorite: false,
                });
            }
            Ok(())
        })
    }
    pub fn info(
        &self,
        s: &UserStorage,
        id: u64,
        title: String,
        labels: Vec<String>,
        favorite: bool,
    ) -> Result<Self, String> {
        self.edit(s, "Edit work information", move |state, _| {
            let e = state
                .entries
                .iter_mut()
                .find(|e| e.id == id)
                .ok_or("Selected work was removed")?;
            e.title = title;
            e.labels = labels;
            e.favorite = favorite;
            Ok(())
        })
    }
    pub fn remove(&self, s: &UserStorage, id: u64) -> Result<Self, String> {
        self.entry(id)?;
        self.edit(s, "Remove from work list", |state, _| {
            state.entries.retain(|e| e.id != id);
            Ok(())
        })
    }
    pub fn reconnect(&self, s: &UserStorage, id: u64, path: &Path) -> Result<Self, String> {
        self.entry(id)?;
        let stamp = browser::Stamp::read(path)?;
        if stamp.directory {
            return Err("Reconnect to a local file".into());
        }
        self.edit(s, "Reconnect work path", move |state, _| {
            state.entries.iter_mut().find(|e| e.id == id).unwrap().path = stamp.path;
            Ok(())
        })
    }
    pub fn signature(&self) -> String {
        assets::hash(
            &serde_json::to_vec(
                &self
                    .state
                    .entries
                    .iter()
                    .map(|e| (e.id, &e.path))
                    .collect::<Vec<_>>(),
            )
            .expect("validated Unicode paths"),
        )
    }
    pub fn recover(s: &UserStorage, empty: bool, confirmed: bool) -> Result<Self, String> {
        if !confirmed {
            return Err("Confirm library recovery; drawing files stay unchanged".into());
        }
        let current = raw(s, DB_FILE, LIMIT)?;
        let mut restored = if empty {
            Self::default()
        } else {
            Self::decode(&raw(s, PREVIOUS, LIMIT)?.ok_or("No previous healthy library snapshot")?)?
        };
        if let Some(b) = &current {
            static SEQUENCE: AtomicU64 = AtomicU64::new(0);
            let name = format!(
                "work-library.preserved-{}-{}.db",
                std::process::id(),
                SEQUENCE.fetch_add(1, Ordering::Relaxed)
            );
            let p = managed(s, &name)?;
            let mut f = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(p)
                .map_err(|e| e.to_string())?;
            f.write_all(b)
                .and_then(|_| f.sync_all())
                .map_err(|e| e.to_string())?;
        }
        let old_generation = current
            .as_deref()
            .and_then(|b| Self::decode(b).ok())
            .map_or(0, |d| d.generation);
        restored.generation = restored
            .generation
            .max(old_generation)
            .checked_add(1)
            .ok_or("Library generation limit")?;
        restored.stamp = current.map(|b| assets::hash(&b));
        restored.publish(s)
    }
}
#[path = "work_index.rs"]
pub mod index;
#[path = "work_library_verify.rs"]
pub mod verify;
