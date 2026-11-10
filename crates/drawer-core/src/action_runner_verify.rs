use super::*;
use crate::document::{Background, CanvasSpec, DEFAULT_DOCUMENT_BUDGET};
fn doc() -> Result<Document, String> {
    Document::create(
        CanvasSpec {
            width: 3,
            height: 2,
            dpi: 300.,
            background: Background::Transparent,
        },
        DEFAULT_DOCUMENT_BUDGET,
    )
    .map_err(|e| e.to_string())
}
pub fn fixture() -> Action {
    Action {
        id: 1,
        name: "Resume without duplicate".into(),
        steps: vec![
            crate::auto_actions::Step {
                id: 2,
                enabled: true,
                operation: crate::auto_actions::Operation::NewRaster {
                    name: "Once".into(),
                },
            },
            crate::auto_actions::Step {
                id: 3,
                enabled: true,
                operation: crate::auto_actions::Operation::RenameRaster {
                    layer: 99,
                    name: "Missing".into(),
                },
            },
            crate::auto_actions::Step {
                id: 4,
                enabled: true,
                operation: crate::auto_actions::Operation::Dpi(720.),
            },
        ],
    }
}
pub fn run(root: &Path) -> Result<(), String> {
    let mut source = doc()?;
    let before = source.snapshot();
    let identity = (source.path.clone(), source.dirty, source.history_counts());
    let flag = AtomicBool::new(false);
    let mut s = Session::new(&source, fixture(), DEFAULT_DOCUMENT_BUDGET)?;
    s.advance(Errors::Stop, &flag)?;
    if s.cursor != 1 || s.document().layers.len() != source.layers.len() + 1 {
        return Err("First step not retained".into());
    }
    flag.store(true, Ordering::Relaxed);
    s.run(Errors::Stop, &flag)?;
    flag.store(false, Ordering::Relaxed);
    for _ in 0..2 {
        if s.run(Errors::Stop, &flag).is_ok() || s.cursor != 1 {
            return Err("Error advanced cursor or retried completed step".into());
        }
    }
    if source.snapshot() != before
        || identity != (source.path.clone(), source.dirty, source.history_counts())
    {
        return Err("Prepared action mutated original".into());
    }
    s.skip()?;
    s.run(Errors::Stop, &flag)?;
    if !s.complete()
        || s.document().layers.len() != source.layers.len() + 1
        || s.document().dpi != 720.
    {
        return Err("Action resume duplicated/missed steps".into());
    }
    s.commit(&mut source)?;
    if source.history_counts() != (1, 0)
        || !source.undo()
        || source.snapshot() != before
        || !source.redo()
    {
        return Err("Resumable action not one Undo/Redo".into());
    }
    let stale = Session::new(&source, fixture(), DEFAULT_DOCUMENT_BUDGET)?;
    let mut stale = stale;
    stale.run(Errors::Skip, &flag)?;
    source.revision += 1;
    if stale.commit(&mut source).is_ok() {
        return Err("Stale result accepted".into());
    }
    let dir = root.join("action-runner-core");
    fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let result = (|| {
        let a = dir.join("한글.drawer");
        let bad = dir.join("bad.drawer");
        let b = dir.join("third.drawer");
        crate::files::save(&mut doc()?, &a).map_err(|e| e.to_string())?;
        crate::files::save(&mut doc()?, &b).map_err(|e| e.to_string())?;
        fs::write(&bad, b"broken").map_err(|e| e.to_string())?;
        let hashes = [&a, &bad, &b].map(|p| crate::asset_catalog::hash(&fs::read(p).unwrap()));
        let inputs = vec![a.clone(), bad.clone(), b.clone()];
        let mut batch = Batch::new(
            inputs.clone(),
            &dir,
            "processed-{index}",
            fixture(),
            DEFAULT_DOCUMENT_BUDGET,
        )?;
        if batch.run(Errors::Stop, Errors::Stop, &flag).is_ok()
            || batch.cursor != 0
            || batch.current.as_ref().unwrap().cursor != 1
        {
            return Err("Batch failed step not resumable".into());
        }
        batch.current.as_mut().unwrap().skip()?;
        if batch.run(Errors::Stop, Errors::Stop, &flag).is_ok()
            || batch.cursor != 1
            || batch.results.len() != 1
        {
            return Err("Bad second file did not stop after first output".into());
        }
        let first = fs::read(&batch.outputs[0]).map_err(|e| e.to_string())?;
        batch.skip_file()?;
        batch.run(Errors::Skip, Errors::Stop, &flag)?;
        if !batch.complete()
            || batch.results.len() != 3
            || fs::read(&batch.outputs[0]).map_err(|e| e.to_string())? != first
        {
            return Err("Batch resume duplicated completed file".into());
        }
        let output = crate::files::open(&batch.outputs[2], DEFAULT_DOCUMENT_BUDGET)
            .map_err(|e| e.to_string())?;
        if output.dpi != 720. || output.layers.len() != doc()?.layers.len() + 1 {
            return Err("Batch output lost action".into());
        }
        if hashes != [&a, &bad, &b].map(|p| crate::asset_catalog::hash(&fs::read(p).unwrap())) {
            return Err("Batch modified originals".into());
        }
        let log: serde_json::Value =
            serde_json::from_slice(&fs::read(&batch.log).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?;
        if log["next_file"] != 3 || log["results"].as_array().map(Vec::len) != Some(3) {
            return Err("Local result log wrong".into());
        }
        if Batch::new(
            inputs,
            &dir,
            "processed-{index}",
            fixture(),
            DEFAULT_DOCUMENT_BUDGET,
        )
        .is_ok()
        {
            return Err("Existing outputs were accepted".into());
        }
        let mut skip = Batch::new(
            vec![bad, a.clone()],
            &dir,
            "skip-{index}",
            fixture(),
            DEFAULT_DOCUMENT_BUDGET,
        )?;
        skip.run(Errors::Skip, Errors::Skip, &flag)?;
        if !skip.complete() || skip.results[0].output.is_some() || skip.results[1].output.is_none()
        {
            return Err("Skip failed file stopped remaining inputs".into());
        }
        let mut changed = Batch::new(
            vec![a.clone()],
            &dir,
            "changed-{index}",
            fixture(),
            DEFAULT_DOCUMENT_BUDGET,
        )?;
        changed.current = Some(Session::new(
            &crate::files::open(&a, DEFAULT_DOCUMENT_BUDGET).map_err(|e| e.to_string())?,
            fixture(),
            DEFAULT_DOCUMENT_BUDGET,
        )?);
        changed.stamp = Some(crate::file_sources::Stamp::read(&a)?);
        fs::write(&a, b"changed").map_err(|e| e.to_string())?;
        if changed.run(Errors::Skip, Errors::Stop, &flag).is_ok() || changed.outputs[0].exists() {
            return Err("Changed source was exported".into());
        }
        let mut pending = Batch::new(
            vec![b],
            &dir,
            "pending-{index}",
            fixture(),
            DEFAULT_DOCUMENT_BUDGET,
        )?;
        // A foreign log edit must stop without replaying already completed work.
        fs::write(&pending.log, b"foreign").map_err(|e| e.to_string())?;
        if pending.run(Errors::Skip, Errors::Stop, &flag).is_ok()
            || pending.cursor != 0
            || pending.outputs[0].exists()
        {
            return Err("Foreign log overwritten".into());
        }
        Ok(())
    })();
    crate::asset_catalog::cleanup_verification(root, &dir)?;
    result
}
#[cfg(test)]
mod tests {
    #[test]
    fn resumable_steps_files_logs_failures_and_originals() {
        let t = crate::test_support::TempDir::new();
        super::run(&t.0).unwrap();
    }
}
