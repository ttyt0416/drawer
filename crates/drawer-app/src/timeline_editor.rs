use super::*;
use drawer_core::{
    cels,
    timeline_editor::{self as core, Edit, Jump},
    timeline_view::{self, Onion},
};
use std::{
    collections::BTreeMap,
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
};
#[derive(Default)]
pub(super) struct Panel {
    open: bool,
    views: BTreeMap<u64, View>,
    staged: Option<Staged>,
    job: Option<Job>,
    binding: Option<Binding>,
    clipboard: Option<Box<Document>>,
    notice: String,
    frames: u32,
    fps: [u32; 2],
    name: String,
    label: String,
    confirm: bool,
    section: u8,
    strip: u32,
    pub apply: Option<egui::Rect>,
    pub open_cel: Option<egui::Rect>,
    pub content_apply: Option<egui::Rect>,
    pub play: Option<egui::Rect>,
    pub overlay_active: bool,
}
struct View {
    track: Option<u64>,
    source: Option<u64>,
    target: Option<u64>,
    onion: Onion,
    only: bool,
    repeat: bool,
    playing: Option<(f64, u32)>,
    image: Option<egui::TextureHandle>,
    overlay: Option<egui::TextureHandle>,
    key: Option<(u64, u32, Option<u64>, bool, Onion)>,
}
impl Default for View {
    fn default() -> Self {
        Self {
            track: None,
            source: None,
            target: None,
            onion: Default::default(),
            only: false,
            repeat: true,
            playing: None,
            image: None,
            overlay: None,
            key: None,
        }
    }
}
#[derive(Clone, Copy)]
struct Stamp {
    id: u64,
    revision: u64,
    frame: u32,
}
impl Stamp {
    fn of(d: &Document) -> Self {
        Self {
            id: d.id,
            revision: d.revision,
            frame: d.current_frame,
        }
    }
    fn matches(self, d: &Document) -> bool {
        self.id == d.id && self.revision == d.revision && self.frame == d.current_frame
    }
}
struct Binding {
    parent: Stamp,
    source: u64,
    child: u64,
    offset: [i64; 2],
}
struct Staged {
    stamp: Stamp,
    prepared: core::Prepared,
    destructive: bool,
    image: Option<egui::TextureHandle>,
}
struct Job {
    cancel: Arc<AtomicBool>,
    receiver: mpsc::Receiver<Result<Staged, String>>,
}
impl Drop for Job {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}
fn job(
    ctx: &egui::Context,
    work: impl FnOnce(&AtomicBool) -> Result<Staged, String> + Send + 'static,
) -> Job {
    let cancel = Arc::new(AtomicBool::new(false));
    let flag = cancel.clone();
    let (tx, receiver) = mpsc::channel();
    let ctx = ctx.clone();
    std::thread::spawn(move || {
        let result = work(&flag);
        let _ = tx.send(result);
        ctx.request_repaint();
    });
    Job { cancel, receiver }
}
fn upload(
    ctx: &egui::Context,
    image: &drawer_core::references::LocalImage,
    name: &str,
) -> egui::TextureHandle {
    let rgba = image.pixels.iter().flatten().copied().collect::<Vec<_>>();
    ctx.load_texture(
        name,
        egui::ColorImage::from_rgba_unmultiplied(
            [image.width as usize, image.height as usize],
            &rgba,
        ),
        egui::TextureOptions::NEAREST,
    )
}
fn preview(ctx: &egui::Context, d: &Document) -> Result<egui::TextureHandle, String> {
    let scale = (d.width.max(d.height) as f64 / 200.).max(1.);
    let w = (d.width as f64 / scale).ceil() as u32;
    let h = (d.height as f64 / scale).ceil() as u32;
    let r = drawer_core::float_render::Renderer::new(d)?;
    let mut pixels = vec![];
    for y in 0..h {
        for x in 0..w {
            pixels.push(
                r.display_sample([
                    (x as f64 + 0.5) * d.width as f64 / w as f64,
                    (y as f64 + 0.5) * d.height as f64 / h as f64,
                ])?
                .rgba8(),
            );
        }
    }
    Ok(upload(
        ctx,
        &drawer_core::references::LocalImage {
            width: w,
            height: h,
            pixels,
        },
        "timeline-change-preview",
    ))
}
fn usage(d: &Document, source: u64) -> (usize, usize) {
    let Some(a) = &d.animation else {
        return (0, 0);
    };
    let keys = d
        .scene
        .cels
        .links
        .iter()
        .filter(|l| l.source == source)
        .map(|l| (l.track, l.cell))
        .collect::<std::collections::BTreeSet<_>>();
    let mut frames = vec![false; a.frames as usize];
    let mut tracks = std::collections::BTreeSet::new();
    for (n, t) in a.tracks.iter().enumerate() {
        let id = d.scene.catalog.tracks.id(n);
        for (i, e) in t.exposures.iter().enumerate() {
            if keys.contains(&(id, d.scene.catalog.cell_id(id, e.cell))) {
                let end = t
                    .exposures
                    .get(i + 1)
                    .map_or(t.end, |next| next.frame)
                    .min(a.frames);
                if e.frame < end {
                    frames[e.frame as usize..end as usize].fill(true);
                    tracks.insert(id);
                }
            }
        }
    }
    (frames.iter().filter(|v| **v).count(), tracks.len())
}
impl Drawer {
    pub(super) fn begin_timeline_editor(&mut self) -> Result<(), String> {
        self.finish_stroke();
        if self.transform_unconfirmed()
            || self.inference_unconfirmed()
            || self.text_editor.draft.is_some()
        {
            return Err("Finish pending edits first".into());
        }
        self.timeline_editor.open = true;
        if self.timeline_editor.frames == 0 {
            self.timeline_editor.frames = 24;
            self.timeline_editor.fps = [24, 1];
            self.timeline_editor.name = "Cel".into();
            self.timeline_editor.label = "Label".into();
        }
        Ok(())
    }
    fn timeline_context(&self) -> Option<usize> {
        let active = self.active?;
        if let Some(b) = &self.timeline_editor.binding
            && self.documents[active].id == b.child
        {
            return self.documents.iter().position(|d| d.id == b.parent.id);
        }
        Some(active)
    }
    fn timeline_budget(&self) -> usize {
        DEFAULT_DOCUMENT_BUDGET.saturating_sub(
            self.documents
                .iter()
                .map(Document::resident_bytes)
                .sum::<usize>(),
        )
    }
    fn prepare_timeline(
        &mut self,
        ctx: &egui::Context,
        edit: Edit,
        destructive: bool,
    ) -> Result<(), String> {
        self.finish_stroke();
        if self.timeline_editor.job.is_some() {
            return Err("Wait for the current timeline preparation".into());
        }
        let d = &self.documents[self.timeline_context().ok_or("Choose a drawing")?];
        let stamp = Stamp::of(d);
        let budget = self.timeline_budget();
        if d.memory_bytes() > budget / 3 {
            return Err("Timeline snapshot budget; close drawings or reduce cel content".into());
        }
        let copy = drawer_core::layer_merge::preview(&d.snapshot(), d.current_frame)?;
        self.timeline_editor
            .views
            .entry(stamp.id)
            .or_default()
            .playing = None;
        self.timeline_editor.staged = None;
        self.timeline_editor.confirm = false;
        self.timeline_editor.job = Some(job(ctx, move |cancel| {
            if cancel.load(Ordering::Relaxed) {
                return Err("Timeline preparation cancelled".into());
            }
            let prepared = core::prepare(&copy, edit, budget)?;
            if cancel.load(Ordering::Relaxed) {
                return Err("Timeline preparation cancelled".into());
            }
            Ok(Staged {
                stamp,
                prepared,
                destructive,
                image: None,
            })
        }));
        Ok(())
    }
    fn apply_timeline(&mut self) -> Result<(), String> {
        let p = self
            .timeline_editor
            .staged
            .as_ref()
            .ok_or("Prepare an edit first")?;
        if p.destructive && !self.timeline_editor.confirm {
            return Err("Review the listed reference removals and confirm deletion".into());
        }
        let n = self
            .documents
            .iter()
            .position(|d| d.id == p.stamp.id)
            .ok_or("Timeline drawing was closed")?;
        if self.timeline_context().map(|i| self.documents[i].id) != Some(p.stamp.id) {
            return Err("Activate the reviewed timeline before applying".into());
        }
        if !p.stamp.matches(&self.documents[n]) {
            return Err("Timeline/frame changed; prepare the change again".into());
        }
        self.documents[n].commit_state(p.prepared.state.clone())?;
        let view = self.timeline_editor.views.entry(p.stamp.id).or_default();
        if let Some(t) = p.prepared.track {
            view.track = Some(t);
        }
        if let Some(id) = p.prepared.source {
            view.source = Some(id);
        }
        view.key = None;
        self.timeline_editor.notice = p.prepared.summary.clone();
        self.timeline_editor.staged = None;
        Ok(())
    }
    fn open_timeline_cel(&mut self) -> Result<(), String> {
        self.finish_stroke();
        if self.timeline_editor.binding.is_some() || self.tabs.len() >= 256 {
            return Err("Apply/return from the existing cel edit or close some views".into());
        }
        let n = self.timeline_context().ok_or("Choose an animation")?;
        let d = &self.documents[n];
        let view = self
            .timeline_editor
            .views
            .get(&d.id)
            .ok_or("Select a cel")?;
        let id = view.source.ok_or("Select a cel definition")?;
        let p = d
            .scene
            .cels
            .get(id)
            .ok_or("Prepare/enable editable cels first")?;
        let refs = core::uses(d, id);
        for l in d.scene.cels.links.iter().filter(|l| l.source == id) {
            if let Some((t, c, _)) = cels::slot(d, l.track, l.cell) {
                cels::editable(d, drawer_core::layer_properties::Target::Track(t as u16))?;
                cels::editable(
                    d,
                    drawer_core::layer_properties::Target::Cell {
                        track: t as u16,
                        cell: c as u16,
                    },
                )?;
            }
        }
        let offset = view
            .track
            .and_then(|track| {
                d.scene
                    .cels
                    .links
                    .iter()
                    .find(|l| l.track == track && l.source == id)
                    .and_then(|l| cels::slot(d, l.track, l.cell).map(|(_, _, c)| c.offset))
            })
            .unwrap_or([0, 0]);
        let (used_frames, used_tracks) = usage(d, id);
        let child = p.open()?;
        let binding = Binding {
            parent: Stamp::of(d),
            source: id,
            child: child.id,
            offset,
        };
        self.add_document(child);
        self.timeline_editor.binding = Some(binding);
        self.timeline_editor.notice = format!(
            "Editing shared cel{id}: {used_frames} frames across {used_tracks} tracks ({} exposure starts). Use every drawing tool in this separate content view; Apply changes all references with one parent Undo.",
            refs.len()
        );
        Ok(())
    }
    fn apply_timeline_cel(&mut self) -> Result<(), String> {
        self.finish_stroke();
        if self.transform_unconfirmed() || self.text_editor.draft.is_some() {
            return Err("Finish the pending content edit first".into());
        }
        let b = self
            .timeline_editor
            .binding
            .as_ref()
            .ok_or("Open a cel first")?;
        let parent = self
            .documents
            .iter()
            .position(|d| d.id == b.parent.id)
            .ok_or("Parent timeline was closed")?;
        if b.parent.id != self.documents[parent].id
            || b.parent.revision != self.documents[parent].revision
        {
            return Err("Parent timeline/frame changed; reopen its cel before applying".into());
        }
        let child = self
            .documents
            .iter()
            .find(|d| d.id == b.child)
            .ok_or("Cel content view was closed")?;
        let state = cels::replace(
            &self.documents[parent],
            b.source,
            child,
            self.timeline_budget(),
        )?;
        let id = b.parent.id;
        self.documents[parent].commit_state(state)?;
        self.timeline_editor.binding = None;
        if let Some(tab) = self.tabs.iter().position(|t| t.document_id == id) {
            self.select_tab(tab);
        }
        self.timeline_editor.views.entry(id).or_default().key = None;
        self.timeline_editor.notice="Applied editable cel content to every shared reference with one drawing Undo. Independent copies remain unchanged.".into();
        Ok(())
    }
    fn return_timeline_cel(&mut self) {
        if let Some(b) = self.timeline_editor.binding.take() {
            if let Some(tab) = self.tabs.iter().position(|t| t.document_id == b.parent.id) {
                self.select_tab(tab);
            }
            self.timeline_editor.notice =
                "Returned without applying; the independent content view remains open.".into();
        }
    }
    fn timeline_clipboard(&self, ctx: &egui::Context) -> Result<Document, String> {
        if let Some(copy) = &self.timeline_editor.clipboard {
            return drawer_core::layer_merge::preview(&copy.snapshot(), 0);
        }
        let vector = if crate::vector_scope::has_clipboard(ctx) {
            Some(crate::vector_scope::read_timeline_packet(ctx)?)
        } else {
            None
        };
        self.timeline_clipboard_with(vector, paste_dialog::read_clipboard)
    }
    fn timeline_clipboard_with(
        &self,
        vector: Option<drawer_core::vector_clipboard::Packet>,
        read: impl FnOnce() -> Result<drawer_core::paste::ClipboardImage, String>,
    ) -> Result<Document, String> {
        let d = &self.documents[self.timeline_context().ok_or("Choose a drawing")?];
        let budget = self.timeline_budget();
        if let Some(packet) = vector {
            let mut child = Document::create(
                CanvasSpec {
                    width: d.width,
                    height: d.height,
                    dpi: d.dpi,
                    background: Background::Transparent,
                },
                budget,
            )
            .map_err(|e| e.to_string())?;
            child.scene.transform = packet.transform;
            let state = drawer_core::layers::prepare(
                &child,
                drawer_core::layers::Command::Create {
                    kind: drawer_core::layers::Kind::Vector,
                    name: "Pasted paths".into(),
                    parent: None,
                },
            )?;
            let drawer_core::layers::Target::Vector(layer) = state.selected else {
                return Err("Vector layer missing".into());
            };
            child.commit_state(state.state)?;
            child.commit_state(
                drawer_core::vector_clipboard::paste(&child, layer, &packet, [0., 0.])?.0,
            )?;
            return Ok(child);
        }
        let packet = read()?;
        let mut child = Document::create_with_color(
            CanvasSpec {
                width: packet.image.width,
                height: packet.image.height,
                dpi: d.dpi,
                background: Background::Transparent,
            },
            budget,
            d.color.clone(),
        )
        .map_err(|e| e.to_string())?;
        child.commit_state(drawer_core::paste::prepare_packet(
            &child,
            &packet,
            [0, 0],
            budget,
        )?)?;
        Ok(child)
    }
    pub(super) fn timeline_tick(&mut self, ctx: &egui::Context) {
        self.canvas_view.timeline_overlay = None;
        self.display.canvas.timeline_overlay = None;
        self.timeline_editor.overlay_active = false;
        if !self.timeline_editor.open {
            return;
        }
        let Some(n) = self.timeline_context() else {
            return;
        };
        let id = self.documents[n].id;
        let Some(a) = &self.documents[n].animation else {
            return;
        };
        let frames = a.frames;
        let fps = a.fps;
        let now = ctx.input(|i| i.time);
        let view = self.timeline_editor.views.entry(id).or_default();
        if view.track.is_none_or(|t| {
            self.documents[n]
                .scene
                .catalog
                .tracks
                .index(t, a.tracks.len())
                .is_none()
        }) {
            view.track =
                (!a.tracks.is_empty()).then(|| self.documents[n].scene.catalog.tracks.id(0));
        }
        if view
            .source
            .is_none_or(|s| self.documents[n].scene.cels.get(s).is_none())
        {
            view.source = self.documents[n]
                .scene
                .cels
                .definitions
                .first()
                .map(|p| p.object);
        }
        if let Some((time, start)) = view.playing
            && let Ok((frame, end)) =
                core::playback(start, (now - time).max(0.), fps, frames, view.repeat)
        {
            self.documents[n].current_frame = frame;
            if end {
                view.playing = None;
            }
            ctx.request_repaint_after(std::time::Duration::from_secs_f64(
                (1. / fps.value()).min(0.1),
            ));
        }
        let d = &self.documents[n];
        let key = (
            d.revision,
            d.current_frame,
            view.track,
            view.only,
            view.onion,
        );
        if view.key != Some(key) {
            match timeline_view::preview(
                d,
                view.track,
                view.only,
                view.onion,
                256,
                DEFAULT_DOCUMENT_BUDGET,
            ) {
                Ok(p) => {
                    view.image = Some(upload(ctx, &p.image, "timeline-view"));
                    view.overlay = Some(upload(ctx, &p.overlay, "timeline-onion"));
                    view.key = Some(key);
                }
                Err(e) => {
                    view.image = None;
                    view.overlay = None;
                    self.timeline_editor.notice = e;
                }
            }
        }
        if let (Some(active), Some(binding)) = (self.active, self.timeline_editor.binding.as_ref())
            && self.documents[active].id == binding.child
            && view.onion.enabled
            && let Some(texture) = &view.overlay
        {
            self.canvas_view.timeline_overlay = Some(crate::canvas_view::TimelineOverlay {
                document: binding.child,
                texture: texture.clone(),
                replace: false,
                origin: [-binding.offset[0] as f64, -binding.offset[1] as f64],
                size: [d.width as f64, d.height as f64],
            });
            self.timeline_editor.overlay_active = true;
        }
        if let Some(active) = self.active
            && self.documents[active].id == id
            && (view.only || view.onion.enabled)
            && let Some(texture) = if view.only {
                view.image.as_ref()
            } else {
                view.overlay.as_ref()
            }
        {
            let overlay = crate::canvas_view::TimelineOverlay {
                document: id,
                texture: texture.clone(),
                replace: view.only,
                origin: [0., 0.],
                size: [d.width as f64, d.height as f64],
            };
            self.canvas_view.timeline_overlay = Some(overlay.clone());
            self.display.canvas.timeline_overlay = Some(overlay);
            self.timeline_editor.overlay_active = true;
        }
    }
    pub(super) fn timeline_window(&mut self, ctx: &egui::Context) {
        if let Some(j) = &self.timeline_editor.job {
            let result = match j.receiver.try_recv() {
                Ok(r) => Some(r),
                Err(mpsc::TryRecvError::Disconnected) => {
                    Some(Err("Timeline worker stopped".into()))
                }
                Err(mpsc::TryRecvError::Empty) => None,
            };
            if let Some(result) = result {
                let cancelled = j.cancel.load(Ordering::Relaxed);
                self.timeline_editor.job = None;
                if cancelled {
                    self.timeline_editor.notice = "Cancelled; drawing unchanged".into();
                } else {
                    match result {
                        Ok(p) => self.timeline_editor.staged = Some(p),
                        Err(e) => self.timeline_editor.notice = e,
                    }
                }
            } else {
                ctx.request_repaint_after(std::time::Duration::from_millis(25));
            }
        }
        if !self.timeline_editor.open {
            return;
        }
        let Some(n) = self.timeline_context() else {
            self.timeline_editor.notice =
                "Choose a drawing or reopen the closed parent timeline".into();
            return;
        };
        let id = self.documents[n].id;
        let mut p = std::mem::take(&mut self.timeline_editor);
        let mut open = true;
        let mut action = 0u8;
        let mut navigate = None;
        let mut edit = None;
        let d = &self.documents[n];
        let view = p.views.entry(id).or_default();
        egui::Window::new("Animation timeline and editable cels").default_pos(egui::pos2(15.,15.)).default_width(730.).open(&mut open).vscroll(true).show(ctx,|ui|{
            ui.label(&p.notice);if p.binding.is_some(){ui.label("Cel content is a separate editable drawing. Apply updates every shared exposure; Return keeps the original timeline.");ui.horizontal(|ui|{let r=ui.button("Apply cel content to timeline");p.content_apply=Some(r.rect);if r.clicked(){action=2;}
                if ui.button("Return without applying").clicked(){action=3;}});}
            if let Some(pending)=&mut p.staged{ui.separator();egui::ScrollArea::vertical().id_salt("timeline-reference-review").max_height(100.).show(ui,|ui|{ui.label(&pending.prepared.summary);});if pending.image.is_none()&& let Ok(prepared)=drawer_core::layer_merge::preview(&pending.prepared.state,pending.stamp.frame){match preview(ui.ctx(),&prepared){Ok(image)=>pending.image=Some(image),Err(e)=>{ui.label(e);}}}
if let Some(image)=&pending.image{ui.collapsing("Change preview",|ui|{ui.image((image.id(),image.size_vec2()));});}
if pending.destructive{ui.checkbox(&mut p.confirm,"I reviewed the affected references and confirm this removal / move");}let r=ui.add_enabled(!pending.destructive || p.confirm,egui::Button::new("Apply reviewed timeline edit"));p.apply=Some(r.rect);if r.clicked(){action=10;}
if ui.button("Discard prepared edit").clicked(){action=11;}}
            if let Some(a)=&d.animation{
                ui.horizontal_wrapped(|ui|{for(label,jump)in [("First",Jump::First),("Previous",Jump::Previous),("Next",Jump::Next),("Last",Jump::Last),("Previous cel",Jump::PreviousCel),("Next cel",Jump::NextCel),("Previous key",Jump::PreviousKey),("Next key",Jump::NextKey),("Previous label",Jump::PreviousLabel),("Next label",Jump::NextLabel)]{if ui.button(label).clicked(){navigate=Some(jump);}}});
                ui.horizontal(|ui|{let mut frame=d.current_frame+1;if ui.add(egui::DragValue::new(&mut frame).range(1..=a.frames)).changed(){navigate=Some(Jump::Number(frame-1));}ui.label(format!("/{} · {}/{} FPS",a.frames,a.fps.numerator,a.fps.denominator));let r=ui.button(if view.playing.is_some(){"Pause"}else{"Play"});p.play=Some(r.rect);if r.clicked(){action=4;}ui.checkbox(&mut view.repeat,"Loop");});
                ui.collapsing("Onion skin and editing-track view",|ui|{
                ui.checkbox(&mut view.only,"Show editing track only (view)");ui.checkbox(&mut view.onion.enabled,"Onion skin (view)");ui.horizontal(|ui|{ui.label("Before / after / opacity");ui.add(egui::DragValue::new(&mut view.onion.before).range(0..=8));ui.add(egui::DragValue::new(&mut view.onion.after).range(0..=8));ui.add(egui::Slider::new(&mut view.onion.opacity,0..=100));});ui.horizontal(|ui|{ui.color_edit_button_srgb(&mut view.onion.colors[0]);ui.color_edit_button_srgb(&mut view.onion.colors[1]);});
                });
                if let Some(image)=&view.image{ui.collapsing("Timeline preview",|ui|{ui.image((image.id(),image.size_vec2()));});}
                ui.horizontal(|ui|{ui.selectable_value(&mut p.section,0,"Tracks / exposure table");ui.selectable_value(&mut p.section,1,"Cel definitions / content");ui.selectable_value(&mut p.section,2,"Labels / clipboard");});
                if p.section==0{
                    ui.horizontal(|ui|{ui.label("Name");ui.text_edit_singleline(&mut p.name);if ui.button("New track + blank cel").clicked(){edit=Some(Edit::NewTrack(p.name.clone()));}
if ui.button("Enable editable cels").clicked(){edit=Some(Edit::Enable);}});
                    for(t,track)in a.tracks.iter().enumerate(){let tid=d.scene.catalog.tracks.id(t);ui.selectable_value(&mut view.track,Some(tid),format!("{} · {} cel slots · {} exposure starts",track.name,track.cells.len(),track.exposures.len()));}
                    if let Some(track)=view.track{ui.horizontal_wrapped(|ui|{if ui.button("Rename track").clicked(){edit=Some(Edit::RenameTrack{track,name:p.name.clone()});}
if ui.button("Copy track: shared").clicked(){edit=Some(Edit::CopyTrack{track,independent:false});}
if ui.button("Copy track: independent").clicked(){edit=Some(Edit::CopyTrack{track,independent:true});}
if ui.button("Delete track…").clicked(){edit=Some(Edit::DeleteTrack(track));}});}
                    ui.add(egui::Slider::new(&mut p.strip,0..=a.frames.saturating_sub(1)).text("Exposure table start"));egui::ScrollArea::horizontal().max_height(230.).show(ui,|ui|{for(t,track)in a.tracks.iter().enumerate(){let tid=d.scene.catalog.tracks.id(t);ui.horizontal(|ui|{ui.label(&track.name);for frame in p.strip..(p.strip+48).min(a.frames){let active=track.exposures.partition_point(|e|e.frame<=frame).checked_sub(1).and_then(|i|track.exposures.get(i)).filter(|_|frame<track.end);let global=active.and_then(|e|d.scene.cels.source(tid,d.scene.catalog.cell_id(tid,e.cell)));let text=global.map_or_else(||format!("{}:–",frame+1),|id|format!("{}:{id}",frame+1));if ui.selectable_label(frame==d.current_frame && view.track==Some(tid),text).clicked(){view.track=Some(tid);if let Some(global)=global{view.source=Some(global);}navigate=Some(Jump::Number(frame));}}});}});
                }else if p.section==1{
                    egui::ComboBox::from_label("Cel definition (includes unused)").selected_text(view.source.map_or("None".into(),|id|format!("Cel{id}"))).show_ui(ui,|ui|{for cel in &d.scene.cels.definitions{let refs=core::uses(d,cel.object);ui.selectable_value(&mut view.source,Some(cel.object),format!("{} · ID{} · {} exposure starts",cel.name,cel.object,refs.len()));}});
                    if let Some(id)=view.source{let refs=core::uses(d,id);ui.label(format!("{} frames across {} tracks ({} exposure starts) use this ID. Shared content edits and their Undo affect all these references.",usage(d,id).0,usage(d,id).1,refs.len()));ui.horizontal_wrapped(|ui|{let r=ui.button("Open editable cel content");p.open_cel=Some(r.rect);if r.clicked(){action=1;}
if ui.button("Rename cel").clicked(){edit=Some(Edit::RenameCel{id,name:p.name.clone()});}
if ui.button("Delete cel and ALL references…").clicked(){edit=Some(Edit::DeleteCel(id));}});}
                    ui.horizontal(|ui|{ui.label("New / renamed cel");ui.text_edit_singleline(&mut p.name);});
                    if let Some(track)=view.track{ui.horizontal_wrapped(|ui|{if ui.button("New blank cel at current frame").clicked(){edit=Some(Edit::NewCel{track,name:p.name.clone()});}
if let Some(source)=view.source{if ui.button("Assign at current frame").clicked(){edit=Some(Edit::Expose{track,source,frame:d.current_frame});}
if ui.button("Independent copy at current frame").clicked(){edit=Some(Edit::Duplicate{track,source,independent:true});}}});
                        if let Some(t)=d.scene.catalog.tracks.index(track,a.tracks.len()){let pos=a.tracks[t].exposures.partition_point(|e|e.frame<=d.current_frame);if let Some(e)=pos.checked_sub(1).and_then(|i|a.tracks[t].exposures.get(i))&& ui.button(format!("Remove exposure starting{} (keep definition)",e.frame+1)).clicked(){edit=Some(Edit::RemoveExposure{track,frame:e.frame});}}
                        egui::ComboBox::from_label("Move slot to track").selected_text(view.target.map_or("Choose".into(),|id|format!("Track{id}"))).show_ui(ui,|ui|{for(t,_)in a.tracks.iter().enumerate(){let id=d.scene.catalog.tracks.id(t);if id!=track{ui.selectable_value(&mut view.target,Some(id),format!("Track{id}"));}}});
                        if let(Some(to),Some(source))=(view.target,view.source) && let Some(l)=d.scene.cels.links.iter().find(|l|l.track==track && l.source==source)&& ui.button("Move selected cel slot and its exposures…").clicked(){edit=Some(Edit::MoveSlot{from:track,cell:l.cell,to});}
                    }

                }else{
                    ui.horizontal(|ui|{ui.text_edit_singleline(&mut p.label);if ui.button("Label at current frame").clicked(){edit=Some(Edit::Label{name:p.label.clone(),frame:d.current_frame});}});for label in &d.scene.cels.labels{ui.horizontal(|ui|{if ui.button(format!("{} · {}",label.name,label.frame+1)).clicked(){navigate=Some(Jump::Number(label.frame));}
if ui.button("Remove label").clicked(){edit=Some(Edit::DeleteLabel(label.id));}});}
                    ui.label("Copy a static drawing or an open cel content view as an editable local clipboard. System paste reads the existing editable vector format or pixel image format.");ui.horizontal_wrapped(|ui|{if ui.button("Copy active drawing content").clicked(){action=5;}
if ui.button("Use system clipboard instead").clicked(){action=6;}
if ui.button("Paste as new cel layer").clicked(){action=7;}
if ui.button("Paste to selected cel (replace content)").clicked(){action=8;}});
                }
            }else{
                ui.label("Create a timeline on this drawing. Its common layers remain.");ui.horizontal(|ui|{ui.label("Frames / FPS numerator / denominator");ui.add(egui::DragValue::new(&mut p.frames).range(1..=10000));ui.add(egui::DragValue::new(&mut p.fps[0]).range(1..=100000));ui.add(egui::DragValue::new(&mut p.fps[1]).range(1..=100000));});if ui.button("Prepare new timeline").clicked(){action=9;}
            }
            ui.horizontal(|ui|{if ui.button("Copy active static drawing / cel content").clicked(){action=5;}
if p.clipboard.is_some(){ui.label("Editable local clipboard ready");}});
            if let Some(job)=&p.job{ui.spinner();if ui.button("Cancel preparation").clicked(){job.cancel.store(true,Ordering::Relaxed);}}

        });
        p.open = open;
        if !open {
            for view in p.views.values_mut() {
                view.only = false;
                view.onion.enabled = false;
                view.playing = None;
            }
            if let Some(j) = &p.job {
                j.cancel.store(true, Ordering::Relaxed);
            }
            p.staged = None;
        }
        self.timeline_editor = p;
        if let Some(jump) = navigate {
            self.finish_stroke();
            let selected = self.timeline_editor.views.get(&id).and_then(|v| v.track);
            match core::navigate(&self.documents[n], selected, jump) {
                Ok(frame) => {
                    self.documents[n].current_frame = frame;
                    self.timeline_editor.views.entry(id).or_default().playing = None;
                }
                Err(e) => self.timeline_editor.notice = e,
            }
        }
        if let Some(edit) = edit {
            let destructive = matches!(
                edit,
                Edit::DeleteCel(_) | Edit::DeleteTrack(_) | Edit::MoveSlot { .. }
            );
            if let Err(e) = self.prepare_timeline(ctx, edit, destructive) {
                self.timeline_editor.notice = e;
            }
        }
        let result = (|| {
            match action {
                1 => self.open_timeline_cel()?,
                2 => self.apply_timeline_cel()?,
                3 => self.return_timeline_cel(),
                4 => {
                    self.finish_stroke();
                    let view = self.timeline_editor.views.entry(id).or_default();
                    view.playing = if view.playing.is_some() {
                        None
                    } else {
                        Some((ctx.input(|i| i.time), self.documents[n].current_frame))
                    };
                    ctx.request_repaint();
                }
                5 => {
                    let d = &self.documents[self.active.ok_or("Open a static drawing")?];
                    if d.animation.is_some() || !d.scene.cels.is_default() {
                        return Err(
                            "Open cel content or a static drawing before copying editable content"
                                .into(),
                        );
                    }
                    if d.memory_bytes() > self.timeline_budget() / 3 {
                        return Err("Clipboard snapshot budget".into());
                    }
                    self.timeline_editor.clipboard = Some(Box::new(
                        drawer_core::layer_merge::preview(&d.snapshot(), 0)?,
                    ));
                    self.timeline_editor.notice =
                        "Copied editable drawing content locally; original unchanged".into();
                }
                6 => self.timeline_editor.clipboard = None,
                7 | 8 => {
                    let view = self
                        .timeline_editor
                        .views
                        .get(&id)
                        .ok_or("Choose a track")?;
                    let track = view.track.ok_or("Choose a track")?;
                    let replace = if action == 8 {
                        Some(view.source.ok_or("Choose a cel")?)
                    } else {
                        None
                    };
                    let clipboard = self.timeline_clipboard(ctx)?;
                    self.prepare_timeline(
                        ctx,
                        Edit::Paste {
                            track,
                            replace,
                            document: Box::new(clipboard),
                        },
                        action == 8,
                    )?;
                }
                9 => {
                    let d = &self.documents[n];
                    let state = core::create(
                        d,
                        self.timeline_editor.frames,
                        drawer_core::animation::Fps {
                            numerator: self.timeline_editor.fps[0],
                            denominator: self.timeline_editor.fps[1],
                        },
                    )?;
                    self.timeline_editor.staged = Some(Staged {
                        stamp: Stamp::of(d),
                        prepared: core::Prepared {
                            state,
                            track: None,
                            source: None,
                            affected: 0,
                            summary: "Create an empty timeline; existing drawing content remains"
                                .into(),
                        },
                        destructive: false,
                        image: None,
                    });
                }
                10 => self.apply_timeline()?,
                11 => self.timeline_editor.staged = None,
                _ => {}
            }
            Ok::<_, String>(())
        })();
        if let Err(e) = result {
            self.timeline_editor.notice = e;
        }
    }
}

/// Runs the same commands and confirmation controls used by the installed application.
pub(super) fn smoke(root: &Path) -> Result<(), String> {
    drawer_core::timeline_verify::verify(root)?;
    let ctx = egui::Context::default();
    let mut app = Drawer::new(&ctx, None);
    app.simple_view = true;
    app.add_document(drawer_core::timeline_verify::parent());
    app.execute_command(&ctx, "animation-timeline");
    if !app.timeline_editor.open {
        return Err("Timeline command did not open".into());
    }
    let parent = app.documents[0].id;
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
    let wait = |app: &mut Drawer, time: &mut f64| -> Result<(), String> {
        let end = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while app.timeline_editor.job.is_some() {
            frame(app, time, vec![]);
            if std::time::Instant::now() > end {
                return Err("Timeline worker timeout".into());
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        if app.timeline_editor.staged.is_none() {
            return Err(format!(
                "Timeline preparation: {}",
                app.timeline_editor.notice
            ));
        }
        Ok(())
    };
    app.timeline_editor.staged = Some(Staged {
        stamp: Stamp::of(&app.documents[0]),
        prepared: core::Prepared {
            state: core::create(&app.documents[0], 10, Default::default())?,
            track: None,
            source: None,
            affected: 0,
            summary: "Create10-frame timeline".into(),
        },
        destructive: false,
        image: None,
    });
    for _ in 0..3 {
        frame(&mut app, &mut time, vec![]);
    }
    let r = app_rect(&app.timeline_editor.apply)?;
    click(&mut app, &mut time, r);
    if app.documents[0].animation.is_none() {
        return Err(format!(
            "Timeline pointer creation failed: {}",
            app.timeline_editor.notice
        ));
    }
    app.prepare_timeline(&ctx, Edit::NewTrack("Track A".into()), false)?;
    wait(&mut app, &mut time)?;
    for _ in 0..3 {
        frame(&mut app, &mut time, vec![]);
    }
    let r = app_rect(&app.timeline_editor.apply)?;
    click(&mut app, &mut time, r);
    if app.documents[0].scene.cels.definitions.is_empty() {
        return Err(format!(
            "New track pointer apply failed at {r:?}, pending={}, notice={}",
            app.timeline_editor.staged.is_some(),
            app.timeline_editor.notice
        ));
    }
    let track = app.documents[0].scene.catalog.tracks.id(0);
    let cel = app.documents[0].scene.cels.definitions[0].object;
    let state = cels::replace(
        &app.documents[0],
        cel,
        &drawer_core::timeline_verify::child(),
        DEFAULT_DOCUMENT_BUDGET,
    )?;
    app.documents[0].commit_state(state)?;
    app.prepare_timeline(
        &ctx,
        Edit::CopyTrack {
            track,
            independent: false,
        },
        false,
    )?;
    wait(&mut app, &mut time)?;
    app.apply_timeline()?;
    let typed = drawer_core::timeline_verify::child();
    let packet = drawer_core::vector_clipboard::Packet {
        transform: Default::default(),
        paths: typed.scene.lines.layers[0].paths.clone(),
    };
    let vector = app.timeline_clipboard_with(Some(packet), || {
        Err("Must not read pixels for vector content".into())
    })?;
    if vector
        .scene
        .lines
        .layers
        .iter()
        .flat_map(|l| &l.paths)
        .count()
        != 1
    {
        return Err("Vector cel clipboard flattened paths".into());
    }
    let pixels = [[120, 20, 30, 128], [0, 0, 0, 0]];
    let raster = app.timeline_clipboard_with(None, || {
        Ok(drawer_core::paste::ClipboardImage {
            color: None,
            origin: None,
            image: drawer_core::references::LocalImage {
                width: 2,
                height: 1,
                pixels: pixels.to_vec(),
            },
        })
    })?;
    if raster.composite_pixel(0, 0) != pixels[0] || raster.composite_pixel(1, 0) != pixels[1] {
        return Err("Pixel cel clipboard lost alpha or colors".into());
    }
    let state = core::prepare(
        &app.documents[0],
        Edit::Paste {
            track,
            replace: None,
            document: Box::new(vector),
        },
        DEFAULT_DOCUMENT_BUDGET,
    )?
    .state;
    let original = app.documents[0].snapshot();
    app.documents[0].commit_state(state)?;
    if !app.documents[0].undo() || app.documents[0].snapshot() != original {
        return Err("Vector cel paste Undo lost reference state".into());
    }
    app.timeline_editor.section = 1;
    {
        let v = app.timeline_editor.views.entry(parent).or_default();
        v.track = Some(track);
        v.source = Some(cel);
        v.onion.enabled = true;
    }
    for _ in 0..3 {
        frame(&mut app, &mut time, vec![]);
    }
    let r = app_rect(&app.timeline_editor.open_cel)?;
    click(&mut app, &mut time, r);
    if app.timeline_editor.binding.is_none() {
        return Err(format!(
            "Cel pointer open failed at {r:?}: {}",
            app.timeline_editor.notice
        ));
    }
    if usage(&app.documents[0], cel) != (10, 2)
        || !app
            .timeline_editor
            .notice
            .contains("10 frames across 2 tracks")
    {
        return Err("Shared cel frame/track usage counts omitted held frames".into());
    }
    let before = app.documents[0].snapshot();
    let history = app.documents[0].history_counts();
    let child = app.active.unwrap();
    app.documents[child].edit_pixel(0, 0, 0, [250, 1, 2, 255]);
    let content = app.documents[child].snapshot();
    app.timeline_editor.clipboard = Some(Box::new(drawer_core::layer_merge::preview(&content, 0)?));
    if app.timeline_clipboard(&ctx)?.snapshot() != content {
        return Err("Editable clipboard lost typed content".into());
    }
    for _ in 0..3 {
        frame(&mut app, &mut time, vec![]);
    }
    if !app.timeline_editor.overlay_active
        || app
            .canvas_view
            .timeline_overlay
            .as_ref()
            .is_none_or(|o| o.document != app.documents[child].id)
    {
        return Err("Cel content canvas did not receive onion view".into());
    }
    app.documents[0].current_frame = 3;
    let r = app_rect(&app.timeline_editor.content_apply)?;
    click(&mut app, &mut time, r);
    if app.timeline_editor.binding.is_some()
        || app.documents[0].history_counts().0 != history.0 + 1
        || app.documents[0]
            .scene
            .cels
            .get(cel)
            .unwrap()
            .document()
            .unwrap()
            .snapshot()
            != content
    {
        return Err(format!(
            "Shared content pointer apply failed: {}",
            app.timeline_editor.notice
        ));
    }
    if !app.documents[0].undo() || app.documents[0].snapshot() != before || !app.documents[0].redo()
    {
        return Err("Parent shared edit is not one Undo/Redo".into());
    }
    app.timeline_editor.section = 2;
    {
        let v = app.timeline_editor.views.entry(parent).or_default();
        v.only = true;
        v.onion.enabled = true;
    }
    frame(&mut app, &mut time, vec![]);
    let plain = app.documents[0].snapshot();
    let h = app.documents[0].history_counts();
    if !app.timeline_editor.overlay_active
        || !app
            .canvas_view
            .timeline_overlay
            .as_ref()
            .is_some_and(|o| o.replace)
    {
        return Err("Only-cell canvas overlay not active".into());
    }
    let r = app_rect(&app.timeline_editor.play)?;
    click(&mut app, &mut time, r);
    for _ in 0..4 {
        frame(&mut app, &mut time, vec![]);
    }
    if app.documents[0].snapshot() != plain
        || app.documents[0].history_counts() != h
        || app.documents[0].current_frame == 3
    {
        return Err("Playback changed content/history or failed to advance".into());
    }
    app.timeline_editor.views.get_mut(&parent).unwrap().playing = None;
    app.prepare_timeline(
        &ctx,
        Edit::Label {
            name: "UI label".into(),
            frame: 2,
        },
        false,
    )?;
    wait(&mut app, &mut time)?;
    app.documents[0].current_frame = (app.documents[0].current_frame + 1) % 10;
    if app.apply_timeline().is_ok() || app.documents[0].snapshot() != plain {
        return Err("Stale-frame prepared edit accepted".into());
    }
    app.timeline_editor.staged = None;
    app.prepare_timeline(&ctx, Edit::DeleteCel(cel), true)?;
    wait(&mut app, &mut time)?;
    if app.apply_timeline().is_ok() {
        return Err("Global deletion bypassed review".into());
    }
    app.timeline_editor.confirm = true;
    app.apply_timeline()?;
    if app.documents[0].scene.cels.get(cel).is_some()
        || !app.documents[0].undo()
        || app.documents[0].snapshot() != plain
    {
        return Err("Global delete Undo failed".into());
    }
    app.prepare_timeline(&ctx, Edit::NewTrack("Cancel".into()), false)?;
    app.timeline_editor
        .job
        .as_ref()
        .unwrap()
        .cancel
        .store(true, Ordering::Relaxed);
    let end = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while app.timeline_editor.job.is_some() {
        frame(&mut app, &mut time, vec![]);
        if std::time::Instant::now() > end {
            return Err("Cancel timeout".into());
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    if app.timeline_editor.staged.is_some() || app.documents[0].snapshot() != plain {
        return Err("Cancelled timeline edit changed drawing".into());
    }
    // Each document owns its playback/view flags independently.
    let other = app.documents[child].id;
    app.return_timeline_cel();
    let tab = app
        .tabs
        .iter()
        .position(|t| t.document_id == other)
        .unwrap();
    app.select_tab(tab);
    frame(&mut app, &mut time, vec![]);
    if app.timeline_editor.overlay_active {
        return Err("Timeline view leaked into another document".into());
    }
    let tab = app
        .tabs
        .iter()
        .position(|t| t.document_id == parent)
        .unwrap();
    app.select_tab(tab);
    app.timeline_editor.open = false;
    frame(&mut app, &mut time, vec![]);
    if app.canvas_view.timeline_overlay.is_some() || app.documents[0].snapshot() != plain {
        return Err("Closing timeline left a canvas override or changed content".into());
    }
    Ok(())
}
fn app_rect(r: &Option<egui::Rect>) -> Result<egui::Rect, String> {
    r.ok_or_else(|| "Timeline confirmation control absent".into())
}
#[cfg(test)]
mod tests {
    #[test]
    fn pointer_timeline_shared_content_playback_views_cancel_and_stale_review() {
        let root = std::env::temp_dir().join(format!("drawer-timeline-ui-{}", std::process::id()));
        std::fs::create_dir(&root).unwrap();
        let result = super::smoke(&root);
        std::fs::remove_dir(&root).unwrap();
        result.unwrap();
    }
}
