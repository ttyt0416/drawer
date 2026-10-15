use super::*;
use drawer_core::input_replay::{Record, Sequence, checksum};
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    mpsc,
};
pub(super) struct Outcome {
    pub record: Record,
    pub pixels: Vec<[u8; 4]>,
}
pub(super) fn frame(ctx: &egui::Context, app: &mut Drawer, time: f64, events: Vec<egui::Event>) {
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1000.0, 700.0),
            )),
            time: Some(time),
            events,
            ..Default::default()
        },
        |ui| app.show(ui),
    );
    output.textures_delta.clear();
}
/// Runs the production Drawer UI/input pipeline in an isolated CPU-only context.
/// No native window, storage, clipboard or connected pen adapter is attached.
pub(super) fn run(
    sequence: &Sequence,
    cancel: &AtomicBool,
    progress: &AtomicUsize,
) -> Result<Outcome, String> {
    sequence.validate()?;
    if cancel.load(Ordering::Relaxed) {
        return Err("Virtual replay cancelled".into());
    }
    let ctx = egui::Context::default();
    let mut app = Drawer::new(&ctx, None);
    app.create_canvas(CanvasSpec {
        width: sequence.width,
        height: sequence.height,
        dpi: 300.0,
        background: Background::Transparent,
    })
    .map_err(|e| e.to_string())?;
    app.documents[0].layers[0].pixels.fill(sequence.background);
    app.documents[0].mark_saved("virtual-baseline.drawer".into());
    app.simple_view = true;
    app.set_tool(sequence.tool);
    app.panels.settings.brush_size = sequence.size;
    app.panels.settings.brush_aspect = sequence.aspect;
    app.panels.settings.brush_angle = sequence.angle;
    app.panels.settings.brush_opacity = sequence.opacity;
    app.panels.settings.brush_hardness = sequence.hardness;
    app.panels.settings.brush_spacing = sequence.spacing;
    app.workspaces.store.current.input.smoothing = sequence.smoothing;
    app.workspaces.store.current.input.ends = sequence.ends;
    app.workspaces.store.current.input.materials = sequence.materials;
    app.workspaces.store.current.input.textures = sequence.textures;
    app.workspaces.store.current.input.mouse_pressure = sequence.mouse_pressure;
    app.workspaces.store.current.cursors = sequence.cursors;
    app.references.session.workspace.color = sequence.color;
    app.workspaces.store.current.pressure = sequence.pressure;
    app.workspaces.store.current.pointer = sequence.pointer;
    frame(&ctx, &mut app, 0.0, vec![]);
    frame(&ctx, &mut app, 0.01, vec![]);
    let before = app.documents[0].snapshot();
    let mut n = 0;
    let mut frames = 2;
    while n < sequence.samples.len() {
        if cancel.load(Ordering::Relaxed) {
            return Err("Virtual replay cancelled".into());
        }
        let micros = sequence.samples[n].micros;
        let rect = app
            .canvas_view
            .viewport_rect()
            .ok_or("Virtual canvas unavailable")?;
        let mut events = vec![];
        // Keep the recorded time/order while yielding between bounded batches
        // so a long same-time sequence can still be cancelled.
        while n < sequence.samples.len() && sequence.samples[n].micros == micros && events.len() < 8
        {
            let mut event = sequence.samples[n].event;
            event.attributes.time_micros = Some(sequence.samples[n].micros);
            event.position = app.documents[0].view.document_to_screen(
                event.position,
                [sequence.width as f64, sequence.height as f64],
                [rect.center().x as f64, rect.center().y as f64],
                ctx.pixels_per_point() as f64,
            );
            events.push(event);
            n += 1;
        }
        app.drawing
            .virtual_events(events, sequence.pen_backend_available);
        frame(&ctx, &mut app, 1.0 + micros as f64 / 1e6, vec![]);
        frames += 1;
        progress.store(n, Ordering::Relaxed);
    }
    let incomplete_cancelled = app.drawing.pending();
    app.drawing
        .virtual_events(vec![], sequence.pen_backend_available);
    let end = sequence.samples.last().unwrap().micros as f64 / 1e6 + 1.01;
    frame(&ctx, &mut app, end, vec![egui::Event::WindowFocused(false)]);
    frames += 1;
    if app.drawing.pending() {
        return Err("Virtual input retained a contact after cancellation".into());
    }
    let doc = &mut app.documents[0];
    let after = doc.snapshot();
    let strokes = doc.history_counts().0;
    let pixels = doc.layers[0].pixels.clone();
    let actual = checksum(&pixels);
    for _ in 0..strokes {
        if !doc.undo() {
            return Err("Virtual undo unavailable".into());
        }
    }
    if doc.snapshot() != before {
        return Err("Virtual undo did not restore the initial drawing".into());
    }
    for _ in 0..strokes {
        if !doc.redo() {
            return Err("Virtual redo unavailable".into());
        }
    }
    if doc.snapshot() != after {
        return Err("Virtual redo changed the drawing".into());
    }
    let passed = strokes == sequence.expected_strokes
        && sequence
            .expected_checksum
            .as_ref()
            .is_none_or(|v| v.eq_ignore_ascii_case(&actual));
    Ok(Outcome {
        record: Record {
            sequence: sequence.clone(),
            actual_strokes: strokes,
            checksum: actual,
            incomplete_cancelled,
            passed,
            frames,
        },
        pixels,
    })
}
struct Job {
    rx: mpsc::Receiver<Result<Outcome, String>>,
    thread: Option<std::thread::JoinHandle<()>>,
    cancel: Arc<AtomicBool>,
    progress: Arc<AtomicUsize>,
}
impl Drop for Job {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}
#[derive(Default)]
pub(super) struct Editor {
    pub open: bool,
    sequence: Sequence,
    path: String,
    replace: bool,
    job: Option<Job>,
    result: Option<Outcome>,
    texture: Option<egui::TextureHandle>,
    message: Option<String>,
    #[cfg(test)]
    run_button: Option<egui::Rect>,
}
impl Editor {
    fn start(&mut self) {
        self.result = None;
        self.texture = None;
        if let Err(e) = self.sequence.validate() {
            self.message = Some(e);
            return;
        }
        let (tx, rx) = mpsc::channel();
        let sequence = self.sequence.clone();
        let cancel = Arc::new(AtomicBool::new(false));
        let progress = Arc::new(AtomicUsize::new(0));
        let flag = cancel.clone();
        let count = progress.clone();
        let thread = std::thread::spawn(move || {
            let _ = tx.send(run(&sequence, &flag, &count));
        });
        self.job = Some(Job {
            rx,
            thread: Some(thread),
            cancel,
            progress,
        });
        self.message = None;
    }
    fn poll(&mut self, ctx: &egui::Context) {
        let result = self.job.as_ref().and_then(|j| match j.rx.try_recv() {
            Ok(r) => Some(r),
            Err(mpsc::TryRecvError::Disconnected) => {
                Some(Err("Virtual input worker stopped".into()))
            }
            _ => None,
        });
        if let Some(result) = result {
            let mut job = self.job.take().unwrap();
            if let Some(thread) = job.thread.take() {
                let _ = thread.join();
            }
            match result {
                Ok(outcome) => {
                    let image = egui::ColorImage::new(
                        [
                            outcome.record.sequence.width as usize,
                            outcome.record.sequence.height as usize,
                        ],
                        outcome
                            .pixels
                            .iter()
                            .map(|p| egui::Color32::from_rgba_unmultiplied(p[0], p[1], p[2], p[3]))
                            .collect(),
                    );
                    self.texture = Some(ctx.load_texture(
                        "virtual-input-result",
                        image,
                        egui::TextureOptions::NEAREST,
                    ));
                    self.result = Some(outcome);
                }
                Err(e) => self.message = Some(e),
            }
        }
        if self.job.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(16));
        }
    }
}
impl Drawer {
    pub(super) fn input_replay(&mut self, ctx: &egui::Context) {
        self.replay.poll(ctx);
        if !self.replay.open {
            return;
        }
        let mut open = true;
        let mut start = false;
        egui::Window::new("Virtual input replay").open(&mut open).vscroll(true).show(ctx,|ui| {
            ui.label("Runs the normal drawing pipeline in an isolated test document. Your open drawings are unchanged. Physical pen validation remains pending.");
            if let Some(job)=&self.replay.job {ui.label(format!("Replayed {} events",job.progress.load(Ordering::Relaxed)));
                if crate::help::button(ui,"Cancel replay","input-replay").clicked(){job.cancel.store(true,Ordering::Relaxed);}
            }else{
                let r=crate::help::button(ui,"Run virtual record","input-replay");start=r.clicked();
                #[cfg(test)] {self.replay.run_button=Some(r.rect);}
                ui.horizontal_wrapped(|ui| {
                    if crate::help::button(ui,"Pressure 0 / jump / half / end","input-replay").clicked(){self.replay.sequence=Sequence::default();}
                    if crate::help::button(ui,"Missing end","input-replay").clicked(){let mut s=Sequence::default();s.samples.pop();s.expected_strokes=0;self.replay.sequence=s;}
                    if crate::help::button(ui,"Tablet area sample","input-replay").clicked(){let mut s=Sequence::default();s.pointer.mapping.enabled=true;for sample in &mut s.samples {sample.event.attributes.tablet=Some([0.5;2]);}self.replay.sequence=s;}
                    if crate::help::button(ui,"Mouse start/end pressure","input-replay").clicked(){let mut s=Sequence::default();s.mouse_pressure[0].enabled=true;for sample in &mut s.samples {sample.event.source=drawer_core::pointer_input::Source::MOUSE;sample.event.pressure=None;}s.pen_backend_available=false;self.replay.sequence=s;}
                    if crate::help::button(ui,"Reverse time (invalid)","input-replay").clicked(){let mut s=Sequence::default();s.samples[1].micros=5000;self.replay.sequence=s;}
                });
                if crate::help::button(ui,"Use current brush settings in record","input-replay").clicked() {
                    let s=&mut self.replay.sequence;
                    s.tool=self.selected;s.size=self.panels.settings.brush_size;s.aspect=self.panels.settings.brush_aspect;s.angle=self.panels.settings.brush_angle;
                    s.color=self.references.session.workspace.color;s.pressure=self.workspaces.store.current.pressure;s.pointer=self.workspaces.store.current.pointer;
                    s.smoothing=self.workspaces.store.current.input.smoothing;s.expected_checksum=None;
                    s.mouse_pressure=self.workspaces.store.current.input.mouse_pressure;
                    s.cursors=self.workspaces.store.current.cursors;
                }
                ui.label(format!("{}×{} · {:?} · {} events · expected {} strokes",self.replay.sequence.width,self.replay.sequence.height,self.replay.sequence.tool,self.replay.sequence.samples.len(),self.replay.sequence.expected_strokes));
                ui.label("Local input JSON path");if super::edit::text(ui,&mut self.replay.path,"input-replay").changed(){self.replay.replace=false;}
                crate::help::checkbox(ui,&mut self.replay.replace,"Replace existing record/report","input-replay");
                let path=PathBuf::from(&self.replay.path);
                if ui.add_enabled(!self.replay.path.is_empty(),egui::Button::new("Load input record")).clicked(){
                    match Sequence::read(&path){Ok(s)=>self.replay.sequence=s,Err(e)=>self.replay.message=Some(e.to_string())}
                }
                if ui.add_enabled(!self.replay.path.is_empty() && (!path.exists() || self.replay.replace),egui::Button::new("Save input record")).clicked()
                    && let Err(e)=self.replay.sequence.save(&path){self.replay.message=Some(e.to_string());}
                let report_path=path.with_extension("result.json");
                if ui.add_enabled(!self.replay.path.is_empty() && self.replay.result.is_some() && (!report_path.exists() || self.replay.replace),egui::Button::new("Save result report (.result.json)")).clicked()
                    && let Err(e)=self.replay.result.as_ref().unwrap().record.save(&report_path){self.replay.message=Some(e.to_string());}
            }
            if let Err(e)=self.replay.sequence.validate(){ui.colored_label(egui::Color32::YELLOW,e);}
            if let Some(result)=&self.replay.result {
                ui.label("Last completed replay result (its input is included in the saved report)");
                ui.label(if result.record.sequence.expected_checksum.is_some() {"Exact expected pixel fingerprint checked"}else{"No expected pixel fingerprint set; stroke count and undo/redo checked"});
                ui.colored_label(if result.record.passed {egui::Color32::LIGHT_GREEN}else{egui::Color32::LIGHT_RED},format!("{} · {} strokes · RGBA {} · incomplete cancelled {} · undo/redo verified",if result.record.passed {"PASS"}else{"FAIL"},result.record.actual_strokes,result.record.checksum,result.record.incomplete_cancelled));
                if let Some(texture)=&self.replay.texture {ui.image((texture.id(),texture.size_vec2()*3.0));}
            }
            if let Some(message)=&self.replay.message {ui.colored_label(egui::Color32::YELLOW,message);}
        });
        self.replay.open = open;
        if !open && let Some(job) = &self.replay.job {
            job.cancel.store(true, Ordering::Relaxed);
        }
        if start {
            self.replay.start();
            ctx.request_repaint();
        }
    }
}

pub(super) fn smoke_ends() -> Result<(), String> {
    let cancel = AtomicBool::new(false);
    let progress = AtomicUsize::new(0);
    let mut s = Sequence::default();
    s.smoothing[0] = 0.95;
    s.ends[0] = drawer_core::stroke_ends::Settings {
        start: 8.0,
        end: 8.0,
    };
    s.pressure.tools[0].size = false;
    s.pressure.tools[0].opacity = true;
    s.samples = vec![s.samples[0].clone(), s.samples[3].clone()];
    for sample in &mut s.samples {
        sample.event.pressure = Some(0.5);
    }
    let a = run(&s, &cancel, &progress)?;
    if !a.record.passed || a.pixels[20 * 64 + 40][3] != 128 || a.pixels[20 * 64 + 58][3] != 0 {
        return Err("Smoothed native-pressure geometry endpoint/opacity failed".into());
    }
    for mode in drawer_core::brush_cursor::Mode::ALL {
        s.cursors.tools[0].mode = mode;
        s.cursors.tools[0].prediction = true;
        let b = run(&s, &cancel, &progress)?;
        if !b.record.passed || b.pixels != a.pixels {
            return Err("Raw/corrected/predicted cursor changed geometry pixels".into());
        }
    }
    let mut dot = s.clone();
    dot.samples[1].event.position = dot.samples[0].event.position;
    let p = dot.samples[0].event.position;
    let d = run(&dot, &cancel, &progress)?;
    if !d.record.passed || d.pixels[p[1].floor() as usize * 64 + p[0].floor() as usize][3] != 128 {
        return Err("Tapered single click vanished".into());
    }
    let mut missing = s;
    missing.samples.pop();
    missing.expected_strokes = 0;
    let m = run(&missing, &cancel, &progress)?;
    if !m.record.passed || !m.record.incomplete_cancelled {
        return Err("Pending geometry stroke was committed on focus loss".into());
    }
    Ok(())
}
pub(super) fn smoke_replay() -> Result<(), String> {
    let cancel = AtomicBool::new(false);
    let progress = AtomicUsize::new(0);
    let s = Sequence::default();
    let a = run(&s, &cancel, &progress)?;
    let b = run(&s, &cancel, &progress)?;
    if !a.record.passed || a.pixels != b.pixels {
        return Err("Virtual replay is not reproducible".into());
    }
    let mut missing = s.clone();
    missing.samples.pop();
    missing.expected_strokes = 0;
    let m = run(&missing, &cancel, &progress)?;
    if !m.record.passed || !m.record.incomplete_cancelled {
        return Err("Missing-end replay was not cancelled".into());
    }
    let mut bad = s;
    bad.samples[1].micros = 9999;
    if run(&bad, &cancel, &progress).is_ok() {
        return Err("Reverse event time was accepted".into());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{TestDirectory, click, tiny_spec};
    #[test]
    fn smoothed_tapered_native_replay_click_end_cursor_and_missing_release() {
        smoke_ends().unwrap();
    }
    #[test]
    fn sparse_thin_pen_mouse_curves_same_timestamps_and_frozen_parameters_replay() {
        let cancel = AtomicBool::new(false);
        let progress = AtomicUsize::new(0);
        for mouse in [false, true] {
            let mut s = Sequence {
                size: 20.0,
                aspect: 0.05,
                angle: 90.0,
                ..Default::default()
            };
            s.pressure.tools[0].size = false;
            s.pressure.tools[0].opacity = false;
            s.samples = vec![s.samples[0].clone(), s.samples[3].clone()];
            s.samples[0].event.position = [4.5, 20.5];
            s.samples[1].event.position = [58.5, 20.5];
            for sample in &mut s.samples {
                sample.event.pressure = Some(1.0);
                if mouse {
                    sample.event.source = drawer_core::pointer_input::Source::MOUSE;
                    sample.event.pressure = None;
                }
            }
            let a = run(&s, &cancel, &progress).unwrap();
            assert!(a.record.passed);
            assert!((5..58).all(|x| a.pixels[20 * 64 + x][3] > 0));
            s.samples[1].micros = 0;
            let b = run(&s, &cancel, &progress).unwrap();
            assert!(b.record.passed);
            assert_eq!(a.pixels, b.pixels);
            s.opacity[0] = 0.0;
            s.expected_strokes = 0;
            let transparent = run(&s, &cancel, &progress).unwrap();
            assert!(transparent.record.passed);
            assert!(transparent.pixels.iter().all(|p| *p == [0; 4]));
            s.opacity[0] = 0.5;
            s.hardness[0] = 0.3;
            s.spacing[0] = 0.2;
            s.expected_strokes = 1;
            let custom = run(&s, &cancel, &progress).unwrap();
            assert!(custom.record.passed);
            assert!(custom.pixels.iter().all(|p| p[3] <= 128));
        }
    }
    #[test]
    fn replay_repeats_actual_pixels_timestamp_groups_zero_half_full_and_missing_end() {
        smoke_replay().unwrap();
        let cancel = AtomicBool::new(false);
        let progress = AtomicUsize::new(0);
        for pressure in [0.0, 0.5, 1.0] {
            let mut s = Sequence::default();
            s.samples = vec![s.samples[0].clone(), s.samples[3].clone()];
            for sample in &mut s.samples {
                sample.event.position = [20.5, 20.5];
                sample.event.pressure = Some(pressure);
                sample.micros = 0;
            }
            s.expected_strokes = usize::from(pressure > 0.0);
            let out = run(&s, &cancel, &progress).unwrap();
            assert!(out.record.passed);
            assert_eq!(
                out.pixels[20 * 64 + 20][3],
                (255.0 * pressure).round() as u8
            );
        }
        let mut s = Sequence::default();
        let baseline = run(&s, &cancel, &progress).unwrap();
        for sample in &mut s.samples {
            sample.micros = 0;
        }
        let grouped = run(&s, &cancel, &progress).unwrap();
        assert!(baseline.pixels == grouped.pixels);
        s.expected_checksum = Some("0000000000000000".into());
        assert!(!run(&s, &cancel, &progress).unwrap().record.passed);
        cancel.store(true, Ordering::Relaxed);
        assert!(run(&s, &cancel, &progress).is_err());
    }
    #[test]
    fn local_record_roundtrip_damage_limits_foreign_temporary_file_and_worker_ui_preserve_drawings()
    {
        let dir = TestDirectory::new();
        let path = dir.0.join("가상-입력.json");
        let s = Sequence::default();
        s.save(&path).unwrap();
        assert_eq!(Sequence::read(&path).unwrap(), s);
        let original = std::fs::read(&path).unwrap();
        let tmp = path.with_extension("json.write.tmp");
        std::fs::write(&tmp, "foreign").unwrap();
        assert!(s.save(&path).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), original);
        std::fs::remove_file(tmp).unwrap();
        std::fs::write(&path, b"{bad").unwrap();
        assert!(Sequence::read(&path).is_err());
        std::fs::write(&path, vec![b' '; 2 * 1024 * 1024 + 1]).unwrap();
        assert!(Sequence::read(&path).is_err());
        let ctx = egui::Context::default();
        let mut app = Drawer::new(&ctx, None);
        app.create_canvas(tiny_spec()).unwrap();
        let before = app.documents[0].snapshot();
        let dirty = app.documents[0].dirty;
        app.replay.open = true;
        frame(&ctx, &mut app, 0.0, vec![]);
        frame(&ctx, &mut app, 0.01, vec![]);
        let rect = app.replay.run_button.unwrap();
        click(&ctx, &mut app, rect);
        assert!(app.replay.job.is_some() || app.replay.result.is_some());
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while app.replay.job.is_some() && std::time::Instant::now() < deadline {
            app.replay.poll(&ctx);
            std::thread::yield_now();
        }
        assert!(app.replay.job.is_none());
        let result = app.replay.result.as_ref().unwrap();
        assert!(result.record.passed);
        result.record.save(&dir.0.join("result.json")).unwrap();
        assert_eq!(
            Sequence::read(&dir.0.join("result.json")).unwrap(),
            result.record.sequence
        );
        assert!(app.replay.texture.is_some());
        assert_eq!(app.documents.len(), 1);
        assert!(app.documents[0].snapshot() == before);
        assert_eq!(app.documents[0].history_counts(), (0, 0));
        assert_eq!(app.documents[0].dirty, dirty);
    }
}
