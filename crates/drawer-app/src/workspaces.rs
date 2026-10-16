use super::*;
use drawer_core::workspaces::{self, Dock, Placement, Scope, Snapshot, Store};
use std::collections::HashSet;

pub(super) struct PadPaint {
    replay: Option<drawer_core::mouse_pressure::Replay>,
    last_input: Option<([f64; 2], u64)>,
    document: Document,
    stroke: drawer_core::painting::Stroke,
}
impl PadPaint {
    pub(super) fn original(&self) -> &[[u8; 4]] {
        &self.document.layers[0].pixels
    }
    pub(super) fn begin(
        pixels: &[[u8; 4]],
        mut style: drawer_core::painting::Style,
        materials: std::sync::Arc<drawer_core::material_library::Library>,
    ) -> Result<Self, String> {
        let size = workspaces::PAD_SIZE;
        let mut document = Document::create(
            CanvasSpec {
                width: size[0] as u32,
                height: size[1] as u32,
                dpi: 300.0,
                background: Background::Transparent,
            },
            pixels.len() * 4,
        )
        .map_err(|e| e.to_string())?;
        document.layers[0].pixels.clone_from_slice(pixels);
        style.shape.size = style.shape.size.clamp(0.1, 64.0);
        let replay = if style.pattern.behavior.continuous {
            Some(
                drawer_core::mouse_pressure::Replay::begin(
                    &document,
                    0,
                    style,
                    Default::default(),
                )?
                .with_materials(materials.clone())?
                .configure_input(Default::default(), true)?,
            )
        } else {
            None
        };
        let stroke =
            drawer_core::painting::Stroke::with_budget(&document, 0, style, pixels.len() * 128)?
                .with_materials(materials)?;
        Ok(Self {
            replay,
            last_input: None,
            document,
            stroke,
        })
    }
    pub(super) fn sample(&mut self, point: [f64; 2], micros: u64) -> Result<(), String> {
        let speed = self.last_input.and_then(|(p, t)| {
            if micros > t {
                Some(
                    ((point[0] - p[0]).hypot(point[1] - p[1]) / ((micros - t) as f64 / 1e6))
                        .min(1e18),
                )
            } else {
                None
            }
        });
        if let Some(replay) = &mut self.replay {
            replay.push_input(
                drawer_core::mouse_pressure::Sample {
                    point,
                    micros: Some(micros),
                },
                1.0,
                0.0,
            )?;
            self.stroke = replay.render(&self.document, false)?;
            self.last_input = Some((point, micros));
            return Ok(());
        }
        self.stroke
            .set_dynamic_input(drawer_core::brush_dynamics::Input {
                speed,
                ..Default::default()
            })?;
        self.stroke.sample_pressure(&self.document, point, 1.0)?;
        self.last_input = Some((point, micros));
        Ok(())
    }
    pub(super) fn tick(&mut self, micros: u64) -> Result<bool, String> {
        let Some(replay) = &mut self.replay else {
            return Ok(false);
        };
        let Some((point, last)) = self.last_input else {
            return Ok(false);
        };
        if micros.saturating_sub(last) < replay.spray_period().unwrap_or(u32::MAX) as u64 {
            return Ok(false);
        }
        replay.push_clock(drawer_core::mouse_pressure::Sample {
            point,
            micros: Some(micros),
        })?;
        self.stroke = replay.render(&self.document, false)?;
        self.last_input = Some((point, micros));
        Ok(true)
    }
    pub(super) fn preview(&self) -> Vec<[u8; 4]> {
        let d = &self.document;
        (0..d.width * d.height)
            .map(|n| {
                self.stroke
                    .pixel(d, 0, &d.layers[0], n % d.width, n / d.width)
                    .unwrap_or(d.layers[0].pixels[n as usize])
            })
            .collect()
    }
    pub(super) fn finish(mut self) -> (Vec<[u8; 4]>, Option<String>) {
        if let Some(replay) = &self.replay {
            match replay.render(&self.document, true) {
                Ok(stroke) => self.stroke = stroke,
                Err(e) => return (self.document.layers.remove(0).pixels, Some(e)),
            }
        }
        let error = self.stroke.commit(&mut self.document).err();
        (self.document.layers.remove(0).pixels, error)
    }
    pub(super) fn cancel(mut self) -> Vec<[u8; 4]> {
        self.document.layers.remove(0).pixels
    }
}
pub(super) struct Workspaces {
    pub store: Store,
    saved: Option<Snapshot>,
    pub open: bool,
    name: String,
    selection: String,
    pub staged: Option<(Snapshot, Option<String>)>,
    scope: Scope,
    save_before: bool,
    before_name: String,
    pub reports: Vec<String>,
    placed: [Option<(Placement, drawer_core::display_view::Monitor)>; 4],
    pad_texture: Option<egui::TextureHandle>,
    pad_point: Option<[f64; 2]>,
    pad_trace: drawer_core::brush_cursor::Trace,
    pad_style: Option<PadPaint>,
    pad_history: drawer_core::color_tables::Pixels,
    pad_actions: [Option<egui::Rect>; 2],
    pad_check_rect: Option<egui::Rect>,
    focused: bool,
    keys: HashSet<egui::Key>,
    buttons: HashSet<usize>,
    pub native_panels: usize,
    #[cfg(test)]
    pub apply_button: Option<egui::Rect>,
    #[cfg(test)]
    pub cancel_button: Option<egui::Rect>,
    #[cfg(test)]
    pub pad_rect: Option<egui::Rect>,
}
impl Default for Workspaces {
    fn default() -> Self {
        Self {
            store: Default::default(),
            saved: None,
            open: false,
            name: String::new(),
            selection: String::new(),
            staged: None,
            scope: Scope::ALL,
            save_before: false,
            before_name: String::new(),
            reports: Vec::new(),
            placed: Default::default(),
            pad_texture: None,
            pad_point: None,
            pad_trace: Default::default(),
            pad_style: None,
            pad_history: Default::default(),
            pad_actions: Default::default(),
            pad_check_rect: None,
            focused: true,
            keys: HashSet::new(),
            buttons: HashSet::new(),
            native_panels: 0,
            #[cfg(test)]
            apply_button: None,
            #[cfg(test)]
            cancel_button: None,
            #[cfg(test)]
            pad_rect: None,
        }
    }
}
impl Workspaces {
    fn pad_rect_for_check(&self) -> Option<egui::Rect> {
        self.pad_check_rect
    }
    fn cancel_pad(&mut self) {
        if let Some(paint) = self.pad_style.take() {
            self.store.current.scratchpad.pixels = paint.cancel();
        }
        self.pad_point = None;
        self.pad_trace.clear();
    }

    // Inspect every release, including multiple device/key releases in one frame.
    pub fn input(&mut self, ctx: &egui::Context) {
        ctx.input(|i| {
            for event in &i.events {
                match event {
                    egui::Event::Key { key, pressed, .. } => {
                        if *pressed {
                            self.keys.insert(*key);
                        } else {
                            self.keys.remove(key);
                        }
                    }
                    egui::Event::PointerButton {
                        button, pressed, ..
                    } => {
                        if *pressed {
                            self.buttons.insert(*button as usize);
                        } else {
                            self.buttons.remove(&(*button as usize));
                            if *button == egui::PointerButton::Primary {
                                self.pad_point = None;
                            }
                        }
                    }
                    egui::Event::WindowFocused(true) => self.focused = true,
                    egui::Event::WindowFocused(false) => {
                        self.focused = false;
                        self.keys.clear();
                        self.buttons.clear();
                        self.pad_point = None;
                    }
                    egui::Event::PointerGone => {
                        self.keys.clear();
                        self.buttons.clear();
                        self.pad_point = None;
                    }
                    _ => {}
                }
            }
            if !i.focused {
                self.focused = false;
                self.keys.clear();
                self.buttons.clear();
                self.pad_point = None;
            }
        });
    }
    pub fn input_idle(&self) -> bool {
        self.keys.is_empty() && self.buttons.is_empty() && self.pad_point.is_none()
    }
    pub fn painting(&self) -> bool {
        self.pad_point.is_some()
    }
    pub fn is_focused(&self) -> bool {
        self.focused
    }
    pub fn command_label(&self, command: &str) -> String {
        self.store
            .current
            .bindings
            .iter()
            .filter(|b| b.command == command)
            .map(|b| b.label())
            .collect::<Vec<_>>()
            .join(" / ")
    }
}
impl Drawer {
    pub(super) fn capture_workspace(&self) -> Snapshot {
        let mut s = self.workspaces.store.current.clone();
        s.panels = self.panels.settings;
        if let Some(paint) = &self.workspaces.pad_style {
            s.scratchpad.pixels = paint.original().to_vec();
        }
        if let Some(pixels) = self.mixer.original() {
            s.mixer.pixels = pixels.to_vec();
        }

        s.navigator = self.canvas_view.navigator;
        s.display = self.display.preferences.clone();
        s.interface = self.interface.settings;
        s.interpolation = self.interpolation;
        s.aids = self.aids.preferences;
        s.tool = self
            .tool_input
            .controller
            .persisted_tool()
            .unwrap_or(self.selected)
            .name()
            .into();
        s.color = self.references.session.workspace.color;
        s.color_modes.observe(s.color);
        s.color_picker.observe(s.color);
        s.eyedropper.observe(s.color);
        if let Some(tool) = s.user_tools.active_entry().cloned()
            && let Ok(p) =
                drawer_core::brush_presets::Preset::capture(&s, tool.preset.tip, &tool.preset.name)
        {
            let _ = s.user_tools.update_active(p);
        }
        s
    }
    pub(super) fn restore_workspace(&mut self, s: &Snapshot) {
        self.drawing.cancel_sample();
        self.color_tables.cancel_drag();
        self.screen_sampler.cancel();
        self.proof_editor = Default::default();
        self.finish_stroke();
        self.mixer.cancel(&mut self.workspaces.store.current.mixer);
        self.workspaces.cancel_pad();
        if self.workspaces.store.current.scratchpad.pixels != s.scratchpad.pixels {
            self.workspaces.pad_history = Default::default();
        }
        if self.workspaces.store.current.mixer.pixels != s.mixer.pixels {
            self.mixer.history = Default::default();
        }

        if self.workspaces.store.current.color_tables != s.color_tables {
            self.color_tables.history = Default::default();
        }
        self.layout.generation += 1;
        self.reset_property_runtime();
        self.tool_input.controller.cancel();
        self.panels.settings = s.panels;
        self.canvas_view.navigator = s.navigator;
        self.canvas_view.navigator_generation += 1;
        self.display.preferences = s.display.clone();
        self.interface.settings = s.interface;
        self.interpolation = s.interpolation;
        self.aids.preferences = s.aids;
        self.references.session.workspace.color = s.color;
        self.selected = BrushTip::ALL
            .into_iter()
            .find(|t| t.name() == s.tool)
            .unwrap_or(BrushTip::Pen);
        if let Some(n) = self.active {
            self.documents[n].tool = self.selected;
        }
        self.workspaces.placed = Default::default();
        self.workspaces.pad_point = None;
        self.workspaces.pad_trace.clear();
        self.workspaces.cancel_pad();
    }
    pub(super) fn load_workspaces(&mut self, storage: &UserStorage) {
        match Store::load(storage) {
            Ok(Some(mut store)) => {
                // Individual settings editors remain compatible. Prefer a file explicitly
                // changed after the last workspace transaction, including damaged-file recovery.
                let legacy = self.capture_workspace();
                let saved_at = std::fs::metadata(storage.root().join("settings/workspaces.json"))
                    .and_then(|m| m.modified())
                    .ok();
                let newer = |file: &str| {
                    std::fs::metadata(storage.root().join("settings").join(file))
                        .and_then(|m| m.modified())
                        .ok()
                        .is_some_and(|t| saved_at.is_none_or(|s| t > s))
                };
                if newer("panels.json") {
                    store.current.panels = legacy.panels;
                }
                // A damaged individual file must not be hidden by equal/coarse timestamps.
                if newer("navigator.json")
                    || drawer_core::navigator_preferences::Preferences::load(storage).is_err()
                {
                    store.current.navigator = legacy.navigator;
                }
                if newer("display.json") {
                    store.current.display = legacy.display;
                }
                if newer("interface.json") {
                    store.current.interface = legacy.interface;
                }
                if newer("interpolation.json") {
                    store.current.interpolation = legacy.interpolation;
                }
                if newer("aids.json") {
                    store.current.aids = legacy.aids;
                }

                self.workspaces.reports = store.current.recover(&self.display.monitors);
                // Also validate bindings on restart; a damaged collection never silently overwrites keys.
                match drawer_core::tool_locks::for_storage(&store.current) {
                    Ok(current) => store.current = current,
                    Err(e) => {
                        self.status = Some(format!("Locked tool settings unavailable: {e}"));
                        return;
                    }
                }
                match workspaces::validate_bindings(&store.current.bindings) {
                    Ok(()) => {
                        self.restore_workspace(&store.current);
                        self.workspaces.saved = Some(store.current.clone());
                        self.workspaces.store = store;
                    }
                    Err(e) => self.status = Some(format!("Workspace settings unavailable: {e}")),
                }
            }
            Ok(None) => self.workspaces.store.current = self.capture_workspace(),
            Err(e) => {
                self.status = Some(format!("Workspace settings unavailable: {e}"));
                self.workspaces.store.current = self.capture_workspace();
            }
        }
    }
    pub(super) fn persist_workspace(&mut self) -> Result<(), String> {
        let current = self.capture_workspace();
        if self.workspaces.saved.as_ref() != Some(&current) {
            let mut next = self.workspaces.store.clone();
            next.current = current.clone();
            if let Some(storage) = &self.storage {
                next.save(storage).map_err(|e| e.to_string())?;
            }
            self.workspaces.store = next;
            self.workspaces.saved = Some(current);
        }
        Ok(())
    }
    pub(super) fn apply_workspace(
        &mut self,
        scope: Scope,
        save_before: Option<&str>,
    ) -> Result<(), String> {
        let Some((incoming, active)) = self.workspaces.staged.clone() else {
            return Ok(());
        };
        if !incoming.valid() || !scope.any() {
            return Err("Invalid workspace or empty import scope".into());
        }
        if let Some(paint) = self.workspaces.pad_style.take() {
            let before = paint.original().to_vec();
            let (pixels, error) = paint.finish();
            if error.is_none() {
                self.workspaces.pad_history.record(before, &pixels);
            }
            self.workspaces.store.current.scratchpad.pixels = pixels;
            self.workspaces.pad_point = None;
            self.workspaces.pad_trace.clear();
            if let Some(e) = error {
                return Err(e);
            }
        }
        let current = self.capture_workspace();
        let reports = self
            .workspaces
            .store
            .apply(
                self.storage.as_ref(),
                current,
                &incoming,
                scope,
                active,
                save_before,
                &self.display.monitors,
            )
            .map_err(|e| e.to_string())?;
        let snapshot = self.workspaces.store.current.clone();
        self.restore_workspace(&snapshot);
        self.workspaces.saved = Some(snapshot);
        self.workspaces.reports = reports;
        self.workspaces.staged = None;
        Ok(())
    }
    pub(super) fn stage_workspace(&mut self, snapshot: Snapshot, active: Option<String>) {
        self.workspaces.staged = Some((snapshot, active));
        self.workspaces.scope = Scope::ALL;
        self.workspaces.save_before = false;
    }
    pub(super) fn workspace_settings(&mut self, ctx: &egui::Context) {
        if self.workspaces.open && self.workspaces.staged.is_none() {
            let mut open = true;
            let mut action = None;
            egui::Window::new("Workspaces").open(&mut open).vscroll(true).scroll_bar_visibility(crate::ui_behavior::scroll(ctx)).show(ctx, |ui| {
                ui.label("Layout changes are saved locally. Named workspaces keep a separate saved layout.");
                for kind in drawer_core::panels::Kind::ALL {
                    ui.push_id(kind.index(), |ui| {
                        ui.horizontal_wrapped(|ui| {
                            ui.label(kind.label());
                            let p = &mut self.workspaces.store.current.placements[kind.index()];
                            for dock in Dock::ALL { ui.selectable_value(&mut p.dock, dock, format!("{dock:?}")); }
                            if p.dock == Dock::Floating {
                                egui::ComboBox::from_id_salt("screen").selected_text(p.monitor.as_deref().unwrap_or("Primary screen")).show_ui(ui, |ui| {
                                    ui.selectable_value(&mut p.monitor, None, "Primary screen");
                                    for m in &self.display.monitors { ui.selectable_value(&mut p.monitor, Some(m.key.clone()), &m.key); }
                                });
                            }
                            let order = &mut self.workspaces.store.current.order;
                            let n = order.iter().position(|k| *k == kind).unwrap();
                            if ui.add_enabled(n > 0, egui::Button::new("Up")).clicked() { order.swap(n, n - 1); }
                            if ui.add_enabled(n < 3, egui::Button::new("Down")).clicked() { order.swap(n, n + 1); }
                        });
                    });
                }
                ui.separator();
                ui.horizontal(|ui| { ui.label("New name"); super::edit::text(ui,&mut self.workspaces.name,"workspace"); if crate::help::button(ui,"Save new workspace","workspace").clicked() { action = Some(0); } });
                egui::ComboBox::from_id_salt("named-workspace").selected_text(&self.workspaces.selection).show_ui(ui, |ui| {
                    for n in &self.workspaces.store.named { ui.selectable_value(&mut self.workspaces.selection, n.name.clone(), &n.name); }
                });
                ui.horizontal_wrapped(|ui| {
                    for (id, label) in [(1, "Switch / preview"), (2, "Update selected saved workspace"), (3, "Delete selected")] { if crate::help::button(ui,label,"workspace").clicked() { action = Some(id); } }
                    if ui.add_enabled(self.workspaces.store.active.is_some(), egui::Button::new("Reapply current saved workspace")).clicked() { action = Some(4); }
                    if crate::help::button(ui,"Reset workspace","workspace").clicked() { action = Some(5); }
                    if crate::help::button(ui,"Import local workspace","workspace").clicked() { action = Some(6); }
                    if crate::help::button(ui,"Export current workspace","workspace").clicked() { action = Some(7); }
                    if ui.add_enabled(self.storage.is_some(), egui::Button::new("Restore before-apply backup")).clicked() { action = Some(8); }
                });
                if crate::help::button(ui,"Popup test painting","workspace").clicked() { self.workspaces.store.current.scratchpad.visible = true; }
                for report in &self.workspaces.reports { ui.colored_label(egui::Color32::YELLOW, report); }
            });
            self.workspaces.open = open;
            if let Some(action) = action {
                let result = (|| -> Result<(), String> {
                    let mut next = self.workspaces.store.clone();
                    match action {
                        0 => next
                            .save_named(self.workspaces.name.trim(), self.capture_workspace())
                            .map_err(|e| e.to_string())?,
                        1 | 4 => {
                            let name = if action == 4 {
                                next.active.as_ref()
                            } else {
                                Some(&self.workspaces.selection)
                            };
                            let n = next
                                .named
                                .iter()
                                .find(|n| Some(&n.name) == name)
                                .ok_or("Select a saved workspace")?;
                            self.stage_workspace(n.snapshot.clone(), Some(n.name.clone()));
                            return Ok(());
                        }
                        2 => {
                            let current = self.capture_workspace();
                            let n = next
                                .named
                                .iter_mut()
                                .find(|n| n.name == self.workspaces.selection)
                                .ok_or("Select a saved workspace")?;
                            n.snapshot = current;
                        }
                        3 => {
                            next.named.retain(|n| n.name != self.workspaces.selection);
                            if next.active.as_ref() == Some(&self.workspaces.selection) {
                                next.active = None;
                            }
                        }
                        5 => {
                            self.stage_workspace(Snapshot::default(), None);
                            return Ok(());
                        }
                        6 => {
                            if let Some(path) = rfd::FileDialog::new()
                                .add_filter("Local workspace", &["json"])
                                .pick_file()
                            {
                                self.stage_workspace(
                                    Snapshot::read(&path).map_err(|e| e.to_string())?,
                                    None,
                                );
                            }
                            return Ok(());
                        }
                        7 => {
                            if let Some(path) = rfd::FileDialog::new()
                                .set_file_name("workspace.json")
                                .save_file()
                            {
                                self.capture_workspace()
                                    .export(&path)
                                    .map_err(|e| e.to_string())?;
                            }
                            return Ok(());
                        }
                        8 => {
                            let s = Snapshot::read(
                                &self
                                    .storage
                                    .as_ref()
                                    .unwrap()
                                    .root()
                                    .join("settings/workspace-before-apply.json"),
                            )
                            .map_err(|e| e.to_string())?;
                            self.stage_workspace(s, None);
                            return Ok(());
                        }
                        _ => return Ok(()),
                    }
                    next.current = self.capture_workspace();
                    if let Some(storage) = &self.storage {
                        next.save(storage).map_err(|e| e.to_string())?;
                    }
                    self.workspaces.store = next;
                    Ok(())
                })();
                if let Err(e) = result {
                    self.status = Some(e);
                }
            }
        }
        if let Some((incoming, _)) = self.workspaces.staged.clone() {
            let mut open = true;
            let mut apply = false;
            let mut cancel = false;
            egui::Window::new("Apply workspace settings")
                .open(&mut open)
                .collapsible(false)
                .vscroll(true)
                .scroll_bar_visibility(crate::ui_behavior::scroll(ctx))
                .show(ctx, |ui| {
                    ui.label(
                        "Choose groups to apply. Current settings are backed up before applying.",
                    );
                    let s = &mut self.workspaces.scope;
                    crate::help::checkbox(
                        ui,
                        &mut s.layout,
                        "Panel layout and appearance",
                        "workspace",
                    );
                    crate::help::checkbox(ui, &mut s.keys, "Shortcuts", "workspace");
                    crate::help::checkbox(ui, &mut s.bars, "Panel command bars", "workspace");
                    crate::help::checkbox(
                        ui,
                        &mut s.other,
                        "Tool, color, test painting, interface, aids and image quality",
                        "workspace",
                    );
                    crate::help::checkbox(
                        ui,
                        &mut self.workspaces.save_before,
                        "Also save the current unsaved layout as a named workspace",
                        "workspace",
                    );
                    if self.workspaces.save_before {
                        super::edit::text(ui, &mut self.workspaces.before_name, "workspace");
                    }
                    match self.capture_workspace().merge(
                        &incoming,
                        self.workspaces.scope,
                        &self.display.monitors,
                    ) {
                        Ok((_, reports)) => {
                            for report in reports {
                                ui.colored_label(egui::Color32::YELLOW, report);
                            }
                        }
                        Err(e) => {
                            ui.colored_label(egui::Color32::LIGHT_RED, e.to_string());
                        }
                    }
                    ui.horizontal(|ui| {
                        let a = crate::help::button(ui, "Apply selected groups", "workspace");
                        let c = crate::help::button(ui, "Cancel workspace apply", "workspace");
                        apply = a.clicked();
                        cancel = c.clicked();
                        #[cfg(test)]
                        {
                            self.workspaces.apply_button = Some(a.rect);
                            self.workspaces.cancel_button = Some(c.rect);
                        }
                    });
                });
            if cancel || !open || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
                self.workspaces.staged = None;
            } else if apply {
                let name = self
                    .workspaces
                    .save_before
                    .then(|| self.workspaces.before_name.trim().to_string());
                if let Err(e) = self.apply_workspace(self.workspaces.scope, name.as_deref()) {
                    self.status = Some(e);
                }
            }
        }
    }
    pub(super) fn floating_panels(&mut self, ctx: &egui::Context) {
        let order = self.workspaces.store.current.order;
        for kind in order {
            let index = kind.index();
            let mut p = self.workspaces.store.current.placements[index].clone();
            if p.dock != Dock::Floating || !self.panels.settings.panels[index].visible {
                continue;
            }
            if let Some(reason) = p.recover(&self.display.monitors) {
                self.workspaces
                    .reports
                    .push(format!("{}: {reason}", kind.label()));
            }
            let monitor =
                drawer_core::display_view::selected(&self.display.monitors, p.monitor.as_deref())
                    .cloned();
            let locks = self.workspaces.store.current.layout.panel_locks[index];
            let titles = self.workspaces.store.current.layout.titles;
            if let Some(monitor) = monitor {
                let reposition =
                    self.workspaces.placed[index].as_ref() != Some(&(p.clone(), monitor.clone()));
                let mut builder = egui::ViewportBuilder::default()
                    .with_title(format!("Drawer — {}", kind.label()))
                    .with_inner_size([p.size[0] as f32, p.size[1] as f32])
                    .with_min_inner_size([80.0, 80.0])
                    .with_decorations(titles)
                    .with_resizable(!(locks.width && locks.height))
                    .with_visible(self.smoke_output.is_none());
                builder.monitor = Some(monitor.index);
                ctx.show_viewport_immediate(
                    egui::ViewportId::from_hash_of(("drawer-panel", index)),
                    builder,
                    |ui, class| {
                        if class != egui::ViewportClass::EmbeddedWindow {
                            self.workspaces.native_panels += 1;
                            let ppp = ui.ctx().pixels_per_point() as f64;
                            let current =
                                egui::vec2((p.size[0] / ppp) as f32, (p.size[1] / ppp) as f32);
                            ui.ctx()
                                .send_viewport_cmd(egui::ViewportCommand::MinInnerSize(
                                    egui::vec2(
                                        if locks.width {
                                            current.x
                                        } else {
                                            (80.0 / ppp) as f32
                                        },
                                        if locks.height {
                                            current.y
                                        } else {
                                            (80.0 / ppp) as f32
                                        },
                                    ),
                                ));
                            ui.ctx()
                                .send_viewport_cmd(egui::ViewportCommand::MaxInnerSize(
                                    egui::vec2(
                                        if locks.width {
                                            current.x
                                        } else {
                                            (4096.0 / ppp) as f32
                                        },
                                        if locks.height {
                                            current.y
                                        } else {
                                            (4096.0 / ppp) as f32
                                        },
                                    ),
                                ));
                            if locks.position {
                                ui.ctx()
                                    .send_viewport_cmd(egui::ViewportCommand::OuterPosition(
                                        egui::pos2(
                                            (p.position[0] / ppp) as f32,
                                            (p.position[1] / ppp) as f32,
                                        ),
                                    ));
                            }
                            if reposition {
                                ui.ctx()
                                    .send_viewport_cmd(egui::ViewportCommand::SetMonitor(
                                        monitor.index,
                                    ));
                                ui.ctx()
                                    .send_viewport_cmd(egui::ViewportCommand::OuterPosition(
                                        egui::pos2(
                                            (p.position[0] / ppp) as f32,
                                            (p.position[1] / ppp) as f32,
                                        ),
                                    ));
                                ui.ctx().send_viewport_cmd(egui::ViewportCommand::InnerSize(
                                    egui::vec2((p.size[0] / ppp) as f32, (p.size[1] / ppp) as f32),
                                ));
                            } else {
                                ui.ctx().input(|i| {
                                    if !locks.position
                                        && let Some(r) = i.viewport().outer_rect
                                    {
                                        p.position = [r.min.x as f64 * ppp, r.min.y as f64 * ppp];
                                    }
                                    if let Some(r) = i.viewport().inner_rect {
                                        let actual =
                                            [r.width() as f64 * ppp, r.height() as f64 * ppp]
                                                .map(|n| n.clamp(80.0, 4096.0));
                                        if !locks.width {
                                            p.size[0] = actual[0];
                                        }
                                        if !locks.height
                                            && !self.ui_behavior.hidden[index].collapsed
                                            && self.ui_behavior.resize_skip[index] == 0
                                        {
                                            p.size[1] = actual[1];
                                        }
                                    }
                                });
                            }
                        }
                        if ui.ctx().input(|i| i.viewport().close_requested()) {
                            self.panels.settings.panels[index].visible = false;
                        }
                        egui::CentralPanel::default().show(ui, |ui| {
                            egui::ScrollArea::vertical()
                                .scroll_bar_visibility(crate::ui_behavior::scroll(ui.ctx()))
                                .show(ui, |ui| {
                                    ui.add_enabled_ui(
                                        self.workspaces.staged.is_none()
                                            && !self.brush_presets.open
                                            && !self.textures.modal()
                                            && !self.lines.modal(),
                                        |ui| self.show_panel(ui, kind),
                                    )
                                });
                        });
                        let collapsed = self.ui_behavior.hidden[index].collapsed;
                        if class != egui::ViewportClass::EmbeddedWindow {
                            if collapsed {
                                ui.ctx().send_viewport_cmd(egui::ViewportCommand::InnerSize(
                                    egui::vec2(
                                        (p.size[0] / ui.ctx().pixels_per_point() as f64) as f32,
                                        80.0,
                                    ),
                                ));
                                self.ui_behavior.resize_skip[index] = 2;
                            } else if self.ui_behavior.resize_skip[index] > 0 {
                                ui.ctx().send_viewport_cmd(egui::ViewportCommand::InnerSize(
                                    egui::vec2(
                                        (p.size[0] / ui.ctx().pixels_per_point() as f64) as f32,
                                        (p.size[1] / ui.ctx().pixels_per_point() as f64) as f32,
                                    ),
                                ));
                                self.ui_behavior.resize_skip[index] -= 1;
                            }
                        }
                    },
                );
                // Header docking controls can change the dock while this window is open.
                p.dock = self.workspaces.store.current.placements[index].dock;
                self.workspaces.placed[index] = Some((p.clone(), monitor.clone()));
            } else {
                let mut visible = true;
                let response = egui::Window::new(format!("{} — floating", kind.label()))
                    .id(egui::Id::new(("floating-panel", index)))
                    .open(&mut visible)
                    .default_size([
                        (p.size[0] / ctx.pixels_per_point() as f64) as f32,
                        (p.size[1] / ctx.pixels_per_point() as f64) as f32,
                    ])
                    .title_bar(titles)
                    .movable(!locks.position)
                    .resizable([!locks.width, !locks.height])
                    .current_pos([
                        (p.position[0] / ctx.pixels_per_point() as f64) as f32,
                        (p.position[1] / ctx.pixels_per_point() as f64) as f32,
                    ])
                    .constrain(true)
                    .vscroll(true)
                    .scroll_bar_visibility(crate::ui_behavior::scroll(ctx))
                    .show(ctx, |ui| {
                        ui.add_enabled_ui(
                            self.workspaces.staged.is_none()
                                && !self.brush_presets.open
                                && !self.textures.modal()
                                && !self.lines.modal(),
                            |ui| self.show_panel(ui, kind),
                        )
                    });
                if let Some(r) = response {
                    let ppp = ctx.pixels_per_point() as f64;
                    if !locks.position {
                        p.position = [
                            r.response.rect.min.x as f64 * ppp,
                            r.response.rect.min.y as f64 * ppp,
                        ];
                    }
                    if !locks.width {
                        p.size[0] = (r.response.rect.width() as f64 * ppp).clamp(80.0, 4096.0);
                    }
                    if !locks.height {
                        p.size[1] = (r.response.rect.height() as f64 * ppp).clamp(80.0, 4096.0);
                    }
                }
                self.panels.settings.panels[index].visible = visible;
            }
            p.dock = self.workspaces.store.current.placements[index].dock;
            self.workspaces.store.current.placements[index] = p;
        }
    }
    pub(super) fn scratchpad(&mut self, ctx: &egui::Context) {
        if !self.workspaces.store.current.scratchpad.visible {
            self.workspaces.cancel_pad();
            return;
        }
        let drawing = self.workspaces.staged.is_none()
            && !self.cursors.open
            && !self.show_new_canvas
            && self.pending_close.is_none()
            && self.sizing_dialog.is_none()
            && self.import_dialog.is_none()
            && self.movie_dialog.is_none()
            && self.paste_dialog.is_none()
            && self.revert_pending.is_none()
            && !self.controls.open
            && !self.tool_input.open
            && !self.native_input.open
            && !self.pressure.open
            && !self.replay.open
            && !self.acquisition.open
            && !self.properties.open
            && !self.brush_presets.open
            && !self.textures.modal()
            && !self.lines.modal()
            && !self.ui_behavior.open;
        let mut visible = true;
        egui::Window::new("Popup test painting")
            .open(&mut visible)
            .resizable(false)
            .show(ctx, |ui| {
                ui.label(
                    "Independent test area — no document edits. Uses current tool, color and size.",
                );
                let size = workspaces::PAD_SIZE;
                let image = egui::ColorImage::new(
                    size,
                    self.workspaces
                        .store
                        .current
                        .scratchpad
                        .pixels
                        .iter()
                        .map(|p| egui::Color32::from_rgba_unmultiplied(p[0], p[1], p[2], p[3]))
                        .collect(),
                );
                let mut texture = self
                    .workspaces
                    .pad_texture
                    .get_or_insert_with(|| {
                        ctx.load_texture(
                            "workspace-test-painting",
                            image.clone(),
                            egui::TextureOptions::NEAREST,
                        )
                    })
                    .clone();
                let response = ui.add(
                    egui::Image::new(&texture)
                        .fit_to_exact_size(egui::vec2(384.0, 288.0))
                        .sense(egui::Sense::drag()),
                );
                #[cfg(test)]
                {
                    self.workspaces.pad_rect = Some(response.rect);
                }
                self.workspaces.pad_check_rect = Some(response.rect);
                let owning = response.is_pointer_button_down_on()
                    || response.drag_stopped_by(egui::PointerButton::Primary);
                if drawing && self.workspaces.focused && owning {
                    let events = ui.input(|i| i.events.clone());
                    for event in events {
                        let mut sample = None;
                        let mut finish = false;
                        match event {
                            egui::Event::PointerButton {
                                pos,
                                button: egui::PointerButton::Primary,
                                pressed: true,
                                ..
                            } if response.rect.contains(pos) => {
                                self.workspaces.pad_trace.clear();
                                let n = BrushTip::ALL
                                    .iter()
                                    .position(|t| *t == self.selected)
                                    .unwrap();
                                let anchor = [
                                    (pos.x - response.rect.min.x) as f64 / 3.0,
                                    (pos.y - response.rect.min.y) as f64 / 3.0,
                                ];
                                let clone_stamp = match if self
                                    .workspaces
                                    .store
                                    .current
                                    .color_modes
                                    .transparent
                                {
                                    Ok(Default::default())
                                } else {
                                    self.workspaces
                                        .store
                                        .current
                                        .input
                                        .clone_stamp
                                        .resolve(anchor)
                                } {
                                    Ok(stamp) => stamp,
                                    Err(e) => {
                                        self.status = Some(e);
                                        continue;
                                    }
                                };
                                let mut style = drawer_core::painting::Style {
                                    clone_stamp,
                                    variation: self.workspaces.store.current.input.variations[n],
                                    pattern: self.workspaces.store.current.input.patterns[n],
                                    dynamics: self.workspaces.store.current.input.dynamics[n],
                                    sequence: self.workspaces.store.current.input.sequences[n],
                                    filters: self.workspaces.store.current.input.filters[n],
                                    profile: {
                                        let mut p = self.workspaces.store.current.input.profiles[n];
                                        p.transparent |=
                                            self.workspaces.store.current.color_modes.transparent;
                                        p
                                    },
                                    texture: self.workspaces.store.current.input.textures[n],
                                    material: self.workspaces.store.current.input.materials[n],
                                    shape: drawer_core::brush_cursor::Shape {
                                        size: self.panels.settings.brush_size,
                                        aspect: self.panels.settings.brush_aspect,
                                        angle: self.panels.settings.brush_angle,
                                    },
                                    tip: self.selected,
                                    color: self.references.session.workspace.color,
                                    smoothing: self.workspaces.store.current.input.smoothing[n],
                                    opacity: self.panels.settings.brush_opacity[n],
                                    hardness: self.panels.settings.brush_hardness[n],
                                    spacing: self.panels.settings.brush_spacing[n],
                                    pressure: {
                                        let mut mapping =
                                            self.workspaces.store.current.pressure.tools[n];
                                        mapping.device_response =
                                            self.workspaces.store.current.pressure.calibration(
                                                drawer_core::pointer_input::Source::MOUSE,
                                            );
                                        mapping
                                    },
                                };
                                style.shape.size = style
                                    .pattern
                                    .behavior
                                    .document_size(
                                        style.shape.size,
                                        3.0 * ctx.pixels_per_point() as f64,
                                    )
                                    .unwrap_or(style.shape.size);
                                match PadPaint::begin(
                                    &self.workspaces.store.current.scratchpad.pixels,
                                    style,
                                    self.materials.library.clone(),
                                ) {
                                    Ok(paint) => self.workspaces.pad_style = Some(paint),
                                    Err(e) => self.status = Some(e),
                                }
                                sample = Some(pos);
                            }
                            egui::Event::PointerMoved(pos)
                                if self.workspaces.pad_style.is_some() =>
                            {
                                sample = Some(pos)
                            }
                            egui::Event::PointerButton {
                                pos,
                                button: egui::PointerButton::Primary,
                                pressed: false,
                                ..
                            } => {
                                sample = Some(pos);
                                finish = true;
                            }
                            egui::Event::Key {
                                key: egui::Key::Escape,
                                pressed: true,
                                ..
                            }
                            | egui::Event::PointerGone
                            | egui::Event::WindowFocused(false) => {
                                self.workspaces.cancel_pad();
                                self.workspaces.pad_trace.clear();
                            }
                            _ => {}
                        }
                        if let Some(pos) = sample
                            && let Some(paint) = &mut self.workspaces.pad_style
                        {
                            let point = [
                                (pos.x - response.rect.min.x) as f64 / 3.0,
                                (pos.y - response.rect.min.y) as f64 / 3.0,
                            ];
                            match paint.sample(point, ctx.input(|i| (i.time.max(0.0) * 1e6) as u64))
                            {
                                Ok(()) => {
                                    self.workspaces.pad_trace = paint.stroke.trace();
                                    self.workspaces.pad_point = self.workspaces.pad_trace.corrected;
                                    self.workspaces.store.current.scratchpad.pixels =
                                        paint.preview();
                                }
                                Err(e) => {
                                    self.status = Some(e);
                                    self.workspaces.cancel_pad();
                                }
                            }
                        }
                        if finish {
                            if let Some(paint) = self.workspaces.pad_style.take() {
                                let before = paint.original().to_vec();
                                let (pixels, error) = paint.finish();
                                if error.is_none() {
                                    self.workspaces.pad_history.record(before, &pixels);
                                }
                                self.workspaces.store.current.scratchpad.pixels = pixels;
                                if let Some(e) = error {
                                    self.status = Some(e);
                                }
                            }
                            self.workspaces.pad_point = None;
                            self.workspaces.pad_trace.clear();
                        }
                    }
                } else {
                    self.workspaces.cancel_pad();
                    self.workspaces.pad_point = None;
                    self.workspaces.pad_trace.clear();
                }
                if let Some(paint) = &mut self.workspaces.pad_style
                    && paint.replay.is_some()
                {
                    ctx.request_repaint_after(std::time::Duration::from_millis(16));
                    match paint.tick(ctx.input(|i| (i.time.max(0.0) * 1e6) as u64)) {
                        Ok(true) => {
                            self.workspaces.pad_trace = paint.stroke.trace();
                            self.workspaces.store.current.scratchpad.pixels = paint.preview();
                        }
                        Ok(false) => {}
                        Err(e) => {
                            self.status = Some(e);
                            self.workspaces.cancel_pad();
                        }
                    }
                }
                if let Some(paint) = &self.workspaces.pad_style
                    && self.workspaces.store.current.behavior.brush_cursor
                {
                    let style = paint.stroke.style();
                    let n = BrushTip::ALL.iter().position(|t| *t == style.tip).unwrap();
                    let _ = crate::cursor_settings::paint(
                        &ui.painter_at(response.rect),
                        &self.workspaces.pad_trace,
                        self.workspaces.store.current.cursors.tools[n],
                        paint.stroke.shape(),
                        crate::cursor_settings::Projection {
                            square: paint.stroke.mask_is_square(),
                            view: drawer_core::view::View {
                                zoom: 3.0 * ui.ctx().pixels_per_point() as f64,
                                ..Default::default()
                            },
                            size: drawer_core::workspaces::PAD_SIZE.map(|v| v as f64),
                            center: [
                                response.rect.center().x as f64,
                                response.rect.center().y as f64,
                            ],
                            ppp: ui.ctx().pixels_per_point() as f64,
                        },
                        self.canvas_view.icon(n),
                    );
                }
                ui.horizontal(|ui| {
                    let undo = ui.button("Undo test stroke");
                    self.workspaces.pad_actions[0] = Some(undo.rect);
                    if undo.clicked() {
                        self.workspaces.cancel_pad();
                        self.workspaces
                            .pad_history
                            .undo(&mut self.workspaces.store.current.scratchpad.pixels);
                    }
                    let redo = ui.button("Redo test stroke");
                    self.workspaces.pad_actions[1] = Some(redo.rect);
                    if redo.clicked() {
                        self.workspaces.cancel_pad();
                        self.workspaces
                            .pad_history
                            .redo(&mut self.workspaces.store.current.scratchpad.pixels);
                    }
                });
                if crate::help::button(ui, "Clear test area", "workspace").clicked() {
                    self.workspaces.cancel_pad();
                    let before = self.workspaces.store.current.scratchpad.pixels.clone();
                    self.workspaces.store.current.scratchpad.pixels.fill([0; 4]);
                    self.workspaces
                        .pad_history
                        .record(before, &self.workspaces.store.current.scratchpad.pixels);
                }
                let image = egui::ColorImage::new(
                    size,
                    self.workspaces
                        .store
                        .current
                        .scratchpad
                        .pixels
                        .iter()
                        .map(|p| egui::Color32::from_rgba_unmultiplied(p[0], p[1], p[2], p[3]))
                        .collect(),
                );
                texture.set(image, egui::TextureOptions::NEAREST);
            });
        if !visible {
            self.workspaces.cancel_pad();
        }
        self.workspaces.store.current.scratchpad.visible = visible;
    }
}

pub(super) fn verify_continuous_pad(mut style: drawer_core::painting::Style) -> Result<(), String> {
    style.shape.size = 1.0;
    style.color = [220, 70, 30, 64];
    style.pattern = Default::default();
    style.pattern.behavior.continuous = true;
    style.pattern.behavior.rate = 20;
    style.pressure.size = false;
    style.pressure.opacity = true;
    let pixels = vec![[0; 4]; workspaces::PAD_SIZE[0] * workspaces::PAD_SIZE[1]];
    let at = 8 * workspaces::PAD_SIZE[0] + 8;
    let mut paint = PadPaint::begin(&pixels, style, Default::default())?;
    paint.sample([8.5, 8.5], 0)?;
    if paint.preview()[at][3] != 64 || !paint.tick(100000)? || paint.preview()[at][3] != 148 {
        return Err("Test painting area did not emit while stationary".into());
    }
    let (finished, error) = paint.finish();
    if error.is_some() || finished[at][3] != 148 {
        return Err("Test painting release added an extra timed stamp".into());
    }
    let mut paint = PadPaint::begin(&pixels, style, Default::default())?;
    paint.sample([8.5, 8.5], 0)?;
    paint.tick(100000)?;
    if paint.cancel() != pixels {
        return Err("Test painting cancel retained timed pixels".into());
    }
    Ok(())
}

pub(super) fn shortcut(
    input: &mut egui::InputState,
    bindings: &[workspaces::Binding],
) -> Option<(String, egui::Key, bool)> {
    let found = input.events.iter().enumerate().find_map(|(index, event)| {
        let egui::Event::Key {
            key,
            pressed: true,
            repeat,
            modifiers,
            ..
        } = event
        else {
            return None;
        };
        bindings.iter().find_map(|b| {
            if *repeat
                && (["help", "screen-mode"].contains(&b.command.as_str())
                    || ["cycle-next:", "cycle-prev:", "quick:", "radial:"]
                        .iter()
                        .any(|prefix| b.command.starts_with(prefix)))
            {
                return None;
            }
            let expected = egui::Key::from_name(&b.key)?;
            (expected == *key
                && modifiers.ctrl == b.ctrl
                && modifiers.shift == b.shift
                && modifiers.alt == b.alt
                && !modifiers.mac_cmd)
                .then(|| (index, b.command.clone(), *key, *repeat))
        })
    });
    found.map(|(i, a, key, repeat)| {
        input.events.remove(i);
        (a, key, repeat)
    })
}

pub(super) fn verify_test_history(root: &std::path::Path) -> Result<(), String> {
    let ctx = egui::Context::default();
    let mut app = Drawer::new(&ctx, None);
    app.simple_view = true;
    app.create_canvas(CanvasSpec {
        width: 4,
        height: 4,
        dpi: 300.0,
        background: Background::Transparent,
    })
    .map_err(|e| e.to_string())?;
    drawer_core::files::save(
        &mut app.documents[0],
        &root.join("test-area-keeps-document.drawer"),
    )
    .map_err(|e| e.to_string())?;
    let original = app.documents[0].snapshot();
    app.workspaces.store.current.scratchpad.visible = true;
    app.references.session.workspace.color = [200, 50, 30, 128];
    let mut time = 0.0;
    let mut frame = |app: &mut Drawer, events| {
        time += 0.1;
        super::input_replay::frame(&ctx, app, time, events);
    };
    frame(&mut app, vec![]);
    frame(&mut app, vec![]);
    let rect = app
        .workspaces
        .pad_rect_for_check()
        .ok_or("Test painting rect missing")?;
    let a = rect.min + egui::vec2(30.0, 30.0);
    let b = a + egui::vec2(18.0, 0.0);
    let click = |p, pressed| {
        vec![
            egui::Event::PointerMoved(p),
            egui::Event::PointerButton {
                pos: p,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Default::default(),
            },
        ]
    };
    frame(&mut app, click(a, true));
    frame(&mut app, vec![egui::Event::PointerMoved(b)]);
    frame(&mut app, click(b, false));
    let painted = app.workspaces.store.current.scratchpad.pixels.clone();
    if app.workspaces.pad_history.counts() != (1, 0) || painted.iter().all(|p| *p == [0; 4]) {
        return Err("Test painting stroke history missing".into());
    }
    for (n, empty) in [(0, true), (1, false)] {
        let p = app.workspaces.pad_actions[n]
            .ok_or("Test undo/redo button missing")?
            .center();
        frame(&mut app, click(p, true));
        frame(&mut app, click(p, false));
        if empty {
            if app
                .workspaces
                .store
                .current
                .scratchpad
                .pixels
                .iter()
                .any(|p| *p != [0; 4])
            {
                return Err("Test-only Undo failed".into());
            }
        } else if app.workspaces.store.current.scratchpad.pixels != painted {
            return Err("Test-only Redo failed".into());
        }
    }
    if app.documents[0].snapshot() != original
        || app.documents[0].dirty
        || app.documents[0].history_counts() != (0, 0)
    {
        return Err("Test-area history changed the real document".into());
    }
    Ok(())
}
