//! Application-local display calibration. Never part of document or export state.
use crate::{soft_proof::Proof, storage::UserStorage};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::Arc,
};
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub version: u32,
    pub profiles: [Option<PathBuf>; 2],
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            version: 1,
            profiles: [None, None],
        }
    }
}
impl Settings {
    pub fn valid(&self) -> bool {
        self.version == 1
            && self
                .profiles
                .iter()
                .flatten()
                .all(|p| !p.as_os_str().is_empty() && p.as_os_str().len() <= 4096)
    }
    pub fn prepare(&self) -> Result<[Option<Arc<Proof>>; 2], String> {
        if !self.valid() {
            return Err("Invalid monitor color settings".into());
        }
        let read = |path: &Option<PathBuf>| -> Result<Option<Arc<Proof>>, String> {
            path.as_ref()
                .map(|p| {
                    read_profile(p)
                        .and_then(|b| Proof::monitor_from_bytes(&b))
                        .map(Arc::new)
                })
                .transpose()
        };
        Ok([read(&self.profiles[0])?, read(&self.profiles[1])?])
    }
    pub fn load(storage: &UserStorage) -> Result<Self, String> {
        let path = storage.root().join("settings/monitor-color.json");
        match fs::symlink_metadata(&path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e.to_string()),
            Ok(m) => {
                if !m.is_file() || m.file_type().is_symlink() || m.len() > 32768 {
                    return Err("Invalid monitor settings file".into());
                }
                let s: Self = serde_json::from_slice(&fs::read(path).map_err(|e| e.to_string())?)
                    .map_err(|e| e.to_string())?;
                if !s.valid() {
                    return Err("Invalid monitor settings version/path".into());
                }
                Ok(s)
            }
        }
    }
    pub fn save(&self, storage: &UserStorage) -> Result<(), String> {
        if !self.valid() {
            return Err("Invalid monitor settings".into());
        }
        self.prepare()?;
        let path = storage.root().join("settings/monitor-color.json");
        if fs::symlink_metadata(&path).is_ok_and(|m| !m.is_file() || m.file_type().is_symlink()) {
            return Err("Monitor destination must be a regular file".into());
        }
        let temp = path.with_extension("json.write.tmp");
        let mut f = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temp)
            .map_err(|e| e.to_string())?;
        let result = (|| {
            f.write_all(&serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?;
            f.sync_all().map_err(|e| e.to_string())?;
            drop(f);
            fs::rename(&temp, &path).map_err(|e| e.to_string())
        })();
        let _ = fs::remove_file(temp);
        result
    }
}
pub fn read_profile(path: &Path) -> Result<Vec<u8>, String> {
    let m = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if !m.is_file() || m.file_type().is_symlink() || m.len() > 4 * 1024 * 1024 {
        return Err("Choose a regular RGB monitor ICC up to 4 MiB".into());
    }
    let path = crate::file_browser::checked(path)?;
    if !crate::font_library::local(&path) {
        return Err("Choose a local monitor ICC".into());
    }
    let mut bytes = vec![];
    fs::File::open(path)
        .map_err(|e| e.to_string())?
        .take(4 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > 4 * 1024 * 1024 {
        return Err("Monitor ICC exceeds 4 MiB".into());
    }
    Ok(bytes)
}
#[cfg(test)]
mod tests {
    #[test]
    fn monitor_is_one_way_rgb_and_preserves_alpha_and_saved_settings() {
        use crate::{
            color_precision::Sample,
            color_profile::{RgbProfile, Transform},
        };
        let t = crate::test_support::TempDir::new();
        let storage = crate::storage::UserStorage::at(t.0.join("user")).unwrap();
        let path = t.0.join("monitor.icc");
        let bytes = crate::color_precision_verify::display_p3_fixture();
        std::fs::write(&path, &bytes).unwrap();
        let s = super::Settings {
            profiles: [Some(path.clone()), None],
            ..Default::default()
        };
        let stages = s.prepare().unwrap();
        let expected = Transform::new(
            &RgbProfile::Srgb,
            &RgbProfile::embedded("monitor".into(), bytes).unwrap(),
        )
        .unwrap()
        .map(Sample::from8([224, 32, 80, 128]))
        .unwrap()
        .rgba8();
        let shown = stages[0].as_ref().unwrap().map([224, 32, 80, 128]);
        assert!(shown.iter().zip(expected).all(|(a, b)| a.abs_diff(b) <= 1));
        assert_eq!(shown[3], 128);
        assert_ne!(shown, [224, 32, 80, 128]);
        s.save(&storage).unwrap();
        assert_eq!(s, super::Settings::load(&storage).unwrap());
        std::fs::write(&path, b"broken").unwrap();
        assert!(s.prepare().is_err());
        assert!(s.save(&storage).is_err());
        assert_eq!(s, super::Settings::load(&storage).unwrap());
        assert!(
            crate::soft_proof::Proof::monitor_from_bytes(&crate::soft_proof::gray_fixture())
                .is_err()
        );
    }
}
