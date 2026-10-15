use super::*;
use drawer_core::pressure::Settings;
#[derive(Default)]
pub(super) struct Editor {
    pub open: bool,
    draft: Option<Settings>,
    tool: usize,
    path: String,
    replace: bool,
    #[cfg(test)]
    buttons: [Option<egui::Rect>; 2],
}
impl Drawer {
    pub(super) fn pressure_settings(&mut self, ctx: &egui::Context) {
        if !self.pressure.open {
            return;
        }
        let mut draft = self
            .pressure
            .draft
            .unwrap_or(self.workspaces.store.current.pressure);
        let mut open = true;
        let mut apply = false;
        let mut cancel = false;
        egui::Window::new("Brush pressure").open(&mut open).vscroll(true).show(ctx,|ui| {
            let a=ui.add_enabled_ui(draft.valid(),|ui|crate::help::button(ui,"Apply pressure","pressure")).inner;
            let c=crate::help::button(ui,"Cancel pressure","pressure");
            apply=a.clicked();cancel=c.clicked();
            #[cfg(test)] {self.pressure.buttons=[Some(a.rect),Some(c.rect)];}
            ui.separator();
            ui.horizontal(|ui| {for (n,tip) in BrushTip::ALL.into_iter().enumerate() {
                if crate::help::selectable(ui,self.pressure.tool==n,tip.name(),"pressure").clicked(){self.pressure.tool=n;}
            }});
            let mapping=&mut draft.tools[self.pressure.tool];
            crate::help::checkbox(ui,&mut mapping.size,"Pressure controls size","pressure");
            crate::help::checkbox(ui,&mut mapping.opacity,"Pressure controls opacity","pressure");
            ui.label("Input pressure range");
            ui.horizontal(|ui| {
                ui.label("Minimum");super::edit::number(ui,&mut mapping.response.minimum,0.0..=1.0,0.01,"","pressure");
                ui.label("Maximum");super::edit::number(ui,&mut mapping.response.maximum,0.0..=1.0,0.01,"","pressure");
            });
            ui.label("Monotone output curve (endpoints fixed at 0 and 1)");
            for n in 1..4 {super::edit::slider(ui,&mut mapping.response.curve[n],0.0..=1.0,format!("At {}% range",n*25),"pressure");}
            if mapping.response.error().is_none() {
                let (rect,_)=ui.allocate_exact_size(egui::vec2(240.0,80.0),egui::Sense::hover());
                let painter=ui.painter();
                painter.rect_stroke(rect,0.0,egui::Stroke::new(1.0,egui::Color32::GRAY),egui::StrokeKind::Inside);
                let points=(0..=64).map(|n| {
                    let x=n as f64/64.0;let y=mapping.response.map(x).unwrap();
                    egui::pos2(rect.left()+x as f32*rect.width(),rect.bottom()-y as f32*rect.height())
                }).collect();
                painter.add(egui::Shape::line(points,egui::Stroke::new(1.5,egui::Color32::LIGHT_BLUE)));
            }
            ui.label("Pressure 0–1 scales each enabled axis independently. Mouse and missing pressure use 1.");
            ui.label("A stroke keeps the settings used when it starts. Opacity pressure limits the stroke's ink contribution, including overlapping stamps.");
            ui.separator();
            ui.label("Local pressure preset path (JSON)");
            if super::edit::text(ui,&mut self.pressure.path,"pressure").changed(){self.pressure.replace=false;}
            crate::help::checkbox(ui,&mut self.pressure.replace,"Replace an existing preset","pressure");
            let path=PathBuf::from(&self.pressure.path);
            if ui.add_enabled(!self.pressure.path.is_empty(),egui::Button::new("Load preset into draft")).clicked() {
                match Settings::read(&path){Ok(s)=>draft=s,Err(e)=>self.status=Some(e.to_string())}
            }
            if ui.add_enabled(draft.valid() && !self.pressure.path.is_empty() && (!path.exists() || self.pressure.replace),egui::Button::new("Save draft preset")).clicked()
                && let Err(e)=draft.save(&path){self.status=Some(e.to_string());}
            if ui.add_enabled(self.storage.is_some(),egui::Button::new("Load before-apply backup")).clicked() {
                match drawer_core::workspaces::Snapshot::read(&self.storage.as_ref().unwrap().root().join("settings/workspace-before-apply.json")) {
                    Ok(s)=>draft=s.pressure,Err(e)=>self.status=Some(e.to_string()),
                }
            }
            if crate::help::button(ui,"Reset pressure draft","pressure").clicked(){draft=Settings::default();}
            for (tip,m) in BrushTip::ALL.into_iter().zip(draft.tools) {
                if let Some(error)=m.response.error(){ui.colored_label(egui::Color32::YELLOW,format!("{}: {error}",tip.name()));}
            }

        });
        if !open || cancel {
            self.pressure.open = false;
            self.pressure.draft = None;
        } else if apply {
            let mut s = self.capture_workspace();
            s.pressure = draft;
            self.stage_workspace(s, self.workspaces.store.active.clone());
            match self.apply_workspace(
                drawer_core::workspaces::Scope {
                    other: true,
                    ..Default::default()
                },
                None,
            ) {
                Ok(()) => {
                    self.pressure.open = false;
                    self.pressure.draft = None;
                }
                Err(e) => {
                    self.status = Some(e);
                    self.pressure.draft = Some(draft);
                    self.workspaces.staged = None;
                }
            }
        } else {
            self.pressure.draft = Some(draft);
        }
    }
}

pub(super) fn smoke_pressure() -> Result<(), String> {
    use drawer_core::{
        brush_cursor::Shape,
        painting::{Stroke, Style},
        pressure::Mapping,
    };
    for curved in [false, true] {
        for pressure in [0.0, 0.5, 1.0] {
            let mut d = Document::create(
                CanvasSpec {
                    width: 32,
                    height: 16,
                    dpi: 300.0,
                    background: Background::Transparent,
                },
                4096,
            )
            .map_err(|e| e.to_string())?;
            let mut style = Style {
                shape: Shape {
                    size: 4.0,
                    aspect: 1.0,
                    angle: 0.0,
                },
                tip: BrushTip::Pen,
                color: [20, 40, 80, 200],
                smoothing: 0.0,
                pressure: Mapping {
                    size: false,
                    opacity: true,
                    ..Default::default()
                },
            };
            if curved {
                style.pressure.response = drawer_core::pressure::Response {
                    minimum: 0.25,
                    maximum: 0.75,
                    curve: [0.0, 0.1, 0.25, 0.8, 1.0],
                };
            }
            let mut s = Stroke::begin(&d, 0, style)?;
            s.sample_pressure(&d, [5.5, 6.5], pressure)?;
            s.sample_pressure(&d, [25.5, 6.5], pressure)?;
            s.commit(&mut d)?;
            let expected = if curved && pressure == 0.5 {
                50
            } else {
                (200.0 * pressure).round() as u8
            };
            if d.composite_pixel(15, 6)[3] != expected {
                return Err("Native pressure opacity mismatch".into());
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{TestDirectory, click, frame, tiny_spec};
    #[test]
    fn pressure_draft_cancel_apply_backup_restart_and_local_preset_validation() {
        let dir = TestDirectory::new();
        let ctx = egui::Context::default();
        let mut app = Drawer::new(&ctx, None).with_storage(UserStorage::at(dir.0.clone()).unwrap());
        app.create_canvas(tiny_spec()).unwrap();
        let before = app.documents[0].snapshot();
        let initial = app.workspaces.store.current.pressure;
        let mut next = initial;
        next.tools[0].size = false;
        next.tools[0].opacity = true;
        next.tools[0].response = drawer_core::pressure::Response {
            minimum: 0.25,
            maximum: 0.75,
            curve: [0.0, 0.1, 0.25, 0.8, 1.0],
        };
        app.pressure.open = true;
        app.pressure.draft = Some(next);
        frame(&ctx, &mut app, vec![]);
        frame(&ctx, &mut app, vec![]);
        let rect = app.pressure.buttons[1].unwrap();
        click(&ctx, &mut app, rect);
        assert_eq!(app.workspaces.store.current.pressure, initial);
        let mut invalid = next;
        invalid.tools[0].response.minimum = 0.75;
        app.pressure.open = true;
        app.pressure.draft = Some(invalid);
        frame(&ctx, &mut app, vec![]);
        frame(&ctx, &mut app, vec![]);
        let rect = app.pressure.buttons[0].unwrap();
        click(&ctx, &mut app, rect);
        assert!(app.pressure.open);
        assert_eq!(app.workspaces.store.current.pressure, initial);
        app.pressure.open = true;
        app.pressure.draft = Some(next);
        frame(&ctx, &mut app, vec![]);
        frame(&ctx, &mut app, vec![]);
        let rect = app.pressure.buttons[0].unwrap();
        click(&ctx, &mut app, rect);
        assert!(!app.pressure.open);
        assert_eq!(app.workspaces.store.current.pressure, next);
        assert_eq!(
            drawer_core::workspaces::Snapshot::read(
                &dir.0.join("settings/workspace-before-apply.json")
            )
            .unwrap()
            .pressure,
            initial
        );
        let restarted = Drawer::new(&egui::Context::default(), None)
            .with_storage(UserStorage::at(dir.0.clone()).unwrap());
        assert_eq!(restarted.workspaces.store.current.pressure, next);
        assert!(app.documents[0].snapshot() == before);
        assert_eq!(app.documents[0].history_counts(), (0, 0));
        let path = dir.0.join("압력.json");
        next.save(&path).unwrap();
        assert_eq!(Settings::read(&path).unwrap(), next);
        let bytes = std::fs::read(&path).unwrap();
        std::fs::write(path.with_extension("json.write.tmp"), "foreign").unwrap();
        assert!(initial.save(&path).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        std::fs::remove_file(path.with_extension("json.write.tmp")).unwrap();
        std::fs::write(&path, b"{broken").unwrap();
        assert!(Settings::read(&path).is_err());
        let mut bad = next;
        bad.version = 99;
        assert!(bad.save(&path).is_err());
        std::fs::write(&path, vec![b' '; 65537]).unwrap();
        assert!(Settings::read(&path).is_err());
    }
}
