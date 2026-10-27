use super::*;
use drawer_core::{
    timeline_batch::{Pattern, Selector},
    timeline_time_edit::{self as core, Choices, Prepared, Request, Scope},
    timeline_timing::{self as timing, Playback},
};
use std::{
    collections::BTreeSet,
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
};
pub(super) struct Panel {
    open: bool,
    section: u8,
    fps: [u32; 2],
    keep_time: bool,
    linked: bool,
    range: [u32; 2],
    repeat: bool,
    at: u32,
    count: u32,
    all: bool,
    globals: bool,
    keyed: std::collections::BTreeSet<drawer_core::keyframes::Owner>,
    inventory: Option<(u64, u64, timing::Data)>,
    split: bool,
    tracks: BTreeSet<u64>,
    audio: BTreeSet<u64>,
    pattern: Pattern,
    preset: String,
    prepared: Option<Prepared>,
    choices: Choices,
    recalculate: bool,
    reviewed: bool,
    stamp: Option<(u64, u64)>,
    job: Option<Job>,
    notice: String,
    pub apply: Option<egui::Rect>,
}
impl Default for Panel {
    fn default() -> Self {
        Self {
            open: false,
            section: 0,
            fps: [24, 1],
            keep_time: true,
            linked: true,
            range: [1, 1],
            repeat: true,
            at: 1,
            count: 1,
            all: true,
            globals: false,
            keyed: Default::default(),
            inventory: None,
            split: true,
            tracks: BTreeSet::new(),
            audio: BTreeSet::new(),
            pattern: Pattern::default(),
            preset: "Cel exposure rule".into(),
            prepared: None,
            choices: Choices::new(),
            recalculate: false,
            reviewed: false,
            stamp: None,
            job: None,
            notice: String::new(),
            apply: None,
        }
    }
}
struct Job {
    cancel: Arc<AtomicBool>,
    receiver: mpsc::Receiver<Result<Prepared, String>>,
}
impl Drop for Job {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}
impl Drawer {
    pub(super) fn begin_timeline_timing(&mut self) -> Result<(), String> {
        self.finish_stroke();
        if self.transform_unconfirmed()
            || self.inference_unconfirmed()
            || self.text_editor.draft.is_some()
        {
            return Err("Finish pending drawing edits first".into());
        }
        let d = &self.documents[self.active.ok_or("Choose an animation")?];
        let a = d.animation.as_ref().ok_or("Choose an animation document")?;
        if self.timeline_timing.job.is_some() {
            self.timeline_timing.open = true;
            return Ok(());
        }
        let p = &mut self.timeline_timing;
        p.fps = [a.fps.numerator, a.fps.denominator];
        p.linked = d.scene.timing.labels_linked;
        let bounds = d.scene.timing.playback.unwrap_or(Playback {
            first: 0,
            end: a.frames,
            repeat: true,
        });
        p.range = [bounds.first + 1, bounds.end];
        p.repeat = bounds.repeat;
        p.pattern.end = a.frames;
        p.pattern.track = d.scene.catalog.tracks.id(0);
        p.at = d.current_frame + 1;
        p.prepared = None;
        p.choices.clear();
        p.open = true;
        self.stop_timeline_playback(d.id);
        Ok(())
    }
    fn prepare_timing(
        &mut self,
        ctx: &egui::Context,
        request: Request,
        preserve_choices: bool,
    ) -> Result<(), String> {
        self.finish_stroke();
        if self.transform_unconfirmed()
            || self.inference_unconfirmed()
            || self.text_editor.draft.is_some()
        {
            return Err("Finish pending drawing edits first".into());
        }
        if self.timeline_timing.job.is_some() {
            return Err("Wait for or cancel the existing preparation".into());
        }
        let d = &self.documents[self.active.ok_or("Choose an animation")?];
        if preserve_choices && self.timeline_timing.stamp != Some((d.id, d.revision)) {
            return Err("Timeline changed; prepare the operation again".into());
        }
        let budget = DEFAULT_DOCUMENT_BUDGET.saturating_sub(
            self.documents
                .iter()
                .map(Document::resident_bytes)
                .sum::<usize>(),
        );
        if d.memory_bytes() > budget / 3 {
            return Err("Close some drawings or reduce the timing snapshot size".into());
        }
        let mut copy = drawer_core::layer_merge::preview(&d.snapshot(), d.current_frame)?;
        copy.id = d.id;
        copy.revision = d.revision;
        let stamp = (d.id, d.revision);
        self.stop_timeline_playback(d.id);
        let p = &mut self.timeline_timing;
        if !preserve_choices {
            p.choices.clear();
        }
        let choices = p.choices.clone();
        p.stamp = Some(stamp);
        p.prepared = None;
        p.reviewed = false;
        p.recalculate = false;
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        let (tx, receiver) = mpsc::channel();
        let repaint = ctx.clone();
        std::thread::spawn(move || {
            let result = (|| {
                if flag.load(Ordering::Relaxed) {
                    return Err("Cancelled timing preparation".into());
                }
                let p = core::prepare(&copy, request, &choices, budget)?;
                if flag.load(Ordering::Relaxed) {
                    return Err("Cancelled timing preparation".into());
                }
                Ok(p)
            })();
            let _ = tx.send(result);
            repaint.request_repaint();
        });
        p.job = Some(Job { cancel, receiver });
        Ok(())
    }
    fn apply_timing(&mut self) -> Result<(), String> {
        if !self.timeline_timing.reviewed || self.timeline_timing.recalculate {
            return Err("Review the conversion table and recalculate changed choices first".into());
        }
        let d = &mut self.documents[self.active.ok_or("Activate the reviewed animation")?];
        if self.timeline_timing.stamp != Some((d.id, d.revision)) {
            return Err("Activate the unchanged reviewed timeline or prepare again".into());
        }
        self.timeline_timing
            .prepared
            .as_ref()
            .ok_or("Prepare a timing change")?
            .commit(d)?;
        self.timeline_timing.prepared = None;
        self.timeline_timing.notice = "Applied the complete timing change as one Undo/Redo".into();
        Ok(())
    }
    fn save_timing_preset(&mut self) -> Result<(), String> {
        let old = self.workspaces.store.current.timeline_patterns.clone();
        self.workspaces.store.current.timeline_patterns.set(
            self.timeline_timing.preset.clone(),
            self.timeline_timing.pattern.clone(),
        )?;
        if let Err(e) = self.persist_workspace() {
            self.workspaces.store.current.timeline_patterns = old;
            return Err(e);
        }
        self.timeline_timing.notice =
            "Saved the exposure rule in local workspace settings; drawing unchanged".into();
        Ok(())
    }
    pub(super) fn timing_window(&mut self, ctx: &egui::Context) {
        if let Some(j) = &self.timeline_timing.job {
            let result = match j.receiver.try_recv() {
                Ok(r) => Some(r),
                Err(mpsc::TryRecvError::Empty) => None,
                Err(mpsc::TryRecvError::Disconnected) => {
                    Some(Err("Timing preparation stopped".into()))
                }
            };
            if let Some(result) = result {
                let cancel = j.cancel.load(Ordering::Relaxed);
                self.timeline_timing.job = None;
                if cancel {
                    self.timeline_timing.notice = "Cancelled; drawing unchanged".into();
                } else {
                    match result {
                        Ok(p) => self.timeline_timing.prepared = Some(p),
                        Err(e) => self.timeline_timing.notice = e,
                    }
                }
            } else {
                ctx.request_repaint_after(std::time::Duration::from_millis(25));
            }
        }
        if !self.timeline_timing.open {
            return;
        }
        let Some(n) = self
            .active
            .filter(|i| self.documents[*i].animation.is_some())
        else {
            self.timeline_timing.notice =
                "Activate an animation document to review/apply timing".into();
            return;
        };
        let d = &self.documents[n];
        let a = d.animation.as_ref().unwrap();
        if self
            .timeline_timing
            .inventory
            .as_ref()
            .map(|(id, revision, _)| (*id, *revision))
            != Some((d.id, d.revision))
        {
            self.timeline_timing.inventory = timing::inventory(d)
                .ok()
                .map(|data| (d.id, d.revision, data));
        }
        let inventory = self
            .timeline_timing
            .inventory
            .as_ref()
            .map(|(_, _, data)| data.clone());
        let mut p = std::mem::take(&mut self.timeline_timing);
        let mut open = true;
        let mut request = None;
        let mut action = 0;
        egui::Window::new("Timeline timing and exposure rules").default_pos(egui::pos2(15.,15.)).default_width(760.).open(&mut open).vscroll(true).show(ctx,|ui|{
            ui.label(&p.notice);ui.label(format!("{} · revision{} · {} frames · {}/{} FPS",d.title,d.revision,a.frames,a.fps.numerator,a.fps.denominator));
            if let Some(prepared)=&p.prepared {
                ui.separator();ui.label("Review every conversion/collision before applying. Original PCM and unused cel definitions are retained.");
                ui.checkbox(&mut p.reviewed,"I reviewed the listed changes and every required choice");
                let r=ui.add_enabled(p.reviewed&&prepared.ready()&&!p.recalculate,egui::Button::new("Apply reviewed timing change"));p.apply=Some(r.rect);if r.clicked(){action=2;}
                if ui.button("Discard timing preview").clicked(){action=3;}
                egui::ScrollArea::vertical().id_salt("timing-table").max_height(150.).show_rows(ui,20.,prepared.rows.len(),|ui,range|{for i in range{ui.add(egui::Label::new(&prepared.rows[i]).truncate()).on_hover_text(&prepared.rows[i]);}});
                if !prepared.issues.is_empty(){egui::ScrollArea::vertical().id_salt("timing-decisions").max_height(170.).show_rows(ui,28.,prepared.issues.len(),|ui,range|{for i in range{let issue=&prepared.issues[i];ui.horizontal(|ui|{ui.add_sized([340.,22.],egui::Label::new(&issue.message).truncate()).on_hover_text(&issue.message);let before=p.choices.get(&issue.id).copied();let mut selected=before;
                    egui::ComboBox::from_id_salt(("timing-choice",&issue.id)).selected_text(selected.and_then(|n|issue.options.get(n)).map_or("Choose…",String::as_str)).show_ui(ui,|ui|{for(n,label)in issue.options.iter().enumerate(){ui.selectable_value(&mut selected,Some(n),label);}});
                    if before!=selected {if let Some(choice)=selected{p.choices.insert(issue.id.clone(),choice);}p.recalculate=true;p.reviewed=false;}
                });}});}
                if p.recalculate&&ui.button("Recalculate table with my choices").clicked(){action=1;}

            }
            if let Some(j)=&p.job{ui.spinner();if ui.button("Cancel preparation").clicked(){j.cancel.store(true,Ordering::Relaxed);}}
            ui.add_enabled_ui(p.job.is_none(),|ui|{
                ui.horizontal(|ui|{ui.selectable_value(&mut p.section,0,"FPS / playback");ui.selectable_value(&mut p.section,1,"Insert / delete time");ui.selectable_value(&mut p.section,2,"Batch cel exposure / presets");});
                if p.section==0{
                    ui.horizontal(|ui|{ui.label("FPS numerator / denominator");ui.add(egui::DragValue::new(&mut p.fps[0]).range(1..=100000));ui.add(egui::DragValue::new(&mut p.fps[1]).range(1..=100000));});
                    ui.checkbox(&mut p.keep_time,"Preserve real time (off: preserve frame positions)");ui.checkbox(&mut p.linked,"Link labels to time edits");
                    if ui.button("Prepare FPS conversion table").clicked(){request=Some(Request::Fps{fps:drawer_core::animation::Fps{numerator:p.fps[0],denominator:p.fps[1]},keep_time:p.keep_time,labels_linked:p.linked});}
                    ui.horizontal(|ui|{ui.label("Playback first / last (inclusive)");ui.add(egui::DragValue::new(&mut p.range[0]).range(1..=a.frames));ui.add(egui::DragValue::new(&mut p.range[1]).range(1..=a.frames));ui.checkbox(&mut p.repeat,"Repeat range");});
                    if ui.button("Prepare saved playback range").clicked(){request=Some(Request::Playback(Playback{first:p.range[0]-1,end:p.range[1],repeat:p.repeat}));}
                }else if p.section==1{
                    ui.horizontal(|ui|{ui.label("At frame / count");ui.add(egui::DragValue::new(&mut p.at).range(1..=a.frames+1));ui.add(egui::DragValue::new(&mut p.count).range(1..=10000));});
                    ui.checkbox(&mut p.all,"All cel/audio tracks and global timing");ui.checkbox(&mut p.split,"Split clips crossing the boundary (off: retain/resize clip)");ui.checkbox(&mut p.linked,"Link labels");
                    if !p.all{
                        ui.checkbox(&mut p.globals,"Global output keys, linked labels and playback range");
                        for t in &d.scene.keyframes.tracks {if matches!(t.owner,drawer_core::keyframes::Owner::Layer(k)if !matches!(k,drawer_core::layer_search::Key::Track(_)|drawer_core::layer_search::Key::Cell{..})){let mut on=p.keyed.contains(&t.owner);if ui.checkbox(&mut on,format!("Layer keys: {:?}",t.owner)).changed(){if on{p.keyed.insert(t.owner);}else{p.keyed.remove(&t.owner);}}}}
                        for(i,t)in a.tracks.iter().enumerate(){let id=d.scene.catalog.tracks.id(i);let mut on=p.tracks.contains(&id);if ui.checkbox(&mut on,format!("Cel track{id}: {}",t.name)).changed(){if on{p.tracks.insert(id);}else{p.tracks.remove(&id);}}}
                        if let Some(data)=&inventory{for c in &data.audio{let mut on=p.audio.contains(&c.id);ui.horizontal(|ui|{if ui.checkbox(&mut on,format!("Audio{}: {} (original{})",c.id,a.audio[c.index].name,c.source)).changed(){if on{p.audio.insert(c.id);}else{p.audio.remove(&c.id);}}
if ui.button(if c.locked{"Prepare unlock"}else{"Prepare lock"}).clicked(){request=Some(Request::AudioLock{id:c.id,locked:!c.locked});}});}}
                    }
                    ui.horizontal(|ui|{for(remove,label)in [(false,"Prepare frame insertion"),(true,"Prepare frame deletion")]{if ui.button(label).clicked(){request=Some(Request::Splice{at:p.at-1,count:p.count,remove,scope:Scope{all:p.all,tracks:p.tracks.iter().copied().collect(),audio:p.audio.iter().copied().collect(),globals:p.globals,keyed:p.keyed.iter().copied().collect()},split:p.split,labels_linked:p.linked});}}});
                }else{
                    egui::ComboBox::from_label("Target cel track").selected_text(format!("Track{}",p.pattern.track)).show_ui(ui,|ui|{for(i,t)in a.tracks.iter().enumerate(){ui.selectable_value(&mut p.pattern.track,d.scene.catalog.tracks.id(i),&t.name);}});
                    ui.horizontal(|ui|{if ui.selectable_label(matches!(p.pattern.selector,Selector::Ids{..}),"Cel ID range").clicked(){p.pattern.selector=Selector::Ids{first:1,last:1};}
if ui.selectable_label(matches!(p.pattern.selector,Selector::Names{..}),"Name range (natural order)").clicked(){p.pattern.selector=Selector::Names{first:String::new(),last:String::new()};}});
                    ui.horizontal(|ui|{ui.label("First / last cel");match &mut p.pattern.selector{Selector::Ids{first,last}=>{ui.add(egui::DragValue::new(first).range(1..=u64::MAX));ui.add(egui::DragValue::new(last).range(1..=u64::MAX));},Selector::Names{first,last}=>{ui.text_edit_singleline(first);ui.text_edit_singleline(last);}}});
                    ui.horizontal(|ui|{let mut first=p.pattern.start+1;ui.label("First / last frame");ui.add(egui::DragValue::new(&mut first).range(1..=a.frames));ui.add(egui::DragValue::new(&mut p.pattern.end).range(1..=a.frames));p.pattern.start=first-1;});
                    ui.horizontal(|ui|{ui.label("Frames/cel / empty gap / ID-or-name step / repetitions (0 = to end)");ui.add(egui::DragValue::new(&mut p.pattern.hold).range(1..=10000));ui.add(egui::DragValue::new(&mut p.pattern.gap).range(0..=10000));ui.add(egui::DragValue::new(&mut p.pattern.step).range(1..=10000));ui.add(egui::DragValue::new(&mut p.pattern.repeats).range(0..=10000));});
                    ui.label("Missing numeric IDs are skipped. Duplicate name endpoints require ID selection. Output is clipped at the chosen last frame; the remaining timeline is preserved.");
                    if ui.button("Prepare batch exposure table").clicked(){request=Some(Request::Batch(p.pattern.clone()));}
                    ui.horizontal(|ui|{ui.text_edit_singleline(&mut p.preset);if ui.button("Save local rule").clicked(){action=4;}
if ui.button("Load selected rule").clicked(){action=5;}
if ui.button("Remove local rule").clicked(){action=6;}});
                    egui::ComboBox::from_label("Saved local rules").selected_text(&p.preset).show_ui(ui,|ui|{for name in self.workspaces.store.current.timeline_patterns.entries.keys(){ui.selectable_value(&mut p.preset,name.clone(),name);}});
                }
            });
        });
        p.open = open;
        if !open {
            p.prepared = None;
            if let Some(j) = &p.job {
                j.cancel.store(true, Ordering::Relaxed);
            }
        }
        self.timeline_timing = p;
        let result = (|| {
            match action {
                1 => {
                    let request = self
                        .timeline_timing
                        .prepared
                        .as_ref()
                        .ok_or("No conversion table")?
                        .request
                        .clone();
                    self.prepare_timing(ctx, request, true)?;
                }
                2 => self.apply_timing()?,
                3 => self.timeline_timing.prepared = None,
                4 => self.save_timing_preset()?,
                5 => {
                    let p = &mut self.timeline_timing;
                    let track = p.pattern.track;
                    p.pattern = self
                        .workspaces
                        .store
                        .current
                        .timeline_patterns
                        .entries
                        .get(&p.preset)
                        .ok_or("Select an existing rule")?
                        .clone();
                    p.pattern.track = track;
                }
                6 => {
                    let old = self.workspaces.store.current.timeline_patterns.clone();
                    self.workspaces
                        .store
                        .current
                        .timeline_patterns
                        .entries
                        .remove(&self.timeline_timing.preset);
                    if let Err(e) = self.persist_workspace() {
                        self.workspaces.store.current.timeline_patterns = old;
                        return Err(e);
                    }
                }
                _ => {}
            }
            if let Some(request) = request {
                self.prepare_timing(ctx, request, false)?;
            }
            Ok::<_, String>(())
        })();
        if let Err(e) = result {
            self.timeline_timing.notice = e;
        }
    }
}
pub(super) fn smoke(root: &Path) -> Result<(), String> {
    drawer_core::timeline_timing_verify::verify(root)?;
    let ctx = egui::Context::default();
    let mut app = Drawer::new(&ctx, None);
    app.simple_view = true;
    app.storage = Some(UserStorage::at(root.join("timing-settings")).map_err(|e| e.to_string())?);
    let result = (|| {
        app.add_document(drawer_core::timeline_timing_verify::fixture());
        app.execute_command(&ctx, "timeline-timing");
        if !app.timeline_timing.open {
            return Err("Timing command absent".into());
        }
        let mut time = 0.;
        let frame = |app: &mut Drawer, time: &mut f64, events| {
            *time += 0.05;
            input_replay::frame(&ctx, app, *time, events);
        };
        let wait = |app: &mut Drawer, time: &mut f64| -> Result<(), String> {
            let end = std::time::Instant::now() + std::time::Duration::from_secs(30);
            while app.timeline_timing.job.is_some() {
                frame(app, time, vec![]);
                if std::time::Instant::now() > end {
                    return Err("Timing worker timeout".into());
                }
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            if app.timeline_timing.prepared.is_none() {
                return Err(app.timeline_timing.notice.clone());
            }
            Ok(())
        };
        let click = |app: &mut Drawer, time: &mut f64, r: egui::Rect| {
            for pressed in [true, false] {
                frame(
                    app,
                    time,
                    vec![
                        egui::Event::PointerMoved(r.center()),
                        egui::Event::PointerButton {
                            pos: r.center(),
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: Default::default(),
                        },
                    ],
                );
            }
        };
        let original = app.documents[0].snapshot();
        let history = app.documents[0].history_counts();
        let request = Request::Fps {
            fps: drawer_core::animation::Fps {
                numerator: 12,
                denominator: 1,
            },
            keep_time: true,
            labels_linked: true,
        };
        app.prepare_timing(&ctx, request.clone(), false)?;
        wait(&mut app, &mut time)?;
        app.timeline_timing.reviewed = true;
        if app.apply_timing().is_ok() {
            return Err("Unresolved timing choices applied".into());
        }
        app.timeline_timing.choices = app
            .timeline_timing
            .prepared
            .as_ref()
            .unwrap()
            .issues
            .iter()
            .map(|i| (i.id.clone(), usize::from(i.id.starts_with("key-"))))
            .collect();
        app.prepare_timing(&ctx, request, true)?;
        wait(&mut app, &mut time)?;
        app.timeline_timing.reviewed = true;
        for _ in 0..3 {
            frame(&mut app, &mut time, vec![]);
        }
        let r = app
            .timeline_timing
            .apply
            .ok_or("Timing confirmation control absent")?;
        click(&mut app, &mut time, r);
        if app.documents[0].animation.as_ref().unwrap().fps.numerator != 12
            || app.documents[0].history_counts().0 != history.0 + 1
        {
            return Err(format!(
                "Timing pointer apply failed at {r:?}, ready={}, reviewed={}, recalc={}: {}",
                app.timeline_timing
                    .prepared
                    .as_ref()
                    .is_some_and(Prepared::ready),
                app.timeline_timing.reviewed,
                app.timeline_timing.recalculate,
                app.timeline_timing.notice
            ));
        }
        if !app.documents[0].undo() || app.documents[0].snapshot() != original {
            return Err("Timing UI Undo did not restore the full document".into());
        }
        let range = Request::Playback(Playback {
            first: 3,
            end: 7,
            repeat: true,
        });
        app.prepare_timing(&ctx, range.clone(), false)?;
        wait(&mut app, &mut time)?;
        app.documents[0].edit_pixel(0, 0, 0, [80, 40, 0, 255]);
        app.timeline_timing.reviewed = true;
        if app.apply_timing().is_ok() {
            return Err("Stale timing preview accepted".into());
        }
        app.documents[0].undo();
        app.prepare_timing(&ctx, range, false)?;
        wait(&mut app, &mut time)?;
        app.timeline_timing.reviewed = true;
        app.apply_timing()?;
        let after = app.documents[0].snapshot();
        let h = app.documents[0].history_counts();
        app.timeline_timing.open = false;
        app.begin_timeline_editor()?;
        for _ in 0..3 {
            frame(&mut app, &mut time, vec![]);
        }
        let r = app.timeline_editor.play.ok_or("Playback control absent")?;
        click(&mut app, &mut time, r);
        for _ in 0..20 {
            frame(&mut app, &mut time, vec![]);
            if !(3..7).contains(&app.documents[0].current_frame) {
                return Err("Production playback escaped the saved range".into());
            }
        }
        if app.documents[0].snapshot() != after || app.documents[0].history_counts() != h {
            return Err("Playback modified document data/history".into());
        }
        app.stop_timeline_playback(app.documents[0].id);
        app.timeline_timing.pattern = Pattern {
            track: app.documents[0].scene.catalog.tracks.id(0),
            end: 12,
            ..Default::default()
        };
        app.save_timing_preset()?;
        let loaded = drawer_core::workspaces::Store::load(app.storage.as_ref().unwrap())
            .map_err(|e| e.to_string())?
            .ok_or("Saved timing preset missing")?;
        if loaded.current.timeline_patterns != app.workspaces.store.current.timeline_patterns
            || app.documents[0].snapshot() != after
        {
            return Err("Local timing preset restart altered drawing or lost rule".into());
        }
        app.prepare_timing(
            &ctx,
            Request::Playback(Playback {
                first: 0,
                end: 1,
                repeat: false,
            }),
            false,
        )?;
        app.timeline_timing
            .job
            .as_ref()
            .unwrap()
            .cancel
            .store(true, Ordering::Relaxed);
        while app.timeline_timing.job.is_some() {
            frame(&mut app, &mut time, vec![]);
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        if app.timeline_timing.prepared.is_some() || app.documents[0].snapshot() != after {
            return Err("Cancelled timing preparation changed drawing".into());
        }
        Ok(())
    })();
    drop(app);
    let cleanup = std::fs::remove_dir_all(root.join("timing-settings")).map_err(|e| e.to_string());
    result.and(cleanup)
}
#[cfg(test)]
mod tests {
    #[test]
    fn pointer_conversion_choices_range_playback_presets_stale_cancel_and_undo() {
        let root = std::env::temp_dir().join(format!("drawer-timing-ui-{}", std::process::id()));
        std::fs::create_dir(&root).unwrap();
        let r = super::smoke(&root);
        std::fs::remove_dir(&root).unwrap();
        r.unwrap();
    }
}
