use std::{
    io::Write,
    path::{Path, PathBuf},
};
pub(super) fn smoke(root: &Path) -> Result<(), String> {
    drawer_core::local_paths_verify::run(root)?;
    for path in drawer_core::local_paths_verify::bad_paths(root) {
        if drawer_infer::inspect(&path, drawer_core::local_inference::Task::Body).is_ok() {
            return Err("Model bundle accepted a nonlocal or escaping path".into());
        }
    }
    Ok(())
}
pub(super) fn cli(mut args: impl Iterator<Item = std::ffi::OsString>) -> Result<(), String> {
    let root = PathBuf::from(args.next().ok_or("Missing fixture root")?);
    let link = PathBuf::from(args.next().ok_or("Missing local junction fixture")?);
    let receipt = PathBuf::from(args.next().ok_or("Missing receipt")?);
    smoke(&root)?;
    drawer_core::local_paths_verify::reject_junction(&link)?;
    let error = drawer_infer::inspect(
        &link.join("manifest.json"),
        drawer_core::local_inference::Task::Body,
    )
    .err()
    .ok_or("Model bundle followed a junction")?;
    if !error.to_lowercase().contains("reparse") {
        return Err("Model manifest was read before junction admission".into());
    }
    let receipt = drawer_core::file_browser::new_path(&receipt)?;
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(receipt)
        .map_err(|e| e.to_string())?;
    f.write_all(b"local_paths=true\njunction_refused_before_parse=true\ncache_preserved_local_relink=true\n").map_err(|e|e.to_string())?;
    f.sync_all().map_err(|e| e.to_string())
}
#[cfg(test)]
mod tests {
    #[test]
    fn local_model_and_cached_reference_admission() {
        let t = crate::tests::TestDirectory::new();
        super::smoke(&t.0).unwrap();
    }
}
