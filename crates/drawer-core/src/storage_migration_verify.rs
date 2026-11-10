use super::*;
pub fn run(root: &Path) -> Result<()> {
    let dir = root.join("storage-migration-core");
    fs::create_dir(&dir).map_err(err)?;
    let result = (|| {
        let anchor = dir.join("사용자 데이터");
        let storage = UserStorage::at(anchor.clone()).map_err(err)?;
        let asset = storage.root().join("materials/한글.png");
        let mut d = crate::png_io_verify::fixture()?;
        crate::png_export::export(&d, &asset, 64 * 1024 * 1024).map_err(err)?;
        let asset = asset.canonicalize().map_err(err)?;
        let userdoc = storage.root().join("documents/user.drawer");
        fs::create_dir(userdoc.parent().ok_or("User document parent")?).map_err(err)?;
        crate::files::save(&mut d, &userdoc).map_err(err)?;
        let mut recent = crate::recent::RecentFiles::load(&storage).map_err(err)?;
        recent.record(&userdoc, Some(&storage)).map_err(err)?;
        write(
            &storage.root().join("settings/test-reference.json"),
            &serde_json::json!({"source":asset,"document":userdoc}),
        )?;
        let original = manifest(storage.root(), &AtomicBool::new(false))?;
        for target in [storage.root().to_owned(), storage.root().join("child")] {
            if Plan::prepare(&storage, &target, false).is_ok() {
                return Err("Overlapping migration accepted".into());
            }
        }
        let cancel = AtomicBool::new(false);
        let fail = dir.join("space-failure");
        if Plan::prepare(&storage, &fail, false)?
            .run_with(&cancel, &AtomicUsize::new(0), |step| {
                if step == "copy" {
                    Err("Injected no disk space".into())
                } else {
                    Ok(())
                }
            })
            .is_ok()
            || fail.exists()
            || manifest(storage.root(), &cancel)? != original
        {
            return Err("Copy failure did not preserve the source".into());
        }
        let cancel = AtomicBool::new(false);
        let canceled = dir.join("cancel");
        if Plan::prepare(&storage, &canceled, false)?
            .run_with(&cancel, &AtomicUsize::new(0), |step| {
                if step == "copied" {
                    cancel.store(true, Ordering::Relaxed);
                }
                Ok(())
            })
            .is_ok()
            || canceled.exists()
        {
            return Err("Canceled copy was activated".into());
        }
        if fs::read_dir(&dir)
            .map_err(err)?
            .filter_map(|e| e.ok())
            .any(|e| e.file_name().to_string_lossy().starts_with(".drawer-move-"))
        {
            return Err("Owned canceled staging was retained".into());
        }
        // An interrupted, partially copied directory is journaled but uncommitted.
        let plan = Plan::prepare(&storage, &dir.join("interrupted"), false)?;
        let j = &plan.journal;
        fs::create_dir(&j.staging).map_err(err)?;
        write(&marker(j), &j.nonce)?;
        write(&j.staging.join(".drawer-move-owner.json"), &j.nonce)?;
        fs::create_dir(j.staging.join("settings")).map_err(err)?;
        write(&side(storage.anchor(), "move")?, j)?;
        if !browser::same(&resolve(storage.anchor())?, storage.root()) || j.staging.exists() {
            return Err("Copy crash recovery chose staging".into());
        }
        // Simulate termination after installation but before the atomic pointer.
        let mut plan = Plan::prepare(&storage, &dir.join("installed-crash"), false)?;
        let j = &mut plan.journal;
        fs::create_dir(&j.staging).map_err(err)?;
        write(&marker(j), &j.nonce)?;
        write(&j.staging.join(".drawer-move-owner.json"), &j.nonce)?;
        for f in &j.source_files {
            let target = j.staging.join(&f.path);
            if f.directory {
                fs::create_dir_all(target).map_err(err)?;
            } else {
                fs::create_dir_all(target.parent().ok_or("Missing copy parent")?).map_err(err)?;
                fs::copy(j.source.join(&f.path), target).map_err(err)?;
            }
        }
        build_index_and_rebase(
            &j.staging,
            &j.source,
            &j.destination,
            false,
            &AtomicBool::new(false),
        )?;
        j.destination_files = manifest(&j.staging, &AtomicBool::new(false))?;
        write(&side(storage.anchor(), "move")?, j)?;
        fs::rename(&j.staging, &j.destination).map_err(err)?;
        if !browser::same(&resolve(storage.anchor())?, storage.root()) || j.destination.exists() {
            return Err("Uncommitted installed copy was selected after restart".into());
        }
        let bad = File {
            path: PathBuf::from("../escape"),
            directory: false,
            size: 0,
            hash: assets::hash(b""),
        };
        if valid_files(&[bad]) {
            return Err("Recovery accepted a traversing manifest".into());
        }
        let before = fs::read(&asset).map_err(err)?;
        let modified = asset.clone();
        let fail = dir.join("hash-failure");
        if Plan::prepare(&storage, &fail, false)?
            .run_with(&AtomicBool::new(false), &AtomicUsize::new(0), |step| {
                if step == "copied" {
                    fs::write(&modified, b"changed during copy").map_err(err)?;
                }
                Ok(())
            })
            .is_ok()
            || fail.exists()
        {
            return Err("Source hash mismatch was committed".into());
        }
        fs::write(&asset, &before).map_err(err)?;
        let missing = storage.root().join("settings/missing.json");
        write(
            &missing,
            &serde_json::json!({"source":storage.root().join("materials/missing.png")}),
        )?;
        if Plan::prepare(&storage, &dir.join("missing"), false)?
            .run(&AtomicBool::new(false), &AtomicUsize::new(0))
            .is_ok()
        {
            return Err("Missing managed asset was silently repaired".into());
        }
        fs::remove_file(missing).map_err(err)?;
        fs::write(storage.root().join("asset-index.json"), b"corrupt index").map_err(err)?;
        if Plan::prepare(&storage, &dir.join("bad-index"), false)?
            .run(&AtomicBool::new(false), &AtomicUsize::new(0))
            .is_ok()
        {
            return Err("Damaged index was replaced without review".into());
        }
        let out = Plan::prepare(&storage, &dir.join("새 위치"), true)?
            .run(&AtomicBool::new(false), &AtomicUsize::new(0))?;
        let (index, _) = assets::Index::read(&out.storage.root().join("asset-index.json"))?;
        if !browser::same(&index.root, &out.storage.root().join("materials"))
            || index.entries.is_empty()
            || fs::read(out.storage.root().join("asset-index.before-move.json")).map_err(err)?
                != b"corrupt index"
            || fs::read(&asset).map_err(err)? != before
        {
            return Err("Verified copy/index/original backup".into());
        }
        let restarted = UserStorage::at(anchor.clone()).map_err(err)?;
        if !browser::same(restarted.root(), out.storage.root()) {
            return Err("Restart selected a second writable root".into());
        }
        let value: serde_json::Value =
            read(&restarted.root().join("settings/test-reference.json"))?
                .ok_or("Reference missing")?;
        if value["source"]
            != serde_json::Value::String(
                restarted
                    .root()
                    .join("materials/한글.png")
                    .to_string_lossy()
                    .into(),
            )
            || value["document"] != serde_json::Value::String(userdoc.to_string_lossy().into())
        {
            return Err("Asset remap changed user document path".into());
        }
        if !crate::recent::RecentFiles::load(&restarted)
            .map_err(err)?
            .paths()
            .iter()
            .any(|p| browser::same(p, &userdoc))
        {
            return Err("Recent user document path was moved with materials".into());
        }
        // The pointer itself commits verified bytes; later normal settings writes
        // must not be mistaken for a failed move when cleanup was interrupted.
        let location: Location =
            read(&side(restarted.anchor(), "location")?)?.ok_or("Missing pointer")?;
        let mut committed = Plan::prepare(&restarted, &dir.join("unused"), false)?.journal;
        committed.nonce = location.nonce.clone();
        committed.destination = restarted.root().to_owned();
        committed.source = storage.root().canonicalize().map_err(err)?;
        committed.staging = committed
            .destination
            .parent()
            .ok_or("Missing parent")?
            .join(format!(".drawer-move-{}", committed.nonce));
        committed.destination_files = manifest(restarted.root(), &AtomicBool::new(false))?;
        write(&marker(&committed), &committed.nonce)?;
        write(&side(restarted.anchor(), "move")?, &committed)?;
        fs::write(
            restarted.root().join("settings/after-commit.txt"),
            b"new legitimate settings",
        )
        .map_err(err)?;
        if !browser::same(&resolve(restarted.anchor())?, restarted.root())
            || fs::read(restarted.root().join("settings/after-commit.txt")).map_err(err)?
                != b"new legitimate settings"
        {
            return Err("Post-commit recovery rolled back legitimate settings".into());
        }
        fs::write(
            restarted.root().join("settings/new.txt"),
            b"latest settings",
        )
        .map_err(err)?;
        let reverse = Plan::prepare(&restarted, &anchor, true)?
            .run(&AtomicBool::new(false), &AtomicUsize::new(0))?;
        if !browser::same(reverse.storage.root(), &anchor)
            || fs::read(reverse.storage.root().join("settings/new.txt")).map_err(err)?
                != b"latest settings"
            || !restarted.root().is_dir()
        {
            return Err("Verified reverse lost retained root/latest settings".into());
        }
        // Committed pointer is authoritative even after legitimate subsequent writes.
        let storage = UserStorage::at(anchor.clone()).map_err(err)?;
        if !browser::same(storage.root(), &anchor) {
            return Err("Reverse restart selected old root".into());
        }
        let custom = storage.root().join("custom-assets");
        fs::create_dir(&custom).map_err(err)?;
        fs::write(custom.join("추가.png"), &before).map_err(err)?;
        let index = assets::scan(
            &custom,
            true,
            None,
            &AtomicBool::new(false),
            &AtomicUsize::new(0),
        )?;
        index.write(&storage.root().join("asset-index.json"), None, true)?;
        write(
            &storage.root().join("settings/custom-source.json"),
            &serde_json::json!({"source":custom.join("추가.png")}),
        )?;
        let custom_out = Plan::prepare(&storage, &dir.join("custom-destination"), false)?
            .run(&AtomicBool::new(false), &AtomicUsize::new(0))?;
        let (index, _) = assets::Index::read(&custom_out.storage.root().join("asset-index.json"))?;
        let value: serde_json::Value = read(
            &custom_out
                .storage
                .root()
                .join("settings/custom-source.json"),
        )?
        .ok_or("Custom reference")?;
        if !browser::same(
            &index.root,
            &custom_out.storage.root().join("custom-assets"),
        ) || value["source"]
            != serde_json::Value::String(
                custom_out
                    .storage
                    .root()
                    .join("custom-assets/추가.png")
                    .to_string_lossy()
                    .into(),
            )
        {
            return Err("Nondefault managed asset root remained at the old location".into());
        }
        let backup = dir.join("tips-textures.json");
        crate::material_library::Library::default()
            .save(&backup)
            .map_err(err)?;
        fs::write(
            storage.root().join("materials/library.json"),
            b"broken tips",
        )
        .map_err(err)?;
        restore_materials(&storage, &backup)?;
        if fs::read(storage.root().join("materials/library.before-restore.json")).map_err(err)?
            != b"broken tips"
            || crate::material_library::Library::read(
                &storage.root().join("materials/library.json"),
            )
            .is_err()
        {
            return Err("Tip/texture restore failed or lost previous bytes".into());
        }
        Ok(())
    })();
    assets::cleanup_verification(root, &dir)?;
    result
}
#[cfg(test)]
mod tests {
    #[test]
    fn verified_root_migration_reverse_restart_cancel_faults_index_and_material_restore() {
        let t = crate::test_support::TempDir::new();
        super::run(&t.0).unwrap();
    }
}
