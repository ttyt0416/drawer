use super::*;
use drawer_core::lighting::{self as core, Kind, Light, Settings, Style};
use std::{
    collections::BTreeMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
};
type RenderResult = Result<(Box<core::Prepared>, Vec<u8>), String>;
struct Job {
    stamp: (u64, u64, u32),
    cancel: Arc<AtomicBool>,
    rx: mpsc::Receiver<RenderResult>,
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
    job: Option<Job>,
    object: Option<u64>,
    settings: Settings,
    stamp: Option<(u64, u64, u32)>,
    staged: Option<Box<core::Prepared>>,
    image: Option<egui::TextureHandle>,
    notice: String,
    selected: usize,
    pub controls: BTreeMap<String, egui::Rect>,
}
impl Drawer {
    pub(super) fn lighting_pending(&self) -> bool {
        self.lighting.staged.is_some() || self.lighting.job.is_some()
    }
    pub(super) fn begin_lighting(&mut self) -> Result<(), String> {
        if self.model_edit_pending() {
            return Err("Apply/discard model controls preview first".into());
        }
        self.model_controls.open = false;
        if self.cameras_pending() {
            return Err("Apply/discard scene camera preview first".into());
        }
        self.scene_cameras.open = false;
        if self.sequence_pending() {
            return Err("Apply/discard pose sequence preview first".into());
        }
        self.pose_sequences.open = false;
        if self.environment_pending() {
            return Err("Apply/discard the environment preview first".into());
        }
        self.environment.open = false;
        if self.line_tones_pending() {
            return Err("Apply / discard contour and tone preview first".into());
        }
        self.line_tones.open = false;
        if self.uv_pending() {
            return Err("Apply or discard the UV stroke first".into());
        }
        self.uv_paint.open = false;
        self.world_physics.open = false;
        self.finish_stroke();
        if self.models_busy() || self.has_world_preview() || self.rig_pending() {
            return Err("Finish the current3D preview first".into());
        }
        if self.timeline_has_cel_binding()
            || self.transform_unconfirmed()
            || self.inference_unconfirmed()
            || self.text_editor.draft.is_some()
        {
            return Err("Finish pending content edits first".into());
        }
        let n = self.active.ok_or("Choose a document")?;
        self.stop_timeline_playback(self.documents[n].id);
        self.world_models.open = false;
        self.model_presets.open = false;
        self.human_rig.open = false;
        self.hide_world_for_models();
        if !self.lighting_pending() {
            let d = &self.documents[n];
            self.lighting.settings = d.scene.world3d.lighting.clone().unwrap_or_default();
            self.lighting.stamp = Some((d.id, d.revision, d.current_frame));
            self.lighting.image = None;
        }
        self.lighting.open = true;
        Ok(())
    }
    fn stage_lighting(&mut self, ctx: &egui::Context) -> Result<(), String> {
        if self.lighting.job.is_some() {
            return Err("Wait for/cancel the lighting preparation".into());
        }
        let n = self.active.ok_or("Document closed")?;
        let d = &self.documents[n];
        let budget = DEFAULT_DOCUMENT_BUDGET.saturating_sub(
            self.documents
                .iter()
                .map(Document::resident_bytes)
                .sum::<usize>(),
        );
        if d.memory_bytes()
            .saturating_mul(3)
            .saturating_add(4 * 1024 * 1024)
            > budget
        {
            return Err("Lighting snapshot exceeds memory budget".into());
        }
        let mut frozen = drawer_core::layer_merge::preview(&d.snapshot(), d.current_frame)?;
        frozen.id = d.id;
        frozen.revision = d.revision;
        frozen.keyframe_content = d.keyframe_content;
        let stamp = (d.id, d.revision, d.current_frame);
        let settings = self.lighting.settings.clone();
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        let (tx, rx) = mpsc::channel();
        let ctx = ctx.clone();
        let worker = std::thread::spawn(move || {
            let result = (|| {
                let p = core::prepare(&frozen, settings, budget, &flag)?;
                let mut state = frozen.snapshot();
                state.scene = p.scene().clone();
                let doc = drawer_core::layer_merge::preview(&state, frozen.current_frame)?;
                let render = drawer_core::render::Renderer::new(&doc);
                if let Some(e) = render.error() {
                    return Err(e.into());
                }
                let mut pixels = Vec::with_capacity(128 * 128 * 4);
                for y in 0..128 {
                    if flag.load(Ordering::Relaxed) {
                        return Err("Lighting canceled".into());
                    }
                    for x in 0..128 {
                        pixels.extend(
                            render
                                .display_sample([
                                    (x as f64 + 0.5) * doc.width as f64 / 128.,
                                    (y as f64 + 0.5) * doc.height as f64 / 128.,
                                ])?
                                .rgba8(),
                        );
                    }
                }
                Ok((Box::new(p), pixels))
            })();
            let _ = tx.send(result);
            ctx.request_repaint();
        });
        self.lighting.staged = None;
        self.lighting.image = None;
        self.lighting.job = Some(Job {
            stamp,
            cancel,
            rx,
            worker: Some(worker),
        });
        Ok(())
    }
    pub(super) fn lighting_window(&mut self, ctx: &egui::Context) {
        let Some(n) = self.active else {
            self.lighting.job = None;
            self.lighting.staged = None;
            self.lighting.image = None;
            self.lighting.stamp = None;
            self.lighting.open = false;
            return;
        };
        let d = &self.documents[n];
        if self.lighting.stamp != Some((d.id, d.revision, d.current_frame)) {
            self.lighting.job = None;
            self.lighting.staged = None;
            self.lighting.image = None;
            self.lighting.settings = d.scene.world3d.lighting.clone().unwrap_or_default();
            self.lighting.stamp = Some((d.id, d.revision, d.current_frame));
            self.lighting.notice = "Document changed; unconfirmed lighting discarded".into();
        }
        if !self.lighting.open {
            self.lighting.job = None;
            self.lighting.staged = None;
            self.lighting.image = None;
            return;
        }
        if ctx.input(|i| i.key_pressed(egui::Key::Escape) || !i.focused) {
            self.lighting.job = None;
            self.lighting.staged = None;
            self.lighting.image = None;
        }
        if self
            .lighting
            .job
            .as_ref()
            .is_some_and(|j| j.stamp != (d.id, d.revision, d.current_frame))
        {
            self.lighting.job = None;
        }
        if let Some(result) = self
            .lighting
            .job
            .as_ref()
            .and_then(|j| j.rx.try_recv().ok())
        {
            self.lighting.job = None;
            match result {
                Ok((p, pixels)) => {
                    self.lighting.staged = Some(p);
                    self.lighting.image = Some(ctx.load_texture(
                        "scene-lighting-preview",
                        egui::ColorImage::from_rgba_unmultiplied([128, 128], &pixels),
                        egui::TextureOptions::NEAREST,
                    ));
                    self.lighting.notice="Preview only. Apply makes one Undo; depth-sorted translucent surfaces include front and back faces.".into();
                }
                Err(e) => self.lighting.notice = e,
            }
        }
        let mut p = std::mem::take(&mut self.lighting);
        let mut action = 0;
        let mut open = true;
        let before = p.settings.clone();
        p.controls.clear();
        egui::Window::new("Scene lighting / surfaces and vanishing guides").default_pos(egui::pos2(15.,15.)).default_width(690.).max_height(650.).vscroll(false).open(&mut open).show(ctx,|ui|{
 ui.add_sized([650.,20.],egui::Label::new(&p.notice).truncate()).on_hover_text(&p.notice);ui.horizontal(|ui|{for(key,label,a)in[("preview","Preview lighting",1),("apply","Apply",2),("discard","Discard",3),("add","Add light",4),("remove","Remove light",5),("clear","Zero lights",6)]{let enabled=if a==2{p.staged.is_some()}else if a==4{p.settings.lights.len()<8}else if a==5{!p.settings.lights.is_empty()}else{true};let r=ui.add_enabled(enabled,egui::Button::new(label));p.controls.insert(key.into(),r.rect);if r.clicked(){action=a;}}});
 ui.horizontal_wrapped(|ui|{for style in [Style::Wire,Style::Unlit,Style::Flat,Style::Smooth,Style::Toon]{ui.selectable_value(&mut p.settings.style,style,format!("{style:?}"));}let r=ui.checkbox(&mut p.settings.guides,"Vanishing guides");p.controls.insert("guides".into(),r.rect);ui.checkbox(&mut p.settings.shadows,"Hard shadows");});
 ui.horizontal(|ui|{ui.label("Ambient");let r=ui.add(egui::Slider::new(&mut p.settings.ambient,0. ..=4.));p.controls.insert("ambient".into(),r.rect);ui.label("Toon bands");ui.add(egui::DragValue::new(&mut p.settings.bands).range(2..=8));});
 if let Some(image)=&p.image{ui.image((image.id(),egui::vec2(180.,180.)));}else{ui.allocate_space(egui::vec2(180.,180.));}
 let scroll=egui::ScrollArea::vertical().id_salt("scene-lights").max_height(230.).show(ui,|ui|{ui.label("Zero lights and ambient0 produce opaque black lit surfaces. Unlit uses material color. Point / spot falloff:1/(1+distance²). Shadows use opaque faces. Gizmos remain wireframe.");if !p.settings.lights.is_empty(){p.selected=p.selected.min(p.settings.lights.len()-1);egui::ComboBox::from_id_salt("scene-light-index").selected_text(format!("Light {}",p.selected+1)).show_ui(ui,|ui|{for i in 0..p.settings.lights.len(){ui.selectable_value(&mut p.selected,i,format!("Light {}",i+1));}});let l=&mut p.settings.lights[p.selected];let mut kind=match l.kind{Kind::Directional{..}=>0,Kind::Point{..}=>1,Kind::Spot{..}=>2};let old=kind;ui.horizontal(|ui|{ui.selectable_value(&mut kind,0,"Directional");ui.selectable_value(&mut kind,1,"Point");ui.selectable_value(&mut kind,2,"Spot");let mut rgb=l.color.map(|v|v as f32);if ui.color_edit_button_rgb(&mut rgb).changed(){l.color=rgb.map(|v|v as f64);}});if kind!=old{l.kind=match kind{0=>Kind::Directional{direction:[0.,0.,-1.]},1=>Kind::Point{position:[0.,2.,4.]},_=>Kind::Spot{position:[0.,2.,4.],direction:[0.,-0.5,-1.],angle:45.}};}
 let response=ui.checkbox(&mut l.follow_camera,"Follow scene camera (position / axes)");p.controls.insert("render-follow".into(),response.rect);
 ui.horizontal(|ui|{ui.label("Linear RGB / intensity");ui.add(egui::Slider::new(&mut l.intensity,0. ..=64.));ui.checkbox(&mut l.shadow,"Casts shadow");});
 match &mut l.kind{Kind::Directional{direction}=>vector(ui,"Travel direction",direction),Kind::Point{position}=>vector(ui,"Position",position),Kind::Spot{position,direction,angle}=>{vector(ui,"Position",position);vector(ui,"Travel direction",direction);ui.add(egui::Slider::new(angle,1. ..=89.).text("Half cone angle"));}}
 }
 let advanced=egui::CollapsingHeader::new("Advanced local scene rendering").id_salt("scene-render-options").show(ui,|ui|{
 use drawer_core::scene_render::*;
 let r=p.settings.render.get_or_insert_with(Default::default);
 ui.horizontal_wrapped(|ui|{ui.label("Editing quality (final output stays full)");for(value,label)in[(EditQuality::Normal,"Normal"),(EditQuality::Fast,"Fast"),(EditQuality::HighSpeed,"High speed (other layers too)")]{ui.selectable_value(&mut r.quality,value,label);}});
 ui.horizontal_wrapped(|ui|{ui.label("Shadow receivers");for(value,label)in[(ShadowTarget::Off,"Off"),(ShadowTarget::Models,"3D models only"),(ShadowTarget::Ground,"Ground only"),(ShadowTarget::Both,"Ground and models")]{ui.selectable_value(&mut r.target,value,label);}for(value,label)in[(ShadowQuality::High,"High1024"),(ShadowQuality::Medium,"Medium512"),(ShadowQuality::Low,"Low256")]{ui.selectable_value(&mut r.shadow,value,label);}});
 ui.horizontal_wrapped(|ui|{ui.label("Environment illumination");for(value,label)in[(LightSource::Current,"Current"),(LightSource::SingleColor,"Single-color"),(LightSource::Files,"Files"),(LightSource::Material,"Embedded local material"),(LightSource::Panorama,"Panorama")]{ui.selectable_value(&mut r.source,value,label);}let mut rgb=r.tint.map(|v|v as f32);if ui.color_edit_button_rgb(&mut rgb).changed(){r.tint=rgb.map(|v|v as f64);}});
 ui.small("Files / material / panorama use the embedded local2:1 PNG/JPEG/HDR snapshot from scene-environment. Its rotation/intensity and background remain independent. No cloud assets. Mark a plane model as ground for ground shadows.");
 let mut fog=r.fog.is_some();let response=ui.checkbox(&mut fog,"Fog");p.controls.insert("render-fog".into(),response.rect);if response.changed(){r.fog=if fog{Some(Default::default())}else{None};}
 if let Some(f)=&mut r.fog{ui.horizontal_wrapped(|ui|{for(value,label)in[(FogKind::Depth,"Depth fog"),(FogKind::Distance,"Distance fog"),(FogKind::HeightBelow,"Fog height / from below"),(FogKind::HeightAbove,"Fog height / from above")]{ui.selectable_value(&mut f.kind,value,label);}ui.label("Start / end / density");for x in [&mut f.start,&mut f.end,&mut f.density]{ui.add(egui::DragValue::new(x).speed(0.1));}let mut rgb=f.color.map(|v|v as f32);if ui.color_edit_button_rgb(&mut rgb).changed(){f.color=rgb.map(|v|v as f64);}});}
 ui.horizontal_wrapped(|ui|{ui.label("Outline width0..4 / color");ui.add(egui::DragValue::new(&mut r.outline).range(0..=4));let mut color=egui::Color32::from_rgba_unmultiplied(r.line[0],r.line[1],r.line[2],r.line[3]);if ui.color_edit_button_srgba(&mut color).changed(){r.line=color.to_srgba_unmultiplied();}});
 ui.horizontal_wrapped(|ui|{ui.label("Object");for i in 0..d.scene.worlds.len(){let id=d.scene.catalog.worlds.id(i);let button=ui.selectable_label(p.object==Some(id),format!("Model{id}"));p.controls.insert(format!("render-object-{id}"),button.rect);if button.clicked(){p.object=Some(id);}}});
 if let Some(id)=p.object.filter(|id|d.scene.catalog.worlds.index(*id,d.scene.worlds.len()).is_some()) {let o=r.objects.entry(id).or_default();ui.horizontal_wrapped(|ui|{for(key,label,x)in[("render-cast","Cast shadows",&mut o.cast),("render-receive","Receive shadows",&mut o.receive),("render-ground","Ground receiver role",&mut o.ground)]{let response=ui.checkbox(x,label);p.controls.insert(key.into(),response.rect);}});ui.horizontal_wrapped(|ui|{ui.label("Billboard");for(value,label)in[(Billboard::Off,"Off"),(Billboard::AllAxes,"All axes"),(Billboard::OnlyHorizontal,"Only horizontal")]{ui.selectable_value(&mut o.billboard,value,label);}ui.label("Alpha");for(value,label)in[(Alpha::Source,"Source material"),(Alpha::Opaque,"Opaque"),(Alpha::Threshold,"Remove with threshold"),(Alpha::Blend,"Semi-transparent")]{ui.selectable_value(&mut o.alpha,value,label);}ui.label("Threshold / alpha multiplier");ui.add(egui::DragValue::new(&mut o.threshold).range(0. ..=1.).speed(0.01));ui.add(egui::DragValue::new(&mut o.alpha_scale).range(0. ..=1.).speed(0.01));});}
 ui.small("Consecutive3D model layers share depth-sorted surface composition. A2D layer/file replacement or per-model2D keyframe separates groups. Single-layer samples are isolated. Billboards affect rendering, not saved geometry/physical placement.");
 });p.controls.insert("render-expand".into(),advanced.header_response.rect);
 });p.controls.insert("render-scroll".into(),scroll.inner_rect.intersect(ui.clip_rect()));
 });
        p.open = open;
        if p.settings != before {
            p.job = None;
            p.staged = None;
            p.image = None;
        }
        if !open {
            p.job = None;
            p.staged = None;
            p.image = None;
        }
        self.lighting = p;
        let result = (|| {
            if !open {
                return Ok(());
            }
            match action {
                1 => self.stage_lighting(ctx)?,
                2 => {
                    let p = self
                        .lighting
                        .staged
                        .take()
                        .ok_or("Prepare lighting preview first")?;
                    p.commit(&mut self.documents[n])?;
                    self.lighting.stamp = Some((
                        self.documents[n].id,
                        self.documents[n].revision,
                        self.documents[n].current_frame,
                    ));
                    self.lighting.notice =
                        "Applied lighting / rendering; one Undo restores the scene".into();
                }
                3 => {
                    self.lighting.job = None;
                    self.lighting.staged = None;
                    self.lighting.image = None;
                    self.lighting.settings = self.documents[n]
                        .scene
                        .world3d
                        .lighting
                        .clone()
                        .unwrap_or_default();
                }
                4 => self.lighting.settings.lights.push(Light::default()),
                5 => {
                    if !self.lighting.settings.lights.is_empty() {
                        let i = self
                            .lighting
                            .selected
                            .min(self.lighting.settings.lights.len() - 1);
                        self.lighting.settings.lights.remove(i);
                    }
                }
                6 => self.lighting.settings.lights.clear(),
                _ => {}
            }
            if matches!(action, 4..=6) {
                self.lighting.job = None;
                self.lighting.staged = None;
                self.lighting.image = None;
            }
            Ok::<_, String>(())
        })();
        if self.lighting.job.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(20));
        }
        if let Err(e) = result {
            self.lighting.notice = e;
        }
    }
}
fn vector(ui: &mut egui::Ui, label: &str, v: &mut [f64; 3]) {
    ui.horizontal(|ui| {
        ui.label(label);
        for v in v {
            ui.add(egui::DragValue::new(v).speed(0.1));
        }
    });
}
pub(super) fn paint_guides(
    painter: &egui::Painter,
    d: &Document,
    view: drawer_core::view::View,
    rect: egui::Rect,
    ppp: f64,
) -> usize {
    let size = [d.width as f64, d.height as f64];
    let center = [rect.center().x as f64, rect.center().y as f64];
    let lines = core::guide_lines(&d.scene);
    for (line, axis) in &lines {
        let points = line.map(|p| {
            let p = view.document_to_screen(
                d.scene
                    .transform
                    .then(drawer_core::camera2d::to_world(d, false).inverse())
                    .map(p),
                size,
                center,
                ppp,
            );
            egui::pos2(p[0] as f32, p[1] as f32)
        });
        painter.line_segment(
            points,
            egui::Stroke::new(
                1.,
                [
                    egui::Color32::LIGHT_RED,
                    egui::Color32::LIGHT_GREEN,
                    egui::Color32::LIGHT_BLUE,
                ][*axis],
            ),
        );
    }
    lines.len()
}
#[path = "lighting_verify.rs"]
mod verify;
pub(super) fn smoke(root: &std::path::Path) -> Result<(), String> {
    verify::run(root)?;
    render_verify::run(root)
}
#[cfg(test)]
mod tests {
    #[test]
    fn pointer_lighting_preview_apply_undo_guides_and_stale() {
        let t = crate::tests::TestDirectory::new();
        super::smoke(&t.0).unwrap();
    }
}

#[path = "scene_render_verify.rs"]
mod render_verify;
#[cfg(test)]
mod render_tests {
    #[test]
    fn pointer_scene_render_fog_roles_preview_apply_undo_cancel_stale_and_close() {
        let t = crate::tests::TestDirectory::new();
        super::render_verify::run(&t.0).unwrap();
    }
}
