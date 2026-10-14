use super::*;
use drawer_core::aids::{self, AidPreferences, Unit};

pub(super) struct AidPanel {
    pub preferences: AidPreferences,
    saved: AidPreferences,
    pub visible: bool,
    target: Option<(u64, u64)>,
    pub spacing: [String; 2],
    pub origin: [String; 2],
    pub guide_position: String,
    pub measured_mm: String,
    error: Option<String>,
    #[cfg(test)]
    pub apply_button: Option<egui::Rect>,
    #[cfg(test)]
    pub guide_button: Option<egui::Rect>,
    #[cfg(test)]
    pub print_button: Option<egui::Rect>,
}
impl Default for AidPanel {
    fn default() -> Self {
        Self {
            preferences: AidPreferences::default(),
            saved: AidPreferences::default(),
            visible: false,
            target: None,
            spacing: ["64".into(), "64".into()],
            origin: ["0".into(), "0".into()],
            guide_position: "0".into(),
            measured_mm: String::new(),
            error: None,
            #[cfg(test)]
            apply_button: None,
            #[cfg(test)]
            guide_button: None,
            #[cfg(test)]
            print_button: None,
        }
    }
}
impl AidPanel {
    pub fn load(&mut self, storage: &UserStorage) {
        match AidPreferences::load(storage) {
            Ok(p) => {
                self.preferences = p;
                self.saved = p;
            }
            Err(e) => self.error = Some(e.to_string()),
        }
    }
    fn refresh(&mut self, doc: &Document) {
        let scale = doc.scene.physical_points(1.0, 72.0);
        let origin = doc.scene.transform.map(doc.scene.grid.origin);
        let unit = self.preferences.unit;
        self.spacing = [
            unit.from_pixels(doc.scene.grid.spacing[0] * scale[0], doc.dpi)
                .to_string(),
            unit.from_pixels(doc.scene.grid.spacing[1] * scale[1], doc.dpi)
                .to_string(),
        ];
        self.origin = origin.map(|v| unit.from_pixels(v, doc.dpi).to_string());
        self.target = Some((doc.id, doc.revision));
    }
}
impl Drawer {
    pub(super) fn apply_grid(&mut self) -> Result<(), String> {
        let index = self.active.ok_or("Open a drawing first")?;
        let doc = &mut self.documents[index];
        let unit = self.aids.preferences.unit;
        let number = |s: &str| {
            s.trim()
                .parse::<f64>()
                .map_err(|_| "Use a finite numeric value.".to_string())
        };
        let scale = doc.scene.physical_points(1.0, 72.0);
        let spacing = [
            unit.to_pixels(number(&self.aids.spacing[0])?, doc.dpi) / scale[0],
            unit.to_pixels(number(&self.aids.spacing[1])?, doc.dpi) / scale[1],
        ];
        let origin = [
            unit.to_pixels(number(&self.aids.origin[0])?, doc.dpi),
            unit.to_pixels(number(&self.aids.origin[1])?, doc.dpi),
        ];
        let mut scene = doc.scene.clone();
        scene.grid = aids::Grid {
            spacing,
            origin: scene.transform.inverse().map(origin),
        };
        doc.edit_scene(scene)?;
        Ok(())
    }
    pub(super) fn add_guide(&mut self, horizontal: bool) -> Result<(), String> {
        let index = self.active.ok_or("Open a drawing first")?;
        let doc = &mut self.documents[index];
        let value = self
            .aids
            .guide_position
            .trim()
            .parse::<f64>()
            .map_err(|_| "Use a numeric guide position.")?;
        let position = self.aids.preferences.unit.to_pixels(value, doc.dpi);
        if !position.is_finite() || position.abs() > 1e9 {
            return Err("The guide position is outside the supported range.".into());
        }
        let mut scene = doc.scene.clone();
        let line = if horizontal {
            [[0.0, position], [doc.width as f64, position]]
        } else {
            [[position, 0.0], [position, doc.height as f64]]
        };
        let inverse = scene.transform.inverse();
        let line = line.map(|p| inverse.map(p));
        if line
            .iter()
            .flatten()
            .any(|v| !v.is_finite() || v.abs() > 1e12)
        {
            return Err("Guide geometry exceeds the coordinate range.".into());
        }
        scene.guides.push(line);
        doc.edit_scene(scene)?;
        Ok(())
    }
    pub(super) fn print_size(&mut self) -> Result<(), String> {
        let index = self.active.ok_or("Open a drawing first")?;
        let ppi = self
            .aids
            .preferences
            .monitor_ppi
            .ok_or("Calibrate this display first")?;
        let doc = &mut self.documents[index];
        let before = doc.view;
        doc.view.zoom = aids::print_zoom(doc.dpi, ppi)
            .ok_or("Print size is outside the supported zoom range")?;
        doc.view.rotate(0.0);
        if let Some(storage) = &self.storage
            && let Err(error) = storage.save_view(doc.view)
        {
            doc.view = before;
            return Err(error.to_string());
        }
        Ok(())
    }
    pub(super) fn show_aids(&mut self, ctx: &egui::Context) {
        let enabled = self.pending_close.is_none() && self.sizing_dialog.is_none();
        let mut apply = false;
        let mut guide = None;
        let mut clear = false;
        let mut print = false;
        let mut calibrate = false;
        let mut restore = false;
        let mut reset = false;
        let mut open = self.aids.visible;
        if open {
            if let Some(index) = self.active
                && self.aids.target
                    != Some((self.documents[index].id, self.documents[index].revision))
            {
                self.aids.refresh(&self.documents[index]);
            }
            let panel = &mut self.aids;
            egui::Window::new("Canvas aids").open(&mut open).default_size(egui::vec2(430.0,450.0)).show(ctx,|ui| {
                ui.add_enabled_ui(enabled,|ui| {
                    ui.horizontal(|ui| {ui.checkbox(&mut panel.preferences.rulers,"Rulers");ui.checkbox(&mut panel.preferences.grid,"Grid");ui.checkbox(&mut panel.preferences.guides,"Guides");});
                    let old_unit=panel.preferences.unit;
                    ui.horizontal(|ui| {for unit in [Unit::Pixels,Unit::Millimeters,Unit::Inches] {ui.selectable_value(&mut panel.preferences.unit,unit,unit.label());}});
                    if let Some(index)=self.active {
                        let dpi=self.documents[index].dpi;
                        if old_unit!=panel.preferences.unit {for text in panel.spacing.iter_mut().chain(&mut panel.origin).chain(std::iter::once(&mut panel.guide_position)) {if let Ok(value)=text.parse::<f64>() {*text=panel.preferences.unit.from_pixels(old_unit.to_pixels(value,dpi),dpi).to_string();}}}
                        ui.label("Grid spacing and origin are saved in the drawing.");
                        egui::Grid::new("grid-aid-fields").show(ui,|ui| {for (index,value) in panel.spacing.iter_mut().enumerate() {ui.label(["Horizontal spacing","Vertical spacing"][index]);ui.text_edit_singleline(value);ui.end_row();}});
                        ui.horizontal(|ui| {ui.label("Origin");for text in &mut panel.origin {ui.add(egui::TextEdit::singleline(text).desired_width(90.0));}});
                        let button=ui.button("Apply grid");apply=button.clicked();#[cfg(test)] {panel.apply_button=Some(button.rect);}
                        ui.horizontal_wrapped(|ui| {ui.label("Guide position");ui.add(egui::TextEdit::singleline(&mut panel.guide_position).desired_width(80.0));let button=ui.button("Add horizontal guide");if button.clicked() {guide=Some(true);}#[cfg(test)] {panel.guide_button=Some(button.rect);}if ui.button("Add vertical guide").clicked() {guide=Some(false);}clear=ui.button("Remove all guides").clicked();});
                    } else {ui.label("Open a drawing to edit grid and guides.");}
                    ui.separator();ui.label("Measure the line below on this display, then enter its length in mm.");
                    let (rect,_)=ui.allocate_exact_size(egui::vec2(200.0,20.0),egui::Sense::hover());ui.painter().line_segment([rect.left_center(),rect.right_center()],egui::Stroke::new(2.0,egui::Color32::WHITE));
                    ui.horizontal(|ui| {ui.label("Measured mm");ui.add(egui::TextEdit::singleline(&mut panel.measured_mm).desired_width(80.0));calibrate=ui.button("Calibrate display").clicked();});
                    ui.label("Physical screen size depends on your measurement. Calibrate again on a different display.");
                    if let Some(ppi)=panel.preferences.monitor_ppi {ui.label(format!("Display calibration: {ppi:.2} pixels per inch"));}
                    let button=ui.add_enabled(self.active.is_some() && panel.preferences.monitor_ppi.is_some(),egui::Button::new("Show print size"));print=button.clicked();#[cfg(test)] {panel.print_button=Some(button.rect);}
                    if ui.button("Reset display settings").clicked() {panel.preferences=AidPreferences::default();panel.target=None;reset=true;}
                    restore=ui.button("Restore previous display settings").clicked();
                    if let Some(error)=&panel.error {ui.colored_label(egui::Color32::LIGHT_RED,error);}
                });
            });
        }
        self.aids.visible = open;
        let result = (|| {
            if restore {
                self.aids.preferences = AidPreferences::previous(
                    self.storage.as_ref().ok_or("User storage is unavailable")?,
                )
                .map_err(|e| e.to_string())?;
                self.aids.target = None;
            }
            if apply {
                self.apply_grid()?;
            }
            if let Some(horizontal) = guide {
                self.add_guide(horizontal)?;
            }
            if clear && let Some(index) = self.active {
                let mut scene = self.documents[index].scene.clone();
                scene.guides.clear();
                self.documents[index].edit_scene(scene)?;
            }
            if calibrate {
                let mm = self
                    .aids
                    .measured_mm
                    .trim()
                    .parse::<f64>()
                    .map_err(|_| "Enter the measured length in millimeters")?;
                self.aids.preferences.monitor_ppi = Some(
                    aids::calibrated_ppi(200.0, ctx.pixels_per_point() as f64, mm)
                        .ok_or("Invalid calibration length or unsupported display density")?,
                );
            }
            if print {
                self.print_size()?;
            }
            if (self.aids.preferences != self.aids.saved || reset)
                && let Some(storage) = &self.storage
            {
                self.aids
                    .preferences
                    .save(storage)
                    .map_err(|e| e.to_string())?;
                self.aids.saved = self.aids.preferences;
            }
            Ok::<_, String>(())
        })();
        if let Err(error) = result {
            self.aids.error = Some(error);
        } else if apply || guide.is_some() || print || calibrate || clear || restore || reset {
            self.aids.error = None;
        }
    }
}
