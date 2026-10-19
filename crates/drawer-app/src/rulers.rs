use super::*;
use drawer_core::rulers::{self as core, Data, Kind};
#[derive(Default)]
pub(super) struct Panel {
    pub open: bool,
    editor: Option<Editor>,
    pub apply_button: Option<egui::Rect>,
}
struct Editor {
    id: u64,
    revision: u64,
    data: Data,
    name: String,
    points: String,
    kind: usize,
    arms: u8,
    edit: Option<u64>,
}
impl Editor {
    fn add(&mut self, doc: &Document) -> Result<(), String> {
        if self.points.len() > 2048 {
            return Err("Ruler coordinate input limit".into());
        }
        let points = self
            .points
            .lines()
            .filter(|s| !s.trim().is_empty())
            .map(|line| {
                let values = line
                    .split(',')
                    .map(|s| {
                        s.trim()
                            .parse::<f64>()
                            .map_err(|_| "Enter x,y on each line")
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                if values.len() != 2 || values.iter().any(|v| !v.is_finite() || v.abs() > 1e9) {
                    return Err("Use finite document coordinates x,y");
                }
                Ok(doc.scene.transform.inverse().map([values[0], values[1]]))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let expected = match self.kind {
            0 | 1 => 2,
            2 | 3 => 1,
            4 => 2,
            _ => 3,
        };
        if points.len() != expected {
            return Err(format!("This ruler needs {expected} coordinate lines"));
        }
        let kind = match self.kind {
            0 => Kind::Straight {
                a: points[0],
                b: points[1],
            },
            1 => Kind::Mirror {
                a: points[0],
                b: points[1],
            },
            2 => Kind::Radial {
                center: points[0],
                arms: self.arms,
            },
            _ => Kind::Perspective { points },
        };
        let mut data = self.data.clone();
        if let Some(id) = self.edit {
            let g = data
                .guides
                .iter_mut()
                .find(|g| g.id == id)
                .ok_or("The ruler was removed")?;
            g.kind = kind;
            g.name = self.name.trim().into();
            data.active = Some(id);
            if !data.valid() {
                return Err("Invalid ruler fields".into());
            }
        } else {
            data.add(&self.name, kind)?;
        }
        self.data = data;
        Ok(())
    }
    fn load(&mut self, id: u64, doc: &Document) {
        if let Some(g) = self.data.guides.iter().find(|g| g.id == id) {
            self.name = g.name.clone();
            self.edit = Some(id);
            let points = match &g.kind {
                Kind::Special(_) => return,
                Kind::Straight { a, b } => {
                    self.kind = 0;
                    vec![*a, *b]
                }
                Kind::Mirror { a, b } => {
                    self.kind = 1;
                    vec![*a, *b]
                }
                Kind::Radial { center, arms } => {
                    self.kind = 2;
                    self.arms = *arms;
                    vec![*center]
                }
                Kind::Perspective { points } => {
                    self.kind = points.len() + 2;
                    points.clone()
                }
            };
            self.points = points
                .into_iter()
                .map(|p| {
                    let p = doc.scene.transform.map(p);
                    format!("{},{}", p[0], p[1])
                })
                .collect::<Vec<_>>()
                .join("\n");
        }
    }
}
impl Drawer {
    pub(super) fn begin_rulers(&mut self) -> Result<(), String> {
        let n = self.active.ok_or("Open a page first")?;
        let d = &self.documents[n];
        self.ruler_tools.editor = Some(Editor {
            id: d.id,
            revision: d.revision,
            data: d.scene.rulers.clone(),
            name: "Ruler".into(),
            points: format!(
                "0,{}\n{},{}",
                d.height as f64 / 2.0,
                d.width,
                d.height as f64 / 2.0
            ),
            kind: 0,
            arms: 4,
            edit: None,
        });
        self.ruler_tools.open = true;
        Ok(())
    }
    fn apply_rulers(&mut self) -> Result<(), String> {
        let e = self.ruler_tools.editor.as_ref().ok_or("No ruler draft")?;
        let d = self
            .documents
            .iter_mut()
            .find(|d| d.id == e.id && d.revision == e.revision)
            .ok_or("The page changed; reopen the ruler editor")?;
        core::apply(d, e.data.clone())?;
        self.ruler_tools.open = false;
        self.ruler_tools.editor = None;
        Ok(())
    }
    pub(super) fn ruler_window(&mut self, ctx: &egui::Context) {
        if !self.ruler_tools.open {
            return;
        }
        let Some(mut e) = self.ruler_tools.editor.take() else {
            self.ruler_tools.open = false;
            return;
        };
        let Some(n) = self.documents.iter().position(|d| d.id == e.id) else {
            self.ruler_tools.open = false;
            return;
        };
        let mut open = true;
        let mut apply = false;
        let mut cancel = false;
        let mut advanced = false;
        let mut follow = false;
        let mut move_layer = false;
        self.ruler_tools.apply_button = None;
        egui::Window::new("Drawing rulers").open(&mut open).vscroll(true).show(ctx,|ui|{
            ui.label("Coordinates are document pixels. Rulers are saved with the page. V switches snapping for following strokes without changing the page.");
            ui.horizontal_wrapped(|ui|{
                if ui.button("Special rulers and layer scope…").clicked(){advanced=true;}
                if ui.button("Draw along saved ruler…").clicked(){follow=true;}
                if ui.button("Move layer with rulers…").clicked(){move_layer=true;}
            });
            egui::ComboBox::from_id_salt("drawing-ruler-kind").selected_text(["Straight","Axial symmetry","Radial symmetry","1-point perspective","2-point perspective","3-point perspective"][e.kind]).show_ui(ui,|ui|{for(i,label)in["Straight","Axial symmetry","Radial symmetry","1-point perspective","2-point perspective","3-point perspective"].into_iter().enumerate(){ui.selectable_value(&mut e.kind,i,label);}});
            ui.text_edit_singleline(&mut e.name);ui.label("One x,y per line: straight/axis 2; radial/1-point 1; 2/3-point perspective 2/3.");ui.text_edit_multiline(&mut e.points);
            if e.kind==2{ui.add(egui::Slider::new(&mut e.arms,2..=16).text("Symmetry arms"));}
            ui.horizontal(|ui|{if ui.button(if e.edit.is_some(){"Replace draft ruler"}else{"Add draft ruler"}).clicked()&&let Err(err)=e.add(&self.documents[n]){self.status=Some(err);}
if ui.button("New ruler fields").clicked(){e.edit=None;}});
            let mut load=None;let mut remove=None;for g in &e.data.guides{ui.push_id(g.id,|ui|{ui.horizontal_wrapped(|ui|{if ui.selectable_label(e.data.active==Some(g.id),format!("{} · {}",g.name,g.kind.label())).clicked(){e.data.active=Some(g.id);}
if ui.button("Edit fields").clicked(){if matches!(g.kind,Kind::Special(_)){e.data.active=Some(g.id);advanced=true;}else{load=Some(g.id);}}
if ui.button("Remove").clicked(){remove=Some(g.id);}});});}
            if let Some(id)=load{e.load(id,&self.documents[n]);}
if let Some(id)=remove{e.data.guides.retain(|g|g.id!=id);if e.data.active==Some(id){e.data.active=None;}
if e.edit==Some(id){e.edit=None;}}
            if ui.button("No active ruler").clicked(){e.data.active=None;}
            ui.horizontal(|ui|{let button=ui.button("Apply rulers");self.ruler_tools.apply_button=Some(button.rect);apply=button.clicked();if ui.button("Cancel").clicked(){cancel=true;}});
        });
        if advanced {
            if let Err(err) = self.begin_special_rulers_with(e.id, e.revision, e.data.clone()) {
                self.status = Some(err);
            } else {
                open = false;
            }
        }
        if follow {
            if let Err(err) = self.begin_follow_ruler() {
                self.status = Some(err);
            } else {
                open = false;
            }
        }
        if move_layer {
            if let Err(err) = self.begin_ruler_layer_move() {
                self.status = Some(err);
            } else {
                open = false;
            }
        }
        open &= !cancel;
        self.ruler_tools.open = open;
        self.ruler_tools.editor = if open { Some(e) } else { None };
        if apply && let Err(err) = self.apply_rulers() {
            self.status = Some(err);
        }
    }
}

fn step(ctx: &egui::Context, app: &mut Drawer, time: &mut f64, events: Vec<egui::Event>) {
    *time += 0.1;
    super::input_replay::frame(ctx, app, *time, events);
}
fn key(pressed: bool, repeat: bool) -> egui::Event {
    egui::Event::Key {
        key: egui::Key::V,
        physical_key: None,
        pressed,
        repeat,
        modifiers: Default::default(),
    }
}
pub(super) fn smoke(root: &std::path::Path) -> Result<(), String> {
    use drawer_core::pointer_input::{Kind as InputKind, Phase, Source};
    core::verify(root)?;
    for pen in [false, true] {
        let ctx = egui::Context::default();
        let mut app = Drawer::new(&ctx, None);
        app.simple_view = true;
        let mut time = 0.0;
        app.create_canvas(CanvasSpec {
            width: 32,
            height: 32,
            dpi: 300.0,
            background: Background::Transparent,
        })
        .map_err(|e| e.to_string())?;
        app.set_tool(BrushTip::PixelPen);
        app.panels.settings.brush_size = 1.0;
        app.workspaces.store.current.pressure.tools[6].size = false;
        app.workspaces.store.current.pressure.tools[6].opacity = false;
        app.workspaces.store.current.input.profiles[6].minimum_size = 1.0;
        app.workspaces.store.current.input.profiles[6].minimum_opacity = 1.0;
        app.workspaces.store.current.input.profiles[6]
            .pixel
            .pressure = false;
        app.choose_color([200, 40, 20, 128]);
        app.documents[0].view.zoom = 1.5;
        app.documents[0].view.angle = 90.0;
        step(&ctx, &mut app, &mut time, vec![]);
        step(&ctx, &mut app, &mut time, vec![]);
        let before = app.documents[0].snapshot();
        app.execute_command(&ctx, "ruler-tools");
        {
            let e = app.ruler_tools.editor.as_mut().unwrap();
            e.kind = 1;
            e.points = "16,0\n16,32".into();
            e.name = "Mirror".into();
            e.add(&app.documents[0])?;
        }
        if app.documents[0].snapshot() != before {
            return Err("Ruler draft changed page".into());
        }
        step(&ctx, &mut app, &mut time, vec![]);
        step(&ctx, &mut app, &mut time, vec![]);
        let rect = app
            .ruler_tools
            .apply_button
            .ok_or("Ruler apply button missing")?;
        for pressed in [true, false] {
            step(
                &ctx,
                &mut app,
                &mut time,
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
        if app.ruler_tools.open
            || app.documents[0].scene.rulers.guides.len() != 1
            || app.documents[0].history_counts() != (1, 0)
        {
            return Err("Actual ruler Apply/one Undo".into());
        }
        app.documents[0].undo();
        if app.documents[0].snapshot() != before {
            return Err("Ruler editor Undo".into());
        }
        app.documents[0].redo();
        let path = root.join(format!("ruler-ui-{pen}.drawer"));
        drawer_core::files::save(&mut app.documents[0], &path).map_err(|e| e.to_string())?;
        let original = app.documents[0].snapshot();
        let base = app.documents[0].history_counts().0;
        let pointer = |app: &Drawer, p, phase| {
            let mut e = super::precision::pointer(&ctx, app, p, phase);
            if pen {
                e.source = Source {
                    kind: InputKind::Pen,
                    device: 101,
                    contact: 5,
                };
                e.pressure = Some(1.0);
            }
            e
        };
        for phase in [Phase::Down, Phase::Up] {
            app.drawing
                .virtual_events(vec![pointer(&app, [5.5, 8.5], phase)], pen);
            step(&ctx, &mut app, &mut time, vec![]);
        }
        if app.documents[0].layers[0].pixel(32, 32, 5, 8) != [200, 40, 20, 128]
            || app.documents[0].layers[0].pixel(32, 32, 26, 8) != [200, 40, 20, 128]
            || app.documents[0].history_counts() != (base + 1, 0)
            || app.drawing.pending()
        {
            return Err(format!(
                "Production mouse/pen symmetry {pen}: {:?}",
                app.status
            ));
        }
        app.documents[0].undo();
        if app.documents[0].snapshot() != original {
            return Err("Symmetry whole stroke Undo".into());
        }
        // V and a prepared pen Down in one frame must retain the new contact.
        app.drawing
            .virtual_events(vec![pointer(&app, [5.5, 8.5], Phase::Down)], pen);
        step(&ctx, &mut app, &mut time, vec![key(true, false)]);
        if !app.drawing.ruler_disabled || !app.drawing.pending() {
            return Err("Snap toggle lost prepared contact".into());
        }
        step(&ctx, &mut app, &mut time, vec![key(true, true)]);
        if !app.drawing.ruler_disabled {
            return Err("Repeated snap key toggled twice".into());
        }
        app.drawing
            .virtual_events(vec![pointer(&app, [5.5, 8.5], Phase::Up)], pen);
        step(&ctx, &mut app, &mut time, vec![key(false, false)]);
        if app.documents[0].layers[0].pixel(32, 32, 26, 8)[3] != 0
            || app.documents[0].scene.rulers != original.scene.rulers
        {
            return Err("Temporary snap toggle changed page rulers".into());
        }
        app.documents[0].undo();
        let history = app.documents[0].history_counts();
        app.execute_command(&ctx, "ruler-snap");
        if app.documents[0].history_counts() != history || app.documents[0].snapshot() != original {
            return Err("Snap toggle entered document history".into());
        }
        app.execute_command(&ctx, "ruler-tools");
        {
            let e = app.ruler_tools.editor.as_mut().unwrap();
            e.kind = 0;
            e.points = "0,10.5\n32,10.5".into();
            e.name = "Line".into();
            e.add(&app.documents[0])?;
        }
        app.apply_rulers()?;
        for (p, phase) in [
            ([2.5, 2.5], Phase::Down),
            ([25.5, 20.5], Phase::Move),
            ([25.5, 20.5], Phase::Up),
        ] {
            app.drawing
                .virtual_events(vec![pointer(&app, p, phase)], pen);
            step(&ctx, &mut app, &mut time, vec![]);
        }
        if app.documents[0].layers[0]
            .pixels
            .iter()
            .enumerate()
            .any(|(n, p)| p[3] > 0 && n / 32 != 10)
        {
            return Err("Rotated view straight ruler misaligned".into());
        }
        drawer_core::files::save(&mut app.documents[0], &path).map_err(|e| e.to_string())?;
        if drawer_core::files::open(&path, DEFAULT_DOCUMENT_BUDGET)
            .map_err(|e| e.to_string())?
            .snapshot()
            != app.documents[0].snapshot()
        {
            return Err("Ruler UI native reopen".into());
        }
        app.execute_command(&ctx, "ruler-tools");
        let draft = app.documents[0].snapshot();
        app.ruler_tools.editor.as_mut().unwrap().data.guides.clear();
        app.ruler_tools.open = false;
        app.ruler_tools.editor = None;
        step(&ctx, &mut app, &mut time, vec![]);
        if app.documents[0].snapshot() != draft {
            return Err("Cancelled ruler draft changed document".into());
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn ruler_editor_and_production_mouse_pen_snap_undo_reopen() {
        let root = crate::tests::TestDirectory::new();
        super::smoke(&root.0).unwrap();
    }
}
