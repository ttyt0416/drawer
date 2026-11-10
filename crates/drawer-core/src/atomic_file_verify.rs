use crate::{
    atomic_file::{self as a, Stage},
    document::{Background, CanvasSpec, DEFAULT_DOCUMENT_BUDGET as B, Document},
};
use std::{fs, io, path::Path};
fn check(b: bool, s: &str) -> Result<(), String> {
    if b { Ok(()) } else { Err(s.into()) }
}
pub fn run(root: &Path) -> Result<(), String> {
    let dir = root.join("atomic-save-fixture");
    fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let result = (|| {
        let path = dir.join("painting.drawer");
        let mut d = Document::create(
            CanvasSpec {
                width: 3,
                height: 2,
                dpi: 300.,
                background: Background::Transparent,
            },
            B,
        )
        .map_err(|e| e.to_string())?;
        d.edit_pixel(0, 0, 0, [10, 20, 30, 255]);
        crate::files::save(&mut d, &path).map_err(|e| e.to_string())?;
        let original = fs::read(&path).map_err(|e| e.to_string())?;
        d.edit_pixel(0, 1, 1, [60, 70, 80, 255]);
        let before = (
            d.snapshot(),
            d.revision,
            d.history_counts(),
            d.path.clone(),
            d.dirty,
        );
        let temp = path.with_file_name("painting.drawer.drawer-write.tmp");
        for fail in [
            Stage::Created,
            Stage::Written,
            Stage::Flushed,
            Stage::Synced,
            Stage::Verified,
            Stage::BackupReady,
            Stage::Replacing,
        ] {
            let r = crate::files::save_observed(&mut d, &path, &mut |stage| {
                if stage == fail {
                    if stage == Stage::Created {
                        fs::write(&temp, b"injected partial write")?;
                    }
                    return Err(io::Error::other(
                        "Injected write/flush/sync/replace interruption",
                    ));
                }
                Ok(())
            });
            check(
                r.is_err()
                    && fs::read(&path).map_err(|e| e.to_string())? == original
                    && !temp.exists(),
                "Interrupted save changed the healthy file or leaked its temporary",
            )?;
            check(
                (
                    d.snapshot(),
                    d.revision,
                    d.history_counts(),
                    d.path.clone(),
                    d.dirty,
                ) == before,
                "Failed save changed live document/history/dirty",
            )?;
        }
        // Corrupt the fully synchronized candidate before its reopen validation.
        check(
            crate::files::save_observed(&mut d, &path, &mut |s| {
                if s == Stage::Synced {
                    fs::write(&temp, b"corrupt candidate")?;
                }
                Ok(())
            })
            .is_err()
                && fs::read(&path).map_err(|e| e.to_string())? == original,
            "Unverified candidate was published",
        )?;
        // Simulated termination leaves an incomplete owned-name file; restart never treats it as a document.
        fs::write(&temp, b"partial interrupted payload").map_err(|e| e.to_string())?;
        check(
            crate::files::open(&path, B)
                .map_err(|e| e.to_string())?
                .layers[0]
                .pixel(3, 2, 0, 0)
                == [10, 20, 30, 255]
                && crate::files::save(&mut d, &path).is_err()
                && fs::read(&temp).map_err(|e| e.to_string())? == b"partial interrupted payload",
            "Restart consumed/deleted an unowned partial save",
        )?;
        fs::remove_file(&temp).map_err(|e| e.to_string())?;
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            let locked = fs::OpenOptions::new()
                .read(true)
                .share_mode(0)
                .open(&path)
                .map_err(|e| e.to_string())?;
            let failed = crate::files::save(&mut d, &path).is_err();
            drop(locked);
            check(
                failed && fs::read(&path).map_err(|e| e.to_string())? == original,
                "Windows sharing violation replaced the prior file",
            )?;
        }
        crate::files::save_observed(&mut d, &path, &mut |s| {
            if s == Stage::Published {
                fs::write(&temp, b"foreign observer file")?;
                Err(io::Error::other("Observer failed after publication"))
            } else {
                Ok(())
            }
        })
        .map_err(|e| e.to_string())?;
        check(
            fs::read(&temp).map_err(|e| e.to_string())? == b"foreign observer file",
            "Cleanup removed a foreign temporary created after publication",
        )?;
        fs::remove_file(&temp).map_err(|e| e.to_string())?;
        let new = crate::files::open(&path, B).map_err(|e| e.to_string())?;
        check(
            new.snapshot() == d.snapshot()
                && !d.dirty
                && fs::read(a::predecessor(&path)).map_err(|e| e.to_string())? == original,
            "Successful save/current dirty/healthy predecessor mismatch",
        )?;
        // A foreign/damaged predecessor is never silently overwritten.
        let previous = a::predecessor(&path);
        fs::write(&previous, b"foreign backup").map_err(|e| e.to_string())?;
        check(
            crate::files::save(&mut d, &path).is_err()
                && fs::read(&previous).map_err(|e| e.to_string())? == b"foreign backup",
            "Unverified predecessor was overwritten",
        )?;
        fs::write(&previous, &original).map_err(|e| e.to_string())?;
        let healthy_before = fs::read(&previous).map_err(|e| e.to_string())?;
        fs::write(&path, b"confirmed replacement of an unreadable original")
            .map_err(|e| e.to_string())?;
        let report = crate::files::save_report(&mut d, &path).map_err(|e| e.to_string())?;
        let retained = report
            .preserved_unverified
            .ok_or("Missing unreadable original preservation")?;
        check(
            fs::read(&retained).map_err(|e| e.to_string())?
                == b"confirmed replacement of an unreadable original"
                && fs::read(&previous).map_err(|e| e.to_string())? == healthy_before,
            "Unreadable bytes replaced the last healthy predecessor",
        )?;
        // An external writer wins before replacement; the app refuses to overwrite that newer file.
        let external = fs::read(&path).map_err(|e| e.to_string())?;
        d.edit_pixel(0, 2, 1, [90, 100, 110, 255]);
        check(
            crate::files::save_observed(&mut d, &path, &mut |s| {
                if s == Stage::Replacing {
                    fs::write(&path, &original)?;
                }
                Ok(())
            })
            .is_err()
                && fs::read(&path).map_err(|e| e.to_string())? == original
                && d.dirty,
            "Concurrent destination change was overwritten",
        )?;
        fs::write(&path, external).map_err(|e| e.to_string())?;
        Ok(())
    })();
    fs::remove_dir_all(&dir).map_err(|e| e.to_string())?;
    result
}
#[cfg(test)]
mod tests {
    #[test]
    fn verified_save_faults_partial_restart_lock_and_last_healthy() {
        let root = crate::test_support::TempDir::new();
        super::run(&root.0).unwrap();
    }
}
