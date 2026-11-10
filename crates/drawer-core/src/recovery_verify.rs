use crate::{
    document::{Background, CanvasSpec, DEFAULT_DOCUMENT_BUDGET as B, Document},
    recovery::{self as r, Session},
};
use std::{fs, path::Path};
fn check(b: bool, s: &str) -> Result<(), String> {
    if b { Ok(()) } else { Err(s.into()) }
}
pub fn fixture() -> Result<Document, String> {
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
    d.edit_pixel(0, 1, 1, [30, 60, 90, 255]);
    Ok(d)
}
pub fn run(root: &Path) -> Result<(), String> {
    let dir = root.join("recovery-fixture");
    fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let result = (|| {
        let mut saved = fixture()?;
        let source = dir.join("original.drawer");
        crate::files::save(&mut saved, &source).map_err(|e| e.to_string())?;
        let bytes = fs::read(&source).map_err(|e| e.to_string())?;
        saved.edit_pixel(0, 0, 0, [100, 120, 140, 255]);
        let unsaved = fixture()?;
        let before = (
            saved.snapshot(),
            saved.revision,
            saved.history_counts(),
            saved.dirty,
            saved.path.clone(),
        );
        let mut session = Session::create(&dir)?;
        session.checkpoint(&[&saved, &unsaved])?;
        check(
            r::scan(&dir)?.is_empty(),
            "Live session appeared in startup recovery",
        )?;
        let old = fs::read(session.directory.join("index.json")).map_err(|e| e.to_string())?;
        for fail in 0..6 {
            check(
                session
                    .checkpoint_observed(&[&saved, &unsaved], &mut |n| {
                        if n == fail {
                            Err("Injected interrupted recovery".into())
                        } else {
                            Ok(())
                        }
                    })
                    .is_err(),
                "Recovery failure was ignored",
            )?;
            check(
                fs::read(session.directory.join("index.json")).map_err(|e| e.to_string())? == old,
                "Failed recovery replaced the published index",
            )?;
        }
        check(
            (
                saved.snapshot(),
                saved.revision,
                saved.history_counts(),
                saved.dirty,
                saved.path.clone(),
            ) == before
                && fs::read(&source).map_err(|e| e.to_string())? == bytes,
            "Recovery changed live document/history/source bytes",
        )?;
        let directory = session.directory.clone();
        drop(session);
        let lock = directory.join("live.lock");
        let permissions = fs::metadata(&lock)
            .map_err(|e| e.to_string())?
            .permissions();
        let mut readonly = permissions.clone();
        readonly.set_readonly(true);
        fs::set_permissions(&lock, readonly).map_err(|e| e.to_string())?;
        let readonly_scan = r::scan(&dir);
        fs::set_permissions(&lock, permissions).map_err(|e| e.to_string())?;
        check(
            readonly_scan?.len() == 2,
            "Read-only closed recovery was mistaken for a live session",
        )?;
        let candidates = r::scan(&dir)?;
        check(
            candidates.len() == 2 && candidates.iter().all(|c| c.error.is_none()),
            "Closed recovery missing complete documents",
        )?;
        let recovered = candidates[0].open()?;
        check(
            recovered.snapshot() == saved.snapshot()
                && recovered.revision == saved.revision
                && recovered.dirty
                && recovered.path.is_none()
                && recovered.source_path.is_none()
                && recovered.history_counts() == (0, 0),
            "Recovery did not create an independent dirty current-state document",
        )?;
        // Cancel is a read-only decision: a later scan sees exactly the same candidates.
        check(
            r::scan(&dir)?.len() == 2 && fs::read(&source).map_err(|e| e.to_string())? == bytes,
            "Cancel/scan altered source or recovery",
        )?;
        let damaged = candidates[1].file.as_ref().unwrap();
        let mut bad = fs::read(damaged).map_err(|e| e.to_string())?;
        let last = bad.len() - 1;
        bad[last] ^= 1;
        fs::write(damaged, bad).map_err(|e| e.to_string())?;
        let candidates = r::scan(&dir)?;
        check(
            candidates.iter().filter(|c| c.error.is_some()).count() == 1
                && candidates[0].open().is_ok()
                && candidates[1].open().is_err(),
            "Partial corruption prevented healthy recovery or accepted corrupt bytes",
        )?;
        r::discard(&directory)?;
        check(
            !directory.exists() && fs::read(&source).map_err(|e| e.to_string())? == bytes,
            "Explicit recovery discard deleted original",
        )?;
        let session = Session::create(&dir)?;
        let path = session.directory.clone();
        session.finish()?;
        check(
            !path.exists(),
            "Clean-session cleanup left its scratch folder",
        )
    })();
    fs::remove_dir_all(&dir).map_err(|e| e.to_string())?;
    result
}
#[cfg(test)]
mod tests {
    #[test]
    fn recovery_current_state_faults_partial_corruption_live_lease_cancel_and_discard() {
        let root = crate::test_support::TempDir::new();
        super::run(&root.0).unwrap();
    }
    #[test]
    fn crash_recovery_after_forced_process_termination() {
        if let Some(root) = std::env::var_os("DRAWER_RECOVERY_CRASH_CHILD") {
            let root = std::path::PathBuf::from(root);
            let d = super::fixture().unwrap();
            let mut session = crate::recovery::Session::create(&root).unwrap();
            session.checkpoint(&[&d]).unwrap();
            std::fs::write(root.join("child-ready"), b"synchronized").unwrap();
            loop {
                std::thread::park();
            }
        }
        let root = crate::test_support::TempDir::new();
        let mut command = std::process::Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "recovery::verify::tests::crash_recovery_after_forced_process_termination",
                "--nocapture",
            ])
            .env("DRAWER_RECOVERY_CRASH_CHILD", &root.0)
            .stdout(std::process::Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000);
        }
        let mut child = command.spawn().unwrap();
        let start = std::time::Instant::now();
        while !root.0.join("child-ready").exists()
            && start.elapsed() < std::time::Duration::from_secs(15)
        {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let ready = root.0.join("child-ready").exists();
        let _ = child.kill();
        child.wait().unwrap();
        assert!(ready, "Recovery child never synchronized its snapshot");
        let candidates = crate::recovery::scan(&root.0).unwrap();
        assert_eq!(candidates.len(), 1);
        let d = candidates[0].open().unwrap();
        assert!(d.dirty && d.path.is_none() && d.history_counts() == (0, 0));
        assert_eq!(d.snapshot(), super::fixture().unwrap().snapshot());
        crate::recovery::discard(&candidates[0].session).unwrap();
    }
}
