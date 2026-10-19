use super::*;
use drawer_core::line_tools::{self as core, Data, Target};
#[derive(Default)]
pub(super) struct Tools {
    pub open: bool,
    editor: Option<Editor>,
    pick: Option<Pick>,
    buttons: [Option<egui::Rect>; 2],
}
struct Editor {
    id: u64,
    revision: u64,
    draft: Data,
    layer: Option<u64>,
    points: String,
    closed: bool,
    width: f64,
    seed: [f64; 2],
    radius: f64,
}
#[derive(Clone, Copy)]
enum Action {
    Fill(usize, [u8; 4]),
    Erase(f64),
}
struct Pick {
    id: u64,
    revision: u64,
    action: Action,
}
impl Tools {
    pub fn modal(&self) -> bool {
        self.open || self.pick.is_some()
    }
}
fn toggle<T: PartialEq + Copy>(ui: &mut egui::Ui, list: &mut Vec<T>, value: T, label: &str) {
    let mut yes = list.contains(&value);
    if ui.checkbox(&mut yes, label).changed() {
        if yes {
            list.push(value)
        } else {
            list.retain(|v| *v != value)
        }
    }
}
fn parse_points(text: &str) -> Result<Vec<[f64; 2]>, String> {
    if text.len() > 131072 {
        return Err("Point text limit".into());
    }
    let mut out = vec![];
    for line in text.lines().filter(|s| !s.trim().is_empty()) {
        let parts = line
            .split(',')
            .map(str::trim)
            .map(str::parse::<f64>)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| "Use one x,y pair per line")?;
        if parts.len() != 2 {
            return Err("Use one x,y pair per line".into());
        }
        let point = [parts[0], parts[1]];
        if !core::point_valid(point) {
            return Err("Invalid path point".into());
        }
        out.push(point);
        if out.len() > 4096 {
            return Err("Path point limit".into());
        }
    }
    Ok(out)
}
impl Drawer {
    pub(super) fn line_inputs(&mut self, ctx: &egui::Context) {
        let Some(pick) = self.lines.pick.take() else {
            return;
        };
        if ctx.input(|i| {
            !i.focused
                || i.key_pressed(egui::Key::Escape)
                || i.events
                    .iter()
                    .any(|e| matches!(e, egui::Event::WindowFocused(false)))
        }) {
            self.lines.open = true;
            return;
        }
        let Some(n) = self.active.filter(|n| {
            self.documents[*n].id == pick.id && self.documents[*n].revision == pick.revision
        }) else {
            self.status = Some("Document changed; line operation cancelled".into());
            self.lines.open = true;
            return;
        };
        let rect = self.canvas_view.viewport_rect();
        let click = ctx.input(|i| {
            i.events.iter().find_map(|e| match e {
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    ..
                } if rect.is_some_and(|r| r.contains(*pos)) => Some(*pos),
                _ => None,
            })
        });
        if let Some(pos) = click {
            let d = &mut self.documents[n];
            let r = rect.unwrap();
            let point = d.view.screen_to_document(
                [pos.x as f64, pos.y as f64],
                [d.width as f64, d.height as f64],
                [r.center().x as f64, r.center().y as f64],
                ctx.pixels_per_point() as f64,
            );
            let result = match pick.action {
                Action::Fill(layer, color) => {
                    drawer_core::line_regions::fill(d, layer, point, color)
                }
                Action::Erase(radius) => core::erase(d, point, radius),
            };
            self.status = Some(match result {
                Ok(true) => "Line operation applied as one undo step".into(),
                Ok(false) => "No pixels or paths changed".into(),
                Err(e) => e,
            });
            self.lines.open = true;
            self.lines.editor = None;
        } else {
            self.lines.pick = Some(pick)
        }
    }
    pub(super) fn line_window(&mut self, ctx: &egui::Context) {
        if self.lines.pick.is_some() {
            egui::Window::new("Choose line operation point").show(ctx, |ui| {
                ui.label("Click the main tab canvas. Escape or focus loss cancels the point pick.");
            });
            return;
        }
        if !self.lines.open {
            return;
        }
        self.finish_stroke();
        let Some(n) = self.active else {
            self.lines.open = false;
            return;
        };
        let d = &self.documents[n];
        if self.lines.editor.is_none() {
            self.lines.editor = Some(Editor {
                id: d.id,
                revision: d.revision,
                draft: d.scene.lines.clone(),
                layer: d.scene.lines.layers.first().map(|l| l.id),
                points: "8,8\n48,8\n48,36\n8,36".into(),
                closed: true,
                width: 1.0,
                seed: [24.0, 24.0],
                radius: 4.0,
            })
        }
        let mut e = self.lines.editor.take().unwrap();
        if e.id != d.id || e.revision != d.revision {
            self.status = Some("Document changed; reopen the line editor".into());
            self.lines.open = false;
            return;
        }
        let mut open = true;
        let mut apply = false;
        let mut cancel = false;
        let mut action = None;
        let mut immediate = false;
        let mut ui_layer = None;
        let color = self.references.session.workspace.color;
        egui::Window::new("Line references and vector layers").open(&mut open).default_width(650.0).vscroll(true).show(ctx,|ui|{
            ui_layer=Some(ui.layer_id());
            ui.label("Vector layers are editable paths above raster layers. Boundary/magnet references, intersection references and erase targets are independent.");
            ui.horizontal(|ui|{let a=ui.button("Apply line settings");let c=ui.button("Cancel draft");self.lines.buttons=[Some(a.rect),Some(c.rect)];apply=a.clicked();cancel=c.clicked();});
            if ui.button("Add vector layer to draft").clicked(){match e.draft.add_layer(&format!("Vector {}",e.draft.layers.len()+1)){Ok(id)=>e.layer=Some(id),Err(err)=>self.status=Some(err)}}
            for l in &mut e.draft.layers{ui.push_id(l.id,|ui|{ui.horizontal(|ui|{ui.selectable_value(&mut e.layer,Some(l.id),format!("#{}",l.id));ui.text_edit_singleline(&mut l.name);ui.checkbox(&mut l.hidden,"Hidden");ui.checkbox(&mut l.locked,"Locked");ui.label(format!("{} paths",l.paths.len()));});});}
            ui.collapsing("Create a vector path",|ui|{
                ui.label("Document pixel coordinates, one x,y per line. Magnet is applied to these vertices before storing page coordinates.");ui.text_edit_multiline(&mut e.points);ui.checkbox(&mut e.closed,"Closed path");super::edit::number(ui,&mut e.width,0.1..=512.0,0.1," px line width","line-references");
                if ui.button("Add path to selected draft layer").clicked(){let result=(||{let id=e.layer.ok_or("Select a vector layer")?;let points=parse_points(&e.points)?.into_iter().map(|p|core::snap(&e.draft,d.scene.transform,p).map(|q|d.scene.transform.inverse().map(q))).collect::<Result<Vec<_>,_>>()?;e.draft.add_path(id,points,e.closed,e.width,color)?;Ok::<_,String>(())})();if let Err(err)=result{self.status=Some(err)}}
                if let Some(l)=e.draft.layers.iter_mut().find(|l|Some(l.id)==e.layer){let mut remove=None;for(i,p)in l.paths.iter().enumerate(){ui.horizontal(|ui|{ui.label(format!("Path #{}: {} vertices{}",p.id,p.points.len(),if p.closed{" (closed)"}else{""}));if ui.add_enabled(!l.locked&&!l.hidden,egui::Button::new("Remove path")).clicked(){remove=Some(i)}});}if let Some(i)=remove{l.paths.remove(i);}}
            });
            e.draft.prune_selection();
            let s=&mut e.draft.settings;ui.checkbox(&mut s.clip_paint,"Restrict brush coloring to the closed reference region at stroke start");ui.checkbox(&mut s.magnet,"Magnet new vector vertices to chosen vector references");super::edit::number(ui,&mut s.distance,0.0..=1024.0,0.5," document px magnet distance","line-references");super::edit::number(ui,&mut s.darkness,1.0..=255.0,1.0," raster darkness × alpha barrier threshold","line-references");
            ui.label("Each row: boundary/magnet · intersection · actual erase target");
            for(i,l)in d.layers.iter().enumerate(){ui.push_id(("raster",i),|ui|{ui.horizontal(|ui|{ui.label(format!("Raster {}: {}{}{}",i+1,l.name,if l.locked{" [LOCKED]"}else{""},if !d.scene.paper.raster_visible(i,d.layers.len()){" [HIDDEN]"}else{""}));toggle(ui,&mut s.references,Target::Raster(i),"Boundary");toggle(ui,&mut s.erase,Target::Raster(i),"Erase");});});}
            for l in &e.draft.layers{ui.push_id(("vector",l.id),|ui|{ui.horizontal(|ui|{ui.label(format!("{}{}{}",l.name,if l.locked{" [LOCKED]"}else{""},if l.hidden{" [HIDDEN]"}else{""}));toggle(ui,&mut s.references,Target::Vector(l.id),"Boundary / magnet");toggle(ui,&mut s.intersections,l.id,"Intersections");toggle(ui,&mut s.erase,Target::Vector(l.id),"Erase");});});}
            if let Some(a)=&d.animation{for(i,t)in a.tracks.iter().enumerate(){ui.label(format!("Animation track {}: {} — unsupported erase target",i+1,t.name));}}
            if !d.scene.objects.is_empty(){ui.label("Page text/tone/shadow objects — unsupported erase targets");}
            let checks=core::preflight_data(d,&e.draft);let ready=!checks.is_empty()&&checks.iter().all(|(_,e)|e.is_none());for(t,error)in checks{ui.label(format!("Erase {t:?}: {}",error.as_deref().unwrap_or("ready")));}
            ui.label("Locked references can be read. Hidden references are ignored. Any selected invalid erase target cancels the whole operation. Clear a raster selection before erasing editable vectors.");
            ui.horizontal(|ui|{for(axis,p)in e.seed.iter_mut().enumerate(){super::edit::number(ui,p,-1e9..=1e9,1.0,if axis==0{" point X"}else{" point Y"},"line-references");}});super::edit::number(ui,&mut e.radius,0.1..=1024.0,0.5," document px erase radius","line-references");
            let raster=match self.panels.targets.get(&d.id){Some(drawer_core::thumbnail::Target::Raster(n))=>Some(*n),None=>Some(0),_=>None};
            ui.horizontal(|ui|{
                if ui.add_enabled(raster.is_some(),egui::Button::new("Apply and pick fill point")).clicked(){action=Some(Action::Fill(raster.unwrap(),color));}
                if ui.add_enabled(ready,egui::Button::new("Apply and pick erase point")).clicked(){action=Some(Action::Erase(e.radius));}
            });
            ui.horizontal(|ui|{if ui.add_enabled(raster.is_some(),egui::Button::new("Apply and fill at X,Y")).clicked(){action=Some(Action::Fill(raster.unwrap(),color));immediate=true;}
            if ui.add_enabled(ready,egui::Button::new("Apply and erase at X,Y")).clicked(){action=Some(Action::Erase(e.radius));immediate=true;}});
        });
        if cancel || !open || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.lines.open = false;
            return;
        }
        if (apply || action.is_some())
            && ui_layer.is_some_and(|l| super::edit::invalid_layer(ctx, l))
        {
            self.status = Some("Correct the invalid numeric input first".into());
            self.lines.editor = Some(e);
            return;
        }
        if apply || action.is_some() {
            let d = &mut self.documents[n];
            let mut scene = d.scene.clone();
            scene.lines = e.draft.clone();
            scene.lines.prune_selection();
            match drawer_core::layer_defaults::scene(
                d,
                scene,
                &self.workspaces.store.current.layer_defaults,
            )
            .and_then(|scene| d.edit_scene(scene))
            {
                Err(err) => {
                    self.status = Some(err);
                    self.lines.editor = Some(e);
                    return;
                }
                Ok(_) => e.revision = d.revision,
            }
            if let Some(action) = action {
                if immediate {
                    let result = match action {
                        Action::Fill(layer, color) => {
                            drawer_core::line_regions::fill(d, layer, e.seed, color)
                        }
                        Action::Erase(r) => core::erase(d, e.seed, r),
                    };
                    self.status = Some(match result {
                        Ok(_) => "Line operation applied".into(),
                        Err(err) => err,
                    });
                    e.revision = d.revision;
                } else {
                    self.lines.pick = Some(Pick {
                        id: d.id,
                        revision: d.revision,
                        action,
                    });
                    self.lines.open = false;
                }
            }
            if apply {
                self.lines.open = false;
                return;
            }
        }
        self.lines.editor = Some(e);
    }
}

fn click(pos: egui::Pos2, pressed: bool) -> Vec<egui::Event> {
    vec![
        egui::Event::PointerMoved(pos),
        egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        },
    ]
}
pub(super) fn smoke(root: &std::path::Path) -> Result<(), String> {
    drawer_core::line_tools_verify::verify(root)?;
    let ctx = egui::Context::default();
    let mut time = 0.0;
    let mut frame = |app: &mut Drawer, events| {
        time += 0.1;
        super::input_replay::frame(&ctx, app, time, events);
    };
    let mut app = Drawer::new(&ctx, None);
    app.create_canvas(CanvasSpec {
        width: 64,
        height: 48,
        dpi: 300.0,
        background: Background::Transparent,
    })
    .map_err(|e| e.to_string())?;
    app.simple_view = true;
    app.documents[0].scene = drawer_core::line_tools_verify::fixture()?.scene;
    frame(&mut app, vec![]);
    frame(&mut app, vec![]);
    for (zoom, angle, flip) in [(0.5, 0.0, false), (2.0, 90.0, true)] {
        app.documents[0].view.zoom = zoom;
        app.documents[0].view.angle = angle;
        app.documents[0].view.flipped = flip;
        frame(&mut app, vec![]);
        let d = &app.documents[0];
        let rect = app
            .canvas_view
            .viewport_rect()
            .ok_or("Line point canvas missing")?;
        let pos = d.view.document_to_screen(
            [24.0, 24.0],
            [64.0, 48.0],
            [rect.center().x as f64, rect.center().y as f64],
            ctx.pixels_per_point() as f64,
        );
        let before = d.snapshot();
        let history = d.history_counts().0;
        app.lines.pick = Some(Pick {
            id: d.id,
            revision: d.revision,
            action: Action::Fill(0, [220, 80, 40, 128]),
        });
        frame(
            &mut app,
            click(egui::pos2(pos[0] as f32, pos[1] as f32), true),
        );
        frame(
            &mut app,
            click(egui::pos2(pos[0] as f32, pos[1] as f32), false),
        );
        let d = &mut app.documents[0];
        if d.layers[0].pixel(64, 48, 24, 24) != [220, 80, 40, 128]
            || d.layers[0].pixel(64, 48, 55, 24) != [0; 4]
            || d.history_counts().0 != history + 1
            || !d.undo()
            || d.snapshot() != before
        {
            return Err("Canvas fill point/view or one-step undo failed".into());
        }
        app.lines.open = false;
        app.lines.editor = None;
    }
    // Draft cancellation is exercised through the actual window's mouse button.
    app.lines.open = true;
    frame(&mut app, vec![]);
    let before = app.documents[0].snapshot();
    app.lines
        .editor
        .as_mut()
        .ok_or("Missing line draft")?
        .draft
        .add_layer("Discard me")?;
    let pos = app.lines.buttons[1]
        .ok_or("Missing Cancel draft button")?
        .center();
    frame(&mut app, click(pos, true));
    frame(&mut app, click(pos, false));
    if app.lines.open || app.documents[0].snapshot() != before {
        return Err("Line draft cancel changed the document".into());
    }
    app.documents[0].scene.lines.settings.clip_paint = true;
    app.panels.settings.brush_size = 24.0;
    app.workspaces.store.current.input.smoothing[0] = 0.0;
    app.workspaces.store.current.pressure.tools[0].size = false;
    app.workspaces.store.current.pressure.tools[0].opacity = false;
    frame(&mut app, vec![]);
    use drawer_core::pointer_input::Phase;
    let mut events = vec![];
    for (point, phase) in [
        ([24.0, 24.0], Phase::Down),
        ([60.0, 24.0], Phase::Move),
        ([60.0, 24.0], Phase::Up),
    ] {
        events.push(super::precision::pointer(&ctx, &app, point, phase));
    }
    app.drawing.virtual_events(events, false);
    frame(&mut app, vec![]);
    if app.documents[0].layers[0].pixel(64, 48, 24, 24)[3] == 0
        || app.documents[0].layers[0].pixel(64, 48, 55, 24)[3] != 0
    {
        return Err("Production brush did not respect line region".into());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn actual_canvas_picks_draft_cancel_and_brush_references() {
        let root = crate::tests::TestDirectory::new();
        super::smoke(&root.0).unwrap();
    }
}
