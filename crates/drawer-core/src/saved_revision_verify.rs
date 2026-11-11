use crate::{document::Document, recovery::verify::fixture};
use std::{fs, path::Path};
fn check(b: bool, s: &str) -> Result<(), String> {
    if b { Ok(()) } else { Err(s.into()) }
}
fn state(
    d: &Document,
) -> (
    crate::sizing::DocumentState,
    u64,
    (usize, usize),
    bool,
    Option<std::path::PathBuf>,
) {
    (
        d.snapshot(),
        d.revision,
        d.history_counts(),
        d.dirty,
        d.path.clone(),
    )
}
pub fn run(root: &Path) -> Result<(), String> {
    let dir = root.join("saved-revision-fixture");
    fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let result = (|| {
        let path = dir.join("current.drawer");
        let mut d = fixture()?;
        let checkpoint = d.begin_save_checkpoint()?;
        let mut frozen = d.current_copy();
        let captured = d.snapshot();
        let target = path.clone();
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let (resume_tx, resume_rx) = std::sync::mpsc::channel();
        let writer = std::thread::spawn(move || -> Result<std::path::PathBuf, String> {
            crate::files::save_observed(&mut frozen, &target, &mut |stage| {
                if stage == crate::atomic_file::Stage::Written {
                    ready_tx.send(()).map_err(std::io::Error::other)?;
                    resume_rx
                        .recv_timeout(std::time::Duration::from_secs(15))
                        .map_err(std::io::Error::other)?;
                }
                Ok(())
            })
            .map_err(|e| e.to_string())?;
            Ok(frozen.path.unwrap())
        });
        ready_rx
            .recv_timeout(std::time::Duration::from_secs(15))
            .map_err(|e| e.to_string())?;
        d.edit_pixel(0, 0, 0, [20, 40, 60, 255]);
        resume_tx.send(()).map_err(|e| e.to_string())?;
        let written = writer.join().map_err(|_| "Checkpoint writer panicked")??;
        check(
            !d.complete_save_checkpoint(checkpoint, &written)? && d.dirty,
            "Later edit became clean after an older snapshot completed",
        )?;
        check(
            d.undo() && d.snapshot() == captured && !d.dirty && d.redo() && d.dirty,
            "Undo to saved checkpoint/Redo dirty boundary",
        )?;
        let before = state(&d);
        let checkpoint = d.begin_save_checkpoint()?;
        check(
            d.cancel_save_checkpoint(checkpoint.clone())
                && !d.cancel_save_checkpoint(checkpoint)
                && state(&d) == before,
            "Canceled save changed drawing/path/history/dirty",
        )?;
        // Capturing a save fences an explicit coalescing group at that logical state.
        let mut after = d.snapshot();
        after.layers[0].pixels[2] = [80, 90, 100, 255];
        d.commit_state_group(after, Some(77))?;
        let checkpoint = d.begin_save_checkpoint()?;
        let captured = d.snapshot();
        let counts = d.history_counts();
        let mut copy = d.current_copy();
        let mut after = d.snapshot();
        after.layers[0].pixels[3] = [110, 120, 130, 255];
        d.commit_state_group(after, Some(77))?;
        check(
            d.history_counts().0 == counts.0 + 1,
            "In-flight save state was mutated by command coalescing",
        )?;
        crate::files::save(&mut copy, &path).map_err(|e| e.to_string())?;
        check(
            !d.complete_save_checkpoint(checkpoint, copy.path.as_ref().unwrap())?
                && d.undo()
                && d.snapshot() == captured
                && !d.dirty,
            "Grouped edits lost the captured saved state",
        )?;
        let checkpoint = d.begin_save_checkpoint()?;
        let mut copy = d.current_copy();
        d.set_layer_comp_policy(crate::layer_comps::Policy::Current)?;
        crate::files::save(&mut copy, &path).map_err(|e| e.to_string())?;
        check(
            !d.complete_save_checkpoint(checkpoint, copy.path.as_ref().unwrap())? && d.dirty,
            "Non-Undo metadata edit became clean",
        )?;
        let old = d.begin_save_checkpoint()?;
        let newest = d.begin_save_checkpoint()?;
        let mut copy = d.current_copy();
        crate::files::save(&mut copy, &path).map_err(|e| e.to_string())?;
        d.complete_save_checkpoint(newest, copy.path.as_ref().unwrap())?;
        let before = state(&d);
        check(
            d.complete_save_checkpoint(old, copy.path.as_ref().unwrap())
                .is_err()
                && state(&d) == before,
            "Stale save completion changed a newer saved boundary",
        )?;
        let checkpoint = d.begin_save_checkpoint()?;
        let mut other = fixture()?;
        let other_before = state(&other);
        check(
            other
                .complete_save_checkpoint(checkpoint.clone(), &path)
                .is_err()
                && state(&other) == other_before,
            "Save result crossed documents",
        )?;
        d.cancel_save_checkpoint(checkpoint);
        let checkpoint = d.begin_save_checkpoint()?;
        d.path = Some(dir.join("renamed.drawer"));
        let before = state(&d);
        check(
            d.complete_save_checkpoint(checkpoint, &path).is_err() && state(&d) == before,
            "Late save restored a physically renamed destination",
        )?;
        let checkpoint = d.begin_save_checkpoint()?;
        d.light_table = Default::default();
        check(
            !d.complete_save_checkpoint(checkpoint, &dir.join("renamed.drawer"))? && d.dirty,
            "Replaced light-table state was falsely marked saved",
        )?;
        // A failed writer and explicit cancellation preserve the prior healthy file.
        let checkpoint = d.begin_save_checkpoint()?;
        let before = state(&d);
        let bytes = fs::read(&path).map_err(|e| e.to_string())?;
        let temp = dir.join("current.drawer.drawer-write.tmp");
        fs::write(&temp, b"foreign operation").map_err(|e| e.to_string())?;
        let mut copy = d.current_copy();
        check(
            crate::files::save(&mut copy, &path).is_err()
                && d.cancel_save_checkpoint(checkpoint)
                && state(&d) == before
                && fs::read(&path).map_err(|e| e.to_string())? == bytes,
            "Failed background writer changed live state/file",
        )?;
        Ok(())
    })();
    fs::remove_dir_all(&dir).map_err(|e| e.to_string())?;
    result
}
#[cfg(test)]
mod tests {
    #[test]
    fn saved_state_late_edits_group_fences_metadata_undo_cancel_stale_and_document_boundaries() {
        let root = crate::test_support::TempDir::new();
        super::run(&root.0).unwrap();
    }
}
