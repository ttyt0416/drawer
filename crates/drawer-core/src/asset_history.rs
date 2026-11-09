use super::*;
const MARKER: &[u8] = b"Drawer local asset trash v1\n";
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Movement {
    pub version: u32,
    pub metadata: Metadata,
    pub root: PathBuf,
    pub original: PathBuf,
    pub trash: PathBuf,
    pub receipt: PathBuf,
    pub sha256: String,
}
impl Movement {
    fn valid(&self) -> bool {
        self.version == 1
            && self.metadata.valid()
            && crate::font_library::local(&self.root)
            && relative(&self.original)
            && relative(&self.trash)
            && relative(&self.receipt)
            && self.trash.starts_with(".drawer-trash")
            && self.receipt.starts_with(".drawer-trash")
            && self.original != self.trash
            && valid_hash(&self.sha256)
    }
    pub fn read(root: &Path, record: &Path) -> Result<Self, String> {
        let movement: Self =
            serde_json::from_slice(&bytes(record, 65536)?).map_err(|e| e.to_string())?;
        if !movement.valid()
            || movement.root != root.canonicalize().map_err(|e| e.to_string())?
            || checked(root, &movement.receipt, true)?
                != record.canonicalize().map_err(|e| e.to_string())?
        {
            return Err("Trash record does not belong to this local asset folder".into());
        }
        Ok(movement)
    }
    pub fn apply(&self, forward: bool) -> Result<(), String> {
        if !self.valid() {
            return Err("Invalid local asset move record".into());
        }
        let (from, to) = if forward {
            (&self.original, &self.trash)
        } else {
            (&self.trash, &self.original)
        };
        let from = checked(&self.root, from, true)?;
        let to = checked(&self.root, to, false)?;
        if to.try_exists().map_err(|e| e.to_string())? {
            return Err(format!(
                "Restore/move destination already exists; preserved: {}",
                to.display()
            ));
        }
        if hash(&bytes(&from, FILE_LIMIT)?) != self.sha256 {
            return Err("Source/trash bytes changed; move refused".into());
        }
        // Both resolved absolute paths were checked against the same configured root.
        fs::rename(from, to).map_err(|e| e.to_string())
    }
}
pub fn restore_record(root: &Path, record: &Path, confirmed: bool) -> Result<PathBuf, String> {
    if !confirmed {
        return Err("Review the exact original/trash paths and confirm restoration".into());
    }
    let movement = Movement::read(root, record)?;
    movement.apply(false)?;
    Ok(movement.original)
}
fn trash(root: &Path) -> Result<(), String> {
    let root = root.canonicalize().map_err(|e| e.to_string())?;
    if !crate::font_library::local(&root) {
        return Err("Trash must stay in the local material folder".into());
    }
    let folder = root.join(".drawer-trash");
    let created = if fs::symlink_metadata(&folder).is_err() {
        fs::create_dir(&folder).map_err(|e| e.to_string())?;
        true
    } else {
        false
    };
    checked(&root, Path::new(".drawer-trash"), true)?;
    let marker = folder.join(".drawer-trash-v1");
    if created {
        let mut f = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&marker)
            .map_err(|e| e.to_string())?;
        f.write_all(MARKER)
            .and_then(|_| f.sync_all())
            .map_err(|e| e.to_string())?;
    }
    if bytes(&marker, 64)? != MARKER {
        return Err("Existing .drawer-trash is not an owned asset trash folder; preserved".into());
    }
    Ok(())
}
pub fn forget(
    index: &Index,
    path: &Path,
    source_file: bool,
    confirmed: bool,
    open_references: usize,
) -> Result<(Index, Option<Movement>), String> {
    if !confirmed {
        return Err(format!(
            "Confirm deletion scope and {open_references} open-document reference(s) first"
        ));
    }
    let entry = index.get(path)?;
    let mut next = index.clone();
    next.entries.retain(|e| e.path != path);
    next.ignored.insert(path.into());
    if !next.valid() {
        return Err("Catalog deletion exceeds local index limit".into());
    }
    if !source_file {
        return Ok((next, None));
    }
    let original = checked(&index.root, path, true)?;
    let data = bytes(&original, FILE_LIMIT)?;
    if !valid_hash(&entry.sha256) || hash(&data) != entry.sha256 {
        return Err("Source changed/unreadable; refresh before removing its file".into());
    }
    trash(&index.root)?;
    let ticket = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_nanos();
    let base = format!("{}-{ticket}", entry.sha256);
    let movement = Movement {
        version: 1,
        metadata: entry.metadata.clone(),
        root: index.root.clone(),
        original: path.into(),
        trash: PathBuf::from(".drawer-trash").join(format!("{base}.asset")),
        receipt: PathBuf::from(".drawer-trash").join(format!("{base}.record.json")),
        sha256: entry.sha256.clone(),
    };
    let receipt = checked(&index.root, &movement.receipt, false)?;
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&receipt)
        .map_err(|e| e.to_string())?;
    let b = serde_json::to_vec(&movement).map_err(|e| e.to_string())?;
    if let Err(e) = f.write_all(&b).and_then(|_| f.sync_all()) {
        drop(f);
        let _ = fs::remove_file(&receipt);
        return Err(e.to_string());
    }
    drop(f);
    if let Err(e) = movement.apply(true) {
        let _ = fs::remove_file(receipt);
        return Err(e);
    }
    Ok((next, Some(movement)))
}
#[derive(Clone)]
struct Change {
    forward: bool,
    before: Arc<Index>,
    after: Arc<Index>,
    movement: Option<Movement>,
}
#[derive(Default)]
pub struct History {
    undo: VecDeque<Change>,
    redo: VecDeque<Change>,
}
impl History {
    pub fn counts(&self) -> (usize, usize) {
        (self.undo.len(), self.redo.len())
    }
    pub fn record(&mut self, before: Arc<Index>, after: Arc<Index>, movement: Option<Movement>) {
        if before == after && movement.is_none() {
            return;
        }
        self.redo.clear();
        self.undo.push_back(Change {
            before,
            after,
            movement,
            forward: true,
        });
        while self.undo.len() > 20
            || self
                .undo
                .iter()
                .map(|c| c.before.resident_bytes() + c.after.resident_bytes())
                .sum::<usize>()
                > 64 * 1024 * 1024
        {
            self.undo.pop_front();
        }
    }
    pub fn record_restore(&mut self, before: Arc<Index>, after: Arc<Index>, movement: Movement) {
        self.record(before, after, Some(movement));
        if let Some(last) = self.undo.back_mut() {
            last.forward = false;
        }
    }
    pub fn undo(
        &mut self,
        current: &Arc<Index>,
        path: &Path,
        token: Option<&str>,
    ) -> Result<Option<(Arc<Index>, String)>, String> {
        self.step(current, path, token, false)
    }
    pub fn redo(
        &mut self,
        current: &Arc<Index>,
        path: &Path,
        token: Option<&str>,
    ) -> Result<Option<(Arc<Index>, String)>, String> {
        self.step(current, path, token, true)
    }
    fn step(
        &mut self,
        current: &Arc<Index>,
        path: &Path,
        token: Option<&str>,
        redo: bool,
    ) -> Result<Option<(Arc<Index>, String)>, String> {
        let change = if redo {
            self.redo.back()
        } else {
            self.undo.back()
        }
        .cloned();
        let Some(change) = change else {
            return Ok(None);
        };
        let (expected, destination) = if redo {
            (change.before.clone(), change.after.clone())
        } else {
            (change.after.clone(), change.before.clone())
        };
        if current != &expected {
            return Err("Catalog changed; reload before restoring list history".into());
        }
        if let Some(m) = &change.movement {
            m.apply(if redo {
                change.forward
            } else {
                !change.forward
            })?;
        }
        let token = match destination.write(path, token, false) {
            Ok(t) => t,
            Err(e) => {
                if let Some(m) = &change.movement
                    && let Err(rollback) = m.apply(if redo {
                        !change.forward
                    } else {
                        change.forward
                    })
                {
                    return Err(format!(
                        "Index save failed: {e}; move rollback failed: {rollback}; bytes remain at the shown original/trash paths"
                    ));
                }
                return Err(e);
            }
        };
        if redo {
            self.redo.pop_back();
            self.undo.push_back(change);
        } else {
            self.undo.pop_back();
            self.redo.push_back(change);
        }
        Ok(Some((destination, token)))
    }
}
/// Roll back a just-published file move if persisting its index failed.
pub fn rollback(movement: &Movement) -> Result<(), String> {
    movement.apply(false)
}
