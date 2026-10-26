use super::*;
use drawer_core::{font_library::owned, text_path as path};
#[derive(Default)]
pub(super) struct Panel {
    files: bool,
    owned: Vec<owned::Owned>,
    source: Option<PathBuf>,
    licensed: bool,
    confirmation: String,
    error: Option<String>,
    pending: Option<Box<Pending>>,
    output: bool,
    ack: Option<(u64, u64, u32)>,
    pub install: Option<egui::Rect>,
    pub remove: Option<egui::Rect>,
    pub output_ack: Option<egui::Rect>,
    pub preview: Option<egui::Rect>,
    pub confirm: Option<egui::Rect>,
    pub cancel: Option<egui::Rect>,
}
struct Pending {
    circle: path::Circle,
    prepared: Option<(Settings, Settings, [egui::TextureHandle; 2])>,
}
fn circle_controls(ui: &mut egui::Ui, c: &mut path::Circle) {
    ui.add(
        egui::DragValue::new(&mut c.radius)
            .range(0.1..=32768.)
            .prefix("Radius px "),
    );
    ui.add(
        egui::DragValue::new(&mut c.angle)
            .range(-360. ..=360.)
            .prefix("Start angle ° "),
    );
    ui.horizontal(|ui| {
        ui.selectable_value(&mut c.side, path::Side::Outside, "Outside");
        ui.selectable_value(&mut c.side, path::Side::Inside, "Inside");
    });
    ui.checkbox(&mut c.equal, "Equal spacing around the circle");
}
pub(super) fn controls(ui: &mut egui::Ui, e: &mut Draft, p: &mut Panel, blocked: bool) {
    egui::CollapsingHeader::new("Circular layout and regular text effects").show(ui, |ui| {
        ui.add_enabled_ui(!blocked, |ui| {
            if let Some(c) = &mut e.settings.style.circle {
                circle_controls(ui, c);
                if ui
                    .button("Return to regular layout (restore all formatting)")
                    .clicked()
                {
                    e.settings.style.circle = None;
                }
            } else if ui.button("Preview circular layout conversion…").clicked() {
                p.pending = Some(Box::new(Pending {
                    circle: Default::default(),
                    prepared: None,
                }));
            }
            for note in path::hidden(&e.settings.text, &e.settings.style) {
                ui.colored_label(egui::Color32::YELLOW, note);
            }
            ui.add_enabled_ui(e.settings.style.circle.is_none(), |ui| {
                if e.settings.style.effects.is_none()
                    && ui
                        .button("Enable underline / strikethrough / slant")
                        .clicked()
                {
                    match drawer_core::text_version::upgrade(&e.settings.style, &e.settings.text) {
                        Ok(mut s) => {
                            s.effects = Some(Default::default());
                            e.settings.style = s
                        }
                        Err(error) => p.error = Some(error),
                    }
                }
                if let Some(fx) = &mut e.settings.style.effects {
                    ui.checkbox(&mut fx.underline, "Underline");
                    ui.checkbox(&mut fx.strike, "Strikethrough");
                    ui.add(
                        egui::DragValue::new(&mut fx.slant)
                            .range(-60. ..=60.)
                            .prefix("Slant ° "),
                    );
                }
            });
            if let Some(error) = &p.error {
                ui.colored_label(egui::Color32::YELLOW, error);
            }
        });
    });
}
impl Drawer {
    pub(crate) fn begin_font_files(&mut self) -> Result<(), String> {
        let storage = self
            .storage
            .as_ref()
            .ok_or("Local app storage unavailable")?;
        self.text_editor.paths.owned = owned::load(storage).map_err(|e| e.to_string())?;
        self.text_editor.paths.files = true;
        self.text_editor.paths.licensed = false;
        self.text_editor.paths.confirmation.clear();
        Ok(())
    }
    pub(super) fn install_font_copy(
        &mut self,
        source: &std::path::Path,
        licensed: bool,
    ) -> Result<(), String> {
        let storage = self
            .storage
            .as_ref()
            .ok_or("Local app storage unavailable")?;
        owned::install(storage, source, licensed).map_err(|e| e.to_string())?;
        self.text_editor.paths.owned = owned::load(storage).map_err(|e| e.to_string())?;
        self.refresh_font_catalog();
        Ok(())
    }
    pub(super) fn remove_font_copy(
        &mut self,
        digest: [u8; 32],
        confirmation: &str,
    ) -> Result<(), String> {
        let storage = self
            .storage
            .as_ref()
            .ok_or("Local app storage unavailable")?;
        owned::remove(storage, digest, confirmation).map_err(|e| e.to_string())?;
        self.text_editor.paths.owned = owned::load(storage).map_err(|e| e.to_string())?;
        self.refresh_font_catalog();
        Ok(())
    }
    pub(crate) fn text_path_escape(&mut self, ctx: &egui::Context) {
        let p = &mut self.text_editor.paths;
        if (p.pending.is_some() || p.files || p.output)
            && ctx.input(|i| i.key_pressed(egui::Key::Escape))
        {
            if p.pending.is_some() {
                p.pending = None;
            } else if p.files {
                p.files = false;
            } else {
                p.output = false;
            }
            ctx.input_mut(|i| {
                i.consume_key(egui::Modifiers::NONE, egui::Key::Escape);
            });
        }
    }
    fn prepare_circle_preview(
        &mut self,
        ctx: &egui::Context,
        pending: &mut Pending,
    ) -> Result<(), String> {
        pending.prepared = None;
        let e = self
            .text_editor
            .draft
            .as_ref()
            .ok_or("Open the text editor")?;
        if e.composition.is_some() {
            return Err("Finish or cancel IME composition first".into());
        }
        let d = self
            .documents
            .iter()
            .find(|d| d.id == e.document)
            .ok_or("Source closed")?;
        if d.revision != e.revision || d.current_frame != e.frame {
            return Err("Source changed; reopen the text editor".into());
        }
        if !pending.circle.valid() {
            return Err("Circle radius must be positive and within32768px".into());
        }
        let mut after = e.settings.clone();
        after.style = drawer_core::text_version::upgrade(&after.style, &after.text)?;
        after.style.circle = Some(pending.circle);
        let a = core::prepare(d, e.id, &e.settings)?;
        let b = core::prepare(d, e.id, &after)?;
        let used: usize = self.documents.iter().map(Document::resident_bytes).sum();
        if used.saturating_add(a.state.memory_bytes().saturating_mul(6)) > DEFAULT_DOCUMENT_BUDGET {
            return Err("Circle preview workspace budget".into());
        }
        let before_page = drawer_core::layer_merge::preview(&a.state, e.frame)?;
        let after_page = drawer_core::layer_merge::preview(&b.state, e.frame)?;
        pending.prepared = Some((
            e.settings.clone(),
            after,
            [
                crate::layer_merge::image(ctx, &before_page, "circle-before"),
                crate::layer_merge::image(ctx, &after_page, "circle-after"),
            ],
        ));
        Ok(())
    }
    pub(crate) fn begin_text_output_review(&mut self) {
        self.text_editor.paths.output = true;
    }
    pub(crate) fn text_output_guard(&self, index: usize) -> Result<(), String> {
        let d = &self.documents[index];
        let notes = path::output_notes(d);
        if !notes.is_empty()
            && self.text_editor.paths.ack != Some((d.id, d.revision, d.current_frame))
        {
            return Err(format!(
                "{} Open Circular text output review in All tools to acknowledge hidden content before exporting.",
                notes.join(" ")
            ));
        }
        Ok(())
    }
    pub(crate) fn text_path_windows(&mut self, ctx: &egui::Context) {
        if self.text_editor.paths.files {
            let mut p = std::mem::take(&mut *self.text_editor.paths);
            let (mut open, mut install, mut remove) = (true, false, None);
            egui::Window::new("App-owned local font files").default_width(620.).open(&mut open).show(ctx,|ui|{
    ui.label("Only app-owned copies are removed. System fonts and the selected source file are preserved.");
    if ui.button("Choose a local font file").clicked(){p.source=rfd::FileDialog::new().add_filter("Fonts",&["ttf","otf","ttc","otc"]).pick_file();p.licensed=false;}
    if let Some(source)=&p.source{ui.label(source.display().to_string());}ui.checkbox(&mut p.licensed,"I have permission to copy this font for local use");let r=ui.add_enabled(p.source.is_some()&&p.licensed,egui::Button::new("Add app-owned copy"));install=r.clicked();p.install=Some(r.rect);
    ui.separator();ui.label("Recently applied fonts");for font in &self.workspaces.store.current.font_lists.recent{ui.label(font.label());}
    ui.separator();ui.add(egui::TextEdit::singleline(&mut p.confirmation).hint_text("Type the first displayed face name to confirm removal"));
    egui::ScrollArea::vertical().max_height(260.).show(ui,|ui|{for entry in &p.owned{let used=self.documents.iter().flat_map(|d|&d.scene.objects).filter(|o|core::Settings::from_object(o).is_some_and(|s|s.style.fonts().iter().any(|f|matches!(f,Font::Local(r)if entry.faces.contains(r))))).count();ui.group(|ui|{ui.small(&entry.file);for r in &entry.faces{ui.label(format!("{} · face{}",r.name(),r.index));}ui.label(format!("{used} open text object(s) reference this font. Removal can cause fallback; Unicode and formatting remain saved."));let r=ui.add_enabled(p.confirmation==entry.faces[0].name(),egui::Button::new("Remove this app-owned copy / collection"));p.remove=Some(r.rect);if r.clicked(){remove=Some(entry.digest);}});}});
    if let Some(error)=&p.error{ui.colored_label(egui::Color32::YELLOW,error);}
   });
            p.files = open;
            let source = p.source.clone();
            let licensed = p.licensed;
            let confirmation = p.confirmation.clone();
            *self.text_editor.paths = p;
            let result = if install {
                self.install_font_copy(source.as_ref().unwrap(), licensed)
            } else if let Some(digest) = remove {
                self.remove_font_copy(digest, &confirmation)
            } else {
                Ok(())
            };
            if let Err(error) = result {
                self.text_editor.paths.error = Some(error);
            }
        }
        if let Some(mut pending) = self.text_editor.paths.pending.take() {
            let mut open = true;
            let (mut preview, mut confirm, mut cancel) = (false, false, false);
            egui::Window::new("Confirm circular text layout").default_width(560.).open(&mut open).show(ctx,|ui|{
    ui.label("Only the first explicit line is rendered. Later lines, ruby, underline, strikethrough, wrapping and slant remain saved and return in regular mode.");let before=pending.circle;circle_controls(ui,&mut pending.circle);if before!=pending.circle{pending.prepared=None;}
    if let Some(e)=&self.text_editor.draft{let mut style=e.settings.style.clone();style.circle=Some(pending.circle);for note in path::hidden(&e.settings.text,&style){ui.label(note);}}
    if let Some((_,_,images))=&pending.prepared{ui.horizontal(|ui|{for(label,image)in ["Regular before","Circular after"].into_iter().zip(images){ui.vertical(|ui|{ui.label(label);ui.add(egui::Image::new(image).fit_to_exact_size(egui::vec2(150.,150.)));});}});}
    let fresh=self.text_editor.draft.as_ref().is_some_and(|e|e.composition.is_none()&&pending.prepared.as_ref().is_some_and(|(base,_,_)|*base==e.settings));
    ui.horizontal(|ui|{let r=ui.button("Prepare circular preview");preview=r.clicked();self.text_editor.paths.preview=Some(r.rect);let r=ui.add_enabled(fresh,egui::Button::new("Confirm preserved hidden content"));confirm=r.clicked();self.text_editor.paths.confirm=Some(r.rect);let r=ui.button("Cancel conversion");cancel=r.clicked();self.text_editor.paths.cancel=Some(r.rect);});
    if let Some(error)=&self.text_editor.paths.error{ui.colored_label(egui::Color32::YELLOW,error);}
   });
            if preview && let Err(error) = self.prepare_circle_preview(ctx, &mut pending) {
                self.text_editor.paths.error = Some(error);
            }
            if confirm {
                if let Some((_, after, _)) = pending.prepared.take()
                    && let Some(e) = &mut self.text_editor.draft
                {
                    e.settings = after;
                    e.prepared = None;
                    e.image = None;
                    e.canvas_settings = None;
                }
            } else if open && !cancel {
                self.text_editor.paths.pending = Some(pending);
            }
        }
        if self.text_editor.paths.output {
            let mut open = true;
            let mut close = false;
            egui::Window::new("Circular text output review").open(&mut open).show(ctx,|ui|{if let Some(n)=self.active{let d=&self.documents[n];for note in path::output_notes(d){ui.label(note);}ui.label("The rendered export contains only visible content. Keep the .drawer original for all Unicode and formatting.");let r=ui.button("Acknowledge this document/frame for export");self.text_editor.paths.output_ack=Some(r.rect);if r.clicked(){self.text_editor.paths.ack=Some((d.id,d.revision,d.current_frame));close=true;}}});
            self.text_editor.paths.output = open && !close;
        }
    }
}
pub(super) fn smoke(root: &std::path::Path) -> Result<(), String> {
    verify::smoke(root)
}
#[path = "text_path_ui_verify.rs"]
mod verify;
