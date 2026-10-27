use super::*;
use drawer_core::{audio as core, audio_mix as mix, timeline_timing as timing};
use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
};
#[derive(Clone, PartialEq)]
struct Values {
    name: String,
    start: u32,
    gain: u16,
    mute: bool,
}
#[derive(Clone, Copy, PartialEq, Eq)]
struct Stamp {
    doc: u64,
    revision: u64,
}
impl Stamp {
    fn of(d: &Document) -> Self {
        Self {
            doc: d.id,
            revision: d.revision,
        }
    }
}
struct Prepared {
    stamp: Stamp,
    input: core::Input,
}
enum Done {
    Imported(Prepared),
    Written { clipped: u64, path: PathBuf },
}
struct Job {
    cancel: Arc<AtomicBool>,
    progress: Arc<AtomicU64>,
    receiver: mpsc::Receiver<Result<Done, String>>,
    import: Option<u64>,
}
impl Drop for Job {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}
fn job(
    ctx: &egui::Context,
    import: Option<u64>,
    work: impl FnOnce(&AtomicBool, &AtomicU64) -> Result<Done, String> + Send + 'static,
) -> Job {
    let cancel = Arc::new(AtomicBool::new(false));
    let progress = Arc::new(AtomicU64::new(0));
    let flag = cancel.clone();
    let p = progress.clone();
    let (tx, receiver) = mpsc::channel();
    let ctx = ctx.clone();
    std::thread::spawn(move || {
        let result = work(&flag, &p);
        let _ = tx.send(result);
        ctx.request_repaint();
    });
    Job {
        cancel,
        progress,
        receiver,
        import,
    }
}
pub(super) struct Panel {
    open: bool,
    tab: u8,
    path: String,
    raw: bool,
    rate: u32,
    channels: u16,
    linked: bool,
    import_at: u32,
    quantize: bool,
    staged: Option<Prepared>,
    job: Option<Job>,
    selected: Option<u64>,
    stamp: Option<(Stamp, u64)>,
    values: Values,
    base: Values,
    export_path: String,
    first: u32,
    last: u32,
    export_rate: u32,
    export_channels: u16,
    overwrite: bool,
    refresh_confirm: bool,
    scrub: bool,
    scrub_frame: Option<(u64, u32)>,
    notice: String,
    source_notice: String,
    report: String,
    mock: bool,
    pub(super) mock_fail: bool,
    pub import_button: Option<egui::Rect>,
    pub apply: Option<egui::Rect>,
    pub insert: Option<egui::Rect>,
    pub play: Option<egui::Rect>,
    pub scrub_button: Option<egui::Rect>,
    pub export: Option<egui::Rect>,
}
impl Default for Panel {
    fn default() -> Self {
        let values = Values {
            name: String::new(),
            start: 1,
            gain: 100,
            mute: false,
        };
        Self {
            open: false,
            tab: 0,
            path: String::new(),
            raw: false,
            rate: 48000,
            channels: 2,
            linked: false,
            import_at: 1,
            quantize: false,
            staged: None,
            job: None,
            selected: None,
            stamp: None,
            values: values.clone(),
            base: values,
            export_path: String::new(),
            first: 1,
            last: 1,
            export_rate: 48000,
            export_channels: 2,
            overwrite: false,
            refresh_confirm: false,
            scrub: false,
            scrub_frame: None,
            notice: String::new(),
            source_notice: String::new(),
            report: String::new(),
            mock: false,
            mock_fail: false,
            import_button: None,
            apply: None,
            insert: None,
            play: None,
            scrub_button: None,
            export: None,
        }
    }
}
enum Driver {
    #[cfg(windows)]
    Native(drawer_platform::audio::Player),
    Mock(Arc<AtomicU64>),
}
pub(super) struct Active {
    doc: u64,
    revision: u64,
    stream: mix::Stream,
    driver: Driver,
    scrub_end: Option<u64>,
    lag: f64,
}
impl Active {
    fn status(&self) -> (u64, bool, Option<String>, u64) {
        match &self.driver {
            #[cfg(windows)]
            Driver::Native(p) => {
                let s = p.status();
                (s.played, s.finished, s.error, s.clipped)
            }
            Driver::Mock(p) => {
                let played = p.load(Ordering::Relaxed);
                (
                    played,
                    self.stream.duration().is_some_and(|n| played >= n),
                    None,
                    0,
                )
            }
        }
    }
}
impl Drawer {
    pub(super) fn audio_owns(&self, id: u64) -> bool {
        self.audio_player
            .as_ref()
            .is_some_and(|a| a.doc == id && a.scrub_end.is_none())
    }
    pub(super) fn stop_audio_for(&mut self, id: u64) {
        if self.audio_player.as_ref().is_some_and(|a| a.doc == id) {
            self.audio_player = None;
        }
    }
    pub(super) fn begin_audio(&mut self) -> Result<(), String> {
        self.finish_stroke();
        if self.transform_unconfirmed()
            || self.inference_unconfirmed()
            || self.text_editor.draft.is_some()
        {
            return Err("Finish pending drawing edits first".into());
        }
        let n = self
            .timeline_audio_context()
            .ok_or("Choose an animation document")?;
        let d = &self.documents[n];
        let a = d.animation.as_ref().ok_or("Create a timeline first")?;
        if !self.audio.open {
            self.audio.import_at = d.current_frame + 1;
            self.audio.first = 1;
            self.audio.last = a.frames;
        }
        self.audio.open = true;
        Ok(())
    }
    pub(super) fn audio_save_guard(&self, index: usize) -> Result<(), String> {
        let id = self.documents[index].id;
        if self.audio.stamp.is_some_and(|(s, _)| s.doc == id)
            && self.audio.values != self.audio.base
            || self
                .audio
                .staged
                .as_ref()
                .is_some_and(|s| s.stamp.doc == id)
            || self
                .audio
                .job
                .as_ref()
                .is_some_and(|j| j.import == Some(id))
        {
            return Err("Apply/reset audio settings or insert/discard prepared audio before saving or closing".into());
        }
        Ok(())
    }
    fn start_audio(
        &mut self,
        n: usize,
        start: u32,
        repeat: bool,
        scrub: bool,
    ) -> Result<(), String> {
        self.audio_player = None;
        if self.audio.mock_fail {
            return Err("No audio output device (verified fallback)".into());
        }
        let d = &self.documents[n];
        let a = d.animation.as_ref().ok_or("Choose an animation")?;
        let plan = Arc::new(mix::Plan::capture(d, 48000, 2)?);
        let range = d.scene.timing.playback.unwrap_or(timing::Playback {
            first: 0,
            end: a.frames,
            repeat,
        });
        let stream = plan.stream(
            start,
            if scrub { start } else { range.first },
            if scrub { a.frames } else { range.end },
            !scrub && range.repeat,
        )?;
        let stream = if scrub { stream.limited(4800)? } else { stream };
        let driver = if self.audio.mock {
            Driver::Mock(Arc::new(AtomicU64::new(0)))
        } else {
            #[cfg(windows)]
            {
                Driver::Native(drawer_platform::audio::Player::start(stream.clone())?)
            }
            #[cfg(not(windows))]
            {
                return Err("Audio playback requires Windows; WAV output remains available".into());
            }
        };
        self.audio_player = Some(Active {
            doc: d.id,
            revision: d.revision,
            stream,
            driver,
            scrub_end: scrub.then_some(4800),
            lag: 0.,
        });
        Ok(())
    }
    pub(super) fn toggle_audio_playback(
        &mut self,
        ctx: &egui::Context,
        n: usize,
        repeat: bool,
    ) -> Result<(), String> {
        self.finish_stroke();
        let id = self.documents[n].id;
        if self.timeline_audio_playing(id) {
            self.stop_timeline_playback(id);
            return Ok(());
        }
        let start = self.documents[n].current_frame;
        self.begin_timeline_editor()?;
        if self.documents[n]
            .animation
            .as_ref()
            .is_some_and(|a| !a.audio.is_empty())
            && let Err(e) = self.start_audio(n, start, repeat, false)
        {
            self.audio.notice = format!("{e}; continuing visual playback only");
        }
        self.timeline_audio_mark(id, ctx.input(|i| i.time), start, true);
        ctx.request_repaint();
        Ok(())
    }
    fn scrub_audio(&mut self, n: usize) -> Result<(), String> {
        self.finish_stroke();
        let id = self.documents[n].id;
        self.stop_timeline_playback(id);
        self.audio.scrub_frame = Some((id, self.documents[n].current_frame));
        self.start_audio(n, self.documents[n].current_frame, false, true)
    }
    pub(super) fn audio_tick(&mut self, ctx: &egui::Context) {
        if let Some(result) = self
            .audio
            .job
            .as_ref()
            .and_then(|j| match j.receiver.try_recv() {
                Ok(r) => Some(r),
                Err(mpsc::TryRecvError::Disconnected) => Some(Err("Audio worker stopped".into())),
                Err(_) => None,
            })
        {
            let cancelled = self
                .audio
                .job
                .as_ref()
                .is_some_and(|j| j.import.is_some() && j.cancel.load(Ordering::Relaxed));
            self.audio.job = None;
            let result = if cancelled {
                Err("Audio import cancelled".into())
            } else {
                result
            };
            match result {
                Ok(Done::Imported(p)) => {
                    self.audio.staged = Some(p);
                    self.audio.notice = "Audio snapshot prepared; review and insert".into();
                }
                Ok(Done::Written { clipped, path }) => {
                    self.audio.notice = format!(
                        "WAV published to {}; {clipped} clipped samples (reduce volume if nonzero)",
                        path.display()
                    );
                }
                Err(e) => self.audio.notice = e,
            }
        }
        let context = self.timeline_audio_context();
        if let Some(mut active) = self.audio_player.take() {
            let n = context.filter(|n| {
                self.documents[*n].id == active.doc
                    && self.documents[*n].revision == active.revision
            });
            if let Some(n) = n.filter(|_| {
                active.scrub_end.is_some()
                    || self.timeline_audio_open() && self.timeline_audio_playing(active.doc)
            }) {
                let (played, finished, error, clipped) = active.status();
                if let Some(e) = error {
                    self.audio.notice = if active.scrub_end.is_some() {
                        e
                    } else {
                        format!("{e}; continuing visual playback only")
                    };
                } else {
                    let position = active.stream.position(played);
                    let d = &mut self.documents[n];
                    if active.scrub_end.is_none() {
                        if position.frame >= d.current_frame
                            && let Some(sample) = active.stream.absolute_sample(played)
                        {
                            let end = mix::boundary(
                                d.current_frame.saturating_add(1),
                                active.stream.plan.fps,
                                48000,
                            );
                            active.lag = active.lag.max(sample.saturating_sub(end) as f64 / 48.);
                        }
                        d.current_frame = position.frame;
                    }
                    self.audio.report = format!(
                        "Device-clock playback · maximum observed display delay {:.2}ms · clipped samples{clipped}. Speaker/display hardware latency is not measured.",
                        active.lag
                    );
                    if !finished
                        && !position.finished
                        && active.scrub_end.is_none_or(|end| played < end)
                    {
                        self.audio_player = Some(active);
                        ctx.request_repaint_after(std::time::Duration::from_millis(10));
                    } else {
                        let id = d.id;
                        self.timeline_audio_mark(id, 0., 0, false);
                    }
                }
            } else {
                self.timeline_audio_mark(active.doc, 0., 0, false);
            }
        }
        if self.audio.open
            && self.audio.scrub
            && self.audio_player.is_none()
            && let Some(n) = context
        {
            let key = (self.documents[n].id, self.documents[n].current_frame);
            if self.audio.scrub_frame != Some(key) {
                self.audio.scrub_frame = Some(key);
                if !self.timeline_audio_playing(key.0)
                    && let Err(e) = self.scrub_audio(n)
                {
                    self.audio.notice = e;
                }
            }
        }
    }
    fn audio_import_path_selected(&mut self, path: PathBuf) {
        self.audio.path = path.display().to_string();
        self.audio.staged = None;
        self.audio.quantize = false;
        if let Some(j) = self.audio.job.as_ref().filter(|j| j.import.is_some()) {
            j.cancel.store(true, Ordering::Relaxed);
        }
    }
    fn prepare_audio_import(&mut self, ctx: &egui::Context, n: usize) -> Result<(), String> {
        if self.audio.job.is_some() {
            return Err("Wait for or cancel the current audio job".into());
        }
        let stamp = Stamp::of(&self.documents[n]);
        let path = PathBuf::from(&self.audio.path);
        let raw = self.audio.raw.then_some(core::Raw {
            rate: self.audio.rate,
            channels: self.audio.channels,
        });
        let linked = self.audio.linked;
        self.audio.staged = None;
        self.audio.quantize = false;
        self.audio.job = Some(job(ctx, Some(stamp.doc), move |cancel, _| {
            let input = core::load(&path, raw, linked)?;
            if cancel.load(Ordering::Relaxed) {
                return Err("Audio import cancelled".into());
            }
            Ok(Done::Imported(Prepared { stamp, input }))
        }));
        Ok(())
    }
    fn insert_audio(&mut self, n: usize) -> Result<(), String> {
        let p = self
            .audio
            .staged
            .as_ref()
            .ok_or("Prepare an audio file first")?;
        if p.stamp != Stamp::of(&self.documents[n]) {
            return Err("Drawing changed since audio preparation; prepare again".into());
        }
        self.finish_stroke();
        self.stop_timeline_playback(self.documents[n].id);
        let p = self.audio.staged.as_ref().unwrap();
        let state = core::insert(
            &self.documents[n],
            &p.input,
            self.audio.import_at.saturating_sub(1),
            self.audio.quantize,
            self.timeline_budget(),
        )?;
        let selected = state.scene.timing.audio.last().map(|c| c.id);
        self.documents[n].commit_state(state)?;
        self.audio.selected = selected;
        self.audio.staged = None;
        self.audio.tab = 1;
        self.audio.stamp = None;
        Ok(())
    }
    fn export_audio(&mut self, ctx: &egui::Context, n: usize) -> Result<(), String> {
        self.audio_save_guard(n)?;
        if self.audio.job.is_some() {
            return Err("Wait for or cancel audio job first".into());
        }
        let o = mix::Options {
            first: self.audio.first.saturating_sub(1),
            last: self.audio.last.saturating_sub(1),
            rate: self.audio.export_rate,
            channels: self.audio.export_channels,
        };
        let stream = mix::prepare(&self.documents[n], o)?;
        let path = PathBuf::from(&self.audio.export_path);
        let overwrite = self.audio.overwrite;
        self.audio.job = Some(job(ctx, None, move |cancel, progress| {
            let dest = mix::Destination::capture(&stream, &path, overwrite)?;
            mix::write(&stream, &dest, cancel, progress)
                .map(|clipped| Done::Written { clipped, path })
        }));
        Ok(())
    }
    pub(super) fn audio_window(&mut self, ctx: &egui::Context) {
        if !self.audio.open {
            return;
        }
        let Some(n) = self.timeline_audio_context() else {
            return;
        };
        let d = &self.documents[n];
        let Some(a) = &d.animation else { return };
        let mut p = std::mem::take(&mut self.audio);
        if p.values != p.base && p.stamp.is_some_and(|(s, _)| s.doc != d.id) {
            egui::Window::new("Local audio pending values").show(ctx,|ui|{ui.label("Audio values belong to another document. Return to it to apply them, or discard them here.");if ui.button("Discard pending values").clicked(){p.values=p.base.clone();p.stamp=None;}});
            self.audio = p;
            return;
        }
        let data = match timing::inventory(d) {
            Ok(x) => x,
            Err(e) => {
                p.notice = e;
                self.audio = p;
                return;
            }
        };
        if p.selected
            .is_none_or(|id| !data.audio.iter().any(|c| c.id == id))
        {
            p.selected = data.audio.first().map(|c| c.id);
            p.stamp = None;
        }
        if let Some(c) = p
            .selected
            .and_then(|id| data.audio.iter().find(|c| c.id == id))
        {
            let stamp = (Stamp::of(d), c.id);
            if p.stamp != Some(stamp) && p.values != p.base {
                p.notice = "Drawing changed; reset pending audio values before applying".into();
            }
            if p.stamp != Some(stamp) && p.values == p.base {
                let audio = &a.audio[c.index];
                p.values = Values {
                    name: audio.name.clone(),
                    start: audio.start + 1,
                    gain: c.sound.gain,
                    mute: c.sound.mute,
                };
                p.base = p.values.clone();
                p.stamp = Some(stamp);
                p.source_notice = c.sound.source.as_ref().map_or_else(
                    || "Embedded legacy PCM".into(),
                    |s| {
                        if s.linked && !s.path.exists() {
                            "Linked file missing; saved PCM retained".into()
                        } else if s.linked {
                            "Linked file; saved PCM used until explicit refresh".into()
                        } else {
                            "Embedded PCM; original file independent".into()
                        }
                    },
                );
            }
        }
        let (mut open, mut action, mut browse) = (p.open, 0, 0);
        egui::Window::new("Local audio / synchronized playback").id(egui::Id::new("timeline-audio")).open(&mut open).default_pos(egui::pos2(70.,50.)).default_width(740.).max_height(610.).vscroll(true).show(ctx,|ui|{
            ui.label(&p.notice);ui.label(&p.report);
            ui.horizontal(|ui|{for (n,name) in [(0,"Import"),(1,"Clips / volume"),(2,"WAV output")]{ui.selectable_value(&mut p.tab,n,name);}});
            ui.horizontal(|ui|{let r=ui.button(if self.timeline_audio_playing(d.id){"Pause"}else{"Play with timeline"});p.play=Some(r.rect);if r.clicked(){action=1;}let r=ui.button("Scrub current frame (100ms)");p.scrub_button=Some(r.rect);if r.clicked(){action=2;}ui.checkbox(&mut p.scrub,"Scrub on frame changes");});
            ui.horizontal(|ui|{let mut seek=d.current_frame+1;ui.label("Seek / scrub frame");if ui.add(egui::DragValue::new(&mut seek).range(1..=a.frames)).changed(){action=11;p.scrub_frame=Some((d.id,seek-1));}});
            ui.label("Playback uses saved PCM. Missing devices keep visual playback and WAV output available. Edit/close/switching documents stops the sound.");
            if let Some(j)=&p.job{ui.horizontal(|ui|{ui.label(format!("Audio job: {} sample frames",j.progress.load(Ordering::Relaxed)));if ui.button("Cancel audio job").clicked(){j.cancel.store(true,Ordering::Relaxed);}});ctx.request_repaint_after(std::time::Duration::from_millis(30));}
            if p.tab==0 {egui::CollapsingHeader::new("Import local WAV / raw PCM").default_open(true).show(ui,|ui|{
                let before=(p.path.clone(),p.raw,p.rate,p.channels,p.linked);ui.horizontal(|ui|{ui.text_edit_singleline(&mut p.path);if ui.button("Browse audio…").clicked(){browse=1;}});
                ui.horizontal(|ui|{ui.checkbox(&mut p.linked,"Link original with saved PCM cache");ui.checkbox(&mut p.raw,"Headerless PCM16LE");});
                if p.raw{ui.horizontal(|ui|{ui.label("Raw rate / channels");ui.add(egui::DragValue::new(&mut p.rate).range(8000..=192000));ui.add(egui::DragValue::new(&mut p.channels).range(1..=2));});}
                if before!=(p.path.clone(),p.raw,p.rate,p.channels,p.linked){p.staged=None;if let Some(j)=p.job.as_ref().filter(|j|j.import.is_some()){j.cancel.store(true,Ordering::Relaxed);}}
                ui.horizontal(|ui|{ui.label("Insert at frame");ui.add(egui::DragValue::new(&mut p.import_at).range(1..=10000));let r=ui.add_enabled(p.job.is_none(),egui::Button::new("Prepare audio snapshot"));p.import_button=Some(r.rect);if r.clicked(){action=3;}});
                ui.label("WAV: PCM8/16/24/32 or float32 mono/stereo. Raw: PCM16LE with explicit rate/channels. Other codecs are unsupported. Imported originals are stored as16-bit PCM.");
                if let Some(s)=&p.staged{ui.label(format!("{} · {} · {:.3}s ·{} sample frames",s.input.name,s.input.decoded.description,s.input.decoded.frames() as f64/s.input.decoded.rate as f64,s.input.decoded.frames()));
                    if s.input.decoded.quantized{ui.checkbox(&mut p.quantize,"Confirm24/32-bit to16-bit conversion (float peaks are clipped)");}
                    ui.horizontal(|ui|{let r=ui.add_enabled(!s.input.decoded.quantized||p.quantize,egui::Button::new("Insert prepared audio"));p.insert=Some(r.rect);if r.clicked(){action=4;}
if ui.button("Discard prepared audio").clicked(){action=5;}});
                }
            });}
            if p.tab==1 {ui.separator();ui.add_enabled_ui(p.values==p.base,|ui|{egui::ComboBox::from_id_salt("audio-clip").selected_text(p.selected.map_or_else(||"No audio clip".into(),|id|format!("Audio clip{id}"))).show_ui(ui,|ui|{for c in &data.audio {ui.selectable_value(&mut p.selected,Some(c.id),format!("{} [id{}]",a.audio[c.index].name,c.id));}});});
            if let Some(c)=p.selected.and_then(|id|data.audio.iter().find(|c|c.id==id)){ui.label(format!("{}Hz · {}ch ·{} samples · source{}{}",a.audio[c.index].rate,a.audio[c.index].channels,a.audio[c.index].sample_count,c.source,if c.locked{" · locked"}else{""}));ui.label(&p.source_notice);
                ui.horizontal(|ui|{ui.label("Name");ui.text_edit_singleline(&mut p.values.name);ui.label("Start frame");ui.add(egui::DragValue::new(&mut p.values.start).range(1..=10000));});
                ui.horizontal(|ui|{ui.add(egui::Slider::new(&mut p.values.gain,0..=400).text("Volume %"));ui.checkbox(&mut p.values.mute,"Mute");let r=ui.button("Apply clip settings");p.apply=Some(r.rect);if r.clicked(){action=6;}
if ui.button("Reset values").clicked(){p.values=p.base.clone();}});
                ui.horizontal(|ui|{if ui.button("Check linked source").clicked(){action=7;}ui.checkbox(&mut p.refresh_confirm,"Confirm shared refresh /16-bit conversion");if ui.button("Refresh linked original").clicked(){action=8;}
if ui.button("Remove this clip").clicked(){action=9;}});
            }
            }
            if p.tab==2 {ui.separator();ui.label("WAV output · immutable snapshot ·16-bit PCM · mono/stereo. Multichannel legacy clips are averaged to mono before mixing. Existing files require replacement confirmation.");
            ui.horizontal(|ui|{ui.text_edit_singleline(&mut p.export_path);if ui.button("Choose WAV output…").clicked(){browse=2;}});
            ui.horizontal(|ui|{ui.label("First / last included");ui.add(egui::DragValue::new(&mut p.first).range(1..=a.frames));ui.add(egui::DragValue::new(&mut p.last).range(1..=a.frames));ui.label("Hz / channels");ui.add(egui::DragValue::new(&mut p.export_rate).range(8000..=192000));ui.add(egui::DragValue::new(&mut p.export_channels).range(1..=2));ui.checkbox(&mut p.overwrite,"Confirm overwrite");});
            let r=ui.add_enabled(p.job.is_none(),egui::Button::new("Export WAV snapshot"));p.export=Some(r.rect);if r.clicked(){action=10;}}
        });
        if !open
            && (p.values != p.base
                || p.staged.is_some()
                || p.job.as_ref().is_some_and(|j| j.import.is_some()))
        {
            p.notice="Apply/reset clip settings or insert/discard/cancel prepared audio before closing controls".into();
            open = true;
        }
        p.open = open;
        self.audio = p;
        if browse == 1
            && let Some(path) = rfd::FileDialog::new()
                .add_filter("Local WAV / PCM", &["wav", "pcm", "raw"])
                .pick_file()
        {
            self.audio_import_path_selected(path);
        }
        if browse == 2
            && let Some(path) = rfd::FileDialog::new()
                .add_filter("WAV", &["wav"])
                .save_file()
        {
            self.audio.export_path = path.display().to_string();
            self.audio.overwrite = false;
        }
        let result = (|| {
            match action {
                1 => self.toggle_audio_playback(ctx, n, true)?,
                2 => self.scrub_audio(n)?,
                3 => self.prepare_audio_import(ctx, n)?,
                4 => self.insert_audio(n)?,
                5 => self.audio.staged = None,
                6 => {
                    if self
                        .audio
                        .stamp
                        .is_none_or(|(s, _)| s != Stamp::of(&self.documents[n]))
                    {
                        return Err("Drawing changed; reset clip values before applying".into());
                    }
                    let id = self.audio.selected.ok_or("Choose an audio clip")?;
                    let v = self.audio.values.clone();
                    self.finish_stroke();
                    self.stop_timeline_playback(self.documents[n].id);
                    let s = core::edit(
                        &self.documents[n],
                        id,
                        v.name,
                        v.start - 1,
                        v.gain,
                        v.mute,
                        self.timeline_budget(),
                    )?;
                    self.documents[n].commit_state(s)?;
                    self.audio.base = self.audio.values.clone();
                    self.audio.stamp = None;
                }
                7 => {
                    let id = self.audio.selected.ok_or("Choose an audio clip")?;
                    let data = timing::inventory(&self.documents[n])?;
                    self.audio.source_notice = data
                        .audio
                        .iter()
                        .find(|c| c.id == id)
                        .and_then(|c| c.sound.source.as_ref())
                        .map_or_else(|| "Embedded legacy source".into(), core::source_status);
                }
                8 => {
                    let id = self.audio.selected.ok_or("Choose linked audio")?;
                    self.stop_timeline_playback(self.documents[n].id);
                    let s = core::refresh(
                        &self.documents[n],
                        id,
                        self.audio.refresh_confirm,
                        self.timeline_budget(),
                    )?;
                    self.documents[n].commit_state(s)?;
                    self.audio.stamp = None;
                }
                9 => {
                    let id = self.audio.selected.ok_or("Choose audio clip")?;
                    self.stop_timeline_playback(self.documents[n].id);
                    let s = core::remove(&self.documents[n], id, self.timeline_budget())?;
                    self.documents[n].commit_state(s)?;
                    self.audio.stamp = None;
                }
                10 => self.export_audio(ctx, n)?,
                11 => {
                    let frame = self
                        .audio
                        .scrub_frame
                        .map_or(self.documents[n].current_frame, |(_, f)| f);
                    self.stop_timeline_playback(self.documents[n].id);
                    self.documents[n].current_frame = frame;
                    if self.audio.scrub {
                        self.scrub_audio(n)?;
                    }
                }
                _ => {}
            }
            Ok::<_, String>(())
        })();
        if let Err(e) = result {
            self.audio.notice = e;
        }
    }
}
pub(super) fn smoke(root: &Path) -> Result<(), String> {
    drawer_core::audio_verify::verify(root)?;
    let dir = root.join("audio-ui-verify");
    std::fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let result = (|| {
        let file = dir.join("voice.wav");
        std::fs::write(&file, drawer_core::audio_verify::wav(10000, 3200))
            .map_err(|e| e.to_string())?;
        let ctx = egui::Context::default();
        let mut app = Drawer::new(&ctx, None);
        app.simple_view = true;
        app.audio.mock = true;
        app.add_document(drawer_core::keyframes_verify::fixture());
        app.execute_command(&ctx, "timeline-audio");
        if !app.audio.open {
            return Err("Audio command missing".into());
        }
        app.audio.path = file.display().to_string();
        app.audio.linked = true;
        let mut time = 0.;
        let frame = |app: &mut Drawer, time: &mut f64, events| {
            *time += 0.05;
            input_replay::frame(&ctx, app, *time, events);
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
        for _ in 0..3 {
            frame(&mut app, &mut time, vec![]);
        }
        let before = app.documents[0].snapshot();
        let history = app.documents[0].history_counts();
        let r = app
            .audio
            .import_button
            .ok_or("Audio prepare button missing")?;
        click(&mut app, &mut time, r);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(4);
        while app.audio.job.is_some() && std::time::Instant::now() < deadline {
            frame(&mut app, &mut time, vec![]);
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        if app.audio.staged.is_none()
            || app.documents[0].snapshot() != before
            || app.documents[0].history_counts() != history
        {
            return Err(format!(
                "Audio pointer preparation mutated drawing / failed: {}",
                app.audio.notice
            ));
        }
        let blocked = dir.join("blocked.drawer");
        if app.save_to(0, &blocked).is_ok() || blocked.exists() {
            return Err("Prepared audio was silently omitted by saving".into());
        }
        let r = app.audio.insert.ok_or("Audio insert button missing")?;
        click(&mut app, &mut time, r);
        if app.documents[0].animation.as_ref().unwrap().audio.len() != 1
            || app.documents[0].history_counts().0 != history.0 + 1
        {
            return Err(format!("Pointer audio insert failed: {}", app.audio.notice));
        }
        for _ in 0..3 {
            frame(&mut app, &mut time, vec![]);
        }
        app.audio.values.gain = 50;
        let r = app.audio.apply.ok_or("Audio settings apply missing")?;
        click(&mut app, &mut time, r);
        let id = app.documents[0].id;
        let clip = app.documents[0].scene.timing.audio[0].id;
        if app.documents[0].scene.timing.audio[0].sound.gain != 50 {
            return Err(format!("Pointer gain edit failed: {}", app.audio.notice));
        }
        frame(&mut app, &mut time, vec![]);
        app.audio.values.gain = 60;
        if app.save_to(0, &blocked).is_ok() {
            return Err("Pending audio settings were omitted by saving".into());
        }
        app.audio.values = app.audio.base.clone();
        let snapshot = app.documents[0].snapshot();
        let history = app.documents[0].history_counts();
        let r = app.audio.play.ok_or("Audio playback control missing")?;
        click(&mut app, &mut time, r);
        let active = app
            .audio_player
            .as_ref()
            .ok_or("Pointer playback did not open an audio transport")?;
        let Driver::Mock(clock) = &active.driver else {
            return Err("Audio UI fixture opened a real non-silent device".into());
        };
        let clock = clock.clone();
        let elapsed = mix::boundary(6, active.stream.plan.fps, 48000)
            - mix::boundary(active.stream.start, active.stream.plan.fps, 48000);
        clock.store(elapsed, Ordering::Relaxed);
        time += 10.;
        frame(&mut app, &mut time, vec![]);
        if app.documents[0].current_frame != 6
            || app.documents[0].snapshot() != snapshot
            || app.documents[0].history_counts() != history
        {
            return Err("Sample master followed UI wall time or changed drawing history".into());
        }
        time += 10.;
        frame(&mut app, &mut time, vec![]);
        if app.documents[0].current_frame != 6
            || app.audio_player.as_ref().is_none_or(|a| a.lag <= 0.)
        {
            return Err("Sample-clock pause / measured display delay failed".into());
        }
        let mut state = app.documents[0].snapshot();
        state.layers[0].pixels[0] = [0, 0, 255, 255];
        app.documents[0].commit_state(state)?;
        frame(&mut app, &mut time, vec![]);
        if app.audio_player.is_some() || app.timeline_audio_playing(id) {
            return Err("Audio did not stop on document edit".into());
        }
        app.documents[0].undo();
        app.timeline_editor.open = false;
        for _ in 0..3 {
            frame(&mut app, &mut time, vec![]);
        }
        let snapshot = app.documents[0].snapshot();
        let frame_before = app.documents[0].current_frame;
        let r = app.audio.scrub_button.ok_or("Scrub control missing")?;
        click(&mut app, &mut time, r);
        let active = app
            .audio_player
            .as_ref()
            .ok_or("Pointer scrub did not start")?;
        if active.stream.duration() != Some(4800) {
            return Err("Scrub stream was not limited to100ms".into());
        }
        let Driver::Mock(clock) = &active.driver else {
            unreachable!()
        };
        clock.store(4800, Ordering::Relaxed);
        frame(&mut app, &mut time, vec![]);
        if app.audio_player.is_some()
            || app.documents[0].snapshot() != snapshot
            || app.documents[0].current_frame != frame_before
        {
            return Err("Scrub changed drawing/frame or failed to stop".into());
        }
        app.audio.mock_fail = true;
        app.toggle_audio_playback(&ctx, 0, false)?;
        frame(&mut app, &mut time, vec![]);
        if app.audio_player.is_some()
            || !app.timeline_audio_playing(id)
            || !app.audio.notice.contains("No audio output device")
        {
            return Err("No-device visual fallback failed".into());
        }
        app.stop_timeline_playback(id);
        app.audio.mock_fail = false;
        app.timeline_editor.open = false;
        app.audio.tab = 2;
        let output = dir.join("ui-mix.wav");
        app.audio.export_path = output.display().to_string();
        app.audio.first = 1;
        app.audio.last = 24;
        for _ in 0..3 {
            frame(&mut app, &mut time, vec![]);
        }
        let r = app.audio.export.ok_or("WAV export control missing")?;
        click(&mut app, &mut time, r);
        let state = core::edit(
            &app.documents[0],
            clip,
            "Later mute".into(),
            4,
            50,
            true,
            app.timeline_budget(),
        )?;
        app.documents[0].commit_state(state)?;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(4);
        while app.audio.job.is_some() && std::time::Instant::now() < deadline {
            frame(&mut app, &mut time, vec![]);
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        let pcm = core::decode(
            &std::fs::read(&output)
                .map_err(|e| format!("Pointer WAV output failed: {e}; {}", app.audio.notice))?,
            None,
        )?;
        if pcm.samples[16002] != 5000 {
            return Err("WAV job did not retain captured unmuted mixing settings".into());
        }
        app.audio.tab = 0;
        app.prepare_audio_import(&ctx, 0)?;
        app.audio_import_path_selected(file.clone());
        let snapshot = app.documents[0].snapshot();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(4);
        while app.audio.job.is_some() && std::time::Instant::now() < deadline {
            frame(&mut app, &mut time, vec![]);
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        if app.audio.staged.is_some() || app.documents[0].snapshot() != snapshot {
            return Err("Cancelled import changed document / retained a staged source".into());
        }
        app.prepare_audio_import(&ctx, 0)?;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(4);
        while app.audio.job.is_some() && std::time::Instant::now() < deadline {
            frame(&mut app, &mut time, vec![]);
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        let mut state = app.documents[0].snapshot();
        state.layers[0].pixels[1] = [0, 255, 0, 255];
        app.documents[0].commit_state(state)?;
        let snapshot = app.documents[0].snapshot();
        if app.insert_audio(0).is_ok() || app.documents[0].snapshot() != snapshot {
            return Err("Stale prepared audio was inserted".into());
        }
        app.audio.staged = None;
        app.toggle_audio_playback(&ctx, 0, true)?;
        if app.audio_player.is_none() {
            return Err("Playback restart failed".into());
        }
        app.add_document(drawer_core::keyframes_verify::fixture());
        frame(&mut app, &mut time, vec![]);
        if app.audio_player.is_some() {
            return Err("Audio continued after switching documents".into());
        }
        drop(app);
        Ok(())
    })();
    std::fs::remove_dir_all(&dir).map_err(|e| e.to_string())?;
    result
}
#[cfg(test)]
mod tests {
    #[test]
    fn audio_pointer_import_mix_clock_scrub_output_and_cancel() {
        let p = std::env::temp_dir().join(format!("drawer-audio-ui-{}", std::process::id()));
        std::fs::create_dir(&p).unwrap();
        let result = super::smoke(&p);
        std::fs::remove_dir(&p).unwrap();
        result.unwrap();
    }
}
