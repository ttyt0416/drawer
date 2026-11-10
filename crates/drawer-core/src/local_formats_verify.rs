use super::*;
pub fn run(root: &Path) -> Result<(), String> {
    let dir = root.join("local-format-support-core");
    std::fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let result = (|| {
        for name in [
            "IllustStudio",
            "ComicStudio / Manga Studio",
            "ibisPaint",
            "kakooyo!",
        ] {
            if !SUPPORT.iter().any(|r| {
                r.application == name
                    && r.kind == Kind::Document
                    && r.status == "Unverified / unsupported"
            }) {
                return Err("Missing document support-table row".into());
            }
        }
        if SUPPORT.iter().filter(|r| r.kind == Kind::Tool).count() != 4 {
            return Err("Missing explicit tool support rows".into());
        }
        let png = dir.join("valid.png");
        image::save_buffer(&png, &[10, 20, 30, 255], 1, 1, image::ColorType::Rgba8)
            .map_err(|e| e.to_string())?;
        let public = std::fs::read(&png).map_err(|e| e.to_string())?;
        let own = serde_json::to_vec(&crate::user_tools::Library::default())
            .map_err(|e| e.to_string())?;
        for ext in PRIVATE_EXTENSIONS {
            let path = dir.join(format!("disguised.{ext}"));
            std::fs::write(&path, &public).map_err(|e| e.to_string())?;
            if probe(&path).err().as_deref() != Some(GUIDE) {
                return Err(format!("Known private suffix bypass {ext}"));
            }
            if std::fs::read(&path).map_err(|e| e.to_string())? != public {
                return Err("Document refusal modified source".into());
            }
            std::fs::write(&path, &own).map_err(|e| e.to_string())?;
            let errors = [
                crate::user_tools::Library::read(&path)
                    .err()
                    .map(|e| e.to_string()),
                crate::brush_presets::Preset::read(&path)
                    .err()
                    .map(|e| e.to_string()),
                crate::auto_actions::Settings::read(&path).err(),
                crate::palette_exchange::Bundle::read(&path).err(),
            ];
            if errors.iter().any(|e| e.as_deref() != Some(GUIDE)) {
                return Err(format!("Preset reader bypass/guidance {ext}: {errors:?}"));
            }
            if crate::user_tools::Library::default()
                .save(&path, true)
                .err()
                .map(|e| e.to_string())
                .as_deref()
                != Some(GUIDE)
                || std::fs::read(&path).map_err(|e| e.to_string())? != own
            {
                return Err("Tool save disguised private format".into());
            }
        }
        let sqlite = b"SQLite format 3\0owned unverified container";
        if private_container(sqlite) != Some("SQLite container; application unverified") {
            return Err("Container signature misidentified application".into());
        }
        for ext in ["png", "tga", "drawer", "psd", "json"] {
            let p = dir.join(format!("renamed.{ext}"));
            std::fs::write(&p, sqlite).map_err(|e| e.to_string())?;
            if probe(&p).err().as_deref() != Some(GUIDE)
                || std::fs::read(p).map_err(|e| e.to_string())? != sqlite
            {
                return Err("Known unverified container bypassed via suffix".into());
            }
        }
        Ok(())
    })();
    crate::asset_catalog::cleanup_verification(root, &dir)?;
    result
}
#[cfg(test)]
mod tests {
    #[test]
    fn explicit_document_tool_matrix_suffix_disguise_and_no_partial_private_read() {
        let t = crate::test_support::TempDir::new();
        super::run(&t.0).unwrap();
    }
}
