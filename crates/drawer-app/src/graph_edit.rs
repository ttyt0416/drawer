use super::*;
use drawer_core::{
    graph_edit::{self as core, Axis, Channel, Edit, Handle, Preferences},
    keyframes::{Curve, Owner},
};
use std::collections::BTreeMap;
#[derive(Clone, Copy)]
enum Target {
    Key(u32),
    Handle(u32, Handle),
}
#[derive(Clone, Copy)]
struct Drag {
    target: Target,
    document: u64,
    revision: u64,
    owner: Owner,
}
type Stamp = (u64, u64, Owner, Channel, u32);
pub(super) struct Panel {
    pub open: bool,
    pub preferences: Preferences,
    owner: Option<Owner>,
    selected: u32,
    point: [f64; 2],
    point_stamp: Option<Stamp>,
    first: u32,
    last: u32,
    time_scale: [u32; 2],
    time_pivot: u32,
    value_scale: f64,
    value_pivot: f64,
    replace: bool,
    staged: Option<Box<core::Prepared>>,
    drag: Option<Drag>,
    select_after: Option<u32>,
    notice: String,
    label: Option<u64>,
    range_label: Option<u64>,
    label_frame: u32,
    pub controls: BTreeMap<&'static str, egui::Rect>,
    pub plot: Option<egui::Rect>,
    pub keys: BTreeMap<u32, egui::Pos2>,
    pub handles: [Option<egui::Pos2>; 2],
}
impl Default for Panel {
    fn default() -> Self {
        Self {
            open: false,
            preferences: Default::default(),
            owner: None,
            selected: 0,
            point: [0., 0.],
            point_stamp: None,
            first: 1,
            last: 1,
            time_scale: [1, 1],
            time_pivot: 1,
            value_scale: 1.,
            value_pivot: 0.,
            replace: false,
            staged: None,
            drag: None,
            select_after: None,
            notice: String::new(),
            label: None,
            range_label: None,
            label_frame: 1,
            controls: BTreeMap::new(),
            plot: None,
            keys: BTreeMap::new(),
            handles: [None, None],
        }
    }
}
impl Drawer {
    pub(super) fn load_graph_preferences(&mut self, storage: &UserStorage) {
        match Preferences::load(storage.root()) {
            Ok(p) => self.graph_editor.preferences = p,
            Err(e) => self.graph_editor.notice = e,
        }
    }
    pub(super) fn begin_graph_editor(&mut self) -> Result<(), String> {
        self.finish_stroke();
        if self.timeline_has_cel_binding()
            || self.transform_unconfirmed()
            || self.inference_unconfirmed()
            || self.text_editor.draft.is_some()
        {
            return Err("Finish pending content edits first".into());
        }
        let n = self.active.ok_or("Choose an animation")?;
        let d = &self.documents[n];
        d.animation.as_ref().ok_or("Create a timeline first")?;
        self.graph_editor.owner = d
            .scene
            .keyframes
            .get(self.keyframes.owner)
            .map(|t| t.owner)
            .or_else(|| d.scene.keyframes.tracks.first().map(|t| t.owner));
        self.graph_editor.first = 1;
        self.graph_editor.last = d.animation.as_ref().unwrap().frames;
        self.graph_editor.open = true;
        self.graph_editor.staged = None;
        self.graph_editor.drag = None;
        self.graph_editor.point_stamp = None;
        self.stop_timeline_playback(d.id);
        Ok(())
    }
    fn graph_budget(&self) -> usize {
        DEFAULT_DOCUMENT_BUDGET.saturating_sub(
            self.documents
                .iter()
                .map(Document::resident_bytes)
                .sum::<usize>(),
        )
    }
    fn stage_graph_edit(&mut self, edit: Edit) -> Result<(), String> {
        self.finish_stroke();
        if self.timeline_has_cel_binding()
            || self.transform_unconfirmed()
            || self.inference_unconfirmed()
            || self.text_editor.draft.is_some()
        {
            return Err("Finish pending edits before changing the graph".into());
        }
        let n = self.active.ok_or("Choose an animation")?;
        self.stop_timeline_playback(self.documents[n].id);
        let owner = self.graph_editor.owner.unwrap_or(Owner::Camera);
        let prepared = core::prepare(&self.documents[n], owner, edit, self.graph_budget())?;
        self.graph_editor.notice = prepared.summary.clone();
        self.graph_editor.staged = Some(Box::new(prepared));
        Ok(())
    }
    fn apply_graph_edit(&mut self) -> Result<(), String> {
        let p = self
            .graph_editor
            .staged
            .take()
            .ok_or("Prepare a graph change")?;
        let n = self.active.ok_or("Choose an animation")?;
        let summary = p.summary.clone();
        p.commit(&mut self.documents[n])?;
        if let Some(f) = self.graph_editor.select_after.take() {
            self.graph_editor.selected = f;
        }
        self.graph_editor.notice = summary;
        self.graph_editor.point_stamp = None;
        self.reset_timeline_view(self.documents[n].id);
        Ok(())
    }
    pub(super) fn graph_window(&mut self, ctx: &egui::Context) {
        if !self.graph_editor.open {
            return;
        }
        let Some(n) = self.active else {
            self.graph_editor.staged = None;
            self.graph_editor.drag = None;
            return;
        };
        let d = &self.documents[n];
        let Some(animation) = &d.animation else {
            self.graph_editor.staged = None;
            self.graph_editor.drag = None;
            return;
        };
        let budget = self.graph_budget();
        let mut p = std::mem::take(&mut self.graph_editor);
        let cancel = ctx.input(|i| !i.focused || i.key_down(egui::Key::Escape))
            || self.pending_close.is_some();
        if cancel
            || p.staged.as_ref().is_some_and(|s| !s.matches(d))
            || p.drag
                .is_some_and(|g| g.document != d.id || g.revision != d.revision)
        {
            p.staged = None;
            p.drag = None;
            p.select_after = None;
        }
        if p.owner.is_none() {
            p.owner = d.scene.keyframes.tracks.first().map(|t| t.owner);
        }
        let owner = p.owner.unwrap_or(Owner::Camera);
        let base = d.scene.keyframes.get(owner);
        if let Some(t) = base
            && !t.keys.iter().any(|k| k.frame == p.selected)
        {
            p.selected = t.keys.first().map_or(0, |k| k.frame);
        }
        let point_stamp = (d.id, d.revision, owner, p.preferences.channel, p.selected);
        if p.point_stamp != Some(point_stamp) {
            if let Some(k) = base.and_then(|t| t.keys.iter().find(|k| k.frame == p.selected)) {
                p.point = [k.frame as f64, p.preferences.channel.get(k.pose)];
            }
            p.point_stamp = Some(point_stamp);
        }
        for f in [
            &mut p.first,
            &mut p.last,
            &mut p.time_pivot,
            &mut p.label_frame,
        ] {
            *f = (*f).clamp(1, animation.frames);
        }
        let shown = p
            .staged
            .as_ref()
            .and_then(|s| s.track(owner))
            .cloned()
            .or_else(|| base.cloned());
        let names = drawer_core::layer_catalog::rows(d)
            .unwrap_or_default()
            .into_iter()
            .map(|r| (Owner::Layer(r.key), r.name))
            .chain([(Owner::Camera, "2D camera".into())])
            .collect::<BTreeMap<_, _>>();
        let mut open = true;
        let mut edit = None;
        let mut action = 0;
        let mut zoom = None;
        egui::Window::new("Animation graph / keys and labels").default_pos(egui::pos2(30.,20.)).default_width(740.).max_height(635.).vscroll(true).open(&mut open).show(ctx,|ui|{
   ui.add_sized([680.,20.],egui::Label::new(&p.notice).truncate()).on_hover_text(&p.notice);
   ui.horizontal(|ui|{let r=ui.add_enabled(p.staged.is_some(),egui::Button::new("Apply prepared graph edit"));p.controls.insert("apply",r.rect);if r.clicked(){action=1;}let r=ui.button("Cancel graph preview");p.controls.insert("cancel",r.rect);if r.clicked(){action=2;}let r=ui.button("Save local graph view");p.controls.insert("save",r.rect);if r.clicked(){action=3;}
 if ui.button("Open keyframe editor").clicked(){action=9;}});
   egui::ComboBox::from_label("Animated owner").selected_text(names.get(&owner).map_or("Choose owner",String::as_str)).show_ui(ui,|ui|{for t in &d.scene.keyframes.tracks{ui.selectable_value(&mut p.owner,Some(t.owner),names.get(&t.owner).cloned().unwrap_or_else(||format!("{:?}",t.owner)));}});
   if p.owner.unwrap_or(Owner::Camera)!=owner{p.staged=None;p.drag=None;p.point_stamp=None;return;}
   ui.horizontal_wrapped(|ui|{for c in Channel::ALL{ui.selectable_value(&mut p.preferences.channel,c,c.name());}});
   ui.horizontal_wrapped(|ui|{ui.label("Axis:");ui.selectable_value(&mut p.preferences.axis,Axis::Free,"Free");ui.selectable_value(&mut p.preferences.axis,Axis::Time,"Time only");ui.selectable_value(&mut p.preferences.axis,Axis::Value,"Value only");ui.checkbox(&mut p.preferences.snap_time,"Time snap");ui.add(egui::DragValue::new(&mut p.preferences.time_step).range(1..=10000));ui.checkbox(&mut p.preferences.snap_value,"Value snap");ui.add(egui::DragValue::new(&mut p.preferences.value_step).range(1e-9..=1e9).speed(0.1));});
   ui.horizontal_wrapped(|ui|{ui.checkbox(&mut p.preferences.split_handles,"Split adjacent tangent handles");if ui.button("Fit keys").clicked(){action=4;}for(label,factor)in [("Time +",[2.,1.]),("Time −",[0.5,1.]),("Value +",[1.,2.]),("Value −",[1.,0.5])]{let r=ui.button(label);if label=="Time +"{p.controls.insert("zoom",r.rect);}
 if r.clicked(){zoom=Some(factor);}}});
   ui.label("Drag a key or its incoming / outgoing handle. Release to apply one Undo; Esc cancels. Curve shape affects all pose channels; key values change only the selected channel.");
   if !p.preferences.valid(){ui.label("Choose increasing finite graph ranges and valid snap steps.");}else if let Some(t)=&shown{
    let(rect,response)=ui.allocate_exact_size(egui::vec2(660.,230.),egui::Sense::click_and_drag());p.plot=Some(rect);let painter=ui.painter_at(rect);painter.rect_stroke(rect,0.,egui::Stroke::new(1.,egui::Color32::GRAY),egui::StrokeKind::Inside);
    let view=p.preferences;let screen=|point:[f64;2]|egui::pos2(rect.left()+((point[0]-view.time[0])/(view.time[1]-view.time[0]))as f32*rect.width(),rect.bottom()-((point[1]-view.value[0])/(view.value[1]-view.value[0]))as f32*rect.height());let world=|pos:egui::Pos2|[view.time[0]+((pos.x-rect.left())/rect.width())as f64*(view.time[1]-view.time[0]),view.value[0]+((rect.bottom()-pos.y)/rect.height())as f64*(view.value[1]-view.value[0])];
    for i in 0..=5{let f=i as f64/5.;let time=view.time[0]+(view.time[1]-view.time[0])*f;let value=view.value[0]+(view.value[1]-view.value[0])*f;let x=screen([time,view.value[0]]).x;let y=screen([view.time[0],value]).y;let grid=egui::Stroke::new(0.5,egui::Color32::from_gray(65));painter.line_segment([egui::pos2(x,rect.top()),egui::pos2(x,rect.bottom())],grid);painter.line_segment([egui::pos2(rect.left(),y),egui::pos2(rect.right(),y)],grid);painter.text(egui::pos2(x,rect.bottom()-2.),egui::Align2::CENTER_BOTTOM,format!("{:.1}",time+1.),egui::FontId::proportional(10.),egui::Color32::LIGHT_GRAY);painter.text(egui::pos2(rect.left()+2.,y),egui::Align2::LEFT_CENTER,format!("{value:.2}"),egui::FontId::proportional(10.),egui::Color32::LIGHT_GRAY);}
    let line=(0..=256).filter_map(|i|{let time=view.time[0]+(view.time[1]-view.time[0])*i as f64/256.;core::sample(t,view.channel,time).map(|v|screen([time,v]))}).collect::<Vec<_>>();painter.add(egui::Shape::line(line,egui::Stroke::new(1.5,egui::Color32::LIGHT_GREEN)));
    p.keys.clear();p.handles=[None,None];for k in &t.keys{let pos=screen([k.frame as f64,view.channel.get(k.pose)]);if rect.contains(pos){p.keys.insert(k.frame,pos);painter.circle_filled(pos,4.,if k.frame==p.selected{egui::Color32::YELLOW}else{egui::Color32::WHITE});}}
    if let Some(k)=t.keys.iter().find(|k|k.frame==p.selected){let center=screen([k.frame as f64,view.channel.get(k.pose)]);for(i,side)in [Handle::Incoming,Handle::Outgoing].into_iter().enumerate(){if let Some(point)=core::handle_point(t,k.frame,side,view.channel){let pos=screen(point);if rect.contains(pos){p.handles[i]=Some(pos);painter.line_segment([center,pos],egui::Stroke::new(1.,egui::Color32::LIGHT_BLUE));painter.rect_stroke(egui::Rect::from_center_size(pos,egui::vec2(8.,8.)),0.,egui::Stroke::new(1.,egui::Color32::LIGHT_BLUE),egui::StrokeKind::Inside);}}}}
    let hit=|pos:egui::Pos2|{let handle=p.handles.iter().enumerate().filter_map(|(i,p)|p.map(|p|(i,p.distance(pos)))).filter(|(_,d)|*d<=8.).min_by(|a,b|a.1.total_cmp(&b.1));if let Some((i,_))=handle{return Some(Target::Handle(p.selected,if i==0{Handle::Incoming}else{Handle::Outgoing}));}p.keys.iter().map(|(f,p)|(*f,p.distance(pos))).filter(|(_,d)|*d<=9.).min_by(|a,b|a.1.total_cmp(&b.1)).map(|(f,_)|Target::Key(f))};
    let drag_hit=ctx.input(|i|i.pointer.press_origin()).and_then(hit);let click_hit=response.interact_pointer_pos().and_then(hit);
    if !cancel&&response.drag_started()&&let Some(target)=drag_hit{p.staged=None;p.drag=Some(Drag{target,document:d.id,revision:d.revision,owner});if let Target::Key(f)=target{p.selected=f;p.point_stamp=None;}}
    if !cancel&&response.clicked()&&let Some(Target::Key(f))=click_hit{p.selected=f;p.point_stamp=None;}
    if !cancel&&(response.dragged()||response.drag_stopped())&&let(Some(g),Some(pos))=(p.drag,response.interact_pointer_pos()){
     let point=world(pos);let request=match g.target{Target::Key(from)=>Edit::Move{from,point,options:view,replace:p.replace},Target::Handle(frame,side)=>Edit::Handle{frame,side,point,options:view}};
     match core::prepare(d,g.owner,request,budget){Ok(s)=>{if let Target::Key(from)=g.target&&let Some(k)=base.and_then(|t|t.keys.iter().find(|k|k.frame==from)){p.select_after=view.snap([from as f64,view.channel.get(k.pose)],point).ok().map(|p|p[0].round()as u32);}p.notice=s.summary.clone();p.staged=Some(Box::new(s));},Err(e)=>{p.staged=None;p.notice=e;}}
    }
    if !cancel&&response.drag_stopped(){p.drag=None;if p.staged.is_some(){action=1;}}
   }else{ui.label("Enable an owner's keyframes in Layer keyframes first.");}
   ui.collapsing("Selected key / interpolation / scaling",|ui|{
    if let Some(t)=base{ui.horizontal_wrapped(|ui|{for k in &t.keys{if ui.selectable_value(&mut p.selected,k.frame,format!("Key{}",k.frame+1)).clicked(){p.point_stamp=None;}}});}
    ui.horizontal(|ui|{let mut f=p.point[0]+1.;ui.label("Time / value");ui.add(egui::DragValue::new(&mut f).range(1.0..=animation.frames as f64));p.point[0]=f-1.;ui.add(egui::DragValue::new(&mut p.point[1]).speed(0.1));if ui.button("Prepare key move").clicked(){p.select_after=Some(p.point[0].round()as u32);edit=Some(Edit::Move{from:p.selected,point:p.point,options:p.preferences,replace:p.replace});}});
    ui.horizontal(|ui|{for(label,curve)in [("Fixed",Curve::Fixed),("Linear",Curve::Linear),("Bezier",Curve::Bezier{first:[1./3.;2],second:[2./3.;2]})]{if ui.button(label).clicked(){edit=Some(Edit::Curve{frame:p.selected,curve});}}});
    ui.checkbox(&mut p.replace,"Replace colliding key times (later selected key wins)");
    ui.horizontal_wrapped(|ui|{ui.label("First / last / time pivot");for f in [&mut p.first,&mut p.last,&mut p.time_pivot]{ui.add(egui::DragValue::new(f).range(1..=animation.frames));}ui.label("Time scale ratio");for v in &mut p.time_scale{ui.add(egui::DragValue::new(v).range(1..=1000));}});
    ui.horizontal(|ui|{ui.label("Value scale / pivot");ui.add(egui::DragValue::new(&mut p.value_scale).range(-1000.0..=1000.).speed(0.1));ui.add(egui::DragValue::new(&mut p.value_pivot).speed(0.1));if ui.button("Prepare selected key scale").clicked(){edit=Some(Edit::Scale{first:p.first-1,last:p.last-1,time:p.time_scale,pivot:p.time_pivot-1,value:p.value_scale,value_pivot:p.value_pivot,channel:p.preferences.channel,replace:p.replace});}});
   });
   ui.collapsing("Timeline / track range labels",|ui|{ui.add(egui::DragValue::new(&mut p.label_frame).range(1..=animation.frames));egui::ComboBox::from_label("Timeline label").selected_text(p.label.map_or("Choose".into(),|id|format!("Label{id}"))).show_ui(ui,|ui|{for l in &d.scene.cels.labels{ui.selectable_value(&mut p.label,Some(l.id),format!("{} @{}",l.name,l.frame+1));}});if ui.button("Prepare timeline label move").clicked(){edit=p.label.map(|id|Edit::TimelineLabel{id,frame:p.label_frame-1});}egui::ComboBox::from_label("Track range label").selected_text(p.range_label.map_or("Choose".into(),|id|format!("Range{id}"))).show_ui(ui,|ui|{for l in &d.scene.timing.edit.labels{ui.selectable_value(&mut p.range_label,Some(l.id),format!("{} {}–{}",l.name,l.first+1,l.end));}});if ui.button("Prepare track label move").clicked(){edit=p.range_label.map(|id|Edit::TrackLabel{id,first:p.label_frame-1});}});
   ui.collapsing("Exact visible frame / value ranges",|ui|{ui.horizontal(|ui|{let mut times=[p.preferences.time[0]+1.,p.preferences.time[1]+1.];for v in &mut times{ui.add(egui::DragValue::new(v).speed(0.1));}p.preferences.time=[times[0]-1.,times[1]-1.];for v in &mut p.preferences.value{ui.add(egui::DragValue::new(v).speed(0.1));}});});
  });
        p.open = open;
        if !open || cancel {
            p.drag = None;
            p.staged = None;
            action = 0;
            edit = None;
        }
        if let Some(factors) = zoom {
            let center = [
                (p.preferences.time[0] + p.preferences.time[1]) / 2.,
                (p.preferences.value[0] + p.preferences.value[1]) / 2.,
            ];
            if let Err(e) = p.preferences.zoom(factors, center) {
                p.notice = e;
            }
        }
        if action == 4
            && let Some(t) = base
            && !t.keys.is_empty()
        {
            let a = t.keys.first().unwrap().frame as f64;
            let b = t.keys.last().unwrap().frame as f64;
            let lo = t
                .keys
                .iter()
                .map(|k| p.preferences.channel.get(k.pose))
                .fold(f64::INFINITY, f64::min);
            let hi = t
                .keys
                .iter()
                .map(|k| p.preferences.channel.get(k.pose))
                .fold(f64::NEG_INFINITY, f64::max);
            let margin = ((hi - lo) * 0.15).max(0.1);
            p.preferences.time = [(a - 1.).max(-10000.), (b + 1.).min(20000.)];
            p.preferences.value = [(lo - margin).max(-2e9), (hi + margin).min(2e9)];
        }
        self.graph_editor = p;
        let result = (|| {
            match action {
                1 => self.apply_graph_edit()?,
                2 => {
                    self.graph_editor.staged = None;
                    self.graph_editor.drag = None;
                    self.graph_editor.select_after = None;
                }
                3 => {
                    if let Some(s) = &self.storage {
                        self.graph_editor.preferences.save(s.root())?;
                    }
                    self.graph_editor.notice = "Local graph view saved; drawing unchanged".into();
                }
                9 => {
                    self.begin_keyframes()?;
                    self.keyframes.owner = owner;
                }
                _ => {}
            }
            if let Some(edit) = edit {
                self.stage_graph_edit(edit)?;
            }
            Ok::<_, String>(())
        })();
        if let Err(e) = result {
            self.graph_editor.notice = e;
        }
    }
}

#[path = "graph_edit_verify.rs"]
mod verify;
pub(super) fn smoke(root: &std::path::Path) -> Result<(), String> {
    verify::run(root)
}
#[cfg(test)]
mod tests {
    #[test]
    fn pointer_key_snap_handle_drag_cancel_zoom_restart_stale_and_close() {
        let t = crate::tests::TestDirectory::new();
        super::smoke(&t.0).unwrap();
    }
}
