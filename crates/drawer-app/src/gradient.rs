use super::*;
use drawer_core::{
    bucket,
    gradient::{self as core, Cover, Repeat, Settings, Shape, VectorFill},
};
#[derive(Default)]
pub(super) struct Panel {
    pub draft: Option<Box<Draft>>,
    apply: Option<egui::Rect>,
}
pub(super) struct Draft {
    id: u64,
    revision: u64,
    frame: u32,
    layer: usize,
    editing: Option<u64>,
    settings: Settings,
    editable: bool,
    reference: bucket::Settings,
}
impl Drawer {
    pub(super) fn begin_gradient(&mut self) -> Result<(), String> {
        let d = &self.documents[self.active.ok_or("Open a page first")?];
        let drawer_core::thumbnail::Target::Raster(layer) = self
            .panels
            .targets
            .get(&d.id)
            .copied()
            .unwrap_or(drawer_core::thumbnail::Target::Raster(0))
        else {
            return Err("Choose a raster target for gradient painting".into());
        };
        let color = self.references.session.workspace.color;
        self.gradient.draft = Some(Box::new(Draft {
            id: d.id,
            revision: d.revision,
            frame: d.current_frame,
            layer,
            editing: None,
            settings: Settings {
                start: [0.5, d.height as f64 / 2.0],
                end: [(d.width as f64 - 0.5).max(0.5), d.height as f64 / 2.0],
                first: color,
                last: [color[0], color[1], color[2], 0],
                drawing: color,
                ..Default::default()
            },
            editable: true,
            reference: bucket::Settings {
                reference: bucket::Reference::Composite,
                ..Default::default()
            },
        }));
        Ok(())
    }
    fn apply_gradient(&mut self) -> Result<(), String> {
        let e = self
            .gradient
            .draft
            .as_ref()
            .ok_or("Gradient editor is closed")?;
        let d = &mut self.documents[self.active.ok_or("Open a page first")?];
        if (d.id, d.revision, d.current_frame) != (e.id, e.revision, e.frame) {
            return Err("Page/frame changed; reopen the gradient editor".into());
        }
        let state = if let Some(id) = e.editing {
            core::edit(d, id, e.settings.clone())?
        } else {
            core::prepare(d, e.layer, e.settings.clone(), e.editable, e.reference)?
        };
        d.commit_state(state)?;
        self.gradient.draft = None;
        Ok(())
    }
    pub(super) fn gradient_window(&mut self, ctx: &egui::Context) {
        let Some(e) = self.gradient.draft.as_mut() else {
            return;
        };
        let mut open = !ctx.input(|i| i.key_pressed(egui::Key::Escape));
        let mut apply = false;
        let mut cancel = false;
        let mut edit_id = None;
        let mut remove_id = None;
        let mut toggle_id = None;
        let mut move_id = None;
        self.gradient.apply = None;
        let layers = self
            .active
            .map(|n| self.documents[n].scene.gradients.layers.clone())
            .unwrap_or_default();
        egui::Window::new("Gradient and freeform gradient").open(&mut open).collapsible(false).default_width(460.0).show(ctx,|ui|{egui::ScrollArea::vertical().max_height(520.0).show(ui,|ui|{
            ui.horizontal(|ui|{ui.label("Shape");egui::ComboBox::from_id_salt("gradient-shape").selected_text(shape_label(e.settings.shape)).show_ui(ui,|ui|{for s in Shape::ALL{ui.selectable_value(&mut e.settings.shape,s,shape_label(s));}});});
            ui.horizontal(|ui|{ui.label("Outside range");egui::ComboBox::from_id_salt("gradient-repeat").selected_text(repeat_label(e.settings.repeat)).show_ui(ui,|ui|{for r in Repeat::ALL{ui.selectable_value(&mut e.settings.repeat,r,repeat_label(r));}});});
            for (label,p) in [("Start",&mut e.settings.start),("End",&mut e.settings.end)]{ui.horizontal(|ui|{ui.label(label);for v in p{ui.add(egui::DragValue::new(v).range(-1e9..=1e9).speed(0.5));}});}
            ui.horizontal(|ui|{ui.label("Start color");super::edit::rgba(ui,&mut e.settings.first);ui.label("End color");super::edit::rgba(ui,&mut e.settings.last);});
            if ui.button("Swap start/end colors").clicked(){std::mem::swap(&mut e.settings.first,&mut e.settings.last);}
            ui.checkbox(&mut e.settings.dithering,"Dithering");
            if e.settings.shape==Shape::Ellipse{ui.add(egui::Slider::new(&mut e.settings.ellipse_ratio,0.01..=100.0).logarithmic(true).text("Ellipse minor/major ratio"));}
            if e.settings.shape==Shape::Freeform {
                for (label,policy,names) in [("Black",&mut e.settings.black,["Do not cover black","Cover black","Replace black with drawing color"]),("Transparency",&mut e.settings.transparency,["Do not cover transparency","Cover transparency","Replace transparency with drawing color"])]{egui::ComboBox::from_id_salt(label).selected_text(names[Cover::ALL.iter().position(|c|c==policy).unwrap()]).show_ui(ui,|ui|{for (c,label) in Cover::ALL.into_iter().zip(names){ui.selectable_value(policy,c,label);}});}
                ui.horizontal(|ui|{ui.label("Drawing color");super::edit::rgba(ui,&mut e.settings.drawing);});
                let mut exclude=e.settings.exclude_color.is_some();if ui.checkbox(&mut exclude,"Do not fill this color").changed(){e.settings.exclude_color=exclude.then_some(e.settings.drawing);}
                if let Some(c)=&mut e.settings.exclude_color{super::edit::rgba(ui,c);}
                let mut adjacent=e.settings.vectors!=VectorFill::Keep;if ui.checkbox(&mut adjacent,"Fill adjacent vector lines").changed(){e.settings.vectors=if adjacent{VectorFill::UpToIntersection}else{VectorFill::Keep};}
                if adjacent{ui.radio_value(&mut e.settings.vectors,VectorFill::UpToIntersection,"Erase up to intersection");ui.radio_value(&mut e.settings.vectors,VectorFill::WholeLine,"Whole line");}
                ui.horizontal(|ui|{ui.radio_value(&mut e.reference.reference,bucket::Reference::Active,"Editing raster reference");ui.radio_value(&mut e.reference.reference,bucket::Reference::Composite,"Visible composite reference");});
                ui.label("Each closed white/transparent region gets its own two-anchor field. Black ink is a boundary; colored islands and outside regions are preserved. Partial black edges retain their coverage. Vector replacement needs an unrestricted selection. Editing keeps the captured regions; topology changes require a new gradient.");
            }
            if let Some(id)=e.editing{ui.label(format!("Editing gradient {id}"));}else{ui.radio_value(&mut e.editable,false,"Draw on editing layer");ui.radio_value(&mut e.editable,true,"Create gradient layer");}
            ui.horizontal(|ui|{let r=ui.button("Apply gradient");self.gradient.apply=Some(r.rect);apply=r.clicked();cancel=ui.button("Cancel").clicked();});
            ui.separator();ui.label("Editable gradient layers (above raster layers, below page objects and vectors)");
            for (index,l) in layers.iter().enumerate(){ui.horizontal(|ui|{ui.label(&l.name);if ui.button("Edit").clicked(){edit_id=Some(l.id)}
if ui.button(if l.hidden{"Show"}else{"Hide"}).clicked(){toggle_id=Some(l.id)}
if ui.button("Delete").clicked(){remove_id=Some(l.id)}
if index>0&&ui.button("Down").clicked(){move_id=Some((index,index-1))}
if index+1<layers.len()&&ui.button("Up").clicked(){move_id=Some((index,index+1))}});}
        });});
        if !open || cancel {
            self.gradient.draft = None;
            return;
        }
        if let Some(n) = self.active {
            let d = &mut self.documents[n];
            if (d.id, d.revision, d.current_frame) != (e.id, e.revision, e.frame) {
                return;
            }
            if let Some(id) = edit_id
                && let Some(l) = layers.iter().find(|l| l.id == id)
            {
                e.editing = Some(id);
                e.settings = l.settings.clone();
            }
            if remove_id.is_some() || toggle_id.is_some() || move_id.is_some() {
                let mut state = d.snapshot();
                state
                    .scene
                    .gradients
                    .layers
                    .retain(|l| Some(l.id) != remove_id);
                if let Some(l) = state
                    .scene
                    .gradients
                    .layers
                    .iter_mut()
                    .find(|l| Some(l.id) == toggle_id)
                {
                    l.hidden = !l.hidden;
                }
                if let Some((a, b)) = move_id {
                    state.scene.gradients.layers.swap(a, b);
                }
                match d.commit_state(state) {
                    Ok(_) => {
                        e.revision = d.revision;
                        if e.editing == remove_id {
                            e.editing = None;
                        }
                    }
                    Err(error) => self.status = Some(error),
                }
            }
        }
        if apply && let Err(error) = self.apply_gradient() {
            self.status = Some(error);
        }
    }
}
pub(super) fn smoke(root: &std::path::Path) -> Result<(), String> {
    drawer_core::gradient_verify::verify(root)?;
    let ctx = egui::Context::default();
    let mut app = Drawer::new(&ctx, None);
    app.simple_view = true;
    app.create_canvas(CanvasSpec {
        width: 16,
        height: 12,
        dpi: 300.0,
        background: Background::Transparent,
    })
    .map_err(|e| e.to_string())?;
    let mut time = 0.0;
    let step = |app: &mut Drawer, time: &mut f64, events| {
        *time += 0.1;
        super::input_replay::frame(&ctx, app, *time, events);
    };
    step(&mut app, &mut time, vec![]);
    step(&mut app, &mut time, vec![]);
    for editable in [true, false] {
        let before = app.documents[0].snapshot();
        let history = app.documents[0].history_counts();
        app.execute_command(&ctx, "gradient-tool");
        let e = app
            .gradient
            .draft
            .as_mut()
            .ok_or("Gradient command missing")?;
        e.editable = editable;
        e.settings.last = [20, 80, 220, 255];
        let expected = core::prepare(
            &app.documents[0],
            e.layer,
            e.settings.clone(),
            e.editable,
            e.reference,
        )?;
        step(&mut app, &mut time, vec![]);
        step(&mut app, &mut time, vec![]);
        if app.documents[0].snapshot() != before {
            return Err("Gradient draft changed page".into());
        }
        let rect = app.gradient.apply.ok_or("Gradient Apply missing")?;
        for pressed in [true, false] {
            step(
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
        if app.gradient.draft.is_some()
            || app.documents[0].snapshot() != expected
            || app.documents[0].history_counts().0 != history.0 + 1
        {
            return Err(format!(
                "Gradient production Apply failed: {:?}",
                app.status
            ));
        }
        if !app.documents[0].undo() || app.documents[0].snapshot() != before {
            return Err("Gradient production Undo".into());
        }
    }
    app.execute_command(&ctx, "gradient-tool");
    let before = app.documents[0].snapshot();
    step(
        &mut app,
        &mut time,
        vec![egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Default::default(),
        }],
    );
    if app.gradient.draft.is_some() || app.documents[0].snapshot() != before {
        return Err("Gradient Escape changed page".into());
    }
    app.execute_command(&ctx, "gradient-tool");
    app.documents[0].edit_pixel(0, 0, 0, [1, 2, 3, 255]);
    let before = app.documents[0].snapshot();
    if app.apply_gradient().is_ok() || app.documents[0].snapshot() != before {
        return Err("Stale gradient applied".into());
    }
    Ok(())
}
fn shape_label(s: Shape) -> &'static str {
    match s {
        Shape::StraightLine => "Straight line",
        Shape::Circle => "Circle",
        Shape::Ellipse => "Ellipse",
        Shape::Cone => "Cone",
        Shape::Freeform => "Freeform",
    }
}
fn repeat_label(r: Repeat) -> &'static str {
    match r {
        Repeat::DoNotRepeat => "Do not repeat",
        Repeat::Repeat => "Repeat",
        Repeat::Reverse => "Reverse",
        Repeat::NotDraw => "Not draw",
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn native_gradient_apply_cancel_stale() {
        let root = crate::tests::TestDirectory::new();
        super::smoke(&root.0).unwrap();
    }
}
