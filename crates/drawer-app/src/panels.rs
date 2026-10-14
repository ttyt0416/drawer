use super::*;
use drawer_core::{
    panels::{Bar, Kind, Settings, Thumbnail},
    thumbnail::Target,
};
use std::collections::HashMap;
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct ThumbnailKey {
    document: u64,
    revision: u64,
    frame: u32,
    target: Target,
    size: usize,
    content: bool,
    checker: bool,
}
pub(super) struct Panels {
    pub settings: Settings,
    saved: Option<Settings>,
    undo: Vec<Settings>,
    redo: Vec<Settings>,
    pub open: bool,
    kind: Kind,
    pub targets: HashMap<u64, Target>,
    thumbnails: HashMap<ThumbnailKey, egui::TextureHandle>,
    strokes: Vec<egui::TextureHandle>,
    previous_color: [u8; 4],
    #[cfg(test)]
    pub targets_buttons: HashMap<Target, egui::Rect>,
}
impl Panels {
    pub fn new(ctx: &egui::Context) -> Self {
        let strokes = BrushTip::ALL
            .iter()
            .map(|tip| {
                let mask = tip.mask(16);
                let mut pixels = vec![egui::Color32::TRANSPARENT; 64 * 20];
                for y in 0..20 {
                    for x in 0..64 {
                        let alpha = (8..=56)
                            .filter_map(|stamp| {
                                let mx = x as isize - stamp + 8;
                                let my = y as isize - 2;
                                if (0..16).contains(&mx) && (0..16).contains(&my) {
                                    Some(mask[my as usize * 16 + mx as usize])
                                } else {
                                    None
                                }
                            })
                            .max()
                            .unwrap_or(0);
                        pixels[y * 64 + x] = egui::Color32::from_white_alpha(alpha);
                    }
                }
                ctx.load_texture(
                    format!("{} stroke preview", tip.name()),
                    egui::ColorImage::new([64, 20], pixels),
                    egui::TextureOptions::LINEAR,
                )
            })
            .collect();
        Self {
            settings: Default::default(),
            saved: None,
            undo: Vec::new(),
            redo: Vec::new(),
            open: false,
            kind: Kind::Brush,
            targets: HashMap::new(),
            thumbnails: HashMap::new(),
            strokes,
            previous_color: [0, 0, 0, 255],
            #[cfg(test)]
            targets_buttons: HashMap::new(),
        }
    }
    pub fn persist(&mut self, storage: Option<&UserStorage>) -> Result<(), String> {
        if self.saved != Some(self.settings) {
            if let Some(storage) = storage {
                self.settings.save(storage).map_err(|e| e.to_string())?;
            }
            if let Some(previous) = self.saved {
                self.undo.push(previous);
                if self.undo.len() > 100 {
                    self.undo.remove(0);
                }
                self.redo.clear();
            }
            self.saved = Some(self.settings);
        }
        Ok(())
    }
    pub fn history(&mut self, redo: bool, storage: Option<&UserStorage>) -> Result<(), String> {
        let Some(next) = if redo {
            self.redo.last()
        } else {
            self.undo.last()
        }
        .copied() else {
            return Ok(());
        };
        if let Some(storage) = storage {
            next.save(storage).map_err(|e| e.to_string())?;
        }
        if redo {
            self.redo.pop();
            self.undo.push(self.settings);
        } else {
            self.undo.pop();
            self.redo.push(self.settings);
        }
        self.settings = next;
        self.saved = Some(next);
        Ok(())
    }
    fn texture(
        &mut self,
        ctx: &egui::Context,
        document: &Document,
        target: Target,
        kind: Kind,
    ) -> Option<egui::TextureHandle> {
        let cfg = self.settings.panels[kind.index()];
        let size = cfg.thumbnail.size();
        if size == 0 {
            return None;
        }
        let key = ThumbnailKey {
            document: document.id,
            revision: document.revision,
            frame: document.current_frame,
            target,
            size,
            content: cfg.content,
            checker: cfg.checker,
        };
        if let Some(texture) = self.thumbnails.get(&key) {
            return Some(texture.clone());
        }
        let pixels =
            drawer_core::thumbnail::image(document, target, size, cfg.content, cfg.checker).ok()?;
        let image = egui::ColorImage::new(
            [size, size],
            pixels
                .into_iter()
                .map(|p| egui::Color32::from_rgba_unmultiplied(p[0], p[1], p[2], p[3]))
                .collect(),
        );
        if self.thumbnails.len() >= 512 {
            self.thumbnails.clear();
        }
        let texture = ctx.load_texture("panel-thumbnail", image, egui::TextureOptions::NEAREST);
        self.thumbnails.insert(key, texture.clone());
        Some(texture)
    }
}
impl Drawer {
    pub(super) fn basic_shortcuts(&mut self, ctx: &egui::Context) {
        if self.pending_close.is_some()
            || self.sizing_dialog.is_some()
            || self.import_dialog.is_some()
            || self.paste_dialog.is_some()
            || self.movie_dialog.is_some()
            || self.revert_pending.is_some()
            || self.show_new_canvas
            || ctx.egui_wants_keyboard_input()
        {
            return;
        }
        let action = ctx.input_mut(shortcut_action);
        match action {
            Some(0 | 1) => {
                if let Some(n) = self.active {
                    let result = if action == Some(0) {
                        self.save_document(n)
                    } else {
                        self.save_as(n)
                    };
                    if let Err(e) = result {
                        self.status = Some(e);
                    }
                }
            }
            Some(2) => {
                if let Some(n) = self.active {
                    self.documents[n].undo();
                }
            }
            Some(3) => {
                if let Some(n) = self.active {
                    self.documents[n].redo();
                }
            }
            Some(n @ 4..=7) => self.set_tool(BrushTip::ALL[(n - 4) as usize]),
            _ => {}
        }
    }
    pub(super) fn show_panels(&mut self, ui: &mut egui::Ui) {
        let height = (ui.available_height() * 0.58).clamp(60.0, 360.0);
        egui::ScrollArea::vertical()
            .id_salt("workspace-panels")
            .max_height(height)
            .show(ui, |ui| {
                for kind in Kind::ALL {
                    let cfg = self.panels.settings.panels[kind.index()];
                    if !cfg.visible {
                        continue;
                    }
                    ui.push_id(kind.index(), |ui| {
                        ui.heading(kind.label());
                        if cfg.properties == Bar::Top {
                            self.panel_properties(ui, kind);
                        }
                        if cfg.commands == Bar::Top {
                            self.panel_commands(ui, kind);
                        }
                        match kind {
                            Kind::Brush => {
                                for (n, tip) in BrushTip::ALL.into_iter().enumerate() {
                                    ui.horizontal(|ui| {
                                        let label = if cfg.names {
                                            format!("{}  {}", tip.name(), tool_key(tip))
                                        } else {
                                            String::new()
                                        };
                                        let response =
                                            ui.selectable_label(self.selected == tip, label);
                                        response.widget_info(|| {
                                            egui::WidgetInfo::selected(
                                                egui::WidgetType::SelectableLabel,
                                                ui.is_enabled(),
                                                self.selected == tip,
                                                tip.name(),
                                            )
                                        });
                                        let response = response.on_hover_text(format!(
                                            "{} ({})",
                                            tip.name(),
                                            tool_key(tip)
                                        ));
                                        let mut clicked = response.clicked();
                                        if cfg.icons {
                                            clicked |= ui
                                                .add(
                                                    egui::Image::new(&self.previews[n])
                                                        .fit_to_exact_size(egui::vec2(16.0, 16.0))
                                                        .sense(egui::Sense::click()),
                                                )
                                                .on_hover_text(tip.name())
                                                .clicked();
                                        }
                                        if cfg.strokes {
                                            clicked |= ui
                                                .add(
                                                    egui::Image::new(&self.panels.strokes[n])
                                                        .fit_to_exact_size(egui::vec2(64.0, 20.0))
                                                        .sense(egui::Sense::click()),
                                                )
                                                .on_hover_text(tip.name())
                                                .clicked();
                                        }
                                        if clicked {
                                            self.set_tool(tip);
                                        }
                                    });
                                }
                            }
                            Kind::Color => {
                                let mut color = self.references.session.workspace.color;
                                if ui
                                    .color_edit_button_srgba_unmultiplied(&mut color)
                                    .changed()
                                {
                                    self.panels.previous_color =
                                        self.references.session.workspace.color;
                                    self.references.session.workspace.color = color;
                                }
                            }
                            Kind::Layers | Kind::Timeline => self.panel_layers(ui, kind),
                        }
                        if cfg.properties == Bar::Bottom {
                            self.panel_properties(ui, kind);
                        }
                        if cfg.commands == Bar::Bottom {
                            self.panel_commands(ui, kind);
                        }
                        ui.separator();
                    });
                }
            });
    }
    fn panel_layers(&mut self, ui: &mut egui::Ui, kind: Kind) {
        let Some(n) = self.active else {
            ui.label("No document open");
            return;
        };
        let doc = &self.documents[n];
        let id = doc.id;
        let selected = *self.panels.targets.entry(id).or_insert(Target::Raster(0));
        let cfg = self.panels.settings.panels[kind.index()];
        let mut rows = Vec::new();
        if kind != Kind::Timeline || !cfg.animation_only {
            for (index, layer) in doc.layers.iter().enumerate() {
                rows.push((Target::Raster(index), layer.name.clone()));
            }
        }
        if let Some(animation) = &doc.animation {
            for (index, track) in animation.tracks.iter().enumerate() {
                rows.push((
                    Target::Track(index),
                    format!("{} · {} cells", track.name, track.cells.len()),
                ));
            }
        }
        if rows.is_empty() {
            ui.label("No animation tracks");
        }
        let row_height = cfg.thumbnail.size().max(22) as f32;
        egui::ScrollArea::vertical()
            .id_salt("panel-rows")
            .max_height(140.0)
            .show_rows(ui, row_height, rows.len(), |ui, range| {
                for index in range {
                    let (target, label) = &rows[index];
                    ui.horizontal(|ui| {
                        let texture =
                            self.panels
                                .texture(ui.ctx(), &self.documents[n], *target, kind);
                        let image_clicked = texture.is_some_and(|texture| {
                            ui.add(
                                egui::Image::new(&texture)
                                    .fit_to_exact_size(egui::vec2(
                                        cfg.thumbnail.size() as f32,
                                        cfg.thumbnail.size() as f32,
                                    ))
                                    .sense(egui::Sense::click()),
                            )
                            .clicked()
                        });
                        let response = ui.selectable_label(selected == *target, label);
                        #[cfg(test)]
                        {
                            self.panels.targets_buttons.insert(*target, response.rect);
                        }
                        if response.clicked() || image_clicked {
                            self.panels.targets.insert(id, *target);
                        }
                    });
                }
            });
        if !rows.iter().any(|(target, _)| *target == selected) {
            ui.label("Selection is outside this list.");
        }
    }
    fn panel_properties(&mut self, ui: &mut egui::Ui, kind: Kind) {
        match kind {
            Kind::Brush => {
                ui.horizontal(|ui| {
                    ui.label("Size");
                    ui.add(
                        egui::DragValue::new(&mut self.panels.settings.brush_size)
                            .range(0.1..=4096.0)
                            .suffix(" px"),
                    );
                });
                ui.horizontal(|ui| {
                    ui.label("Aspect");
                    ui.add(
                        egui::DragValue::new(&mut self.panels.settings.brush_aspect)
                            .range(0.05..=1.0)
                            .speed(0.01),
                    );
                });
                ui.horizontal(|ui| {
                    ui.label("Tip angle");
                    ui.add(
                        egui::DragValue::new(&mut self.panels.settings.brush_angle)
                            .speed(1.0)
                            .suffix("°"),
                    );
                });
                self.panels.settings.brush_angle =
                    self.panels.settings.brush_angle.rem_euclid(360.0);
            }
            Kind::Color => {
                let [r, g, b, a] = self.references.session.workspace.color;
                ui.label(format!("#{r:02X}{g:02X}{b:02X}{a:02X}"));
            }
            Kind::Layers | Kind::Timeline => {
                if let Some(n) = self.active {
                    let doc = &self.documents[n];
                    let target = self
                        .panels
                        .targets
                        .get(&doc.id)
                        .copied()
                        .unwrap_or(Target::Raster(0));
                    let label = match target {
                        Target::Raster(n) => doc.layers.get(n).map(|layer| {
                            let e = layer.extent(doc.width, doc.height);
                            format!(
                                "{} × {} · {}, {}",
                                e.width, e.height, e.offset[0], e.offset[1]
                            )
                        }),
                        Target::Track(n) => doc
                            .animation
                            .as_ref()
                            .and_then(|a| a.tracks.get(n))
                            .map(|track| {
                                format!("{} · {} exposures", track.name, track.exposures.len())
                            }),
                    };
                    ui.label(label.unwrap_or_else(|| "Selected item unavailable".into()));
                }
            }
        }
    }
    fn panel_commands(&mut self, ui: &mut egui::Ui, kind: Kind) {
        ui.horizontal_wrapped(|ui| match kind {
            Kind::Brush => {
                for tip in BrushTip::ALL {
                    if ui
                        .small_button(tool_key(tip))
                        .on_hover_text(tip.name())
                        .clicked()
                    {
                        self.set_tool(tip);
                    }
                }
            }
            Kind::Color => {
                if ui.small_button("Previous color").clicked() {
                    std::mem::swap(
                        &mut self.panels.previous_color,
                        &mut self.references.session.workspace.color,
                    );
                }
                if ui.small_button("Black").clicked() {
                    self.panels.previous_color = self.references.session.workspace.color;
                    self.references.session.workspace.color = [0, 0, 0, 255];
                }
            }
            Kind::Layers | Kind::Timeline => {
                if let Some(n) = self.active {
                    let doc = &mut self.documents[n];
                    if ui.small_button("Top raster").clicked() {
                        self.panels
                            .targets
                            .insert(doc.id, Target::Raster(doc.layers.len() - 1));
                    }
                    if ui.small_button("Center selection").clicked() {
                        let target = self
                            .panels
                            .targets
                            .get(&doc.id)
                            .copied()
                            .unwrap_or(Target::Raster(0));
                        let bounds = drawer_core::thumbnail::bounds(doc, target, true);
                        doc.view.center_on(
                            [
                                (bounds.min[0] + bounds.max[0]) / 2.0,
                                (bounds.min[1] + bounds.max[1]) / 2.0,
                            ],
                            [doc.width as f64, doc.height as f64],
                            [0.0; 2],
                            ui.ctx().pixels_per_point() as f64,
                        );
                    }
                }
            }
        });
    }
    pub(super) fn panel_settings(&mut self, ctx: &egui::Context) {
        if !self.panels.open {
            return;
        }
        let mut open = true;
        let mut history = None;
        egui::Window::new("Panels and shortcuts").open(&mut open).show(ctx,|ui| {
            egui::ComboBox::from_id_salt("panel-kind").selected_text(self.panels.kind.label()).show_ui(ui,|ui|{for kind in Kind::ALL {ui.selectable_value(&mut self.panels.kind,kind,kind.label());}});
            let cfg=&mut self.panels.settings.panels[self.panels.kind.index()];ui.checkbox(&mut cfg.visible,"Show panel");
            if self.panels.kind==Kind::Brush {ui.checkbox(&mut cfg.icons,"Tool icons");ui.checkbox(&mut cfg.names,"Tool names");ui.checkbox(&mut cfg.strokes,"Stroke previews");}
            if matches!(self.panels.kind,Kind::Layers|Kind::Timeline) {
                egui::ComboBox::from_id_salt("panel-thumb").selected_text(cfg.thumbnail.label()).show_ui(ui,|ui| {for size in Thumbnail::ALL {ui.selectable_value(&mut cfg.thumbnail,size,size.label());}});
                ui.checkbox(&mut cfg.content,"Show content bounds instead of whole canvas");ui.checkbox(&mut cfg.checker,"Transparency checker");
                if self.panels.kind==Kind::Timeline {ui.checkbox(&mut cfg.animation_only,"Animation tracks only");}
            }
            for (label,bar) in [("Properties bar",&mut cfg.properties),("Commands bar",&mut cfg.commands)] {egui::ComboBox::from_id_salt(label).selected_text(format!("{label}: {}",bar.label())).show_ui(ui,|ui|{for value in Bar::ALL {ui.selectable_value(bar,value,value.label());}});}
            ui.separator();ui.label("Save Ctrl+S · Save as Ctrl+Shift+S · Undo Ctrl+Z · Redo Ctrl+Y / Ctrl+Shift+Z · Pen B · Pencil N · Eraser E · Wash W");
            if ui.button("Restore all panels").clicked() {self.panels.settings=Settings::default();}
            if ui.add_enabled(!self.panels.undo.is_empty(),egui::Button::new("Undo panel setting")).clicked() {history=Some(false);}
            if ui.add_enabled(!self.panels.redo.is_empty(),egui::Button::new("Redo panel setting")).clicked() {history=Some(true);}
            if ui.add_enabled(self.storage.is_some(),egui::Button::new("Restore previous panels")).clicked() {match Settings::read(&self.storage.as_ref().unwrap().root().join("settings/panels.previous.json")) {Ok(s)=>self.panels.settings=s,Err(e)=>self.status=Some(e.to_string())}}
        });
        self.panels.open = open;
        if let Some(redo) = history
            && let Err(e) = self.panels.history(redo, self.storage.as_ref())
        {
            self.status = Some(e);
        }
    }
}
pub(super) fn tool_key(tip: BrushTip) -> &'static str {
    match tip {
        BrushTip::Pen => "B",
        BrushTip::Pencil => "N",
        BrushTip::Eraser => "E",
        BrushTip::Wash => "W",
    }
}

pub(super) fn shortcut_action(input: &mut egui::InputState) -> Option<u8> {
    let binding = input.events.iter().enumerate().find_map(|(index, event)| {
        let egui::Event::Key {
            key,
            pressed: true,
            modifiers,
            ..
        } = event
        else {
            return None;
        };
        // On Windows command mirrors Control; retain actual modifier specificity.
        let normalized = egui::Modifiers {
            command: false,
            ..*modifiers
        };
        let bindings = [
            (
                egui::Modifiers::CTRL | egui::Modifiers::SHIFT,
                egui::Key::S,
                1,
            ),
            (egui::Modifiers::CTRL, egui::Key::S, 0),
            (
                egui::Modifiers::CTRL | egui::Modifiers::SHIFT,
                egui::Key::Z,
                3,
            ),
            (egui::Modifiers::CTRL, egui::Key::Y, 3),
            (egui::Modifiers::CTRL, egui::Key::Z, 2),
            (egui::Modifiers::NONE, egui::Key::B, 4),
            (egui::Modifiers::NONE, egui::Key::N, 5),
            (egui::Modifiers::NONE, egui::Key::E, 6),
            (egui::Modifiers::NONE, egui::Key::W, 7),
        ];
        bindings
            .into_iter()
            .find_map(|(required, expected, action)| {
                (*key == expected && normalized.matches_exact(required)).then_some((index, action))
            })
    });
    if let Some((index, action)) = binding {
        input.events.remove(index);
        Some(action)
    } else {
        None
    }
}
