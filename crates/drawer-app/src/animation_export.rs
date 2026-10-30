use super::*;
use drawer_core::animation_export::{self as core, Format, Options, Snapshot};
use std::{
    path::Path,
    sync::{
        atomic::{AtomicBool, AtomicU32, Ordering},
        mpsc,
    },
};
pub(super) struct Panel {
    open: bool,
    external: Option<Box<Document>>,
    external_root: Option<PathBuf>,
    path: String,
    options: Options,
    overwrite: bool,
    ack: bool,
    snapshot: Option<Snapshot>,
    source: String,
    job: Option<Job>,
    notice: String,
    watermark: String,
    encoder_path: String,
    encoder: Option<drawer_core::movie_export::Verified>,
    encoder_job: Option<EncoderJob>,
    preview: Option<egui::TextureHandle>,
    preview_job: Option<PreviewJob>,
    formats: Vec<(Format, egui::Rect)>,
    camera: Option<egui::Rect>,
    discard: Option<egui::Rect>,
    pub prepare: Option<egui::Rect>,
    pub start: Option<egui::Rect>,
}
impl Default for Panel {
    fn default() -> Self {
        Self {
            open: false,
            external: None,
            external_root: None,
            path: String::new(),
            options: Options {
                first: 0,
                last: 0,
                format: Format::PngSequence,
                plays: 0,
                ..Default::default()
            },
            overwrite: false,
            ack: false,
            snapshot: None,
            source: String::new(),
            job: None,
            notice: String::new(),
            watermark: String::new(),
            encoder_path: drawer_core::movie_export::discover()
                .map(|d| d.ffmpeg.to_string_lossy().into_owned())
                .unwrap_or_default(),
            encoder: None,
            encoder_job: None,
            preview: None,
            preview_job: None,
            formats: vec![],
            camera: None,
            discard: None,
            prepare: None,
            start: None,
        }
    }
}
struct Job {
    cancel: Arc<AtomicBool>,
    progress: Arc<AtomicU32>,
    total: u32,
    receiver: mpsc::Receiver<core::Outcome>,
}
impl Drop for Job {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}
fn thumbnail(w: u32, h: u32, pixels: &[u8]) -> (u32, u32, Vec<u8>) {
    let scale = (w.max(h) as f64 / 512.).max(1.);
    let width = (w as f64 / scale).round().max(1.) as u32;
    let height = (h as f64 / scale).round().max(1.) as u32;
    let mut out = Vec::with_capacity(width as usize * height as usize * 4);
    for y in 0..height {
        for x in 0..width {
            let sx = ((x as u64 * w as u64) / width as u64) as usize;
            let sy = ((y as u64 * h as u64) / height as u64) as usize;
            let n = (sy * w as usize + sx) * 4;
            out.extend_from_slice(&pixels[n..n + 4]);
        }
    }
    (width, height, out)
}
struct EncoderJob {
    cancel: Arc<AtomicBool>,
    receiver: mpsc::Receiver<Result<drawer_core::movie_export::Verified, String>>,
}
impl Drop for EncoderJob {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}
type PreviewFrame = (u32, u32, Vec<u8>);
struct PreviewJob {
    cancel: Arc<AtomicBool>,
    receiver: mpsc::Receiver<Result<PreviewFrame, String>>,
}
impl Drop for PreviewJob {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}
impl Drawer {
    pub(super) fn begin_recorded_animation_export(
        &mut self,
        document: Document,
        root: PathBuf,
    ) -> Result<(), String> {
        if self.animation_export.job.is_some() {
            return Err("Finish or cancel the current animation output first".into());
        }
        let p = &mut self.animation_export;
        p.options.first = 0;
        p.options.last = document
            .animation
            .as_ref()
            .ok_or("Recording output timeline missing")?
            .frames
            - 1;
        p.external = Some(Box::new(document));
        p.external_root = Some(root);
        p.snapshot = None;
        p.preview = None;
        p.preview_job = None;
        p.path.clear();
        p.options.audio = false;
        p.ack = false;
        p.open = true;
        p.notice="Frozen committed timelapse frames. Later recording and drawing changes will not affect this output. Choose format/range and capture; use an output outside the recording folder.".into();
        Ok(())
    }
    pub(super) fn begin_animation_export(&mut self) -> Result<(), String> {
        self.finish_stroke();
        if self.transform_unconfirmed()
            || self.inference_unconfirmed()
            || self.text_editor.draft.is_some()
        {
            return Err("Finish pending drawing edits before capturing output".into());
        }
        let d = &self.documents[self.active.ok_or("Choose an animation")?];
        let a = d.animation.as_ref().ok_or("Choose an animation document")?;
        if self.animation_export.job.is_some() {
            self.animation_export.open = true;
            return Ok(());
        }
        self.animation_export.external = None;
        self.animation_export.external_root = None;
        self.animation_export.options.first = 0;
        self.animation_export.options.last = a.frames - 1;
        if let Some(area) = d.scene.animation_output.settings.area {
            self.animation_export.options.view.area = area;
            if let Some(range) = d.scene.timing.playback {
                self.animation_export.options.first = range.first;
                self.animation_export.options.last = range.end - 1;
            }
        }
        self.animation_export.snapshot = None;
        self.animation_export.preview = None;
        self.animation_export.preview_job = None;
        self.animation_export.ack = false;
        self.animation_export.open = true;
        Ok(())
    }
    fn prepare_animation_export(&mut self) -> Result<(), String> {
        if self.animation_export.external.is_none() {
            self.finish_stroke();
            if self.transform_unconfirmed()
                || self.inference_unconfirmed()
                || self.text_editor.draft.is_some()
            {
                return Err("Finish pending drawing edits first".into());
            }
        }
        if self.animation_export.job.is_some() {
            return Err("Wait for or cancel the current output".into());
        }
        let d = if let Some(d) = &self.animation_export.external {
            d.as_ref()
        } else {
            &self.documents[self.active.ok_or("Choose an animation")?]
        };
        let budget = DEFAULT_DOCUMENT_BUDGET.saturating_sub(
            self.documents
                .iter()
                .map(Document::resident_bytes)
                .sum::<usize>(),
        );
        let snapshot = core::prepare_with(
            d,
            self.animation_export.options,
            budget,
            if self.animation_export.watermark.trim().is_empty() {
                None
            } else {
                Some(Path::new(&self.animation_export.watermark))
            },
            self.animation_export.encoder.as_ref(),
        )?;
        let source = d.title.clone();
        let p = &mut self.animation_export;
        p.snapshot = Some(snapshot);
        let mut captured = p.snapshot.as_ref().unwrap().clone();
        // The display preview converts embedded working profiles to display sRGB.
        captured.options.view.icc = false;
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        let (tx, receiver) = mpsc::channel();
        p.preview = None;
        std::thread::spawn(move || {
            let size = captured.layout;
            let result =
                drawer_core::animation_render::frame(&captured, captured.options.first, &flag)
                    .map(|b| thumbnail(size.width, size.height, &b));
            let _ = tx.send(result);
        });
        p.preview_job = Some(PreviewJob { cancel, receiver });
        self.animation_export.source = source;
        self.animation_export.ack = false;
        self.animation_export.notice =
            "Snapshot captured. Later edits and playback changes will not affect this output."
                .into();
        Ok(())
    }
    fn start_animation_export(&mut self, ctx: &egui::Context) -> Result<(), String> {
        if self.animation_export.job.is_some() || !self.animation_export.ack {
            return Err("Review and confirm RGBA8 output before starting".into());
        }
        if self.animation_export.path.trim().is_empty() {
            return Err("Choose a local output filename or new sequence folder".into());
        }
        if let Some(root) = &self.animation_export.external_root {
            let path = Path::new(self.animation_export.path.trim());
            let parent = path
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or(Path::new("."))
                .canonicalize()
                .map_err(|e| e.to_string())?;
            if parent.starts_with(root) {
                return Err("Choose an output outside the recording folder to preserve its journal and frames".into());
            }
        }
        let snapshot = self
            .animation_export
            .snapshot
            .take()
            .ok_or("Capture an output snapshot first")?;
        let total = snapshot.count();
        let destination = PathBuf::from(&self.animation_export.path);
        let overwrite = self.animation_export.overwrite;
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        let progress = Arc::new(AtomicU32::new(0));
        let progress_out = progress.clone();
        let (tx, receiver) = mpsc::channel();
        let repaint = ctx.clone();
        std::thread::spawn(move || {
            let result = core::publish(&snapshot, &destination, overwrite, &flag, &progress_out);
            let _ = tx.send(result);
            repaint.request_repaint();
        });
        self.animation_export.job = Some(Job {
            cancel,
            progress,
            total,
            receiver,
        });
        Ok(())
    }
    fn verify_animation_encoder(&mut self) -> Result<(), String> {
        let decoder =
            drawer_core::movie::Decoder::at(PathBuf::from(&self.animation_export.encoder_path))?;
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        let (tx, receiver) = mpsc::channel();
        self.animation_export.encoder = None;
        self.animation_export.snapshot = None;
        self.animation_export.ack = false;
        std::thread::spawn(move || {
            let _ = tx.send(drawer_core::movie_export::verify(decoder, &flag));
        });
        self.animation_export.encoder_job = Some(EncoderJob { cancel, receiver });
        Ok(())
    }
    fn animation_preset(&mut self, load: bool) -> Result<(), String> {
        let dialog = rfd::FileDialog::new().add_filter("Drawer output preset", &["json"]);
        if load {
            if let Some(path) = dialog.pick_file() {
                if !drawer_core::font_library::local(&path) {
                    return Err("Choose a local output preset".into());
                }
                let m = std::fs::metadata(&path).map_err(|e| e.to_string())?;
                if m.len() > 65536 {
                    return Err("Output preset exceeds64KiB".into());
                }
                let p = core::Preset::decode(&std::fs::read(path).map_err(|e| e.to_string())?)?;
                self.animation_export.options = p.options;
                self.animation_export.watermark = p
                    .watermark
                    .map(|p| p.to_string_lossy().into_owned())
                    .unwrap_or_default();
                self.animation_export.encoder_path = p
                    .encoder
                    .map(|p| p.to_string_lossy().into_owned())
                    .unwrap_or_default();
                self.animation_export.encoder = None;
                self.animation_export.encoder_job = None;
                self.animation_export.snapshot = None;
                self.animation_export.preview = None;
                self.animation_export.preview_job = None;
                self.animation_export.ack = false;
                self.animation_export.notice="Loaded local output settings. Encoder verification and output confirmation must be repeated.".into();
            }
        } else if let Some(path) = dialog.set_file_name("drawer-output.json").save_file() {
            let p = &self.animation_export;
            let preset = core::Preset {
                version: 1,
                options: p.options,
                watermark: (!p.watermark.trim().is_empty()).then(|| PathBuf::from(&p.watermark)),
                encoder: (!p.encoder_path.is_empty()).then(|| PathBuf::from(&p.encoder_path)),
            };
            let bytes = preset.encode()?;
            drawer_core::animation_export::save_preset(&path, &bytes)?;
            self.animation_export.notice = format!("Saved local preset: {}", path.display());
        }
        Ok(())
    }
    pub(super) fn animation_export_window(&mut self, ctx: &egui::Context) {
        if let Some(job) = &self.animation_export.job {
            let result = match job.receiver.try_recv() {
                Ok(r) => Some(r),
                Err(mpsc::TryRecvError::Empty) => None,
                Err(mpsc::TryRecvError::Disconnected) => Some(core::Outcome {
                    files: vec![],
                    frames: 0,
                    error: Some("Animation worker stopped".into()),
                    log: None,
                }),
            };
            if let Some(r) = result {
                self.animation_export.job = None;
                self.animation_export.notice = format!(
                    "{} · {} completed frames · {} files. {} Receipt: {}",
                    r.error.as_ref().map_or("Completed", |_| "Stopped"),
                    r.frames,
                    r.files.len(),
                    r.error.unwrap_or_else(|| r
                        .files
                        .iter()
                        .take(4)
                        .map(|p| p.display().to_string())
                        .collect::<Vec<_>>()
                        .join("; ")),
                    r.log
                        .map(|p| p.display().to_string())
                        .unwrap_or_else(|| "unavailable".into())
                );
            } else {
                ctx.request_repaint_after(std::time::Duration::from_millis(40));
            }
        }
        if let Some(job) = &self.animation_export.encoder_job {
            if let Ok(result) = job.receiver.try_recv() {
                self.animation_export.encoder_job = None;
                match result {
                    Ok(v) => {
                        self.animation_export.notice = format!(
                            "Verified local encoder: {}",
                            v.codecs
                                .iter()
                                .map(|c| c.label())
                                .collect::<Vec<_>>()
                                .join("; ")
                        );
                        self.animation_export.encoder = Some(v);
                    }
                    Err(e) => self.animation_export.notice = e,
                }
            } else {
                ctx.request_repaint_after(std::time::Duration::from_millis(40));
            }
        }
        if let Some(job) = &self.animation_export.preview_job {
            if let Ok(result) = job.receiver.try_recv() {
                self.animation_export.preview_job = None;
                match result {
                    Ok((w, h, b)) => {
                        let full =
                            egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &b);
                        self.animation_export.preview = Some(ctx.load_texture(
                            "Captured output preview",
                            full,
                            egui::TextureOptions::LINEAR,
                        ));
                    }
                    Err(e) => self.animation_export.notice = format!("Preview: {e}"),
                }
            } else {
                ctx.request_repaint_after(std::time::Duration::from_millis(40));
            }
        }
        if !self.animation_export.open {
            return;
        }
        let mut p = std::mem::take(&mut self.animation_export);
        let mut open = true;
        let mut action = 0;
        egui::Window::new("Animation file output").default_pos(egui::pos2(15.,15.)).default_width(650.).open(&mut open).vscroll(true).show(ctx,|ui|{
            ui.label(&p.notice);
            ui.add_enabled_ui(p.job.is_none(),|ui|{
                p.formats.clear();p.camera=None;p.prepare=None;p.start=None;p.discard=None;
                if p.snapshot.is_none(){
                let before=p.options;let old_watermark=p.watermark.clone();
                ui.horizontal_wrapped(|ui|{for(format,label)in[(Format::PngSequence,"PNG sequence"),(Format::Apng,"APNG"),(Format::Gif,"GIF"),(Format::Webp,"Animated WebP"),(Format::Movie,"Local movie")]{let r=ui.selectable_value(&mut p.options.format,format,label);p.formats.push((format,r.rect));}});
                ui.horizontal(|ui|{let mut first=p.options.first+1;let mut last=p.options.last+1;ui.label("Inclusive first / last frame");ui.add(egui::DragValue::new(&mut first).range(1..=10000));ui.add(egui::DragValue::new(&mut last).range(1..=10000));p.options.first=first-1;p.options.last=last-1;});
                if p.options.format!=Format::PngSequence && p.options.format!=Format::Movie {ui.horizontal(|ui|{ui.label("Complete plays (0 = infinite)");ui.add(egui::DragValue::new(&mut p.options.plays).range(0..=100000));});}
                ui.horizontal_wrapped(|ui|{ui.label("Output area");use drawer_core::animation_render::Area;for(a,label)in[(Area::Canvas,"Full canvas"),(Area::OutputFrame,"Output frames (union)"),(Area::Overflow,"Canvas + overflow (union)"),(Area::Selection,"Selection")]{ui.selectable_value(&mut p.options.view.area,a,label);}});
                ui.horizontal_wrapped(|ui|{let r=ui.checkbox(&mut p.options.view.camera,"Apply 2D camera");p.camera=Some(r.rect);ui.checkbox(&mut p.options.view.drafts,"Include drafts");ui.checkbox(&mut p.options.view.frame_lines,"Include frame lines / print guides");ui.checkbox(&mut p.options.view.shadows,"Include live drop shadows");ui.checkbox(&mut p.options.view.transparent,"Transparent background (hide paper)");ui.checkbox(&mut p.options.view.icc,"Embed ICC where supported");});
                ui.horizontal(|ui|{ui.label("Opaque matte / H.264 background");ui.color_edit_button_srgb(&mut p.options.view.matte);});
                ui.collapsing("Captured local watermark / coordinate-stable noise",|ui|{
                    ui.horizontal(|ui|{ui.label("Static PNG");ui.text_edit_singleline(&mut p.watermark);if ui.button("Choose PNG").clicked() && let Some(path)=rfd::FileDialog::new().add_filter("PNG",&["png"]).pick_file(){p.watermark=path.to_string_lossy().into_owned();}});
                    ui.horizontal(|ui|{ui.label("Watermark opacity / output x / y");ui.add(egui::DragValue::new(&mut p.options.view.watermark_opacity).range(0..=255));for x in &mut p.options.view.watermark_origin{ui.add(egui::DragValue::new(x).range(-32768..=32768));}});
                    ui.horizontal(|ui|{ui.label("Noise opacity / seed (same pattern in every frame)");ui.add(egui::DragValue::new(&mut p.options.view.noise).range(0..=255));ui.add(egui::DragValue::new(&mut p.options.view.seed));});
                });
                if p.options.format==Format::Movie {
                    ui.label("No bundled encoder or downloads. Without a verified encoder, use PNG sequence plus Timeline audio → WAV output.");
                    let old=p.encoder_path.clone();
                    ui.horizontal(|ui|{ui.text_edit_singleline(&mut p.encoder_path);if ui.button("Choose FFmpeg.exe").clicked() && let Some(path)=rfd::FileDialog::new().add_filter("Executable",&["exe"]).pick_file(){p.encoder_path=path.to_string_lossy().into_owned();}});
                    if old!=p.encoder_path{p.encoder=None;p.encoder_job=None;p.snapshot=None;p.ack=false;}
                    if ui.add_enabled(p.encoder_job.is_none(),egui::Button::new("Verify installed encoder by encode + decode test")).clicked(){action=3;}
                    if p.encoder_job.is_some(){ui.label("Testing local encoder…");if ui.button("Cancel encoder test").clicked(){p.encoder_job=None;}}
                    if let Some(v)=&p.encoder {ui.horizontal_wrapped(|ui|{for c in &v.codecs {ui.selectable_value(&mut p.options.codec,*c,c.label());}});}
                    ui.checkbox(&mut p.options.audio,"Include captured 48kHz stereo mix");
                }
                ui.horizontal(|ui|{if ui.button("Save output preset…").clicked(){action=4;}
                if ui.button("Load output preset…").clicked(){action=5;}});
                if before!=p.options||old_watermark!=p.watermark{p.snapshot=None;p.preview=None;p.preview_job=None;p.ack=false;}
                ui.label(if p.options.format==Format::PngSequence{"New local sequence folder (completed files remain on cancel)"}else{"Local output filename (.apng / .gif / .webp / .mkv / .mp4)"});
                ui.text_edit_singleline(&mut p.path);
                if p.options.format!=Format::PngSequence{ui.checkbox(&mut p.overwrite,"Allow replacement of the existing output file");}
                let r=ui.button("Capture frame range for output");p.prepare=Some(r.rect);if r.clicked(){action=1;}
                }
                if let Some(s)=&p.snapshot {
                    let fps=s.fps();
                    ui.label(format!("Captured {} · revision{} · {}×{} · origin {:?} · {} frames ({}–{}) · {}/{} FPS · camera {}",p.source,s.revision,s.layout.width,s.layout.height,s.layout.origin,s.count(),s.options.first+1,s.options.last+1,fps.numerator,fps.denominator,s.options.view.camera));
                    ui.label(s.warnings());
                    let r=ui.button("Discard captured snapshot / change settings");p.discard=Some(r.rect);if r.clicked(){action=6;}
                    if let Some(image)=&p.preview {ui.add(egui::Image::new(image).max_size(egui::vec2(420.,220.)));ui.label("Captured first frame; transparency uses the panel background. Camera / include policy above is already applied.");}
                    if p.preview_job.is_some(){ui.label("Preparing captured first-frame preview…");}
                    ui.label("Destination for this captured job");ui.text_edit_singleline(&mut p.path);
                    if s.options.format!=Format::PngSequence {ui.checkbox(&mut p.overwrite,"Allow replacement of the existing output file");}
                    ui.checkbox(&mut p.ack,"I reviewed the output preview, format losses, timing and replacement policy");
                    let r=ui.add_enabled(p.ack && p.preview.is_some(),egui::Button::new("Export captured animation"));p.start=Some(r.rect);if r.clicked(){action=2;}
                }
            });
            if let Some(job)=&p.job {
                ui.label(format!("Rendered {} / {} frames. Completed PNG files remain; incomplete private files are removed. Single-file animations publish after verification.",job.progress.load(Ordering::Relaxed),job.total));
                if ui.button("Cancel output").clicked(){job.cancel.store(true,Ordering::Relaxed);}
            }
        });
        p.open = open;
        if !open {
            p.external = None;
            p.external_root = None;
            p.snapshot = None;
            p.preview = None;
            p.preview_job = None;
            p.encoder_job = None;
            if let Some(job) = &p.job {
                job.cancel.store(true, Ordering::Relaxed);
            }
        }
        self.animation_export = p;
        let result = match action {
            1 => self.prepare_animation_export(),
            2 => self.start_animation_export(ctx),
            3 => self.verify_animation_encoder(),
            4 => self.animation_preset(false),
            5 => self.animation_preset(true),
            6 => {
                self.animation_export.snapshot = None;
                self.animation_export.preview = None;
                self.animation_export.preview_job = None;
                self.animation_export.ack = false;
                Ok(())
            }
            _ => Ok(()),
        };
        if let Err(e) = result {
            self.animation_export.notice = e;
        }
    }
}
pub(super) fn smoke(root: &Path) -> Result<(), String> {
    drawer_core::animation_export_verify::verify(root)?;
    drawer_core::animation_delivery_verify::verify(root)?;
    let dir = root.join("animation-export-ui");
    std::fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let result = (|| {
        let ctx = egui::Context::default();
        let mut app = Drawer::new(&ctx, None);
        app.simple_view = true;
        app.add_document(drawer_core::animation_export_verify::fixture());
        app.execute_command(&ctx, "animation-export");
        if !app.animation_export.open {
            return Err("Animation output command did not open".into());
        }
        app.animation_export.options = Options {
            first: 1,
            last: 2,
            format: Format::Apng,
            plays: 1,
            ..Default::default()
        };
        let path = dir.join("검증.apng");
        app.animation_export.path = path.to_string_lossy().into_owned();
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
        let r = app
            .animation_export
            .prepare
            .ok_or("Capture control missing")?;
        click(&mut app, &mut time, r);
        if app.animation_export.snapshot.is_none() || app.start_animation_export(&ctx).is_ok() {
            return Err("Capture pointer failed or output bypassed review".into());
        }
        app.documents[0].edit_pixel(0, 1, 0, [255, 200, 0, 255]);
        app.documents[0].current_frame = 0;
        let before = app.documents[0].snapshot();
        let history = app.documents[0].history_counts();
        let revision = app.documents[0].revision;
        let dirty = app.documents[0].dirty;
        let end = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while app.animation_export.preview_job.is_some() {
            frame(&mut app, &mut time, vec![]);
            if std::time::Instant::now() > end {
                return Err("Preview timeout".into());
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        app.animation_export.ack = true;
        for _ in 0..3 {
            frame(&mut app, &mut time, vec![]);
        }
        let r = app.animation_export.start.ok_or("Export control missing")?;
        click(&mut app, &mut time, r);
        let wait = |app: &mut Drawer, time: &mut f64| -> Result<(), String> {
            let end = std::time::Instant::now() + std::time::Duration::from_secs(30);
            while app.animation_export.job.is_some() {
                frame(app, time, vec![]);
                if std::time::Instant::now() > end {
                    return Err("Animation output timeout".into());
                }
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            Ok(())
        };
        wait(&mut app, &mut time)?;
        if !path.is_file() {
            return Err(format!(
                "Pointer output failed: {}",
                app.animation_export.notice
            ));
        }
        let mut reader = png_decoder(&path)?;
        let mut bytes = vec![0; reader.output_buffer_size().unwrap()];
        reader.next_frame(&mut bytes).map_err(|e| e.to_string())?;
        if reader.info().animation_control.unwrap().num_frames != 2 || bytes[7] != 64 {
            return Err(
                "Output followed the changed live drawing or lost the inclusive range".into(),
            );
        }
        drop(reader);
        if app.documents[0].snapshot() != before
            || app.documents[0].history_counts() != history
            || app.documents[0].revision != revision
            || app.documents[0].current_frame != 0
            || app.documents[0].dirty != dirty
        {
            return Err("Animation output changed the live document/view/history".into());
        }
        app.documents[0].animation.as_mut().unwrap().frames = 10000;
        app.animation_export.options.last = 9999;
        app.animation_export.path = dir.join("cancel.apng").to_string_lossy().into_owned();
        app.prepare_animation_export()?;
        app.animation_export.ack = true;
        app.start_animation_export(&ctx)?;
        app.animation_export
            .job
            .as_ref()
            .unwrap()
            .cancel
            .store(true, Ordering::Relaxed);
        wait(&mut app, &mut time)?;
        if dir.join("cancel.apng").exists()
            || std::fs::read_dir(&dir).unwrap().any(|e| {
                e.unwrap()
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".Drawer-animation")
            })
        {
            return Err("Cancelled output left published/private files".into());
        }
        Ok(())
    })();
    let cleanup = std::fs::remove_dir_all(&dir).map_err(|e| e.to_string());
    result.and(cleanup)
}
fn png_decoder(path: &Path) -> Result<drawer_core::animation_export_verify::PngReader, String> {
    drawer_core::animation_export_verify::reader(path)
}
/// Installed smoke bridge exercises the external committed-record source without switching canvases.
pub(super) fn smoke_recorded_source(
    app: &mut Drawer,
    ctx: &egui::Context,
    root: &Path,
) -> Result<u32, String> {
    let source = app.animation_export.external_root.clone().ok_or_else(|| {
        format!(
            "Timelapse did not prepare frozen output: {}",
            app.timelapse.notice
        )
    })?;
    let frames = app
        .animation_export
        .external
        .as_ref()
        .unwrap()
        .animation
        .as_ref()
        .unwrap()
        .frames;
    app.animation_export.options.format = Format::Apng;
    app.prepare_animation_export()?;
    app.animation_export.ack = true;
    app.animation_export.path = source.join("preserve.apng").to_string_lossy().into_owned();
    if app.start_animation_export(ctx).is_ok() {
        return Err("Recording source folder overwrite guard failed".into());
    }
    app.animation_export.options.format = Format::PngSequence;
    app.prepare_animation_export()?;
    app.animation_export.ack = true;
    let folder = root.join("recorded-output");
    app.animation_export.path = folder.to_string_lossy().into_owned();
    app.start_animation_export(ctx)?;
    let until = std::time::Instant::now() + std::time::Duration::from_secs(30);
    let mut t = 5000.;
    while app.animation_export.job.is_some() {
        t += 0.05;
        input_replay::frame(ctx, app, t, vec![]);
        if std::time::Instant::now() > until {
            return Err("Recorded frame output timeout".into());
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    if !folder.join("animation.json").is_file() {
        return Err(format!(
            "Recorded output failed: {}",
            app.animation_export.notice
        ));
    }
    Ok(frames)
}

#[cfg(test)]
mod tests {
    #[test]
    fn pointer_range_capture_review_export_live_edits_and_cancel_are_isolated() {
        let root =
            std::env::temp_dir().join(format!("drawer-animation-output-ui-{}", std::process::id()));
        std::fs::create_dir(&root).unwrap();
        let r = super::smoke(&root);
        std::fs::remove_dir(&root).unwrap();
        r.unwrap();
    }
    #[test]
    fn format_camera_pointer_changes_discard_stale_preview_and_leave_drawing_untouched() {
        use super::*;
        let ctx = egui::Context::default();
        let mut app = Drawer::new(&ctx, None);
        app.simple_view = true;
        app.add_document(drawer_core::animation_export_verify::fixture());
        app.begin_animation_export().unwrap();
        let before = app.documents[0].snapshot();
        let history = app.documents[0].history_counts();
        let revision = app.documents[0].revision;
        let dirty = app.documents[0].dirty;
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
        let gif = app
            .animation_export
            .formats
            .iter()
            .find(|(f, _)| *f == Format::Gif)
            .unwrap()
            .1;
        click(&mut app, &mut time, gif);
        assert_eq!(app.animation_export.options.format, Format::Gif);
        app.prepare_animation_export().unwrap();
        for _ in 0..3 {
            frame(&mut app, &mut time, vec![]);
        }
        assert!(app.animation_export.formats.is_empty());
        assert!(app.animation_export.camera.is_none());
        // Preview arrival must not move the cancel target between pointer down/up.
        app.animation_export.preview_job = None;
        app.animation_export.preview = None;
        for _ in 0..3 {
            frame(&mut app, &mut time, vec![]);
        }
        let discard = app.animation_export.discard.unwrap();
        app.animation_export.preview = Some(ctx.load_texture(
            "preview-arrival-test",
            egui::ColorImage::from_rgba_unmultiplied([64, 64], &vec![0; 64 * 64 * 4]),
            egui::TextureOptions::NEAREST,
        ));
        for _ in 0..3 {
            frame(&mut app, &mut time, vec![]);
        }
        assert_eq!(app.animation_export.discard, Some(discard));
        click(&mut app, &mut time, discard);
        assert!(app.animation_export.snapshot.is_none());
        assert!(!app.animation_export.ack);
        assert!(app.animation_export.preview.is_none());
        for _ in 0..3 {
            frame(&mut app, &mut time, vec![]);
        }
        let camera = app.animation_export.camera.unwrap();
        click(&mut app, &mut time, camera);
        assert!(!app.animation_export.options.view.camera);
        assert!(app.animation_export.snapshot.is_none());
        assert!(app.animation_export.preview_job.is_none());
        assert_eq!(app.documents[0].snapshot(), before);
        assert_eq!(app.documents[0].history_counts(), history);
        assert_eq!(app.documents[0].revision, revision);
        assert_eq!(app.documents[0].dirty, dirty);
        let (w, h, b) = thumbnail(2000, 1000, &vec![255; 2000 * 1000 * 4]);
        assert_eq!((w, h, b.len()), (512, 256, 512 * 256 * 4));
    }
    #[test]
    fn finishing_defaults_set_capture_area_and_frame_range() {
        use super::*;
        let ctx = egui::Context::default();
        let mut app = Drawer::new(&ctx, None);
        let (mut d, _) = drawer_core::animation_finish_verify::fixture().unwrap();
        d.scene.animation_output.settings.area =
            Some(drawer_core::animation_render::Area::Overflow);
        d.scene.timing.playback = Some(drawer_core::timeline_timing::Playback {
            first: 3,
            end: 8,
            repeat: false,
        });
        app.add_document(d);
        app.begin_animation_export().unwrap();
        assert_eq!(app.animation_export.options.first, 3);
        assert_eq!(app.animation_export.options.last, 7);
        assert_eq!(
            app.animation_export.options.view.area,
            drawer_core::animation_render::Area::Overflow
        );
    }
}
