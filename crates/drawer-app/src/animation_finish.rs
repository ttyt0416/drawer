use super::*;
use drawer_core::{
    animation_finish::{self as core, Edit, Preferences, Settings, View},
    animation_render::Area,
};
use std::{
    collections::BTreeMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
};
#[derive(Default)]
pub(super) struct Paths {
    stamp: Option<(u64, u64, u32, View)>,
    guides: core::Guides,
}
impl Paths {
    pub fn paint(
        &mut self,
        painter: &egui::Painter,
        d: &Document,
        view: drawer_core::view::View,
        rect: egui::Rect,
        ppp: f64,
        flags: View,
    ) -> Result<(), String> {
        let stamp = (d.id, d.revision, d.current_frame, flags);
        if self.stamp != Some(stamp) {
            self.guides = core::guides(d, flags, DEFAULT_DOCUMENT_BUDGET)?;
            self.stamp = Some(stamp);
        }
        let screen = |p| {
            let p = view.document_to_screen(
                p,
                [d.width as f64, d.height as f64],
                [rect.center().x as f64, rect.center().y as f64],
                ppp,
            );
            egui::pos2(p[0] as f32, p[1] as f32)
        };
        for (points, color, label) in [
            (
                self.guides.output,
                egui::Color32::LIGHT_GRAY,
                "Output frame",
            ),
            (self.guides.safe, egui::Color32::LIGHT_BLUE, "Title safe"),
            (
                self.guides.overflow,
                egui::Color32::from_rgb(240, 170, 60),
                "Overflow",
            ),
        ] {
            if let Some(points) = points {
                let p = points.map(screen);
                painter.add(egui::Shape::closed_line(
                    p.to_vec(),
                    egui::Stroke::new(1., color),
                ));
                painter.text(
                    p[0],
                    egui::Align2::LEFT_BOTTOM,
                    label,
                    egui::FontId::proportional(11.),
                    color,
                );
            }
        }
        if self.guides.outside {
            painter.text(
                rect.left_top() + egui::vec2(8., 8.),
                egui::Align2::LEFT_TOP,
                "Frame excluded from default output range",
                egui::FontId::proportional(12.),
                egui::Color32::YELLOW,
            );
        }
        Ok(())
    }
}
struct Job {
    document: u64,
    cancel: Arc<AtomicBool>,
    rx: mpsc::Receiver<Result<Box<core::Prepared>, String>>,
    worker: Option<std::thread::JoinHandle<()>>,
}
impl Drop for Job {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
pub(super) struct Panel {
    pub open: bool,
    pub preferences: Preferences,
    stamp: Option<(u64, u64)>,
    first: u32,
    last: u32,
    safe: bool,
    percent: [u8; 2],
    area: Area,
    update_frame: bool,
    bounds: [f64; 4],
    audio: Option<u64>,
    audio_stamp: Option<(u64, u64, u64)>,
    audio_start: u32,
    gain: u16,
    mute: bool,
    cels: bool,
    shots: bool,
    job: Option<Job>,
    staged: Option<Box<core::Prepared>>,
    review: bool,
    notice: String,
    pub controls: BTreeMap<&'static str, egui::Rect>,
}
impl Default for Panel {
    fn default() -> Self {
        Self {
            open: false,
            preferences: Default::default(),
            stamp: None,
            first: 1,
            last: 1,
            safe: false,
            percent: [90, 90],
            area: Area::Canvas,
            update_frame: false,
            bounds: [0., 0., 1., 1.],
            audio: None,
            audio_stamp: None,
            audio_start: 1,
            gain: 100,
            mute: false,
            cels: true,
            shots: false,
            job: None,
            staged: None,
            review: false,
            notice: String::new(),
            controls: BTreeMap::new(),
        }
    }
}
impl Drawer {
    pub(super) fn load_finish_preferences(&mut self, storage: &UserStorage) {
        match Preferences::load(storage.root()) {
            Ok(p) => self.animation_finish.preferences = p,
            Err(e) => self.animation_finish.notice = e,
        }
    }
    pub(super) fn begin_animation_finish(&mut self) -> Result<(), String> {
        self.finish_stroke();
        if self.timeline_has_cel_binding() {
            return Err("Apply or return from cel content first".into());
        }
        let n = self.active.ok_or("Choose an animation")?;
        self.documents[n]
            .animation
            .as_ref()
            .ok_or("Create a timeline first")?;
        self.animation_finish.open = true;
        self.animation_finish.stamp = None;
        Ok(())
    }
    fn finish_budget(&self) -> usize {
        DEFAULT_DOCUMENT_BUDGET.saturating_sub(
            self.documents
                .iter()
                .map(Document::resident_bytes)
                .sum::<usize>(),
        )
    }
    fn stage_animation_finish(&mut self, ctx: &egui::Context, edit: Edit) -> Result<(), String> {
        if self.animation_finish.job.is_some() {
            return Err("Wait for or cancel current preparation".into());
        }
        self.finish_stroke();
        if self.timeline_has_cel_binding()
            || self.transform_unconfirmed()
            || self.inference_unconfirmed()
            || self.text_editor.draft.is_some()
        {
            return Err("Finish pending content edits first".into());
        }
        let n = self.active.ok_or("Choose an animation")?;
        self.stop_timeline_playback(self.documents[n].id);
        let budget = self.finish_budget();
        let d = &self.documents[n];
        if d.memory_bytes().saturating_mul(3) > budget {
            return Err("Finishing snapshot exceeds available memory".into());
        }
        let document = d.id;
        let mut frozen = drawer_core::layer_merge::preview(&d.snapshot(), d.current_frame)?;
        frozen.id = d.id;
        frozen.revision = d.revision;
        frozen.keyframe_content = d.keyframe_content;
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        let (tx, rx) = mpsc::channel();
        let ctx = ctx.clone();
        let worker = std::thread::spawn(move || {
            let result = core::prepare(&frozen, edit, budget, &flag).map(Box::new);
            let _ = tx.send(result);
            ctx.request_repaint();
        });
        self.animation_finish.staged = None;
        self.animation_finish.review = false;
        self.animation_finish.job = Some(Job {
            document,
            cancel,
            rx,
            worker: Some(worker),
        });
        Ok(())
    }
    fn poll_animation_finish(&mut self, ctx: &egui::Context) {
        if self
            .animation_finish
            .job
            .as_ref()
            .is_some_and(|j| !self.documents.iter().any(|d| d.id == j.document))
        {
            self.animation_finish.job = None;
        }
        if self
            .animation_finish
            .staged
            .as_ref()
            .is_some_and(|s| !self.documents.iter().any(|d| s.matches(d)))
        {
            self.animation_finish.staged = None;
            self.animation_finish.notice =
                "Document changed or closed; prepared result discarded".into();
        }
        let ready = self
            .animation_finish
            .job
            .as_ref()
            .and_then(|j| j.rx.try_recv().ok());
        if let Some(result) = ready {
            let canceled = self
                .animation_finish
                .job
                .as_ref()
                .is_some_and(|j| j.cancel.load(Ordering::Relaxed));
            self.animation_finish.job = None;
            if canceled {
                self.animation_finish.notice = "Cancelled; original preserved".into();
            } else {
                match result {
                    Ok(s) if self.documents.iter().any(|d| s.matches(d)) => {
                        self.animation_finish.staged = Some(s)
                    }
                    Ok(_) => {
                        self.animation_finish.notice =
                            "Prepared result became stale; drawing unchanged".into()
                    }
                    Err(e) => self.animation_finish.notice = e,
                }
            }
        }
        if self.animation_finish.job.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(25));
        }
    }
    fn apply_animation_finish(&mut self) -> Result<(), String> {
        if !self.animation_finish.review {
            return Err("Review output/audio/name changes before applying".into());
        }
        let p = self
            .animation_finish
            .staged
            .take()
            .ok_or("Prepare finishing changes first")?;
        let n = self.active.ok_or("Choose an animation")?;
        let summary = p.summary.clone();
        p.commit(&mut self.documents[n])?;
        self.animation_finish.notice = summary;
        self.animation_finish.stamp = None;
        self.reset_timeline_view(self.documents[n].id);
        Ok(())
    }
    pub(super) fn animation_finish_window(&mut self, ctx: &egui::Context) {
        self.poll_animation_finish(ctx);
        if !self.animation_finish.open {
            return;
        }
        let Some(n) = self.active else {
            return;
        };
        let d = &self.documents[n];
        let Some(a) = &d.animation else {
            return;
        };
        let mut p = std::mem::take(&mut self.animation_finish);
        if p.stamp != Some((d.id, d.revision)) {
            let range = d.scene.timing.playback;
            p.first = range.map_or(1, |p| p.first + 1);
            p.last = range.map_or(a.frames, |p| p.end);
            let s = d.scene.animation_output.settings;
            p.safe = s.safe.is_some();
            p.percent = s.safe.unwrap_or([90, 90]);
            p.area = s.area.unwrap_or(Area::Canvas);
            let b = d
                .scene
                .animation_output
                .at(d.current_frame)
                .map(|f| f.bounds)
                .unwrap_or(drawer_core::scene::Rect {
                    min: [0., 0.],
                    max: [d.width as f64, d.height as f64],
                });
            p.bounds = [b.min[0], b.min[1], b.max[0], b.max[1]];
            p.stamp = Some((d.id, d.revision));
        }
        let audio = drawer_core::timeline_timing::inventory(d)
            .map(|t| t.audio)
            .unwrap_or_default();
        p.audio = p
            .audio
            .filter(|id| audio.iter().any(|c| c.id == *id))
            .or_else(|| audio.first().map(|c| c.id));
        if let Some(c) = p.audio.and_then(|id| audio.iter().find(|c| c.id == id))
            && p.audio_stamp != Some((d.id, d.revision, c.id))
        {
            p.audio_start = a.audio[c.index].start + 1;
            p.gain = c.sound.gain;
            p.mute = c.sound.mute;
            p.audio_stamp = Some((d.id, d.revision, c.id));
        }
        let mut edit = None;
        let mut action = 0;
        let mut open = true;
        egui::Window::new("Animation finishing / safe area, audio and names").default_pos(egui::pos2(25.,20.)).default_width(760.).max_height(635.).vscroll(true).open(&mut open).show(ctx,|ui|{
  ui.add_sized([700.,20.],egui::Label::new(&p.notice).truncate()).on_hover_text(&p.notice);
  if let Some(j)=&p.job{ui.spinner();let r=ui.button("Cancel finishing preparation");p.controls.insert("cancel-job",r.rect);if r.clicked(){j.cancel.store(true,Ordering::Relaxed);}}
  if let Some(s)=&p.staged{ui.label(&s.summary);egui::ScrollArea::vertical().id_salt("finish-rows").max_height(90.).show_rows(ui,20.,s.rows.len(),|ui,range|{for i in range{ui.add(egui::Label::new(&s.rows[i]).truncate()).on_hover_text(&s.rows[i]);}});let r=ui.checkbox(&mut p.review,"I reviewed the captured range, geometry, audio or name changes");p.controls.insert("review",r.rect);let r=ui.add_enabled(p.review,egui::Button::new("Apply reviewed finishing changes"));p.controls.insert("apply",r.rect);if r.clicked(){action=1;}
  if ui.button("Discard finishing preview").clicked(){action=2;}}
  ui.horizontal_wrapped(|ui|{ui.checkbox(&mut p.preferences.view.output,"Show output frame");ui.checkbox(&mut p.preferences.view.safe,"Show title safe");ui.checkbox(&mut p.preferences.view.overflow,"Show overflow bounds");ui.checkbox(&mut p.preferences.view.range,"Mark frames outside output range");let r=ui.button("Save local view / naming defaults");p.controls.insert("save",r.rect);if r.clicked(){action=3;}});
  ui.label("Guides are view-only and never paint into image/movie output. Safe geometry and output defaults are saved in this timeline.");
  ui.horizontal(|ui|{ui.checkbox(&mut p.safe,"Define title-safe area");ui.label("Width / height %");for n in &mut p.percent{ui.add(egui::DragValue::new(n).range(1..=100));}ui.label("Default first / last included frame");ui.add(egui::DragValue::new(&mut p.first).range(1..=a.frames));ui.add(egui::DragValue::new(&mut p.last).range(1..=a.frames));});
  ui.horizontal_wrapped(|ui|{for(area,label)in [(Area::Canvas,"Canvas"),(Area::OutputFrame,"Output frames"),(Area::Overflow,"Canvas + overflow"),(Area::Selection,"Selection")]{ui.selectable_value(&mut p.area,area,label);}});
  ui.checkbox(&mut p.update_frame,"Set output-frame key at the captured playhead");if p.update_frame{ui.horizontal(|ui|{ui.label("Left / top / right / bottom");for v in &mut p.bounds{ui.add(egui::DragValue::new(v).speed(0.5));}});}
  ui.horizontal(|ui|{let r=ui.button("Prepare output / safe settings");p.controls.insert("output",r.rect);if r.clicked(){edit=Some(Edit::Output{settings:Settings{safe:p.safe.then_some(p.percent),area:Some(p.area)},first:p.first-1,end:p.last,bounds:p.update_frame.then_some(drawer_core::scene::Rect{min:[p.bounds[0],p.bounds[1]],max:[p.bounds[2],p.bounds[3]]})});}
  if ui.button("Prepare clearing output-frame keys").clicked(){edit=Some(Edit::ClearFrames);}
 if ui.button("Open image / movie output").clicked(){action=4;}});
  ui.collapsing("Audio start / volume and WAV output",|ui|{egui::ComboBox::from_label("Clip").selected_text(p.audio.map_or("No audio; import in audio manager".into(),|id|format!("Audio{id}"))).show_ui(ui,|ui|{for c in &audio{let audio=&a.audio[c.index];if ui.selectable_value(&mut p.audio,Some(c.id),&audio.name).clicked(){p.audio_start=audio.start+1;p.gain=c.sound.gain;p.mute=c.sound.mute;}}});ui.horizontal(|ui|{ui.label("Start frame / volume %");ui.add(egui::DragValue::new(&mut p.audio_start).range(1..=10000));ui.add(egui::Slider::new(&mut p.gain,0..=400));ui.checkbox(&mut p.mute,"Mute");let r=ui.button("Prepare audio start / volume");p.controls.insert("audio",r.rect);if r.clicked(){edit=p.audio.map(|id|Edit::Audio{id,start:p.audio_start-1,gain:p.gain,mute:p.mute});}});if ui.button("Open audio manager / WAV output").clicked(){action=5;}});
  ui.collapsing("Cel / shot batch naming",|ui|{ui.label("Patterns: {n} counter, {id} stable ID, {old} original name; {{ / }} literal braces. NFC names are limited to48 characters /180 UTF-8 bytes and checked for local filename use / duplicates.");ui.checkbox(&mut p.cels,"Rename all common editable cel sources");ui.checkbox(&mut p.shots,"Rename all shots in active timeline");for(label,rule)in [("Cel",&mut p.preferences.cel),("Shot",&mut p.preferences.shot)]{ui.horizontal(|ui|{ui.label(label);ui.text_edit_singleline(&mut rule.pattern);ui.label("Start / step / padding");ui.add(egui::DragValue::new(&mut rule.start));ui.add(egui::DragValue::new(&mut rule.step).range(1..=1000));ui.add(egui::DragValue::new(&mut rule.width).range(0..=9));});}let r=ui.button("Prepare cel / shot renaming");p.controls.insert("names",r.rect);if r.clicked(){edit=Some(Edit::Names{cels:p.cels,shots:p.shots,cel:p.preferences.cel.clone(),shot:p.preferences.shot.clone()});}});
 });
        p.open = open;
        self.animation_finish = p;
        if !open {
            self.animation_finish.job = None;
            self.animation_finish.staged = None;
            return;
        }
        let result = (|| {
            match action {
                1 => self.apply_animation_finish()?,
                2 => self.animation_finish.staged = None,
                3 => {
                    if let Some(s) = &self.storage {
                        self.animation_finish.preferences.save(s.root())?;
                    }
                    self.animation_finish.notice =
                        "Local guide visibility / name defaults saved".into();
                }
                4 => self.begin_animation_export()?,
                5 => self.begin_audio()?,
                _ => {}
            }
            if let Some(edit) = edit {
                self.stage_animation_finish(ctx, edit)?;
            }
            Ok::<_, String>(())
        })();
        if let Err(e) = result {
            self.animation_finish.notice = e;
        }
    }
}

#[path = "animation_finish_verify.rs"]
mod verify;
pub(super) fn smoke(root: &std::path::Path) -> Result<(), String> {
    verify::run(root)
}
#[cfg(test)]
mod tests {
    #[test]
    fn pointer_safe_output_review_audio_names_view_restart_cancel_stale_and_close() {
        let t = crate::tests::TestDirectory::new();
        super::smoke(&t.0).unwrap();
    }
}
