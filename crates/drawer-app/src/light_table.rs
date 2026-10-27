use super::*;
use drawer_core::{
    keyframes::Pose,
    layer_search::Key,
    light_table::{self as core, Add, Data, Scope, Source},
    timeline_view::ColorMode,
};
use std::path::Path;
#[derive(Clone, PartialEq)]
struct CacheKey {
    document: u64,
    revision: u64,
    frame: u32,
    auxiliary: u64,
    instance: u64,
    cell: Option<u64>,
    form: Option<(u64, Pose, ColorMode, [u8; 3])>,
    opacity: u8,
}
#[derive(Default)]
pub(super) struct Panel {
    open: bool,
    document: Option<u64>,
    selected: Option<u64>,
    cel: Option<u64>,
    cell_scope: bool,
    kind: u8,
    layer: Option<Key>,
    other: Option<u64>,
    file: String,
    name: String,
    frame: u32,
    track: Option<u64>,
    pose: Pose,
    mode: ColorMode,
    color: [u8; 3],
    opacity: u8,
    loaded: Option<(u64, u64, u64, Option<u64>)>,
    drag: Option<([f64; 2], Pose)>,
    cache: Option<CacheKey>,
    image: Option<egui::TextureHandle>,
    notices: Vec<String>,
    notice: String,
    pub preview: Option<egui::Rect>,
    pub add: Option<egui::Rect>,
    pub undo: Option<egui::Rect>,
    pub pin: Option<egui::Rect>,
    pub pivot_tool: Option<egui::Rect>,
    pub move_tool: Option<egui::Rect>,
    pub rotate_tool: Option<egui::Rect>,
    pub show: Option<egui::Rect>,
    pub center: Option<egui::Rect>,
}
impl Drawer {
    pub(super) fn light_table_save_guard(&mut self, n: usize) -> Result<(), String> {
        let d = &self.documents[n];
        let p = &self.light_table;
        if !p.open
            || p.document != Some(d.id)
            || p.loaded
                != Some((
                    d.id,
                    d.light_table.instance(),
                    d.light_table.stamp(),
                    p.selected,
                ))
        {
            return Ok(());
        }
        let changed = p.drag.is_some()
            || p.opacity != d.light_table.data.opacity
            || p.selected
                .and_then(|id| d.light_table.data.get(id))
                .is_some_and(|r| {
                    r.pose != p.pose
                        || r.mode != p.mode
                        || r.color != p.color
                        || r.name != p.name
                        || r.scope
                            != if p.cell_scope {
                                p.cel.map_or(Scope::Common, Scope::Cell)
                            } else {
                                Scope::Common
                            }
                });
        if changed {
            let message =
                "Apply reference changes or reset the placement preview before saving/closing";
            self.light_table.notice = message.into();
            Err(message.into())
        } else {
            Ok(())
        }
    }
    pub(super) fn begin_light_table(&mut self) -> Result<(), String> {
        self.finish_stroke();
        let n = self.light_table_context().ok_or("Choose a drawing")?;
        self.documents[n].light_table.visible = true;
        self.light_table.open = true;
        self.light_table.document = None;
        Ok(())
    }
    pub(super) fn light_table_input_ready(&self) -> bool {
        let Some(n) = self.light_table_context() else {
            return true;
        };
        let d = &self.documents[n];
        d.light_table.pin.is_none_or(|p| {
            core::pin_valid(d)
                && self
                    .active
                    .is_some_and(|a| p.child == Some(self.documents[a].id))
        })
    }
    fn light_table_select_document(&mut self, n: usize) {
        let d = &self.documents[n];
        if self.light_table.document == Some(d.id) {
            return;
        }
        let bound_child = self.light_table_binding(d.id).map(|b| b.0);
        let p = &mut self.light_table;
        p.document = Some(d.id);
        p.selected = d.light_table.data.references.first().map(|r| r.id);
        p.cel = d
            .light_table
            .pin
            .map(|p| p.source)
            .or_else(|| d.scene.cels.definitions.first().map(|p| p.object));
        p.layer = Some(Key::Layer(drawer_core::layer_properties::raster(d, 0)));
        p.other = self
            .documents
            .iter()
            .find(|s| s.id != d.id && bound_child.is_none_or(|id| s.id != id))
            .map(|d| d.id);
        p.name = "Reference".into();
        p.frame = d.current_frame + 1;
        p.opacity = d.light_table.data.opacity;
        p.loaded = None;
        p.drag = None;
        p.cache = None;
    }
    fn light_table_data(&self, n: usize) -> Data {
        let mut data = self.documents[n].light_table.data.clone();
        data.opacity = self.light_table.opacity;
        if let Some(id) = self.light_table.selected
            && let Some(r) = data.get_mut(id)
        {
            r.pose = self.light_table.pose;
            r.mode = self.light_table.mode;
            r.color = self.light_table.color;
        }
        data
    }
    pub(super) fn light_table_tick(&mut self, ctx: &egui::Context) {
        self.canvas_view.light_table_overlay = None;
        self.display.canvas.light_table_overlay = None;
        let Some(n) = self.light_table_context() else {
            return;
        };
        self.light_table_select_document(n);
        let d = &self.documents[n];
        let p = &mut self.light_table;
        let stamp = (
            d.id,
            d.light_table.instance(),
            d.light_table.stamp(),
            p.selected,
        );
        if p.loaded != Some(stamp) {
            if let Some(r) = p.selected.and_then(|id| d.light_table.data.get(id)) {
                p.pose = r.pose;
                p.mode = r.mode;
                p.color = r.color;
                p.name = r.name.clone();
                p.cell_scope = matches!(r.scope, Scope::Cell(_));
                if let Scope::Cell(id) = r.scope {
                    p.cel = Some(id);
                }
            }
            p.opacity = d.light_table.data.opacity;
            p.loaded = Some(stamp);
            p.drag = None;
        }
        let d = &self.documents[n];
        let cell = d.light_table.pin.map(|p| p.source).or(self.light_table.cel);
        let key = CacheKey {
            document: d.id,
            revision: d.revision,
            frame: d.current_frame,
            auxiliary: d.light_table.stamp(),
            instance: d.light_table.instance(),
            cell,
            form: self.light_table.selected.map(|id| {
                (
                    id,
                    self.light_table.pose,
                    self.light_table.mode,
                    self.light_table.color,
                )
            }),
            opacity: self.light_table.opacity,
        };
        if (d.light_table.visible || self.light_table.open)
            && self.light_table.cache.as_ref() != Some(&key)
        {
            let data = self.light_table_data(n);
            match core::preview(&self.documents[n], &data, cell, 512) {
                Ok(p) => {
                    self.light_table.image = Some(ctx.load_texture(
                        "light-table-reference-view",
                        egui::ColorImage::from_rgba_unmultiplied(
                            [p.image.width as usize, p.image.height as usize],
                            p.image.pixels.as_slice().as_flattened(),
                        ),
                        egui::TextureOptions::LINEAR,
                    ));
                    self.light_table.notices = p.notices;
                    self.light_table.cache = Some(key);
                }
                Err(e) => {
                    self.light_table.image = None;
                    self.light_table.notice = e;
                    self.light_table.cache = Some(key);
                }
            }
        }
        if !self.documents[n].light_table.visible {
            return;
        }
        let Some(texture) = self.light_table.image.clone() else {
            return;
        };
        let parent = self.documents[n].id;
        let Some(active) = self.active else { return };
        let origin = if self.documents[active].id == parent {
            [0., 0.]
        } else if let Some((child, _, offset)) = self.light_table_binding(parent) {
            if self.documents[active].id != child {
                return;
            }
            [-offset[0] as f64, -offset[1] as f64]
        } else {
            return;
        };
        let overlay = crate::canvas_view::TimelineOverlay {
            document: self.documents[active].id,
            texture,
            replace: false,
            origin,
            size: [
                self.documents[n].width as f64,
                self.documents[n].height as f64,
            ],
        };
        self.canvas_view.light_table_overlay = Some(overlay.clone());
        self.display.canvas.light_table_overlay = Some(overlay);
    }
    fn commit_light_table_form(&mut self, n: usize) -> Result<(), String> {
        self.finish_stroke();
        let mut data = self.documents[n].light_table.data.clone();
        let id = self.light_table.selected.ok_or("Choose a reference")?;
        let r = data.get_mut(id).ok_or("Reference removed")?;
        r.pose = self.light_table.pose;
        r.mode = self.light_table.mode;
        r.color = self.light_table.color;
        r.name = self.light_table.name.clone();
        r.scope = if self.light_table.cell_scope {
            Scope::Cell(self.light_table.cel.ok_or("Choose a cel scope")?)
        } else {
            Scope::Common
        };
        self.documents[n].edit_light_table(data)?;
        self.light_table.cache = None;
        Ok(())
    }
    pub(super) fn light_table_window(&mut self, ctx: &egui::Context) {
        if !self.light_table.open {
            return;
        }
        let Some(n) = self.light_table_context() else {
            return;
        };
        self.light_table_select_document(n);
        let d = &self.documents[n];
        let mut p = std::mem::take(&mut self.light_table);
        let mut open = p.open;
        let mut action = 0;
        let mut tool = self.workspaces.store.current.light_table_tool;
        let previous_tool = tool;
        let mut visible = d.light_table.visible;
        egui::Window::new("Light table and reference placement").id(egui::Id::new("light-table")).default_pos(egui::pos2(80.,40.)).default_width(730.).max_height(610.).vscroll(true).open(&mut open).show(ctx,|ui|{
  ui.label(&p.notice);if p.selected.is_none(){let r=ui.button("Register reference");p.add=Some(r.rect);if r.clicked(){action=1;}}ui.horizontal(|ui|{let r=ui.checkbox(&mut visible,"Show references on canvas (view)");p.show=Some(r.rect);let counts=d.light_table.counts();let r=ui.add_enabled(counts.0>0,egui::Button::new("Undo reference edit"));p.undo=Some(r.rect);if r.clicked(){action=2;}
if ui.add_enabled(counts.1>0,egui::Button::new("Redo reference edit")).clicked(){action=3;}});
  ui.horizontal(|ui|{let r=ui.button("Move reference");p.move_tool=Some(r.rect);if r.clicked(){tool=0;}let r=ui.button("Rotate reference");p.rotate_tool=Some(r.rect);if r.clicked(){tool=1;}let r=ui.button("Set pivot by click");p.pivot_tool=Some(r.rect);if r.clicked(){tool=2;}
let r=ui.button("Center on reference / canvas");p.center=Some(r.rect);if r.clicked(){action=8;}});
  ui.label("Manipulate the reference in the placement view. Drawing stays on the pinned cel. Move/rotate commits once on release; Escape cancels. Pivot clicks preserve the picture's current placement.");
  if let Some(texture)=&p.image{let size=texture.size_vec2();let factor=(620./size.x).min(180./size.y);let(rect,response)=ui.allocate_exact_size(size*factor,egui::Sense::click_and_drag());p.preview=Some(rect);ui.painter().rect_filled(rect,0.,egui::Color32::from_gray(70));ui.painter().image(texture.id(),rect,egui::Rect::from_min_max(egui::Pos2::ZERO,egui::pos2(1.,1.)),egui::Color32::WHITE);let doc_point=|pos:egui::Pos2|[(pos.x-rect.min.x)as f64/rect.width()as f64*d.width as f64,(pos.y-rect.min.y)as f64/rect.height()as f64*d.height as f64];if p.selected.is_some(){let pivot=[p.pose.pivot[0]+p.pose.offset[0],p.pose.pivot[1]+p.pose.offset[1]];let pos=egui::pos2(rect.min.x+(pivot[0]/d.width as f64)as f32*rect.width(),rect.min.y+(pivot[1]/d.height as f64)as f32*rect.height());ui.painter().circle_stroke(pos,4.,egui::Stroke::new(1.,egui::Color32::YELLOW));if response.drag_started()&&tool!=2&&let Some(pos)=ctx.input(|i|i.pointer.press_origin()){p.drag=Some((doc_point(pos),p.pose));}
if response.dragged()&&let(Some((start,pose)),Some(pos))=(p.drag,response.interact_pointer_pos()){p.pose=core::drag(pose,start,doc_point(pos),tool==1);p.cache=None;}
if response.drag_stopped()&&p.drag.take().is_some(){action=4;}
if tool==2&&response.clicked()&&let Some(pos)=response.interact_pointer_pos(){p.pose=core::pivot(p.pose,doc_point(pos));action=4;}}}
  if ctx.input(|i|i.key_pressed(egui::Key::Escape)){p.loaded=None;p.drag=None;p.cache=None;if action==4{action=0;}}
  for notice in &p.notices{ui.label(notice);}
  ui.horizontal(|ui|{ui.label("Editing cel / per-cel list");egui::ComboBox::from_id_salt("light-cell").selected_text(p.cel.map_or("No editable cel".into(),|id|d.scene.cels.get(id).map_or("Missing editing cel".into(),|s|format!("{} (cel{id})",s.name)))).show_ui(ui,|ui|{for s in &d.scene.cels.definitions{ui.selectable_value(&mut p.cel,Some(s.object),format!("{} (cel{})",s.name,s.object));}});let r=ui.add_enabled(p.cel.is_some(),egui::Button::new("Pin editing cel"));p.pin=Some(r.rect);if r.clicked(){action=6;}
if ui.add_enabled(d.light_table.pin.is_some(),egui::Button::new("Release editing pin")).clicked(){action=7;}});
  if d.light_table.pin.is_some(){ui.horizontal(|ui|{if ui.button("Apply cel content and release pin").clicked(){action=12;}
if ui.button("Return to parent and release pin").clicked(){action=13;}});}
  if let Some(pin)=d.light_table.pin{ui.label(format!("Editing is pinned to cel{}. {}",pin.source,if core::pin_valid(d){"Changing references or frames keeps this target."}else{"Cel removed/locked: drawing is blocked. Release the pin or choose an existing unlocked cel."}));}
  ui.separator();ui.horizontal(|ui|{ui.label("Overall reference opacity");ui.add(egui::Slider::new(&mut p.opacity,0..=100));if ui.button("Apply overall opacity").clicked(){action=5;}});
  egui::ComboBox::from_label("Placed reference").selected_text(p.selected.and_then(|id|d.light_table.data.get(id)).map_or("New reference",|r|r.name.as_str())).show_ui(ui,|ui|{ui.selectable_value(&mut p.selected,None,"New reference");for r in &d.light_table.data.references{ui.selectable_value(&mut p.selected,Some(r.id),format!("{} · {}",r.name,if matches!(r.scope,Scope::Common){"Common"}else{"Per cel"}));}});
  ui.horizontal(|ui|{ui.label("Name");ui.text_edit_singleline(&mut p.name);ui.checkbox(&mut p.cell_scope,"Only for chosen cel (off: common to all cels)");});
  if p.selected.is_some(){egui::Grid::new("light-placement").show(ui,|ui|{for(label,v,range)in [("Position x/y",&mut p.pose.offset,-1e9..=1e9),("Scale x/y",&mut p.pose.scale,0.01..=1000.),("Pivot x/y",&mut p.pose.pivot,-1e9..=1e9)]{ui.label(label);for x in v{ui.add(egui::DragValue::new(x).range(range.clone()).speed(0.1));}ui.end_row();}ui.label("Rotation / individual opacity");ui.add(egui::DragValue::new(&mut p.pose.degrees).range(-36000.0..=36000.));let mut opacity=p.pose.opacity*100.;ui.add(egui::DragValue::new(&mut opacity).range(0.0..=100.).suffix("%"));p.pose.opacity=opacity/100.;ui.end_row();});ui.horizontal(|ui|{for(mode,label)in [(ColorMode::Original,"Original"),(ColorMode::Half,"Half tint"),(ColorMode::Solid,"Solid tint")]{ui.selectable_value(&mut p.mode,mode,label);}ui.color_edit_button_srgb(&mut p.color);});ui.horizontal(|ui|{if ui.button("Apply reference placement").clicked(){action=4;}
if ui.button("Reset placement preview").clicked(){p.loaded=None;p.cache=None;}
if ui.button("Refresh reference source").clicked(){action=9;}
if ui.button("Remove reference").clicked(){action=10;}});if let Some(r)=p.selected.and_then(|id|d.light_table.data.get(id)){match &r.source{Source::File{path,..}=>{ui.label(format!("Local source: {} · Refresh reloads the file and keeps placement.",path.display()));},Source::Copy{title,..}=>{ui.label(format!("Frozen copy from {title}"));},Source::Layer(_)=>{ui.label("Live layer reference; drawing edits refresh it automatically.");},Source::Onion{frame,..}=>{ui.label(format!("Live reference at frame {}",frame+1));}}}}
  else{ui.horizontal(|ui|{for(kind,label)in [(0,"Layer"),(1,"Local file"),(2,"Other canvas copy"),(3,"Onion frame")]{ui.selectable_value(&mut p.kind,kind,label);}});match p.kind{0=>{egui::ComboBox::from_label("Source layer").selected_text(p.layer.and_then(|k|drawer_core::layer_catalog::rows(d).ok()?.into_iter().find(|r|r.key==k).map(|r|r.name)).unwrap_or("Choose a layer".into())).show_ui(ui,|ui|{if let Ok(rows)=drawer_core::layer_catalog::rows(d){for r in rows{if drawer_core::layer_groups::canonical(r.key){ui.selectable_value(&mut p.layer,Some(r.key),r.name);}}}});},1=>{ui.horizontal(|ui|{ui.text_edit_singleline(&mut p.file);if ui.button("Choose PNG / Drawer file").clicked(){action=11;}});},2=>{egui::ComboBox::from_label("Other canvas").selected_text(p.other.and_then(|id|self.documents.iter().find(|d|d.id==id)).map_or("Choose a different canvas",|d|d.title.as_str())).show_ui(ui,|ui|{for other in &self.documents{if other.id!=d.id&&d.light_table.pin.is_none_or(|p|p.child!=Some(other.id)){ui.selectable_value(&mut p.other,Some(other.id),&other.title);}}});},_=>{if let Some(a)=&d.animation{ui.add(egui::DragValue::new(&mut p.frame).range(1..=a.frames));egui::ComboBox::from_label("Reference track").selected_text(p.track.map_or("Whole drawing".into(),|id|format!("Cel track{id}"))).show_ui(ui,|ui|{ui.selectable_value(&mut p.track,None,"Whole drawing");for(i,t)in a.tracks.iter().enumerate(){ui.selectable_value(&mut p.track,Some(d.scene.catalog.tracks.id(i)),&t.name);}});}else{ui.label("Create a timeline to register onion frames.");}}}}
  ui.label("Reference edits have their own Undo and require document saving. Overall opacity multiplies individual opacity. References are view overlays and are excluded from output.");
 });
        p.open = open;
        self.light_table = p;
        self.documents[n].light_table.visible = visible;
        if previous_tool != tool {
            self.workspaces.store.current.light_table_tool = tool;
            if let Err(e) = self.persist_workspace() {
                self.light_table.notice = e;
            }
        }
        let result = (|| {
            match action {
                1 => {
                    self.finish_stroke();
                    let p = &self.light_table;
                    let scope = if p.cell_scope {
                        Scope::Cell(p.cel.ok_or("Choose a cel scope")?)
                    } else {
                        Scope::Common
                    };
                    let input = match p.kind {
                        0 => Add::Layer(p.layer.ok_or("Choose a source layer")?),
                        1 => Add::File(PathBuf::from(&p.file)),
                        2 => {
                            let other = self
                                .documents
                                .iter()
                                .find(|d| Some(d.id) == p.other)
                                .ok_or("Choose another canvas")?;
                            Add::Copy {
                                origin: other.id,
                                title: other.title.clone(),
                                image: core::capture(other)?,
                            }
                        }
                        _ => Add::Onion {
                            track: p.track,
                            frame: p.frame.saturating_sub(1),
                        },
                    };
                    let (data, id) = core::add(&self.documents[n], p.name.clone(), scope, input)?;
                    self.documents[n].edit_light_table(data)?;
                    self.light_table.selected = Some(id);
                    self.light_table.loaded = None;
                }
                2 => {
                    self.finish_stroke();
                    self.documents[n].undo_light_table();
                    self.light_table.loaded = None;
                }
                3 => {
                    self.finish_stroke();
                    self.documents[n].redo_light_table();
                    self.light_table.loaded = None;
                }
                4 => self.commit_light_table_form(n)?,
                5 => {
                    let mut data = self.documents[n].light_table.data.clone();
                    data.opacity = self.light_table.opacity;
                    self.documents[n].edit_light_table(data)?;
                }
                6 => {
                    self.light_table_pin(self.light_table.cel.ok_or("Choose an editable cel")?)?;
                }
                7 => {
                    self.finish_stroke();
                    self.documents[n].light_table.pin = None;
                }
                8 => {
                    let Some(active) = self.active else {
                        return Ok(());
                    };
                    let center = self
                        .light_table
                        .selected
                        .and_then(|id| self.documents[n].light_table.data.get(id))
                        .map_or(
                            [
                                self.documents[n].width as f64 * 0.5,
                                self.documents[n].height as f64 * 0.5,
                            ],
                            |r| {
                                [
                                    r.pose.pivot[0] + r.pose.offset[0],
                                    r.pose.pivot[1] + r.pose.offset[1],
                                ]
                            },
                        );
                    let offset = self
                        .light_table_binding(self.documents[n].id)
                        .filter(|b| b.0 == self.documents[active].id)
                        .map_or([0, 0], |b| b.2);
                    let viewport = self
                        .canvas_view
                        .viewport_rect()
                        .unwrap_or(ctx.content_rect());
                    let d = &mut self.documents[active];
                    d.view.center_on(
                        [center[0] - offset[0] as f64, center[1] - offset[1] as f64],
                        [d.width as f64, d.height as f64],
                        [viewport.center().x as f64, viewport.center().y as f64],
                        ctx.pixels_per_point() as f64,
                    );
                }
                9 => {
                    self.light_table.cache = None;
                    let data = core::refresh(
                        &self.documents[n],
                        self.light_table.selected.ok_or("Choose a reference")?,
                    )?;
                    self.documents[n].edit_light_table(data)?;
                }
                10 => {
                    let mut data = self.documents[n].light_table.data.clone();
                    data.references
                        .retain(|r| Some(r.id) != self.light_table.selected);
                    self.documents[n].edit_light_table(data)?;
                    self.light_table.selected = None;
                    self.light_table.loaded = None;
                }
                12 => self.apply_timeline_cel()?,
                13 => {
                    self.finish_stroke();
                    self.return_timeline_cel();
                }
                11 => {
                    if let Some(path) = rfd::FileDialog::new()
                        .add_filter("PNG / Drawer", &["png", "drawer"])
                        .pick_file()
                    {
                        self.light_table.file = path.to_string_lossy().into_owned();
                    }
                }
                _ => {}
            }
            Ok(())
        })();
        if let Err(e) = result {
            self.light_table.notice = e;
        }
        if action > 0 {
            ctx.request_repaint();
        }
    }
}

pub(super) fn smoke(root: &Path) -> Result<(), String> {
    drawer_core::light_table_verify::verify(root)?;
    let ctx = egui::Context::default();
    let mut app = Drawer::new(&ctx, None);
    app.simple_view = true;
    app.storage =
        Some(UserStorage::at(root.join("light-table-settings")).map_err(|e| e.to_string())?);
    let result = (|| {
        app.add_document(drawer_core::light_table_verify::fixture());
        app.execute_command(&ctx, "light-table");
        if !app.light_table.open {
            return Err("Light table command missing".into());
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
        let drag = |app: &mut Drawer, time: &mut f64, start: egui::Pos2, end: egui::Pos2| {
            frame(
                app,
                time,
                vec![
                    egui::Event::PointerMoved(start),
                    egui::Event::PointerButton {
                        pos: start,
                        button: egui::PointerButton::Primary,
                        pressed: true,
                        modifiers: Default::default(),
                    },
                ],
            );
            frame(app, time, vec![egui::Event::PointerMoved(end)]);
            frame(
                app,
                time,
                vec![egui::Event::PointerButton {
                    pos: end,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: Default::default(),
                }],
            );
        };
        for _ in 0..4 {
            frame(&mut app, &mut time, vec![]);
        }
        let h = app.documents[0].history_counts();
        let state = app.documents[0].snapshot();
        let r = app.light_table.add.ok_or("Register control missing")?;
        click(&mut app, &mut time, r);
        for _ in 0..2 {
            frame(&mut app, &mut time, vec![]);
        }
        if app.documents[0].light_table.data.references.len() != 1
            || !app.documents[0].dirty
            || app.documents[0].snapshot() != state
            || app.documents[0].history_counts() != h
        {
            return Err(format!(
                "Pointer registration/history isolation failed: {} refs={} dirty={} same_state={} history={:?}/{:?} rect={:?}",
                app.light_table.notice,
                app.documents[0].light_table.data.references.len(),
                app.documents[0].dirty,
                app.documents[0].snapshot() == state,
                app.documents[0].history_counts(),
                h,
                r
            ));
        }
        app.request_close(0);
        if app.pending_close.is_none() {
            return Err("Reference-only edit did not request saving on close".into());
        }
        app.pending_close = None;
        let id = app.documents[0].light_table.data.references[0].id;
        let count = app.documents[0].light_table.counts().0;
        let r = app.light_table.pivot_tool.ok_or("Pivot tool missing")?;
        click(&mut app, &mut time, r);
        let preview = app.light_table.preview.ok_or("Placement view missing")?;
        let pos = egui::pos2(
            preview.left() + preview.width() * 0.4,
            preview.top() + preview.height() * 0.3,
        );
        click(
            &mut app,
            &mut time,
            egui::Rect::from_center_size(pos, egui::vec2(1., 1.)),
        );
        for _ in 0..2 {
            frame(&mut app, &mut time, vec![]);
        }
        let pose = app.documents[0].light_table.data.get(id).unwrap().pose;
        if (pose.pivot[0] - 4.).abs() > 0.1 || app.documents[0].light_table.counts().0 != count + 1
        {
            return Err(format!(
                "Pointer pivot failed: {:?} {}",
                pose, app.light_table.notice
            ));
        }
        let r = app.light_table.move_tool.unwrap();
        click(&mut app, &mut time, r);
        let preview = app.light_table.preview.unwrap();
        let start = egui::pos2(preview.left() + preview.width() * 0.4, preview.center().y);
        let end = start + egui::vec2(preview.width() * 0.1, 0.);
        let count = app.documents[0].light_table.counts().0;
        drag(&mut app, &mut time, start, end);
        for _ in 0..2 {
            frame(&mut app, &mut time, vec![]);
        }
        if (app.documents[0]
            .light_table
            .data
            .get(id)
            .unwrap()
            .pose
            .offset[0]
            - 1.)
            .abs()
            > 0.1
            || app.documents[0].light_table.counts().0 != count + 1
        {
            return Err(format!(
                "Pointer move was not one reference Undo: {}",
                app.light_table.notice
            ));
        }
        let r = app.light_table.rotate_tool.unwrap();
        click(&mut app, &mut time, r);
        let preview = app.light_table.preview.unwrap();
        let pose = app.documents[0].light_table.data.get(id).unwrap().pose;
        let center = [
            pose.pivot[0] + pose.offset[0],
            pose.pivot[1] + pose.offset[1],
        ];
        let screen = |p: [f64; 2]| {
            egui::pos2(
                preview.left() + preview.width() * (p[0] / 10.) as f32,
                preview.top() + preview.height() * p[1] as f32,
            )
        };
        drag(
            &mut app,
            &mut time,
            screen([center[0] + 1., center[1]]),
            screen([center[0], center[1] + 0.4]),
        );
        for _ in 0..2 {
            frame(&mut app, &mut time, vec![]);
        }
        if (app.documents[0]
            .light_table
            .data
            .get(id)
            .unwrap()
            .pose
            .degrees
            - 90.)
            .abs()
            > 1.
        {
            return Err(format!(
                "Pointer rotation failed: {}",
                app.light_table.notice
            ));
        }
        let h = app.documents[0].history_counts();
        let dirty = app.documents[0].dirty;
        let count = app.documents[0].light_table.counts();
        let r = app.light_table.show.unwrap();
        click(&mut app, &mut time, r);
        if app.documents[0].history_counts() != h
            || app.documents[0].dirty != dirty
            || app.documents[0].light_table.counts() != count
        {
            return Err("Reference visibility changed document/auxiliary history".into());
        }
        let pending_state = app.documents[0].snapshot();
        let pending_counts = app.documents[0].light_table.counts();
        app.light_table.pose.offset[0] += 0.5;
        let pending_path = root.join("pending-reference.drawer");
        if app.save_to(0, &pending_path).is_ok() || pending_path.exists() {
            return Err("Pending reference preview saved stale placement".into());
        }
        app.request_close(0);
        if app.pending_close.is_some()
            || app.documents[0].snapshot() != pending_state
            || app.documents[0].light_table.counts() != pending_counts
        {
            return Err("Pending placement close changed stored data".into());
        }
        app.light_table.loaded = None;
        frame(&mut app, &mut time, vec![]);
        let before = app.documents[0].snapshot();
        let h = app.documents[0].history_counts();
        let auxiliary = app.documents[0].light_table.counts();
        let dirty = app.documents[0].dirty;
        let old_view = app.documents[0].view;
        let r = app.light_table.center.unwrap();
        click(&mut app, &mut time, r);
        if app.documents[0].view == old_view
            || app.documents[0].snapshot() != before
            || app.documents[0].history_counts() != h
            || app.documents[0].light_table.counts() != auxiliary
            || app.documents[0].dirty != dirty
        {
            return Err(
                "Center reference view changed document/history or did not move the view".into(),
            );
        }
        let r = app.light_table.move_tool.unwrap();
        click(&mut app, &mut time, r);
        let data = app.documents[0].light_table.data.clone();
        let counts = app.documents[0].light_table.counts();
        let preview = app.light_table.preview.unwrap();
        let start = preview.center();
        let end = start + egui::vec2(preview.width() * 0.05, 0.);
        frame(
            &mut app,
            &mut time,
            vec![
                egui::Event::PointerMoved(start),
                egui::Event::PointerButton {
                    pos: start,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: Default::default(),
                },
            ],
        );
        frame(&mut app, &mut time, vec![egui::Event::PointerMoved(end)]);
        frame(
            &mut app,
            &mut time,
            vec![
                egui::Event::Key {
                    key: egui::Key::Escape,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: Default::default(),
                },
                egui::Event::PointerButton {
                    pos: end,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: Default::default(),
                },
            ],
        );
        frame(
            &mut app,
            &mut time,
            vec![egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed: false,
                repeat: false,
                modifiers: Default::default(),
            }],
        );
        if app.documents[0].light_table.data != data
            || app.documents[0].light_table.counts() != counts
        {
            return Err("Escape/release committed a cancelled reference drag".into());
        }
        let r = app.light_table.rotate_tool.unwrap();
        click(&mut app, &mut time, r);
        // Explicit pin opens a separate fixed source canvas and prevents parent/fallback drawing.
        let source = app.documents[0].scene.cels.definitions[0].object;
        app.light_table.cel = Some(source);
        let r = app.light_table.pin.unwrap();
        click(&mut app, &mut time, r);
        for _ in 0..2 {
            frame(&mut app, &mut time, vec![]);
        }
        let mut child = app.documents[0]
            .light_table
            .pin
            .and_then(|p| p.child)
            .ok_or("Pinned content canvas missing")?;
        let child_index = app.documents.iter().position(|d| d.id == child).unwrap();
        if !app.light_table_input_ready() {
            return Err("Pinned cel content cannot receive input".into());
        }
        app.documents[0].current_frame = 5;
        if app.documents[0].light_table.pin.unwrap().source != source {
            return Err("Frame change redirected pin".into());
        }
        let parent_tab = app
            .tabs
            .iter()
            .position(|t| t.document_id == app.documents[0].id)
            .unwrap();
        app.select_tab(parent_tab);
        if app.light_table_input_ready() {
            return Err("Parent drawing stayed enabled while editing cel was pinned".into());
        }
        let mut child_tab = app
            .tabs
            .iter()
            .position(|t| t.document_id == child)
            .unwrap();
        app.select_tab(child_tab);
        let parent = app.documents[0].snapshot();
        app.documents[child_index].edit_pixel(0, 0, 0, [0, 255, 100, 255]);
        if app.documents[0].snapshot() != parent {
            return Err("Pinned content stroke wrote another layer before Apply".into());
        }
        app.apply_timeline_cel()?;
        if app.documents[0].light_table.pin.is_some()
            || app.documents[0]
                .scene
                .cels
                .get(source)
                .unwrap()
                .document()
                .unwrap()
                .layers[0]
                .pixels[0]
                != [0, 255, 100, 255]
            || !app.light_table_input_ready()
        {
            return Err("Applying pinned content did not update the source/release the pin".into());
        }
        app.light_table_pin(source)?;
        child = app.documents[0].light_table.pin.unwrap().child.unwrap();
        child_tab = app
            .tabs
            .iter()
            .position(|t| t.document_id == child)
            .unwrap();
        let prepared = drawer_core::timeline_editor::prepare(
            &app.documents[0],
            drawer_core::timeline_editor::Edit::DeleteCel(source),
            DEFAULT_DOCUMENT_BUDGET,
        )?;
        app.documents[0].commit_state(prepared.state)?;
        if app.light_table_input_ready()
            || app.documents[0].light_table.pin.unwrap().source != source
        {
            return Err("Removed pinned cel fell back to another drawing target".into());
        }
        app.documents[0].undo();
        if !app.light_table_input_ready() {
            return Err("Undo of pinned cel deletion did not restore target availability".into());
        }
        let pin = app.documents[0].light_table.pin;
        app.add_document(drawer_core::onion_verify::fixture());
        for _ in 0..2 {
            frame(&mut app, &mut time, vec![]);
        }
        if !app.light_table_input_ready()
            || app.documents[0].light_table.pin != pin
            || app.canvas_view.light_table_overlay.is_some()
        {
            return Err("Document switch lost its independent pin or leaked references".into());
        }
        app.select_tab(child_tab);
        for _ in 0..2 {
            frame(&mut app, &mut time, vec![]);
        }
        if !app.light_table_input_ready() || app.documents[0].light_table.pin != pin {
            return Err("Returning to the pinned content changed its drawing target".into());
        }
        app.persist_workspace()?;
        let store = drawer_core::workspaces::Store::load(app.storage.as_ref().unwrap())
            .map_err(|e| e.to_string())?;
        if store
            .ok_or("Saved light-table tool settings missing")?
            .current
            .light_table_tool
            != 1
        {
            return Err("Manipulation tool default did not survive local reload".into());
        }
        Ok(())
    })();
    drop(app);
    let cleanup =
        std::fs::remove_dir_all(root.join("light-table-settings")).map_err(|e| e.to_string());
    result.and(cleanup)
}
#[cfg(test)]
mod tests {
    #[test]
    fn pointer_reference_pivot_drag_rotation_pin_close_and_history() {
        let root =
            std::env::temp_dir().join(format!("drawer-light-table-ui-{}", std::process::id()));
        std::fs::create_dir(&root).unwrap();
        let r = super::smoke(&root);
        std::fs::remove_dir(&root).unwrap();
        r.unwrap();
    }
}
