//! Verified local data-root moves. The pointer beside the bootstrap root is the
//! sole commit record; original roots are retained and drawing history is separate.
use crate::{asset_catalog as assets, file_browser as browser, storage::UserStorage};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
};
type Result<T> = std::result::Result<T, String>;
const LIMIT: u64 = 16 * 1024 * 1024 * 1024;
#[path = "storage_backup.rs"]
pub mod backup;
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    path: PathBuf,
    directory: bool,
    size: u64,
    hash: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Location {
    version: u32,
    nonce: String,
    root: PathBuf,
    previous: PathBuf,
    previous_files: Vec<File>,
    retained: Vec<PathBuf>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Journal {
    version: u32,
    nonce: String,
    source: PathBuf,
    destination: PathBuf,
    staging: PathBuf,
    backup: Option<PathBuf>,
    source_files: Vec<File>,
    destination_files: Vec<File>,
}
pub struct Plan {
    anchor: PathBuf,
    journal: Journal,
    previous: Option<Location>,
    reset_index: bool,
}
pub struct Outcome {
    pub storage: UserStorage,
    pub old: PathBuf,
    pub files: usize,
    pub bytes: u64,
}
fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}
fn side(anchor: &Path, suffix: &str) -> Result<PathBuf> {
    browser::lexical(anchor)?;
    let parent = browser::folder(anchor.parent().ok_or("Missing bootstrap parent")?)?;
    let name = anchor
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or("Invalid bootstrap name")?;
    Ok(parent.join(format!(".{name}.drawer-{suffix}.json")))
}
fn read<T: serde::de::DeserializeOwned>(p: &Path) -> Result<Option<T>> {
    if !p.exists() {
        return Ok(None);
    }
    let bytes = assets::bytes(p, 32 * 1024 * 1024)?;
    serde_json::from_slice(&bytes).map(Some).map_err(err)
}
fn write<T: Serialize>(p: &Path, value: &T) -> Result<()> {
    let bytes = serde_json::to_vec(value).map_err(err)?;
    if bytes.len() > 32 * 1024 * 1024 {
        return Err("Migration record size limit".into());
    }
    let temp = p.with_extension("json.write.tmp");
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)
        .map_err(err)?;
    let result = (|| {
        f.write_all(&bytes).map_err(err)?;
        f.sync_all().map_err(err)?;
        drop(f);
        fs::rename(&temp, p).map_err(err)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}
fn digest(p: &Path, c: &AtomicBool) -> Result<(u64, String)> {
    let mut f = fs::File::open(p).map_err(err)?;
    let mut h = Sha256::new();
    let mut size = 0u64;
    let mut block = [0u8; 65536];
    loop {
        assets::canceled(c)?;
        let n = f.read(&mut block).map_err(err)?;
        if n == 0 {
            break;
        }
        size = size.checked_add(n as u64).ok_or("File size overflow")?;
        if size > LIMIT {
            return Err("Migration file size exceeds16GiB".into());
        }
        h.update(&block[..n]);
    }
    Ok((size, format!("{:x}", h.finalize())))
}
fn manifest(root: &Path, c: &AtomicBool) -> Result<Vec<File>> {
    let root = browser::folder(root)?;
    let mut pending = vec![root.clone()];
    let mut files = vec![];
    let mut size = 0u64;
    while let Some(dir) = pending.pop() {
        for e in fs::read_dir(dir).map_err(err)? {
            assets::canceled(c)?;
            let p = e.map_err(err)?.path();
            let m = fs::symlink_metadata(&p).map_err(err)?;
            if p == root.join(".drawer-move-owner.json") {
                if assets::linked(&m) || !m.is_file() || read::<String>(&p)?.is_none() {
                    return Err("Invalid migration ownership marker".into());
                }
                continue;
            }
            if assets::linked(&m) || (!m.is_dir() && !m.is_file()) {
                return Err("Data move refuses links and nonregular entries".into());
            }
            let rel = p.strip_prefix(&root).map_err(err)?.to_owned();
            if rel.components().count() > 32
                || rel.to_string_lossy().len() > 4096
                || files.len() >= 32768
            {
                return Err("Migration limit:32768 entries, depth32, path4096".into());
            }
            let (len, hash) = if m.is_dir() {
                pending.push(p.clone());
                (0, String::new())
            } else {
                digest(&p, c)?
            };
            size = size.checked_add(len).ok_or("Migration size overflow")?;
            if size > LIMIT {
                return Err("Migration exceeds16GiB".into());
            }
            files.push(File {
                path: rel,
                directory: m.is_dir(),
                size: len,
                hash,
            });
        }
    }
    files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(files)
}
fn verify(root: &Path, files: &[File], c: &AtomicBool) -> Result<()> {
    if manifest(root, c)? != files {
        return Err("Migration file count/size/SHA256 changed; root not switched".into());
    }
    Ok(())
}
fn overlap(a: &Path, b: &Path) -> bool {
    browser::remap(a, b, b).is_some() || browser::remap(b, a, a).is_some()
}
fn marker(j: &Journal) -> PathBuf {
    j.staging.with_extension("owner.json")
}
fn owned(j: &Journal) -> Result<()> {
    if read::<String>(&marker(j))?.as_deref() != Some(&j.nonce) {
        return Err("Migration ownership marker differs; copies preserved".into());
    }
    Ok(())
}
fn remove_copy(p: &Path, expected: &[File], j: &Journal) -> Result<()> {
    owned(j)?;
    if !p.exists() {
        return Ok(());
    }
    let c = AtomicBool::new(false);
    if p != j.staging
        && read::<String>(&p.join(".drawer-move-owner.json"))?.as_deref() != Some(&j.nonce)
    {
        return Err("Copy ownership differs; directory preserved".into());
    }
    let actual = manifest(p, &c)?;
    // Partial staging may contain only paths copied from the reviewed source.
    if expected.is_empty() {
        if actual.iter().any(|f| {
            !j.source_files.iter().any(|s| s.path == f.path)
                && !["asset-index.json", "asset-index.before-move.json"]
                    .iter()
                    .any(|p| f.path == Path::new(p))
        }) {
            return Err("Unexpected files in migration copy; kept for review".into());
        }
    } else if actual != expected {
        return Err("Migration copy was externally changed; preserved".into());
    }
    fs::remove_dir_all(p).map_err(err)
}
fn rollback(j: &Journal) -> Result<()> {
    if j.destination.exists()
        && read::<String>(&j.destination.join(".drawer-move-owner.json"))?.as_deref()
            == Some(&j.nonce)
    {
        remove_copy(&j.destination, &j.destination_files, j)?;
    }
    if let Some(backup) = &j.backup
        && backup.exists()
    {
        if j.destination.exists() {
            return Err("Restore destination occupied; retained backup preserved".into());
        }
        verify(
            backup,
            &j.source_files_for_backup()?,
            &AtomicBool::new(false),
        )?;
        fs::rename(backup, &j.destination).map_err(err)?;
    }
    if j.staging.exists() {
        remove_copy(&j.staging, &[], j)?;
    }
    Ok(())
}
fn valid_files(files: &[File]) -> bool {
    let mut seen = std::collections::BTreeSet::new();
    files.len() <= 32768
        && files.iter().all(|f| {
            !f.path.as_os_str().is_empty()
                && f.path
                    .components()
                    .all(|c| matches!(c, std::path::Component::Normal(_)))
                && f.path.components().count() <= 32
                && f.path.to_string_lossy().len() <= 4096
                && f.size <= LIMIT
                && if f.directory {
                    f.size == 0 && f.hash.is_empty()
                } else {
                    assets::valid_hash(&f.hash)
                }
        })
        && files.iter().all(|f| seen.insert(&f.path))
}
impl Journal {
    fn valid(&self) -> bool {
        self.version == 1
            && !self.nonce.is_empty()
            && self.nonce.len() <= 128
            && self.nonce.bytes().all(|b| b.is_ascii_digit() || b == b'-')
            && browser::lexical(&self.source).is_ok()
            && browser::lexical(&self.destination).is_ok()
            && !overlap(&self.source, &self.destination)
            && self.staging.parent() == self.destination.parent()
            && self
                .staging
                .file_name()
                .is_some_and(|n| n == format!(".drawer-move-{}", self.nonce).as_str())
            && self.backup.as_ref().is_none_or(|p| {
                p.parent() == self.destination.parent()
                    && p.file_name()
                        .is_some_and(|n| n == format!(".drawer-retained-{}", self.nonce).as_str())
            })
            && valid_files(&self.source_files)
            && valid_files(&self.destination_files)
    }
    fn source_files_for_backup(&self) -> Result<Vec<File>> {
        // A backup receives its own immutable manifest before any physical rename.
        read::<Vec<File>>(&self.staging.with_extension("backup.json"))?
            .ok_or("Missing retained-root manifest".into())
    }
}
/// Resolve the atomic commit record. An interrupted copy never becomes writable.
pub fn resolve(anchor: &Path) -> Result<PathBuf> {
    browser::lexical(anchor)?;
    if anchor.parent().is_some_and(|p| !p.exists()) {
        return Ok(anchor.to_owned());
    }
    let pointer = side(anchor, "location")?;
    let location: Option<Location> = read(&pointer)?;
    if let Some(l) = &location {
        if l.version != 1
            || l.retained.len() > 64
            || browser::lexical(&l.previous).is_err()
            || !valid_files(&l.previous_files)
            || l.retained.iter().any(|p| browser::lexical(p).is_err())
        {
            return Err("Unsupported data-root pointer; preserved".into());
        }
        browser::folder(&l.root)?;
    }
    let journal_path = side(anchor, "move")?;
    if let Some(j) = read::<Journal>(&journal_path)? {
        if !j.valid() {
            return Err("Invalid interrupted migration record; preserved".into());
        }
        if location.as_ref().is_none_or(|l| l.nonce != j.nonce) {
            // The pointer was never committed. Restore any retained destination
            // and discard only an exclusively owned partial copy.
            rollback(&j)?;
        }
        if location.as_ref().is_some_and(|l| l.nonce == j.nonce) {
            let _ = fs::remove_file(j.destination.join(".drawer-move-owner.json"));
        }
        let _ = fs::remove_file(&journal_path);
        let _ = fs::remove_file(marker(&j));
        let _ = fs::remove_file(j.staging.with_extension("backup.json"));
    }
    Ok(location.map_or_else(|| anchor.to_owned(), |l| l.root))
}
pub fn previous(s: &UserStorage) -> Result<Option<PathBuf>> {
    Ok(read::<Location>(&side(s.anchor(), "location")?)?.map(|l| l.previous))
}
pub fn retained(s: &UserStorage) -> Result<Vec<PathBuf>> {
    Ok(read::<Location>(&side(s.anchor(), "location")?)?.map_or(vec![], |l| l.retained))
}
impl Plan {
    pub fn prepare(s: &UserStorage, target: &Path, reset_index: bool) -> Result<Self> {
        Self::prepare_cancellable(s, target, reset_index, &AtomicBool::new(false))
    }
    pub fn prepare_cancellable(
        s: &UserStorage,
        target: &Path,
        reset_index: bool,
        c: &AtomicBool,
    ) -> Result<Self> {
        browser::lexical(target)?;
        let source = browser::folder(s.root())?;
        let parent = browser::folder(
            target
                .parent()
                .ok_or("Choose a local destination with an existing parent")?,
        )?;
        let name = target
            .file_name()
            .and_then(|s| s.to_str())
            .ok_or("Choose a destination folder name")?;
        browser::component(name)?;
        let destination = parent.join(name);
        if overlap(&source, &destination) {
            return Err("Source and destination must not overlap".into());
        }
        let previous: Option<Location> = read(&side(s.anchor(), "location")?)?;
        let nonce = format!(
            "{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_else(|error| error.duration())
                .as_nanos()
        );
        let staging = parent.join(format!(".drawer-move-{nonce}"));
        let backup = if destination.exists() {
            let old = previous
                .as_ref()
                .filter(|p| browser::same(&p.previous, &destination))
                .ok_or(
                    "Destination exists; choose a new folder or the retained previous location",
                )?;
            verify(&destination, &old.previous_files, c)?;
            Some(parent.join(format!(".drawer-retained-{nonce}")))
        } else {
            None
        };
        if staging.exists() {
            return Err("Staging collision".into());
        }
        let source_files = manifest(&source, c)?;
        Ok(Self {
            anchor: s.anchor().to_owned(),
            journal: Journal {
                version: 1,
                nonce,
                source,
                destination,
                staging,
                backup,
                source_files,
                destination_files: vec![],
            },
            previous,
            reset_index,
        })
    }
    pub fn target(&self) -> &Path {
        &self.journal.destination
    }
    pub fn run(self, c: &AtomicBool, p: &AtomicUsize) -> Result<Outcome> {
        self.run_with(c, p, |_| Ok(()))
    }
    fn run_with(
        mut self,
        c: &AtomicBool,
        p: &AtomicUsize,
        mut checkpoint: impl FnMut(&str) -> Result<()>,
    ) -> Result<Outcome> {
        let j = &mut self.journal;
        let journal_path = side(&self.anchor, "move")?;
        if journal_path.exists() {
            return Err("Recover the previous move before starting another".into());
        }
        verify(&j.source, &j.source_files, c)?;
        fs::create_dir(&j.staging).map_err(err)?;
        let setup = (|| {
            write(&marker(j), &j.nonce)?;
            write(&j.staging.join(".drawer-move-owner.json"), &j.nonce)?;
            if j.backup.is_some() {
                write(
                    &j.staging.with_extension("backup.json"),
                    &self
                        .previous
                        .as_ref()
                        .ok_or("Missing previous root")?
                        .previous_files,
                )?;
            }
            write(&journal_path, j)?;
            Ok::<(), String>(())
        })();
        if let Err(error) = setup {
            if read::<String>(&marker(j)).ok().flatten().as_deref() == Some(&j.nonce) {
                let _ = rollback(j);
                let _ = fs::remove_file(marker(j));
                let _ = fs::remove_file(j.staging.with_extension("backup.json"));
                let _ = fs::remove_file(&journal_path);
            } else {
                let _ = fs::remove_dir(&j.staging);
            }
            return Err(error);
        }
        let result = (|| {
            for (n, file) in j.source_files.iter().enumerate() {
                assets::canceled(c)?;
                let to = j.staging.join(&file.path);
                if file.directory {
                    fs::create_dir_all(&to).map_err(err)?;
                } else {
                    fs::create_dir_all(to.parent().ok_or("Missing copy parent")?).map_err(err)?;
                    let mut input = fs::File::open(j.source.join(&file.path)).map_err(err)?;
                    let mut output = fs::OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .open(to)
                        .map_err(err)?;
                    let mut block = [0u8; 65536];
                    loop {
                        assets::canceled(c)?;
                        let n = input.read(&mut block).map_err(err)?;
                        if n == 0 {
                            break;
                        }
                        output.write_all(&block[..n]).map_err(err)?;
                    }
                    output.sync_all().map_err(err)?;
                }
                p.store(n + 1, Ordering::Relaxed);
                checkpoint("copy")?;
            }
            verify(&j.staging, &j.source_files, c)?;
            checkpoint("copied")?;
            build_index_and_rebase(&j.staging, &j.source, &j.destination, self.reset_index, c)?;
            j.destination_files = manifest(&j.staging, c)?;
            write(&journal_path, j)?;
            checkpoint("verified")?;
            if let Some(backup) = &j.backup {
                verify(
                    &j.destination,
                    &self
                        .previous
                        .as_ref()
                        .ok_or("Missing retained root")?
                        .previous_files,
                    c,
                )?;
                fs::rename(&j.destination, backup).map_err(err)?;
            }
            fs::rename(&j.staging, &j.destination).map_err(err)?;
            verify(&j.destination, &j.destination_files, c)?;
            verify(&j.source, &j.source_files, c)?;
            checkpoint("installed")?;
            assets::canceled(c)?;
            let mut retained = self
                .previous
                .as_ref()
                .map_or(vec![], |l| l.retained.clone());
            retained.retain(|p| !browser::same(p, &j.destination));
            retained.push(j.source.clone());
            if let Some(b) = &j.backup {
                retained.push(b.clone());
            }
            retained.sort();
            retained.dedup();
            if retained.len() > 64 {
                return Err("Review retained roots before moving again".into());
            }
            let location = Location {
                version: 1,
                nonce: j.nonce.clone(),
                root: j.destination.clone(),
                previous: j.source.clone(),
                previous_files: j.source_files.clone(),
                retained,
            };
            write(&side(&self.anchor, "location")?, &location)?;
            // No fallible operation follows the atomic commit.
            Ok(())
        })();
        if let Err(error) = result {
            let cleanup = rollback(j);
            if cleanup.is_ok() {
                let _ = fs::remove_file(&journal_path);
                let _ = fs::remove_file(marker(j));
                let _ = fs::remove_file(j.staging.with_extension("backup.json"));
            }
            return Err(match cleanup {
                Ok(()) => error,
                Err(e) => format!("{error}; recovery record kept: {e}"),
            });
        }
        let _ = fs::remove_file(j.destination.join(".drawer-move-owner.json"));
        let _ = fs::remove_file(&journal_path);
        let _ = fs::remove_file(marker(j));
        let _ = fs::remove_file(j.staging.with_extension("backup.json"));
        Ok(Outcome {
            storage: UserStorage::relocated(j.destination.clone(), self.anchor),
            old: j.source.clone(),
            files: j.source_files.iter().filter(|f| !f.directory).count(),
            bytes: j.source_files.iter().map(|f| f.size).sum(),
        })
    }
}
fn rebase_value(v: &mut serde_json::Value, old: &Path, new: &Path) -> Result<()> {
    match v {
        serde_json::Value::String(s) => {
            let path = Path::new(s);
            if path.is_absolute()
                && let Some(to) = browser::remap(path, old, new)
            {
                browser::checked(path)
                    .map_err(|e| format!("Missing managed asset {}: {e}", path.display()))?;
                *s = to.to_string_lossy().into_owned();
            }
        }
        serde_json::Value::Array(a) => {
            for v in a {
                rebase_value(v, old, new)?;
            }
        }
        serde_json::Value::Object(o) => {
            for (key, v) in o.iter_mut() {
                // Document identities and saved views are independent of material relocation.
                if ["document", "documents", "recent", "views"].contains(&key.as_str()) {
                    continue;
                }
                rebase_value(v, old, new)?;
            }
        }
        _ => {}
    }
    Ok(())
}
pub fn asset_roots(root: &Path) -> Vec<PathBuf> {
    let mut roots = vec![root.join("materials"), root.join("fonts")];
    if let Ok((index, _)) = assets::Index::read(&root.join("asset-index.json"))
        && browser::remap(&index.root, root, root).is_some()
    {
        roots.push(index.root);
    }
    roots.sort_by_key(|p| std::cmp::Reverse(p.components().count()));
    roots.dedup();
    roots
}
pub fn rebase_settings<T: Serialize + serde::de::DeserializeOwned>(
    value: &T,
    old: &Path,
    new: &Path,
) -> Result<T> {
    let mut value = serde_json::to_value(value).map_err(err)?;
    for source in asset_roots(old) {
        let target = browser::remap(&source, old, new).ok_or("Managed asset root remap")?;
        rebase_value(&mut value, &source, &target)?;
    }
    serde_json::from_value(value).map_err(err)
}
fn build_index_and_rebase(
    stage: &Path,
    old: &Path,
    new: &Path,
    reset: bool,
    c: &AtomicBool,
) -> Result<()> {
    let listing = manifest(stage, c)?;
    for f in &listing {
        if f.directory
            || f.path == Path::new("asset-index.json")
            || f.path.parent() == Some(Path::new("settings"))
                && f.path
                    .file_name()
                    .and_then(|s| s.to_str())
                    .is_some_and(|s| s.starts_with("recent.") || s.starts_with("views"))
            || f.path
                .extension()
                .is_none_or(|s| !s.eq_ignore_ascii_case("json"))
            || !["settings", "materials"]
                .iter()
                .any(|p| f.path.starts_with(p))
            || f.size > 32 * 1024 * 1024
        {
            continue;
        }
        let path = stage.join(&f.path);
        if let Ok(mut value) =
            serde_json::from_slice::<serde_json::Value>(&assets::bytes(&path, 32 * 1024 * 1024)?)
        {
            let before = value.clone();
            value = rebase_settings(&value, old, new)?;
            if value != before {
                write(&path, &value)?;
            }
        }
    }
    let index_path = stage.join("asset-index.json");
    let previous = if index_path.exists() {
        match assets::Index::read(&index_path) {
            Ok((index, _)) => Some(index),
            Err(e) if reset => {
                let backup = stage.join("asset-index.before-move.json");
                if backup.exists() {
                    return Err("Index recovery backup name exists; preserved".into());
                }
                fs::copy(&index_path, backup).map_err(err)?;
                let _ = e;
                None
            }
            Err(e) => {
                return Err(format!(
                    "Index unreadable; explicitly confirm rebuild before migration: {e}"
                ));
            }
        }
    } else {
        None
    };
    let staged_assets = stage.join("materials");
    let mut previous = previous;
    let root = if let Some(index) = &mut previous {
        if let Some(mapped) = browser::remap(&index.root, old, stage) {
            index.root = mapped;
        }
        if browser::remap(&index.root, stage, stage).is_none() {
            index.notes.push("External asset root retained without automatic access; rebuild explicitly from the asset library".into());
            index.notes.truncate(128);
            index.write(&index_path, None, true)?;
            return Ok(());
        }
        index.root.clone()
    } else {
        staged_assets.clone()
    };
    // The ownership marker is transaction metadata, never a catalog asset.
    let owner = if browser::same(&root, stage) {
        let p = stage.join(".drawer-move-owner.json");
        let nonce = read::<String>(&p)?.ok_or("Missing staging owner")?;
        fs::remove_file(&p).map_err(err)?;
        Some((p, nonce))
    } else {
        None
    };
    let scanned = assets::scan(&root, true, previous.as_ref(), c, &AtomicUsize::new(0));
    if let Some((path, nonce)) = owner {
        write(&path, &nonce)?;
    }
    let mut index = scanned?;
    if let Some(mapped) = browser::remap(&index.root, stage, new) {
        index.root = mapped;
    }
    index.write(&index_path, None, true)?;
    Ok(())
}
pub fn restore_materials(s: &UserStorage, backup: &Path) -> Result<()> {
    let backup = browser::checked(backup)?;
    let library = crate::material_library::Library::read(&backup).map_err(err)?;
    let target = s.root().join("materials/library.json");
    let recovery = s.root().join("materials/library.before-restore.json");
    if target.exists() {
        let bytes = assets::bytes(&target, 8 * 1024 * 1024)?;
        let mut f = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&recovery)
            .map_err(err)?;
        f.write_all(&bytes).map_err(err)?;
        f.sync_all().map_err(err)?;
    }
    library.save(&target).map_err(err)
}
#[path = "storage_migration_verify.rs"]
pub mod verify;
