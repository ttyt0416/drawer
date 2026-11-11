//! Reviewed local model snapshots, immutable cache folders and task registrations.
use super::*;
use drawer_core::{file_browser as paths, storage::UserStorage};
use std::{
    fs,
    io::Write,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};
const NAMES: [&str; 3] = ["manifest.json", "weights.onnx", "LICENSE.txt"];
pub fn valid_pin(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}
#[derive(Clone, Debug)]
pub struct Review {
    pub manifest: Manifest,
    pub manifest_sha256: String,
    pub license_text: String,
    pub weights_bytes: usize,
}
pub fn review(path: &Path, task: Task) -> Result<Review, String> {
    let b = inspect_data(path, task)?;
    Ok(Review {
        manifest: b.manifest,
        manifest_sha256: b.manifest_sha256,
        license_text: b.license,
        weights_bytes: b.weights.len(),
    })
}
/// Measured admission before the app allocates a frozen inspection/copy snapshot.
pub fn review_memory(path: &Path) -> Result<usize, String> {
    let path = paths::checked(path)?;
    let weights = paths::checked(
        &path
            .parent()
            .ok_or("Missing model folder")?
            .join("weights.onnx"),
    )?;
    let size = fs::metadata(weights).map_err(|e| e.to_string())?.len();
    if size > 64 * 1024 * 1024 {
        return Err("Model weights exceed64MiB".into());
    }
    (size as usize)
        .checked_mul(2)
        .and_then(|n| n.checked_add(8 * 1024 * 1024))
        .ok_or_else(|| "Model memory estimate overflow".into())
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Registration {
    pub task: Task,
    pub manifest_sha256: String,
    pub name: String,
    pub version: String,
    pub license: String,
}
impl Registration {
    pub fn path(&self, storage: &UserStorage) -> PathBuf {
        storage
            .root()
            .join("models")
            .join(&self.manifest_sha256)
            .join("manifest.json")
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Registry {
    version: u32,
    entries: Vec<Registration>,
}
fn registry_path(storage: &UserStorage) -> PathBuf {
    storage.root().join("settings/local-models.json")
}
fn validate(r: &Registry) -> Result<(), String> {
    if r.version != 1 || r.entries.len() > 5 {
        return Err("Unsupported model registration version/count; current bytes preserved".into());
    }
    for (n, e) in r.entries.iter().enumerate() {
        if !valid_pin(&e.manifest_sha256)
            || [&e.name, &e.version, &e.license]
                .iter()
                .any(|s| s.trim().is_empty() || s.len() > 512 || s.chars().any(char::is_control))
            || r.entries[..n].iter().any(|p| p.task == e.task)
        {
            return Err("Invalid or duplicate local model registration".into());
        }
    }
    Ok(())
}
fn load_registry(storage: &UserStorage) -> Result<(Registry, Option<Vec<u8>>), String> {
    let path = paths::new_path(&registry_path(storage))?;
    if !path.try_exists().map_err(|e| e.to_string())? {
        return Ok((
            Registry {
                version: 1,
                entries: vec![],
            },
            None,
        ));
    }
    let bytes = read_bytes(&path, 32768)?;
    let r = serde_json::from_slice(&bytes)
        .map_err(|e| format!("Model registration unreadable; preserved: {e}"))?;
    validate(&r)?;
    Ok((r, Some(bytes)))
}
pub fn registered(storage: &UserStorage, task: Task) -> Result<Option<Registration>, String> {
    Ok(load_registry(storage)?
        .0
        .entries
        .into_iter()
        .find(|r| r.task == task))
}
fn exclusive(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| e.to_string())?;
    f.write_all(bytes).map_err(|e| e.to_string())?;
    f.sync_all().map_err(|e| e.to_string())
}
fn write_registry(storage: &UserStorage, r: &Registry, old: Option<&[u8]>) -> Result<(), String> {
    validate(r)?;
    let bytes = serde_json::to_vec(r).map_err(|e| e.to_string())?;
    let path = paths::new_path(&registry_path(storage))?;
    let parent = paths::folder(path.parent().ok_or("Missing settings folder")?)?;
    let temp = paths::new_path(&parent.join("local-models.write.tmp"))?;
    // create_new is also the per-registry writer exclusion; never remove a foreign temporary.
    let mut output = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)
        .map_err(|e| e.to_string())?;
    let result = (|| {
        output.write_all(&bytes).map_err(|e| e.to_string())?;
        output.sync_all().map_err(|e| e.to_string())?;
        drop(output);
        if read_bytes(&temp, 32768)? != bytes {
            return Err("Model registration write verification failed".into());
        }
        let current = if path.try_exists().map_err(|e| e.to_string())? {
            Some(read_bytes(&path, 32768)?)
        } else {
            None
        };
        if current.as_deref() != old {
            return Err("Model registration changed while writing; retry".into());
        }
        if let Some(old) = old {
            let backup =
                paths::new_path(&parent.join(format!("local-models.{}.previous.json", hash(old))))?;
            if backup.try_exists().map_err(|e| e.to_string())? {
                if read_bytes(&backup, 32768)? != old {
                    return Err("Model registration backup collision; preserved".into());
                }
            } else {
                exclusive(&backup, old)?;
            }
        }
        fs::rename(&temp, &path).map_err(|e| e.to_string())
    })();
    if result.is_err() && read_bytes(&temp, 32768).ok().as_deref() == Some(bytes.as_slice()) {
        let _ = fs::remove_file(&temp);
    }
    result
}
pub fn forget(storage: &UserStorage, task: Task) -> Result<(), String> {
    let (mut r, old) = load_registry(storage)?;
    r.entries.retain(|e| e.task != task);
    write_registry(storage, &r, old.as_deref())
}
fn exact_cache(path: &Path) -> Result<(), String> {
    let root = paths::folder(path)?;
    let mut names = vec![];
    for e in fs::read_dir(&root).map_err(|e| e.to_string())? {
        let e = e.map_err(|e| e.to_string())?;
        let path = paths::checked(&e.path())?;
        if !path.is_file() {
            return Err("Unknown model-cache entries preserved".into());
        }
        names.push(
            e.file_name()
                .to_str()
                .ok_or("Non-Unicode cache filename")?
                .to_owned(),
        );
    }
    names.sort();
    let mut expected = NAMES.map(str::to_owned);
    expected.sort();
    if names != expected {
        return Err("Model cache contains missing or unknown files; preserved".into());
    }
    Ok(())
}
fn cleanup(stage: &Path, owner: &Path, id: &str, expected: &[(String, String)]) {
    if read_bytes(owner, 256).ok().as_deref() != Some(id.as_bytes()) {
        return;
    }
    let Ok(root) = paths::folder(stage) else {
        return;
    };
    let Ok(entries) = fs::read_dir(&root) else {
        return;
    };
    for entry in entries {
        let Ok(entry) = entry else {
            return;
        };
        let Ok(p) = paths::checked(&entry.path()) else {
            return;
        };
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            return;
        };
        let Some((_, digest)) = expected.iter().find(|(n, _)| n == &name) else {
            return;
        };
        if !p.is_file()
            || read_bytes(&p, 64 * 1024 * 1024)
                .ok()
                .is_none_or(|b| hash(&b) != *digest)
        {
            return;
        }
    }
    if fs::remove_dir_all(root).is_ok() {
        let _ = fs::remove_file(owner);
    }
}
/// Explicit permission refers to copying/using the reviewed local notice, not an online entitlement.
pub fn install(
    storage: &UserStorage,
    source: &Path,
    task: Task,
    expected: &str,
    permission: bool,
    available: u64,
) -> Result<Registration, String> {
    if !permission || !valid_pin(expected) {
        return Err("Review LICENSE.txt and confirm local copy/use before installation".into());
    }
    let (mut registry, old) = load_registry(storage)?;
    let b = inspect_data(source, task)?;
    if b.manifest_sha256 != expected {
        return Err("Model changed after review; inspect the license and approve again".into());
    }
    let needed = (b.weights.len() + b.raw_manifest.len() + b.license.len()) as u64 + 128 * 1024;
    if needed > available {
        return Err("Insufficient local model-cache/registration disk space; unchanged".into());
    }
    let cache = paths::new_path(&storage.root().join("models"))?;
    fs::create_dir_all(&cache).map_err(|e| e.to_string())?;
    let cache = paths::folder(&cache)?;
    let target = paths::new_path(&cache.join(expected))?;
    let record = Registration {
        task,
        manifest_sha256: expected.into(),
        name: b.manifest.name.clone(),
        version: b.manifest.version.clone(),
        license: b.manifest.license.clone(),
    };
    if target.try_exists().map_err(|e| e.to_string())? {
        drop(b);
        exact_cache(&target)?;
        if inspect_data(&target.join("manifest.json"), task)?.manifest_sha256 != expected {
            return Err("Existing model cache changed; no overwrite".into());
        }
    } else {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let id = format!(
            "{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_else(|e| e.duration())
                .as_nanos(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        );
        let stage = cache.join(format!(".drawer-model-{id}"));
        let owner = cache.join(format!(".drawer-model-{id}.owner"));
        exclusive(&owner, id.as_bytes())?;
        if let Err(e) = fs::create_dir(&stage) {
            let _ = fs::remove_file(&owner);
            return Err(e.to_string());
        }
        let expected_files: Vec<_> = [
            ("manifest.json", b.raw_manifest.as_slice()),
            ("weights.onnx", b.weights.as_slice()),
            ("LICENSE.txt", b.license.as_bytes()),
        ]
        .iter()
        .map(|(n, bytes)| (n.to_string(), hash(bytes)))
        .collect();
        let result = (|| {
            for (name, bytes) in [
                ("manifest.json", b.raw_manifest.as_slice()),
                ("weights.onnx", b.weights.as_slice()),
                ("LICENSE.txt", b.license.as_bytes()),
            ] {
                exclusive(&stage.join(name), bytes)?;
            }
            drop(b);
            exact_cache(&stage)?;
            if inspect_data(&stage.join("manifest.json"), task)?.manifest_sha256 != expected {
                return Err("Copied model bundle differs".into());
            }
            if target.try_exists().map_err(|e| e.to_string())? {
                return Err("Model-cache destination appeared; no overwrite".into());
            }
            fs::rename(&stage, &target).map_err(|e| e.to_string())?;
            let _ = fs::remove_file(&owner);
            Ok(())
        })();
        if let Err(e) = result {
            cleanup(&stage, &owner, &id, &expected_files);
            return Err(e);
        }
    }
    registry.entries.retain(|r| r.task != task);
    registry.entries.push(record.clone());
    registry.entries.sort_by_key(|r| r.task.label());
    write_registry(storage, &registry, old.as_deref())?;
    Ok(record)
}
#[cfg(test)]
#[path = "bundle_verify.rs"]
mod tests;
