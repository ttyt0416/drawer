//! Disposable thumbnail/index database. Loading never probes registered paths.
use super::*;
const MARKER: &[u8] = b"Drawer work index v1\n";
const INDEX_LIMIT: usize = 32 * 1024 * 1024;
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Thumbnail {
    pub size: [usize; 2],
    pub pixels: Vec<[u8; 4]>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Row {
    pub id: u64,
    pub path: PathBuf,
    pub bytes: Option<u64>,
    pub modified: Option<u64>,
    pub readonly: bool,
    pub thumbnail: Option<Thumbnail>,
    pub status: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Index {
    version: u32,
    pub source: String,
    pub rows: Vec<Row>,
}
impl Index {
    fn valid(&self, db: &Database) -> bool {
        self.version == 1
            && self.source == db.signature()
            && self.rows.len() == db.state.entries.len()
            && self.rows.iter().enumerate().all(|(n, r)| {
                db.entry(r.id).is_ok_and(|e| e.path == r.path)
                    && !self.rows[..n].iter().any(|v| v.id == r.id)
                    && r.status.len() <= 4096
                    && r.thumbnail.as_ref().is_none_or(|t| {
                        t.size[0] > 0
                            && t.size[0] <= 96
                            && t.size[1] > 0
                            && t.size[1] <= 96
                            && t.pixels.len() == t.size[0] * t.size[1]
                    })
            })
            && self.rows.iter().filter(|r| r.thumbnail.is_some()).count() <= 64
    }
    pub fn load(s: &UserStorage, db: &Database) -> Result<Option<Self>, String> {
        let Some(b) = raw(s, INDEX_FILE, INDEX_LIMIT)? else {
            return Ok(None);
        };
        if !b.starts_with(MARKER) {
            return Err(
                "Damaged library thumbnail index; rebuild it without changing the list".into(),
            );
        }
        let value: Self = serde_json::from_slice(&b[MARKER.len()..]).map_err(|e| e.to_string())?;
        if value.source != db.signature() {
            return Ok(None);
        }
        if !value.valid(db) {
            return Err("Invalid thumbnail index; rebuild without changing works".into());
        }
        Ok(Some(value))
    }
    pub fn row(&self, id: u64) -> Option<&Row> {
        self.rows.iter().find(|r| r.id == id)
    }
}
pub fn rebuild(
    s: &UserStorage,
    db: &Database,
    budget: usize,
    c: &AtomicBool,
    local: impl Fn(&Path) -> Result<(), String>,
    checkpoint: impl FnOnce() -> Result<(), String>,
) -> Result<Index, String> {
    if budget < 64 * 1024 * 1024 {
        return Err(
            "Close large drawings before rebuilding thumbnails (64MiB work reserve)".into(),
        );
    }
    let _lease = lease(s)?;
    db.check_disk(s)?;
    let mut rows = vec![];
    let mut thumbs = 0;
    for entry in &db.state.entries {
        assets::canceled(c)?;
        let mut row = Row {
            id: entry.id,
            path: entry.path.clone(),
            bytes: None,
            modified: None,
            readonly: false,
            thumbnail: None,
            status: "Unavailable; reconnect or refresh explicitly".into(),
        };
        let result = (|| {
            local(&entry.path)?;
            let stamp = browser::Stamp::read(&entry.path)?;
            if stamp.directory {
                return Err("Registered path is now a folder; original list retained".into());
            }
            row.bytes = Some(stamp.bytes);
            row.modified = stamp
                .modified
                .duration_since(std::time::UNIX_EPOCH)
                .ok()
                .map(|v| v.as_secs());
            row.readonly = fs::metadata(&stamp.path)
                .map_err(|e| e.to_string())?
                .permissions()
                .readonly();
            if !browser::supported(&entry.path) {
                return Err(crate::exchange_guidance::GUIDE.into());
            }
            if thumbs < 64 {
                let t = browser::thumbnail(&entry.path, budget.min(64 * 1024 * 1024), c)?;
                row.thumbnail = Some(Thumbnail {
                    size: t.size,
                    pixels: t.pixels,
                });
                thumbs += 1;
                row.status = t.notice.unwrap_or_else(|| {
                    "Cached thumbnail; refresh to check current disk contents".into()
                });
            } else {
                row.status = "Metadata cached; thumbnail limit64. File can still be opened.".into();
            }
            stamp.validate()?;
            Ok::<_, String>(())
        })();
        if let Err(e) = result {
            row.thumbnail = None;
            row.status = e.chars().take(1000).collect();
        }
        assets::canceled(c)?;
        rows.push(row);
    }
    let index = Index {
        version: 1,
        source: db.signature(),
        rows,
    };
    if !index.valid(db) {
        return Err("Index reconstruction failed validation".into());
    }
    let mut bytes = MARKER.to_vec();
    bytes.extend(serde_json::to_vec(&index).map_err(|e| e.to_string())?);
    if bytes.len() > INDEX_LIMIT {
        return Err("Thumbnail database memory limit".into());
    }
    atomic(s, INDEX_FILE, &bytes, MARKER, c, || {
        db.check_disk(s)?;
        checkpoint()
    })?;
    Ok(index)
}
pub fn ordered<'a>(db: &'a Database, index: Option<&Index>) -> Vec<&'a Entry> {
    let mut entries = db
        .state
        .entries
        .iter()
        .filter(|e| !db.state.favorites_only || e.favorite)
        .collect::<Vec<_>>();
    entries.sort_by(|a, b| {
        let result = match db.state.sort {
            Sort::Title => a.title.to_lowercase().cmp(&b.title.to_lowercase()),
            Sort::Path => a.path.cmp(&b.path),
            Sort::Modified => index
                .and_then(|i| i.row(a.id))
                .and_then(|r| r.modified)
                .cmp(&index.and_then(|i| i.row(b.id)).and_then(|r| r.modified)),
        }
        .then(a.id.cmp(&b.id));
        if db.state.descending {
            result.reverse()
        } else {
            result
        }
    });
    entries
}
