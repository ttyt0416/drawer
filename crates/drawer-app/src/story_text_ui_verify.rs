use super::*;
use crate::text_editor::verify::{click, settle};
pub(super) fn smoke(root: &std::path::Path) -> Result<(), String> {
    drawer_core::text_extensions_verify::story(root)?;
    let ctx = egui::Context::default();
    let mut app = Box::new(Drawer::new(&ctx, None));
    app.simple_view = true;
    app.storage = Some(UserStorage::at(root.join("story-ui-user")).map_err(|e| e.to_string())?);
    let mut first = drawer_core::text_extensions_verify::page("ABC 가나다")?;
    let first_path = root.join("story-ui-first.drawer");
    drawer_core::files::save(&mut first, &first_path).map_err(|e| e.to_string())?;
    let mut second = drawer_core::text_extensions_verify::page("abc 둘째")?;
    let second_path = root.join("story-ui-second.drawer");
    drawer_core::files::save(&mut second, &second_path).map_err(|e| e.to_string())?;
    let before_second = second.snapshot();
    let bytes = std::fs::read(&second_path).map_err(|e| e.to_string())?;
    let paths = vec![first.path.clone().unwrap(), second.path.clone().unwrap()];
    app.add_document(first);
    let first_id = app.documents[0].id;
    let original = app.documents[0].snapshot();
    let mut time = 0.;
    app.begin_story_text()?;
    {
        let p = &mut app.text_editor.story;
        p.title = "작품".into();
        p.work = Some(story::Work::new(p.title.clone(), paths.clone())?);
        p.whole = true;
        p.find = "abc".into();
        p.replace = "수정".into();
        p.sensitive = false;
    }
    settle(&ctx, &mut app, &mut time);
    let r = app
        .text_editor
        .story
        .prepare
        .ok_or("Story prepare button")?;
    click(&ctx, &mut app, &mut time, r);
    settle(&ctx, &mut app, &mut time);
    if app
        .text_editor
        .story
        .prepared
        .as_ref()
        .is_none_or(|p| p.closed.len() != 1 || p.plan.matches.len() != 2)
        || app.documents.len() != 1
        || app.documents[0].snapshot() != original
    {
        return Err(format!(
            "Closed-page work preview mutated/opened source or missed pages: {:?}",
            app.text_editor.story.error
        ));
    }
    let r = app.text_editor.story.cancel.unwrap();
    click(&ctx, &mut app, &mut time, r);
    settle(&ctx, &mut app, &mut time);
    if app.text_editor.story.prepared.is_some() || app.documents.len() != 1 {
        return Err("Cancelled story preview retained/opened pages".into());
    }
    app.begin_story_text()?;
    let mut p = std::mem::take(&mut *app.text_editor.story);
    app.prepare_story_text(&mut p)?;
    p.prepared.as_mut().unwrap().ack = true;
    *app.text_editor.story = p;
    settle(&ctx, &mut app, &mut time);
    let r = app.text_editor.story.apply.ok_or("Story Apply")?;
    click(&ctx, &mut app, &mut time, r);
    settle(&ctx, &mut app, &mut time);
    if app.documents.len() != 2
        || app.documents.iter().any(|d| {
            !core::Settings::from_object(&d.scene.objects[0])
                .unwrap()
                .text
                .contains("수정")
        })
        || std::fs::read(&second_path).map_err(|e| e.to_string())? != bytes
    {
        return Err(format!(
            "Whole-work Apply or source-file preservation failed: {:?}",
            app.text_editor.story.error
        ));
    }
    app.execute_command(&ctx, "undo");
    if app
        .documents
        .iter()
        .find(|d| d.id == first_id)
        .unwrap()
        .snapshot()
        != original
        || app
            .documents
            .iter()
            .find(|d| d.id != first_id)
            .unwrap()
            .snapshot()
            != before_second
    {
        return Err("One command did not undo both work pages".into());
    }
    app.execute_command(&ctx, "redo");
    if app.documents.iter().any(|d| {
        !core::Settings::from_object(&d.scene.objects[0])
            .unwrap()
            .text
            .contains("수정")
    }) {
        return Err("One command did not redo both work pages".into());
    }
    // A closed file changed after Prepare must block every page before any document is touched.
    let mut other = Box::new(Drawer::new(&ctx, None));
    other.add_document(
        drawer_core::files::open(&first_path, DEFAULT_DOCUMENT_BUDGET)
            .map_err(|e| e.to_string())?,
    );
    let old = other.documents[0].snapshot();
    let mut p = Panel {
        whole: true,
        work: Some(story::Work::new("作品".into(), paths)?),
        find: "abc".into(),
        replace: "X".into(),
        sensitive: false,
        ..Default::default()
    };
    other.prepare_story_text(&mut p)?;
    p.prepared.as_mut().unwrap().ack = true;
    let mut changed = bytes.clone();
    changed.push(1);
    std::fs::write(&second_path, changed).map_err(|e| e.to_string())?;
    if other.apply_story_text(&mut p).is_ok()
        || other.documents.len() != 1
        || other.documents[0].snapshot() != old
    {
        return Err("Changed closed page partly applied work".into());
    }
    std::fs::write(&second_path, bytes).map_err(|e| e.to_string())?;
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn story_current_and_closed_work_pages_mouse_preview_cancel_linked_undo_and_stale_file() {
        let root = crate::tests::TestDirectory::new();
        super::smoke(&root.0).unwrap();
    }
}
