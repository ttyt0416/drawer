use super::*;
use drawer_core::story_text as story;
use sha2::{Digest, Sha256};
#[derive(Default)]
pub(super) struct Panel {
    pub open: bool,
    editing: bool,
    rows: Vec<drawer_core::comic_dialogue::Row>,
    insertion: String,
    work: Option<story::Work>,
    title: String,
    whole: bool,
    find: String,
    replace: String,
    sensitive: bool,
    prepared: Option<Box<Prepared>>,
    error: Option<String>,
    transactions: Vec<story::Transaction>,
    pub prepare: Option<egui::Rect>,
    pub apply: Option<egui::Rect>,
    pub cancel: Option<egui::Rect>,
}
struct Source {
    document: u64,
    revision: u64,
    frame: u32,
    path: Option<PathBuf>,
}
struct Prepared {
    plan: story::Prepared,
    closed: Vec<Document>,
    stamps: Vec<(PathBuf, [u8; 32])>,
    sources: Vec<Source>,
    whole: bool,
    ack: bool,
}
fn stamp(path: &std::path::Path) -> Result<[u8; 32], String> {
    use std::io::Read;
    let mut f = std::fs::File::open(path).map_err(|e| e.to_string())?;
    if f.metadata().map_err(|e| e.to_string())?.len() > 1024 * 1024 * 1024 {
        return Err("Work page file limit1GiB".into());
    }
    let mut hash = Sha256::new();
    let mut bytes = [0; 65536];
    loop {
        let n = f.read(&mut bytes).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        hash.update(&bytes[..n]);
    }
    Ok(hash.finalize().into())
}
impl Drawer {
    pub(crate) fn begin_story_text(&mut self) -> Result<(), String> {
        self.finish_stroke();
        if self.text_editor.draft.is_some()
            || self.text_editor.version.open()
            || self.transform_unconfirmed()
            || self.inference_unconfirmed()
        {
            return Err("Apply or cancel pending edits before work replacement".into());
        }
        if self.active.is_none() {
            return Err("Open a page first".into());
        }
        self.text_editor.story.open = true;
        self.text_editor.story.prepared = None;
        Ok(())
    }
    fn work_from_open_pages(&self, title: String) -> Result<story::Work, String> {
        let paths = self
            .documents
            .iter()
            .map(|d| {
                d.path
                    .clone()
                    .ok_or("Save every open page as .drawer before defining a work")
            })
            .collect::<Result<Vec<_>, _>>()?;
        story::Work::new(title, paths)
    }
    fn prepare_story_text(&mut self, p: &mut Panel) -> Result<(), String> {
        p.prepared = None;
        if self.text_editor.draft.is_some()
            || self.transform_unconfirmed()
            || self.inference_unconfirmed()
        {
            return Err("Apply or cancel pending edits first".into());
        }
        let query = story::Query {
            find: p.find.clone(),
            replace: p.replace.clone(),
            case_sensitive: p.sensitive,
        };
        let mut closed = vec![];
        let mut stamps = vec![];
        let mut ids = vec![];
        let used: usize = self.documents.iter().map(Document::resident_bytes).sum();
        let mut selected_memory = 0usize;
        if p.whole {
            let work = p
                .work
                .as_ref()
                .ok_or("Open or create a local work manifest")?;
            let mut canonical_pages = std::collections::BTreeSet::new();
            for path in &work.pages {
                let path = path
                    .canonicalize()
                    .map_err(|e| format!("Work page {}: {e}", path.display()))?;
                if !drawer_core::font_library::local(&path) {
                    return Err("Work pages must be local disk files".into());
                }
                if !canonical_pages.insert(path.to_string_lossy().to_lowercase()) {
                    return Err("Two work entries resolve to the same page".into());
                }

                if let Some(d) = self
                    .documents
                    .iter()
                    .find(|d| d.path.as_ref() == Some(&path))
                {
                    ids.push(d.id);
                    selected_memory = selected_memory.saturating_add(d.resident_bytes());
                } else {
                    let fingerprint = stamp(&path)?;
                    let remaining = DEFAULT_DOCUMENT_BUDGET
                        .saturating_sub(used + selected_memory.saturating_mul(8));
                    let d =
                        drawer_core::files::open(&path, remaining).map_err(|e| e.to_string())?;
                    if stamp(&path)? != fingerprint {
                        return Err("A closed work page changed while loading".into());
                    }
                    selected_memory = selected_memory.saturating_add(d.resident_bytes());
                    ids.push(d.id);
                    closed.push(d);
                    stamps.push((path, fingerprint));
                }
                if used.saturating_add(selected_memory.saturating_mul(8)) > DEFAULT_DOCUMENT_BUDGET
                {
                    return Err("Work preview workspace memory limit".into());
                }
            }
        } else {
            let d = &self.documents[self.active.ok_or("Open a page")?];
            ids.push(d.id);
            selected_memory = d.resident_bytes();
        }
        if used.saturating_add(selected_memory.saturating_mul(8)) > DEFAULT_DOCUMENT_BUDGET {
            return Err("Work preview workspace memory limit".into());
        }
        let docs: Vec<_> = ids
            .iter()
            .map(|id| {
                self.documents
                    .iter()
                    .chain(&closed)
                    .find(|d| d.id == *id)
                    .unwrap()
            })
            .collect();
        let sources = docs
            .iter()
            .map(|d| Source {
                document: d.id,
                revision: d.revision,
                frame: d.current_frame,
                path: d.path.clone(),
            })
            .collect();
        let plan = if p.editing {
            if p.rows.is_empty() {
                p.rows = drawer_core::comic_dialogue::rows(&docs);
                drawer_core::comic_dialogue::prepare(&docs, &[])?
            } else {
                drawer_core::comic_dialogue::prepare(&docs, &p.rows)?
            }
        } else {
            story::prepare(&docs, &query)?
        };
        p.prepared = Some(Box::new(Prepared {
            plan,
            closed,
            stamps,
            sources,
            whole: p.whole,
            ack: false,
        }));
        Ok(())
    }
    fn apply_story_text(&mut self, p: &mut Panel) -> Result<(), String> {
        if self.text_editor.draft.is_some() {
            return Err("Apply or cancel the text editor first".into());
        }
        if p.transactions.len() >= 4096 {
            return Err(
                "Linked work history limit; save and reopen pages to reset session history".into(),
            );
        }
        let prepared = p.prepared.as_ref().ok_or("Prepare the work preview")?;
        if !prepared.ack
            || prepared.whole != p.whole
            || prepared.plan.query
                != (story::Query {
                    find: p.find.clone(),
                    replace: p.replace.clone(),
                    case_sensitive: p.sensitive,
                })
        {
            return Err("Review a fresh replacement preview".into());
        }
        if !p.whole
            && self
                .active
                .is_none_or(|n| self.documents[n].id != prepared.sources[0].document)
        {
            return Err("The current page changed".into());
        }
        for source in &prepared.sources {
            let d = self
                .documents
                .iter()
                .chain(&prepared.closed)
                .find(|d| d.id == source.document)
                .ok_or("A work page closed")?;
            if d.revision != source.revision
                || d.current_frame != source.frame
                || (p.whole && d.path != source.path)
            {
                return Err("A work page changed; prepare again".into());
            }
        }
        for (path, digest) in &prepared.stamps {
            if stamp(path)? != *digest {
                return Err("A closed work page changed on disk; prepare again".into());
            }
        }
        let ids: Vec<_> = prepared.plan.pages.iter().map(|p| p.id).collect();
        let used: usize = self.documents.iter().map(Document::resident_bytes).sum();
        let extra: usize = prepared
            .plan
            .pages
            .iter()
            .map(|p| p.after.resident_bytes().saturating_mul(4))
            .sum();
        if used.saturating_add(extra) > DEFAULT_DOCUMENT_BUDGET {
            return Err("Work Apply workspace memory limit".into());
        }
        let mut working: Vec<_> = ids
            .iter()
            .map(|id| {
                self.documents
                    .iter()
                    .chain(&prepared.closed)
                    .find(|d| d.id == *id)
                    .unwrap()
                    .clone()
            })
            .collect();
        let prepared = p.prepared.take().unwrap();
        let transaction = prepared.plan.apply(&mut working)?;
        let active = self.active.map(|n| self.documents[n].id);
        for d in working {
            if let Some(n) = self.documents.iter().position(|old| old.id == d.id) {
                self.documents[n] = d;
            } else {
                self.add_document(d);
            }
        }
        if let Some(id) = active
            && let Some(tab) = self.tabs.iter().position(|t| t.document_id == id)
        {
            self.select_tab(tab);
        }
        p.transactions.push(transaction);
        p.open = false;
        self.status=Some("Work text replacement applied. Undo/Redo from any affected page restores the entire operation; source files are saved only when you save the pages.".into());
        Ok(())
    }
    pub(crate) fn story_text_history(&mut self, redo: bool) -> Result<bool, String> {
        let Some(n) = self.active else {
            return Ok(false);
        };
        let d = &self.documents[n];
        let Some(group) = d.history_group(redo) else {
            return Ok(false);
        };
        let Some(transaction) = self
            .text_editor
            .story
            .transactions
            .iter()
            .find(|t| t.id == group && t.contains(d.id))
        else {
            return Ok(false);
        };
        transaction.step(&mut self.documents, redo)?;
        Ok(true)
    }
    pub(crate) fn story_text_window(&mut self, ctx: &egui::Context) {
        if !self.text_editor.story.open {
            return;
        }
        let old_preferences = self.workspaces.store.current.comic_view.clone();
        let mut p = std::mem::take(&mut *self.text_editor.story);
        let mut open = true;
        let (mut prepare, mut apply, mut cancel) = (false, false, false);
        let old = (p.find.clone(), p.replace.clone(), p.sensitive, p.whole);
        egui::Window::new("Story dialogue editor").default_width(650.).open(&mut open).show(ctx,|ui|{
   ui.label("Current page or every page registered in a local work manifest. Closed work pages are previewed without modifying disk files; Apply opens changed pages for saving.");
   ui.horizontal(|ui|{ui.radio_value(&mut p.whole,false,"Current page");ui.add_enabled_ui(p.work.is_some(),|ui|{ui.radio_value(&mut p.whole,true,"Whole local work");});});
   ui.add(egui::TextEdit::singleline(&mut p.title).char_limit(64).hint_text("Local work name"));
   ui.horizontal_wrapped(|ui|{
    if ui.button("Use saved open pages as this work").clicked(){match self.work_from_open_pages(p.title.clone()){Ok(work)=>{p.work=Some(work);p.prepared=None;},Err(error)=>p.error=Some(error)}}
    if ui.button("Open local work list…").clicked()&&let Some(file)=rfd::FileDialog::new().add_filter("Drawer work",&["drawer-work"]).pick_file(){match story::Work::load(&file){Ok(work)=>{p.title=work.title.clone();p.work=Some(work);p.prepared=None;},Err(error)=>p.error=Some(error)}}
    if ui.add_enabled(p.work.is_some(),egui::Button::new("Save work list…")).clicked()&&let Some(file)=rfd::FileDialog::new().add_filter("Drawer work",&["drawer-work"]).save_file()&&let Err(error)=p.work.as_ref().unwrap().save(&file){p.error=Some(error);}
   });
   if let Some(work)=&p.work{ui.label(format!("{} · {} registered pages",work.title,work.pages.len()));}
   ui.horizontal(|ui|{if ui.radio_value(&mut p.editing,false,"Find and replace").changed()||ui.radio_value(&mut p.editing,true,"Edit ordered dialogues").changed(){p.prepared=None;p.rows.clear();}});
   if p.editing {
    let prefs=&mut self.workspaces.store.current.comic_view;
    ui.horizontal(|ui|{ui.checkbox(&mut prefs.show_breaks,"Show newline marks");ui.checkbox(&mut prefs.vertical_list,"Vertical list view");});
    egui::ScrollArea::vertical().id_salt("dialogue-rows").max_height(250.).show(ui,|ui|{for row in &mut p.rows{ui.push_id((row.page,row.id),|ui|{ui.label(format!("Page {} · text#{}",row.page+1,row.id));let before=row.after.clone();ui.add(egui::TextEdit::multiline(&mut row.after.text).char_limit(4096).desired_rows(2));if prefs.show_breaks{ui.label(row.after.text.replace('\n',"↵\n"));}
if prefs.vertical_list{ui.horizontal(|ui|{for line in row.after.text.lines().take(16).collect::<Vec<_>>().into_iter().rev(){ui.vertical(|ui|{for ch in line.chars().take(128){ui.label(ch.to_string());}});}});}ui.add(egui::DragValue::new(&mut row.after.style.line_height).range(0.5..=8.).prefix("Paragraph spacing × "));ui.horizontal(|ui|{ui.radio_value(&mut row.after.style.flow,core::Flow::Horizontal,"Horizontal writing");ui.radio_value(&mut row.after.style.flow,core::Flow::Vertical,"Vertical writing");});if before.text!=row.after.text{row.after.style.remap_text(&before.text,&row.after.text);}
if before!=row.after{p.prepared=None;}});}});
    ui.separator();ui.label("Insert dialogue on the current page");ui.add(egui::TextEdit::multiline(&mut p.insertion).char_limit(4096).desired_rows(2));ui.checkbox(&mut prefs.append,"Append to current text layer");ui.checkbox(&mut prefs.inherit,"Inherit current text format (off: text tool defaults)");
    if ui.button("Reload dialogues (discard list edits)").clicked(){p.rows.clear();p.prepared=None;}
    if ui.button("Insert dialogue as one Undo").clicked(){let result=(||->Result<(),String>{let n=self.active.ok_or("Open a page")?;let d=&self.documents[n];let source=self.panels.targets.get(&d.id).and_then(|t|if let drawer_core::thumbnail::Target::Source(drawer_core::layer_search::Key::Object(id))=t{Some(*id)}else{None});let edit=drawer_core::comic_dialogue::insert(d,source,&self.workspaces.store.current.text_tool.defaults,&p.insertion,if self.workspaces.store.current.comic_view.append{drawer_core::comic_dialogue::Insertion::Current}else{drawer_core::comic_dialogue::Insertion::New},self.workspaces.store.current.comic_view.inherit)?;let id=edit.id;self.documents[n].commit_state(edit.state)?;let i=self.documents[n].scene.catalog.objects.index(id,self.documents[n].scene.objects.len()).ok_or("Inserted text missing")?;self.panels.targets.insert(self.documents[n].id,drawer_core::thumbnail::Target::Source(drawer_core::layer_search::Key::Object(self.documents[n].scene.catalog.objects.id(i))));p.insertion.clear();p.rows.clear();p.prepared=None;Ok(())})();p.error=result.err();}
   } else {
   ui.label("Find literal Unicode text");ui.add(egui::TextEdit::multiline(&mut p.find).char_limit(256).desired_rows(2));ui.label("Replace with");ui.add(egui::TextEdit::multiline(&mut p.replace).char_limit(4096).desired_rows(2));ui.checkbox(&mut p.sensitive,"Case sensitive");
   }
   if old!=(p.find.clone(),p.replace.clone(),p.sensitive,p.whole){p.prepared=None;if old.3!=p.whole{p.rows.clear();}}
   if let Some(prepared)=&mut p.prepared{ui.label(format!("{} match(es), {} changed page(s)",prepared.plan.matches.len(),prepared.plan.pages.len()));egui::ScrollArea::vertical().max_height(180.).show(ui,|ui|{for m in &prepared.plan.matches{ui.label(format!("{} · text#{} · {}..{}: {:?} → {:?}",m.page,m.object,m.first,m.last,m.before.chars().take(100).collect::<String>(),m.after.chars().take(100).collect::<String>()));}});ui.checkbox(&mut prepared.ack,"I reviewed all listed replacements; apply as one linked Undo");}
   if let Some(error)=&p.error{ui.colored_label(egui::Color32::YELLOW,error);}
   ui.horizontal(|ui|{let r=ui.add_enabled(p.editing||!p.find.is_empty(),egui::Button::new(if p.editing{if p.rows.is_empty(){"Load ordered dialogues"}else{"Prepare dialogue edits"}}else{"Prepare replacement list"}));prepare=r.clicked();p.prepare=Some(r.rect);let r=ui.add_enabled(p.prepared.as_ref().is_some_and(|x|x.ack&&!x.plan.pages.is_empty()),egui::Button::new("Apply work replacement"));apply=r.clicked();p.apply=Some(r.rect);let r=ui.button("Cancel work change");cancel=r.clicked();p.cancel=Some(r.rect);});
  });
        if cancel || !open || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            p.open = false;
            p.prepared = None;
            p.rows.clear();
            p.insertion.clear();
        } else if prepare {
            p.error = self.prepare_story_text(&mut p).err();
        } else if apply {
            p.error = self.apply_story_text(&mut p).err();
        }
        if old_preferences != self.workspaces.store.current.comic_view
            && let Err(error) = self.persist_workspace()
        {
            p.error = Some(error);
        }
        *self.text_editor.story = p;
    }
}
pub(super) fn smoke(root: &std::path::Path) -> Result<(), String> {
    verify::smoke(root)
}
#[path = "story_text_ui_verify.rs"]
mod verify;
