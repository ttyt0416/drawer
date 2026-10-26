//! Non-replacing publication on the same local volume, behind the OS boundary.
use std::{io, path::Path};
#[cfg(windows)]
pub fn publish_no_replace(source: &Path, destination: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn MoveFileExW(source: *const u16, destination: *const u16, flags: u32) -> i32;
    }
    let source: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
    let destination: Vec<u16> = destination
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    // SAFETY: The null-terminated UTF-16 buffers outlive this synchronous Kernel32
    // call. The ABI matches the SDK; WRITE_THROUGH does not permit replacement.
    let moved = unsafe { MoveFileExW(source.as_ptr(), destination.as_ptr(), 8) };
    if moved == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}
#[cfg(not(windows))]
pub fn publish_no_replace(source: &Path, destination: &Path) -> io::Result<()> {
    std::fs::hard_link(source, destination)
}
pub fn verify(root: &Path) -> Result<(), String> {
    let dir = root.join("native-no-replace");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let result = (|| {
        let source = dir.join("소스-file.png");
        let destination = dir.join("대상-file.png");
        std::fs::write(&source, b"new PNG bytes").map_err(|e| e.to_string())?;
        std::fs::write(&destination, b"foreign").map_err(|e| e.to_string())?;
        if publish_no_replace(&source, &destination).is_ok()
            || std::fs::read(&source).map_err(|e| e.to_string())? != b"new PNG bytes"
            || std::fs::read(&destination).map_err(|e| e.to_string())? != b"foreign"
        {
            return Err("Native file publication overwrote an existing destination".into());
        }
        std::fs::remove_file(&destination).map_err(|e| e.to_string())?;
        publish_no_replace(&source, &destination).map_err(|e| e.to_string())?;
        if std::fs::read(&destination).map_err(|e| e.to_string())? != b"new PNG bytes" {
            return Err("Native publication changed file bytes".into());
        }
        Ok(())
    })();
    let cleanup = std::fs::remove_dir_all(dir).map_err(|e| e.to_string());
    result.and(cleanup)
}
#[cfg(test)]
mod tests {
    #[test]
    fn unicode_local_publication_does_not_replace_a_foreign_destination() {
        let root =
            std::env::temp_dir().join(format!("drawer-native-publish-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        super::verify(&root).unwrap();
        std::fs::remove_dir(root).unwrap();
    }
}
