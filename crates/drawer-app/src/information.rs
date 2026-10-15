use super::*;
use drawer_core::{
    information::{self, Color, OutputProfile, Settings, Side, Theme, Unit},
    workspaces::{Dock, Scope},
};
use std::collections::HashMap;
#[derive(Default)]
struct Session {
    pointer: Option<[f64; 2]>,
    start: Option<[f64; 2]>,
    end: Option<[f64; 2]>,
    waiting: u8,
}
type ProfileKey = (PathBuf, Option<std::time::SystemTime>, u64);
#[derive(Default)]
pub(super) struct Information {
    sessions: HashMap<u64, Session>,
    profile_key: Option<ProfileKey>,
    profile: Option<OutputProfile>,
    pub profile_error: Option<String>,
    color_key: Option<(u64, u64, u32, i64, i64)>,
    color: Option<Color>,
    pub shown: bool,
    #[cfg(test)]
    pub start_button: Option<egui::Rect>,
    #[cfg(test)]
    pub end_button: Option<egui::Rect>,
}
impl Drawer {
    pub(super) fn measurement_active(&self) -> bool {
        self.workspaces.store.current.information.visible
            && self.active.is_some_and(|n| {
                self.information
                    .sessions
                    .get(&self.documents[n].id)
                    .is_some_and(|s| s.waiting != 0)
            })
    }
    pub(super) fn measurement_line(&self) -> Option<([f64; 2], [f64; 2])> {
        let n = self.active?;
        let s = self.information.sessions.get(&self.documents[n].id)?;
        Some((s.start?, s.end?))
    }
    pub(super) fn capture_information(&mut self) {
        self.information
            .sessions
            .retain(|id, _| self.documents.iter().any(|d| d.id == *id));
        if let Some(n) = self.active {
            let id = self.documents[n].id;
            let s = self.information.sessions.entry(id).or_default();
            s.pointer = self.canvas_view.cursor_trace.raw;
            if let Some((document, point)) = self.canvas_view.first_point
                && document == id
            {
                if s.waiting == 1 {
                    s.start = Some(point);
                } else if s.waiting == 2 {
                    s.end = Some(point);
                }
                s.waiting = 0;
            }
        }
    }
    pub(super) fn apply_information(
        &mut self,
        settings: Settings,
        force_side: bool,
    ) -> Result<(), String> {
        let mut incoming = self.capture_workspace();
        incoming.information = settings.clone();
        if force_side || settings.side != self.workspaces.store.current.information.side {
            for p in &mut incoming.placements {
                if p.dock != Dock::Floating {
                    p.dock = if settings.side == Side::Left {
                        Dock::Left
                    } else {
                        Dock::Right
                    };
                }
            }
        }
        self.stage_workspace(incoming, self.workspaces.store.active.clone());
        let result = self.apply_workspace(
            Scope {
                layout: true,
                other: true,
                ..Default::default()
            },
            None,
        );
        if result.is_err() {
            self.workspaces.staged = None;
        }
        result
    }
    pub(super) fn apply_theme(&self, ctx: &egui::Context) {
        let theme = match self.workspaces.store.current.information.theme {
            Theme::Dark => egui::Theme::Dark,
            Theme::Light => egui::Theme::Light,
        };
        if ctx.theme() != theme {
            ctx.set_theme(match theme {
                egui::Theme::Dark => egui::ThemePreference::Dark,
                egui::Theme::Light => egui::ThemePreference::Light,
            });
        }
    }
    fn refresh_output_profile(&mut self, path: Option<&PathBuf>) {
        let key = path.map(|p| {
            let metadata = std::fs::metadata(p).ok();
            (
                p.clone(),
                metadata.as_ref().and_then(|m| m.modified().ok()),
                metadata.as_ref().map_or(0, |m| m.len()),
            )
        });
        if key == self.information.profile_key {
            return;
        }
        self.information.profile_key = key;
        self.information.profile = None;
        match path {
            None => self.information.profile_error = Some("No CMYK output profile selected".into()),
            Some(path) => match OutputProfile::read(path) {
                Ok(p) => {
                    self.information.profile = Some(p);
                    self.information.profile_error = None;
                }
                Err(e) => self.information.profile_error = Some(e.to_string()),
            },
        }
    }
    pub(super) fn information_panel(&mut self, ctx: &egui::Context) {
        let mut settings = self.workspaces.store.current.information.clone();
        if !settings.visible {
            return;
        }
        let Some(n) = self.active else {
            let mut visible = true;
            egui::Window::new("Information")
                .open(&mut visible)
                .show(ctx, |ui| {
                    ui.label("Open a document to read coordinates, lengths and colors.");
                });
            self.workspaces.store.current.information.visible = visible;
            return;
        };
        let id = self.documents[n].id;
        let dpi = self.documents[n].dpi;
        let s = self.information.sessions.entry(id).or_default();
        let position = s
            .pointer
            .or(s.end)
            .or(s.start)
            .filter(|p| p.iter().all(|v| v.is_finite() && v.abs() <= 1e12));
        if let Some(p) = position {
            let key = (
                id,
                self.documents[n].revision,
                self.documents[n].current_frame,
                p[0].floor() as i64,
                p[1].floor() as i64,
            );
            if self.information.color_key != Some(key) {
                self.information.color = information::sample(&self.documents[n], p);
                self.information.color_key = Some(key);
            }
        } else {
            self.information.color = None;
            self.information.color_key = None;
        }
        if settings.cmyk {
            self.refresh_output_profile(settings.output_profile.as_ref());
            if settings.output_profile.is_none() {
                self.information.profile_error = Some("No CMYK output profile selected".into());
            }
        }
        let cmyk = self
            .information
            .profile
            .as_ref()
            .and_then(|p| self.information.color.map(|c| p.convert(c.rgba)));
        let original = settings.clone();
        let mut visible = true;
        let mut choose = false;
        let mut reload = false;
        let mut force_side = false;
        let docked = self
            .workspaces
            .store
            .current
            .placements
            .iter()
            .filter(|p| p.dock != Dock::Floating)
            .collect::<Vec<_>>();
        let all_left = !docked.is_empty() && docked.iter().all(|p| p.dock == Dock::Left);
        let all_right = !docked.is_empty() && docked.iter().all(|p| p.dock == Dock::Right);
        self.information.shown = true;
        egui::Window::new("Information").open(&mut visible).default_pos(egui::pos2(20.0,50.0)).default_width(390.0).vscroll(true).scroll_bar_visibility(crate::ui_behavior::scroll(ctx)).show(ctx,|ui|{
            ui.horizontal_wrapped(|ui|{ui.selectable_value(&mut settings.theme,Theme::Dark,"Dark theme");ui.selectable_value(&mut settings.theme,Theme::Light,"Light theme");if ui.selectable_label(all_left,"Docked tools left").clicked(){settings.side=Side::Left;force_side=true;}
if ui.selectable_label(all_right,"Docked tools right").clicked(){settings.side=Side::Right;force_side=true;}
if ui.button("Interface size").clicked(){self.interface.open=true;}});
            egui::ComboBox::from_id_salt("information-unit").selected_text(format!("{:?}",settings.unit)).show_ui(ui,|ui|{for unit in Unit::ALL{ui.selectable_value(&mut settings.unit,unit,format!("{unit:?}"));}});ui.label("Document uses the current ruler unit. Measurements stay in document coordinates.");
            let session=self.information.sessions.entry(id).or_default();
            ui.horizontal_wrapped(|ui|{let start=ui.button("Pick start on canvas");let end=ui.button("Pick end on canvas");if start.clicked(){session.waiting=1;}
if end.clicked(){session.waiting=2;}#[cfg(test)]{self.information.start_button=Some(start.rect);self.information.end_button=Some(end.rect);}
if ui.button("Clear measurement").clicked(){session.start=None;session.end=None;session.waiting=0;}});
            if session.waiting!=0{ui.label("Click the canvas to set the measurement point.");}
            for(label,point)in[("Start",&mut session.start),("End",&mut session.end)]{if let Some(p)=point{ui.horizontal(|ui|{ui.label(label);for v in p{ui.add(egui::DragValue::new(v).range(-1e12..=1e12).speed(0.1));}});}}
            if let Some(point)=position{let f=settings.unit.factor(dpi,self.aids.preferences.unit);ui.label(format!("Position: X {:.4}, Y {:.4}",point[0]*f,point[1]*f));}else{ui.label("Position: no pointer or measurement point");}
            if let(Some(a),Some(b))=(session.start,session.end)&&let Some(m)=information::measure(a,b,dpi,settings.unit,self.aids.preferences.unit){ui.label(format!("ΔX {:.4} · ΔY {:.4} · length {:.4}",m.delta[0],m.delta[1],m.length));}
            ui.horizontal_wrapped(|ui|{ui.checkbox(&mut settings.rgb,"RGB");ui.checkbox(&mut settings.hsv,"HSV");ui.checkbox(&mut settings.hsl,"HSL");ui.checkbox(&mut settings.cmyk,"CMYK (output profile)");});
            if let Some(c)=self.information.color{
                ui.label(format!("Alpha {} / 255{}",c.rgba[3],if c.rgba[3]==0{" · transparent"}else{""}));if settings.rgb{ui.label(format!("RGB {} {} {}",c.rgba[0],c.rgba[1],c.rgba[2]));}
if settings.hsv{ui.label(format!("HSV {:.2}° {:.2}% {:.2}%",c.hsv[0],c.hsv[1]*100.0,c.hsv[2]*100.0));}
if settings.hsl{ui.label(format!("HSL {:.2}° {:.2}% {:.2}%",c.hsl[0],c.hsl[1]*100.0,c.hsl[2]*100.0));}
                if settings.cmyk&&let Some(result)=&cmyk{match result{Ok(v)=>{ui.label(format!("CMYK {:.2}% {:.2}% {:.2}% {:.2}%",v[0],v[1],v[2],v[3]));},Err(e)=>{ui.label(format!("CMYK unavailable: {e}"));}}}
            }else{ui.label("Color unavailable outside the canvas or without a point");}
            if settings.cmyk{if let Some(e)=&self.information.profile_error{ui.label(format!("CMYK unavailable: {e}"));}
if let Some(path)=&settings.output_profile{ui.label(format!("Output profile: {}",path.display()));}ui.horizontal_wrapped(|ui|{choose=ui.button("Choose local CMYK output profile…").clicked();reload=ui.button("Reload profile").clicked();if ui.button("Clear output profile").clicked(){settings.output_profile=None;}});}
        });
        settings.visible = visible;
        if !visible {
            self.information.sessions.get_mut(&id).unwrap().waiting = 0;
        }
        if choose
            && let Some(path) = rfd::FileDialog::new()
                .add_filter("ICC output profile", &["icc", "icm"])
                .pick_file()
        {
            match path
                .canonicalize()
                .map_err(|e| e.to_string())
                .and_then(|p| {
                    OutputProfile::read(&p)
                        .map(|_| p)
                        .map_err(|e| e.to_string())
                }) {
                Ok(p) => settings.output_profile = Some(p),
                Err(e) => self.status = Some(e),
            }
        }
        if reload {
            self.information.profile_key = None;
        }
        if (settings != original || force_side)
            && let Err(e) = self.apply_information(settings, force_side)
        {
            self.status = Some(e);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{TestDirectory, click, frame, tiny_spec};
    #[test]
    fn canvas_measurement_mouse_rotation_mirror_units_and_color_information_are_read_only() {
        let ctx = egui::Context::default();
        let mut app = Drawer::new(&ctx, None);
        app.create_canvas(CanvasSpec {
            width: 128,
            height: 128,
            ..tiny_spec()
        })
        .unwrap();
        app.documents[0].view.angle = 37.0;
        app.documents[0].view.flipped = true;
        app.documents[0].view.zoom = 2.0;
        app.workspaces.store.current.information.visible = true;
        app.workspaces.store.current.information.cmyk = true;
        app.workspaces.store.current.information.hsv = true;
        app.workspaces.store.current.information.hsl = true;
        frame(&ctx, &mut app, vec![]);
        frame(&ctx, &mut app, vec![]);
        let before = app.documents[0].snapshot();
        let history = app.documents[0].history_counts();
        let id = app.documents[0].id;
        let a = [50.0, 50.0];
        let b = [53.0, 54.0];
        for (start, point) in [(true, a), (false, b)] {
            let button = if start {
                app.information.start_button.unwrap()
            } else {
                app.information.end_button.unwrap()
            };
            click(&ctx, &mut app, button);
            assert!(app.measurement_active());
            let rect = app.canvas_view.viewport.unwrap();
            let screen = app.documents[0].view.document_to_screen(
                point,
                [128.0; 2],
                [rect.center().x as f64, rect.center().y as f64],
                ctx.pixels_per_point() as f64,
            );
            let pos = egui::pos2(screen[0] as f32, screen[1] as f32);
            click(
                &ctx,
                &mut app,
                egui::Rect::from_center_size(pos, egui::vec2(1.0, 1.0)),
            );
            assert!(!app.measurement_active());
        }
        let session = &app.information.sessions[&id];
        let result = information::measure(
            session.start.unwrap(),
            session.end.unwrap(),
            300.0,
            Unit::Pixels,
            drawer_core::aids::Unit::Pixels,
        )
        .unwrap();
        assert!(
            (result.delta[0] - 3.0).abs() < 1e-4
                && (result.delta[1] - 4.0).abs() < 1e-4
                && (result.length - 5.0).abs() < 1e-4
        );
        assert!(
            app.information
                .profile_error
                .as_ref()
                .unwrap()
                .contains("No CMYK")
        );
        assert_eq!(app.documents[0].snapshot(), before);
        assert_eq!(app.documents[0].history_counts(), history);
    }
    #[test]
    fn theme_side_scale_focus_selection_restart_and_bad_settings_recover_without_document_edits() {
        let ctx = egui::Context::default();
        let root = TestDirectory::new();
        let storage = UserStorage::at(root.0.join("user")).unwrap();
        let mut app = Drawer::new(&ctx, None)
            .with_storage(UserStorage::at(storage.root().to_path_buf()).unwrap());
        app.create_canvas(tiny_spec()).unwrap();
        app.set_tool(BrushTip::Pencil);
        let id = app.documents[0].id;
        app.panels
            .targets
            .insert(id, drawer_core::thumbnail::Target::Raster(0));
        frame(&ctx, &mut app, vec![]);
        let before = app.documents[0].snapshot();
        let focus = egui::Id::new("theme-focus-probe");
        let mut text = "한글 focus".to_string();
        let mut o = ctx.run_ui(egui::RawInput::default(), |ui| {
            ui.add(egui::TextEdit::singleline(&mut text).id(focus))
                .request_focus();
        });
        o.textures_delta.clear();
        let mut settings = app.workspaces.store.current.information.clone();
        settings.theme = Theme::Light;
        settings.unit = Unit::Centimeters;
        settings.side = Side::Right;
        app.apply_information(settings.clone(), false).unwrap();
        let mut o = ctx.run_ui(egui::RawInput::default(), |ui| {
            app.apply_theme(ui.ctx());
            ui.add(egui::TextEdit::singleline(&mut text).id(focus));
        });
        o.textures_delta.clear();
        assert_eq!(ctx.theme(), egui::Theme::Light);
        assert_eq!(ctx.memory(|m| m.focused()), Some(focus));
        assert_eq!(app.selected, BrushTip::Pencil);
        assert_eq!(
            app.panels.targets[&id],
            drawer_core::thumbnail::Target::Raster(0)
        );
        assert!(
            app.workspaces
                .store
                .current
                .placements
                .iter()
                .all(|p| p.dock == Dock::Right)
        );
        let restarted = Drawer::new(&egui::Context::default(), None)
            .with_storage(UserStorage::at(storage.root().to_path_buf()).unwrap());
        assert_eq!(restarted.workspaces.store.current.information, settings);
        assert_eq!(app.documents[0].snapshot(), before);
        assert_eq!(app.documents[0].history_counts(), (0, 0));
        let mut bad = settings.clone();
        bad.version = 99;
        assert!(app.apply_information(bad, false).is_err());
        assert_eq!(app.workspaces.store.current.information, settings);
        let path = storage.root().join("settings/workspaces.json");
        let original = std::fs::read_to_string(&path).unwrap();
        let damaged = original.replacen("\"theme\":\"Light\"", "\"theme\":\"unknown-theme\"", 1);
        assert_ne!(damaged, original);
        std::fs::write(&path, damaged).unwrap();
        let recovered = Drawer::new(&egui::Context::default(), None)
            .with_storage(UserStorage::at(storage.root().to_path_buf()).unwrap());
        assert_eq!(
            recovered.workspaces.store.current.information,
            Settings::default()
        );
        assert!(recovered.status.as_ref().unwrap().contains("Workspace"));
    }
}
