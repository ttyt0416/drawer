use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{self, Read, Seek, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    pub history: crate::history_limits::Settings,
    pub version: u32,
    pub scratch: Option<PathBuf>,
    pub retained: Vec<PathBuf>,
    pub ram_mib: u32,
    pub max_dimension: u32,
    pub recovery_mib: u32,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            history: Default::default(),
            version: 1,
            scratch: None,
            retained: vec![],
            ram_mib: 256,
            max_dimension: 32768,
            recovery_mib: 512,
        }
    }
}
impl Settings {
    pub fn valid(&self) -> Result<(), String> {
        if !self.history.valid()
            || self.version != 1
            || !(64..=256).contains(&self.ram_mib)
            || !(1..=32768).contains(&self.max_dimension)
            || !(16..=4096).contains(&self.recovery_mib)
            || self.retained.len() > 16
        {
            return Err("Resource ranges: RAM64..256MiB; dimension1..32768; recovery16..4096MiB; retained roots≤16".into());
        }
        for p in self.scratch.iter().chain(&self.retained) {
            crate::file_browser::lexical(p)?;
        }
        Ok(())
    }
    pub fn budget(&self) -> usize {
        self.ram_mib as usize * 1024 * 1024
    }
    pub fn root<'a>(&'a self, storage: &'a crate::storage::UserStorage) -> &'a Path {
        self.scratch.as_deref().unwrap_or(storage.root())
    }
    pub fn read(path: &Path) -> io::Result<Self> {
        let path = crate::file_browser::checked(path).map_err(io::Error::other)?;
        if fs::metadata(&path)?.len() > 65536 {
            return Err(io::Error::other("Resource settings limit64KiB"));
        }
        let s: Self = serde_json::from_slice(&fs::read(path)?).map_err(io::Error::other)?;
        s.valid().map_err(io::Error::other)?;
        Ok(s)
    }
    pub fn load(storage: &crate::storage::UserStorage) -> io::Result<Self> {
        let path = storage.root().join("settings/resources.json");
        crate::file_browser::lexical(&path).map_err(io::Error::other)?;
        if !path.try_exists()? {
            return Ok(Self::default());
        }
        Self::read(&path)
    }
    pub fn save(&self, storage: &crate::storage::UserStorage) -> io::Result<()> {
        self.valid().map_err(io::Error::other)?;
        let path = storage.root().join("settings/resources.json");
        let data = serde_json::to_vec(self).map_err(io::Error::other)?;
        crate::atomic_file::publish(
            &path,
            &path.with_file_name("resources-write.tmp"),
            |w| w.write_all(&data),
            |p| Self::read(p).map(|_| ()),
            &mut |_| Ok(()),
        )?;
        Ok(())
    }
}
/// The platform queries the real local drive before calling this deterministic admission/probe.
pub fn probe(root: &Path, available: u64) -> Result<(), String> {
    if available < 1024 * 1024 {
        return Err(format!(
            "Scratch needs at least1048576 free bytes; available {available}. Previous settings preserved"
        ));
    }
    let root = crate::file_browser::folder(root)?;
    static NEXT: AtomicU64 = AtomicU64::new(1);
    let nonce = format!(
        "Drawer scratch probe {} {}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    );
    let path = root.join(format!(
        ".drawer-probe-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let mut f = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|e| format!("Scratch is not writable: {e}"))?;
    let result = (|| -> io::Result<()> {
        f.write_all(nonce.as_bytes())?;
        f.sync_all()?;
        f.rewind()?;
        let mut data = String::new();
        f.read_to_string(&mut data)?;
        if data != nonce {
            return Err(io::Error::other("Scratch verification failed"));
        }
        Ok(())
    })();
    drop(f);
    let removed = fs::remove_file(path);
    result.map_err(|e| e.to_string())?;
    removed.map_err(|e| e.to_string())
}
/// Never clean unresolved snapshots to satisfy a quota. Stop and ask for a recovery decision.
pub fn recovery_usage(root: &Path) -> Result<u64, String> {
    let base = root.join("recovery");
    if !base.exists() {
        return Ok(0);
    }
    let mut todo = vec![(base, 0)];
    let mut bytes = 0u64;
    let mut count = 0usize;
    while let Some((p, depth)) = todo.pop() {
        if p.file_name().is_some_and(|n| n == "live.lock") {
            continue;
        }
        count += 1;
        if count > 100000 || depth > 32 {
            return Err("Recovery inventory limit reached; files preserved".into());
        }
        let p = crate::file_browser::checked(&p)?;
        let m = fs::metadata(&p).map_err(|e| e.to_string())?;
        if m.is_dir() {
            for e in fs::read_dir(p).map_err(|e| e.to_string())? {
                todo.push((e.map_err(|e| e.to_string())?.path(), depth + 1));
            }
        } else if m.is_file() {
            bytes = bytes
                .checked_add(m.len())
                .ok_or("Recovery usage overflow")?;
        } else {
            return Err("Unknown recovery entry preserved".into());
        }
    }
    Ok(bytes)
}
#[path = "resource_settings_verify.rs"]
pub mod verify;
