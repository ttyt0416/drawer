use super::*;
use sha2::{Digest, Sha256};
pub(super) fn load_project(path: &Path) -> Result<crate::story_text::Work, String> {
    let path = crate::file_browser::checked(path)?;
    let base = path.parent().ok_or("Recovery project root missing")?;
    let mut work: crate::story_text::Work = read(&path)?;
    for p in &mut work.pages {
        if p.as_os_str().is_empty()
            || p.components()
                .any(|c| !matches!(c, std::path::Component::Normal(_)))
        {
            return Err("Recovery project page escapes its generation".into());
        }
        *p = crate::file_browser::checked(&base.join(&p))?;
    }
    if !work.valid() {
        return Err("Invalid recovered project".into());
    }
    work.generation
        .as_ref()
        .ok_or("Recovered project generation missing")?
        .verify(&work)?;
    Ok(work)
}
pub(super) fn make_project_portable(path: &Path, root: &Path) -> Result<(), String> {
    let root = crate::file_browser::folder(root)?;
    let mut work = crate::story_text::Work::load(path)?;
    let before = stamp(path)?;
    for p in &mut work.pages {
        *p = p
            .strip_prefix(&root)
            .map_err(|_| "Recovered page is outside its generation")?
            .to_owned();
    }
    let data = serde_json::to_vec(&work).map_err(|e| e.to_string())?;
    if data.len() > 1024 * 1024 {
        return Err("Recovery project metadata limit1MiB".into());
    }
    if stamp(path)? != before {
        return Err("Private recovery project changed before sealing".into());
    }
    let mut f = fs::OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(path)
        .map_err(|e| e.to_string())?;
    f.write_all(&data)
        .and_then(|_| f.sync_all())
        .map_err(|e| e.to_string())?;
    drop(f);
    load_project(path)?;
    Ok(())
}
#[derive(Serialize, Deserialize, PartialEq, Eq, Debug)]
#[serde(deny_unknown_fields)]
struct File {
    path: PathBuf,
    bytes: u64,
    sha256: String,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Inventory {
    version: u32,
    files: Vec<File>,
}
pub(super) fn stamp(path: &Path) -> Result<(u64, String), String> {
    let path = crate::file_browser::checked(path)?;
    let mut f = fs::File::open(path).map_err(|e| e.to_string())?;
    let mut hash = Sha256::new();
    let mut total = 0u64;
    let mut buf = [0; 65536];
    loop {
        let n = f.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        total = total
            .checked_add(n as u64)
            .ok_or("Recovery size overflow")?;
        if total > 2 * 1024 * 1024 * 1024 {
            return Err("Recovery file limit2GiB".into());
        }
        hash.update(&buf[..n]);
    }
    Ok((total, format!("{:x}", hash.finalize())))
}
fn inventory(root: &Path) -> Result<Vec<File>, String> {
    let root = crate::file_browser::folder(root)?;
    let mut todo = vec![(root.clone(), 0)];
    let mut files = vec![];
    let mut count = 0;
    while let Some((p, depth)) = todo.pop() {
        count += 1;
        if count > 4096 || depth > 16 {
            return Err("Recovery generation inventory limit".into());
        }
        let p = crate::file_browser::checked(&p)?;
        let m = fs::metadata(&p).map_err(|e| e.to_string())?;
        if m.is_dir() {
            for e in fs::read_dir(&p).map_err(|e| e.to_string())? {
                todo.push((e.map_err(|e| e.to_string())?.path(), depth + 1));
            }
        } else if m.is_file() {
            let relative = p.strip_prefix(&root).map_err(|e| e.to_string())?.to_owned();
            if relative == Path::new("inventory.json") {
                continue;
            }
            let (bytes, sha256) = stamp(&p)?;
            files.push(File {
                path: relative,
                bytes,
                sha256,
            });
        } else {
            return Err("Unknown recovery entry preserved".into());
        }
    }
    files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(files)
}
pub(super) fn seal(root: &Path) -> Result<(), String> {
    let files = inventory(root)?;
    let data = serde_json::to_vec(&Inventory { version: 1, files }).map_err(|e| e.to_string())?;
    if data.len() > 1024 * 1024 {
        return Err("Recovery inventory limit1MiB".into());
    }
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(root.join("inventory.json"))
        .map_err(|e| e.to_string())?;
    f.write_all(&data)
        .and_then(|_| f.sync_all())
        .map_err(|e| e.to_string())
}
impl Session {
    pub fn set_retention(&mut self, keep: usize) -> Result<(), String> {
        if !(2..=32).contains(&keep) {
            return Err("Keep2..32 verified recovery generations".into());
        }
        self.keep = keep;
        Ok(())
    }
    pub fn take_maintenance_warning(&mut self) -> Option<String> {
        self.maintenance_warning.take()
    }
    pub fn prune_generations(&mut self) -> Result<(), String> {
        let root = owned(&self.directory)?;
        if self.lease.is_none() {
            return Err("Recovery session paused".into());
        }
        let mut protected = std::collections::BTreeSet::new();
        for name in ["index.json", "index.json.drawer-last-good"] {
            let p = root.join(name);
            if p.exists() {
                protected.insert(load_index(&p)?.generation);
            }
        }
        let mut generations = vec![];
        for e in fs::read_dir(&root).map_err(|e| e.to_string())? {
            let e = e.map_err(|e| e.to_string())?;
            let name = e.file_name().to_string_lossy().into_owned();
            if let Some(n) = name
                .strip_prefix("generation-")
                .and_then(|s| s.parse::<u64>().ok())
            {
                generations.push((n, name, e.path()));
            }
        }
        generations.sort_by_key(|e| std::cmp::Reverse(e.0));
        for (_, name, _) in generations.iter().take(self.keep) {
            protected.insert(name.clone());
        }
        for (_, name, path) in generations {
            if protected.contains(&name) {
                continue;
            }
            let path = crate::file_browser::folder(&path)?;
            if path.parent() != Some(root.as_path()) {
                return Err("Recovery cleanup escaped its session".into());
            }
            let manifest: Inventory = read(&path.join("inventory.json"))
                .map_err(|e| format!("Unrecognized old generation preserved: {e}"))?;
            if manifest.version != 1 || manifest.files != inventory(&path)? {
                return Err(
                    "Changed/unknown old generation preserved; resolve it before cleanup".into(),
                );
            }
            fs::remove_dir_all(path).map_err(|e| e.to_string())?;
        }
        Ok(())
    }
}
