//! Local current-state recovery, independent of the source file and session Undo.
use crate::{
    document::{DEFAULT_DOCUMENT_BUDGET as B, Document},
    save_generation::Page,
};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};
static NEXT: AtomicU64 = AtomicU64::new(1);
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Marker {
    format: String,
    version: u32,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    page: Page,
    title: String,
    original: Option<PathBuf>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Index {
    version: u32,
    generation: String,
    documents: Vec<Entry>,
}
fn read<T: serde::de::DeserializeOwned>(p: &Path) -> Result<T, String> {
    let p = crate::file_browser::checked(p)?;
    let mut data = vec![];
    fs::File::open(p)
        .map_err(|e| e.to_string())?
        .take(1024 * 1024 + 1)
        .read_to_end(&mut data)
        .map_err(|e| e.to_string())?;
    if data.len() > 1024 * 1024 {
        return Err("Recovery metadata limit1MiB".into());
    }
    serde_json::from_slice(&data).map_err(|e| e.to_string())
}
fn owned(path: &Path) -> Result<PathBuf, String> {
    let path = crate::file_browser::folder(path)?;
    let marker: Marker = read(&path.join("owner.json"))?;
    if marker.format != "Drawer current-state recovery"
        || marker.version != 1
        || !path
            .file_name()
            .is_some_and(|n| n.to_string_lossy().starts_with("session-"))
    {
        return Err("Unknown recovery directory; preserved".into());
    }
    Ok(path)
}
fn load_index(path: &Path) -> Result<Index, String> {
    let index: Index = read(path)?;
    if index.version != 1
        || !index.generation.starts_with("generation-")
        || index.generation.len() > 64
        || !index
            .generation
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-')
        || index.documents.len() > 256
    {
        return Err("Unknown/invalid recovery index".into());
    }
    let mut seen = std::collections::BTreeSet::new();
    for e in &index.documents {
        if e.page.id == 0
            || !seen.insert(e.page.id)
            || e.title.len() > 1024
            || e.title.chars().any(char::is_control)
            || e.original
                .as_ref()
                .is_some_and(|p| !crate::font_library::local(p) || p.as_os_str().len() > 4096)
            || !crate::asset_catalog::valid_hash(&e.page.sha256)
            || e.page.bytes > 1024 * 1024 * 1024
        {
            return Err("Invalid recovery document metadata".into());
        }
    }
    Ok(index)
}
fn lease(path: &Path, create: bool) -> std::io::Result<fs::File> {
    use std::io::{Error, ErrorKind};
    fn classify(e: Error) -> Error {
        if cfg!(windows) && matches!(e.raw_os_error(), Some(32 | 33)) {
            Error::new(ErrorKind::WouldBlock, e)
        } else {
            e
        }
    }
    let mut options = fs::OpenOptions::new();
    options.read(true);
    if create {
        options.write(true).create_new(true);
    } else {
        // The caller already checked the owning directory. Keep OS sharing errors typed.
        let m = fs::symlink_metadata(path).map_err(classify)?;
        if !m.is_file() || crate::asset_catalog::linked(&m) {
            return Err(Error::other("Recovery lease must be a regular file"));
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(0);
    }
    let file = options.open(path).map_err(classify)?;
    file.try_lock().map_err(|e| match e {
        fs::TryLockError::WouldBlock => Error::from(ErrorKind::WouldBlock),
        fs::TryLockError::Error(e) => classify(e),
    })?;
    Ok(file)
}
#[derive(Clone, Debug)]
pub struct Candidate {
    pub session: PathBuf,
    pub title: String,
    pub original: Option<PathBuf>,
    pub file: Option<PathBuf>,
    pub error: Option<String>,
    expected: Option<Page>,
}
impl Candidate {
    pub fn open(&self) -> Result<Document, String> {
        owned(&self.session)?;
        let path = self
            .file
            .as_ref()
            .ok_or("Recovery entry has no complete document")?;
        let expected = self
            .expected
            .as_ref()
            .ok_or("Recovery block identity missing")?;
        if crate::save_generation::inspect(expected.id, path)? != *expected {
            return Err("Recovery block changed/corrupt; original preserved".into());
        }
        let mut d = crate::files::open(path, B).map_err(|e| e.to_string())?;
        d.mark_recovered(format!("Recovered: {}", self.title));
        Ok(d)
    }
}
/// An OS sharing lease excludes live Windows sessions from startup recovery.
pub struct Session {
    pub directory: PathBuf,
    lease: Option<fs::File>,
    generation: u64,
}
impl Session {
    pub fn create(storage: &Path) -> Result<Self, String> {
        let storage = crate::file_browser::folder(storage)?;
        let base = storage.join("recovery");
        fs::create_dir_all(&base).map_err(|e| e.to_string())?;
        let base = crate::file_browser::folder(&base)?;
        let directory = loop {
            let p = base.join(format!(
                "session-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            match fs::create_dir(&p) {
                Ok(()) => break p,
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(e) => return Err(e.to_string()),
            }
        };
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(directory.join("owner.json"))
            .map_err(|e| e.to_string())?;
        file.write_all(
            &serde_json::to_vec(&Marker {
                format: "Drawer current-state recovery".into(),
                version: 1,
            })
            .map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        file.sync_all().map_err(|e| e.to_string())?;
        drop(file);
        let held = lease(&directory.join("live.lock"), true).map_err(|e| e.to_string())?;
        Ok(Self {
            directory,
            lease: Some(held),
            generation: 0,
        })
    }
    pub fn checkpoint(&mut self, documents: &[&Document]) -> Result<(), String> {
        self.checkpoint_observed(documents, &mut |_| Ok(()))
    }
    pub fn checkpoint_observed(
        &mut self,
        documents: &[&Document],
        tick: &mut impl FnMut(usize) -> Result<(), String>,
    ) -> Result<(), String> {
        owned(&self.directory)?;
        if self.lease.is_none() || documents.len() > 256 {
            return Err("Recovery session is closed or document limit exceeded".into());
        }
        let bytes = documents
            .iter()
            .try_fold(0usize, |n, d| n.checked_add(d.memory_bytes()))
            .ok_or("Recovery capture memory overflow")?;
        if bytes > B {
            return Err(
                "Recovery current-state capture exceeds memory budget; previous recovery preserved"
                    .into(),
            );
        }
        let mut captured = documents
            .iter()
            .map(|d| d.current_copy())
            .collect::<Vec<_>>();
        self.generation = self
            .generation
            .checked_add(1)
            .ok_or("Recovery generation overflow")?;
        let name = format!("generation-{}", self.generation);
        let staging = self.directory.join(format!("{name}-write.tmp"));
        fs::create_dir(&staging).map_err(|e| e.to_string())?;
        let mut owner = Owned {
            directory: staging,
            parent: self.directory.clone(),
            keep: false,
        };
        let mut entries = vec![];
        for (n, d) in captured.iter_mut().enumerate() {
            tick(n * 2)?;
            let title = d.title.clone();
            let original = d
                .path
                .as_ref()
                .or(d.source_path.as_ref())
                .filter(|p| crate::font_library::local(p))
                .cloned();
            let path = owner.directory.join(format!("document-{}.drawer", d.id));
            crate::files::save(d, &path).map_err(|e| e.to_string())?;
            let page = crate::save_generation::inspect(d.id, &path)?;
            entries.push(Entry {
                page,
                title,
                original,
            });
            tick(n * 2 + 1)?;
        }
        let index = Index {
            version: 1,
            generation: name.clone(),
            documents: entries,
        };
        tick(captured.len() * 2)?;
        let final_dir = self.directory.join(&name);
        fs::rename(&owner.directory, &final_dir).map_err(|e| e.to_string())?;
        owner.directory = final_dir;
        let data = serde_json::to_vec(&index).map_err(|e| e.to_string())?;
        let path = self.directory.join("index.json");
        let temp = self.directory.join("index-write.tmp");
        tick(captured.len() * 2 + 1)?;
        crate::atomic_file::publish(
            &path,
            &temp,
            |w| w.write_all(&data),
            |p| {
                let index = load_index(p).map_err(std::io::Error::other)?;
                for e in &index.documents {
                    let file = self
                        .directory
                        .join(&index.generation)
                        .join(format!("document-{}.drawer", e.page.id));
                    if crate::save_generation::inspect(e.page.id, &file)
                        .map_err(std::io::Error::other)?
                        != e.page
                    {
                        return Err(std::io::Error::other("Recovery generation hash mismatch"));
                    }
                }
                Ok(())
            },
            &mut |_| Ok(()),
        )
        .map_err(|e| e.to_string())?;
        owner.keep = true;
        let _ = tick(captured.len() * 2 + 2);
        Ok(())
    }
    /// Only call after an explicit save/discard decision or a clean normal exit.
    pub fn finish(mut self) -> Result<(), String> {
        let path = owned(&self.directory)?;
        self.lease.take();
        remove_owned(&path)
    }
}
struct Owned {
    directory: PathBuf,
    parent: PathBuf,
    keep: bool,
}
impl Drop for Owned {
    fn drop(&mut self) {
        if !self.keep && self.directory.parent() == Some(self.parent.as_path()) {
            let _ = fs::remove_dir_all(&self.directory);
        }
    }
}
fn remove_owned(path: &Path) -> Result<(), String> {
    owned(path)?;
    fn inspect(p: &Path) -> Result<(), String> {
        for e in fs::read_dir(p).map_err(|e| e.to_string())? {
            let e = e.map_err(|e| e.to_string())?;
            let m = fs::symlink_metadata(e.path()).map_err(|e| e.to_string())?;
            if crate::asset_catalog::linked(&m) {
                return Err("Recovery directory contains a link; preserved".into());
            }
            if m.is_dir() {
                inspect(&e.path())?;
            }
        }
        Ok(())
    }
    inspect(path)?;
    fs::remove_dir_all(path).map_err(|e| e.to_string())
}
pub fn discard(session: &Path) -> Result<(), String> {
    let path = owned(session)?;
    let held = lease(&path.join("live.lock"), false).map_err(|e| e.to_string())?;
    drop(held);
    remove_owned(&path)
}
pub fn scan(storage: &Path) -> Result<Vec<Candidate>, String> {
    let storage = crate::file_browser::folder(storage)?;
    let base = storage.join("recovery");
    if !base.exists() {
        return Ok(vec![]);
    }
    let base = crate::file_browser::folder(&base)?;
    let mut out = vec![];
    for e in fs::read_dir(base).map_err(|e| e.to_string())?.take(128) {
        let e = e.map_err(|e| e.to_string())?;
        let path = e.path();
        if owned(&path).is_err() {
            continue;
        }
        let held = match lease(&path.join("live.lock"), false) {
            Ok(held) => held,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => continue,
            Err(e) => {
                out.push(Candidate {
                    session: path.clone(),
                    title: "Recovery session lock unavailable".into(),
                    original: None,
                    file: None,
                    error: Some(e.to_string()),
                    expected: None,
                });
                continue;
            }
        };
        match load_index(&path.join("index.json")) {
            Ok(index) => {
                for entry in index.documents {
                    let file = path
                        .join(&index.generation)
                        .join(format!("document-{}.drawer", entry.page.id));
                    let error = match crate::save_generation::inspect(entry.page.id, &file) {
                        Ok(actual) if actual == entry.page => None,
                        Ok(_) => Some("Recovery page hash mismatch".into()),
                        Err(e) => Some(e),
                    };
                    out.push(Candidate {
                        session: path.clone(),
                        title: entry.title,
                        original: entry.original,
                        file: Some(file),
                        error,
                        expected: Some(entry.page),
                    });
                }
            }
            Err(error) => out.push(Candidate {
                session: path.clone(),
                title: "Incomplete/corrupt recovery metadata".into(),
                original: None,
                file: None,
                error: Some(error),
                expected: None,
            }),
        }
        drop(held);
    }
    Ok(out)
}

#[path = "recovery_verify.rs"]
pub mod verify;
