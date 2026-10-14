use crate::storage::UserStorage;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{self, Write},
    path::Path,
};
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Preferences {
    pub version: u32,
    pub visible: bool,
    pub floating: bool,
    pub size: [f32; 2],
    pub position: [f32; 2],
    pub min_width: f32,
    pub max_width: f32,
    pub hover_outline: bool,
    pub selection_outline: bool,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            version: 1,
            visible: true,
            floating: false,
            size: [160.0; 2],
            position: [40.0, 160.0],
            min_width: 80.0,
            max_width: 480.0,
            hover_outline: false,
            selection_outline: true,
        }
    }
}
impl Preferences {
    pub fn valid(self) -> bool {
        self.version == 1
            && (80.0..=640.0).contains(&self.min_width)
            && (self.min_width..=640.0).contains(&self.max_width)
            && (self.min_width..=self.max_width).contains(&self.size[0])
            && (80.0..=640.0).contains(&self.size[1])
            && self
                .position
                .iter()
                .all(|n| n.is_finite() && n.abs() <= 1e6)
    }
    pub fn outline_visible(self, hover: bool, focus: bool) -> bool {
        !self.hover_outline || hover || focus
    }
    pub fn read(path: &Path) -> io::Result<Self> {
        if fs::metadata(path)?.len() > 65536 {
            return Err(io::Error::other("Navigator preferences are too large"));
        }
        let prefs: Self = serde_json::from_slice(&fs::read(path)?).map_err(io::Error::other)?;
        if !prefs.valid() {
            return Err(io::Error::other("Invalid navigator preferences"));
        }
        Ok(prefs)
    }
    pub fn load(storage: &UserStorage) -> io::Result<Self> {
        match Self::read(&storage.root().join("settings/navigator.json")) {
            Ok(p) => Ok(p),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e),
        }
    }
    pub fn save(self, storage: &UserStorage) -> io::Result<()> {
        if !self.valid() {
            return Err(io::Error::other("Invalid navigator preferences"));
        }
        let path = storage.root().join("settings/navigator.json");
        if path.exists() {
            if fs::metadata(&path)?.len() > 65536 {
                return Err(io::Error::other("Navigator settings exceed recovery limit"));
            }
            let backup = if Self::read(&path).is_ok() {
                "navigator.previous.json"
            } else {
                "navigator.damaged.json"
            };
            atomic(
                &storage.root().join("settings").join(backup),
                &fs::read(&path)?,
            )?;
        }
        atomic(&path, &serde_json::to_vec(&self).map_err(io::Error::other)?)
    }
}
fn atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let temp = path.with_extension("json.write.tmp");
    let mut f = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temp)?;
    let result = (|| {
        f.write_all(bytes)?;
        f.sync_all()
    })();
    drop(f);
    if let Err(e) = result.and_then(|_| fs::rename(&temp, path)) {
        let _ = fs::remove_file(temp);
        return Err(e);
    }
    Ok(())
}
