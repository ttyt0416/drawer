use super::*;
use drawer_core::layer_properties::{self as core, Mark, Patch, Properties, Target};
#[derive(Default)]
pub(super) struct Panel {
    draft: Option<Box<Draft>>,
    apply: Option<egui::Rect>,
    modes: [Option<egui::Rect>; 4],
}
struct Draft {
    id: u64,
    revision: u64,
    frame: u32,
    selected: Vec<Target>,
    values: Properties,
    patch: Patch,
    alpha: bool,
    clip: bool,
    alpha_patch: Option<bool>,
    clip_patch: Option<bool>,
}
pub(super) fn label(d: &Document, t: Target) -> String {
    match t {
        Target::Raster(id) => drawer_core::layers::index(d, id)
            .map_or("Missing raster".into(), |n| {
                format!("Raster {id} · {}", d.layers[n].name)
            }),
        Target::Vector(id) => d
            .scene
            .lines
            .layers
            .iter()
            .find(|l| l.id == id)
            .map_or("Missing vector".into(), |l| {
                format!("Vector {id} · {}", l.name)
            }),
        Target::Folder(n) => d
            .scene
            .paper
            .folders
            .get(n as usize)
            .map_or("Missing folder".into(), |f| {
                format!("Folder {} · {}", n + 1, f.name)
            }),
        Target::Track(n) => d
            .animation
            .as_ref()
            .and_then(|a| a.tracks.get(n as usize))
            .map_or("Missing track".into(), |t| {
                format!("Track {} · {}", n + 1, t.name)
            }),
        Target::Cell { track, cell } => d
            .animation
            .as_ref()
            .and_then(|a| a.tracks.get(track as usize))
            .and_then(|t| t.cells.get(cell as usize))
            .map_or("Missing cell".into(), |c| {
                format!("Track {} / cell {} · {}", track + 1, cell + 1, c.name)
            }),
    }
}
pub(super) fn badge(ui: &mut egui::Ui, mark: Mark) {
    match mark {
        Mark::None => {}
        Mark::Color(c) => {
            let (r, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
            ui.painter()
                .rect_filled(r, 2.0, egui::Color32::from_rgb(c[0], c[1], c[2]));
        }
        Mark::Mixed => {
            ui.small("Mixed")
                .on_hover_text("Different child identification colors");
        }
    }
}
pub(super) fn row_mark(
    ui: &mut egui::Ui,
    d: &Document,
    t: drawer_core::thumbnail::Target,
    preferences: core::Display,
    timeline: bool,
) {
    if (timeline && !preferences.timeline) || (!timeline && !preferences.list) {
        return;
    }
    let target = core::from_thumbnail(d, t);
    let mark = if matches!(target, Target::Folder(_)) && !preferences.parents {
        core::properties(d, target)
            .color
            .map_or(Mark::None, Mark::Color)
    } else {
        core::mark(d, target)
    };
    badge(ui, mark);
    if timeline
        && let drawer_core::thumbnail::Target::Track(n) = t
        && let Some(track) = d.animation.as_ref().and_then(|a| a.tracks.get(n))
    {
        let end = track
            .exposures
            .partition_point(|e| e.frame <= d.current_frame);
        let start = end.saturating_sub(4);
        let end = (start + 8).min(track.exposures.len());
        for e in &track.exposures[start..end] {
            let p = core::properties(
                d,
                Target::Cell {
                    track: n as u16,
                    cell: e.cell as u16,
                },
            );
            if let Some(c) = p.color {
                let (r, response) =
                    ui.allocate_exact_size(egui::vec2(6.0, 10.0), egui::Sense::hover());
                ui.painter()
                    .rect_filled(r, 1.0, egui::Color32::from_rgb(c[0], c[1], c[2]));
                response.on_hover_text(format!(
                    "Frame {} · {}",
                    e.frame + 1,
                    track.cells[e.cell].name
                ));
            }
        }
    }
}
impl Drawer {
    pub(super) fn begin_layer_properties(&mut self) -> Result<(), String> {
        let d = &self.documents[self.active.ok_or("Open a page first")?];
        let t = self
            .panels
            .targets
            .get(&d.id)
            .copied()
            .unwrap_or(drawer_core::thumbnail::Target::Raster(0));
        let target = core::from_thumbnail(d, t);
        self.layer_properties.draft = Some(Box::new(Draft {
            id: d.id,
            revision: d.revision,
            frame: d.current_frame,
            selected: vec![target],
            values: core::properties(d, target),
            patch: Default::default(),
            alpha: d.scene.clipping.alpha.contains(&target),
            clip: d.scene.clipping.binding(target).is_some(),
            alpha_patch: None,
            clip_patch: None,
        }));
        Ok(())
    }
    fn apply_layer_properties(&mut self) -> Result<(), String> {
        let e = self
            .layer_properties
            .draft
            .as_ref()
            .ok_or("Layer properties are closed")?;
        let n = self.active.ok_or("Open a page first")?;
        let d = &self.documents[n];
        if (d.id, d.revision, d.current_frame) != (e.id, e.revision, e.frame) {
            return Err("Page/frame changed; reopen layer properties".into());
        }
        let mut state = core::prepare_patch(d, &e.selected, e.patch)?;
        let clip = drawer_core::clipping::prepare(d, &e.selected, e.alpha_patch, e.clip_patch)?;
        state.scene.clipping = clip.scene.clipping;
        if state.scene.layer_ids.rasters.is_empty() {
            state.scene.layer_ids = clip.scene.layer_ids;
        }
        if state.scene.clipping.graph_active() {
            state.scene.paper.roots = state.scene.paper.root_nodes(state.layers.len());
        }
        let used: usize = self.documents.iter().map(Document::resident_bytes).sum();
        if state.memory_bytes().saturating_mul(2).saturating_add(used) > DEFAULT_DOCUMENT_BUDGET {
            return Err("Layer properties exceed workspace memory".into());
        }
        self.documents[n].commit_state(state)?;
        self.layer_properties.draft = None;
        Ok(())
    }
    pub(super) fn layer_property_window(&mut self, ctx: &egui::Context) {
        let Some(mut e) = self.layer_properties.draft.take() else {
            return;
        };
        let Some(n) = self.active.filter(|n| self.documents[*n].id == e.id) else {
            return;
        };
        let d = &self.documents[n];
        let targets = core::targets(d);
        let fresh = d.revision == e.revision && d.current_frame == e.frame;
        let mut open = !ctx.input(|i| i.key_pressed(egui::Key::Escape));
        let mut apply = false;
        let mut cancel = false;
        let mut view_changed = false;
        let mut solo = None;
        egui::Window::new("Layer properties and identification colors").open(&mut open).collapsible(false).resizable(false).default_width(530.0).min_width(530.0).default_pos(egui::pos2(60.0,25.0)).show(ctx,|ui|{
            if !fresh{ui.label("Page/frame changed; cancel and reopen this draft.");cancel=ui.button("Cancel").clicked();return;}
            ui.horizontal(|ui|{if ui.button("Select all layers / cells").clicked(){e.selected=targets.clone();}
if ui.button("Clear targets").clicked(){e.selected.clear();}ui.label(format!("{} selected",e.selected.len()));});
            egui::ScrollArea::vertical().id_salt("property-layer-targets").max_height(150.0).show_rows(ui,22.0,targets.len(),|ui,rows|{for n in rows{let t=targets[n];let mut selected=e.selected.contains(&t);if ui.checkbox(&mut selected,label(d,t)).changed(){if selected{e.selected.push(t);}else{e.selected.retain(|v|*v!=t);}}}});
            ui.label("Only changed fields apply to the selected targets. Unchanged colors and locks stay per layer.");
            ui.add_enabled_ui(e.selected.iter().any(|t|!matches!(t,Target::Cell{..})),|ui|{ui.horizontal(|ui|{ui.label("Layer blend");for (i,mode) in drawer_core::layer_blend::Mode::ALL.iter().enumerate(){let r=ui.selectable_value(&mut e.values.blend,*mode,format!("{mode:?}"));self.layer_properties.modes[i]=Some(r.rect);if r.changed(){e.patch.blend=Some(*mode);}}});});
            if ui.checkbox(&mut e.values.hidden,"Hidden").changed(){e.patch.hidden=Some(e.values.hidden);}
            if ui.add(egui::Slider::new(&mut e.values.opacity,0..=100).text("Opacity %")).changed(){e.patch.opacity=Some(e.values.opacity);}
            if ui.checkbox(&mut e.values.locked,"Lock all layer content / position").changed(){e.patch.locked=Some(e.values.locked);}
            ui.add_enabled_ui(e.selected.iter().any(|t|!matches!(t,Target::Vector(_))),|ui|{if ui.checkbox(&mut e.values.pixels_locked,"Lock image pixels (raster / cells)").changed(){e.patch.pixels_locked=Some(e.values.pixels_locked);}});
            if ui.checkbox(&mut e.values.position_locked,"Lock position").changed(){e.patch.position_locked=Some(e.values.position_locked);}
            let mut tagged=e.values.color.is_some();if ui.checkbox(&mut tagged,"Identification color").changed(){e.values.color=if tagged{Some([80,150,220])}else{None};e.patch.color=Some(e.values.color);}
            if let Some(c)=e.values.color.as_mut(){ui.horizontal(|ui|{for(k,name)in ["R","G","B"].iter().enumerate(){if ui.add(egui::Slider::new(&mut c[k],0..=255).text(*name)).changed(){e.patch.color=Some(Some(*c));}}});}
            ui.separator();
            ui.add_enabled_ui(e.selected.iter().any(|t|matches!(t,Target::Raster(_)|Target::Cell{..})),|ui|{if ui.checkbox(&mut e.alpha,"Protect existing alpha").changed(){e.alpha_patch=Some(e.alpha);}});
            ui.add_enabled_ui(e.selected.iter().any(|t|matches!(t,Target::Raster(_)|Target::Vector(_)|Target::Folder(_))),|ui|{if ui.checkbox(&mut e.clip,"Clip to the base below").changed(){e.clip_patch=Some(e.clip);}});
            for t in &e.selected{if let Some(base)=d.scene.clipping.binding(*t){ui.small(format!("{} → {} ({})",label(d,*t),label(d,base),if drawer_core::clipping::active(d,*t){"active"}else{"inactive base"}));}}
            ui.horizontal(|ui|{if let [Target::Folder(n)]=e.selected.as_slice() && ui.button("Solo this folder").clicked(){solo=Some(Some(*n as usize));}
if d.scene.clipping.solo.is_some() && ui.button("Restore visibility before solo").clicked(){solo=Some(None);}});
            ui.separator();ui.label("Show identification marks (view settings)");let p=&mut self.workspaces.store.current.layer_colors;
            ui.horizontal(|ui|{view_changed|=ui.checkbox(&mut p.list,"Lists").changed();view_changed|=ui.checkbox(&mut p.parents,"Mixed parent folders").changed();view_changed|=ui.checkbox(&mut p.canvas,"Selected canvas bounds").changed();view_changed|=ui.checkbox(&mut p.timeline,"Timeline cell / clip marks").changed();});
            ui.horizontal(|ui|{let r=ui.add_enabled(!e.selected.is_empty()&&(!e.patch.empty()||e.alpha_patch.is_some()||e.clip_patch.is_some()),egui::Button::new("Apply changed properties"));self.layer_properties.apply=Some(r.rect);apply=r.clicked();cancel=ui.button("Cancel").clicked();});
            ui.label("Property edits use one Undo. View switches save separately. Identification colors never change image output. Hidden layers must be shown before painting.");
        });
        if open && !cancel {
            self.layer_properties.draft = Some(e);
        }
        if apply && let Err(err) = self.apply_layer_properties() {
            self.status = Some(err);
        }
        if let Some(folder) = solo {
            let result = drawer_core::clipping::solo(&self.documents[n], folder).and_then(|s| {
                let used: usize = self.documents.iter().map(Document::resident_bytes).sum();
                if s.memory_bytes().saturating_mul(2).saturating_add(used) > DEFAULT_DOCUMENT_BUDGET
                {
                    return Err("Solo exceeds workspace memory".into());
                }
                self.documents[n].commit_state(s).map(|_| ())
            });
            match result {
                Ok(()) => self.layer_properties.draft = None,
                Err(err) => self.status = Some(err),
            }
        }
        if view_changed && let Err(err) = self.persist_workspace() {
            self.status = Some(err);
        }
    }
}

pub(super) fn smoke(root: &std::path::Path) -> Result<(), String> {
    drawer_core::layer_properties_verify::verify(root)?;
    let ctx = egui::Context::default();
    let mut app = Drawer::new(&ctx, None);
    app.simple_view = true;
    app.create_canvas(CanvasSpec {
        width: 32,
        height: 32,
        dpi: 300.0,
        background: Background::Transparent,
    })
    .map_err(|e| e.to_string())?;
    let copy = app.documents[0].layers[0].clone();
    app.documents[0].layers.push(copy);
    let file = root.join("layer-property-ui-base.drawer");
    drawer_core::files::save(&mut app.documents[0], &file).map_err(|e| e.to_string())?;
    let before = app.documents[0].snapshot();
    let mut time = 0.0;
    let step = |app: &mut Drawer, time: &mut f64, events| {
        *time += 0.1;
        super::input_replay::frame(&ctx, app, *time, events);
    };
    step(&mut app, &mut time, vec![]);
    step(&mut app, &mut time, vec![]);
    app.execute_command(&ctx, "layer-properties");
    let e = app
        .layer_properties
        .draft
        .as_mut()
        .ok_or("Layer property command missing")?;
    e.selected = vec![Target::Raster(1), Target::Raster(2)];
    e.values.opacity = 50;
    e.values.color = Some([20, 100, 220]);
    e.patch = Patch {
        opacity: Some(50),
        color: Some(Some([20, 100, 220])),
        ..Default::default()
    };
    step(&mut app, &mut time, vec![]);
    step(&mut app, &mut time, vec![]);
    if app.documents[0].snapshot() != before || app.documents[0].dirty {
        return Err("Layer property draft changed document".into());
    }
    let r = app
        .layer_properties
        .apply
        .ok_or("Layer property Apply missing")?;
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
    if app.layer_properties.draft.is_some()
        || app.documents[0].history_counts().0 != 1
        || core::properties(&app.documents[0], Target::Raster(2)).opacity != 50
        || app.documents[0].layers != before.layers
    {
        return Err(format!(
            "Property production Apply: {:?}, {r:?}",
            app.status
        ));
    }
    let changed = app.documents[0].snapshot();
    let history = app.documents[0].history_counts();
    app.workspaces.store.current.layer_colors.canvas = true;
    app.persist_workspace()?;
    step(&mut app, &mut time, vec![]);
    if app.canvas_view.label_edges != 4
        || app.documents[0].snapshot() != changed
        || app.documents[0].history_counts() != history
    {
        return Err(
            "Canvas identification display changed document or failed to draw bounds".into(),
        );
    }
    app.workspaces.store.current.layer_colors.canvas = false;
    app.persist_workspace()?;
    step(&mut app, &mut time, vec![]);
    if app.canvas_view.label_edges != 0 || app.documents[0].snapshot() != changed {
        return Err("Canvas identification display did not turn off independently".into());
    }
    if !app.documents[0].undo() || app.documents[0].snapshot() != before || !app.documents[0].redo()
    {
        return Err("Batch properties grouped Undo/Redo".into());
    }
    app.execute_command(&ctx, "layer-properties");
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
    if app.layer_properties.draft.is_some() || app.documents[0].snapshot() != changed {
        return Err("Properties Escape changed document".into());
    }
    app.execute_command(&ctx, "layer-properties");
    app.documents[0].edit_pixel(0, 0, 0, [1, 2, 3, 255]);
    let edited = app.documents[0].snapshot();
    if app.apply_layer_properties().is_ok() || app.documents[0].snapshot() != edited {
        return Err("Stale property draft applied".into());
    }
    app.layer_properties.draft = None;
    let file = root.join("layer-property-ui.drawer");
    drawer_core::files::save(&mut app.documents[0], &file).map_err(|e| e.to_string())?;
    if drawer_core::files::open(&file, DEFAULT_DOCUMENT_BUDGET)
        .map_err(|e| e.to_string())?
        .snapshot()
        != app.documents[0].snapshot()
    {
        return Err("Properties UI native reopen".into());
    }
    app.execute_command(&ctx, "layer-properties");
    app.documents.clear();
    app.active = None;
    app.layer_property_window(&ctx);
    if app.layer_properties.draft.is_some() {
        return Err("Closed property document retained draft".into());
    }
    for pen in [false, true] {
        for case in 0..6 {
            let ctx = egui::Context::default();
            let mut app = Drawer::new(&ctx, None);
            app.simple_view = true;
            app.create_canvas(CanvasSpec {
                width: 32,
                height: 32,
                dpi: 300.0,
                background: Background::Transparent,
            })
            .map_err(|e| e.to_string())?;
            app.documents[0].view.angle = 90.0;
            app.documents[0].view.zoom = 2.0;
            app.set_tool(BrushTip::PixelPen);
            app.panels.settings.brush_size = 1.0;
            app.choose_color([200, 40, 20, 255]);
            let moving = case == 1 || case == 5;
            if moving {
                app.drawing.mode = super::precision::Mode::Move;
            }
            let patch = match case {
                0 | 5 => Patch {
                    pixels_locked: Some(true),
                    ..Default::default()
                },
                1 | 4 => Patch {
                    position_locked: Some(true),
                    ..Default::default()
                },
                2 => Patch {
                    hidden: Some(true),
                    ..Default::default()
                },
                _ => Patch {
                    locked: Some(true),
                    ..Default::default()
                },
            };
            let state = core::prepare_patch(&app.documents[0], &[Target::Raster(1)], patch)?;
            app.documents[0].commit_state(state)?;
            let original = app.documents[0].snapshot();
            let history = app.documents[0].history_counts().0;
            let mut time = 0.0;
            let step = |app: &mut Drawer, time: &mut f64| {
                *time += 0.1;
                super::input_replay::frame(&ctx, app, *time, vec![]);
            };
            step(&mut app, &mut time);
            step(&mut app, &mut time);
            for (phase, p) in [
                (drawer_core::pointer_input::Phase::Down, [8.5, 8.5]),
                (drawer_core::pointer_input::Phase::Up, [12.5, 8.5]),
            ] {
                let mut event = super::precision::pointer(&ctx, &app, p, phase);
                if pen {
                    event.source = drawer_core::pointer_input::Source {
                        kind: drawer_core::pointer_input::Kind::Pen,
                        device: 117,
                        contact: 1,
                    };
                    event.pressure = Some(0.5);
                }
                app.drawing.virtual_events(vec![event], pen);
                step(&mut app, &mut time);
            }
            let allowed = case >= 4;
            if (app.documents[0].snapshot() != original) != allowed
                || app.documents[0].history_counts().0 != history + usize::from(allowed)
            {
                return Err(format!(
                    "Layer locks production pen={pen} case={case}: {:?}",
                    app.status
                ));
            }
            if allowed && (!app.documents[0].undo() || app.documents[0].snapshot() != original) {
                return Err("Allowed lock-independent input Undo".into());
            }
        }
    }
    Ok(())
}
pub(super) fn clipping_smoke(root: &std::path::Path) -> Result<(), String> {
    drawer_core::clipping_verify::verify(root)?;
    let ctx = egui::Context::default();
    let mut app = Drawer::new(&ctx, None);
    app.simple_view = true;
    app.create_canvas(CanvasSpec {
        width: 32,
        height: 32,
        dpi: 300.,
        background: Background::Transparent,
    })
    .map_err(|e| e.to_string())?;
    app.documents[0].layers[0].pixels.fill([30, 40, 50, 128]);
    let copy = app.documents[0].layers[0].clone();
    app.documents[0].layers.push(copy);
    let mut time = 0.;
    let step = |app: &mut Drawer, time: &mut f64, events| {
        *time += 0.1;
        super::input_replay::frame(&ctx, app, *time, events);
    };
    step(&mut app, &mut time, vec![]);
    step(&mut app, &mut time, vec![]);
    app.execute_command(&ctx, "layer-clipping");
    let before = app.documents[0].snapshot();
    let count = app.documents[0].history_counts().0;
    let e = app
        .layer_properties
        .draft
        .as_mut()
        .ok_or("Clipping command missing")?;
    e.selected = vec![Target::Raster(2)];
    e.alpha_patch = Some(true);
    e.clip_patch = Some(true);
    step(&mut app, &mut time, vec![]);
    step(&mut app, &mut time, vec![]);
    if app.documents[0].snapshot() != before {
        return Err("Clipping draft mutated image".into());
    }
    let r = app.layer_properties.apply.ok_or("Clipping Apply missing")?;
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
    if app.layer_properties.draft.is_some()
        || !app.documents[0]
            .scene
            .clipping
            .alpha
            .contains(&Target::Raster(2))
        || app.documents[0].history_counts().0 != count + 1
    {
        return Err(format!("Clipping actual Apply failed: {:?}", app.status));
    }
    let saved = app.documents[0].snapshot();
    if !app.documents[0].undo()
        || app.documents[0].snapshot() != before
        || !app.documents[0].redo()
        || app.documents[0].snapshot() != saved
    {
        return Err("Clipping UI Undo/Redo".into());
    }
    app.execute_command(&ctx, "layer-clipping");
    app.documents[0].edit_pixel(0, 0, 0, [1, 2, 3, 128]);
    if app.apply_layer_properties().is_ok() {
        return Err("Stale clipping draft accepted".into());
    }
    app.layer_properties.draft = None;
    for pen in [false, true] {
        let ctx = egui::Context::default();
        let mut app = Drawer::new(&ctx, None);
        app.simple_view = true;
        app.create_canvas(CanvasSpec {
            width: 32,
            height: 32,
            dpi: 300.,
            background: Background::Transparent,
        })
        .map_err(|e| e.to_string())?;
        for y in 0..32 {
            for x in 8..16 {
                app.documents[0].layers[0].pixels[y * 32 + x] = [30, 40, 50, 128];
            }
        }
        let state = drawer_core::clipping::prepare(
            &app.documents[0],
            &[Target::Raster(1)],
            Some(true),
            None,
        )?;
        app.documents[0].commit_state(state)?;
        app.documents[0].view.angle = 90.;
        app.documents[0].view.zoom = 2.;
        app.set_tool(BrushTip::PixelPen);
        app.panels.settings.brush_size = 1.;
        app.choose_color([200, 20, 10, 255]);
        let original = app.documents[0].snapshot();
        let mut time = 0.;
        let step = |app: &mut Drawer, time: &mut f64| {
            *time += 0.1;
            super::input_replay::frame(&ctx, app, *time, vec![]);
        };
        step(&mut app, &mut time);
        step(&mut app, &mut time);
        for (phase, p) in [
            (drawer_core::pointer_input::Phase::Down, [5.5, 8.5]),
            (drawer_core::pointer_input::Phase::Move, [10.5, 8.5]),
            (drawer_core::pointer_input::Phase::Up, [20.5, 8.5]),
        ] {
            let mut event = super::precision::pointer(&ctx, &app, p, phase);
            if pen {
                event.source = drawer_core::pointer_input::Source {
                    kind: drawer_core::pointer_input::Kind::Pen,
                    device: 118,
                    contact: 1,
                };
                event.pressure = Some(0.5);
            }
            app.drawing.virtual_events(vec![event], pen);
            step(&mut app, &mut time);
        }
        if app.documents[0].snapshot() == original
            || !app.documents[0].layers[0]
                .pixels
                .iter()
                .zip(&original.layers[0].pixels)
                .all(|(a, b)| a[3] == b[3])
        {
            return Err(format!(
                "Protected production stroke pen={pen}: {:?}",
                app.status
            ));
        }
        if !app.documents[0].undo() || app.documents[0].snapshot() != original {
            return Err("Protected production stroke single Undo".into());
        }
    }
    Ok(())
}
pub(super) fn blend_smoke(root: &std::path::Path) -> Result<(), String> {
    use drawer_core::layer_blend::Mode;
    drawer_core::layer_blend_verify::verify(root)?;
    for mode in Mode::ALL {
        let ctx = egui::Context::default();
        let mut app = Drawer::new(&ctx, None);
        app.simple_view = true;
        app.create_canvas(CanvasSpec {
            width: 8,
            height: 8,
            dpi: 300.,
            background: Background::Transparent,
        })
        .map_err(|e| e.to_string())?;
        app.documents[0].layers[0].pixels.fill([64, 128, 192, 255]);
        let mut l = app.documents[0].layers[0].clone();
        l.pixels.fill([192, 64, 128, 128]);
        app.documents[0].layers.push(l);
        let state = core::prepare_patch(
            &app.documents[0],
            &[Target::Raster(2)],
            Patch {
                blend: Some(mode),
                ..Default::default()
            },
        )?;
        app.documents[0].commit_state(state)?;
        let expected = drawer_core::render::Renderer::new(&app.documents[0]).pixel([0.5, 0.5]);
        let upload = egui::Color32::from_rgba_unmultiplied(
            expected[0],
            expected[1],
            expected[2],
            expected[3],
        );
        let mut found = false;
        for time in [0., 0.1] {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1000., 700.),
                    )),
                    time: Some(time),
                    ..Default::default()
                },
                |ui| app.show(ui),
            );
            for deltas in output.textures_delta.set.values() {
                for delta in deltas {
                    let egui::ImageData::Color(image) = &delta.image;
                    if image.size == [8, 8] && image.pixels.iter().all(|c| *c == upload) {
                        found = true;
                    }
                }
            }
            output.textures_delta.clear();
        }
        if !found {
            return Err(format!("CPU/GPU texture upload mismatch for {mode:?}"));
        }
    }
    let ctx = egui::Context::default();
    let mut app = Drawer::new(&ctx, None);
    app.simple_view = true;
    app.create_canvas(CanvasSpec {
        width: 8,
        height: 8,
        dpi: 300.,
        background: Background::Transparent,
    })
    .map_err(|e| e.to_string())?;
    let mut time = 0.;
    let step = |app: &mut Drawer, time: &mut f64, events| {
        *time += 0.1;
        super::input_replay::frame(&ctx, app, *time, events);
    };
    step(&mut app, &mut time, vec![]);
    step(&mut app, &mut time, vec![]);
    app.execute_command(&ctx, "layer-blend");
    let before = app.documents[0].snapshot();
    step(&mut app, &mut time, vec![]);
    step(&mut app, &mut time, vec![]);
    let click = |app: &mut Drawer, time: &mut f64, r: egui::Rect| {
        for pressed in [true, false] {
            step(
                app,
                time,
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
    };
    let r = app.layer_properties.modes[2].ok_or("Screen control missing")?;
    click(&mut app, &mut time, r);
    if app.documents[0].snapshot() != before {
        return Err("Blend selector mutated pixels before Apply".into());
    }
    let r = app.layer_properties.apply.ok_or("Blend Apply missing")?;
    click(&mut app, &mut time, r);
    if core::properties(&app.documents[0], Target::Raster(1)).blend != Mode::Screen
        || !app.documents[0].undo()
        || app.documents[0].snapshot() != before
    {
        return Err(format!("Blend button/apply/Undo failed: {:?}", app.status));
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn blend_buttons_and_gpu_texture_payload() {
        let root = crate::tests::TestDirectory::new();
        super::blend_smoke(&root.0).unwrap();
    }
    #[test]
    fn alpha_clipping_buttons_and_mouse_pen() {
        let root = crate::tests::TestDirectory::new();
        super::clipping_smoke(&root.0).unwrap();
    }
    #[test]
    fn property_buttons_marks_and_mouse_pen_lock_paths() {
        let root = crate::tests::TestDirectory::new();
        super::smoke(&root.0).unwrap();
    }
}
