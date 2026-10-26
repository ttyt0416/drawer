use super::*;
use drawer_core::{
    comics as c,
    scene::{PageObject, Rect},
    sizing::DocumentState,
    webtoon_band::Band,
};
#[path = "comic_controls.rs"]
mod controls;
#[derive(Default)]
pub(super) struct Panel {
    draft: Option<Box<Draft>>,
    job: Option<Job>,
    pub preview: Option<egui::Rect>,
    pub apply: Option<egui::Rect>,
}
struct Job {
    cancel: Arc<std::sync::atomic::AtomicBool>,
    worker: std::thread::JoinHandle<Result<Vec<PathBuf>, String>>,
}
struct Draft {
    document: u64,
    revision: u64,
    frame: u32,
    id: Option<u64>,
    object: c::Object,
    page: c::PageInfo,
    webtoon: c::Webtoon,
    cut: String,
    manual: String,
    band_at: u32,
    band_end: u32,
    remove: bool,
    confirmed: bool,
    prepared: Option<DocumentState>,
    image: Option<egui::TextureHandle>,
    notice: String,
    viewport: drawer_core::comic_view::Viewport,
    before_view: drawer_core::view::View,
    preview_dpi: f64,
}
fn points(s: &str) -> Result<Vec<[f64; 2]>, String> {
    let mut out = vec![];
    for line in s.lines().filter(|s| !s.trim().is_empty()) {
        let v = line
            .split(|c: char| c == ',' || c.is_whitespace())
            .filter(|s| !s.is_empty())
            .map(str::parse::<f64>)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| "Use one X,Y point per line")?;
        if v.len() != 2 {
            return Err("Use one X,Y point per line".into());
        }
        out.push([v[0], v[1]]);
    }
    Ok(out)
}
fn cuts(s: &str) -> Result<Vec<u32>, String> {
    let mut out = s
        .split(|c: char| c == ',' || c.is_whitespace())
        .filter(|s| !s.is_empty())
        .map(str::parse::<u32>)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| "Use comma-separated pixel boundaries")?;
    out.sort_unstable();
    out.dedup();
    Ok(out)
}
impl Drawer {
    pub(super) fn begin_comics(&mut self) -> Result<(), String> {
        self.finish_stroke();
        if self.transform_unconfirmed()
            || self.inference_unconfirmed()
            || self.text_editor.draft.is_some()
        {
            return Err("Apply or cancel the pending edit first".into());
        }
        let d = &self.documents[self.active.ok_or("Open a page")?];
        let object = c::defaults(Rect {
            min: [d.width as f64 * 0.1, d.height as f64 * 0.1],
            max: [d.width as f64 * 0.8, d.height as f64 * 0.6],
        })[0]
            .clone();
        self.comics.draft = Some(Box::new(Draft {
            document: d.id,
            revision: d.revision,
            frame: d.current_frame,
            id: None,
            object,
            page: d.scene.comics.page.clone(),
            webtoon: d.scene.comics.webtoon.clone(),
            cut: String::new(),
            manual: d
                .scene
                .comics
                .webtoon
                .cuts
                .iter()
                .map(u32::to_string)
                .collect::<Vec<_>>()
                .join(","),
            band_at: d.height / 2,
            band_end: 64,
            remove: false,
            confirmed: false,
            prepared: None,
            image: None,
            notice: String::new(),
            viewport: self
                .workspaces
                .store
                .current
                .comic_view
                .get(d.path.as_deref()),
            before_view: d.view,
            preview_dpi: 75.,
        }));
        Ok(())
    }
    pub(super) fn comics_window(&mut self, ctx: &egui::Context) {
        if self
            .comics
            .job
            .as_ref()
            .is_some_and(|j| j.worker.is_finished())
        {
            let job = self.comics.job.take().unwrap();
            self.status = Some(match job.worker.join() {
                Ok(Ok(paths)) => format!("Exported {} local PNG strips", paths.len()),
                Ok(Err(e)) => e,
                Err(_) => "Webtoon output worker failed".into(),
            });
        }
        if let Some(job) = &self.comics.job {
            egui::Window::new("Webtoon output").show(ctx, |ui| {
                ui.label("Preparing local PNG strips…");
                if ui.button("Cancel output and remove new results").clicked() {
                    job.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
                }
            });
            ctx.request_repaint_after(std::time::Duration::from_millis(50));
        }
        let Some(mut e) = self.comics.draft.take() else {
            return;
        };
        let Some(n) = self.active.filter(|n| self.documents[*n].id == e.document) else {
            self.comics.draft = Some(e);
            return;
        };
        let d = &self.documents[n];
        let fresh = d.revision == e.revision && d.current_frame == e.frame;
        let mut action = 0;
        let mut open = true;
        let old = (
            e.object.clone(),
            e.page.clone(),
            e.webtoon.clone(),
            e.manual.clone(),
            e.cut.clone(),
            e.band_at,
            e.band_end,
            e.remove,
            e.confirmed,
            e.preview_dpi,
        );
        egui::Window::new("Comic composition").id(egui::Id::new("comic-editor")).open(&mut open).default_width(650.).default_height(650.).show(ctx,|ui|{
   if !fresh{ui.label("Page changed. Close and reopen this editor.");return;}
   egui::ScrollArea::vertical().id_salt("comic-editor-scroll").max_height(480.).show(ui,|ui|{
    ui.horizontal_wrapped(|ui|{for(i,label)in ["New balloon","New frame folder","New tone","New effect lines"].iter().enumerate(){if ui.button(*label).clicked(){let r=e.object.local_bounds();e.object=c::defaults(r)[i].clone();e.id=None;e.prepared=None;e.image=None;}}});
    egui::ComboBox::from_label("Edit existing object").selected_text(e.id.map_or_else(||"New object".into(),|id|format!("Object#{id}"))).show_ui(ui,|ui|{for(i,o)in d.scene.objects.iter().enumerate(){if let PageObject::Comic(c)=o{let id=d.scene.catalog.objects.id(i);if ui.selectable_label(e.id==Some(id),format!("{} · #{id}",c.name)).clicked(){e.id=Some(id);e.object=(**c).clone();e.prepared=None;e.image=None;}}}});
    controls::object(ui,d,&mut e);
    ui.add(egui::DragValue::new(&mut e.preview_dpi).range(1. ..=4800.).prefix("Tone / composite preview DPI "));
    ui.separator();ui.collapsing("Split selected frame",|ui|{ui.label("Local coordinates: one X,Y per line. Start and finish outside the frame.");ui.add(egui::TextEdit::multiline(&mut e.cut).desired_rows(3));if ui.add_enabled(e.id.is_some(),egui::Button::new("Preview frame cut")).clicked(){action=2;}});
    ui.collapsing("Page, trim and binding",|ui|{controls::page(ui,&mut e.page);ui.label(format!("Side: {} · facing page: {:?}",if e.page.is_right(){"right"}else{"left"},e.page.facing()));});
    ui.collapsing("Webtoon split output",|ui|{ui.add(egui::DragValue::new(&mut e.webtoon.slice).range(1..=32768).prefix("Maximum strip height px "));ui.text_edit_singleline(&mut e.webtoon.prefix);ui.label("Additional manual boundaries (px)");ui.text_edit_singleline(&mut e.manual);if let Ok(cuts)=cuts(&e.manual){let mut s=e.webtoon.clone();s.cuts=cuts;if let Ok(plan)=drawer_core::webtoon::plan(d.height,&s){for strip in plan.iter().take(32){ui.label(format!("{}: rows {}..{}",strip.name,strip.start,strip.end));}
if plan.len()>32{ui.label(format!("{} strips in total",plan.len()));}}}
if ui.button("Preview page / split settings").clicked(){action=3;}
if ui.add_enabled(self.comics.job.is_none(),egui::Button::new("Export local PNG strips…")).clicked(){action=5;}ui.small("Manual and regular boundaries are combined. Each original row is exported exactly once. Existing filenames are never replaced.");});
    ui.collapsing("Webtoon viewport guide (view only)",|ui|{ui.checkbox(&mut e.viewport.enabled,"Show one-screen region");ui.horizontal(|ui|{ui.add(egui::DragValue::new(&mut e.viewport.width).range(1..=32768).prefix("Screen width "));ui.add(egui::DragValue::new(&mut e.viewport.height).range(1..=32768).prefix("Screen height "));});if ui.button("Apply viewport guide").clicked(){action=6;}
if ui.button("Restore original view and hide guide").clicked(){action=7;}});
    ui.collapsing("Insert blank band / remove horizontal strip",|ui|{ui.checkbox(&mut e.remove,"Remove a strip");ui.add(egui::DragValue::new(&mut e.band_at).range(0..=d.height).prefix("Start Y "));ui.add(egui::DragValue::new(&mut e.band_end).range(0..=32768).prefix(if e.remove{"End Y "}else{"Added height "}));let band=if e.remove{Band::Remove{start:e.band_at,end:e.band_end}}else{Band::Insert{at:e.band_at,height:e.band_end}};match drawer_core::webtoon_band::impact(d,band){Ok(list)=>for name in list{ui.label(name);},Err(error)=>{ui.colored_label(egui::Color32::YELLOW,error);}}
if e.remove{ui.checkbox(&mut e.confirmed,"I confirm removal of the selected strip");}
if ui.button("Preview band edit").clicked(){action=4;}});
    if let Some(image)=&e.image{ui.image((image.id(),egui::vec2(160.,160.)));}
if !e.notice.is_empty(){ui.label(&e.notice);}
   });
   if old!=(e.object.clone(),e.page.clone(),e.webtoon.clone(),e.manual.clone(),e.cut.clone(),e.band_at,e.band_end,e.remove,e.confirmed,e.preview_dpi){e.prepared=None;e.image=None;}
   ui.horizontal(|ui|{let r=ui.button("Preview comic object");self.comics.preview=Some(r.rect);if r.clicked(){action=1;}let r=ui.add_enabled(fresh&&e.prepared.is_some(),egui::Button::new("Apply as one Undo"));self.comics.apply=Some(r.rect);if r.clicked(){action=8;}
if ui.button("Close editor").clicked(){action=9;}});
  });
        let result = (|| -> Result<(), String> {
            if !fresh && action != 0 && action != 9 {
                return Err("Page changed; reopen the editor".into());
            }
            if (1..=4).contains(&action) {
                let d = &self.documents[n];
                let state = match action {
                    1 => c::prepare(d, e.id, e.object.clone())?,
                    2 => drawer_core::comic_split::prepare(
                        d,
                        e.id.ok_or("Select a frame")?,
                        &points(&e.cut)?,
                    )?,
                    3 => {
                        let mut s = d.snapshot();
                        e.webtoon.cuts = cuts(&e.manual)?;
                        drawer_core::webtoon::plan(d.height, &e.webtoon)?;
                        e.page.guides(d)?;
                        s.scene.comics.page = e.page.clone();
                        s.scene.comics.webtoon = e.webtoon.clone();
                        drawer_core::layer_groups::finish(d, &s)?;
                        s
                    }
                    4 => drawer_core::webtoon_band::prepare(
                        d,
                        if e.remove {
                            Band::Remove {
                                start: e.band_at,
                                end: e.band_end,
                            }
                        } else {
                            Band::Insert {
                                at: e.band_at,
                                height: e.band_end,
                            }
                        },
                        e.confirmed,
                    )?,
                    _ => unreachable!(),
                };
                let mut copy = d.clone();
                copy.commit_state(state.clone())?;
                let renderer = drawer_core::render::Renderer::new(&copy);
                if let Some(error) = renderer.error() {
                    return Err(error.into());
                }
                let (pw, ph, output) = renderer.output(e.preview_dpi, 16 * 1024 * 1024)?;
                let mut pixels = Vec::with_capacity(160 * 160);
                for y in 0..160 {
                    for x in 0..160 {
                        let p = output
                            [((y * ph as usize / 160) * pw as usize) + (x * pw as usize / 160)];
                        pixels.push(egui::Color32::from_rgba_unmultiplied(
                            p[0], p[1], p[2], p[3],
                        ));
                    }
                }
                e.image = Some(ctx.load_texture(
                    "comic-preview",
                    egui::ColorImage::new([160, 160], pixels),
                    egui::TextureOptions::NEAREST,
                ));
                e.prepared = Some(state);
                e.notice = format!(
                    "Preview at {} DPI ({} × {}). Apply records one Undo.",
                    e.preview_dpi, pw, ph
                );
            }
            if action == 5 {
                self.text_output_guard(n)?;
                e.webtoon.cuts = cuts(&e.manual)?;
                drawer_core::webtoon::plan(self.documents[n].height, &e.webtoon)?;
                if let Some(folder) = rfd::FileDialog::new().pick_folder() {
                    let d = self.documents[n].clone();
                    let s = e.webtoon.clone();
                    let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
                    let flag = cancel.clone();
                    let worker = std::thread::spawn(move || {
                        drawer_core::webtoon::export(&d, &s, &folder, &mut |_, _| {
                            if flag.load(std::sync::atomic::Ordering::Relaxed) {
                                Err("Webtoon output cancelled; new results removed".into())
                            } else {
                                Ok(())
                            }
                        })
                    });
                    self.comics.job = Some(Job { cancel, worker });
                }
            }
            if action == 6 || action == 7 {
                if !e.viewport.valid() {
                    return Err("Invalid viewport dimensions".into());
                }
                if action == 7 {
                    self.documents[n].view = e.before_view;
                    e.viewport.enabled = false;
                }
                self.comics_view_session.insert(e.document, e.viewport);
                if let Some(path) = self.documents[n].path.clone() {
                    self.workspaces
                        .store
                        .current
                        .comic_view
                        .set(path, e.viewport);
                }
                self.persist_workspace()?;
            }
            if action == 8 {
                self.documents[n].commit_state(e.prepared.take().ok_or("Preview first")?)?;
                e.revision = self.documents[n].revision;
                e.image = None;
                self.status = Some("Comic edit applied as one Undo".into());
            }
            Ok(())
        })();
        if let Err(error) = result {
            e.notice = error;
            e.prepared = None;
        }
        if open && action != 9 {
            self.comics.draft = Some(e);
        }
    }
}

pub(super) fn smoke(root: &std::path::Path) -> Result<(), String> {
    drawer_core::comic_verify::verify(root)?;
    let ctx = egui::Context::default();
    let mut app = Drawer::new(&ctx, None);
    app.simple_view = true;
    app.create_canvas(CanvasSpec {
        width: 32,
        height: 64,
        dpi: 300.,
        background: Background::Transparent,
    })
    .map_err(|e| e.to_string())?;
    let mut time = 0.;
    let step = |app: &mut Drawer, time: &mut f64, events| {
        *time += 0.1;
        input_replay::frame(&ctx, app, *time, events);
    };
    let click = |app: &mut Drawer, time: &mut f64, r: egui::Rect| {
        for pressed in [true, false] {
            step(
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
    app.execute_command(&ctx, "comic-composition");
    for _ in 0..3 {
        step(&mut app, &mut time, vec![]);
    }
    let before = app.documents[0].snapshot();
    let r = app.comics.preview.ok_or("Comic preview button missing")?;
    click(&mut app, &mut time, r);
    for _ in 0..3 {
        step(&mut app, &mut time, vec![]);
    }
    if app
        .comics
        .draft
        .as_ref()
        .is_none_or(|d| d.prepared.is_none())
        || app.documents[0].snapshot() != before
    {
        return Err(format!(
            "Comic preview failed or mutated source: {:?}",
            app.comics.draft.as_ref().map(|e| &e.notice)
        ));
    }
    let r = app.comics.apply.ok_or("Comic Apply missing")?;
    click(&mut app, &mut time, r);
    if app.documents[0].scene.objects.len() != 1 || app.documents[0].history_counts().0 != 1 {
        return Err("Comic GUI Apply did not make one edit".into());
    }
    app.documents[0].undo();
    if app.documents[0].snapshot() != before {
        return Err("Comic GUI Undo failed".into());
    }
    Ok(())
}
impl Drop for Panel {
    fn drop(&mut self) {
        if let Some(job) = self.job.take() {
            job.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
            let _ = job.worker.join();
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn comic_real_pointer_preview_apply_and_undo() {
        let root = std::env::temp_dir().join(format!("drawer-comic-ui-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        super::smoke(&root).unwrap();
        std::fs::remove_dir(root).unwrap();
    }
}
