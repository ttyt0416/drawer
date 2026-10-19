use super::*;
use drawer_core::{
    selection_edit::Combine,
    selection_shortcuts::{self as core, Command, Preview, Settings, Source},
};
#[derive(Default)]
pub(super) struct Panel {
    draft: Option<Box<Draft>>,
    settings: Option<Box<Settings>>,
    apply: Option<egui::Rect>,
    confirm: Option<egui::Rect>,
    settings_apply: Option<egui::Rect>,
    pub(super) buttons: Vec<(Command, egui::Rect)>,
}
struct Draft {
    id: u64,
    revision: u64,
    frame: u32,
    source: Source,
    combine: Combine,
    confirm: bool,
    key: Option<(Source, Combine)>,
    preview: Option<Result<Preview, String>>,
}
fn selectable_ruler(kind: &drawer_core::rulers::Kind) -> bool {
    match kind {
        drawer_core::rulers::Kind::Straight { .. } => true,
        drawer_core::rulers::Kind::Special(s) => s.contour().is_ok_and(|p| p.is_some()),
        _ => false,
    }
}
impl Drawer {
    pub(super) fn begin_contour_selection(&mut self) -> Result<(), String> {
        let d = &self.documents[self.active.ok_or("Open a page first")?];
        let source = if let Some(p) = d.scene.lines.layers.iter().flat_map(|l| &l.paths).next() {
            Source::Vector(p.id)
        } else {
            Source::Rulers(
                d.scene
                    .rulers
                    .guides
                    .iter()
                    .filter(|g| selectable_ruler(&g.kind))
                    .map(|g| g.id)
                    .collect(),
            )
        };
        self.selection_shortcuts.draft = Some(Box::new(Draft {
            id: d.id,
            revision: d.revision,
            frame: d.current_frame,
            source,
            combine: self.workspaces.store.current.selection_edit.combine,
            confirm: false,
            key: None,
            preview: None,
        }));
        Ok(())
    }
    pub(super) fn edit_selection_shortcuts(&mut self) {
        self.selection_shortcuts.settings = Some(Box::new(
            self.workspaces.store.current.selection_shortcuts.clone(),
        ));
    }
    fn apply_contour_selection(&mut self) -> Result<(), String> {
        let e = self
            .selection_shortcuts
            .draft
            .as_ref()
            .ok_or("Contour draft is closed")?;
        let d = &mut self.documents[self.active.ok_or("Open a page first")?];
        if d.id != e.id || d.revision != e.revision || d.current_frame != e.frame {
            return Err("Page/frame changed; reopen contour selection".into());
        }
        d.edit_scene(core::prepare(d, &e.source, e.combine, e.confirm)?)?;
        self.selection_shortcuts.draft = None;
        Ok(())
    }
    pub(super) fn selection_shortcut_windows(&mut self, ctx: &egui::Context) {
        self.selection_shortcuts.buttons.clear();
        let has_area = self.active.is_some_and(|n| {
            let s = &self.documents[n].scene;
            s.selection.is_some() || s.selection_mask.is_some()
        });
        if self.workspaces.store.current.selection_shortcuts.visible && has_area {
            let commands = self
                .workspaces
                .store
                .current
                .selection_shortcuts
                .commands
                .clone();
            let mut action = None;
            let mut settings = false;
            egui::Window::new("Selection shortcuts")
                .default_width(500.0)
                .min_width(500.0)
                .default_pos(egui::pos2(12.0, 570.0))
                .collapsible(false)
                .resizable(false)
                .show(ctx, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        for c in commands {
                            let r = ui.add_enabled(
                                self.command_reason(c.action()).is_none(),
                                egui::Button::new(c.label()),
                            );
                            self.selection_shortcuts.buttons.push((c, r.rect));
                            if r.clicked() {
                                action = Some(c.action());
                            }
                        }
                        settings = ui.button("Edit…").clicked();
                    });
                });
            if let Some(c) = action {
                self.execute_command(ctx, c);
            }
            if settings {
                self.edit_selection_shortcuts();
            }
        }
        self.selection_shortcut_settings(ctx);
        let Some(mut e) = self.selection_shortcuts.draft.take() else {
            return;
        };
        let Some(n) = self.active.filter(|n| self.documents[*n].id == e.id) else {
            return;
        };
        let doc = &self.documents[n];
        let fresh = doc.revision == e.revision && doc.current_frame == e.frame;
        let mut open = !ctx.input(|i| i.key_pressed(egui::Key::Escape));
        let mut apply = false;
        let mut cancel = false;
        self.selection_shortcuts.apply = None;
        self.selection_shortcuts.confirm = None;
        egui::Window::new("Selection from vector / ruler contour").default_width(460.0).min_width(460.0).resizable(false).open(&mut open).collapsible(false).show(ctx,|ui| {
            let mut rulers=matches!(e.source,Source::Rulers(_));let before=rulers;
            ui.horizontal(|ui|{ui.radio_value(&mut rulers,false,"Vector path");ui.radio_value(&mut rulers,true,"Ruler segments");});
            if rulers!=before {e.source=if rulers {Source::Rulers(vec![])}else{Source::Vector(doc.scene.lines.layers.iter().flat_map(|l|&l.paths).next().map_or(0,|p|p.id))};}
            match &mut e.source {
                Source::Vector(id)=>{egui::ComboBox::from_id_salt("selection-contour-path").selected_text(format!("Path {id}")).show_ui(ui,|ui|{for l in &doc.scene.lines.layers{for p in &l.paths{ui.selectable_value(id,p.id,format!("{} / {} · {}",l.name,p.id,if p.closed{"closed"}else{"open"}));}}});},
                Source::Rulers(ids)=>{ui.label("Select a connected straight-ruler chain or one closed circle / curve ruler.");egui::ScrollArea::vertical().id_salt("contour-ruler-list").max_height(140.0).show(ui,|ui|{for g in &doc.scene.rulers.guides {if selectable_ruler(&g.kind) {let mut selected=ids.contains(&g.id);if ui.checkbox(&mut selected,&g.name).changed(){if selected{ids.push(g.id);}else{ids.retain(|id|*id!=g.id);}}}}});},
            }
            egui::ComboBox::from_id_salt("selection-contour-combine").selected_text(format!("{:?}",e.combine)).show_ui(ui,|ui|{for op in Combine::ALL{ui.selectable_value(&mut e.combine,op,format!("{op:?}"));}});
            let key=(e.source.clone(),e.combine);if fresh && e.key.as_ref()!=Some(&key){e.confirm=false;e.preview=Some(core::preview(doc,&e.source,e.combine));e.key=Some(key);}
            let mut allowed=false;
            if !fresh {ui.label("Page/frame changed; cancel and reopen this draft.");}
            else if let Some(result)=&e.preview {match result {
                Ok(p)=>{paint_preview(ui,doc,p);allowed=p.closure.is_none();if p.closure.is_some(){let r=ui.checkbox(&mut e.confirm,"Confirm auto-close along the magenta segment");self.selection_shortcuts.confirm=Some(r.rect);allowed=e.confirm;}},
                Err(err)=>{ui.label(err);},
            }}
            ui.label("Blue shows the resulting selection. Pixels, path geometry and rulers remain unchanged. Apply creates one Undo entry; Cancel discards the preview.");
            ui.horizontal(|ui|{let r=ui.add_enabled(allowed,egui::Button::new("Apply contour selection"));self.selection_shortcuts.apply=Some(r.rect);apply=r.clicked();cancel=ui.button("Cancel").clicked();});
        });
        if open && !cancel {
            self.selection_shortcuts.draft = Some(e);
        }
        if apply && let Err(err) = self.apply_contour_selection() {
            self.status = Some(err);
        }
    }
    fn selection_shortcut_settings(&mut self, ctx: &egui::Context) {
        let Some(mut e) = self.selection_shortcuts.settings.take() else {
            return;
        };
        let mut open = !ctx.input(|i| i.key_pressed(egui::Key::Escape));
        let mut apply = false;
        let mut cancel = false;
        egui::Window::new("Edit selection shortcuts")
            .default_width(460.0)
            .min_width(460.0)
            .resizable(false)
            .open(&mut open)
            .collapsible(false)
            .vscroll(true)
            .show(ctx, |ui| {
                ui.checkbox(
                    &mut e.visible,
                    "Show toolbar when this page has a selection",
                );
                let mut action = None;
                for (i, c) in e.commands.iter().copied().enumerate() {
                    ui.push_id(i, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(c.label());
                            if ui.add_enabled(i > 0, egui::Button::new("Up")).clicked() {
                                action = Some((i, 0));
                            }
                            if ui
                                .add_enabled(i + 1 < e.commands.len(), egui::Button::new("Down"))
                                .clicked()
                            {
                                action = Some((i, 1));
                            }
                            if ui.button("Remove").clicked() {
                                action = Some((i, 2));
                            }
                        });
                    });
                }
                if let Some((i, op)) = action {
                    match op {
                        0 => e.commands.swap(i, i - 1),
                        1 => e.commands.swap(i, i + 1),
                        _ => {
                            e.commands.remove(i);
                        }
                    }
                }
                ui.label("Add command:");
                ui.horizontal_wrapped(|ui| {
                    for c in Command::ALL {
                        if ui
                            .add_enabled(!e.commands.contains(&c), egui::Button::new(c.label()))
                            .clicked()
                        {
                            e.commands.push(c);
                        }
                    }
                });
                if ui.button("Restore defaults in draft").clicked() {
                    *e = Settings::default();
                }
                ui.horizontal(|ui| {
                    let r = ui.add_enabled(e.valid(), egui::Button::new("Apply toolbar"));
                    self.selection_shortcuts.settings_apply = Some(r.rect);
                    apply = r.clicked();
                    cancel = ui.button("Cancel").clicked();
                });
            });
        if apply && e.valid() {
            self.workspaces.store.current.selection_shortcuts = *e;
        } else if open && !cancel {
            self.selection_shortcuts.settings = Some(e);
        }
    }
}
fn paint_preview(ui: &mut egui::Ui, doc: &Document, p: &Preview) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(256.0, 192.0), egui::Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, egui::Color32::from_gray(35));
    let mut lo = [0.0f64; 2];
    let mut hi = [doc.width as f64, doc.height as f64];
    for q in &p.points {
        for i in 0..2 {
            lo[i] = lo[i].min(q[i]);
            hi[i] = hi[i].max(q[i]);
        }
    }
    let scale = (248.0 / (hi[0] - lo[0]).max(1.0)).min(184.0 / (hi[1] - lo[1]).max(1.0));
    let project = |q: [f64; 2]| {
        rect.min
            + egui::vec2(
                (4.0 + (q[0] - lo[0]) * scale) as f32,
                (4.0 + (q[1] - lo[1]) * scale) as f32,
            )
    };
    for y in 0..48 {
        for x in 0..64 {
            let a = [
                lo[0] + (hi[0] - lo[0]) * x as f64 / 64.0,
                lo[1] + (hi[1] - lo[1]) * y as f64 / 48.0,
            ];
            let b = [
                lo[0] + (hi[0] - lo[0]) * (x + 1) as f64 / 64.0,
                lo[1] + (hi[1] - lo[1]) * (y + 1) as f64 / 48.0,
            ];
            let alpha = p
                .scene
                .selection_coverage([(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0]);
            if alpha > 0 {
                painter.rect_filled(
                    egui::Rect::from_two_pos(project(a), project(b)),
                    0.0,
                    egui::Color32::from_rgba_unmultiplied(30, 150, 255, alpha / 2),
                );
            }
        }
    }
    for pair in p.points.windows(2) {
        painter.line_segment(
            [project(pair[0]), project(pair[1])],
            egui::Stroke::new(1.5, egui::Color32::WHITE),
        );
    }
    let segment = p
        .closure
        .unwrap_or([*p.points.last().unwrap(), p.points[0]]);
    painter.line_segment(
        segment.map(project),
        egui::Stroke::new(
            2.0,
            if p.closure.is_some() {
                egui::Color32::MAGENTA
            } else {
                egui::Color32::WHITE
            },
        ),
    );
}

pub(crate) fn smoke(root: &std::path::Path) -> Result<(), String> {
    drawer_core::selection_shortcuts_verify::verify(root)?;
    for rulers in [false, true] {
        for closed in [false, true] {
            let ctx = egui::Context::default();
            let mut app = Drawer::new(&ctx, None);
            app.simple_view = true;
            app.create_canvas(CanvasSpec {
                width: 16,
                height: 16,
                dpi: 300.0,
                background: Background::Transparent,
            })
            .map_err(|e| e.to_string())?;
            let points = vec![[2.0, 2.0], [10.0, 2.0], [10.0, 10.0], [2.0, 10.0]];
            let layer = app.documents[0].scene.lines.add_layer("Outline")?;
            let path = app.documents[0].scene.lines.add_path(
                layer,
                points.clone(),
                closed,
                1.0,
                [0, 0, 0, 255],
            )?;
            let source = if rulers {
                let mut ids = vec![];
                for i in 0..if closed { 4 } else { 3 } {
                    ids.push(app.documents[0].scene.rulers.add(
                        &format!("Side {i}"),
                        drawer_core::rulers::Kind::Straight {
                            a: points[i],
                            b: points[(i + 1) % 4],
                        },
                    )?);
                }
                Source::Rulers(ids)
            } else {
                Source::Vector(path)
            };
            let mut time = 0.0;
            let step = |app: &mut Drawer, time: &mut f64, events| {
                *time += 0.1;
                super::input_replay::frame(&ctx, app, *time, events);
            };
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
                step(app, time, vec![]);
            };
            step(&mut app, &mut time, vec![]);
            step(&mut app, &mut time, vec![]);
            let before = app.documents[0].snapshot();
            let history = app.documents[0].history_counts();
            app.execute_command(&ctx, "contour-selection");
            app.selection_shortcuts
                .draft
                .as_mut()
                .ok_or("Contour command missing")?
                .source = source.clone();
            step(&mut app, &mut time, vec![]);
            step(&mut app, &mut time, vec![]);
            if app.documents[0].snapshot() != before || app.documents[0].history_counts() != history
            {
                return Err("Contour preview changed page/history".into());
            }
            if !closed {
                let r = app
                    .selection_shortcuts
                    .apply
                    .ok_or("Apply button missing")?;
                click(&mut app, &mut time, r);
                if app.selection_shortcuts.draft.is_none() || app.documents[0].snapshot() != before
                {
                    return Err("Open contour applied without confirmation".into());
                }
                let r = app
                    .selection_shortcuts
                    .confirm
                    .ok_or("Open contour confirmation missing")?;
                click(&mut app, &mut time, r);
                if !app.selection_shortcuts.draft.as_ref().unwrap().confirm {
                    return Err("Auto-close checkbox did not confirm".into());
                }
            }
            let r = app
                .selection_shortcuts
                .apply
                .ok_or("Contour Apply missing")?;
            click(&mut app, &mut time, r);
            let d = &app.documents[0];
            if app.selection_shortcuts.draft.is_some()
                || d.layers != before.layers
                || d.scene.lines != before.scene.lines
                || d.scene.rulers != before.scene.rulers
                || d.scene.selection_coverage([5.5, 5.5]) != 255
                || d.scene.selection_coverage([0.5, 0.5]) != 0
                || d.history_counts().0 != history.0 + 1
            {
                return Err(format!(
                    "Contour production Apply: rect={r:?} rulers={rulers} closed={closed} draft={} coverage={}/{} history={:?}/{:?} layers={} lines={} rulers={} status={:?}",
                    app.selection_shortcuts.draft.is_some(),
                    d.scene.selection_coverage([5.5, 5.5]),
                    d.scene.selection_coverage([0.5, 0.5]),
                    d.history_counts(),
                    history,
                    d.layers == before.layers,
                    d.scene.lines == before.scene.lines,
                    d.scene.rulers == before.scene.rulers,
                    app.status
                ));
            }
            step(&mut app, &mut time, vec![]);
            step(&mut app, &mut time, vec![]);
            let selected = app.documents[0].snapshot();
            let history = app.documents[0].history_counts();
            let r = app
                .selection_shortcuts
                .buttons
                .iter()
                .find(|(c, _)| *c == Command::Outline)
                .ok_or("Outline toolbar button missing")?
                .1;
            click(&mut app, &mut time, r);
            if app.workspaces.store.current.selection_outline
                || app.documents[0].snapshot() != selected
                || app.documents[0].history_counts() != history
                || app.canvas_view.selection_edges != 0
            {
                return Err(format!(
                    "Toolbar outline changed selection/history: rulers={rulers} closed={closed} rect={r:?} outline={} same={} history={:?}/{history:?} edges={}",
                    app.workspaces.store.current.selection_outline,
                    app.documents[0].snapshot() == selected,
                    app.documents[0].history_counts(),
                    app.canvas_view.selection_edges
                ));
            }
            let r = app
                .selection_shortcuts
                .buttons
                .iter()
                .find(|(c, _)| *c == Command::Undo)
                .ok_or("Undo toolbar button missing")?
                .1;
            click(&mut app, &mut time, r);
            if app.documents[0].snapshot() != before {
                return Err("Toolbar did not Undo selection change".into());
            }
            app.execute_command(&ctx, "redo");
            if app.documents[0].snapshot() != selected {
                return Err("Contour Redo failed".into());
            }
            app.execute_command(&ctx, "selection-shortcuts");
            app.selection_shortcuts.settings.as_mut().unwrap().commands =
                vec![Command::Outline, Command::Contour, Command::Clear];
            step(&mut app, &mut time, vec![]);
            step(&mut app, &mut time, vec![]);
            let r = app
                .selection_shortcuts
                .settings_apply
                .ok_or("Toolbar settings Apply missing")?;
            click(&mut app, &mut time, r);
            step(&mut app, &mut time, vec![]);
            if app
                .selection_shortcuts
                .buttons
                .iter()
                .map(|(c, _)| *c)
                .collect::<Vec<_>>()
                != vec![Command::Outline, Command::Contour, Command::Clear]
                || app.documents[0].snapshot() != selected
            {
                return Err("Toolbar configuration/order changed drawing".into());
            }
            let file = root.join(format!("shortcut-settings-{rulers}-{closed}.json"));
            app.capture_workspace()
                .export(&file)
                .map_err(|e| e.to_string())?;
            if drawer_core::workspaces::Snapshot::read(&file)
                .map_err(|e| e.to_string())?
                .selection_shortcuts
                != app.workspaces.store.current.selection_shortcuts
            {
                return Err("Production toolbar settings reopen".into());
            }
            app.execute_command(&ctx, "contour-selection");
            app.documents[0].edit_pixel(0, 0, 0, [1, 2, 3, 255]);
            let before = app.documents[0].snapshot();
            if app.apply_contour_selection().is_ok() || app.documents[0].snapshot() != before {
                return Err("Stale contour draft changed page".into());
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
            if app.selection_shortcuts.draft.is_some() || app.documents[0].snapshot() != before {
                return Err("Contour Escape changed page".into());
            }
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn production_toolbar_contour_confirmation_undo_settings_cancel() {
        let root = crate::tests::TestDirectory::new();
        super::smoke(&root.0).unwrap();
    }
}
