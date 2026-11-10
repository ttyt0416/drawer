use super::*;
pub(super) fn run(root: &Path) -> Result<(), String> {
    let dir = root.join("browser-completion-core");
    fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let result = (|| {
        let old = new_folder(&dir, "원본 folder")?;
        let png = old.join("Case.png");
        let mut d = crate::png_io_verify::fixture()?;
        crate::png_export::export(&d, &png, 64 * 1024 * 1024).map_err(|e| e.to_string())?;
        let bytes = fs::read(&png).map_err(|e| e.to_string())?;
        let ticket = Ticket::new(&png, Some("CASE.PNG"))?;
        let event = ticket.run(
            true,
            &AtomicBool::new(false),
            |a, b| fs::rename(a, b).map_err(|e| e.to_string()),
            |_, _| Err("No recycle in fixture".into()),
        )?;
        let Event::Renamed { new, .. } = event else {
            return Err("Case rename event".into());
        };
        if fs::read(&new).map_err(|e| e.to_string())? != bytes {
            return Err("Case rename bytes".into());
        }
        fs::write(old.join("other.png"), b"collision").map_err(|e| e.to_string())?;
        if Ticket::new(&new, Some("other.png")).is_ok() {
            return Err("Collision accepted".into());
        }
        let mismatch = old.join("disguised.jpg");
        fs::write(&mismatch, &bytes).map_err(|e| e.to_string())?;
        if crate::exchange_guidance::probe(&mismatch)
            .err()
            .is_none_or(|e| !e.contains("contents require .png"))
            || fs::read(&mismatch).map_err(|e| e.to_string())? != bytes
        {
            return Err("Extension mismatch not explicit or changed source".into());
        }
        let source = crate::file_objects_verify::source(&old, "linked.drawer", [10, 20, 30, 255])?;
        let (state, _) = crate::file_objects::create(&d, &source, "이름")?;
        d.commit_state(state)?;
        d.edit_pixel(0, 0, 0, [90, 80, 70, 255]);
        let counts = d.history_counts();
        let pixels = d.layers.clone();
        let to = dir.join("이동 folder");
        let old = old.canonicalize().map_err(|e| e.to_string())?;
        fs::rename(&old, &to).map_err(|e| e.to_string())?;
        let to = to.canonicalize().map_err(|e| e.to_string())?;
        if d.remap_managed_files(&old, &to) != 1
            || d.history_counts() != counts
            || d.layers != pixels
            || !d.scene.file_objects.entries[0].path.starts_with(&to)
        {
            return Err("Managed paths changed drawing/history or were not rebased".into());
        }
        if !d.undo()
            || !d.scene.file_objects.entries[0].path.starts_with(&to)
            || !d.redo()
            || !d.scene.file_objects.entries[0].path.starts_with(&to)
        {
            return Err("Undo resurrected old physical reference".into());
        }
        Ok(())
    })();
    crate::asset_catalog::cleanup_verification(root, &dir)?;
    result
}
