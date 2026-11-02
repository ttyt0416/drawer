use super::*;
use drawer_core::world_views::{self as core, Direction, Preferences, View};
use std::{
    collections::BTreeMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
};
#[derive(Clone, PartialEq)]
struct Key {
    document: u64,
    revision: u64,
    frame: u32,
    preferences: Preferences,
}
struct Job {
    key: Key,
    cancel: Arc<AtomicBool>,
    rx: mpsc::Receiver<Result<Vec<core::Frame>, String>>,
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
#[derive(Default)]
pub(super) struct Panel {
    pub open: bool,
    preferences: Preferences,
    job: Option<Job>,
    key: Option<Key>,
    frames: Vec<core::Frame>,
    textures: Vec<egui::TextureHandle>,
    undo: Vec<Preferences>,
    redo: Vec<Preferences>,
    notice: String,
    pub controls: BTreeMap<String, egui::Rect>,
    pub guide_count: usize,
}

impl Panel {
    fn clear(&mut self) {
        self.job = None;
        self.key = None;
        self.frames.clear();
        self.textures.clear();
        self.guide_count = 0;
    }
    fn remember(&mut self, old: Preferences) {
        if old != self.preferences {
            self.undo.push(old);
            if self.undo.len() > 32 {
                self.undo.remove(0);
            }
            self.redo.clear();
            self.clear();
        }
    }
}
impl Drawer {
    pub(super) fn load_world_views(&mut self, storage: &UserStorage) {
        match Preferences::load(storage.root()) {
            Ok(p) => self.world_views.preferences = p,
            Err(e) => self.world_views.notice = e,
        }
    }
    pub(super) fn begin_world_views(&mut self) -> Result<(), String> {
        let n = self.active.ok_or("Choose a document")?;
        self.stop_timeline_playback(self.documents[n].id);
        self.world_views.preferences.initialize(&self.documents[n]);
        self.world_views.open = true;
        Ok(())
    }
    fn world_views_budget(&self) -> usize {
        DEFAULT_DOCUMENT_BUDGET.saturating_sub(
            self.documents
                .iter()
                .map(Document::resident_bytes)
                .sum::<usize>(),
        )
    }
    fn start_world_views(&mut self, ctx: &egui::Context) -> Result<(), String> {
        let n = self.active.ok_or("Document closed")?;
        let d = &self.documents[n];
        let budget = self.world_views_budget();
        if d.memory_bytes().saturating_mul(3) > budget {
            return Err("Multiple-camera snapshot exceeds memory budget".into());
        }
        let key = Key {
            document: d.id,
            revision: d.revision,
            frame: d.current_frame,
            preferences: self.world_views.preferences.clone(),
        };
        let mut frozen = drawer_core::layer_merge::preview(&d.snapshot(), d.current_frame)?;
        frozen.keyframe_content = d.keyframe_content;
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        let prefs = key.preferences.clone();
        let (tx, rx) = mpsc::channel();
        let ctx = ctx.clone();
        let worker = std::thread::spawn(move || {
            let result = core::render(&frozen, &prefs, budget, &flag);
            let _ = tx.send(result);
            ctx.request_repaint();
        });
        self.world_views.job = Some(Job {
            key,
            cancel,
            rx,
            worker: Some(worker),
        });
        Ok(())
    }
    fn poll_world_views(&mut self, ctx: &egui::Context) {
        let Some(n) = self.active else {
            self.world_views.clear();
            self.world_views.open = false;
            return;
        };
        let d = &self.documents[n];
        let current = Key {
            document: d.id,
            revision: d.revision,
            frame: d.current_frame,
            preferences: self.world_views.preferences.clone(),
        };
        if self
            .world_views
            .key
            .as_ref()
            .is_some_and(|k| k.document != current.document)
        {
            self.world_views.clear();
        }
        if self
            .world_views
            .job
            .as_ref()
            .is_some_and(|j| j.key != current)
        {
            self.world_views.clear();
        }
        let result = self
            .world_views
            .job
            .as_ref()
            .and_then(|j| j.rx.try_recv().ok());
        if let Some(result) = result {
            let job = self.world_views.job.take().unwrap();
            let canceled = job.cancel.load(Ordering::Relaxed);
            let key = job.key.clone();
            drop(job);
            if !canceled && key == current {
                self.world_views.key = Some(key);
                match result {
                    Ok(frames) => {
                        self.world_views.textures = frames
                            .iter()
                            .enumerate()
                            .map(|(i, f)| {
                                ctx.load_texture(
                                    format!("world-view-{i}"),
                                    egui::ColorImage::from_rgba_unmultiplied(
                                        [f.width as usize, f.height as usize],
                                        &f.pixels,
                                    ),
                                    egui::TextureOptions::NEAREST,
                                )
                            })
                            .collect();
                        self.world_views.frames = frames;
                        self.world_views.notice =
                            "Shared committed scene; editing cameras affect workspace views".into();
                    }
                    Err(e) => {
                        self.world_views.frames.clear();
                        self.world_views.textures.clear();
                        self.world_views.notice = e;
                    }
                }
            }
        }
        if self.world_views.open
            && self.world_views.job.is_none()
            && self.world_views.key.as_ref() != Some(&current)
            && let Err(e) = self.start_world_views(ctx)
        {
            self.world_views.key = Some(current);
            self.world_views.notice = e;
        }
        if self.world_views.job.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(20));
        }
    }
    fn copy_view_to_scene(&mut self) -> Result<(), String> {
        if self.environment_pending() {
            return Err("Apply/discard environment preview first".into());
        }
        if self.has_world_preview()
            || self.models_busy()
            || self.rig_pending()
            || self.lighting_pending()
            || self.uv_pending()
            || self.line_tones_pending()
        {
            return Err("Apply/discard other3D previews before replacing the scene camera".into());
        }
        self.finish_stroke();
        if self.timeline_has_cel_binding()
            || self.transform_unconfirmed()
            || self.text_editor.draft.is_some()
            || self.inference_unconfirmed()
        {
            return Err("Finish pending content edits first".into());
        }
        let n = self.active.ok_or("Choose a document")?;
        let view = &self.world_views.preferences.views[self.world_views.preferences.selected];
        let p = core::copy_to_scene(&self.documents[n], view, self.world_views_budget())?;
        p.commit(&mut self.documents[n])?;
        Ok(())
    }
    pub(super) fn world_views_window(&mut self, ctx: &egui::Context) {
        if !self.world_views.open {
            return;
        }
        self.poll_world_views(ctx);
        if !self.world_views.open {
            return;
        }
        let Some(n) = self.active else { return };
        let d = &self.documents[n];
        let target = core::target(d);
        let mut p = std::mem::take(&mut self.world_views);
        let before = p.preferences.clone();
        let mut action = 0;
        let mut open = true;
        p.controls.clear();
        p.guide_count = 0;
        egui::Window::new("3D shared scene / editing cameras").default_pos(egui::pos2(15.,15.)).default_width(760.).min_width(640.).max_height(650.).vscroll(true).open(&mut open).show(ctx,|ui|{
            ui.add_sized([720.,20.],egui::Label::new(if p.job.is_some(){"Updating shared scene views…"}else{&p.notice}).truncate()).on_hover_text(&p.notice);
            ui.horizontal_wrapped(|ui|{for(key,label,a)in[("save","Save local view workspace",1),("undo","Undo view",2),("redo","Redo view",3),("to-scene","Copy selected view to scene camera",4),("from-scene","Copy scene camera to selected view once",5),("reset","Reset split layout",6)]{let r=ui.button(label);p.controls.insert(key.into(),r.rect);if r.clicked(){action=a;}}});
            ui.horizontal_wrapped(|ui|{ui.label("Views");ui.selectable_value(&mut p.preferences.count,2,"2");ui.selectable_value(&mut p.preferences.count,4,"4");p.preferences.selected=p.preferences.selected.min(p.preferences.count-1);p.preferences.reference=p.preferences.reference.min(p.preferences.count-1);ui.add(egui::Slider::new(&mut p.preferences.split,0.2..=0.8).text("Left split"));ui.checkbox(&mut p.preferences.camera_guides,"Scene camera / area guides");ui.checkbox(&mut p.preferences.edges,"Face-edge guides");});
            ui.horizontal_wrapped(|ui|{egui::ComboBox::from_label("Link reference").selected_text(format!("View{}",p.preferences.reference+1)).show_ui(ui,|ui|{for i in 0..p.preferences.count{ui.selectable_value(&mut p.preferences.reference,i,format!("View{}",i+1));}});let r=ui.button("Link all using chosen reference");p.controls.insert("link".into(),r.rect);if r.clicked(){action=7;}let r=ui.button("Make cameras independent");p.controls.insert("independent".into(),r.rect);if r.clicked(){p.preferences.linked=false;}ui.label(if p.preferences.linked{"Linked: changing a view updates all editing cameras"}else{"Independent editing cameras"});});
            ui.horizontal_wrapped(|ui|{ui.label(format!("Selected View{}",p.preferences.selected+1));for dir in Direction::ALL{let r=ui.button(dir.label());p.controls.insert(dir.label().into(),r.rect);if r.clicked(){let mut v=p.preferences.views[p.preferences.selected].clone();v.direction(dir,target);if let Err(e)=p.preferences.replace(p.preferences.selected,v){p.notice=e;}}}for(key,label)in[("yaw","Orbit +15°"),("pan","Pan right")]{let r=ui.button(label);p.controls.insert(key.into(),r.rect);if r.clicked(){let mut v=p.preferences.views[p.preferences.selected].clone();if key=="yaw"{v.lens.angles[1]+=15.;let delta=drawer_core::world_math::sub(v.camera.position,target);let distance=drawer_core::world_math::dot(delta,delta).sqrt();v.camera.position=drawer_core::world_math::add(target,drawer_core::world_math::Matrix::euler(v.lens.angles).vector([0.,0.,distance]));}else{v.camera.position[0]+=0.25;}
if let Err(e)=p.preferences.replace(p.preferences.selected,v){p.notice=e;}}}});
            let selected=p.preferences.selected;let mut view=p.preferences.views[selected].clone();let old=view.clone();ui.horizontal_wrapped(|ui|{ui.label("Position X/Y/Z");for v in &mut view.camera.position{ui.add(egui::DragValue::new(v).speed(0.1));}ui.label("Rotation X/Y/Z");for v in &mut view.lens.angles{ui.add(egui::DragValue::new(v));}});ui.horizontal_wrapped(|ui|{ui.selectable_value(&mut view.lens.projection,drawer_core::world3d::Projection::Perspective,"Perspective projection");ui.selectable_value(&mut view.lens.projection,drawer_core::world3d::Projection::Orthographic,"Parallel projection");ui.label("FOV / ortho pixels per unit");ui.add(egui::DragValue::new(&mut view.lens.fov).range(1. ..=179.));ui.add(egui::DragValue::new(&mut view.camera.pixels_per_unit).range(1e-6..=1e6));});if view!=old&&let Err(e)=p.preferences.replace(selected,view){p.notice=e;}
            let width=ui.available_width().min(735.);let height=if p.preferences.count==4{280.}else{180.};let(area,_)=ui.allocate_exact_size(egui::vec2(width,height),egui::Sense::hover());let gap=4.;let left=(width-gap)*p.preferences.split;let rows=if p.preferences.count==4{2}else{1};let row_height=(height-gap*(rows-1) as f32)/rows as f32;
            for i in 0..p.preferences.count{let row=i/2;let col=i%2;let x=if col==0{area.min.x}else{area.min.x+left+gap};let w=if col==0{left}else{width-left-gap};let rect=egui::Rect::from_min_size(egui::pos2(x,area.min.y+row as f32*(row_height+gap)),egui::vec2(w.max(1.),row_height.max(1.)));p.controls.insert(format!("pane{i}"),rect);let response=ui.interact(rect,ui.id().with(("world-pane",i)),egui::Sense::click());if response.clicked(){p.preferences.selected=i;}let painter=ui.painter_at(rect);painter.rect_filled(rect,2.,egui::Color32::from_rgb(24,29,34));painter.text(rect.min+egui::vec2(5.,3.),egui::Align2::LEFT_TOP,format!("View {}{}",i+1,if p.preferences.selected==i{" • selected"}else{""}),egui::FontId::proportional(12.),egui::Color32::WHITE);
                if let(Some(texture),Some(frame))=(p.textures.get(i),p.frames.get(i)){let inside=egui::Rect::from_min_max(rect.min+egui::vec2(2.,22.),rect.max-egui::vec2(2.,2.));let scale=(inside.width()/frame.width as f32).min(inside.height()/frame.height as f32).max(0.);let image=egui::Rect::from_center_size(inside.center(),egui::vec2(frame.width as f32*scale,frame.height as f32*scale));painter.image(texture.id(),image,egui::Rect::from_min_max(egui::Pos2::ZERO,egui::pos2(1.,1.)),egui::Color32::WHITE);let overlay=painter.with_clip_rect(image);for(a,b,camera)in &frame.guides{let pos=|v:[f64;2]|egui::pos2(image.min.x+v[0] as f32*image.width(),image.min.y+v[1] as f32*image.height());overlay.line_segment([pos(*a),pos(*b)],egui::Stroke::new(1.,if *camera{egui::Color32::LIGHT_YELLOW}else{egui::Color32::from_white_alpha(100)}));p.guide_count+=1;}}
            }
            ui.small("Scene geometry is shared. Editing cameras and guides are workspace state. A one-time copy does not start linking; only copying to the scene camera changes the document (one Undo). Guides are excluded from output.");
        });
        p.open = open;
        if !open {
            p.clear();
        }
        if action != 2 && action != 3 {
            p.remember(before);
        }
        match action {
            2 => {
                if let Some(old) = p.undo.pop() {
                    p.redo.push(p.preferences.clone());
                    p.preferences = old;
                    p.clear();
                }
            }
            3 => {
                if let Some(next) = p.redo.pop() {
                    p.undo.push(p.preferences.clone());
                    p.preferences = next;
                    p.clear();
                }
            }
            5 => {
                let old = p.preferences.clone();
                match p
                    .preferences
                    .replace(p.preferences.selected, View::scene(d))
                {
                    Ok(()) => p.remember(old),
                    Err(e) => p.notice = e,
                }
            }
            6 => {
                let old = p.preferences.clone();
                p.preferences.reset_split();
                p.remember(old);
            }
            7 => {
                let old = p.preferences.clone();
                match p.preferences.link(p.preferences.reference) {
                    Ok(()) => p.remember(old),
                    Err(e) => p.notice = e,
                }
            }
            _ => {}
        }
        self.world_views = p;
        let result = match action {
            1 => self
                .storage
                .as_ref()
                .ok_or_else(|| "Local storage unavailable".to_owned())
                .and_then(|s| self.world_views.preferences.save(s.root())),
            4 => self.copy_view_to_scene(),
            _ => Ok(()),
        };
        if let Err(e) = result {
            self.world_views.notice = e;
        }
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.world_views.open = false;
            self.world_views.clear();
        }
    }
}
#[path = "world_views_verify.rs"]
mod verify;
pub(super) fn smoke(root: &std::path::Path) -> Result<(), String> {
    verify::run(root)
}
#[cfg(test)]
mod tests {
    #[test]
    fn pointer_independent_link_copy_restore_shared_refresh_and_restart() {
        let t = crate::tests::TestDirectory::new();
        super::smoke(&t.0).unwrap();
    }
}
