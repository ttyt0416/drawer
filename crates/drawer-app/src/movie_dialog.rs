use super::*;
use drawer_core::{
    animation::{Clip, Fps},
    movie::{self, Decoder, Info, Options, Source},
};
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
};
enum ResultValue {
    Info(Info),
    Clip(Clip),
}
struct Job {
    cancel: Arc<AtomicBool>,
    receiver: mpsc::Receiver<Result<ResultValue, String>>,
    handle: Option<thread::JoinHandle<()>>,
}
impl Drop for Job {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}
pub(super) struct MovieDialog {
    source: Source,
    decoder: Option<Decoder>,
    info: Option<Info>,
    job: Option<Job>,
    pub clip: Option<Clip>,
    pub insert: bool,
    pub document: Option<(u64, u64)>,
    pub start_frame: String,
    start: String,
    end: String,
    width: String,
    height: String,
    fps: String,
    denominator: String,
    audio: bool,
    filter: drawer_core::interpolation::Filter,
    decoded: Option<Options>,
    error: Option<String>,
    preview: Option<egui::TextureHandle>,
    preview_frame: usize,
    #[cfg(test)]
    pub accept_button: Option<egui::Rect>,
    #[cfg(test)]
    pub cancel_button: Option<egui::Rect>,
}
impl MovieDialog {
    pub fn working(&self) -> bool {
        self.job.is_some()
    }
    fn options(&self) -> Result<Options, String> {
        Ok(Options {
            start: self
                .start
                .parse()
                .map_err(|_| "Enter a start time in seconds")?,
            end: self
                .end
                .parse()
                .map_err(|_| "Enter an end time in seconds")?,
            width: self.width.parse().map_err(|_| "Invalid output width")?,
            height: self.height.parse().map_err(|_| "Invalid output height")?,
            fps: Fps {
                numerator: self.fps.parse().map_err(|_| "Invalid FPS numerator")?,
                denominator: self
                    .denominator
                    .parse()
                    .map_err(|_| "Invalid FPS denominator")?,
            },
            audio: self.audio,
            filter: self.filter,
        })
    }
    fn job(&mut self, ctx: &egui::Context, budget: usize, decode: bool) {
        self.job = None;
        self.error = None;
        self.preview = None;
        self.clip = None;
        let Some(decoder) = self.decoder.clone() else {
            self.error=Some("No local decoder found. Choose FFmpeg/FFprobe 7.1; the app will not download a codec.".into());
            return;
        };
        let options = if decode {
            match self.options() {
                Ok(o) => Some(o),
                Err(e) => {
                    self.error = Some(e);
                    return;
                }
            }
        } else {
            None
        };
        self.decoded = options;
        let source = self.source.clone();
        let info = self.info.clone();
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        let ctx = ctx.clone();
        let (send, receiver) = mpsc::channel();
        let handle = thread::spawn(move || {
            let result = if let Some(options) = options {
                info.as_ref()
                    .ok_or_else(|| "Inspect the movie first".into())
                    .and_then(|info| movie::decode(&source, &decoder, info, options, budget, &flag))
                    .map(ResultValue::Clip)
            } else {
                movie::inspect(&source, &decoder, &flag).map(ResultValue::Info)
            };
            let _ = send.send(result);
            ctx.request_repaint();
        });
        self.job = Some(Job {
            cancel,
            receiver,
            handle: Some(handle),
        });
    }
    fn poll(&mut self) {
        let result = self
            .job
            .as_ref()
            .and_then(|job| match job.receiver.try_recv() {
                Ok(result) => Some(result),
                Err(mpsc::TryRecvError::Empty) => None,
                Err(mpsc::TryRecvError::Disconnected) => {
                    Some(Err("The movie worker stopped without a result".into()))
                }
            });
        if let Some(result) = result {
            self.job = None;
            match result {
                Ok(ResultValue::Info(info)) => {
                    self.end = info.last_time().to_string();
                    self.width = info.width.to_string();
                    self.height = info.height.to_string();
                    self.info = Some(info);
                }
                Ok(ResultValue::Clip(clip)) => {
                    self.preview_frame = 0;
                    self.clip = Some(clip);
                }
                Err(e) => self.error = Some(e),
            }
        }
    }
    pub(super) fn decoder_missing(&self) -> bool {
        self.decoder.is_none()
            && self
                .error
                .as_ref()
                .is_some_and(|e| e.contains("No local decoder"))
    }
    #[cfg(test)]
    pub(super) fn injected(
        path: &std::path::Path,
        clip: Clip,
        insert: bool,
        document: Option<(u64, u64)>,
    ) -> Self {
        let first = &clip.cells[0];
        let info = Info {
            width: first.width,
            height: first.height,
            times: clip.source_times.clone(),
            fps: clip.fps.value().to_string(),
            has_audio: clip.audio.is_some(),
            audio_codec: clip.audio.as_ref().map(|_| "pcm_s16le".into()),
            audio_channels: clip.audio.as_ref().map_or(0, |a| a.channels),
            video_start: 0.0,
            format_start: 0.0,
        };
        let mut dialog = Self {
            source: Source::read(path, DEFAULT_DOCUMENT_BUDGET).unwrap(),
            decoder: None,
            info: Some(info.clone()),
            job: None,
            clip: Some(clip.clone()),
            insert,
            document,
            start_frame: "1".into(),
            start: "0".into(),
            end: info.last_time().to_string(),
            width: info.width.to_string(),
            height: info.height.to_string(),
            fps: clip.fps.numerator.to_string(),
            denominator: clip.fps.denominator.to_string(),
            audio: clip.audio.is_some(),
            filter: drawer_core::interpolation::Filter::Nearest,
            decoded: None,
            error: None,
            preview: None,
            preview_frame: 0,
            accept_button: None,
            cancel_button: None,
        };
        dialog.decoded = Some(dialog.options().unwrap());
        dialog
    }
}
impl Drawer {
    pub(super) fn begin_movie(
        &mut self,
        ctx: &egui::Context,
        path: &std::path::Path,
        insert: bool,
    ) -> Result<(), String> {
        self.begin_movie_from(ctx, path, insert, Decoder::discover())
    }
    pub(super) fn begin_movie_from(
        &mut self,
        ctx: &egui::Context,
        path: &std::path::Path,
        insert: bool,
        decoder: Option<Decoder>,
    ) -> Result<(), String> {
        if insert
            && self
                .active
                .is_none_or(|n| self.documents[n].animation.is_none())
        {
            return Err("Open an animation document before inserting into its timeline".into());
        }
        let budget = DEFAULT_DOCUMENT_BUDGET
            .saturating_sub(self.documents.iter().map(Document::resident_bytes).sum());
        let source = Source::read(path, budget)?;
        let mut dialog = MovieDialog {
            source,
            decoder,
            info: None,
            job: None,
            clip: None,
            insert,
            document: self
                .active
                .map(|n| (self.documents[n].id, self.documents[n].revision)),
            start_frame: "1".into(),
            start: "0".into(),
            end: "0".into(),
            width: "640".into(),
            height: "480".into(),
            fps: "24".into(),
            denominator: "1".into(),
            audio: false,
            filter: self.interpolation.animation,
            decoded: None,
            error: None,
            preview: None,
            preview_frame: 0,
            #[cfg(test)]
            accept_button: None,
            #[cfg(test)]
            cancel_button: None,
        };
        dialog.job(ctx, budget, false);
        self.movie_dialog = Some(dialog);
        Ok(())
    }
    pub(super) fn accept_movie(&mut self) -> Result<(), String> {
        let dialog = self.movie_dialog.as_ref().ok_or("No movie import open")?;
        let clip = dialog
            .clip
            .as_ref()
            .ok_or("Decode and preview the movie range first")?;
        if dialog.decoded != Some(dialog.options()?) {
            return Err("The preview is out of date; decode the new options first".into());
        }
        let original = dialog.source.path.clone();
        if dialog.insert {
            let (id, revision) = dialog.document.ok_or("No target document")?;
            let n = self
                .documents
                .iter()
                .position(|d| d.id == id && d.revision == revision)
                .ok_or("The target drawing changed; reopen the import")?;
            let start = dialog
                .start_frame
                .parse::<u32>()
                .ok()
                .and_then(|n| n.checked_sub(1))
                .ok_or("Enter an insertion frame from 1 onward")?;
            let incoming = clip.cells.iter().map(|c| c.pixels.len() * 4).sum::<usize>()
                + clip.audio.as_ref().map_or(0, |a| a.samples.len() * 2);
            let available = DEFAULT_DOCUMENT_BUDGET
                .saturating_sub(self.documents.iter().map(Document::resident_bytes).sum());
            if incoming
                .saturating_mul(3)
                .saturating_add(self.documents[n].memory_bytes().saturating_mul(2))
                > available
            {
                return Err("The insertion exceeds the workspace memory budget".into());
            }
            let mut state = clip.prepare_insert(&self.documents[n], start)?;
            super::layer_defaults::apply(
                &self.documents[n],
                &mut state,
                &self.workspaces.store.current.layer_defaults,
                DEFAULT_DOCUMENT_BUDGET.saturating_sub(available),
            )?;
            self.documents[n].commit_state(state)?;
        } else {
            if self.tabs.len() >= 256 {
                return Err("View limit reached".into());
            }
            let budget = DEFAULT_DOCUMENT_BUDGET
                .saturating_sub(self.documents.iter().map(Document::resident_bytes).sum());
            let doc = clip.document(budget)?;
            self.documents.try_reserve(1).map_err(|e| e.to_string())?;
            self.tabs.try_reserve(1).map_err(|e| e.to_string())?;
            self.add_document(doc);
        }
        self.protected_sources.insert(original);
        self.movie_dialog = None;
        Ok(())
    }
    pub(super) fn show_movie(&mut self, ctx: &egui::Context) {
        let budget = DEFAULT_DOCUMENT_BUDGET
            .saturating_sub(self.documents.iter().map(Document::resident_bytes).sum());
        let target = self
            .movie_dialog
            .as_ref()
            .and_then(|dialog| dialog.document)
            .and_then(|key| self.documents.iter().find(|d| d.id == key.0))
            .and_then(|doc| doc.animation.as_ref().map(|a| (a.fps, a.frames)));
        let Some(dialog) = &mut self.movie_dialog else {
            return;
        };
        dialog.poll();
        let before = dialog.options().ok();
        let mut cancel = false;
        let mut accept = false;
        let mut decode = false;
        let mut choose = false;
        let mut switch_screen = false;
        egui::Modal::new(egui::Id::new("movie-import")).show(ctx,|ui| {
            let switch = super::screen_mode::preview_switch(ui, self.simple_view);
            #[cfg(test)] { self.screen_button = Some(switch.rect); }
            switch_screen = switch.clicked();

            ui.set_max_width(650.0);ui.heading(if dialog.insert {"Insert local movie into timeline"}else {"New animation from local movie"});
            ui.label(dialog.source.path.display().to_string());
            if let Some(decoder)=&dialog.decoder {ui.label(format!("Local codec: {}",decoder.ffmpeg.display()));}
            if crate::help::button(ui,"Choose local FFmpeg…","movie").clicked() {choose=true;}
            if let Some(info)=&dialog.info {
                ui.label(format!("{} × {} · source FPS {} · {} decoded source frames · last time {:.6}s",info.width,info.height,info.fps,info.times.len(),info.last_time()));
                egui::Grid::new("movie-options").show(ui,|ui| {
                    for (name,value) in [("Start seconds",&mut dialog.start),("End seconds (inclusive)",&mut dialog.end),("Output width",&mut dialog.width),("Output height",&mut dialog.height),("Sampling FPS numerator",&mut dialog.fps),("FPS denominator",&mut dialog.denominator)] {ui.label(name);let seconds=name.contains("seconds");let max=if name.contains("width")||name.contains("height"){32768.0}else if seconds{1e9}else{100000.0};super::edit::finite(ui,value,if seconds{0.0}else{1.0},max,!seconds,"movie");ui.end_row();}
                    if dialog.insert {ui.label("Insertion frame");super::edit::finite(ui,&mut dialog.start_frame,1.0,u32::MAX as f64,true,"movie");ui.end_row();}
                });

                egui::ComboBox::from_id_salt("movie-interpolation").selected_text(dialog.filter.label()).show_ui(ui,|ui|{for filter in drawer_core::interpolation::Filter::ALL {ui.selectable_value(&mut dialog.filter,filter,filter.label());}});
                ui.add_enabled_ui(info.has_audio,|ui|{crate::help::checkbox(ui,&mut dialog.audio,"Import audio separately (48 kHz PCM)","movie");});
                if !info.has_audio {ui.label("This movie has no audio stream.");}
                if ui.add_enabled(dialog.job.is_none(),egui::Button::new("Decode range and preview")).clicked() {decode=true;}
            }
            if let Some(clip)=&dialog.clip {
                let previous=dialog.preview_frame;let mut shown=dialog.preview_frame+1;super::edit::slider(ui,&mut shown,1..=clip.cells.len(),"Preview cell","movie");dialog.preview_frame=shown-1;
                if previous!=dialog.preview_frame {dialog.preview=None;}
                let cell=&clip.cells[dialog.preview_frame];
                if dialog.preview.is_none() {
                    let reduction=cell.width.max(cell.height).div_ceil(2048).max(1);
                    let width=cell.width.div_ceil(reduction);let height=cell.height.div_ceil(reduction);
                    let mut pixels=Vec::with_capacity((width*height) as usize);
                    for y in 0..height {for x in 0..width {let p=cell.pixels[((y*reduction)*cell.width+x*reduction) as usize];pixels.push(egui::Color32::from_rgba_unmultiplied(p[0],p[1],p[2],p[3]));}}
                    dialog.preview=Some(ctx.load_texture("movie-cell-preview",egui::ColorImage::new([width as usize,height as usize],pixels),egui::TextureOptions::NEAREST));
                }
                let scale=(300.0/cell.width as f32).min(150.0/cell.height as f32).min(1.0);ui.add(egui::Image::new(dialog.preview.as_ref().unwrap()).fit_to_exact_size(egui::vec2(cell.width as f32*scale,cell.height as f32*scale)));
                let start=dialog.start_frame.parse::<u32>().ok().and_then(|n|n.checked_sub(1)).unwrap_or(u32::MAX);let fps=if dialog.insert {target.map_or(clip.fps,|t|t.0)}else {clip.fps};
                match clip.mapping(fps,if dialog.insert {start}else {0}) {
                    Ok(mapping)=>{
                        ui.label(format!("{} samples → {} timeline frames at {}/{} FPS",clip.cells.len(),mapping.len(),fps.numerator,fps.denominator));
                        if dialog.insert&&clip.fps!=fps {ui.label("Frame rates differ. Review the time mapping below before inserting.");}
                        if dialog.insert&&target.is_some_and(|t|mapping.last().unwrap().frame>=t.1) {ui.colored_label(egui::Color32::LIGHT_RED,"The insertion exceeds the timeline.");}
                        egui::ScrollArea::vertical().scroll_bar_visibility(crate::ui_behavior::scroll(ui.ctx())).max_height(100.0).show_rows(ui,ui.text_style_height(&egui::TextStyle::Body),mapping.len(),|ui,rows|{for index in rows {let row=mapping[index];ui.label(format!("Frame {} ← cell {} · source {:.6}s",row.frame+1,row.cell+1,clip.source_times[row.cell]));}});
                    },Err(e)=>{ui.colored_label(egui::Color32::LIGHT_RED,e);}
                }
            }
            if dialog.job.is_some() {ui.spinner();ui.label("Reading local movie… Cancel stops decoding.");ctx.request_repaint();}
            if let Some(error)=&dialog.error {ui.colored_label(egui::Color32::LIGHT_RED,error);}
            ui.horizontal(|ui| {let a=ui.add_enabled(dialog.clip.is_some()&&dialog.job.is_none(),egui::Button::new(if dialog.insert {"Insert track"}else {"Create animation"}));accept=a.clicked();let c=crate::help::button(ui,"Cancel","movie");cancel=c.clicked();#[cfg(test)] {dialog.accept_button=Some(a.rect);dialog.cancel_button=Some(c.rect);}});
        });
        if switch_screen {
            self.simple_view = !self.simple_view;
            self.canvas_view.cancel_input();
        }
        if before != dialog.options().ok() {
            dialog.job = None;
            dialog.clip = None;
            dialog.preview = None;
            dialog.decoded = None;
        }
        if choose
            && let Some(path) = rfd::FileDialog::new()
                .add_filter("FFmpeg executable", &["exe"])
                .pick_file()
        {
            match Decoder::at(path) {
                Ok(decoder) => {
                    dialog.decoder = Some(decoder);
                    dialog.job(ctx, budget, false);
                }
                Err(e) => dialog.error = Some(e),
            }
        }
        if decode {
            dialog.job(ctx, budget, true);
        }
        if accept && let Err(e) = self.accept_movie() {
            self.movie_dialog.as_mut().unwrap().error = Some(e);
        }
        if cancel || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.movie_dialog = None;
            ctx.request_repaint();
        }
    }
    pub(super) fn movie_controls(&mut self, ui: &mut egui::Ui) {
        ui.menu_button("Movie", |ui| {
            for (label, insert) in [
                ("New animation from movie…", false),
                ("Insert movie into timeline…", true),
            ] {
                if ui
                    .add_enabled(
                        !insert
                            || self
                                .active
                                .is_some_and(|n| self.documents[n].animation.is_some()),
                        egui::Button::new(label),
                    )
                    .clicked()
                    && let Some(path) = rfd::FileDialog::new()
                        .add_filter("Local movie", &["mp4", "mov", "mkv", "webm", "avi"])
                        .pick_file()
                {
                    if let Err(e) = self.begin_movie(ui.ctx(), &path, insert) {
                        self.status = Some(e);
                    }
                    ui.close();
                }
            }
        });
        if let Some(n) = self.active
            && let Some(animation) = &self.documents[n].animation
        {
            let total = animation.frames;
            let fps = animation.fps;
            let tracks = animation.tracks.len();
            let audio = animation.audio.len();
            let mut frame = self.documents[n].current_frame + 1;
            ui.horizontal(|ui| {
                ui.label(format!(
                    "Animation · {}/{} FPS · {tracks} cell tracks · {audio} audio tracks",
                    fps.numerator, fps.denominator
                ));
                if super::edit::slider(ui, &mut frame, 1..=total, "Frame", "movie").changed() {
                    self.documents[n].current_frame = frame - 1;
                }
            });
        }
    }
}
