use super::index::Index;
use super::*;
use std::sync::atomic::AtomicUsize;
fn check(v: bool, s: &str) -> Result<(), String> {
    if v { Ok(()) } else { Err(s.into()) }
}
pub fn run(root: &Path) -> Result<(), String> {
    let dir = root.join("work-library-core");
    fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let result = (|| {
        let s = UserStorage::at(dir.join("data")).map_err(|e| e.to_string())?;
        let a = dir.join("a.png");
        let b = dir.join("b.png");
        image::save_buffer(&a, &[10, 20, 30, 200], 1, 1, image::ColorType::Rgba8)
            .map_err(|e| e.to_string())?;
        image::save_buffer(
            &b,
            &[200, 40, 60, 255, 20, 60, 90, 255],
            2,
            1,
            image::ColorType::Rgba8,
        )
        .map_err(|e| e.to_string())?;
        let original_a = fs::read(&a).map_err(|e| e.to_string())?;
        let original_b = fs::read(&b).map_err(|e| e.to_string())?;
        let db = Database::load(&s)?;
        let db = db.register(&s, &[a.clone(), b.clone()])?;
        let first = db.state.entries[0].id;
        let second = db.state.entries[1].id;
        check(
            db.counts() == (1, 0) && db.state.entries.len() == 2,
            "Registration atomic library metadata",
        )?;
        let db = db.info(
            &s,
            first,
            "한글 work".into(),
            vec!["인물".into(), "local".into()],
            true,
        )?;
        let c = AtomicBool::new(false);
        let idx = index::rebuild(&s, &db, 64 * 1024 * 1024, &c, |_| Ok(()), || Ok(()))?;
        check(
            idx.rows.len() == 2 && idx.rows.iter().all(|r| r.thumbnail.is_some()),
            "Local thumbnail index",
        )?;
        let saved_db = raw(&s, DB_FILE, LIMIT)?.unwrap();
        let saved_index = raw(&s, INDEX_FILE, 32 * 1024 * 1024)?.unwrap();
        let restarted = Database::load(&s)?;
        check(
            restarted.entry(first)?.favorite
                && restarted.entry(first)?.labels == vec!["인물", "local"]
                && Index::load(&s, &restarted)?.is_some(),
            "Library restart title/labels/favorites/cache",
        )?;
        // Index failure/cancellation never publishes half of a cache or touches metadata.
        check(
            index::rebuild(
                &s,
                &db,
                64 * 1024 * 1024,
                &c,
                |_| Ok(()),
                || Err("simulated interruption before publication".into()),
            )
            .is_err(),
            "Index interruption injection",
        )?;
        check(
            raw(&s, INDEX_FILE, 32 * 1024 * 1024)?.unwrap() == saved_index
                && raw(&s, DB_FILE, LIMIT)?.unwrap() == saved_db,
            "Interrupted index changed committed data",
        )?;
        check(
            index::rebuild(
                &s,
                &db,
                64 * 1024 * 1024,
                &AtomicBool::new(true),
                |_| Ok(()),
                || Ok(()),
            )
            .is_err(),
            "Index cancellation",
        )?;
        let stage = managed(&s, &format!("{INDEX_FILE}.write.tmp"))?;
        let mut partial = b"Drawer work index v1\n".to_vec();
        partial.extend(b"{incomplete");
        fs::write(&stage, partial).map_err(|e| e.to_string())?;
        check(
            Index::load(&s, &Database::load(&s)?)?.is_some(),
            "Restart did not use last complete index",
        )?;
        index::rebuild(&s, &db, 64 * 1024 * 1024, &c, |_| Ok(()), || Ok(()))?;
        check(!stage.exists(), "Owned interrupted index stage leaked")?;
        let denied = index::rebuild(
            &s,
            &db,
            64 * 1024 * 1024,
            &c,
            |_| Err("Access denied / drive unavailable".into()),
            || Ok(()),
        )?;
        check(
            denied
                .rows
                .iter()
                .all(|r| r.bytes.is_none() && r.thumbnail.is_none())
                && Database::load(&s)?.state.entries.len() == 2,
            "Disconnected/denied entries were removed",
        )?;
        let moved = dir.join("moved.png");
        fs::rename(&a, &moved).map_err(|e| e.to_string())?;
        let missing = index::rebuild(&s, &db, 64 * 1024 * 1024, &c, |_| Ok(()), || Ok(()))?;
        check(
            missing.row(first).unwrap().thumbnail.is_none() && db.state.entries.len() == 2,
            "Missing entry list preservation",
        )?;
        let db = db.reconnect(&s, first, &moved)?;
        check(
            db.entry(first)?.favorite
                && db.entry(first)?.title == "한글 work"
                && fs::read(&moved).map_err(|e| e.to_string())? == original_a,
            "Reconnect information/source preservation",
        )?;
        let db = db.remove(&s, second)?;
        check(
            b.exists() && db.state.entries.len() == 1,
            "List removal deleted a file",
        )?;
        let db = db.undo(&s, false)?;
        check(db.entry(second).is_ok(), "Library undo removal")?;
        let db = db.undo(&s, true)?;
        check(db.entry(second).is_err(), "Library redo removal")?;
        let db = db.undo(&s, false)?;
        let db = db.info(&s, second, "New branch".into(), vec![], false)?;
        check(db.counts().1 == 0, "Library edit did not discard redo")?;
        let before = raw(&s, DB_FILE, LIMIT)?.unwrap();
        check(
            db.info(&s, second, "x".repeat(513), vec![], false).is_err()
                && raw(&s, DB_FILE, LIMIT)?.unwrap() == before,
            "Invalid library metadata changed disk",
        )?;
        let stale = db.clone();
        let db = db.info(&s, second, "Other session edit".into(), vec![], false)?;
        check(
            stale.remove(&s, first).is_err() && Database::load(&s)?.entry(first).is_ok(),
            "Stale writer overwrote saved metadata",
        )?;
        // Read-only primary and an occupied lease fail without altering any work/list.
        let primary = managed(&s, DB_FILE)?;
        let old_permissions = fs::metadata(&primary)
            .map_err(|e| e.to_string())?
            .permissions();
        let mut readonly = old_permissions.clone();
        readonly.set_readonly(true);
        fs::set_permissions(&primary, readonly).map_err(|e| e.to_string())?;
        let rejected = db.remove(&s, first).is_err();
        fs::set_permissions(&primary, old_permissions).map_err(|e| e.to_string())?;
        check(
            rejected && Database::load(&s)?.entry(first).is_ok(),
            "Read-only library lost state",
        )?;
        let lock = lease(&s)?;
        check(
            db.remove(&s, first).is_err(),
            "Occupied library lease allowed write",
        )?;
        drop(lock);
        // A broken image remains listed and marked without a fake thumbnail.
        fs::write(&b, b"damaged image").map_err(|e| e.to_string())?;
        let idx = index::rebuild(&s, &db, 64 * 1024 * 1024, &c, |_| Ok(()), || Ok(()))?;
        check(
            idx.row(second).unwrap().thumbnail.is_none(),
            "Damaged file reported a usable thumbnail",
        )?;
        fs::write(&b, &original_b).map_err(|e| e.to_string())?;
        // Recover from a verified previous snapshot, preserving the damaged bytes.
        fs::write(&primary, b"damaged local library").map_err(|e| e.to_string())?;
        check(
            Database::load(&s).is_err() && Database::recover(&s, false, false).is_err(),
            "Corrupt library silently reset",
        )?;
        let recovered = Database::recover(&s, false, true)?;
        check(
            !recovered.state.entries.is_empty(),
            "Previous healthy library recovery",
        )?;
        check(
            fs::read_dir(s.root().join("settings"))
                .map_err(|e| e.to_string())?
                .filter_map(Result::ok)
                .any(|e| {
                    e.file_name()
                        .to_string_lossy()
                        .starts_with("work-library.preserved-")
                }),
            "Pre-recovery bytes not preserved",
        )?;
        check(
            fs::read(&moved).map_err(|e| e.to_string())? == original_a
                && fs::read(&b).map_err(|e| e.to_string())? == original_b,
            "Library operations modified drawing files",
        )?;
        // User document paths inside an old data root are still document identities.
        let docs = s.root().join("documents");
        fs::create_dir(&docs).map_err(|e| e.to_string())?;
        let inside = docs.join("inside.png");
        fs::write(&inside, &original_a).map_err(|e| e.to_string())?;
        let db = Database::load(&s)?.register(&s, std::slice::from_ref(&inside))?;
        let path = db.state.entries.last().unwrap().path.clone();
        let outcome =
            crate::storage_migration::Plan::prepare(&s, &dir.join("relocated-data"), false)?
                .run(&c, &AtomicUsize::new(0))?;
        let relocated = Database::load(&outcome.storage)?;
        check(
            relocated.state.entries.last().unwrap().path == path && inside.exists(),
            "Data migration rebased registered user document path",
        )?;
        let mut view = relocated.state.clone();
        view.sort = Sort::Title;
        view.favorites_only = true;
        let db = relocated.edit(&outcome.storage, "View", |v, _| {
            v.sort = view.sort;
            v.favorites_only = true;
            Ok(())
        })?;
        check(
            index::ordered(&db, None).iter().all(|e| e.favorite),
            "Favorites filter",
        )?;
        Ok(())
    })();
    assets::cleanup_verification(root, &dir)?;
    result
}
#[cfg(test)]
mod tests {
    #[test]
    fn local_works_thumbnails_transactions_recovery_restart_missing_readonly_and_migration() {
        let t = crate::test_support::TempDir::new();
        super::run(&t.0).unwrap();
    }
}
