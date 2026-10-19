use super::*;
use drawer_core::{
    bucket,
    enclosure::{self as core, Mode, Settings},
};
#[derive(Clone, Copy)]
pub(super) struct Fill {
    pub layer: usize,
    pub budget: usize,
    pub config: ([u8; 4], bool, bool),
    pub reference: bucket::Settings,
    pub settings: Settings,
    pub mode: Mode,
}
impl drawing::Drawing {
    pub(super) fn enclosure_changed(&self, fill: Option<Fill>) -> bool {
        fill.is_some_and(|s| {
            s.reference != self.fill_settings
                || s.settings != self.enclosure_settings
                || s.config != self.fill_config
                || Some(s.layer) != self.move_layer
        })
    }
}
impl Drawer {
    pub(super) fn fill_small_gaps(&mut self) -> Result<(), String> {
        let n = self.active.ok_or("Open a page first")?;
        let d = &self.documents[n];
        let drawer_core::thumbnail::Target::Raster(layer) = self
            .panels
            .targets
            .get(&d.id)
            .copied()
            .unwrap_or(drawer_core::thumbnail::Target::Raster(0))
        else {
            return Err("Choose a raster fill target".into());
        };
        let slot = BrushTip::ALL
            .iter()
            .position(|t| *t == self.selected)
            .unwrap();
        let config = &self.workspaces.store.current;
        let state = core::prepare(
            d,
            layer,
            &[],
            Mode::SmallGaps,
            config.enclosure,
            config.input.fills[slot],
            self.references.session.workspace.color,
            config.color_modes.transparent || config.input.profiles[slot].transparent,
            config.input.materials[slot].alpha_lock,
        )?;
        let used = self
            .documents
            .iter()
            .map(Document::resident_bytes)
            .sum::<usize>();
        if state.memory_bytes().saturating_mul(2).saturating_add(used) > DEFAULT_DOCUMENT_BUDGET {
            return Err("Small-area fill exceeds workspace memory".into());
        }
        self.documents[n].commit_state(state)?;
        Ok(())
    }
    pub(super) fn enclosure_controls(&mut self, ui: &mut egui::Ui) {
        ui.collapsing("Lasso and enclosed-area fill",|ui|{
            ui.label("Lasso fills the traced shape. Enclose fills only transparent components completely enclosed by reference pixels and the lasso. A component connected to the outer background is kept. Transparency uses Fill options; gaps are not sealed by the lasso.");
            let old=self.workspaces.store.current.enclosure;
            ui.add(egui::Slider::new(&mut self.workspaces.store.current.enclosure.maximum_area,1..=4*1024*1024).logarithmic(true).text("Maximum small-area pixels"));
            if self.workspaces.store.current.enclosure!=old{self.finish_stroke();}
            ui.horizontal_wrapped(|ui|{for(c,label)in[("lasso-fill-tool","Lasso fill"),("enclose-fill-tool","Enclose and fill"),("fill-small-gaps","Patch small unpainted areas")]{if ui.button(label).clicked(){self.execute_command(ui.ctx(),c);}}});
        });
    }
}
pub(super) fn smoke(root: &std::path::Path) -> Result<(), String> {
    use drawer_core::pointer_input::{Kind, Phase, Source};
    core::verify(root)?;
    for pen in [false, true] {
        for enclose in [false, true] {
            let ctx = egui::Context::default();
            let mut app = Drawer::new(&ctx, None);
            app.simple_view = true;
            let mut time = 0.0;
            let mut d = drawer_core::fill_reference_verify::fixture()?;
            d.view.zoom = 8.0;
            d.view.angle = 90.0;
            app.add_document(d);
            app.choose_color([200, 40, 20, 128]);
            app.workspaces.store.current.input.fills[0] = bucket::Settings {
                reference: bucket::Reference::Composite,
                antialias: false,
                ..Default::default()
            };
            let step = |app: &mut Drawer, time: &mut f64, events| {
                *time += 0.1;
                super::input_replay::frame(&ctx, app, *time, events);
            };
            step(&mut app, &mut time, vec![]);
            step(&mut app, &mut time, vec![]);
            let pointer = |app: &Drawer, p, phase| {
                let mut e = super::precision::pointer(&ctx, app, p, phase);
                if pen {
                    e.source = Source {
                        kind: Kind::Pen,
                        device: 102,
                        contact: 1,
                    };
                    e.pressure = Some(0.5);
                }
                e
            };
            let command = if enclose {
                "enclose-fill-tool"
            } else {
                "lasso-fill-tool"
            };
            app.execute_command(&ctx, command);
            let path = root.join(format!("enclosure-base-{pen}-{enclose}.drawer"));
            drawer_core::files::save(&mut app.documents[0], &path).map_err(|e| e.to_string())?;
            let before = app.documents[0].snapshot();
            let history = app.documents[0].history_counts();
            let points = [[0.0, 0.0], [7.0, 0.0], [7.0, 7.0], [0.0, 7.0], [0.0, 0.0]];
            for (i, p) in points.into_iter().enumerate() {
                let phase = if i == 0 {
                    Phase::Down
                } else if i == 4 {
                    Phase::Up
                } else {
                    Phase::Move
                };
                app.drawing
                    .virtual_events(vec![pointer(&app, p, phase)], pen);
                step(&mut app, &mut time, vec![]);
                if i < 4
                    && (app.documents[0].snapshot() != before
                        || app.documents[0].dirty
                        || !app.drawing.pending())
                {
                    return Err(format!(
                        "Lasso fill draft {pen}/{enclose}: {:?}",
                        app.status
                    ));
                }
            }
            if app.drawing.pending()
                || app.documents[0].layers[0].pixel(7, 7, 3, 3) != [200, 40, 20, 128]
                || app.documents[0].layers[0].pixel(7, 7, 0, 0)[3] != if enclose { 0 } else { 128 }
                || app.documents[0].scene != before.scene
                || app.documents[0].history_counts().0 != history.0 + 1
            {
                return Err(format!(
                    "Enclosure production release {pen}/{enclose}: {:?}",
                    app.status
                ));
            }
            let after = app.documents[0].snapshot();
            if !app.documents[0].undo()
                || app.documents[0].snapshot() != before
                || !app.documents[0].redo()
                || app.documents[0].snapshot() != after
            {
                return Err("Lasso fill production Undo/Redo".into());
            }
            app.documents[0].undo();
            for cancel in 0..6 {
                app.execute_command(&ctx, command);
                app.drawing
                    .virtual_events(vec![pointer(&app, [0.0, 0.0], Phase::Down)], pen);
                step(&mut app, &mut time, vec![]);
                let events = match cancel {
                    0 => vec![egui::Event::Key {
                        key: egui::Key::Escape,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: Default::default(),
                    }],
                    1 => vec![egui::Event::WindowFocused(false)],
                    2 => {
                        app.documents[0].view.zoom += 0.1;
                        vec![]
                    }
                    3 => {
                        app.workspaces.store.current.enclosure.maximum_area += 1;
                        vec![]
                    }
                    4 => {
                        app.workspaces.store.current.input.fills[0].exclude.draft = true;
                        vec![]
                    }
                    _ => {
                        app.pressure.open = true;
                        vec![]
                    }
                };
                step(&mut app, &mut time, events);
                if app.drawing.pending() || app.documents[0].snapshot() != before {
                    return Err(format!("Lasso fill cancel {cancel}"));
                }
                app.pressure.open = false;
                app.workspaces.store.current.input.fills[0].exclude.draft = false;
                app.drawing
                    .virtual_events(vec![pointer(&app, [0.0, 0.0], Phase::Up)], pen);
                step(
                    &mut app,
                    &mut time,
                    vec![
                        egui::Event::WindowFocused(true),
                        egui::Event::Key {
                            key: egui::Key::Escape,
                            physical_key: None,
                            pressed: false,
                            repeat: false,
                            modifiers: Default::default(),
                        },
                    ],
                );
            }
            app.workspaces.store.current.enclosure.maximum_area = 8;
            let history = app.documents[0].history_counts();
            app.execute_command(&ctx, "fill-small-gaps");
            if app.documents[0].snapshot() != before || app.documents[0].history_counts() != history
            {
                return Err("Small-area maximum below region changed document".into());
            }
            app.workspaces.store.current.enclosure.maximum_area = 9;
            app.execute_command(&ctx, "fill-small-gaps");
            if app.documents[0].layers[0].pixel(7, 7, 3, 3) != [200, 40, 20, 128]
                || app.documents[0].layers[0].pixel(7, 7, 0, 0)[3] != 0
                || app.documents[0].scene != before.scene
            {
                return Err(format!(
                    "Small-area command limit equality: {:?}",
                    app.status
                ));
            }
            let path = root.join("enclosure-settings.json");
            app.workspaces
                .store
                .current
                .export(&path)
                .map_err(|e| e.to_string())?;
            if drawer_core::workspaces::Snapshot::read(&path)
                .map_err(|e| e.to_string())?
                .enclosure
                != app.workspaces.store.current.enclosure
            {
                return Err("Enclosure settings reopen".into());
            }
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn production_lasso_enclose_mouse_pen_cancel_small_limits() {
        let root = crate::tests::TestDirectory::new();
        super::smoke(&root.0).unwrap();
    }
}
