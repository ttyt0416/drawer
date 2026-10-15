use super::*;
use drawer_core::ui_scale::{self, Settings};
pub(super) struct Interface {
    pub settings: Settings,
    pub open: bool,
    pub ppp: f64,
    saved: Option<Settings>,
    undo: Vec<f32>,
    redo: Vec<f32>,
}
impl Interface {
    pub fn new(ctx: &egui::Context) -> Self {
        Self {
            settings: Default::default(),
            open: false,
            ppp: ctx.pixels_per_point() as f64,
            saved: None,
            undo: Vec::new(),
            redo: Vec::new(),
        }
    }
}
impl Drawer {
    pub(super) fn observe_scale(&mut self, ctx: &egui::Context) {
        let after = ctx.pixels_per_point() as f64;
        let before = self.interface.ppp;
        if after != before {
            self.sync_active_view();
            for tab in &mut self.tabs {
                ui_scale::rebase(&mut tab.view, before, after);
            }
            for doc in &mut self.documents {
                ui_scale::rebase(&mut doc.view, before, after);
            }
            ui_scale::rebase(&mut self.default_view, before, after);
            self.interface.ppp = after;
        }
    }
    pub(super) fn persist_interface(&mut self) {
        if let Some(n) = self.active {
            let mut view = self.documents[n].view;
            view.pan = view.pan.map(|p| p * self.interface.ppp);
            self.interface.settings.default_view = Some(view);
        }
        if self.interface.saved != Some(self.interface.settings) {
            if let Some(storage) = &self.storage
                && let Err(e) = self.interface.settings.save(storage)
            {
                self.status = Some(e.to_string());
                return;
            }
            self.interface.saved = Some(self.interface.settings);
        }
    }
    pub(super) fn interface_zoom(&mut self, ctx: &egui::Context, zoom: f32) -> Result<(), String> {
        let next = Settings {
            zoom,
            ..self.interface.settings
        };
        if !next.valid() {
            return Err("Interface size must be 50–200%".into());
        }
        if let Some(storage) = &self.storage {
            next.save(storage).map_err(|e| e.to_string())?;
        }
        if zoom != self.interface.settings.zoom {
            self.interface.undo.push(self.interface.settings.zoom);
            if self.interface.undo.len() > 100 {
                self.interface.undo.remove(0);
            }
            self.interface.redo.clear();
        }
        self.interface.settings = next;
        self.interface.saved = Some(next);
        ctx.set_zoom_factor(zoom);
        Ok(())
    }
    pub(super) fn interface_history(
        &mut self,
        ctx: &egui::Context,
        redo: bool,
    ) -> Result<(), String> {
        let Some(zoom) = if redo {
            self.interface.redo.last()
        } else {
            self.interface.undo.last()
        }
        .copied() else {
            return Ok(());
        };
        let mut undo = self.interface.undo.clone();
        let mut future = self.interface.redo.clone();
        let current = self.interface.settings.zoom;
        self.interface_zoom(ctx, zoom)?;
        if redo {
            future.pop();
            undo.push(current);
        } else {
            undo.pop();
            future.push(current);
        }
        self.interface.undo = undo;
        self.interface.redo = future;
        Ok(())
    }
    pub(super) fn interface_settings(&mut self, ctx: &egui::Context) {
        if !self.interface.open {
            return;
        }
        let mut open = true;
        let mut zoom = self.interface.settings.zoom;
        let mut history = None;
        egui::Window::new("Interface size").open(&mut open).show(ctx,|ui| {
            ui.label("Monitor scaling is automatic. This setting adds your preferred interface size.");
            super::edit::slider(ui,&mut zoom,0.5..=2.0,"Interface scale","view");
            ui.label(format!("Current screen scale: {:.0}%",ctx.pixels_per_point()*100.0));
            if crate::help::button(ui,"Reset interface size","view").clicked() {zoom=1.0;}
            if ui.add_enabled(!self.interface.undo.is_empty(),egui::Button::new("Undo interface setting")).clicked() {history=Some(false);}
            if ui.add_enabled(!self.interface.redo.is_empty(),egui::Button::new("Redo interface setting")).clicked() {history=Some(true);}
            if ui.add_enabled(self.storage.is_some(),egui::Button::new("Restore previous interface size")).clicked() {match Settings::read(&self.storage.as_ref().unwrap().root().join("settings/interface.previous.json")) {Ok(s)=>zoom=s.zoom,Err(e)=>self.status=Some(e.to_string())}}
        });
        self.interface.open = open;
        let result = if let Some(redo) = history {
            self.interface_history(ctx, redo)
        } else if zoom != self.interface.settings.zoom {
            self.interface_zoom(ctx, zoom)
        } else {
            Ok(())
        };
        if let Err(e) = result {
            self.status = Some(e);
        }
    }
}
