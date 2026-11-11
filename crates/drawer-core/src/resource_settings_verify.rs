use super::*;
pub fn run(root: &Path) -> Result<(), String> {
    let storage =
        crate::storage::UserStorage::at(root.join("resource-config")).map_err(|e| e.to_string())?;
    let mut s = Settings {
        scratch: Some(storage.root().to_owned()),
        ram_mib: 64,
        max_dimension: 1024,
        recovery_mib: 16,
        ..Default::default()
    };
    s.save(&storage).map_err(|e| e.to_string())?;
    if Settings::load(&storage).map_err(|e| e.to_string())? != s {
        return Err("Resource settings restart mismatch".into());
    }
    probe(storage.root(), 1024 * 1024)?;
    if probe(storage.root(), 0).is_ok()
        || probe(&storage.root().join("detached"), 1024 * 1024).is_ok()
    {
        return Err("Scratch admission accepted missing/full storage".into());
    }
    let before =
        fs::read(storage.root().join("settings/resources.json")).map_err(|e| e.to_string())?;
    let mut invalid = s.clone();
    invalid.ram_mib = 0;
    if invalid.save(&storage).is_ok() {
        return Err("Invalid settings accepted".into());
    }
    let temp = storage.root().join("settings/resources-write.tmp");
    fs::write(&temp, b"foreign").map_err(|e| e.to_string())?;
    s.ram_mib = 128;
    if s.save(&storage).is_ok()
        || fs::read(storage.root().join("settings/resources.json")).map_err(|e| e.to_string())?
            != before
        || fs::read(&temp).map_err(|e| e.to_string())? != b"foreign"
    {
        return Err("Failed settings write changed previous settings".into());
    }
    fs::remove_file(temp).map_err(|e| e.to_string())?;
    let recovery = storage.root().join("recovery");
    fs::create_dir(&recovery).map_err(|e| e.to_string())?;
    fs::write(recovery.join("unresolved"), b"keep").map_err(|e| e.to_string())?;
    if recovery_usage(storage.root())? != 4 {
        return Err("Recovery quota inventory mismatch".into());
    }
    if fs::read(recovery.join("unresolved")).map_err(|e| e.to_string())? != b"keep" {
        return Err("Quota removed unresolved content".into());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use std::fs;
    #[test]
    fn persistence_failed_probe_and_unresolved_preservation() {
        let root =
            std::env::temp_dir().join(format!("drawer-resource-config-{}", std::process::id()));
        fs::create_dir(&root).unwrap();
        let r = super::run(&root);
        fs::remove_dir_all(root).unwrap();
        r.unwrap();
    }
}
