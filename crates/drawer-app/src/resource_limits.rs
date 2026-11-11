use super::*;
pub(super) fn run(root: &std::path::Path) -> Result<(), String> {
    std::fs::create_dir_all(root).map_err(|e| e.to_string())?;
    let dir = root.join("resource-limits-ui");
    std::fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let result: Result<(), String> = (|| {
        drawer_core::resource_plan_verify::run(&dir)?;
        let ctx = egui::Context::default();
        let mut app = Drawer::new(&ctx, None);
        app.create_canvas(CanvasSpec {
            width: 2,
            height: 2,
            dpi: 300.,
            background: Background::Transparent,
        })
        .map_err(|e| e.to_string())?;
        let path = dir.join("save.drawer");
        app.save_to(0, &path)?;
        app.documents[0].edit_pixel(0, 0, 0, [1, 2, 3, 255]);
        let before = std::fs::read(&path).map_err(|e| e.to_string())?;
        let revision = app.documents[0].revision;
        let history = app.documents[0].history_counts();
        let error = app
            .save_to_space(0, &path, Some(0))
            .expect_err("Zero space must reject");
        if !error.contains("available 0")
            || !app.documents[0].dirty
            || app.documents[0].revision != revision
            || app.documents[0].history_counts() != history
            || std::fs::read(&path).map_err(|e| e.to_string())? != before
        {
            return Err("Rejected save changed state or omitted cost".into());
        }
        if drawer_platform::file_browser::available_space(&dir)? == 0
            || drawer_platform::file_browser::available_space(std::path::Path::new(
                r"\\server\share",
            ))
            .is_ok()
        {
            return Err("Disk query boundary incorrect".into());
        }
        Ok(())
    })();
    std::fs::remove_dir_all(dir).map_err(|e| e.to_string())?;
    result?;
    println!("resource_limits=true");
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn denied_save_preserves_document_and_original() {
        let root = std::env::temp_dir().join(format!("drawer-resource-ui-{}", std::process::id()));
        std::fs::create_dir(&root).unwrap();
        let r = super::run(&root);
        std::fs::remove_dir_all(root).unwrap();
        r.unwrap();
    }
}
