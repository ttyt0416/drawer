use super::*;
use drawer_core::interpolation::{Filter, Settings};
impl Drawer {
    pub(super) fn apply_quality(&mut self, next: Settings) -> Result<(), String> {
        if let Some(storage) = &self.storage {
            next.save(storage).map_err(|error| error.to_string())?;
        }
        if next != self.interpolation {
            self.quality_undo.push(self.interpolation);
            if self.quality_undo.len() > 100 {
                self.quality_undo.remove(0);
            }
            self.quality_redo.clear();
            self.interpolation = next;
        }
        Ok(())
    }

    pub(super) fn interpolation_settings(&mut self, ctx: &egui::Context) {
        if !self.show_interpolation {
            return;
        }
        let mut open = true;
        let mut next = self.interpolation;
        let mut history = None;
        egui::Window::new("Image interpolation").open(&mut open).show(ctx,|ui| {
            ui.label("View changes leave saved pixels unchanged. Editing operations use their own settings.");
            egui::Grid::new("quality-scopes").show(ui,|ui| {
                for (name,filter) in [("Canvas view",&mut next.view),("Resolution changes",&mut next.resolution),("Raster object transforms",&mut next.object),("Animation image sampling",&mut next.animation)] {
                    ui.label(name);egui::ComboBox::from_id_salt(name).selected_text(filter.label()).show_ui(ui,|ui| {for f in Filter::ALL {ui.selectable_value(filter,f,f.label());}});ui.end_row();
                }
            });
            ui.label("Average colors integrates the source pixel footprint. Oversampling takes a 4×4 grid of bilinear samples.");
            ui.label("Object transforms and animation keep separate interpolation choices.");
            if crate::help::button(ui,"Reset interpolation","view").clicked() {next=Default::default();}
            if ui.add_enabled(!self.quality_undo.is_empty(),egui::Button::new("Undo interpolation setting")).clicked() {history=Some(false);}
            if ui.add_enabled(!self.quality_redo.is_empty(),egui::Button::new("Redo interpolation setting")).clicked() {history=Some(true);}
            if ui.add_enabled(self.storage.is_some(),egui::Button::new("Restore previous interpolation")).clicked() {
                match Settings::read(&self.storage.as_ref().unwrap().root().join("settings/interpolation.previous.json")) {Ok(s)=>next=s,Err(e)=>self.status=Some(e.to_string()),}
            }
        });
        self.show_interpolation = open;
        if let Some(redo) = history {
            let mut undo = self.quality_undo.clone();
            let mut future = self.quality_redo.clone();
            if let Some(next) = if redo { future.pop() } else { undo.pop() } {
                let current = self.interpolation;
                match self.apply_quality(next) {
                    Ok(()) => {
                        if redo {
                            undo.push(current);
                        } else {
                            future.push(current);
                        }
                        self.quality_undo = undo;
                        self.quality_redo = future;
                        ctx.request_repaint();
                    }
                    Err(error) => self.status = Some(error),
                }
            }
        } else if next != self.interpolation {
            match self.apply_quality(next) {
                Ok(()) => ctx.request_repaint(),
                Err(error) => self.status = Some(error),
            }
        }
    }
}
