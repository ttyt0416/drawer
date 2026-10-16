use super::*;
use drawer_core::display_view::{self, Framing, Monitor, Preferences};
use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

pub(super) struct DisplayPane {
    pub preferences: Preferences,
    pub settings_open: bool,
    pub monitors: Vec<Monitor>,
    pub canvas: canvas_view::CanvasView,
    framing: HashMap<u64, Framing>,
    placed: Option<(Monitor, bool)>,
    poll_at: Instant,
    undo: Vec<Preferences>,
    redo: Vec<Preferences>,
    pub error: Option<String>,
    pub rendered_native: bool,
    pub rendered_frames: usize,
    pub native_windowed: bool,
    #[cfg(test)]
    pub close_button: Option<egui::Rect>,
    #[cfg(test)]
    pub sync_button: Option<egui::Rect>,
    #[cfg(test)]
    pub undo_button: Option<egui::Rect>,
    #[cfg(test)]
    pub redo_button: Option<egui::Rect>,
}
impl DisplayPane {
    #[cfg(test)]
    pub fn framing_for(&self, id: u64) -> Option<Framing> {
        self.framing.get(&id).copied()
    }
    pub fn new(ctx: &egui::Context) -> Self {
        Self {
            preferences: Default::default(),
            settings_open: false,
            monitors: Vec::new(),
            canvas: canvas_view::CanvasView::new(ctx),
            framing: HashMap::new(),
            placed: None,
            poll_at: Instant::now(),
            undo: Vec::new(),
            redo: Vec::new(),
            error: None,
            rendered_native: false,
            rendered_frames: 0,
            native_windowed: false,
            #[cfg(test)]
            close_button: None,
            #[cfg(test)]
            sync_button: None,
            #[cfg(test)]
            undo_button: None,
            #[cfg(test)]
            redo_button: None,
        }
    }
    pub fn poll_monitors(&mut self, ctx: &egui::Context, frame: &eframe::Frame) {
        if Instant::now() >= self.poll_at {
            self.poll_at = Instant::now() + Duration::from_secs(1);
            if let Some(window) = frame.winit_window() {
                let primary = window.primary_monitor();
                let monitors = window
                    .available_monitors()
                    .enumerate()
                    .map(|(index, m)| {
                        let p = m.position();
                        let s = m.size();
                        Monitor {
                            key: m
                                .name()
                                .unwrap_or_else(|| format!("screen-{index}-{}-{}", p.x, p.y)),
                            index,
                            position: [p.x, p.y],
                            size: [s.width, s.height],
                            scale: m.scale_factor(),
                            primary: primary.as_ref() == Some(&m),
                        }
                    })
                    .filter(Monitor::valid)
                    .collect::<Vec<_>>();
                if self.monitors != monitors {
                    self.monitors = monitors;
                    self.placed = None;
                }
            }
        }
        ctx.request_repaint_after(Duration::from_secs(1));
    }
    pub(super) fn apply(
        &mut self,
        next: Preferences,
        storage: Option<&UserStorage>,
        record: bool,
    ) -> Result<(), String> {
        if self.preferences == next {
            return Ok(());
        }
        if !next.valid() {
            return Err("Invalid display settings".into());
        }
        if let Some(storage) = storage
            && let Err(error) = next.save(storage)
        {
            self.error = Some(format!(
                "Display changed, but settings could not be saved: {error}"
            ));
        }
        if record {
            self.undo.push(self.preferences.clone());
            if self.undo.len() > 100 {
                self.undo.remove(0);
            }
            self.redo.clear();
        }
        if next.preferred != self.preferences.preferred
            || next.fullscreen != self.preferences.fullscreen
            || next.visible != self.preferences.visible
        {
            self.placed = None;
        }
        self.preferences = next;
        Ok(())
    }
    fn controls(
        &mut self,
        ctx: &egui::Context,
        storage: Option<&UserStorage>,
        active: Option<u64>,
    ) {
        if !self.settings_open {
            return;
        }
        let mut open = true;
        let mut next = self.preferences.clone();
        let mut history = None;
        egui::Window::new("Canvas display")
            .open(&mut open)
            .collapsible(false)
            .show(ctx, |ui| {
                ui.label("A separate canvas window follows the selected document.");
                egui::ComboBox::from_id_salt("display-monitor")
                    .selected_text(next.preferred.as_deref().unwrap_or("Primary screen (automatic)"))
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut next.preferred, None, "Primary screen (automatic)");
                        for monitor in &self.monitors {
                            ui.selectable_value(
                                &mut next.preferred,
                                Some(monitor.key.clone()),
                                format!("{} · {}×{} · {}%", monitor.key, monitor.size[0], monitor.size[1], monitor.scale * 100.0),
                            );
                        }
                    });
                if self.monitors.is_empty() {
                    ui.label("No screen information available. The editor remains accessible.");
                }
                if next.preferred.as_ref().is_some_and(|key| !self.monitors.iter().any(|monitor| &monitor.key == key)) {
                    ui.label("Preferred screen is unavailable; the view returns to an available screen. Reconnecting restores the preference.");
                }
                let sync = crate::help::checkbox(ui,&mut next.synced, "Sync zoom and center with editor","view");
                #[cfg(test)] { self.sync_button = Some(sync.rect); }
                #[cfg(not(test))] let _ = sync;
                crate::help::checkbox(ui,&mut next.fullscreen, "Fullscreen on selected screen","view");
                ui.add_enabled_ui(!next.synced, |ui| {
                    ui.horizontal(|ui| {
                        ui.label("Zoom");
                        let mut zoom = next.framing.zoom * 100.0;
                        if super::edit::number(ui,&mut zoom,1.0_f64..=6400.0_f64,1.0,"%","view").changed() {
                            next.framing.zoom = zoom / 100.0;
                        }
                        ui.label("Angle");
                        super::edit::number(ui,&mut next.framing.angle,-1e9_f64..=1e9_f64,0.5,"°","view");
                        next.framing.angle = next.framing.angle.rem_euclid(360.0);
                    });
                    crate::help::checkbox(ui,&mut next.framing.flipped, "Mirror view","view");
                    if crate::help::button(ui,"Reset independent view","view").clicked() {
                        next.framing = Default::default();
                    }
                });
                let visibility = crate::help::button(ui,if next.visible { "Close display view" } else { "Show display view" },"view");
                #[cfg(test)] { self.close_button = Some(visibility.rect); }
                if visibility.clicked() { next.visible = !next.visible; }
                ui.label("Independent view: wheel to zoom, middle/Space drag to pan. F11 toggles fullscreen; Escape returns to a window.");
                ui.horizontal_wrapped(|ui| {
                    let undo = ui.add_enabled(!self.undo.is_empty(), egui::Button::new("Undo display setting"));
                    let redo = ui.add_enabled(!self.redo.is_empty(), egui::Button::new("Redo display setting"));
                    #[cfg(test)] { self.undo_button = Some(undo.rect); self.redo_button = Some(redo.rect); }
                    if undo.clicked() { history = Some(false); }
                    if redo.clicked() { history = Some(true); }
                    if crate::help::button(ui,"Reset display settings","view").clicked() { next = Default::default(); }
                    if ui.add_enabled(storage.is_some(), egui::Button::new("Restore previous settings")).clicked() {
                        match Preferences::previous(storage.unwrap()) {
                            Ok(preferences) => next = preferences,
                            Err(error) => self.error = Some(error.to_string()),
                        }
                    }
                });
                if let Some(error) = &self.error { ui.colored_label(egui::Color32::LIGHT_RED, error); }
            });
        self.settings_open = open;
        let before = self.preferences.framing;
        let result = if let Some(redo) = history {
            let stack = if redo { &self.redo } else { &self.undo };
            if let Some(next) = stack.last().cloned() {
                let current = self.preferences.clone();
                let result = self.apply(next, storage, false);
                if result.is_ok() {
                    if redo {
                        self.redo.pop();
                        self.undo.push(current);
                    } else {
                        self.undo.pop();
                        self.redo.push(current);
                    }
                }
                result
            } else {
                Ok(())
            }
        } else {
            self.apply(next, storage, true)
        };
        if let Err(error) = result {
            self.error = Some(error);
        }
        if self.preferences.framing != before
            && let Some(id) = active
        {
            self.framing.insert(id, self.preferences.framing);
        }
    }
    fn present(
        &mut self,
        ctx: &egui::Context,
        document: Option<&Document>,
        storage: Option<&UserStorage>,
        hidden: bool,
    ) {
        if !self.preferences.visible {
            return;
        }
        let Some(monitor) =
            display_view::selected(&self.monitors, self.preferences.preferred.as_deref()).cloned()
        else {
            return;
        };
        let mut next = self.preferences.clone();
        let main_ppp = ctx.pixels_per_point() as f64;
        let framing = document.map(|doc| {
            if next.synced {
                Framing::capture(doc.view, [doc.width as f64, doc.height as f64], main_ppp)
            } else {
                *self.framing.entry(doc.id).or_insert(next.framing)
            }
        });
        if !next.synced
            && let Some(framing) = framing
        {
            next.framing = framing;
        }
        let placement = (monitor.clone(), next.fullscreen);
        let reposition = self.placed.as_ref() != Some(&placement);
        let id = egui::ViewportId::from_hash_of("drawer-canvas-display");
        let mut builder = egui::ViewportBuilder::default()
            .with_title(format!(
                "Drawer — {}",
                document.map_or("Canvas display", |d| d.title.as_str())
            ))
            .with_inner_size([800.0, 600.0])
            .with_min_inner_size([128.0, 128.0])
            .with_fullscreen(next.fullscreen)
            .with_visible(!hidden);
        builder.monitor = Some(monitor.index);
        ctx.show_viewport_immediate(id, builder, |ui, class| {
            if class != egui::ViewportClass::EmbeddedWindow {
                self.rendered_native = true;
                if ui.ctx().input(|i| i.viewport().fullscreen) == Some(false) {
                    self.native_windowed = true;
                }
            }
            self.rendered_frames += 1;
            let ppp = ui.ctx().pixels_per_point() as f64;
            if ui.ctx().input(|i| i.viewport().close_requested()) {
                next.visible = false;
            }
            if ui.ctx().input(|i| i.key_pressed(egui::Key::Escape)) {
                next.fullscreen = false;
            }
            if ui.ctx().input(|i| i.key_pressed(egui::Key::F11)) {
                next.fullscreen = !next.fullscreen;
            }
            if class != egui::ViewportClass::EmbeddedWindow
                && (reposition || next.fullscreen != placement.1)
            {
                ui.ctx()
                    .send_viewport_cmd(egui::ViewportCommand::SetMonitor(monitor.index));
                if !next.fullscreen {
                    ui.ctx()
                        .send_viewport_cmd(egui::ViewportCommand::Fullscreen(false));
                    let (position, size) = monitor.window();
                    ui.ctx()
                        .send_viewport_cmd(egui::ViewportCommand::OuterPosition(egui::pos2(
                            (position[0] / ppp) as f32,
                            (position[1] / ppp) as f32,
                        )));
                    ui.ctx()
                        .send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(
                            (size[0] / ppp) as f32,
                            (size[1] / ppp) as f32,
                        )));
                }
            }
            egui::CentralPanel::default().show(ui, |ui| {
                if let (Some(doc), Some(framing)) = (document, framing) {
                    let size = [doc.width as f64, doc.height as f64];
                    let mut view = framing.view(size, ppp);
                    let before = view;
                    self.canvas.show_only(ui, doc, &mut view, !next.synced);
                    if view != before {
                        next.framing = Framing::capture(view, size, ppp);
                    }
                } else {
                    ui.centered_and_justified(|ui| ui.label("Open a drawing in the editor"));
                }
            });
        });
        self.placed = Some((monitor, next.fullscreen));
        if let Err(error) = self.apply(next, storage, true) {
            self.error = Some(error);
        }
        if !self.preferences.synced
            && let Some(document) = document
        {
            self.framing.insert(document.id, self.preferences.framing);
        }
        if !self.preferences.visible {
            ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::Visible(true));
            ctx.send_viewport_cmd_to(
                egui::ViewportId::ROOT,
                egui::ViewportCommand::Minimized(false),
            );
        }
    }
}
impl Drawer {
    pub(super) fn show_display(&mut self, ctx: &egui::Context) {
        self.display.controls(
            ctx,
            self.storage.as_ref(),
            self.active.map(|i| self.documents[i].id),
        );
        self.display
            .framing
            .retain(|id, _| self.documents.iter().any(|d| d.id == *id));
        self.display.present(
            ctx,
            self.active.map(|i| &self.documents[i]),
            self.storage.as_ref(),
            self.smoke_output.is_some(),
        );
    }
}
