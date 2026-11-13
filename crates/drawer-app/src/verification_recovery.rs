use super::{
    editor::check,
    native::{read, write},
};
use drawer_core::{
    asset_catalog::hash,
    atomic_file::Stage,
    document::{Background, CanvasSpec, DEFAULT_DOCUMENT_BUDGET as B, Document},
};
use serde::{Deserialize, Serialize};
use std::{
    path::Path,
    process::{Child, Stdio},
    time::{Duration, Instant},
};
const STAGES: [Stage; 8] = [
    Stage::Created,
    Stage::Written,
    Stage::Flushed,
    Stage::Synced,
    Stage::Verified,
    Stage::BackupReady,
    Stage::Replacing,
    Stage::Published,
];
const NEW: [u8; 4] = [200, 40, 80, 255];
const UNSAVED: [u8; 4] = [9, 80, 170, 128];
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    format: String,
    version: u32,
    stage: usize,
    original_sha256: String,
}
struct OwnedChild(Child);
impl Drop for OwnedChild {
    fn drop(&mut self) {
        if !matches!(self.0.try_wait(), Ok(Some(_))) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}
fn page() -> Result<Document, String> {
    Document::create(
        CanvasSpec {
            width: 3,
            height: 2,
            dpi: 300.,
            background: Background::Transparent,
        },
        B,
    )
    .map_err(|e| e.to_string())
}
pub(super) fn worker(root: &Path, expected: &str) -> Result<(), String> {
    check(
        drawer_core::asset_catalog::valid_hash(expected),
        "Invalid crash manifest hash",
    )?;
    let root = drawer_core::file_browser::folder(root)?;
    let bytes = read(&root.join("crash.json"), 4096)?;
    check(hash(&bytes) == expected, "Crash manifest changed")?;
    let config: Config = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    check(
        config.format == "Drawer owned save crash"
            && config.version == 1
            && config.stage < 8
            && drawer_core::asset_catalog::valid_hash(&config.original_sha256),
        "Invalid owned crash configuration",
    )?;
    let target = root.join("document.drawer");
    check(
        hash(&read(&target, 1024 * 1024)?) == config.original_sha256,
        "Original crash document changed",
    )?;
    let mut saved = drawer_core::files::open(&target, B).map_err(|e| e.to_string())?;
    check(saved.edit_pixel(0, 0, 0, NEW), "Crash worker edit failed")?;
    let mut unsaved = page()?;
    unsaved.title = "저장하지 않은 그림".into();
    check(
        unsaved.edit_pixel(0, 0, 0, UNSAVED),
        "Unsaved fixture edit failed",
    )?;
    let storage =
        drawer_core::storage::UserStorage::at(root.join("profile")).map_err(|e| e.to_string())?;
    let mut session = drawer_core::recovery::Session::create(storage.root())?;
    session.checkpoint(&[&saved, &unsaved])?;
    drawer_core::files::save_observed(&mut saved, &target, &mut |stage| {
        if stage == STAGES[config.stage] {
            let ready = serde_json::to_vec(
                &serde_json::json!({"version":1,"stage":config.stage,"pid":std::process::id()}),
            )
            .map_err(std::io::Error::other)?;
            let pending = root.join("ready-write.tmp");
            write(&pending, &ready).map_err(std::io::Error::other)?;
            std::fs::rename(pending, root.join("ready.json"))?;
            std::hint::black_box(&session);
            loop {
                std::thread::park();
            }
        }
        Ok(())
    })
    .map_err(|e| e.to_string())?;
    Err("Requested save stage was never reached".into())
}
pub(super) fn run(root: &Path) -> Result<serde_json::Value, String> {
    drawer_core::atomic_file::verify::run(root)?;
    drawer_core::recovery::generations_verify::run(root)?;
    crate::recovery::verify::run(root)?;
    let mut cases = vec![];
    for (stage_index, stage) in STAGES.into_iter().enumerate() {
        let dir = root.join(format!("forced-{stage_index}"));
        std::fs::create_dir(&dir).map_err(|e| e.to_string())?;
        let mut old = page()?;
        old.edit_pixel(0, 0, 0, [10, 20, 30, 255]);
        let target = dir.join("document.drawer");
        drawer_core::files::save(&mut old, &target).map_err(|e| e.to_string())?;
        let original = read(&target, 1024 * 1024)?;
        let config = Config {
            format: "Drawer owned save crash".into(),
            version: 1,
            stage: stage_index,
            original_sha256: hash(&original),
        };
        let bytes = serde_json::to_vec(&config).map_err(|e| e.to_string())?;
        write(&dir.join("crash.json"), &bytes)?;
        let expected = hash(&bytes);
        let mut command =
            std::process::Command::new(std::env::current_exe().map_err(|e| e.to_string())?);
        if cfg!(test) {
            command.args(["--exact","verification::recovery::tests::save_stage_failure_and_real_termination_recover_only_complete_documents","--nocapture"]).env("DRAWER_VERIFY_CRASH_ROOT",&dir).env("DRAWER_VERIFY_CRASH_HASH",&expected);
        } else {
            command.arg("--verify-save-crash").arg(&dir).arg(&expected);
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000);
        }
        let errors = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(dir.join("worker-errors.txt"))
            .map_err(|e| e.to_string())?;
        command.stdout(Stdio::null()).stderr(errors);
        let mut child = OwnedChild(command.spawn().map_err(|e| e.to_string())?);
        let start = Instant::now();
        while !dir.join("ready.json").exists() {
            if let Some(status) = child.0.try_wait().map_err(|e| e.to_string())? {
                return Err(format!(
                    "Crash worker exited before {stage:?}: {status}: {}",
                    String::from_utf8_lossy(&read(&dir.join("worker-errors.txt"), 16384)?)
                ));
            }
            if start.elapsed() > Duration::from_secs(30) {
                return Err(format!("Crash worker timed out before {stage:?}"));
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let ready: serde_json::Value =
            serde_json::from_slice(&read(&dir.join("ready.json"), 4096)?)
                .map_err(|e| e.to_string())?;
        check(
            ready["stage"] == stage_index && ready["pid"] == child.0.id(),
            "Crash worker marker identity differs",
        )?;
        child.0.kill().map_err(|e| e.to_string())?;
        let status = child.0.wait().map_err(|e| e.to_string())?;
        check(
            !status.success(),
            "Crash worker terminated normally instead of being killed",
        )?;
        let bytes = read(&target, 1024 * 1024)?;
        let main = drawer_core::files::open(&target, B)
            .map_err(|e| format!("{stage:?} normal document is incomplete: {e}"))?;
        if stage == Stage::Published {
            check(
                main.layers[0].pixels[0] == NEW,
                "Published stage lost the complete new generation",
            )?;
        } else {
            check(
                bytes == original,
                "Pre-publication termination replaced the healthy original generation",
            )?;
        }
        let profile = dir.join("profile");
        let candidates = drawer_core::recovery::scan(&profile)?;
        check(
            candidates.len() == 2 && candidates.iter().all(|c| c.error.is_none()),
            "Restart did not find exactly two complete recovery documents",
        )?;
        let mut unsaved = false;
        let mut saved = false;
        for c in candidates {
            let d = c.open()?;
            check(
                d.path.is_none() && d.dirty,
                "Recovered document would overwrite its original automatically",
            )?;
            if c.original.is_none() {
                unsaved = true;
                check(
                    d.layers[0].pixels[0] == UNSAVED,
                    "Unsaved recovery pixels differ",
                )?;
            } else {
                saved = true;
                check(
                    d.layers[0].pixels[0] == NEW,
                    "Saved-document recovery lost the uncommitted edit",
                )?;
            }
        }
        check(
            saved && unsaved,
            "Saved/unsaved recovery identities were merged",
        )?;
        let normal = std::fs::read_dir(&dir)
            .map_err(|e| e.to_string())?
            .filter_map(Result::ok)
            .filter(|e| e.path().extension().is_some_and(|v| v == "drawer"))
            .count();
        check(
            normal == 1,
            "Intermediate save file appeared as a normal document",
        )?;
        cases.push(serde_json::json!({"stage":format!("{stage:?}"),"forcedTermination":true,"reaped":true,"normalGeneration":if stage==Stage::Published{"new-complete"}else{"original-complete"},"normalNativeSha256":hash(&bytes),"recoveredSavedAndUnsaved":true,"temporaryNormalDocuments":0}));
    }
    Ok(
        serde_json::json!({"saveStageCrashes":cases,"forcedTerminations":8,"stageErrorInjection":true,"recoveryGenerationValidation":true,"startupRecoveryDialog":true,"unsavedRecoveryGuidance":true,"hardwarePowerLossValidated":false}),
    )
}
#[cfg(test)]
mod tests {
    #[test]
    fn save_stage_failure_and_real_termination_recover_only_complete_documents() {
        if let Some(root) = std::env::var_os("DRAWER_VERIFY_CRASH_ROOT") {
            let expected = std::env::var("DRAWER_VERIFY_CRASH_HASH").unwrap();
            super::worker(Path::new(&root), &expected).unwrap();
            return;
        }
        let r = crate::tests::TestDirectory::new();
        super::run(&r.0).unwrap();
    }
    use std::path::Path;
}
