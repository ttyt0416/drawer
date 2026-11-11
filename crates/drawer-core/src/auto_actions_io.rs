use super::Settings;
use std::{
    fs,
    io::{Read, Write},
    path::Path,
};
const LIMIT: usize = 2 * 1024 * 1024;
impl Settings {
    pub fn decode(bytes: &[u8]) -> Result<Self, String> {
        if crate::exchange_guidance::private_container(bytes).is_some() {
            return Err(crate::exchange_guidance::GUIDE.into());
        }
        if bytes.len() > LIMIT {
            return Err("Auto action file exceeds 2 MiB".into());
        }
        let s: Self = serde_json::from_slice(bytes)
            .map_err(|e| format!("Invalid or unsupported auto action file: {e}"))?;
        if !s.valid() {
            return Err("Invalid auto action version, IDs, names or capacity".into());
        }
        Ok(s)
    }
    pub fn read(path: &Path) -> Result<Self, String> {
        crate::exchange_guidance::reject_private(path)?;
        let m = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
        if !m.is_file() || m.file_type().is_symlink() || m.len() > LIMIT as u64 {
            return Err("Choose a regular auto action file up to 2 MiB".into());
        }
        let path = crate::file_browser::checked(path)?;
        if !crate::font_library::local(&path) {
            return Err("Choose a local auto action file".into());
        }
        let mut bytes = vec![];
        fs::File::open(path)
            .map_err(|e| e.to_string())?
            .take(LIMIT as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        Self::decode(&bytes)
    }
    pub fn write(&self, path: &Path, replace: bool) -> Result<(), String> {
        crate::exchange_guidance::reject_private(path)?;
        if !self.valid() {
            return Err("Invalid auto action settings".into());
        }
        let bytes = serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?;
        if bytes.len() > LIMIT {
            return Err("Auto action file exceeds 2 MiB".into());
        }
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."))
            .canonicalize()
            .map_err(|e| e.to_string())?;
        if !crate::font_library::local(&parent) {
            return Err("Choose a local auto action folder".into());
        }
        let path = parent.join(path.file_name().ok_or("Missing auto action filename")?);
        let check = || -> Result<(), String> {
            match fs::symlink_metadata(&path) {
                Ok(m) => {
                    if !m.is_file() || m.file_type().is_symlink() {
                        return Err("Choose a regular action destination".into());
                    }
                    if !replace {
                        return Err("Confirm replacement of the existing action file".into());
                    }
                    Ok(())
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(e) => Err(e.to_string()),
            }
        };
        check()?;
        let mut name = path.file_name().unwrap().to_owned();
        name.push(".actions-write.tmp");
        let temp = path.with_file_name(name);
        let mut f = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temp)
            .map_err(|e| e.to_string())?;
        let result = (|| {
            f.write_all(&bytes).map_err(|e| e.to_string())?;
            f.sync_all().map_err(|e| e.to_string())?;
            drop(f);
            check()?;
            if replace {
                fs::rename(&temp, &path).map_err(|e| e.to_string())
            } else {
                fs::hard_link(&temp, &path).map_err(|e| e.to_string())
            }
        })();
        let _ = fs::remove_file(temp);
        result
    }
    pub fn load(storage: &crate::storage::UserStorage) -> Result<Self, String> {
        let path = storage.root().join("settings/auto-actions.json");
        match fs::symlink_metadata(&path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e.to_string()),
            Ok(_) => Self::read(&path),
        }
    }
    pub fn save(&self, storage: &crate::storage::UserStorage) -> Result<(), String> {
        self.write(&storage.root().join("settings/auto-actions.json"), true)
    }
}
