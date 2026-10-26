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

fn replace_field(ctx: &egui::Context, app: &mut Drawer, time: &mut f64, r: egui::Rect, text: &str) {
    click(ctx, app, time, r);
    let modifiers = egui::Modifiers {
        ctrl: true,
        command: true,
        ..Default::default()
    };
    crate::text_editor::verify::frame(
        ctx,
        app,
        time,
        vec![
            egui::Event::Key {
                key: egui::Key::A,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers,
            },
            egui::Event::Key {
                key: egui::Key::A,
                physical_key: None,
                pressed: false,
                repeat: false,
                modifiers,
            },
            egui::Event::Text(text.into()),
        ],
    );
    settle(ctx, app, time);
}
pub(super) fn dialogue(root: &std::path::Path) -> Result<(), String> {
    std::fs::create_dir_all(root).map_err(|e| e.to_string())?;
    let dir = root.join("ordered-dialogue-ui");
    std::fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let result = (|| {
        let ctx = egui::Context::default();
        let mut app = Box::new(Drawer::new(&ctx, None));
        app.simple_view = true;
        app.storage = Some(UserStorage::at(dir.join("user")).map_err(|e| e.to_string())?);
        let mut a = drawer_core::text_extensions_verify::page("First\noriginal")?;
        let pa = dir.join("first.drawer");
        drawer_core::files::save(&mut a, &pa).map_err(|e| e.to_string())?;
        let mut b = drawer_core::text_extensions_verify::page("Second original")?;
        let pb = dir.join("second.drawer");
        drawer_core::files::save(&mut b, &pb).map_err(|e| e.to_string())?;
        let disk = std::fs::read(&pb).map_err(|e| e.to_string())?;
        let before_a = a.snapshot();
        let before_b = b.snapshot();
        app.add_document(a);
        app.begin_story_text()?;
        app.text_editor.story.editing = true;
        app.text_editor.story.whole = true;
        app.text_editor.story.work = Some(story::Work::new(
            "dialogues".into(),
            vec![
                pa.canonicalize().map_err(|e| e.to_string())?,
                pb.canonicalize().map_err(|e| e.to_string())?,
            ],
        )?);
        let mut time = 0.;
        settle(&ctx, &mut app, &mut time);
        let r = app
            .text_editor
            .story
            .prepare
            .ok_or("Load dialogue button missing")?;
        click(&ctx, &mut app, &mut time, r);
        settle(&ctx, &mut app, &mut time);
        if app.text_editor.story.rows.len() != 2 || app.documents.len() != 1 {
            return Err(format!(
                "Dialogue list not loaded lazily: {:?}",
                app.text_editor.story.error
            ));
        }
        let fields = app.text_editor.story.row_fields.clone();
        if fields.len() != 2 {
            return Err("Ordered dialogue fields missing".into());
        }
        replace_field(&ctx, &mut app, &mut time, fields[0], "First edited\nline");
        let r = *app
            .text_editor
            .story
            .row_fields
            .get(1)
            .ok_or("Closed-page dialogue field missing")?;
        replace_field(&ctx, &mut app, &mut time, r, "Second edited");
        if app.text_editor.story.rows[0].after.text != "First edited\nline"
            || app.text_editor.story.rows[1].after.text != "Second edited"
            || app.documents[0].snapshot() != before_a
        {
            return Err("Dialogue field typing mutated source or missed the row".into());
        }
        app.workspaces.store.current.comic_view.show_breaks = true;
        app.workspaces.store.current.comic_view.vertical_list = true;
        settle(&ctx, &mut app, &mut time);
        if app.text_editor.story.rows[0].after.text != "First edited\nline" {
            return Err("Newline/list display changed dialogue text".into());
        }
        let r = app.text_editor.story.prepare.unwrap();
        click(&ctx, &mut app, &mut time, r);
        settle(&ctx, &mut app, &mut time);
        let p = app.text_editor.story.prepared.as_mut().ok_or_else(|| {
            format!(
                "Dialogue preview missing: {:?}; open={} rows={} button={:?} viewport={:?}",
                app.text_editor.story.error,
                app.text_editor.story.open,
                app.text_editor.story.rows.len(),
                r,
                ctx.content_rect()
            )
        })?;
        p.ack = true;
        let mut panel = std::mem::take(&mut *app.text_editor.story);
        let body = panel.rows[0].after.text.clone();
        panel.rows[0].after.text.push_str(" stale");
        if app.apply_story_text(&mut panel).is_ok() || app.documents[0].snapshot() != before_a {
            return Err("A changed dialogue draft was applied without a fresh preview".into());
        }
        panel.rows[0].after.text = body;
        *app.text_editor.story = panel;
        settle(&ctx, &mut app, &mut time);
        let r = app.text_editor.story.apply.unwrap();
        click(&ctx, &mut app, &mut time, r);
        settle(&ctx, &mut app, &mut time);
        if app.documents.len() != 2
            || !app.documents.iter().all(|d| {
                core::Settings::from_object(&d.scene.objects[0])
                    .is_some_and(|s| s.text.contains("edited"))
            })
        {
            return Err(format!(
                "Direct dialogue Apply failed: {:?}; documents={} preview={} button={:?}",
                app.text_editor.story.error,
                app.documents.len(),
                app.text_editor.story.prepared.is_some(),
                r
            ));
        }
        if std::fs::read(&pb).map_err(|e| e.to_string())? != disk {
            return Err("Dialogue Apply overwrote the closed-page source file".into());
        }
        app.execute_command(&ctx, "undo");
        if app.documents[0].snapshot() != before_a || app.documents[1].snapshot() != before_b {
            return Err("Direct dialogue linked Undo failed".into());
        }
        app.execute_command(&ctx, "redo");
        app.begin_story_text()?;
        app.text_editor.story.rows.clear();
        settle(&ctx, &mut app, &mut time);
        let n = app.active.unwrap();
        let current = app.documents[n].snapshot();
        let r = app
            .text_editor
            .story
            .insert_field
            .ok_or("Insertion field missing")?;
        replace_field(&ctx, &mut app, &mut time, r, "\nextra");
        app.workspaces.store.current.comic_view.append = true;
        app.workspaces.store.current.comic_view.inherit = true;
        app.panels.targets.remove(&app.documents[n].id);
        let r = app
            .text_editor
            .story
            .insert_button
            .ok_or("Insertion button missing")?;
        click(&ctx, &mut app, &mut time, r);
        settle(&ctx, &mut app, &mut time);
        if app.documents[n].snapshot() != current || app.text_editor.story.error.is_none() {
            return Err("Missing inheritance source was not rejected atomically".into());
        }
        let id = app.documents[n].scene.catalog.objects.id(0);
        app.panels.targets.insert(
            app.documents[n].id,
            drawer_core::thumbnail::Target::Source(drawer_core::layer_search::Key::Object(id)),
        );
        let r = app.text_editor.story.insert_button.unwrap();
        click(&ctx, &mut app, &mut time, r);
        settle(&ctx, &mut app, &mut time);
        if !core::Settings::from_object(&app.documents[n].scene.objects[0])
            .unwrap()
            .text
            .ends_with("\nextra")
        {
            return Err(format!(
                "Inherited dialogue append failed: {:?}",
                app.text_editor.story.error
            ));
        }
        app.execute_command(&ctx, "undo");
        if app.documents[n].snapshot() != current {
            return Err("Inserted dialogue was not one Undo".into());
        }
        app.workspaces.store.current.comic_view.append = false;
        app.workspaces.store.current.comic_view.inherit = false;
        settle(&ctx, &mut app, &mut time);
        let field = app.text_editor.story.insert_field.unwrap();
        replace_field(&ctx, &mut app, &mut time, field, "New dialogue");
        let button = app.text_editor.story.insert_button.unwrap();
        click(&ctx, &mut app, &mut time, button);
        settle(&ctx, &mut app, &mut time);
        if app.documents[n].scene.objects.len() != current.scene.objects.len() + 1
            || !app.documents[n].scene.objects.last().is_some_and(|o| {
                core::Settings::from_object(o)
                    .is_some_and(|s| s.text == "New dialogue" && s.baseline[1] > 0.)
            })
        {
            return Err("New dialogue object was not created at a visible baseline".into());
        }
        app.execute_command(&ctx, "undo");
        if app.documents[n].snapshot() != current {
            return Err("New dialogue object Undo failed".into());
        }
        app.workspaces.store.current.comic_view.append = true;
        app.workspaces.store.current.comic_view.inherit = true;
        app.persist_workspace()?;
        let prefs = drawer_core::workspaces::Store::load(app.storage.as_ref().unwrap())
            .map_err(|e| e.to_string())?
            .ok_or("Dialogue preferences missing")?;
        if !prefs.current.comic_view.show_breaks
            || !prefs.current.comic_view.vertical_list
            || !prefs.current.comic_view.append
            || !prefs.current.comic_view.inherit
        {
            return Err("Dialogue display/insertion defaults did not persist".into());
        }
        Ok(())
    })();
    let cleanup = std::fs::remove_dir_all(dir).map_err(|e| e.to_string());
    result.and(cleanup)
}
#[cfg(test)]
mod dialogue_tests {
    #[test]
    fn ordered_dialogue_pointer_typing_closed_page_apply_append_display_and_undo() {
        let root = std::env::temp_dir().join(format!("drawer-dialogue-ui-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        super::dialogue(&root).unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }
}
#[cfg(test)]
mod tests {
    #[test]
    fn story_current_and_closed_work_pages_mouse_preview_cancel_linked_undo_and_stale_file() {
        let root = crate::tests::TestDirectory::new();
        super::smoke(&root.0).unwrap();
    }
}
