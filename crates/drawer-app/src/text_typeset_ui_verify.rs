use super::*;
use crate::text_editor::verify::{frame, settle};
pub(super) fn smoke(root: &std::path::Path) -> Result<(), String> {
    non_text_target(root)?;
    drawer_core::text_typeset_verify::verify(root)?;
    crate::drawing::text_tool::smoke()?;
    let ctx = egui::Context::default();
    let mut app = Box::new(Drawer::new(&ctx, None));
    app.simple_view = true;
    app.storage = Some(UserStorage::at(root.join("typeset-ui-user")).map_err(|e| e.to_string())?);
    app.documents
        .push(drawer_core::text_version_verify::fixture()?);
    app.active = Some(0);
    let doc = app.documents[0].id;
    let id = app.documents[0].scene.catalog.objects.id(1);
    app.panels.targets.insert(
        doc,
        drawer_core::thumbnail::Target::Source(drawer_core::layer_search::Key::Object(id)),
    );
    let original = app.documents[0].snapshot();
    let history = app.documents[0].history_counts();
    let mut time = 0.;
    for target in [
        ty::ApplyTarget::None,
        ty::ApplyTarget::NewTextOnly,
        ty::ApplyTarget::SelectedText,
    ] {
        let mut tool = ty::Tool {
            target,
            ..Default::default()
        };
        tool.defaults.size = 35.;
        tool.defaults.color = [20, 40, 70, 100];
        app.save_text_tool(&ctx, Box::new(tool))?;
        if app.documents[0].snapshot() != original || app.documents[0].history_counts() != history {
            return Err("Saving text defaults changed drawing".into());
        }
        if target == ty::ApplyTarget::SelectedText {
            if app
                .text_editor
                .draft
                .as_ref()
                .is_none_or(|e| e.id != Some(id) || e.settings.size != 35.)
            {
                return Err("SelectedText target did not open a formatted draft".into());
            }
            app.text_editor.draft = None;
        }
    }
    let tool = ty::Tool {
        create: ty::CreateAction::CreateLayerAlways,
        target: ty::ApplyTarget::None,
        ..Default::default()
    };
    *app.workspaces.store.current.text_tool = tool;
    app.begin_text_editor(&ctx)?;
    if app.text_editor.draft.as_ref().unwrap().id.is_some() {
        return Err("Create layer always reused selection".into());
    }
    app.text_editor.draft = None;
    app.workspaces.store.current.text_tool.create = ty::CreateAction::AddToSelectedText;
    app.begin_text_editor(&ctx)?;
    if app.text_editor.draft.as_ref().unwrap().id != Some(id) {
        return Err("Add to selected did not retain selected text".into());
    }
    app.text_editor.draft = None;
    app.workspaces.store.current.text_tool.create = ty::CreateAction::CreateLayerAlways;
    app.workspaces.store.current.text_tool.drag = ty::DragAction::CreateTextBox;
    app.drawing.text_tool_request = Some(crate::drawing::text_tool::Request {
        document: doc,
        revision: app.documents[0].revision,
        frame: 0,
        start: [20., 40.],
        end: [90., 80.],
    });
    app.poll_text_tool(&ctx);
    let e = app
        .text_editor
        .draft
        .as_mut()
        .ok_or("Drag text box draft")?;
    if e.id.is_some() || e.settings.baseline != [20., 64.] || e.settings.style.wrap_width != 70. {
        return Err("Canvas drag text box geometry".into());
    }
    e.settings.text = "12한글".into();
    e.settings.style = drawer_core::text_typeset_verify::style();
    e.settings.style.flow = core::Flow::Vertical;
    e.settings.style.typeset.as_mut().unwrap().horizontal = vec![[0, 2]];
    let pending = e.settings.clone();
    settle(&ctx, &mut app, &mut time);
    let widget = composition::widget_id(app.text_editor.draft.as_ref().unwrap());
    let mut state = egui::text_edit::TextEditState::load(&ctx, widget).unwrap_or_default();
    state
        .cursor
        .set_char_range(Some(egui::text::CCursorRange::one(
            egui::text::CCursor::new(0),
        )));
    state.store(&ctx, widget);
    ctx.memory_mut(|m| m.request_focus(widget));
    frame(
        &ctx,
        &mut app,
        &mut time,
        vec![egui::Event::Ime(egui::ImeEvent::Preedit {
            text: "한".into(),
            active_range_chars: None,
        })],
    );
    if app
        .text_editor
        .draft
        .as_ref()
        .is_none_or(|e| e.composition.is_none())
    {
        return Err("Advanced text IME did not start".into());
    }
    let unchanged = app.workspaces.store.current.text_tool.clone();
    if app.save_text_tool(&ctx, Box::default()).is_ok()
        || app.workspaces.store.current.text_tool != unchanged
    {
        return Err("IME allowed default format change".into());
    }
    frame(
        &ctx,
        &mut app,
        &mut time,
        vec![egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Default::default(),
        }],
    );
    if app
        .text_editor
        .draft
        .as_ref()
        .is_none_or(|e| e.composition.is_some() || e.settings != pending)
    {
        return Err("IME cancel lost typography/TCY snapshot".into());
    }
    let output = drawer_core::render::Renderer::new(&app.documents[0])
        .output(300., DEFAULT_DOCUMENT_BUDGET)?
        .2;
    let original_filter = app.canvas_view.filter;
    let mut prepared_pixels = None;
    for quality in [ty::Quality::Fast, ty::Quality::High, ty::Quality::Auto] {
        app.workspaces.store.current.text_tool.quality = quality;
        app.preview_text(&ctx)?;
        let prepared = &app
            .text_editor
            .draft
            .as_ref()
            .unwrap()
            .prepared
            .as_ref()
            .unwrap()
            .1
            .state;
        let preview = drawer_core::layer_merge::preview(prepared, 0)?;
        let pixels = drawer_core::render::Renderer::new(&preview)
            .output(300., DEFAULT_DOCUMENT_BUDGET)?
            .2;
        if prepared_pixels.as_ref().is_some_and(|old| *old != pixels) {
            return Err("Quality changed prepared final text output".into());
        }
        prepared_pixels = Some(pixels);
        settle(&ctx, &mut app, &mut time);
        if app.documents[0].snapshot() != original
            || drawer_core::render::Renderer::new(&app.documents[0])
                .output(300., DEFAULT_DOCUMENT_BUDGET)?
                .2
                != output
            || app.canvas_view.filter != original_filter
        {
            return Err("Preview quality changed final drawing or saved view filter".into());
        }
    }
    app.apply_text()?;
    if app.documents[0].history_counts().0 != history.0 + 1
        || !app.documents[0].undo()
        || app.documents[0].snapshot() != original
    {
        return Err("Typography Apply one Undo".into());
    }
    app.workspaces.store.current.text_tool.drag = ty::DragAction::SelectText;
    app.drawing.text_tool_request = Some(crate::drawing::text_tool::Request {
        document: doc,
        revision: app.documents[0].revision,
        frame: 0,
        start: [10., 70.],
        end: [180., 130.],
    });
    app.poll_text_tool(&ctx);
    if app
        .text_editor
        .draft
        .as_ref()
        .is_none_or(|e| e.id != Some(id))
    {
        return Err("Drag SelectText did not select the intersected text".into());
    }
    app.text_editor.draft = None;
    app.drawing.text_tool_request = Some(crate::drawing::text_tool::Request {
        document: doc,
        revision: app.documents[0].revision + 1,
        frame: 0,
        start: [1.; 2],
        end: [40.; 2],
    });
    app.poll_text_tool(&ctx);
    if app.text_editor.draft.is_some() || app.documents[0].snapshot() != original {
        return Err("Stale text capture changed document".into());
    }
    Ok(())
}
fn non_text_target(root: &std::path::Path) -> Result<(), String> {
    let ctx = egui::Context::default();
    let mut app = Box::new(Drawer::new(&ctx, None));
    app.storage =
        Some(UserStorage::at(root.join("non-text-target-user")).map_err(|e| e.to_string())?);
    let mut doc = drawer_core::text_layout_verify::fixture()?;
    let state =
        drawer_core::gradient::prepare(&doc, 0, Default::default(), true, Default::default())?;
    doc.commit_state(state)?;
    let id = doc.scene.gradients.layers[0].id;
    app.documents.push(doc);
    app.active = Some(0);
    app.panels.targets.insert(
        app.documents[0].id,
        drawer_core::thumbnail::Target::Source(drawer_core::layer_search::Key::Gradient(id)),
    );
    let old = app.workspaces.store.current.text_tool.clone();
    let source = app.documents[0].snapshot();
    let tool = ty::Tool {
        target: ty::ApplyTarget::SelectedText,
        ..Default::default()
    };
    if app.save_text_tool(&ctx, Box::new(tool)).is_ok()
        || app.workspaces.store.current.text_tool != old
        || app.documents[0].snapshot() != source
        || app.text_editor.draft.is_some()
    {
        return Err("Non-text target changed settings or opened text editor".into());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn non_text_source_cannot_be_used_as_selected_text() {
        let root = crate::tests::TestDirectory::new();
        super::non_text_target(&root.0).unwrap();
    }
    #[test]
    fn text_tool_creation_targets_ime_quality_native_and_undo() {
        let root = crate::tests::TestDirectory::new();
        super::smoke(&root.0).unwrap();
    }
}
