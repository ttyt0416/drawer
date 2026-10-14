use crate::{storage::UserStorage, view::View};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{self, Write},
    path::Path,
};
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub version: u32,
    pub zoom: f32,
    /// Pan is stored in device pixels, independent of UI/monitor scale.
    pub default_view: Option<View>,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            version: 1,
            zoom: 1.0,
            default_view: None,
        }
    }
}
impl Settings {
    pub fn valid(self) -> bool {
        self.version == 1
            && (0.5..=2.0).contains(&self.zoom)
            && self.default_view.is_none_or(|v| {
                v.zoom.is_finite()
                    && (0.01..=64.0).contains(&v.zoom)
                    && v.angle.is_finite()
                    && v.pan.iter().all(|n| n.is_finite() && n.abs() <= 16e9)
            })
    }
    pub fn read(path: &Path) -> io::Result<Self> {
        if fs::metadata(path)?.len() > 65536 {
            return Err(io::Error::other("Interface settings too large"));
        }
        let s: Self = serde_json::from_slice(&fs::read(path)?).map_err(io::Error::other)?;
        if !s.valid() {
            return Err(io::Error::other("Invalid interface settings"));
        }
        Ok(s)
    }
    pub fn load(storage: &UserStorage) -> io::Result<Self> {
        match Self::read(&storage.root().join("settings/interface.json")) {
            Ok(s) => Ok(s),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e),
        }
    }
    pub fn save(self, storage: &UserStorage) -> io::Result<()> {
        if !self.valid() {
            return Err(io::Error::other("Invalid interface settings"));
        }
        let path = storage.root().join("settings/interface.json");
        if path.exists() {
            if fs::metadata(&path)?.len() > 65536 {
                return Err(io::Error::other("Interface settings exceed recovery limit"));
            }
            let backup = if Self::read(&path).is_ok() {
                "interface.previous.json"
            } else {
                "interface.damaged.json"
            };
            atomic(
                &storage.root().join("settings").join(backup),
                &fs::read(&path)?,
            )?;
        }
        atomic(&path, &serde_json::to_vec(&self).map_err(io::Error::other)?)
    }
}
pub fn rebase(view: &mut View, before: f64, after: f64) {
    if !before.is_finite() || !after.is_finite() || before <= 0.0 || after <= 0.0 {
        return;
    }
    view.pan = [view.pan[0] * before / after, view.pan[1] * before / after];
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
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn monitor_and_interface_scaling_preserve_document_center_rotation_zoom_and_mirror() {
        let mut view = View {
            zoom: 2.1,
            angle: 37.0,
            pan: [43.0, -18.0],
            flipped: true,
        };
        let original = view;
        let size = [800.0, 600.0];
        let point = view.screen_to_document([0.0; 2], size, [0.0; 2], 1.0);
        let mut p = 1.0;
        for next in [1.5, 2.0, 0.5, 4.0, 1.0] {
            rebase(&mut view, p, next);
            p = next;
            let actual = view.screen_to_document([0.0; 2], size, [0.0; 2], next);
            assert!((actual[0] - point[0]).abs() < 1e-9 && (actual[1] - point[1]).abs() < 1e-9);
            assert_eq!(
                (view.zoom, view.angle, view.flipped),
                (original.zoom, original.angle, original.flipped)
            );
        }
        assert_eq!(view, original);
    }
}
