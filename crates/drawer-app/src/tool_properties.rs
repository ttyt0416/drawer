use super::*;
use drawer_core::numeric::{Range, Scalar, Unit};
use drawer_core::tool_properties::{Control, CopyKind, Preview, Property, Settings, Tool};
fn property_value(
    ui: &mut egui::Ui,
    value: &mut f64,
    property: Property,
    control: Control,
    size_unit: Unit,
    dpi: f64,
) -> Option<egui::Response> {
    let [min, max] = property.bounds();
    let mut scalar = match Scalar::new(*value, Range { min, max }) {
        Ok(value) => value,
        Err(e) => {
            ui.colored_label(egui::Color32::LIGHT_RED, e);
            return None;
        }
    };
    let unit = match property {
        Property::Size => size_unit,
        Property::Opacity | Property::Hardness | Property::Stabilization => Unit::Percent,
        _ => Unit::Scalar,
    };
    let (mut display, range) = match scalar.display(unit, dpi) {
        Ok(result) => result,
        Err(e) => {
            ui.colored_label(egui::Color32::LIGHT_RED, e);
            return None;
        }
    };
    let step = if matches!(property, Property::Size | Property::Angle) {
        1.0
    } else {
        0.01
    };
    let factor = unit.factor(dpi).unwrap();
    ui.horizontal(|ui| {
        let suffix = if property == Property::Angle {
            "°"
        } else {
            unit.label()
        };
        ui.label(format!("{} ({suffix})", property.name()));
        let response = match control {
            Control::Slider => Some(super::edit::slider_with_step(
                ui,
                &mut display,
                range.min..=range.max,
                "",
                "tool-properties",
                step / factor,
            )),
            Control::Number => Some(super::edit::number(
                ui,
                &mut display,
                range.min..=range.max,
                step / factor,
                "",
                "tool-properties",
            )),
            Control::Indicator => {
                ui.label(display.to_string());
                None
            }
        };
        if response.as_ref().is_some_and(|r| r.changed()) {
            match scalar.set_display(display, unit, dpi) {
                Ok(true) => *value = scalar.value(),
                Ok(false) => {}
                Err(e) => {
                    ui.colored_label(egui::Color32::LIGHT_RED, e);
                }
            }
        }
        response
    })
    .inner
}
/// Exercise the production widgets in the installed process without external UI control.
pub(super) fn smoke() -> Result<(), String> {
    for unit in Unit::SIZE {
        for control in [Control::Number, Control::Slider] {
            let ctx = egui::Context::default();
            let mut value = 300.0;
            let mut id = egui::Id::NULL;
            let mut draw = |events| {
                let mut out = ctx.run_ui(
                    egui::RawInput {
                        events,
                        ..Default::default()
                    },
                    |ui| {
                        id = property_value(ui, &mut value, Property::Size, control, unit, 300.0)
                            .unwrap()
                            .id;
                    },
                );
                out.textures_delta.clear();
                (id, value)
            };
            let (id, initial) = draw(vec![]);
            if initial != 300.0 {
                return Err("Unit display changed canonical size".into());
            }
            ctx.memory_mut(|m| m.request_focus(id));
            draw(vec![]);
            let key = if control == Control::Number {
                egui::Key::ArrowUp
            } else {
                egui::Key::ArrowRight
            };
            let (_, next) = draw(vec![egui::Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Default::default(),
            }]);
            if (next - 301.0).abs() > 1e-9 {
                return Err(format!(
                    "Unit/widget increment diverged: {unit:?}/{control:?}: {next}"
                ));
            }
        }
    }
    Ok(())
}
fn size_unit(ui: &mut egui::Ui, unit: &mut Unit, dpi: f64) {
    let valid = !super::edit::invalid_layer(ui.ctx(), ui.layer_id());
    ui.add_enabled_ui(valid,|ui| {
        egui::ComboBox::from_id_salt("brush-size-unit").selected_text(format!("Size unit: {}",unit.label())).show_ui(ui,|ui| {
            for value in Unit::SIZE {ui.selectable_value(unit,value,value.label());}
        }).response.on_hover_text(format!("Physical size uses the active document DPI ({dpi}); canonical size stays in document pixels"));
    });
}
struct Editor {
    draft: Settings,
    source: Tool,
    scope: u8,
    targets: [bool; 6],
    properties: [bool; 7],
    kind: CopyKind,
    preview: Option<Preview>,
    reset_preview: bool,
    hide_reset: bool,
    factory: std::collections::HashSet<BrushTip>,
    pending_factory: Option<BrushTip>,
}
impl Editor {
    fn new(draft: Settings, source: Tool) -> Self {
        Self {
            draft,
            source,
            scope: 0,
            targets: [false; 6],
            properties: [true; 7],
            kind: CopyKind::Values,
            preview: None,
            reset_preview: false,
            hide_reset: false,
            factory: Default::default(),
            pending_factory: None,
        }
    }
}
#[derive(Default)]
pub(super) struct Properties {
    pub open: bool,
    editor: Option<Editor>,
    applied: Option<BrushTip>,
    #[cfg(test)]
    apply: Option<egui::Rect>,
    #[cfg(test)]
    cancel: Option<egui::Rect>,
    #[cfg(test)]
    accept_copy: Option<egui::Rect>,
    #[cfg(test)]
    cancel_copy: Option<egui::Rect>,
    #[cfg(test)]
    reset: Option<egui::Rect>,
    #[cfg(test)]
    factory_reset: Option<egui::Rect>,
}
fn migrated(app: &Drawer) -> Settings {
    let mut settings = Settings::default();
    for tip in BrushTip::ALL {
        let values = [
            app.panels.settings.brush_size,
            app.panels.settings.brush_aspect,
            app.panels.settings.brush_angle,
            app.workspaces.store.current.input.smoothing
                [BrushTip::ALL.iter().position(|t| *t == tip).unwrap()],
            app.panels.settings.brush_opacity
                [BrushTip::ALL.iter().position(|t| *t == tip).unwrap()],
            app.panels.settings.brush_hardness
                [BrushTip::ALL.iter().position(|t| *t == tip).unwrap()],
            app.panels.settings.brush_spacing
                [BrushTip::ALL.iter().position(|t| *t == tip).unwrap()],
        ];
        for (p, value) in Property::ALL.into_iter().zip(values) {
            settings
                .tools
                .get_mut(&Tool::brush(tip))
                .unwrap()
                .get_mut(&p)
                .unwrap()
                .value = value;
        }
    }
    settings
}
impl Drawer {
    pub(super) fn sync_tool_properties(&mut self) {
        if self.workspaces.store.current.properties.is_none() {
            self.workspaces.store.current.properties = Some(migrated(self));
        }
        self.workspaces
            .store
            .current
            .properties
            .as_mut()
            .unwrap()
            .upgrade(
                self.panels.settings.brush_opacity,
                self.panels.settings.brush_hardness,
                self.panels.settings.brush_spacing,
            );
        let n = BrushTip::ALL
            .iter()
            .position(|t| *t == self.selected)
            .unwrap();
        let live = [
            self.panels.settings.brush_size,
            self.panels.settings.brush_aspect,
            self.panels.settings.brush_angle,
            self.workspaces.store.current.input.smoothing[n],
            self.panels.settings.brush_opacity[n],
            self.panels.settings.brush_hardness[n],
            self.panels.settings.brush_spacing[n],
        ];
        let tool = Tool::brush(self.selected);
        let settings = self.workspaces.store.current.properties.as_mut().unwrap();
        if self.properties.applied == Some(self.selected) {
            for (p, value) in Property::ALL.into_iter().zip(live) {
                let [min, max] = p.bounds();
                settings
                    .tools
                    .get_mut(&tool)
                    .unwrap()
                    .get_mut(&p)
                    .unwrap()
                    .value = value.clamp(min, max);
            }
        }
        let values = Property::ALL.map(|p| settings.tools[&tool][&p].value);
        self.panels.settings.brush_size = values[0];
        self.panels.settings.brush_aspect = values[1];
        self.panels.settings.brush_angle = values[2];
        self.workspaces.store.current.input.smoothing[n] = values[3];
        self.panels.settings.brush_opacity[n] = values[4];
        self.panels.settings.brush_hardness[n] = values[5];
        self.panels.settings.brush_spacing[n] = values[6];
        self.properties.applied = Some(self.selected);
    }
    pub(super) fn reset_property_runtime(&mut self) {
        self.properties.applied = None;
    }
    pub(super) fn tool_property_controls(&mut self, ui: &mut egui::Ui) {
        self.sync_tool_properties();
        let tool = Tool::brush(self.selected);
        let states = self
            .workspaces
            .store
            .current
            .properties
            .as_ref()
            .unwrap()
            .tools[&tool]
            .clone();
        let unit = self
            .workspaces
            .store
            .current
            .properties
            .as_ref()
            .unwrap()
            .size_unit;
        let dpi = self.active.map_or(300.0, |n| self.documents[n].dpi);
        for (p, state) in states {
            if !state.visible {
                continue;
            }
            let mut value = state.value;
            property_value(ui, &mut value, p, state.control, unit, dpi);
            match p {
                Property::Size => self.panels.settings.brush_size = value,
                Property::Aspect => self.panels.settings.brush_aspect = value,
                Property::Angle => self.panels.settings.brush_angle = value.rem_euclid(360.0),
                Property::Stabilization => {
                    let n = BrushTip::ALL
                        .iter()
                        .position(|t| *t == self.selected)
                        .unwrap();
                    self.workspaces.store.current.input.smoothing[n] = value;
                }
                Property::Opacity | Property::Hardness | Property::Spacing => {
                    let n = BrushTip::ALL
                        .iter()
                        .position(|t| *t == self.selected)
                        .unwrap();
                    match p {
                        Property::Opacity => self.panels.settings.brush_opacity[n] = value,
                        Property::Hardness => self.panels.settings.brush_hardness[n] = value,
                        Property::Spacing => self.panels.settings.brush_spacing[n] = value,
                        _ => unreachable!(),
                    }
                }
            }
        }
        size_unit(
            ui,
            &mut self
                .workspaces
                .store
                .current
                .properties
                .as_mut()
                .unwrap()
                .size_unit,
            dpi,
        );
        if crate::help::button(ui, "Configure tool properties", "tool-properties").clicked() {
            self.properties.open = true;
        }
    }
    pub(super) fn property_settings(&mut self, ctx: &egui::Context) {
        if !self.properties.open {
            return;
        }
        self.sync_tool_properties();
        let mut editor = self.properties.editor.take().unwrap_or_else(|| {
            Editor::new(
                self.workspaces
                    .store
                    .current
                    .properties
                    .as_ref()
                    .unwrap()
                    .clone(),
                Tool::brush(self.selected),
            )
        });
        let mut open = true;
        let mut apply = false;
        let mut cancel = false;
        let mut layer = None;
        let dpi = self.active.map_or(300.0, |n| self.documents[n].dpi);
        egui::Window::new("Tool property settings").open(&mut open).constrain(true).vscroll(true).show(ctx,|ui|{
            layer = Some(ui.layer_id());
            ui.horizontal(|ui|{
                let a=ui.add_enabled(!super::edit::invalid_layer(ctx,ui.layer_id()) && editor.draft.valid(),egui::Button::new("Apply tool properties"));let c=crate::help::button(ui,"Cancel tool properties","tool-properties");apply=a.clicked();cancel=c.clicked();#[cfg(test)]{self.properties.apply=Some(a.rect);self.properties.cancel=Some(c.rect);}
            });
            let mut accept=false;let mut dismiss=false;
            if let Some(preview)=&editor.preview {
                if editor.reset_preview {crate::help::checkbox(ui,&mut editor.hide_reset,"Do not show this reset confirmation again","preferences");}
                ui.separator();ui.heading("Review property changes");egui::ScrollArea::vertical().id_salt("property-copy-preview").max_height(180.0).show(ui,|ui|{for line in &preview.reports{ui.label(line);}});
                ui.horizontal(|ui|{let a=crate::help::button(ui,"Accept preview into draft","tool-properties");let c=crate::help::button(ui,"Cancel property copy","tool-properties");accept=a.clicked();dismiss=c.clicked();#[cfg(test)]{self.properties.accept_copy=Some(a.rect);self.properties.cancel_copy=Some(c.rect);}});
            }
            if accept{if let Some(tip)=editor.pending_factory.take(){editor.factory.insert(tip);}editor.draft=editor.preview.take().unwrap().next;super::edit::discard_layer(ctx,ui.layer_id());if editor.reset_preview&&editor.hide_reset{self.workspaces.store.current.preferences.suppressed.insert(drawer_core::preferences::Confirmation::ResetProperties);}editor.reset_preview=false;editor.hide_reset=false;}else if dismiss{editor.pending_factory=None;editor.preview=None;editor.reset_preview=false;editor.hide_reset=false;}
            ui.horizontal_wrapped(|ui|{for tool in Tool::ALL {if crate::help::selectable(ui,editor.source==tool,tool.name(),"tool-properties").clicked(){super::edit::discard_layer(ctx,ui.layer_id());editor.source=tool;}}});
            if editor.draft.tools[&editor.source].is_empty(){ui.label("This tool has no shared brush properties; copies skip unsupported properties.");}
            for p in Property::ALL {if let Some(state)=editor.draft.tools.get_mut(&editor.source).unwrap().get_mut(&p) {ui.push_id(p.name(),|ui|{
                property_value(ui,&mut state.value,p,Control::Number,editor.draft.size_unit,dpi);
                ui.horizontal(|ui|{crate::help::checkbox(ui,&mut state.visible,"Visible","tool-properties");});
                ui.horizontal(|ui|{for control in [Control::Slider,Control::Number,Control::Indicator]{ui.selectable_value(&mut state.control,control,format!("{control:?}"));}});
            });}}
            size_unit(ui,&mut editor.draft.size_unit,dpi);
            let pending=super::edit::invalid_layer(ctx,ui.layer_id());
            if pending {ui.colored_label(egui::Color32::LIGHT_RED,"Finish or cancel the invalid number before applying, copying, or saving.");}
            ui.separator();ui.label("Copy selected properties");
            ui.horizontal(|ui|{for(n,p)in Property::ALL.into_iter().enumerate(){crate::help::checkbox(ui,&mut editor.properties[n],p.name(),"tool-properties");}});
            ui.horizontal(|ui|{ui.selectable_value(&mut editor.kind,CopyKind::Values,"Values only");ui.selectable_value(&mut editor.kind,CopyKind::Appearance,"Visibility / controls only");});
            ui.horizontal(|ui|{for(n,name)in ["Same tool group","Selected tools","All tools"].into_iter().enumerate(){ui.selectable_value(&mut editor.scope,n as u8,name);}});
            if editor.scope==1{ui.horizontal_wrapped(|ui|{for(n,t)in Tool::ALL.into_iter().enumerate(){crate::help::checkbox(ui,&mut editor.targets[n],t.name(),"tool-properties");}});}
            if ui.add_enabled(!pending,egui::Button::new("Preview property copy")).clicked(){
                let targets=Tool::ALL.into_iter().enumerate().filter_map(|(n,t)|(editor.scope==2 || (editor.scope==0&&t.group()==editor.source.group()) || (editor.scope==1&&editor.targets[n])).then_some(t)).collect::<Vec<_>>();
                let properties=Property::ALL.into_iter().enumerate().filter_map(|(n,p)|editor.properties[n].then_some(p)).collect::<Vec<_>>();
                match editor.draft.preview(editor.source,&targets,&properties,editor.kind){Ok(preview)=>{editor.preview=Some(preview);editor.pending_factory=None;editor.reset_preview=false;},Err(e)=>self.status=Some(e)}
            }
            let factory_reset=ui.add_enabled(!pending,egui::Button::new("Preview current basic brush reset…"));#[cfg(test)]{self.properties.factory_reset=Some(factory_reset.rect);}
            if factory_reset.clicked(){
                if let Some(tip)=BrushTip::ALL.into_iter().find(|t|Tool::brush(*t)==editor.source){
                    let mut next=editor.draft.clone();
                    match drawer_core::basic_brush::restore_values(&mut next,tip){
                        Ok(())=>{editor.preview=Some(Preview{next,reports:vec![format!("Restore {} factory values, pressure mapping and mouse dynamics? Other brushes, presentation and drawing stay unchanged. Apply is required.",tip.label())]});editor.pending_factory=Some(tip);editor.reset_preview=false;},
                        Err(e)=>self.status=Some(e),
                    }
                }else{self.status=Some("This tool has no basic brush preset".into());}
            }
            let reset=crate::help::button(ui,"Reset all tool properties…","tool-properties");#[cfg(test)]{self.properties.reset=Some(reset.rect);}
            if reset.clicked(){editor.pending_factory=None;if self.workspaces.store.current.preferences.suppressed.contains(&drawer_core::preferences::Confirmation::ResetProperties){editor.draft=Settings::default();editor.preview=None;super::edit::discard_layer(ctx,ui.layer_id());}else{editor.reset_preview=true;editor.preview=Some(Preview{next:Settings::default(),reports:vec!["Reset all tool values and property presentation? Accept only to replace the draft.".into()]});}}
            ui.horizontal_wrapped(|ui|{
                if ui.add_enabled(!pending,egui::Button::new("Save local property preset…")).clicked() && let Some(path)=rfd::FileDialog::new().set_file_name("tool-properties.json").save_file() && let Err(e)=editor.draft.save(&path){self.status=Some(e.to_string());}
                if crate::help::button(ui,"Read local property preset…","tool-properties").clicked() && let Some(path)=rfd::FileDialog::new().add_filter("Tool property preset",&["json"]).pick_file(){match Settings::read(&path){Ok(s)=>{editor.draft=s;editor.preview=None;editor.factory.clear();editor.pending_factory=None;super::edit::discard_layer(ctx,ui.layer_id());},Err(e)=>self.status=Some(e.to_string())}}
                if ui.add_enabled(self.storage.is_some(),egui::Button::new("Restore before-apply properties")).clicked(){match drawer_core::workspaces::Snapshot::read(&self.storage.as_ref().unwrap().root().join("settings/workspace-before-apply.json")){Ok(s)=>if let Some(mut properties)=s.properties{properties.upgrade(s.panels.brush_opacity,s.panels.brush_hardness,s.panels.brush_spacing);editor.draft=properties;editor.preview=None;editor.factory.clear();editor.pending_factory=None;super::edit::discard_layer(ctx,ui.layer_id());}else{self.status=Some("The backup predates per-tool properties".into());},Err(e)=>self.status=Some(e.to_string())}}
            });
        });
        if cancel || !open || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            if let Some(layer) = layer {
                super::edit::discard_layer(ctx, layer);
            }
            self.properties.open = false;
            self.properties.editor = None;
        } else if apply && layer.is_some_and(|layer| super::edit::invalid_layer(ctx, layer)) {
            self.status =
                Some("Invalid numeric input: finish or cancel the field before applying".into());
            self.properties.editor = Some(editor);
        } else if apply {
            let mut snapshot = self.capture_workspace();
            snapshot.properties = Some(editor.draft.clone());
            for tip in &editor.factory {
                drawer_core::basic_brush::restore_input(&mut snapshot, *tip);
            }
            snapshot.input.smoothing = BrushTip::ALL
                .map(|tip| editor.draft.tools[&Tool::brush(tip)][&Property::Stabilization].value);
            self.stage_workspace(snapshot, self.workspaces.store.active.clone());
            match self.apply_workspace(
                drawer_core::workspaces::Scope {
                    other: true,
                    ..Default::default()
                },
                None,
            ) {
                Ok(()) => {
                    if let Some(layer) = layer {
                        super::edit::discard_layer(ctx, layer);
                    }
                    self.properties.open = false;
                    self.properties.editor = None;
                    self.sync_tool_properties();
                }
                Err(e) => {
                    self.workspaces.staged = None;
                    self.status = Some(e);
                    self.properties.editor = Some(editor);
                }
            }
        } else {
            self.properties.editor = Some(editor);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{TestDirectory, click, frame, tiny_spec};
    #[test]
    fn canonical_units_and_slider_number_keys_use_the_same_increment() {
        smoke().unwrap();
    }
    #[test]
    fn factory_preview_cancel_apply_backup_restart_preserves_other_brush_and_drawing() {
        let root = TestDirectory::new();
        let storage = UserStorage::at(root.0.join("user")).unwrap();
        let ctx = egui::Context::default();
        let mut app = Drawer::new(&ctx, None)
            .with_storage(UserStorage::at(storage.root().to_path_buf()).unwrap());
        app.create_canvas(tiny_spec()).unwrap();
        app.set_tool(BrushTip::Wash);
        frame(&ctx, &mut app, vec![]);
        app.panels.settings.brush_size = 90.0;
        app.workspaces.store.current.input.mouse_pressure[3].enabled = true;
        app.workspaces.store.current.input.mouse_pressure[0].enabled = true;
        app.workspaces.store.current.pressure.tools[3].opacity = true;
        frame(&ctx, &mut app, vec![]);
        let baseline = app.capture_workspace();
        let doc = app.documents[0].snapshot();
        let dirty = app.documents[0].dirty;
        let mut factory = baseline.properties.clone().unwrap();
        drawer_core::basic_brush::restore_values(&mut factory, BrushTip::Wash).unwrap();
        let editor = || {
            let mut e = Editor::new(baseline.properties.clone().unwrap(), Tool::Wash);
            e.pending_factory = Some(BrushTip::Wash);
            e.preview = Some(Preview {
                next: factory.clone(),
                reports: vec!["Restore Airbrush factory values and dynamics".into()],
            });
            e
        };
        app.properties.open = true;
        app.properties.editor = Some(editor());
        frame(&ctx, &mut app, vec![]);
        frame(&ctx, &mut app, vec![]);
        let rect = app.properties.cancel_copy.unwrap();
        click(&ctx, &mut app, rect);
        assert_eq!(app.capture_workspace(), baseline);
        assert!(
            app.properties
                .editor
                .as_ref()
                .unwrap()
                .pending_factory
                .is_none()
        );
        app.properties.editor = Some(editor());
        frame(&ctx, &mut app, vec![]);
        frame(&ctx, &mut app, vec![]);
        let rect = app.properties.accept_copy.unwrap();
        click(&ctx, &mut app, rect);
        assert!(
            app.properties
                .editor
                .as_ref()
                .unwrap()
                .factory
                .contains(&BrushTip::Wash)
        );
        let rect = app.properties.cancel.unwrap();
        click(&ctx, &mut app, rect);
        assert_eq!(app.capture_workspace(), baseline);
        app.properties.open = true;
        app.properties.editor = Some(editor());
        frame(&ctx, &mut app, vec![]);
        frame(&ctx, &mut app, vec![]);
        let rect = app.properties.accept_copy.unwrap();
        click(&ctx, &mut app, rect);
        frame(&ctx, &mut app, vec![]);
        let rect = app.properties.apply.unwrap();
        click(&ctx, &mut app, rect);
        assert_eq!(app.panels.settings.brush_size, 12.0);
        assert_eq!(app.panels.settings.brush_opacity[3], 0.6);
        assert!(!app.workspaces.store.current.input.mouse_pressure[3].enabled);
        assert!(app.workspaces.store.current.input.mouse_pressure[0].enabled);
        assert!(!app.workspaces.store.current.pressure.tools[3].opacity);
        assert_eq!(
            app.workspaces
                .store
                .current
                .properties
                .as_ref()
                .unwrap()
                .tools[&Tool::Pen],
            baseline.properties.as_ref().unwrap().tools[&Tool::Pen]
        );
        let backup = drawer_core::workspaces::Snapshot::read(
            &storage.root().join("settings/workspace-before-apply.json"),
        )
        .unwrap();
        assert_eq!(backup, baseline);
        let restarted = Drawer::new(&egui::Context::default(), None)
            .with_storage(UserStorage::at(storage.root().to_path_buf()).unwrap());
        assert_eq!(restarted.panels.settings.brush_size, 12.0);
        assert!(!restarted.workspaces.store.current.input.mouse_pressure[3].enabled);
        assert!(app.documents[0].snapshot() == doc);
        assert_eq!(app.documents[0].dirty, dirty);
        assert_eq!(app.documents[0].history_counts(), (0, 0));
    }
    #[test]
    fn values_switch_per_tool_and_display_state_preset_restart_preserve_document() {
        let root = TestDirectory::new();
        let storage = UserStorage::at(root.0.join("user")).unwrap();
        let ctx = egui::Context::default();
        let mut app = Drawer::new(&ctx, None)
            .with_storage(UserStorage::at(storage.root().to_path_buf()).unwrap());
        app.create_canvas(tiny_spec()).unwrap();
        app.panels.settings.brush_size = 31.0;
        app.sync_tool_properties();
        app.set_tool(BrushTip::Eraser);
        assert_eq!(app.panels.settings.brush_size, 12.0);
        app.panels.settings.brush_size = 9.0;
        app.sync_tool_properties();
        app.set_tool(BrushTip::Pen);
        assert_eq!(app.panels.settings.brush_size, 31.0);
        let settings = app.workspaces.store.current.properties.as_mut().unwrap();
        let size = settings
            .tools
            .get_mut(&Tool::Pen)
            .unwrap()
            .get_mut(&Property::Size)
            .unwrap();
        size.visible = false;
        size.control = Control::Indicator;
        app.persist_workspace().unwrap();
        let saved = app
            .workspaces
            .store
            .current
            .properties
            .as_ref()
            .unwrap()
            .clone();
        let file = root.0.join("properties.json");
        saved.save(&file).unwrap();
        assert_eq!(Settings::read(&file).unwrap(), saved);
        let before = std::fs::read(&file).unwrap();
        std::fs::write(root.0.join("properties.json.write.tmp"), "foreign").unwrap();
        assert!(saved.save(&file).is_err());
        assert_eq!(std::fs::read(&file).unwrap(), before);
        std::fs::write(root.0.join("damaged.json"), "damaged").unwrap();
        assert!(Settings::read(&root.0.join("damaged.json")).is_err());
        let restarted = Drawer::new(&egui::Context::default(), None)
            .with_storage(UserStorage::at(storage.root().to_path_buf()).unwrap());
        assert_eq!(
            restarted
                .workspaces
                .store
                .current
                .properties
                .as_ref()
                .unwrap(),
            &saved
        );
        let before = app.documents[0].snapshot();
        let history = app.documents[0].history_counts();
        frame(&ctx, &mut app, vec![]);
        assert_eq!(app.documents[0].snapshot(), before);
        assert_eq!(app.documents[0].history_counts(), history);
    }
    #[test]
    fn bulk_copy_cancel_accept_apply_reset_and_backup_are_atomic() {
        let root = TestDirectory::new();
        let storage = UserStorage::at(root.0.join("user")).unwrap();
        let ctx = egui::Context::default();
        let mut app = Drawer::new(&ctx, None)
            .with_storage(UserStorage::at(storage.root().to_path_buf()).unwrap());
        app.create_canvas(tiny_spec()).unwrap();
        app.panels.settings.brush_size = 31.0;
        frame(&ctx, &mut app, vec![]);
        let baseline = app
            .workspaces
            .store
            .current
            .properties
            .as_ref()
            .unwrap()
            .clone();
        let doc = app.documents[0].snapshot();
        let history = app.documents[0].history_counts();
        let preview = baseline
            .preview(Tool::Pen, &Tool::ALL, &[Property::Size], CopyKind::Values)
            .unwrap();
        app.properties.open = true;
        let mut editor = Editor::new(baseline.clone(), Tool::Pen);
        editor.preview = Some(preview.clone());
        app.properties.editor = Some(editor);
        frame(&ctx, &mut app, vec![]);
        frame(&ctx, &mut app, vec![]);
        let rect = app.properties.cancel_copy.unwrap();
        click(&ctx, &mut app, rect);
        assert_eq!(app.properties.editor.as_ref().unwrap().draft, baseline);
        assert!(app.properties.editor.as_ref().unwrap().preview.is_none());
        app.properties.editor.as_mut().unwrap().preview = Some(preview.clone());
        frame(&ctx, &mut app, vec![]);
        frame(&ctx, &mut app, vec![]);
        let rect = app.properties.accept_copy.unwrap();
        click(&ctx, &mut app, rect);
        assert_eq!(app.properties.editor.as_ref().unwrap().draft, preview.next);
        assert_eq!(
            app.workspaces.store.current.properties.as_ref().unwrap(),
            &baseline
        );
        frame(&ctx, &mut app, vec![]);
        let rect = app.properties.cancel.unwrap();
        click(&ctx, &mut app, rect);
        assert_eq!(
            app.workspaces.store.current.properties.as_ref().unwrap(),
            &baseline
        );
        app.properties.open = true;
        app.properties.editor = Some(Editor::new(preview.next.clone(), Tool::Pen));
        frame(&ctx, &mut app, vec![]);
        frame(&ctx, &mut app, vec![]);
        let rect = app.properties.apply.unwrap();
        click(&ctx, &mut app, rect);
        assert_eq!(
            app.workspaces.store.current.properties.as_ref().unwrap(),
            &preview.next
        );
        assert_eq!(
            drawer_core::workspaces::Snapshot::read(
                &storage.root().join("settings/workspace-before-apply.json")
            )
            .unwrap()
            .properties,
            Some(baseline)
        );
        assert!(
            preview.next.tools[&Tool::Eyedropper].is_empty()
                && preview.next.tools[&Tool::Move].is_empty()
        );
        app.properties.open = true;
        let mut editor = Editor::new(preview.next.clone(), Tool::Pen);
        editor.preview = Some(Preview {
            next: Settings::default(),
            reports: vec!["Confirm reset".into()],
        });
        app.properties.editor = Some(editor);
        frame(&ctx, &mut app, vec![]);
        frame(&ctx, &mut app, vec![]);
        let rect = app.properties.accept_copy.unwrap();
        click(&ctx, &mut app, rect);
        frame(&ctx, &mut app, vec![]);
        let rect = app.properties.apply.unwrap();
        click(&ctx, &mut app, rect);
        assert_eq!(
            app.workspaces.store.current.properties.as_ref().unwrap(),
            &Settings::default()
        );
        assert_eq!(app.panels.settings.brush_size, 12.0);
        assert_eq!(app.documents[0].snapshot(), doc);
        assert_eq!(app.documents[0].history_counts(), history);
    }
}
