use super::*;
pub fn run(root: &Path) -> Result<(), String> {
    super::completion::run(root)?;
    let dir = root.join("file-browser-core");
    fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let result = (|| {
        let c = AtomicBool::new(false);
        let data = UserStorage::at(dir.join("data")).map_err(|e| e.to_string())?;
        let mut s = Settings {
            filter: Filter::All,
            ..Default::default()
        };
        let child = new_folder(&dir, "한글 폴더")?;
        let a = child.join("한글.png");
        let b = child.join("two.txt");
        let mut d = crate::png_io_verify::fixture()?;
        crate::png_export::export(&d, &a, 64 * 1024 * 1024).map_err(|e| e.to_string())?;
        fs::write(&b, b"original").map_err(|e| e.to_string())?;
        let listing = scan(&child, &s, &c)?;
        if listing.entries.len() != 2 {
            return Err("Folder listing lost Unicode entries".into());
        }
        s.folder = Some(child.clone());
        s.favorite(&child)?;
        s.sort = Sort::ExtensionModified;
        s.descending = true;
        s.view = View::Thumbnails;
        s.save(&data)?;
        if Settings::load(&data)? != s {
            return Err("Browser view/favorites did not restart".into());
        }
        if modified_label(Some(std::time::UNIX_EPOCH)) != "1970-01-01 00:00:00 UTC" {
            return Err("File date display epoch incorrect".into());
        }
        let mut rows = listing.entries.clone();
        rows[0].modified = Some(std::time::UNIX_EPOCH + std::time::Duration::from_secs(1));
        rows[1].modified = Some(std::time::UNIX_EPOCH + std::time::Duration::from_secs(2));
        for key in [
            Sort::Name,
            Sort::Extension,
            Sort::Modified,
            Sort::ExtensionModified,
        ] {
            let mut asc = rows.clone();
            sort(&mut asc, key, false);
            let mut desc = rows.clone();
            sort(&mut desc, key, true);
            if asc.iter().map(|e| &e.path).rev().collect::<Vec<_>>()
                != desc.iter().map(|e| &e.path).collect::<Vec<_>>()
            {
                return Err("File sort direction unstable".into());
            }
        }
        let old =
            fs::read(data.root().join("settings/file-browser.json")).map_err(|e| e.to_string())?;
        fs::write(
            data.root().join("settings/file-browser.json.write.tmp"),
            b"foreign",
        )
        .map_err(|e| e.to_string())?;
        if s.save(&data).is_ok()
            || fs::read(data.root().join("settings/file-browser.json"))
                .map_err(|e| e.to_string())?
                != old
        {
            return Err("Failed settings save changed previous data".into());
        }
        fs::remove_file(data.root().join("settings/file-browser.json.write.tmp"))
            .map_err(|e| e.to_string())?;
        for name in [
            "CON.png",
            "COM¹.png",
            "..",
            "x.",
            "bad/name",
            "bad:name",
            "trailing ",
        ] {
            if component(name).is_ok() {
                return Err(format!("Reserved name accepted: {name}"));
            }
        }
        if scan(Path::new(r"\\server\share"), &s, &c).is_ok()
            || scan(&child, &s, &AtomicBool::new(true)).is_ok()
        {
            return Err("Network/canceled folder scan accepted".into());
        }
        let thumb = thumbnail(&a, 64 * 1024 * 1024, &c)?;
        if thumb.size != [3, 1] || thumb.pixels.len() != 3 {
            return Err("PNG thumbnail content/dimensions".into());
        }
        let bad = child.join("broken.png");
        fs::write(&bad, b"bad").map_err(|e| e.to_string())?;
        if thumbnail(&bad, 64 * 1024 * 1024, &c).is_ok()
            || fs::read(&bad).map_err(|e| e.to_string())? != b"bad"
        {
            return Err("Corrupt thumbnail changed source".into());
        }
        let native = child.join("editable.drawer");
        crate::files::save(&mut d, &native).map_err(|e| e.to_string())?;
        if thumbnail(&native, 64 * 1024 * 1024, &c)?.pixels != thumb.pixels {
            return Err("Native thumbnail differs from PNG".into());
        }
        let t = Ticket::new(&b, Some("renamed.txt"))?;
        if t.run(false, &c, |_, _| Ok(()), |_, _| Ok(())).is_ok() {
            return Err("Unconfirmed file operation ran".into());
        }
        if t.run(true, &c, |_, _| Err("denied".into()), |_, _| Ok(()))
            .is_ok()
            || fs::read(&b).map_err(|e| e.to_string())? != b"original"
        {
            return Err("Failed rename changed source".into());
        }
        let event = t.run(
            true,
            &c,
            |a, b| {
                if b.exists() {
                    return Err("collision".into());
                }
                fs::rename(a, b).map_err(|e| e.to_string())
            },
            |_, _| Err("no recycle".into()),
        )?;
        let Event::Renamed { old, new } = event else {
            return Err("Rename result wrong".into());
        };
        if old.exists() || fs::read(&new).map_err(|e| e.to_string())? != b"original" {
            return Err("Rename changed bytes".into());
        }
        let t = Ticket::new(&new, None)?;
        if t.run(
            true,
            &c,
            |_, _| Ok(()),
            |_, _| Err("Recycle unavailable".into()),
        )
        .is_ok()
            || !new.exists()
        {
            return Err("Recycle failure used permanent deletion".into());
        }
        if Ticket::new(&new, Some("한글.png")).is_ok() {
            return Err("Rename collision accepted".into());
        }
        struct Writable(PathBuf);
        impl Drop for Writable {
            fn drop(&mut self) {
                if let Ok(m) = fs::metadata(&self.0) {
                    let mut p = m.permissions();
                    #[allow(clippy::permissions_set_readonly_false)]
                    p.set_readonly(false);
                    let _ = fs::set_permissions(&self.0, p);
                }
            }
        }
        let _restore_writable = Writable(new.clone());
        let mut permissions = fs::metadata(&new).map_err(|e| e.to_string())?.permissions();
        permissions.set_readonly(true);
        fs::set_permissions(&new, permissions).map_err(|e| e.to_string())?;
        if Ticket::new(&new, None).is_ok() {
            return Err("Read-only physical target accepted".into());
        }
        let mut writable = fs::metadata(&new).map_err(|e| e.to_string())?.permissions();
        #[allow(clippy::permissions_set_readonly_false)]
        writable.set_readonly(false);
        fs::set_permissions(&new, writable).map_err(|e| e.to_string())?;
        if let Some(mapped) = remap(
            &child.join("nested/file.png"),
            &child,
            &dir.join("renamed-folder"),
        ) {
            if !mapped.ends_with("nested/file.png") {
                return Err("Folder remap dropped suffix".into());
            }
        } else {
            return Err("Folder remap missing".into());
        }
        if remap(
            &dir.join("prefix-other/file.png"),
            &dir.join("prefix"),
            &child,
        )
        .is_some()
        {
            return Err("Path remap matched a partial component".into());
        }
        let long = new_folder(&child, &"긴".repeat(80))?;
        let deeper = new_folder(&long, &"x".repeat(100))?;
        fs::write(deeper.join("long.txt"), b"long").map_err(|e| e.to_string())?;
        if scan(
            &deeper,
            &Settings {
                filter: Filter::All,
                ..Default::default()
            },
            &c,
        )?
        .entries
        .len()
            != 1
        {
            return Err("Long path listing failed".into());
        }
        if !checked(&deeper)?.starts_with(dir.canonicalize().map_err(|e| e.to_string())?) {
            return Err("Long-path cleanup escaped exclusive fixture".into());
        }
        fs::remove_dir_all(&deeper).map_err(|e| e.to_string())?;
        if scan(&deeper, &s, &c).is_ok() {
            return Err("Disconnected/missing folder was silently replaced".into());
        }
        Ok(())
    })();
    crate::asset_catalog::cleanup_verification(root, &dir)?;
    result
}
#[cfg(test)]
mod tests {
    #[test]
    fn local_unicode_long_paths_settings_thumbs_and_reviewed_operations() {
        let t = crate::test_support::TempDir::new();
        super::run(&t.0).unwrap();
    }
}
