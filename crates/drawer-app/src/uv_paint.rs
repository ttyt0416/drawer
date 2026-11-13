use super::*;
use drawer_core::{
    uv_paint::{self as core, Brush},
    world3d,
};
use std::{collections::BTreeMap, sync::atomic::AtomicBool};
#[derive(Default)]
pub(super) struct Panel {
    pub open: bool,
    id: Option<u64>,
    brush: Brush,
    stamp: Option<(u64, u64, u32)>,
    staged: Option<Box<world3d::Prepared>>,
    stroke: Option<Vec<[f64; 2]>>,
    image: Option<egui::TextureHandle>,
    uv: Option<egui::TextureHandle>,
    review: bool,
    notices: Option<Result<Vec<String>, String>>,
    blocked: bool,
    notice: String,
    pub controls: BTreeMap<String, egui::Rect>,
}
impl Panel {
    fn clear(&mut self) {
        self.staged = None;
        self.stroke = None;
        self.image = None;
        self.uv = None;
        self.review = false;
        self.notices = None;
    }
    fn images(&mut self, ctx: &egui::Context, d: &Document) -> Result<(), String> {
        if self.image.is_some() {
            return Ok(());
        }
        let mut state = d.snapshot();
        if let Some(p) = &self.staged {
            state.scene = p.scene().clone();
        }
        let doc = drawer_core::layer_merge::preview(&state, d.current_frame)?;
        let render = drawer_core::render::Renderer::new(&doc);
        if let Some(e) = render.error() {
            return Err(e.into());
        }
        let mut bytes = vec![];
        for y in 0..128 {
            for x in 0..128 {
                bytes.extend(render.pixel([
                    (x as f64 + 0.5) * doc.width as f64 / 128.,
                    (y as f64 + 0.5) * doc.height as f64 / 128.,
                ]));
            }
        }
        self.image = Some(ctx.load_texture(
            "uv-paint-page",
            egui::ColorImage::from_rgba_unmultiplied([128, 128], &bytes),
            egui::TextureOptions::NEAREST,
        ));
        if let Some(p) = self
            .id
            .and_then(|id| doc.scene.world3d.surfaces.get(&id))
            .and_then(|s| s.paint.as_ref())
        {
            let pixels = p.decode()?;
            self.uv = Some(ctx.load_texture(
                "uv-paint-map",
                egui::ColorImage::from_rgba_unmultiplied([p.size as usize; 2], &pixels),
                egui::TextureOptions::NEAREST,
            ));
        }
        Ok(())
    }
}
impl Drawer {
    pub(super) fn uv_pending(&self) -> bool {
        self.uv_paint.staged.is_some() || self.uv_paint.stroke.is_some()
    }
    pub(super) fn begin_uv_paint(&mut self) -> Result<(), String> {
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
        self.finish_stroke();
        if self.models_busy()
            || self.has_world_preview()
            || self.rig_pending()
            || self.lighting_pending()
        {
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
        if !self.uv_pending() {
            self.uv_paint.id = self.documents[n]
                .scene
                .world3d
                .surfaces
                .iter()
                .find(|(_, s)| s.triangles.iter().any(|t| t.t.is_some()))
                .map(|(id, _)| *id)
                .or_else(|| {
                    (!self.documents[n].scene.worlds.is_empty())
                        .then_some(self.documents[n].scene.catalog.worlds.id(0))
                });
            self.uv_paint.clear();
            self.uv_paint.stamp = None;
        }
        self.hide_world_for_models();
        self.world_models.open = false;
        self.model_presets.open = false;
        self.human_rig.open = false;
        self.lighting.open = false;
        self.world_physics.open = false;
        self.uv_paint.open = true;
        Ok(())
    }
    pub(super) fn uv_paint_window(&mut self, ctx: &egui::Context) {
        let Some(n) = self.active else {
            self.uv_paint.clear();
            self.uv_paint.open = false;
            self.uv_paint.stamp = None;
            return;
        };
        if !self.uv_paint.open {
            self.uv_paint.clear();
            return;
        }
        let d = &self.documents[n];
        let budget = DEFAULT_DOCUMENT_BUDGET.saturating_sub(
            self.documents
                .iter()
                .map(Document::resident_bytes)
                .sum::<usize>(),
        );
        let mut p = std::mem::take(&mut self.uv_paint);
        if p.stamp != Some((d.id, d.revision, d.current_frame)) {
            p.clear();
            p.blocked = ctx.input(|i| i.pointer.primary_down());
            p.stamp = Some((d.id, d.revision, d.current_frame));
            p.id = p.id.filter(|id| {
                d.scene
                    .catalog
                    .worlds
                    .index(*id, d.scene.worlds.len())
                    .is_some()
            });
        }
        if !ctx.input(|i| i.pointer.primary_down()) {
            p.blocked = false;
        }
        if ctx.input(|i| i.key_pressed(egui::Key::Escape) || !i.focused) {
            p.clear();
            p.blocked = true;
        }
        if let Err(e) = p.images(ctx, d) {
            p.notice = e;
        }
        p.controls.clear();
        let mut action = 0;
        let mut open = true;
        egui::Window::new("UV surface painting / local RGB diffuse texture").default_pos(egui::pos2(15.,15.)).default_width(710.).max_height(650.).open(&mut open).show(ctx,|ui|{
 ui.add_sized([680.,20.],egui::Label::new(&p.notice).truncate()).on_hover_text(&p.notice);ui.horizontal(|ui|{let apply=p.staged.as_ref().is_some_and(|s|s.warnings.is_empty()||p.review);for(key,label,a,enabled)in[("apply","Apply stroke",1,apply),("discard","Discard",2,true),("export","Export texture PNG",3,p.staged.is_none()&&p.stroke.is_none())]{let r=ui.add_enabled(enabled,egui::Button::new(label));p.controls.insert(key.into(),r.rect);if r.clicked(){action=a;}}});
 egui::ComboBox::from_id_salt("uv-model").selected_text(p.id.map_or("Select a model".into(),|id|format!("Model {id}"))).show_ui(ui,|ui|{for i in 0..d.scene.worlds.len(){let id=d.scene.catalog.worlds.id(i);if ui.selectable_value(&mut p.id,Some(id),format!("Model {id}")).changed(){p.clear();}}});
 ui.horizontal(|ui|{let mut color=p.brush.rgb;if ui.color_edit_button_srgb(&mut color).changed(){p.brush.rgb=color;}ui.label("Texture brush radius");ui.add(egui::Slider::new(&mut p.brush.radius,0.5..=64.));ui.label(crate::i18n::text("Opacity"));ui.add(egui::Slider::new(&mut p.brush.opacity,0. ..=1.));});ui.horizontal(|ui|{ui.label("New texture size (existing map size retained)");for size in [64,128,256,512,1024]{ui.selectable_value(&mut p.brush.size,size,format!("{size}"));}});
 let surface=p.id.and_then(|id|d.scene.world3d.surfaces.get(&id));let warnings=p.notices.get_or_insert_with(||surface.map(core::inspect).unwrap_or_else(||Err("This model has no UV surface".into()))).clone();let valid=warnings.is_ok();match warnings{Ok(w)=>{for w in w{ui.colored_label(egui::Color32::YELLOW,w);}},Err(e)=>{ui.label(e);}}
if let Some(s)=&p.staged&& !s.warnings.is_empty(){let r=ui.checkbox(&mut p.review,"I reviewed UV sharing / partial mapping notices");p.controls.insert("review".into(),r.rect);}
 ui.horizontal(|ui|{let(r,response)=ui.allocate_exact_size(egui::vec2(270.,270.),if valid&&p.staged.is_none(){egui::Sense::drag()}else{egui::Sense::hover()});p.controls.insert("pad".into(),r);if let Some(image)=&p.image{ui.painter_at(r).image(image.id(),r,egui::Rect::from_min_max(egui::Pos2::ZERO,egui::pos2(1.,1.)),egui::Color32::WHITE);}
 let point=|pos:egui::Pos2|[((pos.x-r.left())/r.width())as f64*d.width as f64,((pos.y-r.top())/r.height())as f64*d.height as f64];if valid&&!p.blocked&&p.staged.is_none(){if response.is_pointer_button_down_on()&&p.stroke.is_none()&&let Some(pos)=response.interact_pointer_pos(){p.stroke=Some(vec![point(pos)]);}
if let Some(points)=&mut p.stroke&&ctx.input(|i|i.pointer.primary_down())&&let Some(pos)=ctx.input(|i|i.pointer.latest_pos()){let current=point(pos);if points.last().is_none_or(|old|(old[0]-current[0]).hypot(old[1]-current[1])>0.05){points.push(current);}}
if p.stroke.is_some()&&ctx.input(|i|i.pointer.primary_released()){action=4;}}
 let(uv,_)=ui.allocate_exact_size(egui::vec2(270.,270.),egui::Sense::hover());p.controls.insert("uv".into(),uv);let paint=ui.painter_at(uv);paint.rect_filled(uv,0.,egui::Color32::WHITE);if let Some(image)=&p.uv{paint.image(image.id(),uv,egui::Rect::from_min_max(egui::Pos2::ZERO,egui::pos2(1.,1.)),egui::Color32::WHITE);}
if let Some(s)=surface{for t in &s.triangles{if let Some(indices)=t.t{let points=indices.map(|i|{let p=s.texcoords[i as usize];egui::pos2(uv.left()+p[0]as f32*uv.width(),uv.bottom()-p[1]as f32*uv.height())});paint.add(egui::Shape::closed_line(points.to_vec(),egui::Stroke::new(0.6,egui::Color32::BLACK)));}}}
 });ui.small("Left: page projection, selected front 3D geometry. Right: UV tile / wire overlay. One stroke preview at a time. Opaque RGB paint multiplies material color. Wire views adopt Unlit on Apply; PNG export uses committed texture. Tile edges wrap; island seams split interpolation.");
 });
        p.open = open;
        if !open {
            p.clear();
        }
        if open {
            let result = (|| {
                match action {
                    1 => {
                        let s = p.staged.take().ok_or("Prepare a texture stroke")?;
                        if !s.warnings.is_empty() && !p.review {
                            return Err("Review UV notices first".into());
                        }
                        s.commit(&mut self.documents[n])?;
                        p.clear();
                        p.stamp = None;
                        p.notice =
                            "Applied texture stroke; one Undo restores the previous map".into();
                    }
                    2 => p.clear(),
                    3 => {
                        if let Some(path) = rfd::FileDialog::new()
                            .add_filter("PNG texture", &["png"])
                            .set_file_name("texture.png")
                            .save_file()
                        {
                            core::export(
                                &self.documents[n],
                                p.id.ok_or("Select a UV model")?,
                                &path,
                            )?;
                            p.notice =
                                "Exported committed local PNG; model source unchanged".into();
                        }
                    }
                    4 => {
                        let points = p.stroke.take().ok_or("No stroke points")?;
                        match core::prepare(
                            &self.documents[n],
                            p.id.ok_or("Select a UV model")?,
                            &points,
                            p.brush,
                            budget,
                            &AtomicBool::new(false),
                        ) {
                            Ok(s) => {
                                p.notice = s.summary.clone();
                                p.staged = Some(Box::new(s));
                                p.image = None;
                                p.uv = None;
                                p.review = false;
                            }
                            Err(e) => return Err(e),
                        }
                    }
                    _ => {}
                }
                Ok::<_, String>(())
            })();
            if let Err(e) = result {
                p.notice = e;
            }
        }
        self.uv_paint = p;
    }
}
#[path = "uv_paint_verify.rs"]
mod verify;
pub(super) fn smoke(root: &std::path::Path) -> Result<(), String> {
    verify::run(root)
}
#[cfg(test)]
mod tests {
    #[test]
    fn pointer_uv_stroke_preview_commit_undo_review_cancel_and_stale() {
        let t = crate::tests::TestDirectory::new();
        super::smoke(&t.0).unwrap();
    }
}
