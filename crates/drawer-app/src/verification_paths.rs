use super::editor::{Editor, check};
use drawer_core::document::DEFAULT_DOCUMENT_BUDGET as B;
use std::{
    fs,
    path::{Path, PathBuf},
};
struct Restore {
    path: PathBuf,
    permissions: fs::Permissions,
}
impl Drop for Restore {
    fn drop(&mut self) {
        let _ = fs::set_permissions(&self.path, self.permissions.clone());
    }
}
fn unchanged(
    e: &Editor,
    before: &drawer_core::sizing::DocumentState,
    revision: u64,
    counts: (usize, usize),
    dirty: bool,
    path: &Option<PathBuf>,
) -> Result<(), String> {
    let d = &e.app.documents[0];
    check(
        d.snapshot() == *before
            && d.revision == revision
            && d.history_counts() == counts
            && d.dirty == dirty
            && d.path == *path,
        "Failed path operation changed drawing/history/path/dirty",
    )
}
pub(super) fn run(root: &Path) -> Result<serde_json::Value, String> {
    let mut e = Editor::new(1.)?;
    check(
        e.app.documents[0].edit_pixel(0, 1, 1, [123, 45, 67, 128]),
        "Path fixture edit failed",
    )?;
    let name = "한글🙂e\u{301}.drawer";
    let path = root.join(name);
    e.app.save_to(0, &path)?;
    let healthy = fs::read(&path).map_err(|e| e.to_string())?;
    check(
        e.app.documents[0].path.as_ref().and_then(|p| p.file_name())
            == Some(std::ffi::OsStr::new(name)),
        "Unicode filename changed on save",
    )?;
    check(
        drawer_core::files::open(&path, B)
            .map_err(|e| e.to_string())?
            .snapshot()
            == e.app.documents[0].snapshot(),
        "Unicode reopen changed drawing",
    )?;
    check(
        e.app.documents[0].edit_pixel(0, 2, 1, [19, 87, 142, 255]),
        "Unsaved path fixture edit failed",
    )?;
    let d = &e.app.documents[0];
    let before = d.snapshot();
    let revision = d.revision;
    let counts = d.history_counts();
    let dirty = d.dirty;
    let association = d.path.clone();
    let restore = Restore {
        path: path.clone(),
        permissions: fs::metadata(&path)
            .map_err(|e| e.to_string())?
            .permissions(),
    };
    let mut permissions = restore.permissions.clone();
    permissions.set_readonly(true);
    fs::set_permissions(&path, permissions).map_err(|e| e.to_string())?;
    let readonly = e
        .app
        .save_to(0, &path)
        .expect_err("Read-only save must fail");
    check(
        readonly.contains("read-only")
            && readonly.contains(name)
            && fs::read(&path).map_err(|e| e.to_string())? == healthy,
        "Read-only failure omitted guidance or replaced source",
    )?;
    unchanged(&e, &before, revision, counts, dirty, &association)?;
    drop(restore);
    #[cfg(windows)]
    let locked = {
        use std::os::windows::fs::OpenOptionsExt;
        let lock = fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&path)
            .map_err(|e| e.to_string())?;
        let error = e
            .app
            .save_to(0, &path)
            .expect_err("Exclusive lock must reject save");
        drop(lock);
        check(
            error.contains("locked")
                && error.contains(name)
                && fs::read(&path).map_err(|e| e.to_string())? == healthy,
            "Locked save omitted guidance or replaced source",
        )?;
        unchanged(&e, &before, revision, counts, dirty, &association)?;
        Some(error)
    };
    #[cfg(not(windows))]
    let locked: Option<String> = None;
    let foreign = path.with_file_name(format!("{name}.drawer-write.tmp"));
    fs::write(&foreign, b"another operation owns this file").map_err(|e| e.to_string())?;
    let collision = e
        .app
        .save_to(0, &path)
        .expect_err("Foreign temporary must reject save");
    check(
        !collision.is_empty()
            && fs::read(&foreign).map_err(|e| e.to_string())?
                == b"another operation owns this file"
            && fs::read(&path).map_err(|e| e.to_string())? == healthy,
        "Save collision changed foreign or normal file",
    )?;
    unchanged(&e, &before, revision, counts, dirty, &association)?;
    fs::remove_file(foreign).map_err(|e| e.to_string())?;
    let mut long = root.to_owned();
    for n in 0..5 {
        long.push(format!("한글경로-{n}-{}", "a".repeat(54)));
        fs::create_dir(&long).map_err(|e| e.to_string())?;
    }
    let long_path = long.join("긴경로🙂.drawer");
    let utf16 = long_path
        .as_os_str()
        .to_string_lossy()
        .encode_utf16()
        .count();
    check(
        utf16 > 260,
        "Long-path fixture did not exceed260UTF16 units",
    )?;
    e.app.save_to(0, &long_path)?;
    check(
        drawer_core::files::open(&long_path, B)
            .map_err(|e| e.to_string())?
            .snapshot()
            == e.app.documents[0].snapshot(),
        "Long-path reopen changed drawing",
    )?;
    let mut reserved = Vec::new();
    for name in [
        "CON.drawer",
        "NUL.drawer",
        "AUX.drawer",
        "COM1.drawer",
        "LPT9.drawer",
        "COM¹.drawer",
        "name.",
        "name ",
        "bad:name.drawer",
    ] {
        check(
            drawer_core::file_browser::new_path(&root.join(name)).is_err(),
            "Reserved/ambiguous component was admitted",
        )?;
        reserved.push(name);
    }
    check(
        drawer_core::file_browser::new_path(&root.join("..\\escape.drawer")).is_err(),
        "Parent traversal admitted",
    )?;
    let parent_is_file = root.join("not-a-folder");
    fs::write(&parent_is_file, b"keep").map_err(|e| e.to_string())?;
    check(
        e.app
            .save_to(0, &parent_is_file.join("child.drawer"))
            .is_err()
            && fs::read(&parent_is_file).map_err(|e| e.to_string())? == b"keep",
        "Invalid parent changed a file",
    )?;
    Ok(
        serde_json::json!({"unicodeName":name,"unicodeSourceSha256":drawer_core::asset_catalog::hash(&healthy),"unicodeLoss":0,"longPathUtf16Units":utf16,
        "readOnlyError":readonly,"exclusiveLockError":locked,"foreignTemporaryPreserved":true,"reservedNamesRejected":reserved,
        "traversalRejected":true,"invalidParentPreserved":true,"retrySaveReopen":true}),
    )
}
#[cfg(test)]
mod tests {
    #[test]
    fn unicode_long_readonly_exclusive_lock_reserved_and_retry_paths() {
        let root = crate::tests::TestDirectory::new();
        super::run(&root.0).unwrap();
    }
}
