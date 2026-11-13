//! Bounded local cancellation causes; no drawing, coordinate or key content.
use drawer_core::storage::UserStorage;
use serde::{Deserialize, Serialize};
use std::{
    io::{self, Read, Write},
    path::Path,
};
const REASONS: [&str; 4] = [
    "focus-loss",
    "capture-or-device-cancel",
    "escape",
    "native-input-failure",
];
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    sequence: u64,
    reason: String,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Log {
    version: u32,
    next: u64,
    events: Vec<Entry>,
}
fn read(path: &Path) -> io::Result<Log> {
    let path = drawer_core::file_browser::checked(path).map_err(io::Error::other)?;
    let mut bytes = vec![];
    std::fs::File::open(path)?
        .take(16385)
        .read_to_end(&mut bytes)?;
    if bytes.len() > 16384 {
        return Err(io::Error::other("Input diagnostic exceeds16KiB"));
    }
    let log: Log = serde_json::from_slice(&bytes).map_err(io::Error::other)?;
    if log.version != 1
        || log.next == 0
        || log.events.len() > 32
        || !log.events.iter().all(|e| {
            e.sequence > 0 && e.sequence < log.next && REASONS.contains(&e.reason.as_str())
        })
        || !log.events.windows(2).all(|p| p[0].sequence < p[1].sequence)
    {
        return Err(io::Error::other(
            "Invalid cancellation diagnostic; original preserved",
        ));
    }
    Ok(log)
}
pub(super) fn append(storage: &UserStorage, reasons: &[&str]) -> io::Result<()> {
    if reasons.len() > 32 || reasons.iter().any(|r| !REASONS.contains(r)) {
        return Err(io::Error::other("Invalid cancellation reasons"));
    }
    if reasons.is_empty() {
        return Ok(());
    }
    let path = drawer_core::file_browser::new_path(
        &storage.root().join("settings/input-cancellations.json"),
    )
    .map_err(io::Error::other)?;
    let mut log = if path.try_exists()? {
        read(&path)?
    } else {
        Log {
            version: 1,
            next: 1,
            events: vec![],
        }
    };
    for reason in reasons {
        let next = log
            .next
            .checked_add(1)
            .ok_or_else(|| io::Error::other("Diagnostic sequence exhausted"))?;
        log.events.push(Entry {
            sequence: log.next,
            reason: (*reason).into(),
        });
        log.next = next;
        if log.events.len() > 32 {
            log.events.remove(0);
        }
    }
    let data = serde_json::to_vec(&log).map_err(io::Error::other)?;
    let temp =
        drawer_core::file_browser::new_path(&path.with_file_name("input-cancellations-write.tmp"))
            .map_err(io::Error::other)?;
    drawer_core::atomic_file::publish(
        &path,
        &temp,
        |w| w.write_all(&data),
        |p| read(p).map(|_| ()),
        &mut |_| Ok(()),
    )?;
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn bounded_restart_foreign_temp_and_damaged_log_preserve_previous() {
        let r = crate::tests::TestDirectory::new();
        let s = drawer_core::storage::UserStorage::at(r.0.join("data")).unwrap();
        for _ in 0..35 {
            super::append(&s, &["focus-loss"]).unwrap();
        }
        let path = s.root().join("settings/input-cancellations.json");
        let log = super::read(&path).unwrap();
        assert_eq!(log.events.len(), 32);
        assert_eq!(log.events[0].sequence, 4);
        assert_eq!(log.next, 36);
        let before = std::fs::read(&path).unwrap();
        let temp = s.root().join("settings/input-cancellations-write.tmp");
        std::fs::write(&temp, b"foreign").unwrap();
        assert!(super::append(&s, &["escape"]).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), before);
        assert_eq!(std::fs::read(&temp).unwrap(), b"foreign");
        std::fs::remove_file(temp).unwrap();
        std::fs::write(&path, b"invalid owned log").unwrap();
        assert!(super::append(&s, &["escape"]).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"invalid owned log");
    }
}
