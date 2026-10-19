use super::*;
use drawer_core::{
    extended_fill::{Boundary, Settings, Target},
    scene::Rect,
};
#[derive(Default)]
pub(super) struct Panel {
    draft: Option<Box<Draft>>,
    apply: Option<egui::Rect>,
}
struct Draft {
    id: u64,
    revision: u64,
    frame: u32,
    bounds: Rect,
    frames: u32,
    create: bool,
}
pub(crate) fn controls(ui: &mut egui::Ui, s: &mut Settings) {
    ui.collapsing("Fill target, boundary and underpaint",|ui|{
        egui::ComboBox::from_id_salt("fill-target-color").selected_text(format!("{:?}",s.target)).show_ui(ui,|ui|{for target in Target::ALL{ui.selectable_value(&mut s.target,target,format!("{target:?}"));}});
        ui.checkbox(&mut s.semi_transparent,"Treat semi-transparent pixels as transparent");ui.checkbox(&mut s.connected,"Connected to clicked point only");
        egui::ComboBox::from_id_salt("fill-domain-boundary").selected_text(format!("{:?}",s.boundary)).show_ui(ui,|ui|{ui.selectable_value(&mut s.boundary,Boundary::Canvas,"Canvas boundary");ui.selectable_value(&mut s.boundary,Boundary::AnimationFrame,"Active animation output frame");});
        ui.checkbox(&mut s.vector_centerline,"Reference vector centerline instead of displayed stroke");ui.label("Vector references use the composite, marked, selected or folder scope.");
        let mut exclude=s.excluded_color.is_some();if ui.checkbox(&mut exclude,"Exclude RGB from writing").changed(){s.excluded_color=exclude.then_some([0,0,0,255]);}
        if let Some(color)=&mut s.excluded_color{super::edit::rgba(ui,color);ui.add(egui::Slider::new(&mut s.excluded_tolerance,0..=255).text("Excluded RGB tolerance"));}
        ui.checkbox(&mut s.include_outlines,"Include outlines / underpaint boundary");ui.add_enabled_ui(s.include_outlines,|ui|{ui.add(egui::Slider::new(&mut s.outline_distance,1..=32).text("Underpaint pixels"));});
        ui.label("Surrounded/enclosed modes omit components touching the active boundary. Excluded colors block writing only. Expansion and underpaint remain inside the selection and output frame.");
    });
}
impl Drawer {
    pub(super) fn begin_animation_output(&mut self) -> Result<(), String> {
        let d = &self.documents[self.active.ok_or("Open a page first")?];
        let bounds = d.scene.animation_output.at(d.current_frame).map_or(
            Rect {
                min: [0.0, 0.0],
                max: [d.width as f64, d.height as f64],
            },
            |f| {
                let points = f.corners(d.scene.transform);
                Rect {
                    min: [
                        points.iter().map(|p| p[0]).fold(f64::INFINITY, f64::min),
                        points.iter().map(|p| p[1]).fold(f64::INFINITY, f64::min),
                    ],
                    max: [
                        points
                            .iter()
                            .map(|p| p[0])
                            .fold(f64::NEG_INFINITY, f64::max),
                        points
                            .iter()
                            .map(|p| p[1])
                            .fold(f64::NEG_INFINITY, f64::max),
                    ],
                }
            },
        );
        self.extended_fill.draft = Some(Box::new(Draft {
            id: d.id,
            revision: d.revision,
            frame: d.current_frame,
            bounds,
            frames: d.animation.as_ref().map_or(24, |a| a.frames),
            create: d.animation.is_none(),
        }));
        Ok(())
    }
    fn apply_animation_output(&mut self) -> Result<(), String> {
        let e = self
            .extended_fill
            .draft
            .as_ref()
            .ok_or("Output frame draft is closed")?;
        let d = &mut self.documents[self.active.ok_or("Open a page first")?];
        if d.id != e.id || d.revision != e.revision || d.current_frame != e.frame {
            return Err("Page/frame changed; reopen output frame".into());
        }
        d.commit_state(drawer_core::animation_output::prepare_state(
            d, e.bounds, e.frames,
        )?)?;
        self.extended_fill.draft = None;
        Ok(())
    }
    pub(super) fn animation_output_window(&mut self, ctx: &egui::Context) {
        let Some(e) = self.extended_fill.draft.as_mut() else {
            return;
        };
        let mut open = !ctx.input(|i| i.key_pressed(egui::Key::Escape));
        let mut apply = false;
        let mut cancel = false;
        self.extended_fill.apply = None;
        egui::Window::new("Animation output frame").open(&mut open).collapsible(false).default_width(420.0).min_width(420.0).resizable(false).show(ctx,|ui|{
            ui.label(format!("Frame {}. Coordinates are document pixels. This frame holds until the next key.",e.frame+1));
            for i in 0..2{ui.horizontal(|ui|{ui.label(if i==0{"Left / Right"}else{"Top / Bottom"});ui.add(egui::DragValue::new(&mut e.bounds.min[i]).range(-1e9..=1e9));ui.add(egui::DragValue::new(&mut e.bounds.max[i]).range(-1e9..=1e9));});}
            if e.create{ui.label("This will create a local 24 fps animation with its output frame.");ui.add(egui::Slider::new(&mut e.frames,1..=drawer_core::animation::MAX_FRAMES).text("Timeline frames"));}
            ui.label("Apply saves one Undo entry. The blue frame follows page geometry and is used when fill boundary is set to Active animation output frame.");
            ui.horizontal(|ui|{let r=ui.button("Apply output frame");self.extended_fill.apply=Some(r.rect);apply=r.clicked();cancel=ui.button("Cancel").clicked();});
        });
        if !open || cancel {
            self.extended_fill.draft = None;
        } else if apply && let Err(err) = self.apply_animation_output() {
            self.status = Some(err);
        }
    }
}
pub(crate) fn paint_frame(
    painter: &egui::Painter,
    doc: &Document,
    view: drawer_core::view::View,
    rect: egui::Rect,
    ppp: f64,
) {
    if doc.animation.is_none() {
        return;
    }
    let Some(frame) = doc.scene.animation_output.at(doc.current_frame) else {
        return;
    };
    let size = [doc.width as f64, doc.height as f64];
    let center = [rect.center().x as f64, rect.center().y as f64];
    let points = frame.corners(doc.scene.transform).map(|p| {
        let q = view.document_to_screen(p, size, center, ppp);
        egui::pos2(q[0] as f32, q[1] as f32)
    });
    for i in 0..4 {
        painter.line_segment(
            [points[i], points[(i + 1) % 4]],
            egui::Stroke::new(1.5, egui::Color32::from_rgb(50, 130, 255)),
        );
    }
}

pub(crate) fn smoke(root: &std::path::Path) -> Result<(), String> {
    drawer_core::extended_fill_verify::verify(root)?;
    let ctx = egui::Context::default();
    let mut app = Drawer::new(&ctx, None);
    app.simple_view = true;
    app.create_canvas(CanvasSpec {
        width: 7,
        height: 7,
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
    app.execute_command(&ctx, "animation-output-frame");
    let before = app.documents[0].snapshot();
    let draft = app
        .extended_fill
        .draft
        .as_mut()
        .ok_or("Output frame command missing")?;
    draft.bounds = Rect {
        min: [1.0, 1.0],
        max: [6.0, 6.0],
    };
    draft.frames = 4;
    step(&mut app, &mut time, vec![]);
    step(&mut app, &mut time, vec![]);
    if app.documents[0].snapshot() != before {
        return Err("Output frame draft mutated page".into());
    }
    let r = app
        .extended_fill
        .apply
        .ok_or("Output frame Apply missing")?;
    for pressed in [true, false] {
        step(
            &mut app,
            &mut time,
            vec![
                egui::Event::PointerMoved(r.center()),
                egui::Event::PointerButton {
                    pos: r.center(),
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: Default::default(),
                },
            ],
        );
    }
    step(&mut app, &mut time, vec![]);
    if app.extended_fill.draft.is_some()
        || app.documents[0]
            .animation
            .as_ref()
            .is_none_or(|a| a.frames != 4)
        || app.documents[0].scene.animation_output.keys.len() != 1
        || app.documents[0].history_counts().0 != 1
    {
        return Err(format!("Output frame production Apply: {:?}", app.status));
    }
    let configured = app.documents[0].snapshot();
    if !app.documents[0].undo()
        || app.documents[0].snapshot() != before
        || !app.documents[0].redo()
        || app.documents[0].snapshot() != configured
    {
        return Err("Output frame creation one Undo/Redo".into());
    }
    let mut source = app.documents[0].layers[0].clone();
    for x in 1..7 {
        source.pixels[3 * 7 + x] = [0, 0, 0, 255];
    }
    app.documents[0].layers.push(source);
    app.documents[0].view.zoom = 4.0;
    app.documents[0].view.angle = 90.0;
    app.choose_color([200, 40, 20, 255]);
    app.execute_command(&ctx, "bucket-tool");
    let slot = BrushTip::ALL
        .iter()
        .position(|t| *t == app.selected)
        .unwrap();
    app.workspaces.store.current.input.fills[slot] = drawer_core::bucket::Settings {
        reference: drawer_core::bucket::Reference::Layer(1),
        antialias: false,
        advanced: Settings {
            boundary: Boundary::AnimationFrame,
            ..Default::default()
        },
        ..Default::default()
    };
    step(&mut app, &mut time, vec![]);
    step(&mut app, &mut time, vec![]);
    let before = app.documents[0].snapshot();
    let history = app.documents[0].history_counts();
    for phase in [
        drawer_core::pointer_input::Phase::Down,
        drawer_core::pointer_input::Phase::Up,
    ] {
        let e = super::precision::pointer(&ctx, &app, [2.5, 2.5], phase);
        app.drawing.virtual_events(vec![e], false);
        step(&mut app, &mut time, vec![]);
        if phase == drawer_core::pointer_input::Phase::Down
            && (!app.drawing.pending() || app.documents[0].snapshot() != before)
        {
            return Err("Output frame fill preview changed page".into());
        }
    }
    let d = &app.documents[0];
    if d.layers[0].pixels.iter().filter(|p| p[3] > 0).count() != 10
        || d.layers[0].pixels[2 * 7 + 2] != [200, 40, 20, 255]
        || d.layers[0].pixels[5 * 7 + 2] != [0; 4]
        || d.scene != before.scene
        || d.layers[1] != before.layers[1]
        || d.history_counts().0 != history.0 + 1
    {
        return Err(format!(
            "Output frame production fill domain: {:?}",
            app.status
        ));
    }
    if !app.documents[0].undo() || app.documents[0].snapshot() != before {
        return Err("Framed fill one Undo".into());
    }
    app.execute_command(&ctx, "animation-output-frame");
    app.documents[0].edit_pixel(0, 0, 0, [1, 2, 3, 255]);
    let before = app.documents[0].snapshot();
    if app.apply_animation_output().is_ok() || app.documents[0].snapshot() != before {
        return Err("Stale output frame changed page".into());
    }
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
    if app.extended_fill.draft.is_some() {
        return Err("Output frame Escape did not cancel".into());
    }
    for pen in [false, true] {
        for centerline in [false, true] {
            let ctx = egui::Context::default();
            let mut app = Drawer::new(&ctx, None);
            app.simple_view = true;
            let mut time = 0.0;
            app.create_canvas(CanvasSpec {
                width: 11,
                height: 11,
                dpi: 300.0,
                background: Background::Transparent,
            })
            .map_err(|e| e.to_string())?;
            let layer = app.documents[0].scene.lines.add_layer("Thick reference")?;
            app.documents[0].scene.lines.add_path(
                layer,
                vec![[2.5, 2.5], [8.5, 2.5], [8.5, 8.5], [2.5, 8.5]],
                true,
                4.0,
                [0, 0, 0, 255],
            )?;
            app.documents[0].scene.selection_mask = Some(drawer_core::selection::Mask {
                runs: vec![drawer_core::selection::Run {
                    x: 0,
                    y: 5,
                    length: 11,
                    alpha: 128,
                }],
            });
            app.documents[0].view.zoom = 4.0;
            app.documents[0].view.angle = 90.0;
            app.execute_command(&ctx, "bucket-tool");
            app.choose_color([200, 40, 20, 255]);
            let slot = BrushTip::ALL
                .iter()
                .position(|t| *t == app.selected)
                .unwrap();
            app.workspaces.store.current.input.fills[slot] = drawer_core::bucket::Settings {
                reference: drawer_core::bucket::Reference::Composite,
                antialias: false,
                advanced: Settings {
                    vector_centerline: centerline,
                    ..Default::default()
                },
                ..Default::default()
            };
            let step = |app: &mut Drawer, time: &mut f64| {
                *time += 0.1;
                super::input_replay::frame(&ctx, app, *time, vec![]);
            };
            step(&mut app, &mut time);
            step(&mut app, &mut time);
            let before = app.documents[0].snapshot();
            let history = app.documents[0].history_counts();
            for phase in [
                drawer_core::pointer_input::Phase::Down,
                drawer_core::pointer_input::Phase::Up,
            ] {
                let mut e = super::precision::pointer(&ctx, &app, [5.5, 5.5], phase);
                if pen {
                    e.source = drawer_core::pointer_input::Source {
                        kind: drawer_core::pointer_input::Kind::Pen,
                        device: 112,
                        contact: 1,
                    };
                    e.pressure = Some(0.5);
                }
                app.drawing.virtual_events(vec![e], pen);
                step(&mut app, &mut time);
                if phase == drawer_core::pointer_input::Phase::Down
                    && app.documents[0].snapshot() != before
                {
                    return Err("Centerline preview changed source".into());
                }
            }
            let d = &app.documents[0];
            if d.layers[0].pixels[5 * 11 + 4][3] != if centerline { 128 } else { 0 }
                || d.layers[0].pixels[5 * 11 + 5] != [200, 40, 20, 128]
                || d.layers[0].pixels[4 * 11 + 5] != [0; 4]
                || d.scene != before.scene
                || d.history_counts().0 != history.0 + 1
            {
                return Err(format!(
                    "Centerline/selection production fill {pen}/{centerline}: {:?}",
                    app.status
                ));
            }
            if !app.documents[0].undo() || app.documents[0].snapshot() != before {
                return Err("Centerline fill one Undo".into());
            }
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn output_frame_buttons_creation_and_fill_centerline_mouse_pen() {
        let root = crate::tests::TestDirectory::new();
        super::smoke(&root.0).unwrap();
    }
}
