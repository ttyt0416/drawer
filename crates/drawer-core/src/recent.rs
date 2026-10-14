use crate::storage::UserStorage;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
};
const LIMIT: usize = 20;
const MAX_BYTES: u64 = 65536;
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct Settings {
    version: u32,
    paths: Vec<PathBuf>,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            version: 1,
            paths: Vec::new(),
        }
    }
}
#[derive(Default)]
pub struct RecentFiles {
    settings: Settings,
    undo: Vec<Settings>,
    redo: Vec<Settings>,
}
fn read(path: &Path) -> io::Result<Settings> {
    if fs::metadata(path)?.len() > MAX_BYTES {
        return Err(io::Error::other("Recent file settings are too large"));
    }
    let settings: Settings = serde_json::from_slice(&fs::read(path)?).map_err(io::Error::other)?;
    if settings.version != 1
        || settings.paths.len() > LIMIT
        || settings.paths.iter().any(|p| !p.is_absolute())
        || settings.paths.iter().collect::<HashSet<_>>().len() != settings.paths.len()
    {
        return Err(io::Error::other("Invalid recent file settings"));
    }
    Ok(settings)
}
fn write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let temporary = path.with_extension("json.write.tmp");
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)?;
    let result = (|| {
        file.write_all(bytes)?;
        file.sync_all()
    })();
    drop(file);
    if let Err(error) = result.and_then(|_| fs::rename(&temporary, path)) {
        let _ = fs::remove_file(temporary);
        return Err(error);
    }
    Ok(())
}
impl RecentFiles {
    pub fn load(storage: &UserStorage) -> io::Result<Self> {
        match read(&storage.root().join("settings/recent.json")) {
            Ok(settings) => Ok(Self {
                settings,
                ..Default::default()
            }),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(error),
        }
    }
    pub fn paths(&self) -> &[PathBuf] {
        &self.settings.paths
    }
    fn apply(&mut self, settings: Settings, storage: Option<&UserStorage>) -> io::Result<()> {
        if settings == self.settings {
            return Ok(());
        }
        if let Some(storage) = storage {
            let path = storage.root().join("settings/recent.json");
            if path.exists() {
                if fs::metadata(&path)?.len() > MAX_BYTES {
                    return Err(io::Error::other(
                        "Recent settings exceed the recovery limit; original preserved",
                    ));
                }
                let name = if read(&path).is_ok() {
                    "recent.previous.json"
                } else {
                    "recent.damaged.json"
                };
                write(
                    &storage.root().join("settings").join(name),
                    &fs::read(&path)?,
                )?;
            }
            let bytes = serde_json::to_vec(&settings).map_err(io::Error::other)?;
            if bytes.len() as u64 > MAX_BYTES {
                return Err(io::Error::other("Recent file paths are too long"));
            }
            write(&path, &bytes)?;
        }
        self.undo
            .push(std::mem::replace(&mut self.settings, settings));
        if self.undo.len() > LIMIT {
            self.undo.remove(0);
        }
        self.redo.clear();
        Ok(())
    }
    pub fn record(&mut self, path: &Path, storage: Option<&UserStorage>) -> io::Result<()> {
        let path = path.canonicalize()?;
        let mut next = self.settings.clone();
        next.paths.retain(|p| p != &path);
        next.paths.insert(0, path);
        next.paths.truncate(LIMIT);
        self.apply(next, storage)
    }
    pub fn reset(&mut self, storage: Option<&UserStorage>) -> io::Result<()> {
        self.apply(Settings::default(), storage)
    }
    pub fn restore(&mut self, storage: &UserStorage) -> io::Result<()> {
        self.apply(
            read(&storage.root().join("settings/recent.previous.json"))?,
            Some(storage),
        )
    }
    pub fn counts(&self) -> (usize, usize) {
        (self.undo.len(), self.redo.len())
    }
    pub fn undo(&mut self, storage: Option<&UserStorage>) -> io::Result<()> {
        let Some(next) = self.undo.last().cloned() else {
            return Ok(());
        };
        let mut history = self.undo.clone();
        history.pop();
        let mut redo = self.redo.clone();
        redo.push(self.settings.clone());
        self.apply(next, storage)?;
        self.undo = history;
        self.redo = redo;
        Ok(())
    }
    pub fn redo(&mut self, storage: Option<&UserStorage>) -> io::Result<()> {
        let Some(next) = self.redo.last().cloned() else {
            return Ok(());
        };
        let mut redo = self.redo.clone();
        redo.pop();
        let mut undo = self.undo.clone();
        undo.push(self.settings.clone());
        self.apply(next, storage)?;
        self.undo = undo;
        self.redo = redo;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TempDir;
    #[test]
    fn recent_order_dedup_restart_missing_undo_backup_and_failed_write() {
        let root = TempDir::new();
        let storage = UserStorage::at(root.0.join("userdata")).unwrap();
        let mut recent = RecentFiles::load(&storage).unwrap();
        for n in 0..22 {
            let path = root.0.join(format!("{n}.drawer"));
            fs::write(&path, "fixture").unwrap();
            recent.record(&path, Some(&storage)).unwrap();
        }
        assert_eq!(recent.paths().len(), 20);
        let latest = recent.paths()[0].clone();
        recent
            .record(&root.0.join("5.drawer"), Some(&storage))
            .unwrap();
        assert_eq!(
            recent.paths()[0],
            root.0.join("5.drawer").canonicalize().unwrap()
        );
        assert_eq!(recent.paths()[1], latest);
        let before = recent.paths().to_vec();
        fs::remove_file(&latest).unwrap();
        assert_eq!(RecentFiles::load(&storage).unwrap().paths(), before);
        recent.reset(Some(&storage)).unwrap();
        assert!(recent.paths().is_empty());
        recent.undo(Some(&storage)).unwrap();
        assert_eq!(recent.paths(), before);
        recent.redo(Some(&storage)).unwrap();
        assert!(recent.paths().is_empty());
        recent.restore(&storage).unwrap();
        assert_eq!(recent.paths(), before);
        let temporary = storage.root().join("settings/recent.json.write.tmp");
        fs::write(&temporary, "foreign operation").unwrap();
        assert!(recent.reset(Some(&storage)).is_err());
        assert_eq!(recent.paths(), before);
        assert_eq!(RecentFiles::load(&storage).unwrap().paths(), before);
        assert_eq!(fs::read_to_string(temporary).unwrap(), "foreign operation");
    }
    #[test]
    fn damaged_settings_are_reported_and_preserved_before_recovery() {
        let root = TempDir::new();
        let storage = UserStorage::at(root.0.join("userdata")).unwrap();
        let path = storage.root().join("settings/recent.json");
        fs::write(&path, b"damaged").unwrap();
        assert!(RecentFiles::load(&storage).is_err());
        let input = root.0.join("image.png");
        fs::write(&input, "fixture").unwrap();
        RecentFiles::default()
            .record(&input, Some(&storage))
            .unwrap();
        assert_eq!(
            fs::read(storage.root().join("settings/recent.damaged.json")).unwrap(),
            b"damaged"
        );
        assert_eq!(RecentFiles::load(&storage).unwrap().paths().len(), 1);
    }
}
