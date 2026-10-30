use super::*;
use drawer_core::clip_edit::{self as core, Edit, Node, Preferences};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
};
type ImageKey = (
    u64,
    u64,
    u64,
    u32,
    drawer_core::timeline_playback::Preferences,
);
struct Held {
    document: u64,
    revision: u64,
    frame: u32,
    view: drawer_core::view::View,
}
struct Staged {
    prepared: core::Prepared,
    clipboard: Option<core::Clipboard>,
}
struct Job {
    document: u64,
    cancel: Arc<AtomicBool>,
    rx: mpsc::Receiver<Result<Box<Staged>, String>>,
    worker: Option<std::thread::JoinHandle<()>>,
}
impl Drop for Job {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
        if let Some(w) = self.worker.take() {
            let _ = w.join();
        }
    }
}
pub(super) struct Panel {
    pub open: bool,
    pub preferences: Preferences,
    held: Option<Held>,
    track: Option<u64>,
    to: Option<u64>,
    selected: BTreeSet<Node>,
    first: u32,
    end: u32,
    at: u32,
    name: String,
    replace: bool,
    review: bool,
    notice: String,
    clipboard: Option<core::Clipboard>,
    staged: Option<Box<Staged>>,
    job: Option<Job>,
    inventory: Option<(u64, u64, drawer_core::timeline_timing::Data)>,
    images: [Option<egui::TextureHandle>; 2],
    image_keys: [Option<ImageKey>; 2],
    pub controls: BTreeMap<&'static str, egui::Rect>,
}
impl Default for Panel {
    fn default() -> Self {
        Self {
            open: false,
            preferences: Default::default(),
            held: None,
            track: None,
            to: None,
            selected: BTreeSet::new(),
            first: 1,
            end: 1,
            at: 1,
            name: "Clip label / folder".into(),
            replace: false,
            review: false,
            notice: String::new(),
            clipboard: None,
            staged: None,
            job: None,
            inventory: None,
            images: [None, None],
            image_keys: [None, None],
            controls: BTreeMap::new(),
        }
    }
}
impl Drawer {
    pub(super) fn load_clip_preferences(&mut self, storage: &UserStorage) {
        match Preferences::load(storage.root()) {
            Ok(p) => self.clip_editor.preferences = p,
            Err(e) => self.clip_editor.notice = e,
        }
    }
    pub(super) fn begin_clip_editor(&mut self) -> Result<(), String> {
        if self.timeline_has_cel_binding() {
            return Err("Apply or return from cel content first".into());
        }
        self.begin_timeline_editor()?;
        let n = self.active.ok_or("Choose an animation")?;
        let d = &self.documents[n];
        let a = d.animation.as_ref().ok_or("Create a timeline first")?;
        self.clip_editor.open = true;
        self.clip_editor.end = a.frames;
        self.clip_editor.at = (d.current_frame + 1).min(a.frames);
        self.clip_editor.inventory = None;
        Ok(())
    }
    fn restore_clip_flip(&mut self) {
        if let Some(h) = self.clip_editor.held.take()
            && let Some(d) = self.documents.iter_mut().find(|d| d.id == h.document)
        {
            d.current_frame = h.frame.min(
                d.animation
                    .as_ref()
                    .map_or(0, |a| a.frames.saturating_sub(1)),
            );
            d.view = h.view;
        }
    }
    pub(super) fn clip_flip_active(&self) -> bool {
        self.clip_editor.held.is_some()
    }
    pub(super) fn clip_flip_input(&mut self, ctx: &egui::Context) {
        let focused = ctx.input(|i| i.focused);
        let down = ctx.input(|i| {
            i.modifiers.alt
                && (i.key_down(egui::Key::ArrowLeft) || i.key_down(egui::Key::ArrowRight))
        });
        if !focused
            || !down
            || !self.clip_editor.open
            || self.clip_editor.held.as_ref().is_some_and(|h| {
                self.active.is_none_or(|n| {
                    self.documents[n].id != h.document || self.documents[n].revision != h.revision
                })
            })
        {
            self.restore_clip_flip();
        }
        if !focused
            || !self.clip_editor.open
            || super::preferences::composing(ctx)
            || ctx.egui_wants_keyboard_input()
            || self.timeline_has_cel_binding()
            || self.transform_unconfirmed()
            || self.inference_unconfirmed()
            || self.text_editor.draft.is_some()
            || self.pending_close.is_some()
        {
            return;
        }
        let direction = ctx.input_mut(|i| {
            if i.consume_key(egui::Modifiers::ALT, egui::Key::ArrowLeft) {
                Some(drawer_core::timeline_editor::Jump::PreviousCel)
            } else if i.consume_key(egui::Modifiers::ALT, egui::Key::ArrowRight) {
                Some(drawer_core::timeline_editor::Jump::NextCel)
            } else {
                None
            }
        });
        let Some(jump) = direction else {
            return;
        };
        let Some(n) = self.active else {
            return;
        };
        let d = &self.documents[n];
        let track = self.clip_editor.track.or_else(|| {
            d.animation
                .as_ref()
                .and_then(|a| (!a.tracks.is_empty()).then(|| d.scene.catalog.tracks.id(0)))
        });
        let next = drawer_core::timeline_editor::navigate(d, track, jump);
        if let Ok(frame) = next {
            self.finish_stroke();
            self.stop_timeline_playback(self.documents[n].id);
            let d = &mut self.documents[n];
            self.clip_editor.held.get_or_insert(Held {
                document: d.id,
                revision: d.revision,
                frame: d.current_frame,
                view: d.view,
            });
            d.current_frame = frame;
        }
    }
    fn clip_work_budget(&self) -> usize {
        DEFAULT_DOCUMENT_BUDGET
            .saturating_sub(
                self.documents
                    .iter()
                    .map(Document::resident_bytes)
                    .sum::<usize>(),
            )
            .saturating_sub(
                self.clip_editor
                    .staged
                    .as_ref()
                    .map_or(0, |p| p.prepared.memory_bytes()),
            )
    }
    fn stage_clip(
        &mut self,
        ctx: &egui::Context,
        edit: Edit,
        clipboard: Option<core::Clipboard>,
    ) -> Result<(), String> {
        if self.clip_editor.job.is_some() {
            return Err("Wait for or cancel clip preparation".into());
        }
        if self.timeline_has_cel_binding()
            || self.transform_unconfirmed()
            || self.inference_unconfirmed()
            || self.text_editor.draft.is_some()
        {
            return Err("Finish pending content edits first".into());
        }
        self.restore_clip_flip();
        self.finish_stroke();
        let n = self.active.ok_or("Choose an animation")?;
        self.stop_timeline_playback(self.documents[n].id);
        let budget = self.clip_work_budget();
        let d = &self.documents[n];
        if d.memory_bytes().saturating_mul(3) > budget {
            return Err("Clip snapshot exceeds available memory".into());
        }
        let mut frozen = drawer_core::layer_merge::preview(&d.snapshot(), d.current_frame)?;
        frozen.id = d.id;
        frozen.revision = d.revision;
        frozen.keyframe_content = d.keyframe_content;
        if frozen.scene.timing.is_default() {
            frozen.scene.timing.labels_linked = self.clip_editor.preferences.default_linked;
        }
        let document = frozen.id;
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        let (tx, rx) = mpsc::channel();
        let ctx = ctx.clone();
        let worker = std::thread::spawn(move || {
            let result = core::prepare(&frozen, edit, budget, &flag).map(|prepared| {
                Box::new(Staged {
                    prepared,
                    clipboard,
                })
            });
            let _ = tx.send(result);
            ctx.request_repaint();
        });
        self.clip_editor.staged = None;
        self.clip_editor.job = Some(Job {
            document,
            cancel,
            rx,
            worker: Some(worker),
        });
        self.clip_editor.review = false;
        Ok(())
    }
    fn poll_clip(&mut self, ctx: &egui::Context) {
        if self
            .clip_editor
            .job
            .as_ref()
            .is_some_and(|j| !self.documents.iter().any(|d| d.id == j.document))
        {
            self.clip_editor.job = None;
        }
        if self
            .clip_editor
            .staged
            .as_ref()
            .is_some_and(|p| !self.documents.iter().any(|d| p.prepared.matches(d)))
        {
            self.clip_editor.staged = None;
            self.clip_editor.notice =
                "Prepared clip edit discarded after drawing change or close".into();
        }

        if self.active.is_none() {
            self.clip_editor.job = None;
            self.clip_editor.staged = None;
            self.clip_editor.images = [None, None];
            self.clip_editor.image_keys = [None, None];
            self.restore_clip_flip();
            return;
        }
        let ready = self
            .clip_editor
            .job
            .as_ref()
            .and_then(|j| j.rx.try_recv().ok());
        if let Some(result) = ready {
            let cancelled = self
                .clip_editor
                .job
                .as_ref()
                .is_some_and(|j| j.cancel.load(Ordering::Relaxed));
            self.clip_editor.job = None;
            if cancelled {
                self.clip_editor.notice = "Cancelled; original preserved".into();
            } else {
                match result {
                    Ok(p) if self.documents.iter().any(|d| p.prepared.matches(d)) => {
                        self.clip_editor.staged = Some(p)
                    }
                    Ok(_) => {
                        self.clip_editor.notice =
                            "Clip preparation became stale; original preserved".into()
                    }
                    Err(e) => self.clip_editor.notice = e,
                }
            }
        }
        if self.clip_editor.job.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(25));
        }
    }
    fn apply_clip(&mut self) -> Result<(), String> {
        if !self.clip_editor.review {
            return Err("Review prepared clip changes before applying".into());
        }
        let p = self
            .clip_editor
            .staged
            .take()
            .ok_or("Prepare a clip edit first")?;
        let n = self.active.ok_or("Choose a drawing")?;
        let summary = p.prepared.summary.clone();
        p.prepared.commit(&mut self.documents[n])?;
        if let Some(c) = p.clipboard {
            self.clip_editor.clipboard = Some(c);
        }
        self.clip_editor.notice = summary;
        self.clip_editor.inventory = None;
        self.clip_editor.images = [None, None];
        self.reset_timeline_view(self.documents[n].id);
        Ok(())
    }
    pub(super) fn clip_window(&mut self, ctx: &egui::Context) {
        self.poll_clip(ctx);
        if !self.clip_editor.open {
            return;
        }
        let Some(n) = self.active else {
            return;
        };
        let d = &self.documents[n];
        let Some(a) = &d.animation else {
            return;
        };
        let mut p = std::mem::take(&mut self.clip_editor);
        if p.inventory
            .as_ref()
            .is_none_or(|(id, rev, _)| *id != d.id || *rev != d.revision)
        {
            match core::inventory(d, self.clip_work_budget()) {
                Ok(t) => p.inventory = Some((d.id, d.revision, t)),
                Err(e) => p.notice = e,
            }
        }
        let Some((_, _, data)) = &p.inventory else {
            self.clip_editor = p;
            return;
        };
        let data = data.clone();
        p.track = p
            .track
            .filter(|t| d.scene.catalog.tracks.index(*t, a.tracks.len()).is_some())
            .or_else(|| (!a.tracks.is_empty()).then(|| d.scene.catalog.tracks.id(0)));
        p.to =
            p.to.filter(|t| {
                Some(*t) != p.track && d.scene.catalog.tracks.index(*t, a.tracks.len()).is_some()
            })
            .or_else(|| {
                (0..a.tracks.len())
                    .map(|n| d.scene.catalog.tracks.id(n))
                    .find(|t| Some(*t) != p.track)
            });
        p.selected
            .retain(|n| data.edit.range(*n, &data.clips).is_some());
        if p.selected.is_empty()
            && let Some(c) = data.clips.iter().find(|c| Some(c.track) == p.track)
        {
            p.selected.insert(Node::Clip(c.id));
        }
        let selected = p
            .selected
            .iter()
            .flat_map(|n| data.edit.leaves(*n, &data.clips).unwrap_or_default())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        for v in [&mut p.first, &mut p.end, &mut p.at] {
            *v = (*v).clamp(1, a.frames);
        }
        for f in &mut p.preferences.frames {
            *f = (*f).min(a.frames - 1);
        }
        let mut open = true;
        let mut edit = None;
        let mut action = 0;
        egui::Window::new("Clip editing and split timeline view").default_pos(egui::pos2(25.,20.)).default_width(730.).max_height(630.).vscroll(true).open(&mut open).show(ctx,|ui|{
   ui.label(&p.notice);let r=ui.button("Save local split view / defaults");p.controls.insert("options",r.rect);if r.clicked(){action=6;}
 ui.label("Hold Alt+Left / Alt+Right to flip cels. Release both keys, close this panel or lose focus to restore the original frame and view. Painting is disabled during the temporary flip.");
   if let Some(j)=&p.job{ui.spinner();if ui.button("Cancel clip preparation").clicked(){j.cancel.store(true,Ordering::Relaxed);}}
   if let Some(s)=&p.staged{ui.label(&s.prepared.summary);ui.collapsing("Prepared clip / range-label changes",|ui|{egui::ScrollArea::vertical().max_height(100.).show_rows(ui,20.,s.prepared.rows.len(),|ui,range|{for i in range{ui.add(egui::Label::new(&s.prepared.rows[i]).truncate()).on_hover_text(&s.prepared.rows[i]);}});});let r=ui.checkbox(&mut p.review,"I reviewed clip ranges, keys, labels and replacement policy");p.controls.insert("review",r.rect);let r=ui.add_enabled(p.review,egui::Button::new("Apply reviewed clip edit"));p.controls.insert("apply",r.rect);if r.clicked(){action=1;}
   if ui.button("Discard clip preview").clicked(){action=2;}}
   ui.horizontal_wrapped(|ui|{ui.label("Track:");for(n,t)in a.tracks.iter().enumerate(){ui.selectable_value(&mut p.track,Some(d.scene.catalog.tracks.id(n)),&t.name);}});
   egui::ScrollArea::vertical().id_salt("clip-selection").max_height(95.).show(ui,|ui|{for c in &data.clips{let node=Node::Clip(c.id);let mut on=p.selected.contains(&node);if ui.checkbox(&mut on,format!("Clip{} · track{} · frames{}–{}",c.id,c.track,c.first+1,c.end)).changed(){if on{p.selected.insert(node);}else{p.selected.remove(&node);}}}for f in &data.edit.folders{let node=Node::Folder(f.id);let mut on=p.selected.contains(&node);let range=data.edit.range(node,&data.clips);if ui.checkbox(&mut on,format!("Folder: {} · {:?}",f.name,range.map(|(a,b)|(a+1,b)))).changed(){if on{p.selected.insert(node);}else{p.selected.remove(&node);}}}});
   ui.horizontal(|ui|{ui.label("Range first / last, target or right-half first frame:");ui.add(egui::DragValue::new(&mut p.first).range(1..=a.frames));ui.add(egui::DragValue::new(&mut p.end).range(1..=a.frames));ui.add(egui::DragValue::new(&mut p.at).range(1..=a.frames));});
   let id=selected.first().copied();ui.horizontal_wrapped(|ui|{let r=ui.add_enabled(selected.len()==1,egui::Button::new("Prepare clip split"));p.controls.insert("split",r.rect);if r.clicked(){edit=id.map(|id|Edit::Split{id,at:p.at-1});}
 if ui.button("Prepare clip join").clicked(){edit=Some(Edit::Join(selected.clone()));}
 if ui.button("Prepare selected bounds").clicked(){edit=id.map(|id|Edit::Bounds{id,first:p.first-1,end:p.end});}
 if ui.button("Prepare automatic bounds").clicked(){edit=id.map(|id|Edit::AutoExpand{id,on:true});}
 if ui.button("Disable automatic bounds").clicked(){edit=id.map(|id|Edit::AutoExpand{id,on:false});}});
   ui.checkbox(&mut p.replace,"Replace destination clips / frames / overlapping keys (review before applying)");
   ui.horizontal_wrapped(|ui|{for(copy,label)in [(false,"Prepare clip move"),(true,"Prepare clip copy")]{if ui.button(label).clicked(){edit=Some(Edit::Move{ids:selected.clone(),at:p.at-1,copy,replace:p.replace});}}
 if ui.button("Prepare clip deletion").clicked(){edit=Some(Edit::Delete(selected.clone()));}
 if ui.button("Copy range locally").clicked(){action=3;}
 if ui.button("Prepare range cut").clicked(){action=4;}
 if ui.button("Prepare range paste").clicked(){action=5;}});
   ui.horizontal(|ui|{ui.text_edit_singleline(&mut p.name);if ui.button("Prepare range label").clicked(){edit=id.map(|clip|Edit::Label{clip,first:p.first-1,end:p.end,name:p.name.clone()});}
 if ui.button("Prepare clip folder").clicked(){edit=Some(Edit::Folder{name:p.name.clone(),children:p.selected.iter().copied().collect()});}});
   let mut linked=data.labels_linked;if ui.checkbox(&mut linked,"Link range labels to clip time edits (document)").changed(){edit=Some(Edit::LinkLabels(linked));}
   ui.collapsing("Track split / merge",|ui|{if ui.button("Prepare track split").clicked(){edit=p.track.map(|track|Edit::SplitTrack{track,at:p.at-1});}ui.horizontal_wrapped(|ui|{ui.label("Merge into:");for(n,t)in a.tracks.iter().enumerate(){let id=d.scene.catalog.tracks.id(n);if Some(id)!=p.track{ui.selectable_value(&mut p.to,Some(id),&t.name);}}});if ui.button("Prepare adjacent track merge").clicked(){edit=p.track.zip(p.to).map(|(from,to)|Edit::MergeTracks{from,to,replace:p.replace});}ui.label("Merge requires adjacent tracks with matching track appearance/keyframes. Overlap uses the explicit replacement policy. Track structure changes affect every stored arrangement.");});
   ui.separator();ui.checkbox(&mut p.preferences.split,"Show two timeline / frame panes");ui.checkbox(&mut p.preferences.default_linked,"Default label linking for new clip timing");
   if p.preferences.split{ui.columns(2,|cols|{for(i,ui)in cols.iter_mut().enumerate(){ui.label(if i==0{"Left timeline"}else{"Right timeline"});let mut f=p.preferences.frames[i]+1;ui.add(egui::Slider::new(&mut f,1..=a.frames).text("Frame"));p.preferences.frames[i]=f-1;let key=(d.id,d.revision,d.scene.timelines.active,f-1,self.timeline_manager.preferences);if p.image_keys[i]!=Some(key){let result=(||{if d.memory_bytes().saturating_mul(3)>self.clip_work_budget(){return Err("Split preview exceeds available memory".to_owned());}let mut frozen=drawer_core::layer_merge::preview(&d.snapshot(),f-1)?;frozen.keyframe_content=d.keyframe_content;let preview=drawer_core::timeline_playback::preview(&frozen,drawer_core::timeline_playback::View{track:None,only:false,onion:Default::default()},self.timeline_manager.preferences,self.clip_work_budget(),&AtomicBool::new(false))?;let bytes=preview.image.pixels.iter().flatten().copied().collect::<Vec<_>>();Ok(ctx.load_texture(format!("clip-pane-{i}"),egui::ColorImage::from_rgba_unmultiplied([preview.image.width as usize,preview.image.height as usize],&bytes),egui::TextureOptions::NEAREST))})();match result{Ok(image)=>{p.images[i]=Some(image);p.image_keys[i]=Some(key);},Err(e)=>{p.images[i]=None;p.notice=e;}}}
 if let Some(image)=&p.images[i]{ui.image((image.id(),image.size_vec2()*(250./image.size_vec2().x).min(180./image.size_vec2().y).min(1.)));}for c in data.clips.iter().filter(|c|c.first<f&&f-1<c.end){ui.label(format!("Track{} · clip{} · {}–{}",c.track,c.id,c.first+1,c.end));}}});}
   ui.collapsing("Saved range labels",|ui|{for l in &data.edit.labels{ui.label(format!("{} · {}–{}",l.name,l.first+1,l.end));}});
  });
        p.open = open;
        self.clip_editor = p;
        if !open {
            self.restore_clip_flip();
            self.clip_editor.job = None;
            self.clip_editor.staged = None;
            self.clip_editor.images = [None, None];
            self.clip_editor.image_keys = [None, None];
            return;
        }
        let result = (|| {
            match action {
                1 => self.apply_clip()?,
                2 => self.clip_editor.staged = None,
                3 | 4 => {
                    let n = self.active.ok_or("Choose an animation")?;
                    let p = &self.clip_editor;
                    let track = p.track.ok_or("Choose a track")?;
                    if self.documents[n].memory_bytes().saturating_mul(3) > self.clip_work_budget()
                    {
                        return Err("Clip clipboard exceeds available memory".into());
                    }
                    let copy = core::copy(&self.documents[n], track, p.first - 1, p.end)?;
                    if action == 3 {
                        self.clip_editor.clipboard = Some(copy);
                        self.clip_editor.notice =
                            "Copied clip range locally; document unchanged".into();
                    } else {
                        self.stage_clip(
                            ctx,
                            Edit::Cut {
                                track,
                                first: p.first - 1,
                                end: p.end,
                            },
                            Some(copy),
                        )?;
                    }
                }
                5 => {
                    let p = &self.clip_editor;
                    let clipboard = p
                        .clipboard
                        .clone()
                        .ok_or("Copy or cut a clip range first")?;
                    self.stage_clip(
                        ctx,
                        Edit::Paste {
                            clipboard,
                            at: p.at - 1,
                            replace: p.replace,
                        },
                        None,
                    )?;
                }
                6 => {
                    if let Some(s) = &self.storage {
                        self.clip_editor.preferences.save(s.root())?;
                    }
                    self.clip_editor.notice = "Local split view and defaults saved".into();
                }
                _ => {}
            }
            if let Some(e) = edit {
                self.stage_clip(ctx, e, None)?;
            }
            Ok::<_, String>(())
        })();
        if let Err(e) = result {
            self.clip_editor.notice = e;
        }
    }
}

#[path = "clip_edit_verify.rs"]
mod verify;
pub(super) fn smoke(root: &std::path::Path) -> Result<(), String> {
    verify::run(root)
}
#[cfg(test)]
mod tests {
    #[test]
    fn held_flip_restore_pointer_review_stale_cancel_split_panes_and_local_restart() {
        let t = crate::tests::TestDirectory::new();
        super::smoke(&t.0).unwrap();
    }
}
