#[path = "text_canvas.rs"]
mod canvas;
#[path = "text_composition.rs"]
mod composition;
#[path = "text_editor_verify.rs"]
mod verify;
pub(super) use canvas::CanvasPreview;
pub(super) fn smoke(root: &std::path::Path) -> Result<(), String> {
    verify::smoke(root)?;
    verify::composition_smoke()?;
    canvas::smoke()
}
use super::*;
use drawer_core::text_layout::{self as core, Align, Font, Settings, Style};
#[derive(Default)]
pub(super) struct Panel {
    pub draft: Option<Box<Draft>>,
    pub text: Option<egui::Rect>,
    pub preview: Option<egui::Rect>,
    pub apply: Option<egui::Rect>,
    pub cancel: Option<egui::Rect>,
}
pub(super) struct Draft {
    pub(super) document: u64,
    revision: u64,
    frame: u32,
    id: Option<u64>,
    settings: Settings,
    composition: Option<Box<composition::Composition>>,
    prepared: Option<(Settings, core::Prepared)>,
    image: Option<egui::TextureHandle>,
    canvas_settings: Option<Settings>,
    canvas_notice: String,
}
impl Drawer {
    pub(super) fn text_save_guard(&self, index: usize) -> Result<(), String> {
        if self
            .text_editor
            .draft
            .as_ref()
            .is_some_and(|e| e.document == self.documents[index].id)
        {
            Err("Apply or cancel the text edit before saving".into())
        } else {
            Ok(())
        }
    }
    pub(super) fn begin_text_editor(&mut self, ctx: &egui::Context) -> Result<(), String> {
        if self.text_editor.draft.is_some() {
            return Err("Apply or cancel the existing text edit first".into());
        }
        self.finish_stroke();
        if self.transform_unconfirmed()
            || self.inference_unconfirmed()
            || self.batch_filters.draft.is_some()
            || self.detail_filter.draft.is_some()
            || self.surface_effects.draft.is_some()
        {
            return Err("Apply or cancel the pending edit first".into());
        }
        let d = &self.documents[self.active.ok_or("Open a page first")?];
        let selected = self
            .panels
            .targets
            .get(&d.id)
            .and_then(|t| drawer_core::layer_catalog::from_thumbnail(d, *t));
        let old = selected.and_then(|key| {
            if let drawer_core::layer_search::Key::Object(id) = key {
                drawer_core::layer_catalog::source_index(d, key)
                    .and_then(|n| Settings::from_object(&d.scene.objects[n]).map(|s| (id, s)))
            } else {
                None
            }
        });
        let (id, settings) = old.map_or_else(
            || {
                (
                    None,
                    Settings {
                        baseline: d
                            .scene
                            .transform
                            .inverse()
                            .map([d.width as f64 / 4., d.height as f64 / 3.]),
                        text: String::new(),
                        size: 24.,
                        color: self.references.session.workspace.color,
                        style: Style {
                            wrap_width: (d.width as f64 / 2.).clamp(64., 32768.),
                            ..Default::default()
                        },
                    },
                )
            },
            |(id, s)| (Some(id), s),
        );
        self.text_editor.draft = Some(Box::new(Draft {
            document: d.id,
            revision: d.revision,
            frame: d.current_frame,
            id,
            settings,
            composition: None,
            prepared: None,
            image: None,
            canvas_settings: None,
            canvas_notice: String::new(),
        }));
        self.drawing.text_document = Some(d.id);
        let e = self.text_editor.draft.as_ref().unwrap();
        let id = composition::widget_id(e);
        let mut state = egui::text_edit::TextEditState::default();
        state
            .cursor
            .set_char_range(Some(egui::text::CCursorRange::one(
                egui::text::CCursor::new(e.settings.text.chars().count()),
            )));
        state.store(ctx, id);
        Ok(())
    }
    fn preview_text(&mut self, ctx: &egui::Context) -> Result<(), String> {
        let n = self.active.ok_or("Open a page")?;
        let e = self
            .text_editor
            .draft
            .as_mut()
            .ok_or("Text editor closed")?;
        if e.composition.is_some() {
            return Err("Finish or cancel the IME composition first".into());
        }
        e.prepared = None;
        e.image = None;
        let d = &self.documents[n];
        if d.id != e.document || d.revision != e.revision || d.current_frame != e.frame {
            return Err("Source changed; reopen the text editor".into());
        }
        let p = core::prepare(d, e.id, &e.settings)?;
        let used: usize = self.documents.iter().map(Document::resident_bytes).sum();
        if used.saturating_add(p.state.memory_bytes().saturating_mul(3)) > DEFAULT_DOCUMENT_BUDGET {
            return Err("Text preview workspace memory limit".into());
        }
        let after = drawer_core::layer_merge::preview(&p.state, e.frame)?;
        e.image = Some(crate::layer_merge::image(
            ctx,
            &after,
            "text-layout-preview",
        ));
        e.prepared = Some((e.settings.clone(), p));
        Ok(())
    }
    fn apply_text(&mut self) -> Result<(), String> {
        let n = self.active.ok_or("Open a page")?;
        let e = self
            .text_editor
            .draft
            .as_mut()
            .ok_or("Text editor closed")?;
        let d = &self.documents[n];
        if e.composition.is_some()
            || d.id != e.document
            || d.revision != e.revision
            || d.current_frame != e.frame
            || e.prepared.as_ref().is_none_or(|(s, _)| *s != e.settings)
        {
            return Err("Prepare a fresh text preview first".into());
        }
        let (_, p) = e.prepared.take().unwrap();
        self.documents[n].commit_state(p.state)?;
        self.panels.targets.insert(
            self.documents[n].id,
            drawer_core::thumbnail::Target::Source(drawer_core::layer_search::Key::Object(p.id)),
        );
        self.text_editor.draft = None;
        self.drawing.text_document = None;
        self.drawing.text_canvas = None;
        self.status = None;
        Ok(())
    }
    pub(super) fn text_editor_window(&mut self, ctx: &egui::Context) {
        let Some(mut e) = self.text_editor.draft.take() else {
            self.drawing.text_document = None;
            self.drawing.text_canvas = None;
            return;
        };
        let fresh = self.active.is_some_and(|n| {
            let d = &self.documents[n];
            d.id == e.document && d.revision == e.revision && d.current_frame == e.frame
        });
        let (ime_commit, ime_cancel) = composition::before(ctx, &mut e);
        let mut open = ime_cancel || !ctx.input(|i| i.key_pressed(egui::Key::Escape));
        let (mut preview, mut apply, mut cancel) = (false, false, false);
        let old = e.settings.clone();
        egui::Window::new("Text content and layout").default_width(470.).default_pos(egui::pos2(25.,25.)).open(&mut open).show(ctx,|ui|{
            ui.label(if e.id.is_some(){"Edit selected text object"}else{"Create a new editable text object"});if !fresh{ui.colored_label(egui::Color32::YELLOW,"Document changed; cancel and reopen.");}
            let widget_id=composition::widget_id(&e);let mut output=egui::TextEdit::multiline(&mut e.settings.text).id(widget_id).char_limit(4096).desired_rows(4).desired_width(430.).show(ui);self.text_editor.text=Some(output.response.rect);composition::after(ctx,&mut e,&mut output,ime_commit);
            egui::ComboBox::from_id_salt("text-font").selected_text(e.settings.style.font.label()).show_ui(ui,|ui|{for font in Font::ALL{ui.selectable_value(&mut e.settings.style.font,font,font.label());}});
            ui.horizontal(|ui|{ui.add(egui::DragValue::new(&mut e.settings.size).range(0.1..=4096.).prefix("Size px "));ui.add(egui::DragValue::new(&mut e.settings.style.line_height).range(0.5..=8.).prefix("Line spacing × "));});
            ui.horizontal(|ui|{for(value,label)in [(Align::Left,"Left"),(Align::Center,"Center"),(Align::Right,"Right")]{ui.radio_value(&mut e.settings.style.align,value,label);}});
            ui.horizontal(|ui|{ui.add(egui::DragValue::new(&mut e.settings.style.wrap_width).range(0. ..=32768.).prefix("Wrap width px (0 = none) "));});
            ui.horizontal(|ui|{for(v,label)in e.settings.baseline.iter_mut().zip(["Position X ","Baseline Y "]){ui.add(egui::DragValue::new(v).range(-1e6..=1e6).prefix(label));}ui.color_edit_button_srgba_unmultiplied(&mut e.settings.color);});
            if e.settings!=old{e.prepared=None;e.image=None;}
            let r=ui.add_enabled(fresh&&e.composition.is_none(),egui::Button::new("Prepare text preview"));self.text_editor.preview=Some(r.rect);preview=r.clicked();
            if !e.canvas_notice.is_empty(){ui.label(&e.canvas_notice);}
            if e.composition.is_some(){ui.label("IME composition preview on canvas; finish or cancel before Apply.");}
            if let Some(image)=&e.image{ui.add(egui::Image::new(image).fit_to_exact_size(egui::vec2(120.,120.)));}
            let r=ui.add_enabled(fresh&&e.composition.is_none()&&e.prepared.is_some(),egui::Button::new("Apply text as one Undo"));self.text_editor.apply=Some(r.rect);apply=r.clicked();let r=ui.button("Cancel text edit");self.text_editor.cancel=Some(r.rect);cancel=r.clicked();
            ui.small("Text field Undo handles local typing/composition. Apply changes the document once. Local system fonts are read without copying them into the package. Emoji uses monochrome outlines.");
        });
        if open && !cancel {
            self.text_editor.draft = Some(e);
            self.refresh_text_canvas();
        } else {
            self.drawing.text_document = None;
            self.drawing.text_canvas = None;
        }
        if preview && let Err(error) = self.preview_text(ctx) {
            self.status = Some(error);
        }
        if apply && let Err(error) = self.apply_text() {
            self.status = Some(error);
        }
    }
}
