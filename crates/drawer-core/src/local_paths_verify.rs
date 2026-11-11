//! Owned fixtures for local admission, blocked embedded references and retained caches.
use crate::{document::DEFAULT_DOCUMENT_BUDGET as B, file_browser as paths};
use std::{
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
};
pub fn bad_paths(root: &Path) -> Vec<PathBuf> {
    vec![
        "http://drawer.invalid/resource.drawer",
        "https://drawer.invalid/resource.drawer",
        "file://drawer.invalid/share/resource.drawer",
        r"\\drawer.invalid\share\resource.drawer",
        r"\\?\UNC\drawer.invalid\share\resource.drawer",
    ]
    .into_iter()
    .map(PathBuf::from)
    .chain([PathBuf::from(format!(
        r"{}\..\escape.drawer",
        root.display()
    ))])
    .collect()
}
pub fn run(root: &Path) -> Result<(), String> {
    let root = paths::folder(root)?;
    let dir = root.join("local-reference-admission");
    std::fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let result = (|| {
        let mut doc = crate::layer_roles_verify::page()?;
        let before = doc.snapshot();
        let counts = doc.history_counts();
        let old_path = doc.path.clone();
        let dirty = doc.dirty;
        let c = AtomicBool::new(false);
        for bad in bad_paths(&dir) {
            if paths::checked(&bad).is_ok()
                || paths::new_path(&bad).is_ok()
                || crate::files::open(&bad, B).is_ok()
                || crate::files::save(&mut doc, &bad).is_ok()
                || crate::image_import::ImageSource::read(&bad, B, &c).is_ok()
                || crate::movie::Source::read(&bad, B).is_ok()
                || crate::movie::Decoder::at(bad.clone()).is_ok()
                || crate::file_sources::Stamp::read(&bad).is_ok()
                || crate::asset_catalog::bytes(&bad, 1024).is_ok()
                || crate::storage::UserStorage::at(bad.clone()).is_ok()
            {
                return Err(format!(
                    "A nonlocal/traversing input reached a local operation: {bad:?}"
                ));
            }
        }
        if doc.snapshot() != before
            || doc.history_counts() != counts
            || doc.path != old_path
            || doc.dirty != dirty
        {
            return Err("Rejected destination changed the document".into());
        }
        for href in [
            "https://drawer.invalid/pixel.png",
            r"\\drawer.invalid\share\pixel.png",
            "../escape.png",
            "%2e%2e/escape.png",
        ] {
            let svg = format!(
                r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="4" height="4"><rect width="4" height="4" fill="red"/><image xlink:href="{href}" width="4" height="4"/></svg>"#
            );
            let preview = crate::svg_import::prepare(&doc, &svg, Some(&dir))?;
            if preview.images != 0 || preview.warnings.is_empty() {
                return Err("Blocked SVG reference was not reported".into());
            }
        }
        // Existing end-to-end fixture: no automatic refresh, missing source opens
        // from Native cache, explicit local relink, one Undo, original preservation.
        crate::file_objects_verify::verify(&dir)?;
        Ok(())
    })();
    crate::asset_catalog::cleanup_verification(&root, &dir)?;
    result
}
pub fn reject_junction(link: &Path) -> Result<(), String> {
    let mut doc = crate::layer_roles_verify::page()?;
    let failures = [
        paths::checked(&link.join("probe.drawer")).err(),
        paths::new_path(&link.join("new.drawer")).err(),
        crate::files::open(&link.join("probe.drawer"), B)
            .err()
            .map(|e| e.to_string()),
        crate::files::save(&mut doc, &link.join("new.drawer"))
            .err()
            .map(|e| e.to_string()),
        crate::movie::Source::read(&link.join("probe.mp4"), B).err(),
        crate::movie::Decoder::at(link.join("ffmpeg.exe")).err(),
        crate::storage::UserStorage::at(link.join("new-data"))
            .err()
            .map(|e| e.to_string()),
        crate::asset_catalog::bytes(&link.join("probe.drawer"), 1024).err(),
        crate::model_import::load(&link.join("probe.obj"), B, &AtomicBool::new(false)).err(),
    ];
    if failures.iter().any(|e| {
        e.as_ref()
            .is_none_or(|s| !s.to_lowercase().contains("reparse"))
    }) {
        return Err(format!(
            "Junction admission did not stop before file parsing: {failures:?}"
        ));
    }
    let root = paths::folder(link.parent().ok_or("Missing fixture parent")?)?;
    let source =
        crate::file_objects_verify::source(&root, "cache-seed.drawer", [211, 22, 70, 255])?;
    let (state, id) = crate::file_objects::create(&doc, &source, "Cached reference")?;
    doc.commit_state(state)?;
    let key = crate::layer_search::Key::Object(id);
    doc.scene.file_objects.entries[0].path = link.join("probe.drawer");
    let native = root.join("junction-cache.drawer");
    crate::files::save(&mut doc, &native).map_err(|e| e.to_string())?;
    std::fs::remove_file(source).map_err(|e| e.to_string())?;
    let mut reopened = crate::files::open(&native, B).map_err(|e| e.to_string())?;
    let before = reopened.snapshot();
    if crate::render::Renderer::new(&reopened).pixel([0.5; 2]) != [211, 22, 70, 255]
        || crate::file_objects::refresh(&reopened, key, None).is_ok()
        || reopened.snapshot() != before
    {
        return Err("Native cache was refreshed through a junction".into());
    }
    let replacement =
        crate::file_objects_verify::source(&root, "local-replacement.drawer", [31, 170, 90, 255])?;
    let state = crate::file_objects::refresh(&reopened, key, Some(&replacement))?;
    reopened.commit_state(state)?;
    if crate::render::Renderer::new(&reopened).pixel([0.5; 2]) != [31, 170, 90, 255]
        || !reopened.undo()
        || reopened.snapshot() != before
    {
        return Err("Explicit local relink/one Undo changed the retained cache".into());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn blocked_paths_embedded_svg_and_missing_cache_local_relink() {
        let root = std::env::temp_dir().join(format!("drawer-local-refs-{}", std::process::id()));
        std::fs::create_dir(&root).unwrap();
        let result = super::run(&root);
        std::fs::remove_dir(&root).unwrap();
        result.unwrap();
    }
}
