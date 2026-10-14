use crate::{display_view::Framing, storage::UserStorage};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
};
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SavedView {
    pub tab: u64,
    pub document: u64,
    pub session: u64,
    pub path: Option<PathBuf>,
    pub title: String,
    pub framing: Framing,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Layout {
    pub version: u32,
    pub rows: Vec<SavedView>,
    pub active: Option<u64>,
}
impl Layout {
    pub fn empty() -> Self {
        Self {
            version: 1,
            ..Default::default()
        }
    }
    pub fn valid(&self) -> bool {
        self.version == 1
            && self.rows.len() <= 256
            && self.rows.iter().all(|v| {
                v.tab > 0
                    && v.document > 0
                    && v.title.len() <= 4096
                    && v.framing.valid()
                    && v.path.as_ref().is_none_or(|p| p.is_absolute())
            })
            && self
                .rows
                .iter()
                .map(|v| v.tab)
                .collect::<HashSet<_>>()
                .len()
                == self.rows.len()
            && self
                .active
                .is_none_or(|id| self.rows.iter().any(|v| v.tab == id))
    }
    pub fn read(path: &Path) -> io::Result<Self> {
        if fs::metadata(path)?.len() > 1024 * 1024 {
            return Err(io::Error::other("View layout too large"));
        }
        let s: Self = serde_json::from_slice(&fs::read(path)?).map_err(io::Error::other)?;
        if !s.valid() {
            return Err(io::Error::other("Invalid view layout"));
        }
        Ok(s)
    }
    pub fn load(storage: &UserStorage) -> io::Result<Self> {
        match Self::read(&storage.root().join("settings/views.json")) {
            Ok(s) => Ok(s),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Self::empty()),
            Err(e) => Err(e),
        }
    }
    pub fn save(&self, storage: &UserStorage) -> io::Result<()> {
        if !self.valid() {
            return Err(io::Error::other("Invalid view layout"));
        }
        let path = storage.root().join("settings/views.json");
        if path.exists() {
            if fs::metadata(&path)?.len() > 1024 * 1024 {
                return Err(io::Error::other("View layout exceeds recovery limit"));
            }
            let name = if Self::read(&path).is_ok() {
                "views.previous.json"
            } else {
                "views.damaged.json"
            };
            atomic(
                &storage.root().join("settings").join(name),
                &fs::read(&path)?,
            )?;
        }
        atomic(&path, &serde_json::to_vec(self).map_err(io::Error::other)?)
    }
}
fn atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let temporary = path.with_extension("json.write.tmp");
    let mut file = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temporary)?;
    let result = (|| {
        file.write_all(bytes)?;
        file.sync_all()
    })();
    drop(file);
    if let Err(e) = result.and_then(|_| fs::rename(&temporary, path)) {
        let _ = fs::remove_file(temporary);
        return Err(e);
    }
    Ok(())
}
