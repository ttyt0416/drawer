use super::*;
pub fn run(root: &Path) -> Result<(), String> {
    let storage = root.join("recovery-generations");
    fs::create_dir(&storage).map_err(|e| e.to_string())?;
    let spec = crate::document::CanvasSpec {
        width: 4,
        height: 4,
        dpi: 300.,
        background: crate::document::Background::Transparent,
    };
    let mut a = Document::create(spec, B).map_err(|e| e.to_string())?;
    let mut b = Document::create(spec, B).map_err(|e| e.to_string())?;
    let first = storage.join("source-a.drawer");
    let second = storage.join("source-b.drawer");
    crate::files::save(&mut a, &first).map_err(|e| e.to_string())?;
    crate::files::save(&mut b, &second).map_err(|e| e.to_string())?;
    let original = (
        fs::read(&first).map_err(|e| e.to_string())?,
        fs::read(&second).map_err(|e| e.to_string())?,
    );
    let mut model = crate::page_work::Model::new(crate::story_text::Work::new(
        "Recovery project".into(),
        vec![first.clone(), second.clone()],
    )?)?;
    model.edit(crate::page_work::Edit::Move { id: 1, index: 1 })?;
    model.edit(crate::page_work::Edit::Metadata {
        title: "Reordered recovery".into(),
        right_binding: true,
        first_right: false,
    })?;
    a.edit_pixel(0, 0, 0, [9, 8, 7, 255]);
    let state = a.snapshot();
    let mut session = Session::create(&storage)?;
    session.set_retention(2)?;
    for _ in 0..5 {
        session.checkpoint_project(&[&a], &model, &[(1, a.id)])?;
    }
    let dirs = fs::read_dir(&session.directory)
        .map_err(|e| e.to_string())?
        .filter_map(Result::ok)
        .filter(|e| e.file_name().to_string_lossy().starts_with("generation-"))
        .count();
    if dirs != 2 {
        return Err(format!(
            "Recovery retention expected2 generations, found{dirs}"
        ));
    }
    let directory = session.directory.clone();
    drop(session);
    let moved = root.join("moved-recovery-generations");
    fs::rename(&storage, &moved).map_err(|e| e.to_string())?;
    let portable = scan(&moved)?
        .into_iter()
        .find(|c| c.project)
        .ok_or("Moved project recovery missing")?
        .open_project()?;
    if !portable
        .pages
        .iter()
        .all(|p| p.starts_with(moved.canonicalize().unwrap()))
    {
        return Err("Moved project still references the previous scratch root".into());
    }
    fs::rename(&moved, &storage).map_err(|e| e.to_string())?;
    let candidates = scan(&storage)?;
    let project = candidates
        .iter()
        .find(|c| c.project)
        .ok_or("Project recovery missing")?;
    let work = project.open_project()?;
    if work.title != "Reordered recovery"
        || !work.right_binding
        || work.first_right
        || work.page_ids != vec![2, 1]
        || crate::files::open(&work.pages[1], B)
            .map_err(|e| e.to_string())?
            .snapshot()
            != state
    {
        return Err("Project metadata/order/open-page revision not recovered together".into());
    }
    if fs::read(&first).map_err(|e| e.to_string())? != original.0
        || fs::read(&second).map_err(|e| e.to_string())? != original.1
    {
        return Err("Project recovery changed source pages".into());
    }
    discard(&directory)?;
    // Unknown content in an older owned generation must never be swept automatically.
    let mut s = Session::create(&storage)?;
    s.set_retention(2)?;
    s.checkpoint(&[&a])?;
    let foreign = s.directory.join("generation-1/foreign.guard");
    fs::write(&foreign, b"keep").map_err(|e| e.to_string())?;
    s.checkpoint(&[&a])?;
    s.checkpoint(&[&a])?;
    if s.take_maintenance_warning().is_none()
        || fs::read(&foreign).map_err(|e| e.to_string())? != b"keep"
    {
        return Err("Unrecognized generation content was deleted".into());
    }
    s.finish()?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use std::fs;
    #[test]
    fn project_recovery_and_verified_generation_retention() {
        let root = std::env::temp_dir().join(format!(
            "drawer-recovery-generations-{}",
            std::process::id()
        ));
        fs::create_dir(&root).unwrap();
        let r = super::run(&root);
        fs::remove_dir_all(root).unwrap();
        r.unwrap();
    }
}
