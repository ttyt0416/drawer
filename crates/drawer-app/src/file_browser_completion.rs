use super::*;
pub(super) fn run(root: &Path) -> Result<(), String> {
    let dir = root.join("browser-completion-ui");
    std::fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let result = (|| {
        let ctx = egui::Context::default();
        let mut a = Drawer::new(&ctx, None);
        a.simple_view = true;
        let d = drawer_core::png_io_verify::fixture()?;
        a.add_document(d);
        let before = a.documents[0].snapshot();
        let first = dir.join("첫째.png");
        let second = dir.join("둘째.png");
        for p in [&first, &second] {
            drawer_core::png_export::export(&a.documents[0], p, 64 * 1024 * 1024)
                .map_err(|e| e.to_string())?;
        }
        a.browser_drop(&ctx, vec![first.clone(), second.clone()])?;
        if a.import_dialog.is_none() || a.file_browser.opens.len() != 1 {
            return Err("Drop overwrote sequential review".into());
        }
        a.import_dialog = None;
        a.file_browser_window(&ctx);
        if a.import_dialog.is_none() || !a.file_browser.opens.is_empty() {
            return Err("Next drop did not wait for image review".into());
        }
        a.import_dialog = None;
        a.browser_drop(&ctx, vec![dir.clone()])?;
        for n in 0..8000 {
            input_replay::frame(&ctx, &mut a, n as f64 * 0.02, vec![]);
            if a.file_browser.job.is_none() && a.file_browser.pending.is_none() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        if !a
            .file_browser
            .listing
            .as_ref()
            .is_some_and(|l| core::same(&l.folder, &dir))
        {
            return Err("Folder drop did not navigate".into());
        }
        let mut p = std::mem::take(&mut a.file_browser);
        p.address = first.display().to_string();
        a.browser_command(&mut p, &ctx, "go")?;
        a.file_browser = p;
        for n in 0..8000 {
            input_replay::frame(&ctx, &mut a, 200. + n as f64 * 0.02, vec![]);
            if a.import_dialog.is_some() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        if a.import_dialog.is_none() || a.documents[0].snapshot() != before {
            return Err("Direct file address/source isolation".into());
        }
        if a.browser_drop(&ctx, vec![dir.clone(), first]).is_ok()
            || a.browser_drop(&ctx, vec![PathBuf::from(r"\\server\share")])
                .is_ok()
        {
            return Err("Mixed/network drop accepted".into());
        }
        Ok(())
    })();
    drawer_core::asset_catalog::cleanup_verification(root, &dir)?;
    result
}
