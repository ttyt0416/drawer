use super::*;
use drawer_core::{
    keyframes::{self as core, Curve, Edit, Keyframe, Owner, Pose},
    layer_search::Key,
};
use std::path::Path;
pub(super) struct Panel {
    pub(super) open: bool,
    pub(super) owner: Owner,
    frame: u32,
    move_to: u32,
    pose: Pose,
    curve: Curve,
    replace: bool,
    clear: bool,
    stamp: Option<(u64, u64, u32, Owner)>,
    notice: String,
    name: String,
    label: String,
    label_id: Option<u64>,
    label_frame: u32,
    pub apply: Option<egui::Rect>,
    pub toggle: Option<egui::Rect>,
    pub content: Option<egui::Rect>,
}
impl Default for Panel {
    fn default() -> Self {
        Self {
            open: false,
            owner: Owner::Camera,
            frame: 1,
            move_to: 1,
            pose: Pose::default(),
            curve: Curve::Linear,
            replace: false,
            clear: false,
            stamp: None,
            notice: String::new(),
            name: String::new(),
            label: String::new(),
            label_id: None,
            label_frame: 1,
            apply: None,
            toggle: None,
            content: None,
        }
    }
}
impl Drawer {
    pub(super) fn begin_camera_keyframes(&mut self) -> Result<(), String> {
        self.begin_keyframes()?;
        self.keyframes.owner = Owner::Camera;
        self.keyframes.stamp = None;
        Ok(())
    }
    pub(super) fn begin_keyframes(&mut self) -> Result<(), String> {
        self.finish_stroke();
        if self.transform_unconfirmed()
            || self.inference_unconfirmed()
            || self.text_editor.draft.is_some()
        {
            return Err("Finish pending drawing edits first".into());
        }
        let d = &self.documents[self.active.ok_or("Choose an animation document")?];
        d.animation.as_ref().ok_or("Create a timeline first")?;
        let key = Key::Layer(drawer_core::layer_properties::raster(d, 0));
        self.keyframes.owner = d.keyframe_content.unwrap_or(Owner::Layer(key));
        self.keyframes.open = true;
        self.keyframes.stamp = None;
        self.stop_timeline_playback(d.id);
        Ok(())
    }
    fn apply_keyframe(&mut self, edit: Edit) -> Result<(), String> {
        self.finish_stroke();
        let n = self.active.ok_or("Choose a document")?;
        let d = &mut self.documents[n];
        let s = core::prepare(d, self.keyframes.owner, edit)?;
        d.commit_state(s)?;
        self.keyframes.stamp = None;
        Ok(())
    }
    pub(super) fn keyframes_window(&mut self, ctx: &egui::Context) {
        if !self.keyframes.open {
            return;
        }
        let Some(n) = self.active else { return };
        let d = &self.documents[n];
        let Some(a) = &d.animation else { return };
        let mut p = std::mem::take(&mut self.keyframes);
        let stamp = (d.id, d.revision, d.current_frame, p.owner);
        if p.stamp != Some(stamp) {
            p.frame = d.current_frame + 1;
            p.move_to = p.frame;
            p.pose = d
                .scene
                .keyframes
                .get(p.owner)
                .map_or(Pose::default(), |t| t.at(d.current_frame));
            p.curve = d
                .scene
                .keyframes
                .get(p.owner)
                .and_then(|t| t.keys.iter().find(|k| k.frame == d.current_frame))
                .map_or(Curve::Linear, |k| k.curve);
            p.clear = false;
            p.replace = false;
            p.stamp = Some(stamp);
            if let Owner::Layer(Key::Track(id)) = p.owner
                && let Some(i) = d.scene.catalog.tracks.index(id, a.tracks.len())
            {
                p.name = a.tracks[i].name.clone();
            }
        }
        let mut open = p.open;
        let mut edit = None;
        let mut content = None;
        let mut seek = None;
        let mut track_name = false;
        let mut label_action = 0;
        let names = drawer_core::layer_catalog::rows(d)
            .unwrap_or_default()
            .into_iter()
            .enumerate()
            .map(|(i, r)| (Owner::Layer(r.key), format!("Layer {}: {}", i + 1, r.name)))
            .chain([(Owner::Camera, "2D camera (parent / final)".into())])
            .collect::<std::collections::BTreeMap<_, _>>();
        let owner_name = names
            .get(&p.owner)
            .cloned()
            .unwrap_or_else(|| "Choose a layer".into());
        let current = d.current_frame + 1;
        let active = d.scene.keyframes.get(p.owner).is_some_and(|t| t.enabled);
        let mut parent = d
            .scene
            .keyframes
            .get(p.owner)
            .is_some_and(|t| t.parent_camera);
        egui::Window::new("Layer keyframes / track names and labels").id(egui::Id::new("layer-keyframes")).open(&mut open).default_pos(egui::pos2(95.,65.)).default_width(680.).max_height(585.).resizable(true).vscroll(true).show(ctx,|ui|{
   ui.label(&p.notice);
   ui.horizontal(|ui|{let r=ui.button(if active{"Disable keyframes (keep current pose)"}else{"Enable / resume retained keys"});p.toggle=Some(r.rect);if r.clicked(){edit=Some(Edit::Enable(!active));}let r=ui.button(if d.keyframe_content==Some(p.owner){"Finish base content editing"}else{"Edit base content"});p.content=Some(r.rect);if r.clicked(){content=Some(if d.keyframe_content==Some(p.owner){None}else{Some(p.owner)});}});
   ui.horizontal(|ui|{let r=ui.button("Add / replace key");p.apply=Some(r.rect);if r.clicked(){edit=Some(Edit::Put{key:Keyframe{frame:p.frame-1,pose:p.pose,curve:p.curve},replace:p.replace});}ui.checkbox(&mut p.replace,"Confirm replacement at occupied time");});
   egui::ComboBox::from_label("Animated owner").selected_text(&owner_name).show_ui(ui,|ui|{ui.selectable_value(&mut p.owner,Owner::Camera,"2D camera (parent / final)");if let Ok(keys)=drawer_core::layer_groups::live(d){for k in keys.into_keys(){ui.selectable_value(&mut p.owner,Owner::Layer(k),names.get(&Owner::Layer(k)).cloned().unwrap_or_else(||"Layer".into()));}}});
   if p.owner!=stamp.3 {p.stamp=None;return}
   ui.horizontal(|ui|{ui.label(format!("Current frame {current} / {}",a.frames));if ui.button("Previous key").clicked(){seek=d.scene.keyframes.get(p.owner).and_then(|t|t.keys.iter().rev().find(|k|k.frame<d.current_frame)).map(|k|k.frame);}
if ui.button("Next key").clicked(){seek=d.scene.keyframes.get(p.owner).and_then(|t|t.keys.iter().find(|k|k.frame>d.current_frame)).map(|k|k.frame);}});
   if p.owner!=Owner::Camera&&ui.checkbox(&mut parent,"Use 2D parent camera (relative to layer parent)").changed(){edit=Some(Edit::ParentCamera(parent));}
   ui.label("Keys are relative to the preserved base geometry. Opacity multiplies base opacity. Base editing temporarily shows original coordinates; exports always evaluate animation.");
   ui.horizontal(|ui|{ui.label("Key frame (1 based)");ui.add(egui::DragValue::new(&mut p.frame).range(1..=a.frames));});
   egui::Grid::new("key-pose").num_columns(3).show(ui,|ui|{for(label,v,range)in [("Offset x / y",&mut p.pose.offset,-1e9..=1e9),("Scale x / y",&mut p.pose.scale,0.01..=1000.),("Pivot x / y",&mut p.pose.pivot,-1e9..=1e9)]{ui.label(label);for x in v{ui.add(egui::DragValue::new(x).range(range.clone()).speed(0.1));}ui.end_row();}ui.label("Rotation / relative opacity");ui.add(egui::DragValue::new(&mut p.pose.degrees).range(-36000.0..=36000.).speed(0.5));ui.add(egui::DragValue::new(&mut p.pose.opacity).range(0.0..=1.).speed(0.01));ui.end_row();});
   ui.horizontal(|ui|{ui.selectable_value(&mut p.curve,Curve::Fixed,"Fixed");ui.selectable_value(&mut p.curve,Curve::Linear,"Linear");if ui.selectable_label(matches!(p.curve,Curve::Bezier{..}),"Bezier").clicked(){p.curve=Curve::Bezier{first:[0.25,0.1],second:[0.75,0.9]};}});
   if let Curve::Bezier{first,second}=&mut p.curve{ui.horizontal(|ui|{ui.label("Handles x1,y1,x2,y2 (0..1; x1 <= x2)");for v in first.iter_mut().chain(second){ui.add(egui::DragValue::new(v).range(0.0..=1.).speed(0.01));}});}
   if let Some(t)=d.scene.keyframes.get(p.owner){ui.label(format!("Base opacity {}%; original layer content is preserved.",t.basis_opacity));ui.horizontal_wrapped(|ui|{for k in &t.keys{if ui.button(format!("{} {:?}",k.frame+1,k.curve)).clicked(){seek=Some(k.frame);}}});}
   ui.horizontal(|ui|{ui.label("Move key at chosen frame to");ui.add(egui::DragValue::new(&mut p.move_to).range(1..=a.frames));if ui.button("Move existing key").clicked(){edit=Some(Edit::Move{from:p.frame-1,to:p.move_to-1});}
if ui.button("Delete chosen key").clicked(){edit=Some(Edit::Delete(p.frame-1));}});
   ui.separator();ui.label(format!("Delete ALL keys of {}: {}. Current local pose will remain.",owner_name,core::range(d,p.owner)));ui.checkbox(&mut p.clear,"Confirm the displayed owner / complete range");if ui.add_enabled(p.clear,egui::Button::new("Delete all keys")).clicked(){edit=Some(Edit::Clear{confirmed:p.clear});}
   if matches!(p.owner,Owner::Layer(Key::Track(_))){ui.separator();ui.horizontal(|ui|{ui.label("Track name");ui.text_edit_singleline(&mut p.name);if ui.button("Rename track").clicked(){track_name=true;}});}
   ui.separator();ui.label("Timeline labels (saved with the drawing)");egui::ComboBox::from_label("Label").selected_text(p.label_id.map_or("New label".into(),|id|format!("Label{id}"))).show_ui(ui,|ui|{if ui.selectable_value(&mut p.label_id,None,"New label").clicked(){p.label.clear();p.label_frame=current;}for l in &d.scene.cels.labels{if ui.selectable_value(&mut p.label_id,Some(l.id),format!("{}: {} @{}",l.id,l.name,l.frame+1)).clicked(){p.label=l.name.clone();p.label_frame=l.frame+1;}}});
   ui.horizontal(|ui|{ui.text_edit_singleline(&mut p.label);ui.add(egui::DragValue::new(&mut p.label_frame).range(1..=a.frames));if ui.button("Save label").clicked(){label_action=1;}
if ui.add_enabled(p.label_id.is_some(),egui::Button::new("Delete label")).clicked(){label_action=2;}});
  });
        p.open = open;
        self.keyframes = p;
        let result = (|| {
            if let Some(edit) = edit {
                self.apply_keyframe(edit)?;
            }
            if let Some(owner) = content {
                self.finish_stroke();
                core::edit_content(&mut self.documents[n], owner)?;
                self.keyframes.notice = if owner.is_some() {
                    "Base content view is active. Existing keys stay unchanged; each content edit has its own Undo.".into()
                } else {
                    "Animation view restored".into()
                };
            }
            if let Some(frame) = seek {
                self.finish_stroke();
                self.documents[n].current_frame = frame;
                self.keyframes.stamp = None;
            }
            if track_name {
                let Owner::Layer(Key::Track(track)) = self.keyframes.owner else {
                    return Err("Choose a cel track".into());
                };
                let s = drawer_core::timeline_editor::prepare(
                    &self.documents[n],
                    drawer_core::timeline_editor::Edit::RenameTrack {
                        track,
                        name: self.keyframes.name.clone(),
                    },
                    drawer_core::document::DEFAULT_DOCUMENT_BUDGET,
                )?
                .state;
                self.documents[n].commit_state(s)?;
                self.keyframes.stamp = None;
            }
            if label_action > 0 {
                let mut s = self.documents[n].snapshot();
                if label_action == 2 {
                    s.scene
                        .cels
                        .labels
                        .retain(|l| Some(l.id) != self.keyframes.label_id);
                } else if let Some(id) = self.keyframes.label_id {
                    let name = &self.keyframes.label;
                    if name.trim().is_empty()
                        || name.len() > 256
                        || name.chars().any(char::is_control)
                    {
                        return Err(
                            "Use a label name of1..256 bytes without control characters".into()
                        );
                    }
                    let l = s
                        .scene
                        .cels
                        .labels
                        .iter_mut()
                        .find(|l| l.id == id)
                        .ok_or("Label removed")?;
                    l.name = name.clone();
                    l.frame = self.keyframes.label_frame - 1;
                } else {
                    s = drawer_core::timeline_editor::prepare(
                        &self.documents[n],
                        drawer_core::timeline_editor::Edit::Label {
                            name: self.keyframes.label.clone(),
                            frame: self.keyframes.label_frame - 1,
                        },
                        drawer_core::document::DEFAULT_DOCUMENT_BUDGET,
                    )?
                    .state;
                }
                self.documents[n].commit_state(s)?;
            }
            Ok(())
        })();
        if let Err(e) = result {
            self.keyframes.notice = e;
        }
    }
}
pub(super) fn smoke(root: &Path) -> Result<(), String> {
    drawer_core::keyframes_verify::verify(root)?;
    let ctx = egui::Context::default();
    let mut app = Drawer::new(&ctx, None);
    app.simple_view = true;
    app.add_document(drawer_core::keyframes_verify::fixture());
    app.execute_command(&ctx, "layer-keyframes");
    if !app.keyframes.open {
        return Err("Layer keyframe command missing".into());
    }
    let mut time = 0.;
    let frame = |app: &mut Drawer, time: &mut f64, events| {
        *time += 0.05;
        input_replay::frame(&ctx, app, *time, events);
    };
    let click = |app: &mut Drawer, time: &mut f64, rect: egui::Rect| {
        for pressed in [true, false] {
            frame(
                app,
                time,
                vec![
                    egui::Event::PointerMoved(rect.center()),
                    egui::Event::PointerButton {
                        pos: rect.center(),
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
    let r = app.keyframes.toggle.ok_or("Enable control missing")?;
    click(&mut app, &mut time, r);
    let owner = app.keyframes.owner;
    if !app.documents[0]
        .scene
        .keyframes
        .get(owner)
        .is_some_and(|t| t.enabled)
    {
        return Err(format!(
            "Pointer activation failed: {}",
            app.keyframes.notice
        ));
    }
    frame(&mut app, &mut time, vec![]);
    app.keyframes.frame = 9;
    app.keyframes.pose.offset = [4., 0.];
    app.keyframes.pose.opacity = 0.5;
    let r = app.keyframes.apply.unwrap();
    click(&mut app, &mut time, r);
    if app.documents[0]
        .scene
        .keyframes
        .get(owner)
        .unwrap()
        .keys
        .len()
        != 2
    {
        return Err(format!(
            "Pointer key insertion failed: {}",
            app.keyframes.notice
        ));
    }
    let keys = app.documents[0].scene.keyframes.clone();
    let snapshot = app.documents[0].snapshot();
    let counts = app.documents[0].history_counts();
    let r = app.keyframes.content.unwrap();
    click(&mut app, &mut time, r);
    if app.documents[0].keyframe_content != Some(owner)
        || app.documents[0].snapshot() != snapshot
        || app.documents[0].history_counts() != counts
        || app.documents[0].scene.keyframes != keys
    {
        return Err("Content view changed stored state or was inaccessible".into());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn pointer_enable_insert_and_base_content() {
        let root = std::env::temp_dir().join(format!("drawer-keyframe-ui-{}", std::process::id()));
        std::fs::create_dir(&root).unwrap();
        let r = super::smoke(&root);
        std::fs::remove_dir(&root).unwrap();
        r.unwrap();
    }
}
