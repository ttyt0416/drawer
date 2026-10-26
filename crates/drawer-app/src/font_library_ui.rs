use super::*;
use drawer_core::font_library::{self as fonts, Catalog, Display, Settings};
#[derive(Clone)]
struct Selector {
    catalog: Arc<Catalog>,
    settings: Settings,
}
#[derive(Default, Clone)]
struct Previews {
    fonts: Vec<Font>,
    bytes: usize,
}
struct Scan {
    cancel: Arc<std::sync::atomic::AtomicBool>,
    receiver: std::sync::mpsc::Receiver<Catalog>,
}
impl Drop for Scan {
    fn drop(&mut self) {
        self.cancel
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }
}
pub(super) struct Panel {
    pub open: bool,
    loaded: bool,
    pub catalog: Arc<Catalog>,
    scan: Option<Scan>,
    pub draft: Option<Settings>,
    pub restore: Option<Settings>,
    pub name: String,
    pub confirm: String,
    pub query: String,
    pub favorites_only: bool,
    pub character_font: Font,
    pub code: u32,
    pub block: u32,
    pub error: Option<String>,
    pub create: Option<egui::Rect>,
    pub apply: Option<egui::Rect>,
    pub cancel: Option<egui::Rect>,
    pub insert: Option<egui::Rect>,
    pub rename: Option<egui::Rect>,
}
impl Default for Panel {
    fn default() -> Self {
        Self {
            open: false,
            loaded: false,
            catalog: Arc::new(Catalog::discover(&[], || false)),
            scan: None,
            draft: None,
            restore: None,
            name: String::new(),
            confirm: String::new(),
            query: String::new(),
            favorites_only: false,
            character_font: Font::Korean,
            code: 0x2605,
            block: 0x2600,
            error: None,
            create: None,
            apply: None,
            cancel: None,
            insert: None,
            rename: None,
        }
    }
}
fn selector_id() -> egui::Id {
    egui::Id::new("drawer-local-font-selector")
}
fn choices(ctx: &egui::Context) -> (Vec<Font>, Display) {
    let selected = ctx.data(|d| d.get_temp::<Selector>(selector_id()));
    selected.map_or_else(
        || (Font::ALL.to_vec(), Display::Ui),
        |s| {
            let display = s
                .settings
                .active
                .and_then(|id| s.settings.lists.iter().find(|l| l.id == id))
                .map_or(Display::Ui, |l| l.display);
            (s.settings.choices(&s.catalog), display)
        },
    )
}
fn preview(ctx: &egui::Context, font: Font) -> Option<egui::FontFamily> {
    let name = format!("local-font-preview-{font:?}");
    let family = egui::FontFamily::Name(name.clone().into());
    if let Font::Local(reference) = font
        && fonts::data(reference).is_none()
    {
        return None;
    }
    if font == Font::Korean && !core::font_supports(font, 'A') {
        return None;
    }

    if ctx.fonts(|f| f.definitions().families.contains_key(&family)) {
        return Some(family);
    }
    let id = egui::Id::new("font-library-preview-budget");
    let mut cache = ctx.data(|d| d.get_temp::<Previews>(id)).unwrap_or_default();
    if cache.fonts.contains(&font) || cache.fonts.len() >= 32 {
        return None;
    }
    let (data, index) = core::font_bytes(font)?;
    if cache.bytes + data.len() > 64 * 1024 * 1024 {
        return None;
    }
    cache.bytes += data.len();
    cache.fonts.push(font);
    ctx.data_mut(|d| d.insert_temp(id, cache));
    let mut data = egui::FontData::from_owned(data.as_ref().clone());
    data.index = index;
    ctx.add_font(egui::epaint::text::FontInsert::new(
        &name,
        data,
        vec![egui::epaint::text::InsertFontFamily {
            family: family.clone(),
            priority: egui::epaint::text::FontPriority::Highest,
        }],
    ));
    ctx.request_repaint();
    None
}
fn label(ctx: &egui::Context, font: Font, display: Display) -> egui::RichText {
    let text = egui::RichText::new(font.label());
    if display == Display::OwnFont
        && core::font_covers(font, &font.label())
        && let Some(f) = preview(ctx, font)
    {
        text.font(egui::FontId::new(14., f))
    } else {
        text
    }
}
pub(super) fn picker(ui: &mut egui::Ui, id: &str, font: &mut Font) {
    let (values, display) = choices(ui.ctx());
    egui::ComboBox::from_id_salt(id)
        .selected_text(font.label())
        .show_ui(ui, |ui| {
            ui.ctx()
                .data_mut(|d| d.insert_temp(egui::Id::new("request-font-scan"), true));
            if values.is_empty() {
                ui.label("Selected font list is empty. Add fonts in Font lists.");
            }
            let qid = ui.id().with("font-search");
            let mut query = ui
                .ctx()
                .data(|d| d.get_temp::<String>(qid))
                .unwrap_or_default();
            ui.add(egui::TextEdit::singleline(&mut query).hint_text("Search fonts"));
            ui.ctx().data_mut(|d| d.insert_temp(qid, query.clone()));
            egui::ScrollArea::vertical()
                .max_height(200.)
                .show(ui, |ui| {
                    for f in values
                        .into_iter()
                        .filter(|f| f.label().to_lowercase().contains(&query.to_lowercase()))
                    {
                        ui.selectable_value(font, f, label(ui.ctx(), f, display));
                    }
                });
            if ui.button("Manage font lists and characters").clicked() {
                ui.ctx()
                    .data_mut(|d| d.insert_temp(egui::Id::new("open-font-library"), true));
            }
        });
}
impl Drawer {
    fn publish_fonts(&self, ctx: &egui::Context) {
        ctx.data_mut(|d| {
            d.insert_temp(
                selector_id(),
                Selector {
                    catalog: self.text_editor.fonts.catalog.clone(),
                    settings: self.workspaces.store.current.font_lists.clone(),
                },
            )
        });
    }
    pub(crate) fn font_library_escape(&mut self, ctx: &egui::Context) {
        if self.text_editor.fonts.open && ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.text_editor.fonts.open = false;
            self.text_editor.fonts.draft = None;
            ctx.input_mut(|i| {
                i.events.retain(|v| {
                    !matches!(
                        v,
                        egui::Event::Key {
                            key: egui::Key::Escape,
                            ..
                        }
                    )
                })
            });
        }
    }
    pub(crate) fn begin_font_library(&mut self, ctx: &egui::Context) {
        let p = &mut self.text_editor.fonts;
        p.open = true;
        p.draft = Some(self.workspaces.store.current.font_lists.clone());
        p.error = None;
        p.restore = self.workspaces.store.font_restore.as_deref().cloned();
        self.refresh_font_catalog();
        self.publish_fonts(ctx);
    }
    fn refresh_font_catalog(&mut self) {
        self.text_editor.fonts.loaded = true;
        let roots = fonts::default_roots(self.storage.as_ref().map(|s| s.root()));
        let (cancel, receiver) = {
            let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
            let (c, r) = std::sync::mpsc::channel();
            let token = cancel.clone();
            std::thread::spawn(move || {
                let catalog =
                    Catalog::discover(&roots, || token.load(std::sync::atomic::Ordering::Relaxed));
                let _ = c.send(catalog);
            });
            (cancel, r)
        };
        self.text_editor.fonts.scan = Some(Scan { cancel, receiver });
    }
    pub(super) fn commit_fonts(
        &mut self,
        ctx: &egui::Context,
        mut next: Settings,
    ) -> Result<(), String> {
        if !next.valid() {
            return Err("Invalid font-list settings".into());
        }
        let old = self.workspaces.store.current.font_lists.clone();
        next.next_id = next.next_id.max(old.next_id);
        let previous = self.workspaces.store.font_restore.clone();
        self.workspaces.store.font_restore = Some(Box::new(old.clone()));
        self.workspaces.store.current.font_lists = next.clone();
        if let Err(error) = self.persist_workspace() {
            self.workspaces.store.current.font_lists = old;
            self.workspaces.store.font_restore = previous;
            return Err(error);
        }
        self.text_editor.fonts.restore = Some(old);
        self.text_editor.fonts.draft = Some(next);
        self.publish_fonts(ctx);
        Ok(())
    }
    pub(super) fn insert_character(
        &mut self,
        ctx: &egui::Context,
        font: Font,
        code: u32,
    ) -> Result<(), String> {
        let (c, _) = fonts::character(font, code)?;
        if self.text_editor.draft.is_none() {
            self.begin_text_editor(ctx)?;
        }
        let e = self.text_editor.draft.as_mut().unwrap();
        if e.composition.is_some() {
            return Err("Finish or cancel IME composition before inserting a character".into());
        }
        let id = composition::widget_id(e);
        let mut state = egui::text_edit::TextEditState::load(ctx, id).unwrap_or_default();
        let range = state.cursor.char_range().unwrap_or_else(|| {
            egui::text::CCursorRange::one(egui::text::CCursor::new(e.settings.text.chars().count()))
        });
        let a = usize::from(range.primary.index).min(usize::from(range.secondary.index));
        let b = usize::from(range.primary.index).max(usize::from(range.secondary.index));
        let (a, b) = fmt_text::expand(&e.settings.text, a, b);
        if e.settings.text.chars().count().saturating_sub(b - a) + 1 > 4096 {
            return Err("Character insertion exceeds4096 Unicode characters".into());
        }
        let old = e.settings.clone();
        let mut next = old.clone();
        let from = next
            .text
            .char_indices()
            .nth(a)
            .map_or(next.text.len(), |(n, _)| n);
        let to = next
            .text
            .char_indices()
            .nth(b)
            .map_or(next.text.len(), |(n, _)| n);
        next.text.replace_range(from..to, &c.to_string());
        next.style.remap_text(&old.text, &next.text);
        let rich = next.style.rich.get_or_insert_with(Box::default);
        let (start, end) = fmt_text::expand(&next.text, a, a + 1);
        let mut attr = rich.resolve(
            fmt_text::slice(&next.text, start, end),
            start,
            next.style.font,
        );
        attr.font = Some(font);
        rich.set(&next.text, start, end, Some(attr))?;
        reading_ui::remember(
            e,
            old.text.clone(),
            old.style.ruby.clone(),
            old.style.rich.clone(),
        );
        let mut undo = state.undoer();
        undo.add_undo(&(range, old.text));
        state.set_undoer(undo);
        e.settings = next;
        e.prepared = None;
        e.image = None;
        state
            .cursor
            .set_char_range(Some(egui::text::CCursorRange::one(
                egui::text::CCursor::new(end),
            )));
        state.store(ctx, id);
        self.refresh_text_canvas();
        ctx.request_repaint();
        Ok(())
    }
    fn accept_font_catalog(&mut self, ctx: &egui::Context, catalog: Catalog) {
        core::invalidate_font_data();
        self.canvas_view.invalidate_font_display();
        self.display.canvas.invalidate_font_display();
        self.drawing.text_canvas = None;
        if let Some(e) = &mut self.text_editor.draft {
            e.canvas_settings = None;
            e.prepared = None;
            e.image = None;
        }

        self.text_editor.fonts.catalog = Arc::new(catalog);
        self.text_editor.fonts.scan = None;
        self.publish_fonts(ctx);
        ctx.request_repaint();
    }
    pub(crate) fn font_library_window(&mut self, ctx: &egui::Context) {
        if ctx
            .data_mut(|d| d.remove_temp::<bool>(egui::Id::new("request-font-scan")))
            .unwrap_or(false)
            && !self.text_editor.fonts.loaded
        {
            self.refresh_font_catalog();
        }
        let changed = ctx
            .data(|d| d.get_temp::<Selector>(selector_id()))
            .is_none_or(|s| {
                s.settings != self.workspaces.store.current.font_lists
                    || !Arc::ptr_eq(&s.catalog, &self.text_editor.fonts.catalog)
            });
        if changed {
            self.publish_fonts(ctx);
        }
        if ctx
            .data_mut(|d| d.remove_temp::<bool>(egui::Id::new("open-font-library")))
            .unwrap_or(false)
        {
            self.begin_font_library(ctx);
        }
        if let Some(scan) = &self.text_editor.fonts.scan
            && let Ok(catalog) = scan.receiver.try_recv()
        {
            self.accept_font_catalog(ctx, catalog);
        } else if self.text_editor.fonts.scan.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
        if !self.text_editor.fonts.open {
            return;
        }
        let p = &mut self.text_editor.fonts;
        let mut open = p.open;
        let mut apply = false;
        let mut cancel = false;
        let mut refresh = false;
        let mut restore = false;
        let mut insert = None;
        let mut use_font = None;
        let mut valid_character = false;
        let Some(settings) = &mut p.draft else {
            return;
        };
        egui::Window::new("Local font lists and characters").id(egui::Id::new("font-library")).default_pos(egui::pos2(300.,30.)).default_width(640.).default_height(620.).max_height(620.).open(&mut open).show(ctx,|ui|{
            if p.scan.is_some(){ui.label("Reading installed and app-local fonts…");}ui.horizontal(|ui|{if ui.button("Refresh local fonts").clicked(){refresh=true;}ui.label(format!("{} fonts; {} unreadable or unsupported files",p.catalog.entries.len(),p.catalog.failures));});
            let height=(ui.available_height()-76.).max(120.);egui::ScrollArea::vertical().id_salt("font-library-form").max_height(height).show(ui,|ui|{
                egui::ComboBox::from_id_salt("font-list-choice").selected_text(settings.active.and_then(|id|settings.lists.iter().find(|l|l.id==id)).map_or("All fonts",|l|l.name.as_str())).show_ui(ui,|ui|{ui.selectable_value(&mut settings.active,None,"All fonts");for l in &settings.lists{ui.selectable_value(&mut settings.active,Some(l.id),&l.name);}});
                ui.add(egui::TextEdit::singleline(&mut p.name).char_limit(64).hint_text("New / edited list name"));
                ui.horizontal_wrapped(|ui|{let r=ui.button("Create empty font list");p.create=Some(r.rect);if r.clicked()&&let Err(error)=settings.create(p.name.clone()){p.error=Some(error);}let active=settings.active;let r=ui.add_enabled(active.is_some(),egui::Button::new("Rename selected list"));p.rename=Some(r.rect);if r.clicked()&&let Some(id)=active&&let Err(error)=settings.rename(id,p.name.clone()){p.error=Some(error);}
if ui.add_enabled(active.is_some(),egui::Button::new("Duplicate selected list")).clicked()&&let Some(id)=active&&let Err(error)=settings.duplicate(id,p.name.clone()){p.error=Some(error);}});
                ui.horizontal(|ui|{ui.add(egui::TextEdit::singleline(&mut p.confirm).hint_text("Exact list name to confirm deletion"));if ui.add_enabled(settings.active.is_some(),egui::Button::new("Delete selected list")).clicked()&&let Some(id)=settings.active&&let Err(error)=settings.delete(id,&p.confirm){p.error=Some(error);}});
                if let Some(l)=settings.active.and_then(|id|settings.lists.iter_mut().find(|l|l.id==id)){ui.horizontal(|ui|{ui.selectable_value(&mut l.display,Display::Ui,"Names in UI font");ui.selectable_value(&mut l.display,Display::OwnFont,"Names in their own font");});if l.members.is_empty(){ui.label("This font list is empty.");}let missing:Vec<_>=l.members.iter().copied().filter(|f|!p.catalog.entries.iter().any(|e|e.font==*f)).collect();for font in missing{ui.horizontal(|ui|{ui.label(format!("Missing font retained: {}",font.label()));if ui.small_button("Remove reference from this list").clicked(){l.members.retain(|f|*f!=font);}});}}
                ui.horizontal(|ui|{ui.add(egui::TextEdit::singleline(&mut p.query).hint_text("Search local font names"));ui.checkbox(&mut p.favorites_only,"Favorites only");});
                ui.horizontal(|ui|{for(value,title)in[(true,"Include search results"),(false,"Exclude search results")]{if ui.add_enabled(settings.active.is_some(),egui::Button::new(title)).clicked()&&let Some(id)=settings.active{let filtered=Catalog{entries:p.catalog.entries.iter().filter(|e|!p.favorites_only||settings.favorites.contains(&e.font)).cloned().collect(),failures:0};if let Err(error)=settings.bulk(id,&filtered,&p.query,value){p.error=Some(error);}}}});
                let entries:Vec<_>=p.catalog.filtered(&p.query).into_iter().filter(|e|!p.favorites_only||settings.favorites.contains(&e.font)).collect();let display=settings.active.and_then(|id|settings.lists.iter().find(|l|l.id==id)).map_or(Display::Ui,|l|l.display);
                egui::ScrollArea::vertical().id_salt("local-font-rows").max_height(170.).show_rows(ui,22.,entries.len(),|ui,rows|{for n in rows{let entry=entries[n];ui.push_id(format!("{:?}",entry.font),|ui|{ui.horizontal(|ui|{let mut favorite=settings.favorites.contains(&entry.font);if ui.checkbox(&mut favorite,"★").changed(){settings.favorite(entry.font,favorite);}let mut member=settings.active.and_then(|id|settings.lists.iter().find(|l|l.id==id)).is_some_and(|l|l.members.contains(&entry.font));if ui.add_enabled(settings.active.is_some(),egui::Checkbox::new(&mut member,"Include")).changed()&&let Some(id)=settings.active{let _=settings.include(id,entry.font,member);}ui.label(label(ui.ctx(),entry.font,display));if ui.small_button("Characters").clicked(){p.character_font=entry.font;}
if ui.small_button("Use in text").clicked(){use_font=Some(entry.font);}});});}});
                let missing_favorites:Vec<_>=settings.favorites.iter().copied().filter(|f|!p.catalog.entries.iter().any(|e|e.font==*f)).collect();for font in missing_favorites{ui.horizontal(|ui|{ui.label(format!("Missing favorite: {}",font.label()));if ui.small_button("Remove missing favorite").clicked(){settings.favorite(font,false);}});}
                ui.separator();ui.label("Character map");picker(ui,"character-map-font",&mut p.character_font);ui.horizontal(|ui|{for (title,value)in[("Latin",0x20),("Combining",0x300),("Hangul",0xac00),("Symbols",0x2600),("Emoji",0x1f600)]{if ui.small_button(title).clicked(){p.block=value;}}});
                let family=preview(ctx,p.character_font);egui::Grid::new("local-character-map").show(ui,|ui|{for offset in 0..64{let code=p.block+offset;let text=char::from_u32(code).filter(|c|!c.is_control()).map_or("·".into(),|c|c.to_string());let mut label=egui::RichText::new(text);if let Some(f)=&family{label=label.font(egui::FontId::new(16.,f.clone()));}
if ui.small_button(label).clicked(){p.code=code;}
if offset%16==15{ui.end_row();}}});
                ui.horizontal(|ui|{ui.add(egui::DragValue::new(&mut p.code).range(0..=0x10ffff).hexadecimal(6,false,true).prefix("U+ "));if ui.button("Show this character block").clicked(){p.block=p.code/64*64;}});
                let char_result=fonts::character(p.character_font,p.code);if let Ok((c,supported))=&char_result{ui.label(format!("{c} — {}",if *supported{"Glyph is present in the selected font"}else{"Missing glyph: Unicode insertion is allowed; document fallback/marker is used"}));}else if let Err(error)=&char_result{ui.colored_label(egui::Color32::YELLOW,error);}
                valid_character=char_result.is_ok();
                if ui.button("Reset font preferences (pending)").clicked(){*settings=Settings::default();}
if ui.add_enabled(p.restore.is_some(),egui::Button::new("Restore previous saved font preferences")).clicked(){restore=true;}
                ui.small("Font files are read only. A missing or damaged name preview uses the UI font. Settings are pending until Apply; Cancel restores the previous lists. Font file installation/removal is a separate feature.");if let Some(error)=&p.error{ui.colored_label(egui::Color32::YELLOW,error);}
            });ui.separator();ui.horizontal(|ui|{let r=ui.button("Apply font preferences");p.apply=Some(r.rect);apply=r.clicked();let r=ui.button("Cancel font preferences");p.cancel=Some(r.rect);cancel=r.clicked();let r=ui.add_enabled(valid_character,egui::Button::new("Insert selected Unicode character"));p.insert=Some(r.rect);if r.clicked(){insert=Some((p.character_font,p.code));}});
        });
        if apply {
            let next = self.text_editor.fonts.draft.as_ref().unwrap().clone();
            if let Err(error) = self.commit_fonts(ctx, next) {
                self.text_editor.fonts.error = Some(error);
            } else {
                open = false;
            }
        }
        if restore
            && let Some(old) = self.text_editor.fonts.restore.take()
            && let Err(error) = self.commit_fonts(ctx, old)
        {
            self.text_editor.fonts.error = Some(error);
        }
        if refresh {
            self.refresh_font_catalog();
        }
        if let Some((font, code)) = insert
            && let Err(error) = self.insert_character(ctx, font, code)
        {
            self.text_editor.fonts.error = Some(error);
        }
        if let Some(font) = use_font {
            if self.text_editor.draft.is_none()
                && let Err(error) = self.begin_text_editor(ctx)
            {
                self.text_editor.fonts.error = Some(error);
            }
            if let Some(e) = &mut self.text_editor.draft {
                if e.composition.is_none() {
                    e.settings.style.font = font;
                    e.prepared = None;
                    e.image = None;
                } else {
                    self.text_editor.fonts.error =
                        Some("Finish or cancel IME composition before changing fonts".into());
                }
            }
        }
        self.text_editor.fonts.open = open && !cancel;
        if !self.text_editor.fonts.open {
            self.text_editor.fonts.draft = None;
        }
    }
}

#[path = "font_library_ui_verify.rs"]
mod verify;
pub(super) fn smoke(root: &std::path::Path) -> Result<(), String> {
    verify::smoke(root)
}
