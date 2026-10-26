//! Newly owned page files become project resources only after the model edit succeeds.
use crate::{book_io, document::Document};
use std::{
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
};
static NEXT: AtomicU64 = AtomicU64::new(1);
pub struct Prepared {
    directory: PathBuf,
    pub paths: Vec<PathBuf>,
    keep: bool,
}
impl Drop for Prepared {
    fn drop(&mut self) {
        if !self.keep {
            for p in &self.paths {
                let _ = std::fs::remove_file(p);
            }
            let _ = std::fs::remove_dir(&self.directory);
        }
    }
}
impl Prepared {
    pub fn keep(mut self) {
        self.keep = true;
    }
}
pub fn prepare(
    pages: &[Document],
    directory: &Path,
    cancel: &AtomicBool,
) -> Result<Prepared, String> {
    if pages.is_empty() || pages.len() > 512 {
        return Err("Choose 1..512 prepared pages".into());
    }
    book_io::check_cancel(cancel)?;
    let parent = directory.canonicalize().map_err(|e| e.to_string())?;
    if !crate::font_library::local(&parent) || !parent.is_dir() {
        return Err("Choose a local resource folder".into());
    }
    let directory = parent.join(format!(
        "Drawer-pages-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir(&directory).map_err(|e| e.to_string())?;
    let mut stage = Prepared {
        directory,
        paths: vec![],
        keep: false,
    };
    for (i, d) in pages.iter().enumerate() {
        book_io::check_cancel(cancel)?;
        let path = stage.directory.join(format!("page-{:04}.drawer", i + 1));
        crate::files::write_new_native(d, &path).map_err(|e| e.to_string())?;
        stage.paths.push(path);
    }
    book_io::check_cancel(cancel)?;
    Ok(stage)
}

pub fn verify(root: &Path) -> Result<(), String> {
    let dir = root.join("owned-page-resource-verify");
    std::fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let result = (|| {
        let good = crate::document::Document::create(
            crate::document::CanvasSpec {
                width: 2,
                height: 2,
                dpi: 300.,
                background: crate::document::Background::Transparent,
            },
            4096,
        )
        .map_err(|e| e.to_string())?;
        let mut bad = good.clone();
        bad.width = 0;
        if prepare(&[good.clone(), bad], &dir, &AtomicBool::new(false)).is_ok()
            || std::fs::read_dir(&dir).unwrap().count() != 0
        {
            return Err("Failed page resource batch left a partial owned directory".into());
        }
        if prepare(std::slice::from_ref(&good), &dir, &AtomicBool::new(true)).is_ok()
            || std::fs::read_dir(&dir).unwrap().count() != 0
        {
            return Err("Cancelled resource batch wrote files".into());
        }
        let stage = prepare(std::slice::from_ref(&good), &dir, &AtomicBool::new(false))?;
        let folder = stage.directory.clone();
        let own = stage.paths[0].clone();
        let foreign = folder.join("foreign.txt");
        std::fs::write(&foreign, b"foreign").unwrap();
        drop(stage);
        if own.exists() || std::fs::read(&foreign).unwrap() != b"foreign" {
            return Err("Resource rollback deleted a foreign file or retained its own page".into());
        }
        std::fs::remove_file(&foreign).unwrap();
        std::fs::remove_dir(folder).unwrap();
        let path = dir.join("existing.drawer");
        std::fs::write(&path, b"foreign native").unwrap();
        if crate::files::write_new_native(&good, &path).is_ok()
            || std::fs::read(&path).unwrap() != b"foreign native"
        {
            return Err("New native resource overwrote a foreign file".into());
        }
        Ok(())
    })();
    let cleanup = std::fs::remove_dir_all(dir).map_err(|e| e.to_string());
    result.and(cleanup)
}
#[cfg(test)]
mod tests {
    #[test]
    fn staged_page_failures_cancel_and_foreign_file_preservation() {
        let root = crate::test_support::TempDir::new();
        super::verify(&root.0).unwrap();
    }
}
