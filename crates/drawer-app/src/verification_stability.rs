//! Bounded, owned stress and fault injection, without exhausting the host volume/RAM.
use super::editor::check;
use drawer_core::{atomic_file::Stage, document::DEFAULT_DOCUMENT_BUDGET as B};
use std::{
    path::Path,
    time::{Duration, Instant},
};
pub(super) fn run(root: &Path) -> Result<serde_json::Value, String> {
    drawer_core::resource_failure_verify::run()?;
    crate::resource_limits::run(root)?;
    drawer_core::atomic_file::verify::run(root)?;
    let mut editor = super::editor::Editor::new(1.)?;
    let mut d = editor.app.documents.remove(0);
    let limits = drawer_core::history_limits::Settings {
        steps: 8,
        memory_mib: 1,
        group_ms: 0,
    };
    editor.app.resources.current.history = limits;
    d.configure_history(limits)?;
    let path = root.join("stability.drawer");
    drawer_core::files::save(&mut d, &path).map_err(|e| e.to_string())?;
    let healthy = std::fs::read(&path).map_err(|e| e.to_string())?;
    check(
        d.edit_pixel(0, 0, 0, [31, 72, 103, 255]),
        "Fault fixture edit failed",
    )?;
    let before = (
        d.snapshot(),
        d.revision,
        d.history_counts(),
        d.dirty,
        d.path.clone(),
    );
    let mut faults = Vec::new();
    for stage in [
        Stage::Created,
        Stage::Written,
        Stage::Flushed,
        Stage::Synced,
        Stage::Verified,
        Stage::BackupReady,
        Stage::Replacing,
    ] {
        let e = drawer_core::files::save_observed(&mut d, &path, &mut |s| {
            if s == stage {
                Err(std::io::Error::new(
                    std::io::ErrorKind::StorageFull,
                    "Injected local disk full",
                ))
            } else {
                Ok(())
            }
        })
        .expect_err("Disk-full injection must fail");
        check(
            e.kind() == std::io::ErrorKind::StorageFull
                && std::fs::read(&path).map_err(|e| e.to_string())? == healthy
                && (
                    d.snapshot(),
                    d.revision,
                    d.history_counts(),
                    d.dirty,
                    d.path.clone(),
                ) == before,
            "Disk-full save changed original/live state",
        )?;
        faults.push(format!("{stage:?}"));
    }
    let mut corruptions = 0;
    let bad_path = root.join("damaged.drawer");
    for mode in 0..4 {
        let mut bytes = healthy.clone();
        match mode {
            0 => bytes.truncate(7),
            1 => {
                bytes.pop();
            }
            2 => {
                let end = bytes.len() - 1;
                bytes[end] ^= 1;
            }
            _ => bytes[8..12].copy_from_slice(&u32::MAX.to_le_bytes()),
        }
        std::fs::write(&bad_path, &bytes).map_err(|e| e.to_string())?;
        let started = Instant::now();
        check(
            drawer_core::files::open(&bad_path, B).is_err()
                && started.elapsed() < Duration::from_secs(5)
                && std::fs::read(&bad_path).map_err(|e| e.to_string())? == bytes
                && std::fs::read(&path).map_err(|e| e.to_string())? == healthy,
            "Malformed file changed input or exceeded time limit",
        )?;
        corruptions += 1;
    }
    let duration = if cfg!(test) {
        Duration::ZERO
    } else {
        Duration::from_secs(600)
    };
    let started = Instant::now();
    let mut cycles = 0u64;
    let mut saves = 0;
    let mut max_ms = 0f64;
    let mut max_bytes = 0;
    let mut checkpoints = Vec::new();
    while cycles < 768 || started.elapsed() < duration {
        let tick = Instant::now();
        let before = d.snapshot();
        let x = (cycles % 64) as u32;
        let y = ((cycles / 64) % 48) as u32;
        check(
            d.edit_pixel(
                0,
                x,
                y,
                [(cycles % 251) as u8, ((cycles / 251) % 251) as u8, 31, 255],
            ),
            "Stress edit unexpectedly failed",
        )?;
        let after = d.snapshot();
        check(
            d.undo() && d.snapshot() == before && d.redo() && d.snapshot() == after,
            "Stress Undo/Redo mismatch",
        )?;
        check(
            d.history_counts().0 <= 8 && d.history_bytes() <= 1024 * 1024,
            "Stress history exceeds configured limit",
        )?;
        max_bytes = max_bytes.max(d.allocation_bytes());
        cycles += 1;
        if cycles.is_multiple_of(256) {
            d.clear_session_history();
            drawer_core::files::save(&mut d, &path).map_err(|e| e.to_string())?;
            check(
                drawer_core::files::open(&path, B)
                    .map_err(|e| e.to_string())?
                    .snapshot()
                    == d.snapshot(),
                "Stress save/reopen mismatch",
            )?;
            saves += 1;
            editor.app.documents.push(d);
            editor.frame(vec![]);
            d = editor
                .app
                .documents
                .pop()
                .ok_or("Stress editor lost its document")?;
            if checkpoints.len() < 32 {
                checkpoints.push(serde_json::json!({"cycle":cycles,"seconds":started.elapsed().as_secs_f64(),"documentAllocationBytes":d.allocation_bytes()}));
            }
        }
        max_ms = max_ms.max(tick.elapsed().as_secs_f64() * 1000.);
        if !cfg!(test) {
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    check(max_ms < 5000., "Stress operation exceeded5s response bound")?;
    super::strokes::edge_cases(root)?;
    Ok(
        serde_json::json!({"mode":"owned CPU document/history/save/editor-frame soak", "durationSeconds":started.elapsed().as_secs_f64(),
        "minimumSoakSeconds":if cfg!(test){0}else{600},"cycles":cycles,"editorFrames":saves+2,"nativePresentationMeasured":false,"saveReopenCycles":saves,
        "diskFullInjectedStages":faults,"corruptInputsRejectedUnchanged":corruptions,"memoryBudgetFailureAndRecovery":true,
        "maxOperationMs":max_ms,"operationLimitMs":5000,"maxDocumentAllocationBytes":max_bytes,"checkpoints":checkpoints,
        "physicalDiskExhausted":false,"hostOutOfMemoryInduced":false,"degenerateEdgesAnd256LayerMerge":true}),
    )
}
#[cfg(test)]
mod tests {
    #[test]
    fn stress_limits_disk_full_faults_corrupt_inputs_and_edit_recovery() {
        let root = crate::tests::TestDirectory::new();
        super::run(&root.0).unwrap();
    }
}
