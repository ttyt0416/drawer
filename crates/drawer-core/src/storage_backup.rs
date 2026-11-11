//! Versioned, hash-verified local snapshots; restoration never overwrites a live root.
use super::*;
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Bundle {
    version: u32,
    application: String,
    native_version: u32,
    source: PathBuf,
    files: Vec<File>,
}
fn nonce() -> String {
    format!(
        "{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_else(|e| e.duration())
            .as_nanos()
    )
}
fn destination(path: &Path, sources: &[&Path]) -> Result<PathBuf> {
    browser::lexical(path)?;
    let parent = browser::folder(path.parent().ok_or("Choose an existing local parent")?)?;
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or("Missing folder name")?;
    browser::component(name)?;
    let result = parent.join(name);
    if result.try_exists().map_err(err)? || sources.iter().any(|p| overlap(p, &result)) {
        return Err(
            "Backup/restore destination exists or overlaps a source; choose a new folder".into(),
        );
    }
    Ok(result)
}
fn total(files: &[File]) -> Result<u64> {
    files.iter().try_fold(65536u64, |n, f| {
        n.checked_add(f.size)
            .ok_or_else(|| "Snapshot size overflow".into())
    })
}
fn admitted(files: &[File]) -> bool {
    valid_files(files) && {
        let mut seen = std::collections::BTreeSet::new();
        files.iter().all(|f| {
            seen.insert(f.path.to_string_lossy().to_lowercase())
                && f.path.components().all(|c| {
                    c.as_os_str()
                        .to_str()
                        .is_some_and(|s| browser::component(s).is_ok())
                })
        })
    }
}
fn copy(from: &Path, to: &Path, files: &[File], c: &AtomicBool, p: &AtomicUsize) -> Result<()> {
    for f in files {
        assets::canceled(c)?;
        let out = to.join(&f.path);
        if f.directory {
            fs::create_dir_all(out).map_err(err)?;
        } else {
            fs::create_dir_all(out.parent().ok_or("Missing copy parent")?).map_err(err)?;
            let mut input = fs::File::open(browser::checked(&from.join(&f.path))?).map_err(err)?;
            let mut output = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(out)
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
        p.fetch_add(1, Ordering::Relaxed);
    }
    Ok(())
}
fn discard_owned(stage: &Path, owner: &str, expected: &[File], has_bundle: bool) {
    // Preserve unknown, changed or linked contents, including interrupted partial files.
    let c = AtomicBool::new(false);
    if read::<String>(&stage.join(".drawer-move-owner.json"))
        .ok()
        .flatten()
        .as_deref()
        != Some(owner)
        || browser::folder(stage).is_err()
    {
        return;
    }
    let Ok(actual) = manifest(stage, &c) else {
        return;
    };
    if actual.iter().any(|f| {
        !(has_bundle && f.path == Path::new("bundle.json") && !f.directory) && !expected.contains(f)
    }) {
        return;
    }
    let _ = fs::remove_dir_all(stage);
}
/// `available` is measured by the platform on the destination volume before work starts.
pub fn create(
    s: &UserStorage,
    target: &Path,
    documents: &[PathBuf],
    available: u64,
    c: &AtomicBool,
    p: &AtomicUsize,
) -> Result<PathBuf> {
    let source = browser::folder(s.root())?;
    let target = destination(target, &[&source])?;
    if documents.len() > 1024 {
        return Err("Select at most1024 document files".into());
    }
    let data = manifest(&source, c)?;
    let mut files = vec![
        File {
            path: "userdata".into(),
            directory: true,
            size: 0,
            hash: String::new(),
        },
        File {
            path: "documents".into(),
            directory: true,
            size: 0,
            hash: String::new(),
        },
    ];
    files.extend(data.iter().cloned().map(|mut f| {
        f.path = Path::new("userdata").join(f.path);
        f
    }));
    let mut docs = vec![];
    let mut names = std::collections::BTreeSet::new();
    for path in documents {
        let path = browser::checked(path)?;
        if overlap(&target, &path) || !path.is_file() {
            return Err("Document must be a separate local regular file".into());
        }
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or("Invalid document name")?;
        browser::component(name)?;
        if !names.insert(name.to_lowercase()) {
            return Err(
                "Duplicate document filename; choose distinct names before backing up".into(),
            );
        }
        let (size, hash) = digest(&path, c)?;
        files.push(File {
            path: Path::new("documents").join(name),
            directory: false,
            size,
            hash,
        });
        docs.push(path);
    }
    files.sort_by(|a, b| a.path.cmp(&b.path));
    if !admitted(&files) || total(&files)? > LIMIT || total(&files)? > available {
        return Err(
            "Snapshot limits or available disk space exceeded; no destination created".into(),
        );
    }
    let id = nonce();
    let stage = target.with_file_name(format!(".drawer-backup-{id}"));
    fs::create_dir(&stage).map_err(err)?;
    let owned_files = files.clone();
    let result = (|| {
        write(&stage.join(".drawer-move-owner.json"), &id)?;
        fs::create_dir(stage.join("userdata")).map_err(err)?;
        fs::create_dir(stage.join("documents")).map_err(err)?;
        copy(&source, &stage.join("userdata"), &data, c, p)?;
        for path in &docs {
            let name = path.file_name().ok_or("Missing document name")?;
            let record = files
                .iter()
                .find(|f| f.path == Path::new("documents").join(name))
                .ok_or("Missing document record")?;
            copy(
                path.parent().ok_or("Missing document parent")?,
                &stage.join("documents"),
                &[File {
                    path: name.into(),
                    ..record.clone()
                }],
                c,
                p,
            )?;
        }
        verify(&source, &data, c)?;
        verify(&stage, &files, c)?;
        write(
            &stage.join("bundle.json"),
            &Bundle {
                version: 1,
                application: env!("CARGO_PKG_VERSION").into(),
                native_version: 125,
                source: source.clone(),
                files,
            },
        )?;
        validate(&stage, c)?;
        assets::canceled(c)?;
        if target.try_exists().map_err(err)? {
            return Err("Destination appeared; backup not published".into());
        }
        fs::rename(&stage, &target).map_err(err)?;
        Ok(target.clone())
    })();
    if result.is_err() {
        discard_owned(&stage, &id, &owned_files, true);
    }
    result
}
fn validate(root: &Path, c: &AtomicBool) -> Result<Bundle> {
    let root = browser::folder(root)?;
    let bundle = read::<Bundle>(&root.join("bundle.json"))?.ok_or("Missing bundle manifest")?;
    if bundle.version != 1
        || bundle.native_version > 125
        || bundle.application.len() > 64
        || !admitted(&bundle.files)
        || total(&bundle.files)? > LIMIT
    {
        return Err("Unsupported or invalid backup manifest".into());
    }
    browser::lexical(&bundle.source)?;
    let actual = manifest(&root, c)?;
    let actual: Vec<_> = actual
        .into_iter()
        .filter(|f| f.path != Path::new("bundle.json"))
        .collect();
    if actual != bundle.files {
        return Err("Backup file list/size/SHA256 differs; nothing restored".into());
    }
    if !bundle
        .files
        .iter()
        .any(|f| f.directory && f.path == Path::new("userdata"))
        || !bundle
            .files
            .iter()
            .any(|f| f.directory && f.path == Path::new("documents"))
    {
        return Err("Missing backup roots".into());
    }
    if bundle
        .files
        .iter()
        .any(|f| !f.path.starts_with("userdata") && !f.path.starts_with("documents"))
    {
        return Err("Unexpected backup payload".into());
    }
    Ok(bundle)
}
fn remap_values(v: &mut serde_json::Value, old: &Path, new: &Path) {
    match v {
        serde_json::Value::String(s) => {
            if let Some(to) = browser::remap(Path::new(s), old, new) {
                *s = to.to_string_lossy().into_owned();
            }
        }
        serde_json::Value::Array(a) => {
            for v in a {
                remap_values(v, old, new);
            }
        }
        serde_json::Value::Object(o) => {
            for v in o.values_mut() {
                remap_values(v, old, new);
            }
        }
        _ => {}
    }
}
pub struct Restored {
    pub root: PathBuf,
    pub previous_backup: PathBuf,
}
/// Requires a quiescent workspace and restart after success. The location pointer is the sole commit.
pub fn restore(
    s: &UserStorage,
    bundle: &Path,
    target: &Path,
    available: u64,
    c: &AtomicBool,
    p: &AtomicUsize,
) -> Result<Restored> {
    let bundle = browser::folder(bundle)?;
    let meta = validate(&bundle, c)?;
    let target = destination(target, &[s.root(), &bundle])?;
    let previous_backup = target.with_file_name(format!(
        "{}-before-restore",
        target.file_name().ok_or("Missing name")?.to_string_lossy()
    ));
    destination(&previous_backup, &[s.root(), &bundle, &target])?;
    let current = manifest(s.root(), c)?;
    let needed = total(&meta.files)?
        .checked_add(total(&current)?)
        .and_then(|n| n.checked_add(32 * 1024 * 1024))
        .ok_or("Restore size overflow")?;
    if needed > available {
        return Err("Insufficient restore + current-settings backup space; nothing changed".into());
    }
    if side(s.anchor(), "move")?.try_exists().map_err(err)? {
        return Err("Resolve interrupted move before restore".into());
    }
    let previous: Option<Location> = read(&side(s.anchor(), "location")?)?;
    let mut retained = previous.map_or(vec![], |l| l.retained);
    retained.push(s.root().into());
    retained.sort();
    retained.dedup();
    if retained.len() > 64 {
        return Err("Review retained locations before restoring".into());
    }
    create(s, &previous_backup, &[], available, c, p)?;
    let id = nonce();
    let stage = target.with_file_name(format!(".drawer-restore-{id}"));
    fs::create_dir(&stage).map_err(err)?;
    let mut owned_files = Vec::new();
    for f in &meta.files {
        if let Ok(path) = f.path.strip_prefix("userdata")
            && !path.as_os_str().is_empty()
        {
            owned_files.push(File {
                path: path.into(),
                ..f.clone()
            });
        }
        if let Ok(path) = f.path.strip_prefix("documents") {
            owned_files.push(File {
                path: Path::new("restored-documents").join(path),
                ..f.clone()
            });
        }
    }
    let result = (|| {
        write(&stage.join(".drawer-move-owner.json"), &id)?;
        let data: Vec<_> = meta
            .files
            .iter()
            .filter_map(|f| {
                f.path
                    .strip_prefix("userdata")
                    .ok()
                    .filter(|p| !p.as_os_str().is_empty())
                    .map(|path| File {
                        path: path.into(),
                        ..f.clone()
                    })
            })
            .collect();
        copy(&bundle.join("userdata"), &stage, &data, c, p)?;
        // Additional chosen documents remain inside the new root; originals are never overwritten.
        let restored_docs = stage.join("restored-documents");
        if restored_docs.try_exists().map_err(err)? {
            return Err("Restored document folder collides with user data".into());
        }
        fs::create_dir(&restored_docs).map_err(err)?;
        let docs: Vec<_> = meta
            .files
            .iter()
            .filter_map(|f| {
                f.path
                    .strip_prefix("documents")
                    .ok()
                    .filter(|p| !p.as_os_str().is_empty())
                    .map(|path| File {
                        path: path.into(),
                        ..f.clone()
                    })
            })
            .collect();
        copy(&bundle.join("documents"), &restored_docs, &docs, c, p)?;
        verify(
            &stage,
            &{
                let mut all = data.clone();
                all.push(File {
                    path: "restored-documents".into(),
                    directory: true,
                    size: 0,
                    hash: String::new(),
                });
                all.extend(docs.iter().cloned().map(|mut f| {
                    f.path = Path::new("restored-documents").join(f.path);
                    f
                }));
                all.sort_by(|a, b| a.path.cmp(&b.path));
                all
            },
            c,
        )?;
        for f in manifest(&stage, c)? {
            if !f.directory
                && f.size <= 32 * 1024 * 1024
                && f.path
                    .extension()
                    .is_some_and(|s| s.eq_ignore_ascii_case("json"))
                && (f.path == Path::new("asset-index.json")
                    || ["settings", "materials"]
                        .iter()
                        .any(|n| f.path.starts_with(n)))
            {
                let path = stage.join(f.path);
                if let Ok(mut value) = serde_json::from_slice::<serde_json::Value>(&assets::bytes(
                    &path,
                    32 * 1024 * 1024,
                )?) {
                    remap_values(&mut value, &meta.source, &target);
                    write(&path, &value)?;
                }
            }
        }
        validate(&bundle, c)?;
        verify(s.root(), &current, c)?;
        assets::canceled(c)?;
        let final_files = manifest(&stage, c)?;
        if target.try_exists().map_err(err)? {
            return Err("Restore destination appeared".into());
        }
        fs::rename(&stage, &target).map_err(err)?;
        let pointer = Location {
            version: 1,
            nonce: id.clone(),
            root: target.clone(),
            previous: s.root().into(),
            previous_files: current,
            retained,
        };
        // On a pointer failure the complete target and current backup are retained; active root is unchanged.
        verify(&target, &final_files, c)?;
        write(&side(s.anchor(), "location")?, &pointer)?;
        Ok(Restored {
            root: target.clone(),
            previous_backup: previous_backup.clone(),
        })
    })();
    if result.is_err() {
        discard_owned(&stage, &id, &owned_files, false);
    }
    result
}

#[cfg(test)]
#[path = "storage_backup_verify.rs"]
mod tests;
