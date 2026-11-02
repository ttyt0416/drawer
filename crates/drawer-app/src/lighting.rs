use super::*;
use drawer_core::{
    lighting::{self as core, Kind, Light, Settings, Style},
    world3d,
};
use std::{collections::BTreeMap, sync::atomic::AtomicBool};
#[derive(Default)]
pub(super) struct Panel {
    pub open: bool,
    settings: Settings,
    stamp: Option<(u64, u64, u32)>,
    staged: Option<Box<world3d::Prepared>>,
    image: Option<egui::TextureHandle>,
    notice: String,
    selected: usize,
    pub controls: BTreeMap<String, egui::Rect>,
}
impl Drawer {
    pub(super) fn lighting_pending(&self) -> bool {
        self.lighting.staged.is_some()
    }
    pub(super) fn begin_lighting(&mut self) -> Result<(), String> {
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
        self.lighting.staged = None;
        self.lighting.image = None;
        let n = self.active.ok_or("Document closed")?;
        let d = &self.documents[n];
        let budget = DEFAULT_DOCUMENT_BUDGET.saturating_sub(
            self.documents
                .iter()
                .map(Document::resident_bytes)
                .sum::<usize>(),
        );
        let p = core::prepare(
            d,
            self.lighting.settings.clone(),
            budget,
            &AtomicBool::new(false),
        )?;
        let mut state = d.snapshot();
        state.scene = p.scene().clone();
        let doc = drawer_core::layer_merge::preview(&state, d.current_frame)?;
        let render = drawer_core::render::Renderer::new(&doc);
        if let Some(e) = render.error() {
            return Err(e.into());
        }
        let size = 128;
        let mut pixels = vec![];
        for y in 0..size {
            for x in 0..size {
                pixels.extend(render.pixel([
                    (x as f64 + 0.5) * doc.width as f64 / size as f64,
                    (y as f64 + 0.5) * doc.height as f64 / size as f64,
                ]));
            }
        }
        self.lighting.image = Some(ctx.load_texture(
            "scene-lighting-preview",
            egui::ColorImage::from_rgba_unmultiplied([size, size], &pixels),
            egui::TextureOptions::NEAREST,
        ));
        self.lighting.staged = Some(Box::new(p));
        self.lighting.notice =
            "Page preview; Apply commits one undo step. Guides are an overlay.".into();
        Ok(())
    }
    pub(super) fn lighting_window(&mut self, ctx: &egui::Context) {
        let Some(n) = self.active else {
            self.lighting.staged = None;
            self.lighting.image = None;
            self.lighting.stamp = None;
            self.lighting.open = false;
            return;
        };
        let d = &self.documents[n];
        if self.lighting.stamp != Some((d.id, d.revision, d.current_frame)) {
            self.lighting.staged = None;
            self.lighting.image = None;
            self.lighting.settings = d.scene.world3d.lighting.clone().unwrap_or_default();
            self.lighting.stamp = Some((d.id, d.revision, d.current_frame));
            self.lighting.notice = "Document changed; unconfirmed lighting discarded".into();
        }
        if !self.lighting.open {
            self.lighting.staged = None;
            self.lighting.image = None;
            return;
        }
        if ctx.input(|i| i.key_pressed(egui::Key::Escape) || !i.focused) {
            self.lighting.staged = None;
            self.lighting.image = None;
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
 egui::ScrollArea::vertical().id_salt("scene-lights").max_height(230.).show(ui,|ui|{ui.label("Zero lights and ambient0 produce opaque black lit surfaces. Unlit uses material color. Point / spot falloff:1/(1+distance²). Shadows use opaque faces. Gizmos remain wireframe.");if !p.settings.lights.is_empty(){p.selected=p.selected.min(p.settings.lights.len()-1);egui::ComboBox::from_id_salt("scene-light-index").selected_text(format!("Light {}",p.selected+1)).show_ui(ui,|ui|{for i in 0..p.settings.lights.len(){ui.selectable_value(&mut p.selected,i,format!("Light {}",i+1));}});let l=&mut p.settings.lights[p.selected];let mut kind=match l.kind{Kind::Directional{..}=>0,Kind::Point{..}=>1,Kind::Spot{..}=>2};let old=kind;ui.horizontal(|ui|{ui.selectable_value(&mut kind,0,"Directional");ui.selectable_value(&mut kind,1,"Point");ui.selectable_value(&mut kind,2,"Spot");let mut rgb=l.color.map(|v|v as f32);if ui.color_edit_button_rgb(&mut rgb).changed(){l.color=rgb.map(|v|v as f64);}});if kind!=old{l.kind=match kind{0=>Kind::Directional{direction:[0.,0.,-1.]},1=>Kind::Point{position:[0.,2.,4.]},_=>Kind::Spot{position:[0.,2.,4.],direction:[0.,-0.5,-1.],angle:45.}};}
 ui.horizontal(|ui|{ui.label("Linear RGB / intensity");ui.add(egui::Slider::new(&mut l.intensity,0. ..=64.));ui.checkbox(&mut l.shadow,"Casts shadow");});
 match &mut l.kind{Kind::Directional{direction}=>vector(ui,"Travel direction",direction),Kind::Point{position}=>vector(ui,"Position",position),Kind::Spot{position,direction,angle}=>{vector(ui,"Position",position);vector(ui,"Travel direction",direction);ui.add(egui::Slider::new(angle,1. ..=89.).text("Half cone angle"));}}
 }});
 });
        p.open = open;
        if p.settings != before {
            p.staged = None;
            p.image = None;
        }
        if !open {
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
                self.lighting.staged = None;
                self.lighting.image = None;
            }
            Ok::<_, String>(())
        })();
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
    verify::run(root)
}
#[cfg(test)]
mod tests {
    #[test]
    fn pointer_lighting_preview_apply_undo_guides_and_stale() {
        let t = crate::tests::TestDirectory::new();
        super::smoke(&t.0).unwrap();
    }
}
