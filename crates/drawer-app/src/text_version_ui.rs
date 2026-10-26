use super::*;
use drawer_core::text_version as version;
#[derive(Default)]
pub(super) struct Panel {
    review: Option<Box<Review>>,
    pub apply: Option<egui::Rect>,
    pub cancel: Option<egui::Rect>,
    pub prepare: Option<egui::Rect>,
}
struct Review {
    document: u64,
    revision: u64,
    frame: u32,
    selected: Option<u64>,
    all: bool,
    ack: bool,
    prepared: Option<version::Prepared>,
    images: Option<[egui::TextureHandle; 2]>,
    error: Option<String>,
}
impl Panel {
    pub(super) fn open(&self) -> bool {
        self.review.is_some()
    }
}
impl Drawer {
    pub(crate) fn begin_text_conversion(&mut self) -> Result<(), String> {
        if self.text_editor.draft.is_some()
            || self.transform_unconfirmed()
            || self.inference_unconfirmed()
        {
            return Err("Apply or cancel the pending edit first".into());
        }
        self.finish_stroke();
        let d = &self.documents[self.active.ok_or("Open a page first")?];
        let selected = self
            .panels
            .targets
            .get(&d.id)
            .and_then(|t| drawer_core::layer_catalog::from_thumbnail(d, *t))
            .and_then(|k| match k {
                drawer_core::layer_search::Key::Object(id) => Some(id),
                _ => None,
            });
        self.text_editor.version.review = Some(Box::new(Review {
            document: d.id,
            revision: d.revision,
            frame: d.current_frame,
            selected,
            all: false,
            ack: false,
            prepared: None,
            images: None,
            error: None,
        }));
        self.drawing.text_document = Some(d.id);
        Ok(())
    }
    fn prepare_text_conversion(
        &mut self,
        ctx: &egui::Context,
        e: &mut Review,
    ) -> Result<(), String> {
        e.prepared = None;
        e.images = None;
        e.ack = false;
        let d = &self.documents[self.active.ok_or("Open the source page")?];
        if d.id != e.document || d.revision != e.revision || d.current_frame != e.frame {
            return Err("Source changed; reopen conversion".into());
        }
        let ids = e.selected.map(|id| [id]);
        let p = version::prepare(
            d,
            if e.all {
                None
            } else {
                Some(
                    ids.as_ref()
                        .ok_or("Select text or choose all text on this page")?
                        .as_slice(),
                )
            },
        )?;
        let used: usize = self.documents.iter().map(Document::resident_bytes).sum();
        if used.saturating_add(p.state.memory_bytes().saturating_mul(3)) > DEFAULT_DOCUMENT_BUDGET {
            return Err("Conversion preview workspace memory limit".into());
        }
        let after = drawer_core::layer_merge::preview(&p.state, e.frame)?;
        e.images = Some([
            crate::layer_merge::image(ctx, d, "text-generation-before"),
            crate::layer_merge::image(ctx, &after, "text-generation-after"),
        ]);
        e.prepared = Some(p);
        Ok(())
    }
    pub(crate) fn text_conversion_window(&mut self, ctx: &egui::Context) {
        let Some(mut e) = self.text_editor.version.review.take() else {
            return;
        };
        let fresh = self.active.is_some_and(|n| {
            let d = &self.documents[n];
            d.id == e.document && d.revision == e.revision && d.current_frame == e.frame
        });
        let (mut open, mut prepare, mut apply, mut cancel) = (true, false, false, false);
        egui::Window::new("Drawer text layout conversion").default_width(520.).open(&mut open).show(ctx,|ui|{
            ui.label("Opening a drawing never upgrades its text. Conversion keeps Unicode and formatting, but glyph positions, wrapping and fallback can change. Compare before and after; keep the original if the result differs.");
            ui.horizontal(|ui|{let old=e.all;ui.radio_value(&mut e.all,false,"Selected text");ui.radio_value(&mut e.all,true,"All text on this page");if old!=e.all{e.prepared=None;e.images=None;e.ack=false;}});
            if !fresh{ui.colored_label(egui::Color32::YELLOW,"Source changed; close and reopen.");}
            if let Some(error)=&e.error{ui.colored_label(egui::Color32::YELLOW,error);}
            if let Some(images)=&e.images{ui.horizontal(|ui|{for (label,image) in ["Before","After"].into_iter().zip(images){ui.vertical(|ui|{ui.label(label);ui.add(egui::Image::new(image).fit_to_exact_size(egui::vec2(160.,160.)));});}});}
            if let Some(p)=&e.prepared{ui.label(format!("{} text object(s) will change to Drawer layout {}",p.changed,version::CURRENT));egui::ScrollArea::vertical().max_height(100.).show(ui,|ui|{for note in &p.notices{ui.label(note);}});ui.checkbox(&mut e.ack,"I reviewed the layout and font differences");}
            ui.horizontal(|ui|{
                let r=ui.add_enabled(fresh,egui::Button::new("Prepare conversion preview"));prepare=r.clicked();{self.text_editor.version.prepare=Some(r.rect);}
                let r=ui.add_enabled(fresh&&e.ack&&e.prepared.as_ref().is_some_and(|p|p.changed>0),egui::Button::new("Apply one Undo"));apply=r.clicked();{self.text_editor.version.apply=Some(r.rect);}
                let r=ui.button("Cancel conversion");cancel=r.clicked();{self.text_editor.version.cancel=Some(r.rect);}
            });
        });
        if !open || cancel || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.drawing.text_document = None;
            return;
        }
        if prepare {
            e.error = self.prepare_text_conversion(ctx, &mut e).err();
        }
        if apply && fresh && e.ack {
            let result =
                self.documents[self.active.unwrap()].commit_state(e.prepared.take().unwrap().state);
            match result {
                Ok(_) => {
                    self.drawing.text_document = None;
                    return;
                }
                Err(error) => e.error = Some(error),
            }
        }
        self.drawing.text_document = Some(e.document);
        self.text_editor.version.review = Some(e);
    }
}
#[path = "text_version_ui_verify.rs"]
mod verify;
pub(super) fn smoke(root: &std::path::Path) -> Result<(), String> {
    verify::smoke(root)
}
