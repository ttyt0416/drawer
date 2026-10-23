use super::*;
use crate::drawing::Drawing;
pub(crate) struct CanvasPreview {
    document: u64,
    revision: u64,
    frame: u32,
    page: Document,
}
impl Drawing {
    pub(crate) fn refresh_text_canvas(&mut self, d: &Document) {
        if let Some(p) = &mut self.text_canvas {
            if p.document == d.id && p.revision == d.revision && p.frame == d.current_frame {
                p.page.view = d.view;
            } else if p.document == d.id {
                self.text_canvas = None;
            }
        }
    }
    pub(crate) fn text_canvas_preview(&self, d: &Document) -> Option<&Document> {
        self.text_canvas
            .as_ref()
            .filter(|p| {
                p.document == d.id && p.revision == d.revision && p.frame == d.current_frame
            })
            .map(|p| &p.page)
    }
}
impl Drawer {
    pub(super) fn refresh_text_canvas(&mut self) {
        let Some(e) = self.text_editor.draft.as_mut() else {
            self.drawing.text_canvas = None;
            return;
        };
        let Some(d) = self.documents.iter().find(|d| d.id == e.document) else {
            self.drawing.text_canvas = None;
            return;
        };
        if d.revision != e.revision || d.current_frame != e.frame {
            self.drawing.text_canvas = None;
            return;
        }
        if e.canvas_settings.as_ref() == Some(&e.settings) {
            return;
        }
        e.canvas_settings = Some(e.settings.clone());
        self.drawing.text_canvas = None;
        let result = (|| -> Result<(Document, String), String> {
            let p = core::prepare(d, e.id, &e.settings)?;
            let used: usize = self.documents.iter().map(Document::resident_bytes).sum();
            if used.saturating_add(p.state.memory_bytes().saturating_mul(4))
                > DEFAULT_DOCUMENT_BUDGET
            {
                return Err("Canvas text preview workspace memory limit".into());
            }
            let mut page = drawer_core::layer_merge::preview(&p.state, e.frame)?;
            page.view = d.view;
            Ok((page, p.notice))
        })();
        match result {
            Ok((page, notice)) => {
                e.canvas_notice = notice;
                self.drawing.text_canvas = Some(Box::new(CanvasPreview {
                    document: d.id,
                    revision: d.revision,
                    frame: d.current_frame,
                    page,
                }));
            }
            Err(error) => e.canvas_notice = format!("Canvas text preview: {error}"),
        }
    }
}
pub(super) fn smoke() -> Result<(), String> {
    use super::verify::{click, frame, settle};
    let ctx = egui::Context::default();
    let mut app = Box::new(Drawer::new(&ctx, None));
    app.simple_view = true;
    app.create_canvas(CanvasSpec {
        width: 240,
        height: 160,
        dpi: 300.,
        background: Background::Transparent,
    })
    .map_err(|e| e.to_string())?;
    let original = app.documents[0].snapshot();
    let mut time = 0.;
    app.execute_command(&ctx, "text-editor");
    settle(&ctx, &mut app, &mut time);
    let field = app.text_editor.text.ok_or("Canvas IME field")?;
    click(&ctx, &mut app, &mut time, field);
    let preedit = |text: &str| {
        egui::Event::Ime(egui::ImeEvent::Preedit {
            text: text.into(),
            active_range_chars: None,
        })
    };
    for syllable in ["하", "한", "핝", "한"] {
        frame(&ctx, &mut app, &mut time, vec![preedit(syllable)]);
        settle(&ctx, &mut app, &mut time);
        let p = app
            .drawing
            .text_canvas_preview(&app.documents[0])
            .ok_or("IME canvas preview missing")?;
        let s = Settings::from_object(&p.scene.objects[0]).ok_or("Canvas text object missing")?;
        if s.text != syllable
            || app.canvas_view.rendered_document_id() != Some(p.id)
            || app.documents[0].snapshot() != original
        {
            return Err("IME final consonant/canvas isolation".into());
        }
        let r = drawer_core::render::Renderer::new(p);
        let mut visible = 0;
        for y in 25..60 {
            for x in 55..100 {
                visible += usize::from(r.sample([x as f64 + 0.5, y as f64 + 0.5])?.alpha() > 0.);
            }
        }
        if visible == 0 {
            return Err("IME preedit had no rendered glyph pixels".into());
        }
    }
    let preview_id = app.drawing.text_canvas.as_ref().unwrap().page.id;
    settle(&ctx, &mut app, &mut time);
    if app.drawing.text_canvas.as_ref().unwrap().page.id != preview_id {
        return Err("Unchanged preedit rebuilt the page".into());
    }
    frame(
        &ctx,
        &mut app,
        &mut time,
        vec![egui::Event::WindowFocused(false)],
    );
    if app
        .text_editor
        .draft
        .as_ref()
        .is_none_or(|e| e.composition.is_some() || !e.settings.text.is_empty())
    {
        return Err("App switch retained an uncommitted syllable".into());
    }
    frame(
        &ctx,
        &mut app,
        &mut time,
        vec![egui::Event::WindowFocused(true)],
    );
    settle(&ctx, &mut app, &mut time);
    let field = app.text_editor.text.unwrap();
    click(&ctx, &mut app, &mut time, field);
    frame(&ctx, &mut app, &mut time, vec![preedit("ㅎ")]);
    frame(&ctx, &mut app, &mut time, vec![preedit("")]);
    if app
        .text_editor
        .draft
        .as_ref()
        .is_none_or(|e| e.composition.is_some() || !e.settings.text.is_empty())
    {
        return Err("Empty preedit failed cancellation".into());
    }
    let field = app.text_editor.text.unwrap();
    click(&ctx, &mut app, &mut time, field);
    frame(&ctx, &mut app, &mut time, vec![preedit("한")]);
    frame(
        &ctx,
        &mut app,
        &mut time,
        vec![
            egui::Event::Ime(egui::ImeEvent::Commit("한".into())),
            egui::Event::WindowFocused(false),
        ],
    );
    if app
        .text_editor
        .draft
        .as_ref()
        .is_none_or(|e| e.composition.is_some() || e.settings.text != "한")
    {
        return Err("Commit before focus loss was discarded".into());
    }
    frame(
        &ctx,
        &mut app,
        &mut time,
        vec![egui::Event::WindowFocused(true)],
    );
    settle(&ctx, &mut app, &mut time);
    let field = app.text_editor.text.unwrap();
    click(&ctx, &mut app, &mut time, field);
    frame(&ctx, &mut app, &mut time, vec![preedit("글")]);
    frame(
        &ctx,
        &mut app,
        &mut time,
        vec![
            egui::Event::WindowFocused(false),
            egui::Event::Ime(egui::ImeEvent::Commit("글".into())),
        ],
    );
    if app
        .text_editor
        .draft
        .as_ref()
        .is_none_or(|e| e.composition.is_some() || e.settings.text != "한")
    {
        return Err("Commit after focus loss changed the committed text".into());
    }
    frame(
        &ctx,
        &mut app,
        &mut time,
        vec![egui::Event::WindowFocused(true)],
    );
    settle(&ctx, &mut app, &mut time);
    // A supplied font lacks these glyphs; fallback names and unmapped codepoints are explicit.
    let e = app.text_editor.draft.as_mut().unwrap();
    e.settings.style.font = Font::Emoji;
    e.settings.text = "한 A \u{10ffff}".into();
    settle(&ctx, &mut app, &mut time);
    let e = app.text_editor.draft.as_ref().unwrap();
    if !e.canvas_notice.contains("Fallback characters") || !e.canvas_notice.contains("U+10FFFF") {
        return Err(format!("Font fallback/missing notice: {}", e.canvas_notice));
    }
    let canvas = app.canvas_view.viewport_rect().ok_or("Canvas viewport")?;
    let point = egui::Rect::from_center_size(
        egui::pos2(canvas.right() - 30., canvas.bottom() - 30.),
        egui::vec2(2., 2.),
    );
    click(&ctx, &mut app, &mut time, point);
    if app.documents[0].snapshot() != original {
        return Err("Canvas click painted through text edit".into());
    }
    app.preview_text(&ctx)?;
    app.apply_text()?;
    if app.drawing.text_canvas.is_some() || app.drawing.text_document.is_some() {
        return Err("Applied edit retained live composition cache".into());
    }
    app.documents[0].undo();
    if app.documents[0].snapshot() != original {
        return Err("Canvas preview leaked to document history".into());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn canvas_ime_consonant_correction_app_switch_missing_fonts_and_isolation() {
        super::smoke().unwrap();
    }
}
