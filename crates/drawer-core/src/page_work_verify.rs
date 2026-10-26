use crate::{
    document::{Background, CanvasSpec, DEFAULT_DOCUMENT_BUDGET, Document},
    page_work::{self as p, Edit, Model, Navigation},
    story_text::Work,
};
use std::path::{Path, PathBuf};
fn check(v: bool, s: &str) -> Result<(), String> {
    if v { Ok(()) } else { Err(s.into()) }
}
fn doc() -> Document {
    Document::create(
        CanvasSpec {
            width: 2,
            height: 2,
            dpi: 300.,
            background: Background::Transparent,
        },
        DEFAULT_DOCUMENT_BUDGET,
    )
    .unwrap()
}
pub fn lazy() -> Result<(), String> {
    let paths: Vec<_> = (0..512)
        .map(|i| std::env::temp_dir().join(format!("drawer-lazy-page-{i}.drawer")))
        .collect();
    let mut m = Model::new(Work::new("hundreds".into(), paths)?)?;
    check(m.loads == 0, "Project creation eagerly loaded pages")?;
    let original = m.work.clone();
    let last = m.navigate(Navigation::Last)?;
    check(m.current() == 511 && last == 512, "Last navigation")?;
    m.navigate(Navigation::Next)?;
    check(m.selected == last, "Next beyond last")?;
    m.navigate(Navigation::First)?;
    m.navigate(Navigation::Previous)?;
    check(m.current() == 0, "Previous before first")?;
    for n in [0, 513, usize::MAX] {
        let selected = m.selected;
        check(
            m.navigate(Navigation::Number(n)).is_err() && m.selected == selected,
            "Invalid navigation changed selection",
        )?;
    }
    check(
        m.work == original && m.loads == 0,
        "Navigation changed data or loaded unrelated pages",
    )?;
    m.navigate(Navigation::Number(241))?;
    let id = m.selected;
    m.edit(Edit::Delete(id))?;
    check(
        !m.work.page_ids.contains(&id) && m.work.pages.len() == 511,
        "Delete lost wrong page",
    )?;
    m.undo();
    check(
        m.work.page_ids == original.page_ids && m.work.pages == original.pages,
        "Delete Undo did not restore stable IDs/order",
    )?;
    m.redo();
    m.undo();
    m.edit(Edit::Move { id, index: 0 })?;
    check(m.work.page_ids[0] == id, "Reorder lost page identity")?;
    m.undo();
    for right in [false, true] {
        for first in [false, true] {
            m.edit(Edit::Metadata {
                title: "hundreds".into(),
                right_binding: right,
                first_right: first,
            })?;
            for n in [0, 1, 240, 510, 511] {
                if let Some(other) = m.facing(n) {
                    check(
                        m.facing(other) == Some(n),
                        "Facing page pair is not reciprocal",
                    )?;
                }
            }
        }
    }
    check(m.loads == 0, "Facing metadata loaded all page data")
}
fn paths(root: &Path) -> Result<Vec<PathBuf>, String> {
    let mut out = vec![];
    for n in 0..3 {
        let mut d = doc();
        d.layers[0].pixels.fill([n as u8 * 30, 70, 90, 255]);
        let path = root.join(format!("page-{n}.drawer"));
        crate::files::save(&mut d, &path).map_err(|e| e.to_string())?;
        out.push(path.canonicalize().map_err(|e| e.to_string())?);
    }
    Ok(out)
}
pub fn files(root: &Path) -> Result<(), String> {
    let dir = root.join("page-work-fixture");
    std::fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let result = (|| {
        let paths = paths(&dir)?;
        let mut m = Model::new(Work::new("Pages".into(), paths[..2].to_vec())?)?;
        let d = m.open_page(1, DEFAULT_DOCUMENT_BUDGET)?;
        check(
            m.loads == 1 && d.layers[0].pixels[0] == [0, 70, 90, 255],
            "Lazy selected-page load",
        )?;
        m.edit(Edit::Add {
            path: paths[2].clone(),
            index: 1,
        })?;
        let added = m.work.page_ids[1];
        m.undo();
        m.edit(Edit::Add {
            path: paths[2].clone(),
            index: 1,
        })?;
        check(
            m.work.page_ids[1] > added,
            "Discarded page ID reused by a new branch",
        )?;
        let blank = dir.join("blank.drawer");
        let mut empty = doc();
        crate::files::save(&mut empty, &blank).map_err(|e| e.to_string())?;
        let one = Model::new(Work::new("one blank page".into(), vec![blank.clone()])?)?;
        check(one.work.pages.len() == 1, "Single empty page rejected")?;
        let mut one = one;
        check(
            one.edit(Edit::Delete(one.selected)).is_err(),
            "Final page silently deleted",
        )?;
        let original = std::fs::read(&paths[0]).map_err(|e| e.to_string())?;
        let copy = dir.join("duplicate.drawer");
        p::duplicate(&d, &copy)?;
        let mut duplicate =
            crate::files::open(&copy, DEFAULT_DOCUMENT_BUDGET).map_err(|e| e.to_string())?;
        duplicate.edit_pixel(0, 0, 0, [255, 0, 0, 255]);
        check(
            d.layers[0].pixels[0] == [0, 70, 90, 255],
            "Duplicate edit changed source",
        )?;
        check(
            p::duplicate(&d, &copy).is_err(),
            "Duplicate overwrote an existing file",
        )?;
        check(
            std::fs::read(&paths[0]).map_err(|e| e.to_string())? == original,
            "Page operation changed source bytes",
        )?;
        let manifest = dir.join("project.drawer-work");
        m.work.save(&manifest)?;
        let loaded = Model::load(&manifest)?;
        check(
            loaded.work == m.work && loaded.loads == 0,
            "Project IDs/order roundtrip or lazy load",
        )?;
        let legacy = dir.join("legacy.drawer-work");
        let json = serde_json::json!({"version":1,"title":"Legacy","pages":[paths[0]]});
        std::fs::write(&legacy, serde_json::to_vec(&json).unwrap()).map_err(|e| e.to_string())?;
        let old = Model::load(&legacy)?;
        check(
            old.work.page_ids == vec![1] && old.work.next_page_id == 2,
            "Legacy work not upgraded independently",
        )?;
        let storage = crate::storage::UserStorage::at(dir.join("settings-root"))
            .map_err(|e| e.to_string())?;
        let mut store = crate::workspaces::Store::default();
        store.current.page_view.dock = crate::workspaces::Dock::Right;
        store.current.page_view.auto_scroll = false;
        store.current.page_view.new_view = true;
        store.save(&storage).map_err(|e| e.to_string())?;
        check(
            crate::workspaces::Store::load(&storage)
                .map_err(|e| e.to_string())?
                .ok_or("Settings missing")?
                .current
                .page_view
                == store.current.page_view,
            "Page view preferences not restored",
        )?;
        snapshot(&dir, &m, &manifest)?;
        Ok(())
    })();
    let cleanup = std::fs::remove_dir_all(&dir).map_err(|e| e.to_string());
    result.and(cleanup)
}
fn snapshot(dir: &Path, m: &Model, manifest: &Path) -> Result<(), String> {
    let before = std::fs::read(manifest).map_err(|e| e.to_string())?;
    let originals: Vec<_> = m
        .work
        .pages
        .iter()
        .map(std::fs::read)
        .collect::<Result<_, _>>()
        .map_err(|e| e.to_string())?;
    let mut current =
        crate::files::open(&m.work.pages[0], DEFAULT_DOCUMENT_BUDGET).map_err(|e| e.to_string())?;
    current.edit_pixel(0, 0, 0, [111, 33, 55, 255]);
    let state = current.snapshot();
    let old_path = current.path.clone();
    let old_counts = current.history_counts();
    let selected = m.work.page_ids[0];
    let result = p::snapshot(m, manifest, &[(selected, &current)], &mut |done, _| {
        if done == 1 {
            Err("cancel".into())
        } else {
            Ok(())
        }
    });
    check(
        result.is_err() && std::fs::read(manifest).map_err(|e| e.to_string())? == before,
        "Cancelled snapshot replaced previous manifest",
    )?;
    check(
        !std::fs::read_dir(dir).unwrap().any(|e| {
            e.unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".drawer-project")
        }),
        "Cancelled snapshot left staging",
    )?;
    let work = p::snapshot(m, manifest, &[(selected, &current)], &mut |_, _| Ok(()))?;
    check(
        work.page_ids == m.work.page_ids
            && work
                .pages
                .iter()
                .all(|p| p.parent() == work.pages[0].parent())
            && work.pages != m.work.pages,
        "Snapshot not a coherent independent page set",
    )?;
    let saved =
        crate::files::open(&work.pages[0], DEFAULT_DOCUMENT_BUDGET).map_err(|e| e.to_string())?;
    check(
        saved.layers[0].pixels[0] == [111, 33, 55, 255],
        "Snapshot omitted unsaved open page data",
    )?;
    check(
        current.snapshot() == state
            && current.path == old_path
            && current.history_counts() == old_counts
            && current.dirty,
        "Snapshot modified open document/history/save marker",
    )?;
    for (p, b) in m.work.pages.iter().zip(&originals) {
        check(
            std::fs::read(p).map_err(|e| e.to_string())? == *b,
            "Snapshot overwrote a source page",
        )?;
    }
    let published = std::fs::read(manifest).map_err(|e| e.to_string())?;
    let source = m.work.pages[1].clone();
    let bytes = std::fs::read(&source).map_err(|e| e.to_string())?;
    let result = p::snapshot(m, manifest, &[(selected, &current)], &mut |done, _| {
        if done == 0 {
            std::fs::write(&source, b"damaged mid-save").map_err(|e| e.to_string())?;
        }
        Ok(())
    });
    std::fs::write(&source, &bytes).map_err(|e| e.to_string())?;
    check(
        result.is_err() && std::fs::read(manifest).map_err(|e| e.to_string())? == published,
        "Changed/invalid closed page partially published",
    )?;
    let blocker = manifest.with_extension("drawer-work-write.tmp");
    std::fs::write(&blocker, b"foreign").map_err(|e| e.to_string())?;
    let dirs = std::fs::read_dir(dir).unwrap().count();
    check(
        p::snapshot(m, manifest, &[(selected, &current)], &mut |_, _| Ok(())).is_err(),
        "Foreign manifest temporary file overwritten",
    )?;
    check(
        std::fs::read(&blocker).unwrap() == b"foreign"
            && std::fs::read_dir(dir).unwrap().count() == dirs,
        "Failed publication left resources or deleted foreign temporary file",
    )?;
    std::fs::remove_file(blocker).map_err(|e| e.to_string())?;
    Ok(())
}
pub fn verify(root: &Path) -> Result<(), String> {
    lazy()?;
    files(root)
}
#[cfg(test)]
mod tests {
    #[test]
    fn five_hundred_pages_stable_ids_navigation_pairing_reorder_delete_and_lazy_load() {
        super::lazy().unwrap();
    }
    #[test]
    fn project_snapshot_unsaved_data_atomic_failure_sources_id_and_preferences() {
        let root = std::env::temp_dir().join(format!("drawer-page-work-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        super::files(&root).unwrap();
        std::fs::remove_dir(root).unwrap();
    }
}
