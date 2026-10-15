#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use drawer_core::assets::{APP_NAME, BrushTip, application_icon};
use drawer_core::document::{
    Background, CanvasSpec, CreateError, DEFAULT_DOCUMENT_BUDGET, Document,
};
use drawer_core::storage::UserStorage;
use eframe::egui;
use std::{path::PathBuf, sync::Arc};
mod acquisition;
mod action_controls;
mod aid_panel;
mod canvas_view;
mod cursor_settings;
mod display_pane;
mod drawing;
mod edit;
mod help;
mod image_import_dialog;
mod information;
mod input_replay;
mod layout;
mod movie_dialog;
mod navigator_panel;
mod new_canvas;
mod panels;
mod paste_dialog;
mod pointer_input;
mod preferences;
mod pressure_settings;
mod quality;
mod reference_pane;
mod revert;
mod screen_mode;
mod size_fields;
mod sizing_dialog;
mod tool_input;
mod tool_properties;
mod ui_behavior;
mod ui_scale;
mod view_commands;
mod workspace;
mod workspaces;
use new_canvas::NewCanvas;

struct Drawer {
    selected: BrushTip,
    previews: Vec<egui::TextureHandle>,
    smoke_output: Option<PathBuf>,
    frames: usize,
    storage: Option<UserStorage>,
    status: Option<String>,
    documents: Vec<Document>,
    tabs: Vec<workspace::DocumentTab>,
    active_tab: Option<usize>,
    next_tab_id: u64,
    pending_close: Option<u64>,
    quit_requested: bool,
    quit_ready: bool,
    quit_approved: std::collections::HashSet<u64>,
    #[cfg(test)]
    close_buttons: std::collections::HashMap<u64, egui::Rect>,
    #[cfg(test)]
    transform_buttons: std::collections::HashMap<&'static str, egui::Rect>,
    #[cfg(test)]
    cancel_close_button: Option<egui::Rect>,
    #[cfg(test)]
    discard_close_button: Option<egui::Rect>,
    #[cfg(test)]
    save_close_button: Option<egui::Rect>,
    active: Option<usize>,
    new_canvas: NewCanvas,
    show_new_canvas: bool,
    sizing_dialog: Option<sizing_dialog::SizingDialog>,
    canvas_view: canvas_view::CanvasView,
    default_view: drawer_core::view::View,
    references: reference_pane::ReferencePane,
    aids: aid_panel::AidPanel,
    recent: drawer_core::recent::RecentFiles,
    import_dialog: Option<image_import_dialog::ImportDialog>,
    protected_sources: std::collections::HashSet<PathBuf>,
    display: display_pane::DisplayPane,
    size_presets: size_fields::Presets,
    paste_dialog: Option<paste_dialog::PasteDialog>,
    interpolation: drawer_core::interpolation::Settings,
    show_interpolation: bool,
    quality_undo: Vec<drawer_core::interpolation::Settings>,
    quality_redo: Vec<drawer_core::interpolation::Settings>,
    saved_layout: drawer_core::view_layout::Layout,
    last_layout: drawer_core::view_layout::Layout,
    view_session: u64,
    navigator_saved: Option<drawer_core::navigator_preferences::Preferences>,
    navigator_undo: Vec<drawer_core::navigator_preferences::Preferences>,
    navigator_redo: Vec<drawer_core::navigator_preferences::Preferences>,
    show_navigator_settings: bool,
    movie_dialog: Option<movie_dialog::MovieDialog>,
    revert_pending: Option<u64>,
    panels: panels::Panels,
    interface: ui_scale::Interface,
    workspaces: workspaces::Workspaces,
    controls: action_controls::Controls,
    tool_input: tool_input::ToolInput,
    properties: tool_properties::Properties,
    preferences: preferences::Preferences,
    drawing: drawing::Drawing,
    native_input: pointer_input::Native,
    #[cfg(windows)]
    native_mouse: Option<drawer_platform::Input>,
    pressure: pressure_settings::Editor,
    replay: input_replay::Editor,
    acquisition: acquisition::Acquisition,
    help: help::Help,
    ui_behavior: ui_behavior::Behavior,
    cursors: cursor_settings::Cursors,
    information: information::Information,
    layout: layout::Layout,
    simple_view: bool,
    screen_modes_seen: u8,
    #[cfg(test)]
    screen_button: Option<egui::Rect>,
    #[cfg(test)]
    commands_button: Option<egui::Rect>,
    #[cfg(test)]
    new_button: Option<egui::Rect>,
    #[cfg(test)]
    create_button: Option<egui::Rect>,
    #[cfg(test)]
    cancel_button: Option<egui::Rect>,
}

impl Drawer {
    fn new(ctx: &egui::Context, smoke_output: Option<PathBuf>) -> Self {
        let korean_font = help::setup_ui_font(ctx);
        let mut style = (*ctx.style_of(egui::Theme::Dark)).clone();
        style.visuals = egui::Visuals::dark();
        style.visuals.selection.bg_fill = egui::Color32::from_rgb(48, 126, 105);
        style.spacing.item_spacing = egui::vec2(10.0, 10.0);
        ctx.set_style_of(egui::Theme::Dark, style);
        ctx.set_theme(egui::ThemePreference::Dark);
        let previews = BrushTip::ALL
            .into_iter()
            .map(|tip| {
                let pixels = tip
                    .mask(64)
                    .into_iter()
                    .map(egui::Color32::from_white_alpha)
                    .collect();
                ctx.load_texture(
                    tip.name(),
                    egui::ColorImage::new([64, 64], pixels),
                    egui::TextureOptions::LINEAR,
                )
            })
            .collect();
        let mut app = Self {
            selected: BrushTip::Pen,
            previews,
            smoke_output,
            frames: 0,
            storage: None,
            status: None,
            documents: Vec::new(),
            tabs: Vec::new(),
            active_tab: None,
            next_tab_id: 0,
            pending_close: None,
            quit_requested: false,
            quit_ready: false,
            quit_approved: workspace::empty_approvals(),
            #[cfg(test)]
            close_buttons: Default::default(),
            #[cfg(test)]
            transform_buttons: Default::default(),
            #[cfg(test)]
            cancel_close_button: None,
            #[cfg(test)]
            discard_close_button: None,
            #[cfg(test)]
            save_close_button: None,
            active: None,
            new_canvas: NewCanvas::default(),
            show_new_canvas: false,
            sizing_dialog: None,
            canvas_view: canvas_view::CanvasView::new(ctx),
            default_view: Default::default(),
            references: Default::default(),
            aids: Default::default(),
            recent: Default::default(),
            import_dialog: None,
            protected_sources: Default::default(),
            display: display_pane::DisplayPane::new(ctx),
            size_presets: Default::default(),
            paste_dialog: None,
            interpolation: Default::default(),
            show_interpolation: false,
            quality_undo: Vec::new(),
            quality_redo: Vec::new(),
            saved_layout: drawer_core::view_layout::Layout::empty(),
            last_layout: drawer_core::view_layout::Layout::empty(),
            navigator_saved: None,
            navigator_undo: Vec::new(),
            navigator_redo: Vec::new(),
            show_navigator_settings: false,
            movie_dialog: None,
            revert_pending: None,
            panels: panels::Panels::new(ctx),
            interface: ui_scale::Interface::new(ctx),
            workspaces: Default::default(),
            controls: Default::default(),
            tool_input: Default::default(),
            properties: Default::default(),
            preferences: Default::default(),
            drawing: Default::default(),
            native_input: Default::default(),
            #[cfg(windows)]
            native_mouse: None,
            pressure: Default::default(),
            replay: Default::default(),
            acquisition: Default::default(),
            help: help::Help::new(korean_font),
            ui_behavior: Default::default(),
            cursors: Default::default(),
            information: Default::default(),
            layout: Default::default(),
            simple_view: false,
            screen_modes_seen: 0,
            #[cfg(test)]
            screen_button: None,
            #[cfg(test)]
            commands_button: None,
            view_session: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos() as u64,
            #[cfg(test)]
            new_button: None,
            #[cfg(test)]
            create_button: None,
            #[cfg(test)]
            cancel_button: None,
        };
        if app.smoke_output.is_some() {
            app.create_canvas(CanvasSpec {
                width: 7,
                height: 5,
                dpi: 300.0,
                background: Background::Transparent,
            })
            .unwrap();
        }
        app
    }

    fn create_canvas(&mut self, spec: CanvasSpec) -> Result<(), CreateError> {
        if self.tabs.len() >= 256 {
            return Err(CreateError::ViewLimit);
        }
        let used: usize = self.documents.iter().map(Document::resident_bytes).sum();
        let mut document = Document::create(spec, DEFAULT_DOCUMENT_BUDGET.saturating_sub(used))?;
        document.view = self.default_view;
        self.documents
            .try_reserve(1)
            .map_err(|_| CreateError::Allocation)?;
        self.tabs
            .try_reserve(1)
            .map_err(|_| CreateError::Allocation)?;
        self.add_document(document);
        Ok(())
    }

    fn new_canvas_dialog(&mut self, ctx: &egui::Context) {
        if !self.show_new_canvas || self.pending_close.is_some() || self.sizing_dialog.is_some() {
            return;
        }
        let mut open = true;
        let mut create = false;
        let mut cancel = false;
        egui::Window::new("New canvas")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                size_fields::fields(ui, &mut self.new_canvas.size, true, None);
                self.size_presets
                    .show(ui, &mut self.new_canvas.size, self.storage.as_ref());
                crate::help::checkbox(
                    ui,
                    &mut self.new_canvas.transparent,
                    "Transparent background",
                    "size",
                );
                ui.add_enabled_ui(!self.new_canvas.transparent, |ui| {
                    ui.horizontal(|ui| {
                        ui.label("Background");
                        ui.color_edit_button_srgb(&mut self.new_canvas.color);
                    });
                });
                if let Some(error) = &self.new_canvas.error {
                    ui.colored_label(egui::Color32::LIGHT_RED, error);
                }
                ui.horizontal(|ui| {
                    let create_response = crate::help::button(ui, "Create", "size");
                    let cancel_response = crate::help::button(ui, "Cancel", "size");
                    create = create_response.clicked();
                    cancel = cancel_response.clicked();
                    #[cfg(test)]
                    {
                        self.create_button = Some(create_response.rect);
                        self.cancel_button = Some(cancel_response.rect);
                    }
                });
            });
        if create {
            match self
                .new_canvas
                .spec()
                .and_then(|spec| self.create_canvas(spec))
            {
                Ok(()) => {
                    self.show_new_canvas = false;
                    ctx.request_repaint();
                }
                Err(error) => self.new_canvas.error = Some(error.to_string()),
            }
        }
        if !open || cancel || ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
            self.show_new_canvas = false;
            ctx.request_repaint();
        }
    }

    fn with_storage(mut self, storage: UserStorage) -> Self {
        match drawer_core::panels::Settings::load(&storage) {
            Ok(s) => self.panels.settings = s,
            Err(e) => self.status = Some(format!("Panel settings reset: {e}")),
        }
        match drawer_core::navigator_preferences::Preferences::load(&storage) {
            Ok(p) => self.canvas_view.navigator = p,
            Err(e) => self.status = Some(format!("Navigator settings reset: {e}")),
        }
        match drawer_core::view_layout::Layout::load(&storage) {
            Ok(layout) => self.saved_layout = layout,
            Err(error) => self.status = Some(format!("Saved views unavailable: {error}")),
        }
        match drawer_core::interpolation::Settings::load(&storage) {
            Ok(s) => self.interpolation = s,
            Err(e) => self.status = Some(format!("Interpolation reset: {e}")),
        }
        self.size_presets.load(&storage);
        match drawer_core::display_view::Preferences::load(&storage) {
            Ok(prefs) => self.display.preferences = prefs,
            Err(error) => self.display.error = Some(format!("Display settings reset: {error}")),
        }
        match drawer_core::recent::RecentFiles::load(&storage) {
            Ok(recent) => self.recent = recent,
            Err(error) => self.status = Some(format!("Recent files reset: {error}")),
        }
        self.aids.load(&storage);
        match drawer_core::references::ReferenceSession::load(&storage) {
            Ok(session) => self.references.session = session,
            Err(error) => self.references.error = Some(error.to_string()),
        }
        match storage.load_brush() {
            Ok(brush) => {
                self.selected = brush;
                if let Err(error) = storage.save_brush(self.selected) {
                    self.status = Some(error.to_string());
                }
            }
            Err(error) => self.status = Some(error.to_string()),
        }
        match storage.load_view() {
            Ok(view) => self.default_view = view,
            Err(error) => self.status = Some(error.to_string()),
        }
        match drawer_core::ui_scale::Settings::load(&storage) {
            Ok(settings) => {
                self.interface.settings = settings;
                if let Some(mut view) = settings.default_view {
                    view.pan = view.pan.map(|p| p / self.interface.ppp);
                    self.default_view = view;
                }
            }
            Err(e) => self.status = Some(format!("Interface settings reset: {e}")),
        }
        if let Some(active) = self.active {
            self.documents[active].tool = self.selected;
        }
        self.load_workspaces(&storage);
        self.storage = Some(storage);
        self
    }

    fn show(&mut self, ui: &mut egui::Ui) {
        self.process_drawing(ui.ctx());
        self.pointer_settings(ui.ctx());
        self.pressure_settings(ui.ctx());
        self.input_replay(ui.ctx());
        self.apply_theme(ui.ctx());
        self.canvas_view.measurement_mode = self.measurement_active();
        self.canvas_view.measurement = self.measurement_line();
        if ui.ctx().zoom_factor() != self.interface.settings.zoom {
            ui.ctx().set_zoom_factor(self.interface.settings.zoom);
        }
        self.begin_input_preferences(ui.ctx());
        self.begin_context_help(ui.ctx());
        self.workspaces.input(ui.ctx());
        self.screen_mode_inputs(ui.ctx());
        self.canvas_view.simple = self.simple_view;
        self.screen_modes_seen |= if self.simple_view { 1 } else { 2 };
        self.held_tool_inputs(ui.ctx());
        self.observe_scale(ui.ctx());
        self.view_shortcuts(ui.ctx());
        self.basic_shortcuts(ui.ctx());
        self.direct_actions(ui.ctx());
        self.sync_tool_properties();
        self.canvas_view.brush_shape = drawer_core::brush_cursor::Shape {
            size: self.panels.settings.brush_size,
            aspect: self.panels.settings.brush_aspect,
            angle: self.panels.settings.brush_angle,
        };
        ui.ctx().data_mut(|d| {
            d.insert_temp(
                egui::Id::new("drawer-scrollbars"),
                self.workspaces.store.current.behavior.scrollbars,
            )
        });
        self.canvas_view.cursors = self.workspaces.store.current.cursors;
        self.canvas_view.smoothing = self.workspaces.store.current.input.smoothing;
        self.display.canvas.cursors = self.workspaces.store.current.cursors;
        self.display.canvas.smoothing = self.workspaces.store.current.input.smoothing;
        self.canvas_view.behavior = self.workspaces.store.current.behavior;
        self.display.canvas.behavior = self.workspaces.store.current.behavior;
        self.canvas_view.input_active = self.workspaces.is_focused();
        self.canvas_view.drag_pixels = self.workspaces.store.current.input.drag_pixels;
        self.display.canvas.drag_pixels = self.workspaces.store.current.input.drag_pixels;
        self.canvas_view.filter = self.interpolation.view;
        self.display.canvas.filter = self.interpolation.view;
        self.canvas_view.working = false;
        #[cfg(test)]
        {
            self.canvas_view.navigator_rect = None;
            self.layout.panel_rects.clear();
        }
        egui::CentralPanel::default().show(ui, |ui| {
            self.screen_mode_controls(ui);
            ui.add_enabled_ui(
                self.pending_close.is_none()
                    && self.sizing_dialog.is_none()
                    && self.import_dialog.is_none()
                    && self.paste_dialog.is_none()
                    && self.movie_dialog.is_none()
                    && self.revert_pending.is_none()
                    && self.workspaces.staged.is_none()
                    && !self.cursors.open
                    && !self.ui_behavior.open
                    && !self.tool_input.open
                    && !self.native_input.open
                    && !self.pressure.open
                    && !self.replay.open
                    && !self.acquisition.open
                    && !self.properties.open
                    && !self.controls.open
                    && self.controls.radial.is_none(),
                |ui| {
                    if self.simple_view || !self.workspaces.store.current.layout.menu {
                        let _menu = ui.menu_button("All tools and commands", |ui| {
                            ui.set_min_width(500.0_f32.min(ui.ctx().content_rect().width() - 40.0));
                            egui::ScrollArea::vertical()
                                .id_salt("simple-all-commands")
                                .max_height(420.0)
                                .show(ui, |ui| {
                                    ui.push_id("simple-commands", |ui| self.full_commands(ui));
                                });
                        });
                        #[cfg(test)]
                        {
                            self.commands_button = Some(_menu.response.rect);
                        }
                        self.document_tabs(ui);
                        ui.horizontal_wrapped(|ui| {
                            for tip in BrushTip::ALL {
                                if ui
                                    .selectable_label(self.selected == tip, tip.name())
                                    .clicked()
                                {
                                    self.set_tool(tip);
                                }
                            }
                        });
                        if self.simple_view {
                            self.simple_canvas(ui);
                            return;
                        }
                    }
                    if self.workspaces.store.current.layout.menu {
                        egui::ScrollArea::vertical()
                            .scroll_bar_visibility(crate::ui_behavior::scroll(ui.ctx()))
                            .id_salt("document-commands")
                            .max_height((ui.available_height() * 0.5).clamp(100.0, 360.0))
                            .show(ui, |ui| self.full_commands(ui));
                    }
                    self.docked_workspace(ui);
                },
            );
        });
        self.layout_windows(ui.ctx());
        self.layout_settings(ui.ctx());
        self.capture_information();
        self.information_panel(ui.ctx());
        if !self.simple_view {
            self.floating_panels(ui.ctx());
        }
        self.scratchpad(ui.ctx());
        self.workspace_settings(ui.ctx());
        self.command_settings(ui.ctx());
        self.input_settings(ui.ctx());
        self.property_settings(ui.ctx());
        self.preference_settings(ui.ctx());
        self.behavior_settings(ui.ctx());
        self.cursor_settings(ui.ctx());
        self.command_popups(ui.ctx());
        self.new_canvas_dialog(ui.ctx());
        self.close_dialog(ui.ctx());
        self.show_sizing(ui.ctx());
        self.show_references(ui.ctx());
        self.show_aids(ui.ctx());
        self.show_import(ui.ctx());
        self.acquisition_window(ui.ctx());
        self.show_display(ui.ctx());
        self.show_paste(ui.ctx());
        self.show_movie(ui.ctx());
        self.revert_dialog(ui.ctx());
        self.interpolation_settings(ui.ctx());
        self.persist_views(ui.ctx().pixels_per_point() as f64);
        self.navigator_settings(ui.ctx());
        self.persist_navigator();
        self.panel_settings(ui.ctx());
        self.interface_settings(ui.ctx());
        self.resolve_context_help(ui.ctx());
        self.help_window(ui.ctx());
        self.update_input_preferences(ui.ctx());
        self.persist_interface();
        self.sync_tool_properties();
        if let Some(e) = self.drawing.error.take() {
            self.status = Some(e);
            ui.ctx().request_repaint();
        }
        self.update_power();
        if let Err(e) = self.persist_workspace() {
            self.status = Some(e);
        }
        if let Err(e) = self.panels.persist(self.storage.as_ref()) {
            self.status = Some(e);
        }
    }
}

impl eframe::App for Drawer {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        self.display.poll_monitors(ui.ctx(), frame);
        if self.smoke_output.is_none()
            && !self.quit_ready
            && ui.ctx().input(|input| input.viewport().close_requested())
            && (self.drawing.pending()
                || self.documents.iter().any(|doc| doc.dirty)
                || self.sizing_dialog.is_some()
                || self.import_dialog.is_some()
                || self.paste_dialog.is_some()
                || self.movie_dialog.is_some()
                || self.revert_pending.is_some())
        {
            ui.ctx()
                .send_viewport_cmd(egui::ViewportCommand::CancelClose);
            if self.sizing_dialog.is_none()
                && self.import_dialog.is_none()
                && self.paste_dialog.is_none()
                && self.movie_dialog.is_none()
                && self.revert_pending.is_none()
            {
                self.process_drawing(ui.ctx());
                self.begin_quit();
            }
        }
        self.show(ui);
        if self.quit_ready {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
        }
        if let Some(path) = self.smoke_output.clone()
            && !self.quit_ready
        {
            self.frames += 1;
            if self.frames == 1 {
                self.create_canvas(CanvasSpec {
                    width: 5,
                    height: 3,
                    dpi: 300.0,
                    background: Background::Color([20, 80, 140]),
                })
                .unwrap();
                self.duplicate_view();
                self.request_close(self.tabs.len() - 1);
                self.request_close(self.tabs.len() - 1);
                self.add_reference_image(
                    drawer_core::references::LocalImage::from_rgba(
                        2,
                        1,
                        &[240, 30, 80, 128, 20, 40, 60, 0],
                        8,
                    )
                    .unwrap(),
                    "Smoke reference".into(),
                    None,
                )
                .unwrap();
                self.references.session.workspace.visible = true;
                self.aids.visible = true;
                self.aids.preferences.monitor_ppi = Some(96.0);
                self.aids.preferences.grid = true;
                self.workspaces.store.current.placements[0].dock =
                    drawer_core::workspaces::Dock::Floating;
                self.open_help("licenses");
                self.workspaces.store.current.layout.panel_locks[0].width = true;
                self.workspaces.store.current.layout.panel_locks[0].position = true;
                self.workspaces.store.current.behavior.auto_hide = true;
                self.workspaces.store.current.behavior.hide_delay_ms = 0;
                self.workspaces.store.current.cursors.tools[0].mode =
                    drawer_core::brush_cursor::Mode::Both;
                self.workspaces.store.current.cursors.tools[0].prediction = true;
                self.workspaces.store.current.cursors.tools[0].minimum_pixels = 4.0;
                self.workspaces.store.current.input.smoothing[0] = 0.6;
                self.workspaces.store.current.information.visible = true;
                self.workspaces.store.current.information.cmyk = true;
                self.workspaces.store.current.scratchpad.visible = true;
                self.workspaces.store.current.scratchpad.stroke(
                    [20.0, 20.0],
                    [80.0, 60.0],
                    8.0,
                    BrushTip::Pen,
                    [80, 90, 100, 255],
                );
                self.display.preferences.visible = true;
                self.interpolation.view = drawer_core::interpolation::Filter::Bicubic;
                self.interpolation
                    .save(self.storage.as_ref().unwrap())
                    .unwrap();
                self.display.preferences.fullscreen = false;
                self.display
                    .preferences
                    .save(self.storage.as_ref().unwrap())
                    .unwrap();
            }
            if self.frames == 3 {
                self.workspaces.store.current.layout.titles = false;
                self.toggle_screen_mode();
                self.workspaces.store.current.information.theme =
                    drawer_core::information::Theme::Light;
                self.cancel_close();
                self.select_tab(0);
                self.begin_sizing(sizing_dialog::Mode::Dpi);
                self.sizing_dialog.as_mut().unwrap().dpi = "600".into();
            }
            if self.frames == 4 {
                self.smoke_start_sizing(ui.ctx());
            }
            if self.frames == 5 {
                // Give only this hidden smoke window an input context for the next-pass probe.
                let probe_id = egui::Id::new("drawer-smoke-ime-probe");
                ui.memory_mut(|m| m.request_focus(probe_id));
                let mut probe = String::new();
                ui.add(egui::TextEdit::singleline(&mut probe).id(probe_id));
                self.workspaces.store.current.layout.titles = true;
                self.workspaces.store.current.layout.arrangement =
                    drawer_core::layout::Arrangement::Tile;
                self.toggle_screen_mode();
            }
            if self.frames > 120 {
                fail_smoke();
            }
            if self.frames >= 6 && self.sizing_dialog.is_none() {
                let native_ime = preferences::native_probe(frame).unwrap_or_else(|e| {
                    eprintln!("IME probe: {e}");
                    fail_smoke()
                });
                if self.layout.rendered_windows != self.tabs.len()
                    || !self.workspaces.store.current.layout.panel_locks[0].position
                {
                    fail_smoke();
                }
                if !self
                    .workspaces
                    .store
                    .current
                    .properties
                    .as_ref()
                    .is_some_and(|s| s.valid())
                {
                    fail_smoke();
                }
                if !help::has_context(ui.ctx()) {
                    fail_smoke();
                }
                if self.screen_modes_seen != 3 || self.simple_view {
                    fail_smoke();
                }
                let validated = self.documents.len() == 2
                    && self.tabs.len() == 2
                    && self.pending_close.is_none()
                    && self.documents[0].dpi == 600.0;
                if !validated {
                    fail_smoke();
                }
                if !self.documents[0].undo() || self.documents[0].dpi != 300.0 {
                    fail_smoke();
                }
                let original = self.documents[0].snapshot();
                self.aids.spacing = ["3".into(), "4".into()];
                self.apply_grid().unwrap();
                self.aids.guide_position = "1".into();
                self.add_guide(true).unwrap();
                self.print_size().unwrap();
                if self.documents[0].scene.guides.len() != 1
                    || self.documents[0].view.zoom != 0.32
                    || !self.documents[0].undo()
                    || !self.documents[0].undo()
                    || self.documents[0].snapshot() != original
                {
                    fail_smoke();
                }
                self.open_reference_document().unwrap();
                if self.documents[2].layers[0].pixels != [[240, 30, 80, 128], [20, 40, 60, 0]]
                    || self.documents[2].path.is_some()
                    || self.documents[2].history_counts() != (0, 0)
                {
                    fail_smoke();
                }
                self.document_from_clipboard(|| {
                    drawer_core::references::LocalImage::from_rgba(
                        2,
                        1,
                        &[10, 20, 30, 128, 40, 50, 60, 255],
                        8,
                    )
                    .map_err(|e| e.to_string())
                })
                .unwrap();
                let root = self.storage.as_ref().unwrap().root().to_owned();
                let first = root.join("smoke-first.drawer");
                let second = root.join("smoke-second.drawer");
                let output = root.join("smoke-export.png");
                if self.save_as_from(3, || None).unwrap() || self.documents[3].path.is_some() {
                    fail_smoke();
                }
                self.save_as_from(3, || Some(first.clone())).unwrap();
                let first_bytes = std::fs::read(&first).unwrap();
                self.save_as_from(3, || Some(second.clone())).unwrap();
                self.documents[3].edit_pixel(0, 1, 0, [80, 90, 100, 255]);
                let snapshot = self.documents[3].snapshot();
                self.export_from(3, || Some(output.clone())).unwrap();
                if self.documents[3].path != Some(second.canonicalize().unwrap())
                    || !self.documents[3].dirty
                    || self.documents[3].snapshot() != snapshot
                    || std::fs::read(&first).unwrap() != first_bytes
                {
                    fail_smoke();
                }
                let cancel = std::sync::atomic::AtomicBool::new(false);
                let source = drawer_core::image_import::ImageSource::read(
                    &output,
                    DEFAULT_DOCUMENT_BUDGET,
                    &cancel,
                )
                .unwrap();
                let page = source
                    .decode(0, true, DEFAULT_DOCUMENT_BUDGET, &cancel)
                    .unwrap();
                if page.image.pixels != [[10, 20, 30, 128], [80, 90, 100, 255]] {
                    fail_smoke();
                }
                let recent =
                    drawer_core::recent::RecentFiles::load(self.storage.as_ref().unwrap()).unwrap();
                if recent.paths() != self.recent.paths() || recent.paths().len() != 2 {
                    fail_smoke();
                }
                if self.display.monitors.is_empty()
                    || !self.display.rendered_native
                    || self.display.rendered_frames < 2
                    || !self.display.native_windowed
                {
                    fail_smoke();
                }
                self.size_presets
                    .add(
                        &drawer_core::canvas_size::Dimensions::new(3, 2, 144.0),
                        "Smoke size".into(),
                        self.storage.as_ref(),
                    )
                    .unwrap();
                if drawer_core::canvas_size::PresetBook::load(self.storage.as_ref().unwrap())
                    .unwrap()
                    != self.size_presets.book
                {
                    fail_smoke();
                }
                let original = self.documents[0].snapshot();
                let sized = drawer_core::sizing::prepare(
                    &original,
                    drawer_core::sizing::Change::CanvasEdges {
                        left: -1,
                        top: 0,
                        right: 2,
                        bottom: 1,
                    },
                    DEFAULT_DOCUMENT_BUDGET,
                    &cancel,
                )
                .unwrap();
                self.documents[0].commit_state(sized).unwrap();
                if self.documents[0].width != 8
                    || self.documents[0].height != 6
                    || !self.documents[0].undo()
                    || self.documents[0].snapshot() != original
                {
                    fail_smoke();
                }
                self.display.preferences.visible = false;
                if drawer_core::interpolation::Settings::load(self.storage.as_ref().unwrap())
                    .unwrap()
                    != self.interpolation
                {
                    fail_smoke();
                }
                self.select_tab(0);
                let original = self.documents[0].snapshot();
                self.begin_paste(ui.ctx(), || {
                    Ok(drawer_core::paste::ClipboardImage {
                        image: drawer_core::references::LocalImage::from_rgba(
                            2,
                            1,
                            &[100, 20, 30, 128, 40, 50, 60, 0],
                            8,
                        )
                        .unwrap(),
                        origin: Some([-100, 1000]),
                    })
                })
                .unwrap();
                self.paste_dialog.as_mut().unwrap().move_to([1, 1]).unwrap();
                self.accept_paste().unwrap();
                if self.documents[0].layers.len() != 2
                    || self.documents[0].composite_pixel(1, 1) != [100, 20, 30, 128]
                {
                    fail_smoke();
                }
                let saved = root.join("smoke-paste.drawer");
                self.save_to(0, &saved).unwrap();
                if drawer_core::files::open(&saved, DEFAULT_DOCUMENT_BUDGET)
                    .unwrap()
                    .snapshot()
                    != self.documents[0].snapshot()
                {
                    fail_smoke();
                }
                self.copy_top_layer_from(|packet, budget| {
                    assert_eq!(packet.origin, Some([1, 1]));
                    assert!(packet.encode(budget).is_ok());
                    Ok(())
                })
                .unwrap();
                if !self.documents[0].undo() || self.documents[0].snapshot() != original {
                    fail_smoke();
                }
                self.display
                    .preferences
                    .save(self.storage.as_ref().unwrap())
                    .unwrap();
                let before_revert = self.documents[0].snapshot();
                if self.revert_from(0, false).unwrap()
                    || self.documents[0].snapshot() != before_revert
                {
                    fail_smoke();
                }
                if !self.revert_from(0, true).unwrap()
                    || self.documents[0].snapshot()
                        != drawer_core::files::open(&saved, DEFAULT_DOCUMENT_BUDGET)
                            .unwrap()
                            .snapshot()
                {
                    fail_smoke();
                }
                let provider = root.join("smoke-movie.avi");
                std::fs::write(&provider, [0u8; 128]).unwrap();
                self.begin_movie_from(ui.ctx(), &provider, false, None)
                    .unwrap();
                if !self.movie_dialog.as_ref().unwrap().decoder_missing() {
                    fail_smoke();
                }
                self.movie_dialog = None;
                let clip = drawer_core::animation::Clip {
                    fps: Default::default(),
                    cells: vec![drawer_core::animation::Cell {
                        name: "Smoke cell".into(),
                        width: 2,
                        height: 1,
                        offset: [0; 2],
                        pixels: Arc::new(vec![[120, 30, 40, 128], [9, 8, 7, 0]]),
                    }],
                    source_times: vec![0.0],
                    audio: None,
                };
                let mut animation = clip.document(DEFAULT_DOCUMENT_BUDGET).unwrap();
                let expected = animation.snapshot();
                let animation_path = root.join("smoke-animation.drawer");
                drawer_core::files::save(&mut animation, &animation_path).unwrap();
                if drawer_core::files::open(&animation_path, DEFAULT_DOCUMENT_BUDGET)
                    .unwrap()
                    .snapshot()
                    != expected
                {
                    fail_smoke();
                }
                self.persist_views(ui.ctx().pixels_per_point() as f64);
                let layout =
                    drawer_core::view_layout::Layout::load(self.storage.as_ref().unwrap()).unwrap();
                self.restore_views(layout.clone(), ui.ctx().pixels_per_point() as f64)
                    .unwrap();
                if self
                    .capture_views(ui.ctx().pixels_per_point() as f64)
                    .rows
                    .len()
                    != layout.rows.len()
                {
                    fail_smoke();
                }
                self.persist_navigator();
                if drawer_core::navigator_preferences::Preferences::load(
                    self.storage.as_ref().unwrap(),
                )
                .unwrap()
                    != self.canvas_view.navigator
                {
                    fail_smoke();
                }
                self.panels.persist(self.storage.as_ref()).unwrap();
                if drawer_core::panels::Settings::load(self.storage.as_ref().unwrap()).unwrap()
                    != self.panels.settings
                {
                    fail_smoke();
                }
                self.persist_interface();
                if drawer_core::ui_scale::Settings::load(self.storage.as_ref().unwrap()).unwrap()
                    != self.interface.settings
                {
                    fail_smoke();
                }
                if self.canvas_view.cursor_tool_count() != 4
                    || !self.canvas_view.brush_shape.valid()
                {
                    fail_smoke();
                }
                let expected_tool = self.workspaces.store.current.actions.cycles[0]
                    .next(self.selected.name(), false)
                    .unwrap();
                self.execute_command(ui.ctx(), "cycle-next:1");
                if self.selected != expected_tool {
                    fail_smoke();
                }
                let mut commands = self.capture_workspace();
                commands.actions.radials[0].resize(8);
                commands
                    .actions
                    .devices
                    .push(drawer_core::actions::DeviceBinding {
                        device: "pointer".into(),
                        action: "extra-1".into(),
                        command: "cycle-prev:1".into(),
                    });
                self.apply_commands(commands.clone()).unwrap();
                if drawer_core::workspaces::Store::load(self.storage.as_ref().unwrap())
                    .unwrap()
                    .unwrap()
                    .current
                    .actions
                    != commands.actions
                {
                    fail_smoke();
                }
                if self.workspaces.native_panels == 0 {
                    fail_smoke();
                }
                let current_workspace = self.capture_workspace();
                self.stage_workspace(drawer_core::workspaces::Snapshot::default(), None);
                self.apply_workspace(
                    drawer_core::workspaces::Scope {
                        bars: true,
                        ..Default::default()
                    },
                    None,
                )
                .unwrap();
                self.persist_workspace().unwrap();
                let loaded = drawer_core::workspaces::Store::load(self.storage.as_ref().unwrap())
                    .unwrap()
                    .unwrap();
                if loaded.current.scratchpad != current_workspace.scratchpad
                    || loaded.current.placements != current_workspace.placements
                {
                    fail_smoke();
                }
                let mut input_workspace = self.capture_workspace();
                input_workspace.input.hold_millis = 350;
                input_workspace.input.modifiers =
                    [Some(BrushTip::Eraser), None, Some(BrushTip::Pencil)];
                input_workspace.input.drag_pixels = 12.0;
                self.stage_workspace(
                    input_workspace.clone(),
                    self.workspaces.store.active.clone(),
                );
                self.apply_workspace(
                    drawer_core::workspaces::Scope {
                        other: true,
                        ..Default::default()
                    },
                    None,
                )
                .unwrap();
                if drawer_core::workspaces::Store::load(self.storage.as_ref().unwrap())
                    .unwrap()
                    .unwrap()
                    .current
                    .input
                    != input_workspace.input
                {
                    fail_smoke();
                }
                let mut held = drawer_core::tool_input::Controller::default();
                held.press("E", BrushTip::Eraser, self.selected, None, 0.0, false);
                if held.release("E", 0.351, input_workspace.input) != Some((self.selected, None)) {
                    fail_smoke();
                }
                held.press_modifier("@Ctrl", BrushTip::Eraser, self.selected, None, 1.0);
                if held.release("@Ctrl", 1.01, input_workspace.input) != Some((self.selected, None))
                {
                    fail_smoke();
                }
                if self.help.rendered_topic.as_deref() != Some("licenses")
                    || self.help.notices_count() == 0
                    || !self.help.korean_font
                {
                    fail_smoke();
                }
                if drawer_core::help::search("작업공간", &self.capture_workspace()).is_empty() {
                    fail_smoke();
                }
                let help_settings = self.capture_workspace().help;
                if drawer_core::workspaces::Store::load(self.storage.as_ref().unwrap())
                    .unwrap()
                    .unwrap()
                    .current
                    .help
                    != help_settings
                {
                    fail_smoke();
                }
                if let Err(e) = self.verify_native_power() {
                    eprintln!("Power request: {e}");
                    fail_smoke();
                }
                if !self.ui_behavior.hidden[0].collapsed
                    || self.workspaces.store.current.placements[0].size[1] < 100.0
                {
                    fail_smoke();
                }
                let mut trace = drawer_core::brush_cursor::Trace::default();
                trace.receive([5.0, 5.0], 0.6);
                trace.receive([80.0, 30.0], 0.6);
                if trace
                    .marks(self.workspaces.store.current.cursors.tools[0])
                    .len()
                    != 3
                    || drawer_core::workspaces::Store::load(self.storage.as_ref().unwrap())
                        .unwrap()
                        .unwrap()
                        .current
                        .cursors
                        != self.workspaces.store.current.cursors
                {
                    fail_smoke();
                }
                if !self.information.shown
                    || ui.ctx().theme() != egui::Theme::Light
                    || self
                        .information
                        .profile_error
                        .as_ref()
                        .is_none_or(|e| !e.contains("No CMYK"))
                {
                    fail_smoke();
                }
                let mut information_settings = self.workspaces.store.current.information.clone();
                information_settings.unit = drawer_core::information::Unit::Millimeters;
                information_settings.side = drawer_core::information::Side::Right;
                self.apply_information(information_settings.clone(), true)
                    .unwrap();
                if drawer_core::workspaces::Store::load(self.storage.as_ref().unwrap())
                    .unwrap()
                    .unwrap()
                    .current
                    .information
                    != information_settings
                {
                    fail_smoke();
                }
                if let Err(e) = drawing::smoke_painting() {
                    eprintln!("Mouse painting: {e}");
                    fail_smoke();
                }
                let mut pointer_workspace = self.capture_workspace();
                pointer_workspace.pointer.pen.stabilization = Some(0.35);
                pointer_workspace.pointer.features.tilt_angle = true;
                pointer_workspace.pointer.features.button_eraser = true;
                pointer_workspace.pressure.tools[0].opacity = true;
                pointer_workspace.pressure.tools[0].response = drawer_core::pressure::Response {
                    minimum: 0.25,
                    maximum: 0.75,
                    curve: [0.0, 0.1, 0.25, 0.8, 1.0],
                };
                self.stage_workspace(pointer_workspace, self.workspaces.store.active.clone());
                if self
                    .apply_workspace(
                        drawer_core::workspaces::Scope {
                            other: true,
                            ..Default::default()
                        },
                        None,
                    )
                    .is_err()
                    || drawer_core::workspaces::Store::load(self.storage.as_ref().unwrap())
                        .unwrap()
                        .unwrap()
                        .current
                        .pointer
                        != self.workspaces.store.current.pointer
                {
                    fail_smoke();
                }
                if pressure_settings::smoke_pressure().is_err()
                    || drawer_core::workspaces::Store::load(self.storage.as_ref().unwrap())
                        .unwrap()
                        .unwrap()
                        .current
                        .pressure
                        != self.workspaces.store.current.pressure
                {
                    fail_smoke();
                }
                if drawing::smoke_attributes().is_err() {
                    fail_smoke();
                }
                if drawing::smoke_defaults().is_err() {
                    fail_smoke();
                }
                if input_replay::smoke_replay().is_err() {
                    fail_smoke();
                }
                if pointer_input::smoke_controls().is_err() {
                    fail_smoke();
                }
                if pointer_input::smoke_mapping().is_err() {
                    fail_smoke();
                }
                let acquisition_work = self
                    .storage
                    .as_ref()
                    .unwrap()
                    .root()
                    .join("acquisition-work");
                if let Err(error) = acquisition::smoke_pixels(&acquisition_work) {
                    eprintln!("Acquisition pixels: {error}");
                    fail_smoke();
                }
                #[cfg(windows)]
                if let Err(error) = acquisition::smoke_broker(&acquisition_work) {
                    eprintln!("Acquisition helper: {error}");
                    fail_smoke();
                }
                #[cfg(windows)]
                if self
                    .native_mouse
                    .as_ref()
                    .is_none_or(|input| !input.attached())
                    || drawing::smoke_dedup().is_err()
                {
                    fail_smoke();
                }
                #[cfg(windows)]
                let backend_check =
                    match pointer_input::smoke_backends(self.native_mouse.as_ref().unwrap()) {
                        Ok(report) => report,
                        Err(_) => {
                            fail_smoke();
                        }
                    };
                #[cfg(not(windows))]
                let backend_check = "not available on this target";
                let native_pen = &self.native_input.report;
                if native_pen.is_empty() {
                    fail_smoke();
                }
                let report = format!(
                    "app={APP_NAME}\nframes={}\ntools=Pen,Pencil,Eraser,Wash\nembedded_tips={}\ncanvases={}\nviews={}\nclose_cancel={}\nsizing_dpi=true\nsizing_undo=true\nreference_copy=true\nreference_document=true\ngrid_guide_undo=true\nprint_size_view=true\nclipboard_document=true\nsave_as_export=true\nrecent_restart=true\nimage_decode=true\nnative_display_view=true\nmonitor_inventory=true\ncanvas_presets=true\nsigned_edges_undo=true\npaste_layer_undo=true\npaste_native_reopen=true\ninterpolation_view=true\ninterpolation_settings=true\nview_layout=true\nnavigator_settings=true\nrevert_saved=true\nmovie_decoder_missing=true\nanimation_native=true\npanel_settings=true\ninterface_settings=true\ncursor_assets=true\nworkspace_settings=true\nnative_floating_panels=true\npopup_test_painting=true\ncommand_settings=true\ntool_cycle=true\nradial_settings=true\ndevice_action_profile=true\ntemporary_tool=true\ndrag_threshold=true\nlocal_help=true\nlocal_help_licenses=true\nlocal_korean_font=true\npanel_auto_hide=true\nlocal_power_release=true\ncursor_modes=true\ninformation_panel=true\nlocal_color_profile=true\ntheme_settings=true\nmodifier_tools=true\nscreen_modes=true\ncontext_input=true\nlayout_windows=true\ntool_property_settings=true\nime_backend={native_ime}\nmouse_painting=true\npointer_settings=true\npressure_mapping=true\npressure_curve=true\nsynthetic_pen_attributes=true\nmouse_origin_backend=true\nsynthetic_pen_dedup=true\nmissing_pen_properties=true\nvirtual_input_replay=true\nnative_pen={native_pen}\n",
                    self.frames,
                    self.previews.len(),
                    self.documents.len(),
                    self.tabs.len(),
                    validated,
                );
                let report = format!(
                    "{report}backend_switching=true\npen_click_controls=true\ntablet_mapping=true\nlocal_acquisition=true\nwintab_initialization={backend_check}\n"
                );
                if let Err(error) = std::fs::write(path, report) {
                    eprintln!("Smoke receipt: {error}");
                    fail_smoke();
                }
                self.quit_ready = true;
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
            } else {
                ui.ctx().request_repaint();
            }
        }
    }
}

#[track_caller]
fn fail_smoke() -> ! {
    let location = std::panic::Location::caller();
    let message = format!(
        "Smoke assertion failed at {}:{}",
        location.file(),
        location.line()
    );
    eprintln!("{message}");
    if let Some(root) = std::env::var_os("DRAWER_SMOKE_DIAGNOSTICS") {
        let _ = std::fs::write(PathBuf::from(root).join("smoke-failure.txt"), message);
    }
    std::process::exit(1)
}

fn main() -> eframe::Result {
    let mut args = std::env::args_os().skip(1);
    let first = args.next();
    if first.as_deref() == Some(std::ffi::OsStr::new("--acquire-helper")) {
        let result = args
            .next()
            .ok_or_else(|| "Missing acquisition directory".to_string())
            .and_then(|p| acquisition::helper(&PathBuf::from(p)));
        if let Err(error) = result {
            eprintln!("{error}");
            std::process::exit(1);
        }
        return Ok(());
    }
    let (smoke_output, startup_paths) =
        if first.as_deref() == Some(std::ffi::OsStr::new("--smoke-test")) {
            (
                Some(PathBuf::from(args.next().expect("smoke receipt path"))),
                Vec::new(),
            )
        } else {
            (
                None,
                first
                    .into_iter()
                    .chain(args)
                    .map(PathBuf::from)
                    .collect::<Vec<_>>(),
            )
        };
    #[cfg(windows)]
    let native_mouse = drawer_platform::Input::default();
    let options = eframe::NativeOptions {
        #[cfg(windows)]
        event_loop_builder: Some(Box::new({
            let native = native_mouse.clone();
            move |builder| native.install(builder)
        })),
        viewport: egui::ViewportBuilder::default()
            .with_title(APP_NAME)
            .with_inner_size([1100.0, 760.0])
            .with_min_inner_size([640.0, 480.0])
            .with_icon(Arc::new(egui::IconData {
                rgba: application_icon(64),
                width: 64,
                height: 64,
            })),
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };
    eframe::run_native(
        APP_NAME,
        options,
        Box::new(move |cc| {
            let mut app = Drawer::new(&cc.egui_ctx, smoke_output)
                .with_storage(UserStorage::from_environment()?);
            #[cfg(windows)]
            if let Some(window) = cc.winit_window() {
                match native_mouse.attach(window) {
                    Ok(()) => app.native_mouse = Some(native_mouse),
                    Err(e) => app.status = Some(e),
                }
            }
            for path in startup_paths {
                if let Err(e) = app.open_path(&path) {
                    app.status = Some(format!("Cannot open {}: {e}", path.display()));
                }
            }
            Ok(Box::new(app))
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    pub(crate) struct TestDirectory(pub(crate) PathBuf);
    impl TestDirectory {
        pub(crate) fn new() -> Self {
            use std::sync::atomic::{AtomicU64, Ordering};
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "drawer-workspace-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for TestDirectory {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).unwrap();
        }
    }
    pub(crate) fn tiny_spec() -> CanvasSpec {
        CanvasSpec {
            width: 1,
            height: 1,
            dpi: 300.0,
            background: Background::Transparent,
        }
    }
    #[test]
    fn workspace_selective_apply_cancel_restart_backup_and_document_invariance() {
        use drawer_core::workspaces::{Dock, Scope, Snapshot};
        let ctx = egui::Context::default();
        let root = TestDirectory::new();
        let storage = UserStorage::at(root.0.join("user")).unwrap();
        let mut app = Drawer::new(&ctx, None)
            .with_storage(UserStorage::at(storage.root().to_path_buf()).unwrap());
        app.create_canvas(tiny_spec()).unwrap();
        app.documents[0].edit_pixel(0, 0, 0, [1, 2, 3, 128]);
        let document = app.documents[0].snapshot();
        let history = app.documents[0].history_counts();
        let dirty = app.documents[0].dirty;
        frame(&ctx, &mut app, vec![]);
        let before = app.capture_workspace();
        let mut incoming = before.clone();
        incoming.placements[0].dock = Dock::Right;
        incoming.panels.panels[1].visible = false;
        incoming.bindings[5].key = "F2".into();
        incoming.interface.zoom = 1.5;
        app.stage_workspace(incoming.clone(), None);
        frame(&ctx, &mut app, vec![]);
        frame(&ctx, &mut app, vec![]);
        let cancel = app.workspaces.cancel_button.unwrap();
        click(&ctx, &mut app, cancel);
        assert!(app.workspaces.staged.is_none());
        assert_eq!(app.capture_workspace(), before);
        app.stage_workspace(incoming.clone(), None);
        app.apply_workspace(
            Scope {
                layout: true,
                ..Default::default()
            },
            None,
        )
        .unwrap();
        assert_eq!(app.capture_workspace().bindings, before.bindings);
        assert_eq!(app.interface.settings, before.interface);
        assert_eq!(app.workspaces.store.current.placements[0].dock, Dock::Right);
        assert_eq!(
            Snapshot::read(&storage.root().join("settings/workspace-before-apply.json")).unwrap(),
            before
        );
        let layout = app.capture_workspace();
        app.stage_workspace(incoming.clone(), None);
        app.apply_workspace(
            Scope {
                keys: true,
                ..Default::default()
            },
            Some("Before keys"),
        )
        .unwrap();
        assert_eq!(app.capture_workspace().placements, layout.placements);
        assert_eq!(app.capture_workspace().bindings, incoming.bindings);
        assert_eq!(app.workspaces.store.named[0].snapshot, layout);
        frame(
            &ctx,
            &mut app,
            vec![egui::Event::Key {
                key: egui::Key::F2,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Default::default(),
            }],
        );
        assert_eq!(app.selected, BrushTip::Pen);
        app.workspaces
            .store
            .save_named("Saved", before.clone())
            .unwrap();
        app.workspaces.store.active = Some("Saved".into());
        app.persist_workspace().unwrap();
        app.stage_workspace(before.clone(), Some("Saved".into()));
        frame(&ctx, &mut app, vec![]);
        frame(&ctx, &mut app, vec![]);
        let cancel = app.workspaces.cancel_button.unwrap();
        click(&ctx, &mut app, cancel);
        assert_eq!(app.capture_workspace().placements, layout.placements);
        let current = app.capture_workspace();
        incoming.bindings.push(incoming.bindings[0].clone());
        app.stage_workspace(incoming, None);
        assert!(app.apply_workspace(Scope::ALL, None).is_err());
        assert_eq!(app.capture_workspace(), current);
        app.workspaces.staged = None;
        std::fs::write(root.0.join("damaged.json"), "damaged").unwrap();
        assert!(Snapshot::read(&root.0.join("damaged.json")).is_err());
        assert_eq!(app.documents[0].snapshot(), document);
        assert_eq!(app.documents[0].history_counts(), history);
        assert_eq!(app.documents[0].dirty, dirty);
        let restarted = Drawer::new(&ctx, None)
            .with_storage(UserStorage::at(storage.root().to_path_buf()).unwrap());
        assert_eq!(restarted.capture_workspace(), current);
    }
    #[test]
    fn popup_test_painting_device_releases_and_restart_preserve_document() {
        let ctx = egui::Context::default();
        let root = TestDirectory::new();
        let storage = UserStorage::at(root.0.join("user")).unwrap();
        let mut app = Drawer::new(&ctx, None)
            .with_storage(UserStorage::at(storage.root().to_path_buf()).unwrap());
        app.create_canvas(tiny_spec()).unwrap();
        let before = app.documents[0].snapshot();
        let history = app.documents[0].history_counts();
        app.workspaces.store.current.scratchpad.visible = true;
        app.set_tool(BrushTip::Wash);
        app.references.session.workspace.color = [10, 80, 90, 255];
        frame(&ctx, &mut app, vec![]);
        frame(&ctx, &mut app, vec![]);
        let rect = app.workspaces.pad_rect.unwrap();
        let pos = rect.center();
        let key = |pressed| egui::Event::Key {
            key: egui::Key::Space,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers: Default::default(),
        };
        let button = |button, pressed| egui::Event::PointerButton {
            pos,
            button,
            pressed,
            modifiers: Default::default(),
        };
        frame(
            &ctx,
            &mut app,
            vec![
                egui::Event::PointerMoved(pos),
                key(true),
                button(egui::PointerButton::Primary, true),
                button(egui::PointerButton::Secondary, true),
            ],
        );
        assert!(!app.workspaces.input_idle());
        frame(
            &ctx,
            &mut app,
            vec![egui::Event::PointerMoved(pos + egui::vec2(45.0, 20.0))],
        );
        frame(
            &ctx,
            &mut app,
            vec![
                key(false),
                button(egui::PointerButton::Primary, false),
                button(egui::PointerButton::Secondary, false),
            ],
        );
        assert!(app.workspaces.input_idle());
        assert!(
            app.workspaces
                .store
                .current
                .scratchpad
                .pixels
                .iter()
                .any(|p| p[3] > 0)
        );
        let pixels = app.workspaces.store.current.scratchpad.pixels.clone();
        frame(
            &ctx,
            &mut app,
            vec![
                key(true),
                button(egui::PointerButton::Primary, true),
                egui::Event::WindowFocused(false),
            ],
        );
        assert!(app.workspaces.input_idle());
        assert_eq!(app.documents[0].snapshot(), before);
        assert_eq!(app.documents[0].history_counts(), history);
        let restarted = Drawer::new(&egui::Context::default(), None)
            .with_storage(UserStorage::at(storage.root().to_path_buf()).unwrap());
        assert_eq!(restarted.selected, BrushTip::Wash);
        assert!(restarted.workspaces.store.current.scratchpad.visible);
        assert_eq!(restarted.workspaces.store.current.scratchpad.pixels, pixels);
    }
    #[test]
    fn floating_workspace_panels_recover_missing_screen_without_drawing_changes() {
        use drawer_core::{display_view::Monitor, workspaces::Dock};
        let ctx = egui::Context::default();
        let mut app = Drawer::new(&ctx, None);
        app.create_canvas(tiny_spec()).unwrap();
        let before = app.documents[0].snapshot();
        for p in &mut app.workspaces.store.current.placements {
            p.dock = Dock::Floating;
            p.monitor = Some("removed".into());
            p.position = [5000.0, -5000.0];
        }
        app.display.monitors = vec![Monitor {
            key: "main".into(),
            index: 0,
            position: [0, 0],
            size: [1920, 1080],
            scale: 2.0,
            primary: true,
        }];
        frame(&ctx, &mut app, vec![]);
        frame(&ctx, &mut app, vec![]);
        for p in &app.workspaces.store.current.placements {
            assert_eq!(p.monitor.as_deref(), Some("main"));
            assert!(p.position[0] >= 0.0 && p.position[1] >= 0.0);
        }
        assert!(app.workspaces.reports.len() >= 4);
        assert_eq!(app.documents[0].snapshot(), before);
    }
    #[test]
    fn new_context_displays_all_built_in_tools() {
        let ctx = egui::Context::default();
        let mut app = Drawer::new(&ctx, None);
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1100.0, 760.0),
                )),
                ..Default::default()
            },
            |ui| app.show(ui),
        );
        assert_eq!(app.previews.len(), BrushTip::ALL.len());
        assert!(!output.shapes.is_empty());
        output.textures_delta.clear();
        for tip in BrushTip::ALL {
            app.selected = tip;
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1100.0, 760.0),
                    )),
                    ..Default::default()
                },
                |ui| app.show(ui),
            );
            assert!(!output.shapes.is_empty());
            output.textures_delta.clear();
        }
    }

    #[test]
    fn new_canvas_preserves_existing_document_and_failure_adds_nothing() {
        let ctx = egui::Context::default();
        let mut app = Drawer::new(&ctx, None);
        let spec = CanvasSpec {
            width: 1,
            height: 1,
            dpi: 300.0,
            background: Background::Color([10, 20, 30]),
        };
        app.create_canvas(spec).unwrap();
        assert!(app.create_canvas(CanvasSpec { width: 0, ..spec }).is_err());
        assert_eq!(app.documents.len(), 1);
        assert_eq!(app.active, Some(0));
        app.create_canvas(CanvasSpec {
            background: Background::Transparent,
            ..spec
        })
        .unwrap();
        assert_eq!(app.active, Some(1));
        assert_eq!(app.documents[0].layers[0].pixels, [[10, 20, 30, 255]]);
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| app.show(ui));
        output.textures_delta.clear();
    }

    pub(crate) fn frame(ctx: &egui::Context, app: &mut Drawer, events: Vec<egui::Event>) {
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1100.0, 760.0),
                )),
                events,
                ..Default::default()
            },
            |ui| app.show(ui),
        );
        output.textures_delta.clear();
    }
    pub(crate) fn click(ctx: &egui::Context, app: &mut Drawer, rect: egui::Rect) {
        let pos = rect.center();
        frame(ctx, app, vec![egui::Event::PointerMoved(pos)]);
        for pressed in [true, false] {
            frame(
                ctx,
                app,
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::default(),
                    },
                ],
            );
        }
    }
    #[test]
    fn mouse_creation_rejects_zero_size_and_cancel_preserves_documents() {
        let ctx = egui::Context::default();
        let mut app = Drawer::new(&ctx, None);
        frame(&ctx, &mut app, vec![]);
        let button = app.new_button.unwrap();
        click(&ctx, &mut app, button);
        frame(&ctx, &mut app, vec![]);
        assert!(app.show_new_canvas);
        app.new_canvas.width = "1".into();
        app.new_canvas.height = "1".into();
        app.new_canvas.transparent = true;
        let button = app.create_button.unwrap();
        click(&ctx, &mut app, button);
        assert_eq!(app.documents.len(), 1);
        assert_eq!(app.documents[0].layers[0].pixels, [[0; 4]]);
        let button = app.new_button.unwrap();
        click(&ctx, &mut app, button);
        frame(&ctx, &mut app, vec![]);
        assert!(app.show_new_canvas);
        app.new_canvas.width = "0".into();
        frame(&ctx, &mut app, vec![]);
        let button = app.create_button.unwrap();
        click(&ctx, &mut app, button);
        frame(&ctx, &mut app, vec![]);
        assert_eq!(app.documents.len(), 1);
        assert!(app.new_canvas.error.is_some());
        let button = app.cancel_button.unwrap();
        click(&ctx, &mut app, button);
        assert!(!app.show_new_canvas);
        assert_eq!(app.documents.len(), 1);
    }

    #[test]
    fn wheel_and_middle_drag_change_only_view_state() {
        use std::hash::{Hash, Hasher};
        fn pixels_hash(document: &Document) -> u64 {
            let mut hash = std::hash::DefaultHasher::new();
            document.layers[0].pixels.hash(&mut hash);
            hash.finish()
        }
        let ctx = egui::Context::default();
        let mut app = Drawer::new(&ctx, None);
        app.create_canvas(CanvasSpec {
            width: 32,
            height: 16,
            dpi: 300.0,
            background: Background::Color([10, 20, 30]),
        })
        .unwrap();
        app.documents[0].layers[0].pixels[0] = [200, 10, 40, 255];
        app.documents[0].dirty = false;
        let hash = pixels_hash(&app.documents[0]);
        frame(&ctx, &mut app, vec![]);
        let point = app.canvas_view.viewport.unwrap().center();
        frame(&ctx, &mut app, vec![egui::Event::PointerMoved(point)]);
        frame(
            &ctx,
            &mut app,
            vec![egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                phase: egui::TouchPhase::Move,
                delta: egui::vec2(0.0, 120.0),
                modifiers: Default::default(),
            }],
        );
        assert!(app.documents[0].view.zoom > 1.0);
        let before = app.documents[0].view.pan;
        frame(
            &ctx,
            &mut app,
            vec![egui::Event::PointerButton {
                pos: point,
                button: egui::PointerButton::Middle,
                pressed: true,
                modifiers: Default::default(),
            }],
        );
        let moved = point + egui::vec2(40.0, 25.0);
        frame(&ctx, &mut app, vec![egui::Event::PointerMoved(moved)]);
        frame(
            &ctx,
            &mut app,
            vec![egui::Event::PointerButton {
                pos: moved,
                button: egui::PointerButton::Middle,
                pressed: false,
                modifiers: Default::default(),
            }],
        );
        assert_ne!(app.documents[0].view.pan, before);
        app.documents[0].view.rotate(90.0);
        app.documents[0].view.flipped = true;
        frame(&ctx, &mut app, vec![]);
        assert_eq!(pixels_hash(&app.documents[0]), hash);
        assert!(!app.documents[0].dirty);
    }

    #[test]
    fn same_named_files_keep_separate_tools_views_and_histories() {
        let root = TestDirectory::new();
        let a = root.0.join("a");
        let b = root.0.join("b");
        std::fs::create_dir(&a).unwrap();
        std::fs::create_dir(&b).unwrap();
        for (directory, color) in [(&a, [200, 0, 0, 255]), (&b, [0, 0, 200, 255])] {
            let mut doc = Document::create(tiny_spec(), 4).unwrap();
            doc.edit_pixel(0, 0, 0, color);
            drawer_core::files::save(&mut doc, &directory.join("same.drawer")).unwrap();
        }
        let ctx = egui::Context::default();
        let mut app = Drawer::new(&ctx, None);
        app.open_path(&a.join("same.drawer")).unwrap();
        let first = app.documents[0].id;
        app.set_tool(BrushTip::Wash);
        app.documents[0].view.zoom = 2.0;
        app.documents[0].edit_pixel(0, 0, 0, [20, 30, 40, 255]);
        app.open_path(&b.join("same.drawer")).unwrap();
        let second = app.documents[1].id;
        assert_ne!(first, second);
        assert_eq!(app.documents[0].title, app.documents[1].title);
        assert_eq!(app.selected, BrushTip::Wash);
        app.set_tool(BrushTip::Eraser);
        app.select_tab(0);
        assert_eq!(app.selected, BrushTip::Wash);
        assert_eq!(app.documents[0].view.zoom, 2.0);
        assert_eq!(app.documents[0].history_counts(), (1, 0));
        assert_eq!(app.documents[1].history_counts(), (0, 0));
        app.select_tab(1);
        assert_eq!(app.selected, BrushTip::Eraser);
        app.open_path(&a.join("same.drawer")).unwrap();
        assert_eq!(app.documents.len(), 2);
        assert_eq!(app.active, Some(0));
        let state = (app.documents.len(), app.active);
        assert!(app.open_path(&root.0.join("missing.png")).is_err());
        assert_eq!((app.documents.len(), app.active), state);
    }

    #[test]
    fn mouse_close_prompts_only_for_last_view_and_cancel_keeps_state() {
        let ctx = egui::Context::default();
        let mut app = Drawer::new(&ctx, None);
        app.create_canvas(tiny_spec()).unwrap();
        app.documents[0].edit_pixel(0, 0, 0, [90, 80, 70, 255]);
        app.set_tool(BrushTip::Pencil);
        app.documents[0].view.zoom = 2.0;
        app.duplicate_view();
        app.documents[0].view.zoom = 4.0;
        app.select_tab(0);
        assert_eq!(app.documents[0].view.zoom, 2.0);
        frame(&ctx, &mut app, vec![]);
        let id = app.tabs[1].id;
        let button = app.close_buttons[&id];
        click(&ctx, &mut app, button);
        assert_eq!(app.tabs.len(), 1);
        assert_eq!(app.documents.len(), 1);
        assert!(app.pending_close.is_none());
        frame(&ctx, &mut app, vec![]);
        let button = app.close_buttons[&app.tabs[0].id];
        click(&ctx, &mut app, button);
        frame(&ctx, &mut app, vec![]);
        assert!(app.pending_close.is_some());
        let button = app.cancel_close_button.unwrap();
        click(&ctx, &mut app, button);
        assert!(app.pending_close.is_none());
        assert_eq!(app.documents[0].layers[0].pixels[0], [90, 80, 70, 255]);
        assert_eq!(app.documents[0].history_counts(), (1, 0));
        assert_eq!(app.selected, BrushTip::Pencil);
    }

    #[test]
    fn quit_cancellation_does_not_drop_previously_approved_documents() {
        let ctx = egui::Context::default();
        let mut app = Drawer::new(&ctx, None);
        app.create_canvas(tiny_spec()).unwrap();
        app.create_canvas(tiny_spec()).unwrap();
        let ids: Vec<_> = app.documents.iter().map(|doc| doc.id).collect();
        app.begin_quit();
        app.accept_close();
        assert_eq!(app.quit_approved.len(), 1);
        assert!(!app.quit_ready);
        app.cancel_close();
        assert_eq!(
            app.documents.iter().map(|doc| doc.id).collect::<Vec<_>>(),
            ids
        );
        assert_eq!(app.tabs.len(), 2);
        assert!(!app.quit_ready);
        app.begin_quit();
        app.accept_close();
        app.accept_close();
        assert!(app.quit_ready);
        assert_eq!(app.documents.len(), 2);
    }

    #[test]
    fn failed_save_keeps_close_prompt_and_successful_save_closes_document() {
        let root = TestDirectory::new();
        let path = root.0.join("picture.drawer");
        let mut doc = Document::create(tiny_spec(), 4).unwrap();
        drawer_core::files::save(&mut doc, &path).unwrap();
        let original = std::fs::read(&path).unwrap();
        let ctx = egui::Context::default();
        let mut app = Drawer::new(&ctx, None);
        app.open_path(&path).unwrap();
        app.documents[0].edit_pixel(0, 0, 0, [50, 100, 150, 255]);
        let temporary = root.0.join("picture.drawer.drawer-write.tmp");
        std::fs::write(&temporary, "blocked operation").unwrap();
        app.request_close(0);
        frame(&ctx, &mut app, vec![]);
        frame(&ctx, &mut app, vec![]);
        let button = app.save_close_button.unwrap();
        click(&ctx, &mut app, button);
        assert!(app.pending_close.is_some());
        assert!(app.status.is_some());
        assert_eq!(std::fs::read(&path).unwrap(), original);
        assert!(app.documents[0].dirty);
        std::fs::remove_file(temporary).unwrap();
        frame(&ctx, &mut app, vec![]);
        frame(&ctx, &mut app, vec![]);
        let button = app.save_close_button.unwrap();
        click(&ctx, &mut app, button);
        assert!(app.documents.is_empty());
        assert!(app.tabs.is_empty());
        let restored = drawer_core::files::open(&path, 4).unwrap();
        assert_eq!(restored.layers[0].pixels[0], [50, 100, 150, 255]);
    }
    fn finish_sizing(ctx: &egui::Context, app: &mut Drawer) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while app.sizing_dialog.is_some() && std::time::Instant::now() < deadline {
            frame(ctx, app, vec![]);
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        assert!(app.sizing_dialog.is_none(), "Sizing job did not finish");
    }
    #[test]
    fn mouse_sizing_apply_is_one_undo_and_preview_cancel_keeps_drawing() {
        use sizing_dialog::Mode;
        let ctx = egui::Context::default();
        let mut app = Drawer::new(&ctx, None);
        app.create_canvas(CanvasSpec {
            width: 4,
            height: 2,
            dpi: 300.0,
            background: Background::Transparent,
        })
        .unwrap();
        app.documents[0].layers[0].pixels[0] = [200, 10, 40, 255];
        let original = app.documents[0].snapshot();
        app.begin_sizing(Mode::Canvas);
        app.sizing_dialog.as_mut().unwrap().width = "1".into();
        frame(&ctx, &mut app, vec![]);
        frame(&ctx, &mut app, vec![]);
        let button = app.sizing_dialog.as_ref().unwrap().apply_button.unwrap();
        click(&ctx, &mut app, button);
        assert!(app.sizing_dialog.as_ref().unwrap().job_is_idle());
        assert_eq!(app.documents[0].snapshot(), original);
        let button = app.sizing_dialog.as_ref().unwrap().cancel_button.unwrap();
        click(&ctx, &mut app, button);
        assert!(app.sizing_dialog.is_none());
        assert_eq!(app.documents[0].history_counts(), (0, 0));
        app.begin_sizing(Mode::Resample);
        app.sizing_dialog.as_mut().unwrap().width = "8".into();
        app.sizing_dialog.as_mut().unwrap().height = "4".into();
        frame(&ctx, &mut app, vec![]);
        frame(&ctx, &mut app, vec![]);
        let button = app.sizing_dialog.as_ref().unwrap().apply_button.unwrap();
        click(&ctx, &mut app, button);
        finish_sizing(&ctx, &mut app);
        assert_eq!((app.documents[0].width, app.documents[0].height), (8, 4));
        assert_eq!(app.documents[0].history_counts(), (1, 0));
        assert!(app.documents[0].undo());
        assert_eq!(app.documents[0].snapshot(), original);
        app.begin_sizing(Mode::Dpi);
        app.sizing_dialog.as_mut().unwrap().dpi = "600".into();
        frame(&ctx, &mut app, vec![]);
        frame(&ctx, &mut app, vec![]);
        let button = app.sizing_dialog.as_ref().unwrap().apply_button.unwrap();
        click(&ctx, &mut app, button);
        finish_sizing(&ctx, &mut app);
        assert_eq!(app.documents[0].dpi, 600.0);
        assert_eq!(app.documents[0].layers, original.layers);
        assert!(app.documents[0].undo());
        assert_eq!(app.documents[0].snapshot(), original);
        app.begin_sizing(Mode::Crop);
        let dialog = app.sizing_dialog.as_mut().unwrap();
        dialog.x = "1".into();
        dialog.width = "2".into();
        dialog.confirmed = true;
        frame(&ctx, &mut app, vec![]);
        frame(&ctx, &mut app, vec![]);
        let button = app.sizing_dialog.as_ref().unwrap().apply_button.unwrap();
        click(&ctx, &mut app, button);
        finish_sizing(&ctx, &mut app);
        assert_eq!(app.documents[0].width, 2);
        assert_eq!(app.documents[0].scene.clips.len(), 1);
        assert!(app.documents[0].undo());
        assert_eq!(app.documents[0].snapshot(), original);
    }
    #[test]
    fn canceling_background_sizing_and_invalid_input_preserve_saved_state() {
        use sizing_dialog::Mode;
        let ctx = egui::Context::default();
        let mut app = Drawer::new(&ctx, None);
        app.create_canvas(tiny_spec()).unwrap();
        app.documents[0].mark_saved("test.drawer".into());
        let original = app.documents[0].snapshot();
        app.begin_sizing(Mode::Resample);
        app.sizing_dialog.as_mut().unwrap().width = "0".into();
        frame(&ctx, &mut app, vec![]);
        assert_eq!(app.documents[0].snapshot(), original);
        assert!(!app.documents[0].dirty);
        let button = app.sizing_dialog.as_ref().unwrap().cancel_button.unwrap();
        click(&ctx, &mut app, button);
        app.begin_sizing(Mode::Resample);
        app.sizing_dialog.as_mut().unwrap().width = "1024".into();
        app.sizing_dialog.as_mut().unwrap().height = "1024".into();
        frame(&ctx, &mut app, vec![]);
        frame(&ctx, &mut app, vec![]);
        let button = app.sizing_dialog.as_ref().unwrap().apply_button.unwrap();
        click(&ctx, &mut app, button);
        app.sizing_dialog = None;
        frame(&ctx, &mut app, vec![]);
        assert_eq!(app.documents[0].snapshot(), original);
        assert_eq!(app.documents[0].history_counts(), (0, 0));
        assert!(!app.documents[0].dirty);
        app.begin_sizing(Mode::Dpi);
        app.sizing_dialog.as_mut().unwrap().dpi = "600".into();
        frame(&ctx, &mut app, vec![]);
        frame(&ctx, &mut app, vec![]);
        let button = app.sizing_dialog.as_ref().unwrap().apply_button.unwrap();
        click(&ctx, &mut app, button);
        finish_sizing(&ctx, &mut app);
        assert_eq!(
            (
                app.documents[0].width,
                app.documents[0].height,
                app.documents[0].dpi
            ),
            (1, 1, 600.0)
        );
        assert_eq!(app.documents[0].history_counts(), (1, 0));
        assert!(app.documents[0].undo());
        assert_eq!(app.documents[0].snapshot(), original);
        assert!(!app.documents[0].dirty);
    }
    #[test]
    fn mouse_image_transform_uses_document_history_and_preserves_view() {
        let ctx = egui::Context::default();
        let mut app = Drawer::new(&ctx, None);
        app.create_canvas(CanvasSpec {
            width: 2,
            height: 1,
            dpi: 300.0,
            background: Background::Transparent,
        })
        .unwrap();
        app.documents[0].layers[0].pixels = vec![[255, 0, 0, 255], [0, 255, 0, 255]];
        app.documents[0].view.flipped = true;
        let view = app.documents[0].view;
        let original = app.documents[0].snapshot();
        frame(&ctx, &mut app, vec![]);
        let button = app.transform_buttons["Flip left/right"];
        click(&ctx, &mut app, button);
        finish_sizing(&ctx, &mut app);
        assert_eq!(
            app.documents[0].layers[0].pixels,
            [[0, 255, 0, 255], [255, 0, 0, 255]]
        );
        assert_eq!(app.documents[0].history_counts(), (1, 0));
        assert_eq!(app.documents[0].view, view);
        assert!(app.documents[0].undo());
        assert_eq!(app.documents[0].snapshot(), original);
    }
    #[test]
    fn navigator_mouse_click_and_drag_move_only_view_in_rotated_mirrored_canvas() {
        let ctx = egui::Context::default();
        let mut app = Drawer::new(&ctx, None);
        app.create_canvas(CanvasSpec {
            width: 32,
            height: 16,
            dpi: 300.0,
            background: Background::Color([20, 40, 60]),
        })
        .unwrap();
        app.documents[0].mark_saved("saved.drawer".into());
        let original = app.documents[0].snapshot();
        app.documents[0].view.angle = 37.0;
        app.documents[0].view.flipped = true;
        frame(&ctx, &mut app, vec![]);
        let rect = app.canvas_view.navigator_rect.unwrap();
        let pos = rect.center() + egui::vec2(40.0, -20.0);
        let target = egui::Rect::from_center_size(pos, egui::vec2(1.0, 1.0));
        click(&ctx, &mut app, target);
        let viewport = app.canvas_view.viewport.unwrap();
        let center = [viewport.center().x as f64, viewport.center().y as f64];
        let docpoint = app.documents[0].view.screen_to_document(
            center,
            [32.0, 16.0],
            center,
            ctx.pixels_per_point() as f64,
        );
        assert!((docpoint[0] - 24.0).abs() < 1e-4 && (docpoint[1] - 4.0).abs() < 1e-4);
        frame(
            &ctx,
            &mut app,
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: Default::default(),
                },
            ],
        );
        let moved = pos + egui::vec2(-30.0, 20.0);
        frame(&ctx, &mut app, vec![egui::Event::PointerMoved(moved)]);
        frame(
            &ctx,
            &mut app,
            vec![egui::Event::PointerButton {
                pos: moved,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: Default::default(),
            }],
        );
        let docpoint = app.documents[0].view.screen_to_document(
            center,
            [32.0, 16.0],
            center,
            ctx.pixels_per_point() as f64,
        );
        assert!((docpoint[0] - 18.0).abs() < 1e-4 && (docpoint[1] - 8.0).abs() < 1e-4);
        assert_eq!(app.documents[0].snapshot(), original);
        assert!(!app.documents[0].dirty);
        assert_eq!(app.documents[0].history_counts(), (0, 0));
    }
    #[test]
    fn reference_mouse_and_virtual_pen_pick_raw_alpha_without_changing_the_main_drawing() {
        use drawer_core::references::LocalImage;
        let root = TestDirectory::new();
        let storage = UserStorage::at(root.0.join("userdata")).unwrap();
        let ctx = egui::Context::default();
        let mut app = Drawer::new(&ctx, None).with_storage(storage);
        app.create_canvas(tiny_spec()).unwrap();
        app.documents[0].mark_saved("main.drawer".into());
        let original = app.documents[0].snapshot();
        app.paste_reference_from(|| {
            LocalImage::from_rgba(2, 1, &[240, 30, 80, 128, 20, 40, 60, 0], 8)
                .map_err(|e| e.to_string())
        })
        .unwrap();
        let workspace = app.references.session.workspace.clone();
        assert!(
            app.paste_reference_from(|| Err("No clipboard image".into()))
                .is_err()
        );
        assert_eq!(app.references.session.workspace, workspace);
        app.references.session.workspace.visible = true;
        app.references.session.workspace.auto_pick = true;
        app.references
            .session
            .workspace
            .active_record_mut()
            .unwrap()
            .view = drawer_core::view::View {
            zoom: 32.0,
            angle: 37.0,
            pan: [0.0; 2],
            flipped: false,
        };
        frame(&ctx, &mut app, vec![]);
        frame(&ctx, &mut app, vec![]);
        let rect = app.references.viewport.unwrap();
        let center = [rect.center().x as f64, rect.center().y as f64];
        let view = app
            .references
            .session
            .workspace
            .active_record()
            .unwrap()
            .view;
        let point = view.document_to_screen(
            [0.5, 0.5],
            [2.0, 1.0],
            center,
            ctx.pixels_per_point() as f64,
        );
        let point = egui::pos2(point[0] as f32, point[1] as f32);
        frame(&ctx, &mut app, vec![egui::Event::PointerMoved(point)]);
        assert_eq!(app.references.session.workspace.color, [240, 30, 80, 128]);
        assert_eq!(app.references.ring.unwrap().1, [240, 30, 80, 128]);
        let rect = app.references.viewport.unwrap();
        let center = [rect.center().x as f64, rect.center().y as f64];
        let point = view.document_to_screen(
            [1.5, 0.5],
            [2.0, 1.0],
            center,
            ctx.pixels_per_point() as f64,
        );
        let point = egui::pos2(point[0] as f32, point[1] as f32);
        frame(
            &ctx,
            &mut app,
            vec![
                egui::Event::PointerGone,
                egui::Event::Touch {
                    device_id: egui::TouchDeviceId(42),
                    id: egui::TouchId(1),
                    phase: egui::TouchPhase::Move,
                    pos: point,
                    force: Some(0.6),
                },
            ],
        );
        assert_eq!(app.references.session.workspace.color, [20, 40, 60, 0]);
        assert_eq!(app.references.ring.unwrap().1, [20, 40, 60, 0]);
        frame(
            &ctx,
            &mut app,
            vec![egui::Event::PointerMoved(egui::pos2(1080.0, 10.0))],
        );
        assert_eq!(app.references.session.workspace.color, [20, 40, 60, 0]);
        assert!(app.references.ring.is_none());
        assert_eq!(app.documents[0].snapshot(), original);
        assert!(!app.documents[0].dirty);
        assert_eq!(app.documents[0].history_counts(), (0, 0));
        frame(&ctx, &mut app, vec![]);
        let button = app.references.open_document_button.unwrap();
        click(&ctx, &mut app, button);
        assert_eq!(app.documents.len(), 2);
        assert!(app.documents[1].path.is_none() && app.documents[1].source_path.is_none());
        assert_eq!(
            app.documents[1].layers[0].pixels,
            [[240, 30, 80, 128], [20, 40, 60, 0]]
        );
        assert_eq!(app.documents[0].snapshot(), original);
        frame(&ctx, &mut app, vec![]);
        let button = app.references.remove_button.unwrap();
        click(&ctx, &mut app, button);
        assert!(app.references.session.workspace.entries.is_empty());
        assert_eq!(app.documents.len(), 2);
    }
    #[test]
    fn mouse_grid_and_guide_editing_use_document_undo_but_print_size_only_changes_view() {
        let ctx = egui::Context::default();
        let mut app = Drawer::new(&ctx, None);
        app.create_canvas(CanvasSpec {
            width: 600,
            height: 300,
            dpi: 300.0,
            background: Background::Color([10, 20, 30]),
        })
        .unwrap();
        app.documents[0].mark_saved("main.drawer".into());
        let original = app.documents[0].snapshot();
        app.aids.visible = true;
        frame(&ctx, &mut app, vec![]);
        frame(&ctx, &mut app, vec![]);
        app.aids.preferences.unit = drawer_core::aids::Unit::Millimeters;
        app.aids.spacing = ["5".into(), "10".into()];
        let button = app.aids.apply_button.unwrap();
        click(&ctx, &mut app, button);
        assert!((app.documents[0].scene.grid.spacing[0] - 5.0 * 300.0 / 25.4).abs() < 1e-9);
        assert_eq!(app.documents[0].history_counts(), (1, 0));
        assert_eq!(app.documents[0].layers, original.layers);
        frame(&ctx, &mut app, vec![]);
        app.aids.guide_position = "25.4".into();
        let button = app.aids.guide_button.unwrap();
        click(&ctx, &mut app, button);
        assert_eq!(
            app.documents[0].scene.guides[0],
            [[0.0, 300.0], [600.0, 300.0]]
        );
        assert_eq!(app.documents[0].history_counts(), (2, 0));
        assert!(app.documents[0].undo());
        assert!(app.documents[0].undo());
        assert_eq!(app.documents[0].snapshot(), original);
        assert!(!app.documents[0].dirty);
        app.aids.preferences.monitor_ppi = Some(96.0);
        frame(&ctx, &mut app, vec![]);
        frame(&ctx, &mut app, vec![]);
        let button = app.aids.print_button.unwrap();
        click(&ctx, &mut app, button);
        assert_eq!(app.documents[0].view.zoom, 0.32);
        assert_eq!(app.documents[0].snapshot(), original);
        assert!(!app.documents[0].dirty);
        assert_eq!(app.documents[0].history_counts(), (0, 2));
        app.aids.spacing[0] = "0".into();
        assert!(app.apply_grid().is_err());
        assert_eq!(app.documents[0].snapshot(), original);
    }

    fn wait_import(ctx: &egui::Context, app: &mut Drawer) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            frame(ctx, app, vec![]);
            let dialog = app.import_dialog.as_ref().unwrap();
            if dialog.ready() || dialog.failed() {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "import worker timeout"
            );
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        frame(ctx, app, vec![]);
        frame(ctx, app, vec![]);
    }
    #[test]
    fn clipboard_errors_add_nothing_and_new_document_has_independent_pixels_and_history() {
        use drawer_core::references::LocalImage;
        let ctx = egui::Context::default();
        let mut app = Drawer::new(&ctx, None);
        app.create_canvas(tiny_spec()).unwrap();
        app.documents[0].edit_pixel(0, 0, 0, [90, 80, 70, 255]);
        let original = app.documents[0].snapshot();
        for error in ["Empty clipboard", "Clipboard contains text, not an image"] {
            assert!(app.document_from_clipboard(|| Err(error.into())).is_err());
            assert_eq!(app.documents.len(), 1);
        }
        assert!(
            app.document_from_clipboard(|| Ok(LocalImage {
                width: 2,
                height: 1,
                pixels: vec![[0; 4]]
            }))
            .is_err()
        );
        app.document_from_clipboard(|| {
            LocalImage::from_rgba(2, 1, &[1, 2, 3, 0, 4, 5, 6, 128], 8).map_err(|e| e.to_string())
        })
        .unwrap();
        assert_eq!(app.documents.len(), 2);
        let doc = &app.documents[1];
        assert_eq!((doc.width, doc.height), (2, 1));
        assert_eq!(doc.layers[0].pixels, [[1, 2, 3, 0], [4, 5, 6, 128]]);
        assert!(doc.path.is_none() && doc.source_path.is_none() && doc.dirty);
        assert_eq!(doc.history_counts(), (0, 0));
        assert_eq!(app.documents[0].snapshot(), original);
        assert_eq!(app.documents[0].history_counts(), (1, 0));
    }
    #[test]
    fn save_as_export_and_overwrite_cancel_keep_the_correct_paths_and_files() {
        let root = TestDirectory::new();
        let storage = UserStorage::at(root.0.join("userdata")).unwrap();
        let ctx = egui::Context::default();
        let mut app = Drawer::new(&ctx, None).with_storage(storage);
        app.create_canvas(tiny_spec()).unwrap();
        let a = root.0.join("a.drawer");
        let b = root.0.join("b.drawer");
        let output = root.0.join("export.png");
        assert!(!app.save_as_from(0, || None).unwrap());
        assert!(app.documents[0].path.is_none());
        assert!(app.save_as_from(0, || Some(a.clone())).unwrap());
        let original_file = std::fs::read(&a).unwrap();
        app.documents[0].edit_pixel(0, 0, 0, [20, 40, 60, 128]);
        let snapshot = app.documents[0].snapshot();
        let history = app.documents[0].history_counts();
        std::fs::write(&b, "keep prior file").unwrap();
        std::fs::write(&output, "keep prior export").unwrap();
        // None also covers the native dialog's refusal to overwrite an existing destination.
        assert!(!app.save_as_from(0, || None).unwrap());
        assert!(!app.export_from(0, || None).unwrap());
        assert_eq!(std::fs::read_to_string(&b).unwrap(), "keep prior file");
        assert_eq!(
            std::fs::read_to_string(&output).unwrap(),
            "keep prior export"
        );
        assert_eq!(app.documents[0].path, Some(a.canonicalize().unwrap()));
        assert!(app.documents[0].dirty);
        assert_eq!(app.documents[0].history_counts(), history);
        assert_eq!(app.documents[0].snapshot(), snapshot);
        assert!(app.save_as_from(0, || Some(b.clone())).unwrap());
        assert_eq!(app.documents[0].path, Some(b.canonicalize().unwrap()));
        assert!(!app.documents[0].dirty);
        assert_eq!(std::fs::read(&a).unwrap(), original_file);
        app.documents[0].edit_pixel(0, 0, 0, [70, 80, 90, 255]);
        let state = app.documents[0].snapshot();
        let history = app.documents[0].history_counts();
        assert!(app.export_from(0, || Some(output.clone())).unwrap());
        assert_eq!(app.documents[0].path, Some(b.canonicalize().unwrap()));
        assert!(app.documents[0].dirty);
        assert_eq!(app.documents[0].snapshot(), state);
        assert_eq!(app.documents[0].history_counts(), history);
        assert_eq!(image_import_pixels(&output), vec![[70, 80, 90, 255]]);
        let list = app.recent.paths().to_vec();
        assert_eq!(list.len(), 2);
        assert!(!list.contains(&output));
        let c = root.0.join("c.drawer");
        std::fs::write(root.0.join("c.drawer.drawer-write.tmp"), "foreign").unwrap();
        assert!(app.save_as_from(0, || Some(c)).is_err());
        assert_eq!(app.documents[0].path, Some(b.canonicalize().unwrap()));
        assert!(app.documents[0].dirty);
        let mut reopened =
            Drawer::new(&ctx, None).with_storage(UserStorage::at(root.0.join("userdata")).unwrap());
        assert_eq!(reopened.recent.paths(), list);
        reopened.open_path(&list[0]).unwrap();
        std::fs::remove_file(&a).unwrap();
        let before = reopened.documents[0].snapshot();
        assert!(reopened.open_path(&a).is_err());
        assert_eq!(reopened.documents.len(), 1);
        assert_eq!(reopened.documents[0].snapshot(), before);
        assert_eq!(reopened.recent.paths(), list);
    }
    fn image_import_pixels(path: &std::path::Path) -> Vec<[u8; 4]> {
        let cancel = std::sync::atomic::AtomicBool::new(false);
        drawer_core::image_import::ImageSource::read(path, DEFAULT_DOCUMENT_BUDGET, &cancel)
            .unwrap()
            .decode(0, true, DEFAULT_DOCUMENT_BUDGET, &cancel)
            .unwrap()
            .image
            .pixels
    }
    #[test]
    fn mouse_import_cancel_confirm_and_repeat_create_independent_documents_keep_source() {
        let root = TestDirectory::new();
        let path = root.0.join("input.png");
        let image =
            drawer_core::references::LocalImage::from_rgba(2, 1, &[1, 2, 3, 0, 4, 5, 6, 128], 8)
                .unwrap();
        // Write a local reference copy, then use it as a read-only input fixture.
        let storage = UserStorage::at(root.0.join("userdata")).unwrap();
        let mut session = drawer_core::references::ReferenceSession::default();
        session
            .add(&storage, image.clone(), "fixture".into(), None)
            .unwrap();
        std::fs::copy(
            storage.root().join("materials/references/reference-1.png"),
            &path,
        )
        .unwrap();
        let source_bytes = std::fs::read(&path).unwrap();
        let ctx = egui::Context::default();
        let mut app = Drawer::new(&ctx, None).with_storage(storage);
        app.create_canvas(tiny_spec()).unwrap();
        app.documents[0].edit_pixel(0, 0, 0, [200, 50, 10, 255]);
        let original = app.documents[0].snapshot();
        app.begin_import(&ctx, &path);
        wait_import(&ctx, &mut app);
        assert_eq!(app.import_dialog.as_ref().unwrap().preview_image(), &image);
        let cancel = app.import_dialog.as_ref().unwrap().cancel_button.unwrap();
        click(&ctx, &mut app, cancel);
        assert!(app.import_dialog.is_none());
        assert_eq!(app.documents.len(), 1);
        assert!(app.recent.paths().is_empty());
        app.begin_import(&ctx, &path);
        wait_import(&ctx, &mut app);
        let confirm = app.import_dialog.as_ref().unwrap().import_button.unwrap();
        click(&ctx, &mut app, confirm);
        assert!(app.import_dialog.is_none());
        assert_eq!(app.documents.len(), 2);
        assert_eq!(app.documents[1].layers[0].pixels, image.pixels);
        assert_eq!(app.documents[1].history_counts(), (0, 0));
        assert!(app.documents[1].path.is_none() && app.documents[1].source_path.is_none());
        let id = app.documents[1].id;
        assert!(app.export_png_to(1, &path).is_err());
        app.save_to(1, &root.0.join("saved.drawer")).unwrap();
        assert!(app.export_png_to(1, &path).is_err());
        app.begin_import(&ctx, &path);
        wait_import(&ctx, &mut app);
        app.accept_import().unwrap();
        assert_eq!(app.documents.len(), 3);
        assert_ne!(app.documents[2].id, id);
        assert_eq!(app.documents[0].snapshot(), original);
        assert_eq!(app.documents[0].history_counts(), (1, 0));
        assert_eq!(std::fs::read(&path).unwrap(), source_bytes);
    }
    #[test]
    fn tiff_preview_option_changes_and_failed_canceled_jobs_never_add_empty_documents() {
        use drawer_core::image_import::ImageSource;
        let root = TestDirectory::new();
        let path = root.0.join("pages.tif");
        let mut file = std::fs::File::create(&path).unwrap();
        let mut encoder = tiff::encoder::TiffEncoder::new(&mut file).unwrap();
        encoder
            .write_image::<tiff::encoder::colortype::RGB8>(2, 1, &[255, 0, 0, 0, 255, 0])
            .unwrap();
        let mut page = encoder
            .new_image::<tiff::encoder::colortype::RGB8>(1, 2)
            .unwrap();
        page.encoder()
            .write_tag(tiff::tags::Tag::Orientation, 6u16)
            .unwrap();
        page.write_data(&[0, 0, 255, 255, 255, 0]).unwrap();
        drop(file);
        let ctx = egui::Context::default();
        let mut app = Drawer::new(&ctx, None);
        app.create_canvas(tiny_spec()).unwrap();
        let original = app.documents[0].snapshot();
        app.begin_import(&ctx, &path);
        wait_import(&ctx, &mut app);
        {
            let dialog = app.import_dialog.as_mut().unwrap();
            dialog.page = 1;
            dialog.start_page(&ctx, DEFAULT_DOCUMENT_BUDGET);
        }
        wait_import(&ctx, &mut app);
        let preview = app.import_dialog.as_ref().unwrap().preview_image();
        assert_eq!((preview.width, preview.height), (2, 1));
        let orientation = app
            .import_dialog
            .as_ref()
            .unwrap()
            .orientation_button
            .unwrap();
        click(&ctx, &mut app, orientation);
        wait_import(&ctx, &mut app);
        let preview = app.import_dialog.as_ref().unwrap().preview_image().clone();
        assert_eq!((preview.width, preview.height), (1, 2));
        let expected = ImageSource::read(
            &path,
            DEFAULT_DOCUMENT_BUDGET,
            &std::sync::atomic::AtomicBool::new(false),
        )
        .unwrap()
        .decode(
            1,
            false,
            DEFAULT_DOCUMENT_BUDGET,
            &std::sync::atomic::AtomicBool::new(false),
        )
        .unwrap()
        .image;
        assert_eq!(preview, expected);
        app.accept_import().unwrap();
        assert_eq!(app.documents[1].layers[0].pixels, preview.pixels);
        app.begin_import(&ctx, &path);
        app.import_dialog = None;
        app.begin_import(&ctx, &root.0.join("missing.png"));
        wait_import(&ctx, &mut app);
        assert!(app.import_dialog.as_ref().unwrap().failed());
        assert_eq!(app.documents.len(), 2);
        assert!(app.accept_import().is_err());
        app.import_dialog = None;
        assert_eq!(app.documents[0].snapshot(), original);
    }

    fn display_monitors() -> Vec<drawer_core::display_view::Monitor> {
        vec![
            drawer_core::display_view::Monitor {
                key: "main".into(),
                index: 0,
                position: [0, 0],
                size: [1920, 1080],
                scale: 1.0,
                primary: true,
            },
            drawer_core::display_view::Monitor {
                key: "external".into(),
                index: 1,
                position: [-3840, -200],
                size: [3840, 2160],
                scale: 2.0,
                primary: false,
            },
        ]
    }
    #[test]
    fn display_and_document_tabs_share_pixel_updates_but_keep_independent_framing() {
        let ctx = egui::Context::default();
        let mut app = Drawer::new(&ctx, None);
        app.create_canvas(CanvasSpec {
            width: 4,
            height: 3,
            ..tiny_spec()
        })
        .unwrap();
        app.display.monitors = display_monitors();
        app.display.preferences.visible = true;
        app.display.preferences.synced = false;
        app.display.preferences.framing = drawer_core::display_view::Framing {
            zoom: 4.0,
            center: [0.25, 0.75],
            angle: 37.0,
            flipped: true,
        };
        app.documents[0].view.zoom = 2.0;
        app.duplicate_view();
        app.documents[0].view.zoom = 3.0;
        frame(&ctx, &mut app, vec![]);
        let id = app.documents[0].id;
        let independent = app.display.framing_for(id).unwrap();
        assert_eq!(independent.zoom, 4.0);
        assert_eq!(app.documents[0].view.zoom, 3.0);
        let original = app.documents[0].snapshot();
        app.documents[0].edit_pixel(0, 1, 1, [200, 30, 70, 255]);
        frame(&ctx, &mut app, vec![]);
        assert_eq!(
            app.canvas_view.rendered_revision(),
            app.display.canvas.rendered_revision()
        );
        assert_eq!(
            app.display.canvas.rendered_revision(),
            Some((id, app.documents[0].revision))
        );
        assert_eq!(app.display.framing_for(id), Some(independent));
        app.select_tab(0);
        frame(&ctx, &mut app, vec![]);
        assert_eq!(app.documents[0].view.zoom, 2.0);
        assert_eq!(
            app.canvas_view.rendered_revision(),
            app.display.canvas.rendered_revision()
        );
        app.request_close(1);
        assert_eq!(app.documents.len(), 1);
        assert_eq!(app.tabs.len(), 1);
        assert!(app.documents[0].undo());
        frame(&ctx, &mut app, vec![]);
        assert_eq!(app.documents[0].snapshot(), original);
        assert_eq!(
            app.canvas_view.rendered_revision(),
            app.display.canvas.rendered_revision()
        );
        app.create_canvas(CanvasSpec {
            width: 2,
            height: 2,
            ..tiny_spec()
        })
        .unwrap();
        frame(&ctx, &mut app, vec![]);
        let second = app.documents[1].id;
        assert_eq!(
            app.display.canvas.rendered_revision(),
            Some((second, app.documents[1].revision))
        );
        app.display.preferences.framing.zoom = 6.0;
        app.select_tab(0);
        frame(&ctx, &mut app, vec![]);
        assert_eq!(app.display.framing_for(id), Some(independent));
        assert_eq!(app.documents[0].view.zoom, 2.0);
        assert_eq!(app.documents[0].snapshot(), original);
    }
    #[test]
    fn display_navigation_fullscreen_keys_disconnect_and_close_preserve_drawing() {
        let ctx = egui::Context::default();
        let mut app = Drawer::new(&ctx, None);
        app.create_canvas(CanvasSpec {
            width: 80,
            height: 60,
            ..tiny_spec()
        })
        .unwrap();
        let original = app.documents[0].snapshot();
        let before = app.documents[0].view;
        let history = app.documents[0].history_counts();
        app.display.monitors = display_monitors();
        app.display.preferences.visible = true;
        app.display.preferences.synced = false;
        app.display.preferences.preferred = Some("external".into());
        frame(&ctx, &mut app, vec![]);
        frame(&ctx, &mut app, vec![]);
        let rect = app.display.canvas.viewport.unwrap();
        let pos = rect.center();
        frame(&ctx, &mut app, vec![egui::Event::PointerMoved(pos)]);
        frame(
            &ctx,
            &mut app,
            vec![egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: egui::vec2(0.0, 120.0),
                modifiers: Default::default(),
                phase: egui::TouchPhase::Move,
            }],
        );
        assert!(app.display.preferences.framing.zoom > 1.0);
        assert_eq!(app.documents[0].view, before);
        let key = |key| egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Default::default(),
        };
        frame(&ctx, &mut app, vec![key(egui::Key::Escape)]);
        assert!(!app.display.preferences.fullscreen);
        frame(&ctx, &mut app, vec![key(egui::Key::F11)]);
        assert!(app.display.preferences.fullscreen);
        app.display.monitors.pop();
        frame(&ctx, &mut app, vec![]);
        assert_eq!(
            drawer_core::display_view::selected(
                &app.display.monitors,
                app.display.preferences.preferred.as_deref()
            )
            .unwrap()
            .key,
            "main"
        );
        assert_eq!(
            app.display.preferences.preferred.as_deref(),
            Some("external")
        );
        app.display.monitors = display_monitors();
        frame(&ctx, &mut app, vec![]);
        assert_eq!(
            drawer_core::display_view::selected(
                &app.display.monitors,
                app.display.preferences.preferred.as_deref()
            )
            .unwrap()
            .key,
            "external"
        );
        let mut raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1100.0, 760.0),
            )),
            ..Default::default()
        };
        raw.viewports
            .get_mut(&egui::ViewportId::ROOT)
            .unwrap()
            .events
            .push(egui::ViewportEvent::Close);
        let mut output = ctx.run_ui(raw, |ui| app.show(ui));
        output.textures_delta.clear();
        assert!(!app.display.preferences.visible);
        assert_eq!(app.documents.len(), 1);
        assert_eq!(app.tabs.len(), 1);
        assert_eq!(app.documents[0].snapshot(), original);
        assert_eq!(app.documents[0].history_counts(), history);
        assert_eq!(app.documents[0].view, before);
    }
    #[test]
    fn display_settings_undo_redo_restart_and_save_failure_never_prevent_closing() {
        let root = TestDirectory::new();
        let storage = UserStorage::at(root.0.join("userdata")).unwrap();
        let ctx = egui::Context::default();
        let mut app = Drawer::new(&ctx, None).with_storage(storage);
        app.create_canvas(tiny_spec()).unwrap();
        let before = app.documents[0].snapshot();
        app.display.settings_open = true;
        app.display.preferences.visible = true;
        frame(&ctx, &mut app, vec![]);
        frame(&ctx, &mut app, vec![]);
        let close = app.display.close_button.unwrap();
        click(&ctx, &mut app, close);
        assert!(!app.display.preferences.visible);
        frame(&ctx, &mut app, vec![]);
        let undo = app.display.undo_button.unwrap();
        click(&ctx, &mut app, undo);
        assert!(app.display.preferences.visible);
        frame(&ctx, &mut app, vec![]);
        let redo = app.display.redo_button.unwrap();
        click(&ctx, &mut app, redo);
        assert!(!app.display.preferences.visible);
        let storage = app.storage.as_ref().unwrap();
        assert_eq!(
            drawer_core::display_view::Preferences::load(storage).unwrap(),
            app.display.preferences
        );
        assert!(
            drawer_core::display_view::Preferences::previous(storage)
                .unwrap()
                .visible
        );
        let mut restarted =
            Drawer::new(&ctx, None).with_storage(UserStorage::at(root.0.join("userdata")).unwrap());
        assert_eq!(restarted.display.preferences, app.display.preferences);
        restarted.display.preferences.visible = true;
        restarted
            .display
            .preferences
            .save(restarted.storage.as_ref().unwrap())
            .unwrap();
        restarted.display.settings_open = true;
        std::fs::write(
            root.0.join("userdata/settings/display.json.write.tmp"),
            "foreign operation",
        )
        .unwrap();
        frame(&ctx, &mut restarted, vec![]);
        frame(&ctx, &mut restarted, vec![]);
        let close = restarted.display.close_button.unwrap();
        click(&ctx, &mut restarted, close);
        assert!(!restarted.display.preferences.visible);
        assert!(restarted.display.error.is_some());
        assert!(
            drawer_core::display_view::Preferences::load(restarted.storage.as_ref().unwrap())
                .unwrap()
                .visible
        );
        assert_eq!(app.documents[0].snapshot(), before);
        assert_eq!(app.documents[0].history_counts(), (0, 0));
    }
    fn wait_sizing(ctx: &egui::Context, app: &mut Drawer) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while app.sizing_dialog.is_some() {
            frame(ctx, app, vec![]);
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    }
    #[test]
    fn physical_creation_dpi_policies_signed_edge_mouse_apply_and_presets_restart() {
        use drawer_core::{aids::Unit, canvas_size::Dimensions};
        let root = TestDirectory::new();
        let ctx = egui::Context::default();
        let mut app =
            Drawer::new(&ctx, None).with_storage(UserStorage::at(root.0.join("userdata")).unwrap());
        app.show_new_canvas = true;
        app.new_canvas.size = Dimensions::new(300, 150, 300.0);
        app.new_canvas.size.switch_unit(Unit::Millimeters).unwrap();
        frame(&ctx, &mut app, vec![]);
        frame(&ctx, &mut app, vec![]);
        let create = app.create_button.unwrap();
        click(&ctx, &mut app, create);
        assert_eq!(
            (app.documents[0].width, app.documents[0].height),
            (300, 150)
        );
        let original = app.documents[0].snapshot();
        app.begin_sizing(sizing_dialog::Mode::Resample);
        let dialog = app.sizing_dialog.as_mut().unwrap();
        dialog.keep_pixels = true;
        dialog.dpi = "600".into();
        frame(&ctx, &mut app, vec![]);
        frame(&ctx, &mut app, vec![]);
        let apply = app.sizing_dialog.as_ref().unwrap().apply_button.unwrap();
        click(&ctx, &mut app, apply);
        wait_sizing(&ctx, &mut app);
        assert_eq!(app.documents[0].dpi, 600.0);
        assert_eq!(app.documents[0].layers, original.layers);
        assert!(app.documents[0].undo());
        app.begin_sizing(sizing_dialog::Mode::Resample);
        let dialog = app.sizing_dialog.as_mut().unwrap();
        dialog.dpi = "600".into();
        dialog.size.edited_dpi("300", false).unwrap();
        frame(&ctx, &mut app, vec![]);
        frame(&ctx, &mut app, vec![]);
        let apply = app.sizing_dialog.as_ref().unwrap().apply_button.unwrap();
        click(&ctx, &mut app, apply);
        wait_sizing(&ctx, &mut app);
        assert_eq!(
            (app.documents[0].width, app.documents[0].height),
            (600, 300)
        );
        assert!(app.documents[0].undo());
        app.begin_sizing(sizing_dialog::Mode::Canvas);
        let dialog = app.sizing_dialog.as_mut().unwrap();
        dialog.use_edges = true;
        dialog.edges = ["-1".into(), "0".into(), "2".into(), "1".into()];
        dialog.confirmed = true;
        frame(&ctx, &mut app, vec![]);
        frame(&ctx, &mut app, vec![]);
        let apply = app.sizing_dialog.as_ref().unwrap().apply_button.unwrap();
        click(&ctx, &mut app, apply);
        wait_sizing(&ctx, &mut app);
        assert_eq!(
            (app.documents[0].width, app.documents[0].height),
            (301, 151)
        );
        assert!(app.documents[0].undo());
        assert_eq!(app.documents[0].snapshot(), original);
        app.size_presets
            .add(
                &Dimensions::new(3, 2, 144.0),
                "User size".into(),
                app.storage.as_ref(),
            )
            .unwrap();
        assert!(
            app.size_presets
                .add(
                    &Dimensions::new(9, 9, 300.0),
                    "User size".into(),
                    app.storage.as_ref()
                )
                .is_err()
        );
        let restored =
            Drawer::new(&ctx, None).with_storage(UserStorage::at(root.0.join("userdata")).unwrap());
        assert_eq!(restored.size_presets.book, app.size_presets.book);
        app.size_presets
            .history(false, app.storage.as_ref())
            .unwrap();
        assert_eq!(app.size_presets.book.entries.len(), 3);
        app.size_presets
            .history(true, app.storage.as_ref())
            .unwrap();
        assert_eq!(app.size_presets.book, restored.size_presets.book);
        assert_eq!(app.documents[0].snapshot(), original);
    }
    #[test]
    fn paste_preview_center_origin_mouse_move_cancel_commit_copy_and_undo() {
        use drawer_core::{paste::ClipboardImage, references::LocalImage};
        let root = TestDirectory::new();
        let ctx = egui::Context::default();
        let mut app = Drawer::new(&ctx, None);
        app.create_canvas(CanvasSpec {
            width: 4,
            height: 3,
            ..tiny_spec()
        })
        .unwrap();
        app.save_to(0, &root.0.join("saved.drawer")).unwrap();
        let original = app.documents[0].snapshot();
        let pixels = vec![[120, 30, 80, 128]; 8];
        let make = || {
            Ok(ClipboardImage {
                image: LocalImage {
                    width: 4,
                    height: 2,
                    pixels: pixels.clone(),
                },
                origin: Some([-20, 30]),
            })
        };
        assert!(
            app.begin_paste(&ctx, || Err("Clipboard contains text".into()))
                .is_err()
        );
        assert!(app.paste_dialog.is_none());
        app.begin_paste(&ctx, make).unwrap();
        assert_eq!(app.paste_dialog.as_ref().unwrap().offset, [0, 0]);
        assert_eq!(app.documents[0].snapshot(), original);
        assert!(!app.documents[0].dirty);
        frame(&ctx, &mut app, vec![]);
        frame(&ctx, &mut app, vec![]);
        let origin = app.paste_dialog.as_ref().unwrap().origin_button.unwrap();
        click(&ctx, &mut app, origin);
        assert_eq!(app.paste_dialog.as_ref().unwrap().offset, [-20, 30]);
        let rect = app.paste_dialog.as_ref().unwrap().viewport();
        let pos = rect.center();
        frame(
            &ctx,
            &mut app,
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: Default::default(),
                },
            ],
        );
        frame(
            &ctx,
            &mut app,
            vec![egui::Event::PointerMoved(pos + egui::vec2(64.0, 0.0))],
        );
        frame(
            &ctx,
            &mut app,
            vec![egui::Event::PointerButton {
                pos: pos + egui::vec2(64.0, 0.0),
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: Default::default(),
            }],
        );
        assert_ne!(app.paste_dialog.as_ref().unwrap().offset, [-20, 30]);
        let cancel = app.paste_dialog.as_ref().unwrap().cancel_button.unwrap();
        click(&ctx, &mut app, cancel);
        assert!(app.paste_dialog.is_none());
        assert_eq!(app.documents[0].snapshot(), original);
        assert_eq!(app.documents[0].history_counts(), (0, 0));
        assert!(!app.documents[0].dirty);
        app.begin_paste(&ctx, make).unwrap();
        app.paste_dialog.as_mut().unwrap().move_to([1, 1]).unwrap();
        frame(&ctx, &mut app, vec![]);
        frame(&ctx, &mut app, vec![]);
        let place = app.paste_dialog.as_ref().unwrap().apply_button.unwrap();
        click(&ctx, &mut app, place);
        assert!(app.paste_dialog.is_none());
        assert_eq!(app.documents[0].layers.len(), 2);
        assert_eq!(app.documents[0].history_counts(), (1, 0));
        assert_eq!(app.documents[0].composite_pixel(1, 1), [120, 30, 80, 128]);
        app.copy_top_layer_from(|packet, budget| {
            assert_eq!(packet.origin, Some([1, 1]));
            assert_eq!(packet.image.pixels, pixels);
            assert!(packet.encode(budget).is_ok());
            Ok(())
        })
        .unwrap();
        app.save_to(0, &root.0.join("pasted.drawer")).unwrap();
        assert_eq!(
            drawer_core::files::open(&root.0.join("pasted.drawer"), 10000)
                .unwrap()
                .snapshot(),
            app.documents[0].snapshot()
        );
        assert!(app.documents[0].undo());
        assert_eq!(app.documents[0].snapshot(), original);
        app.begin_paste(&ctx, || {
            Ok(ClipboardImage {
                image: LocalImage {
                    width: 8,
                    height: 6,
                    pixels: vec![[255, 0, 0, 0]; 48],
                },
                origin: None,
            })
        })
        .unwrap();
        assert_eq!(app.paste_dialog.as_ref().unwrap().offset, [-2, -2]);
        app.accept_paste().unwrap();
        assert_eq!(app.documents[0].layers[1].pixels.len(), 48);
        assert_eq!(app.documents[0].layers[1].pixels[0], [255, 0, 0, 0]);
    }
    #[test]
    fn view_quality_changes_render_all_filters_and_leave_saved_pixels_and_history_unchanged() {
        use drawer_core::interpolation::Filter;
        let root = TestDirectory::new();
        let ctx = egui::Context::default();
        let mut app = Drawer::new(&ctx, None);
        app.create_canvas(CanvasSpec {
            width: 8,
            height: 8,
            ..tiny_spec()
        })
        .unwrap();
        for (i, p) in app.documents[0].layers[0].pixels.iter_mut().enumerate() {
            *p = if (i % 8 + i / 8) % 2 == 0 {
                [255; 4]
            } else {
                [255, 0, 0, 0]
            };
        }
        app.save_to(0, &root.0.join("quality.drawer")).unwrap();
        let original = app.documents[0].snapshot();
        let history = app.documents[0].history_counts();
        app.export_png_to(0, &root.0.join("before.png")).unwrap();
        let bytes = std::fs::read(root.0.join("before.png")).unwrap();
        for angle in [0.0, 37.0, 90.0] {
            for filter in Filter::ALL {
                app.documents[0].view.angle = angle;
                app.documents[0].view.zoom = 0.5;
                app.interpolation.view = filter;
                frame(&ctx, &mut app, vec![]);
                assert_eq!(app.documents[0].snapshot(), original);
                assert_eq!(app.documents[0].history_counts(), history);
                assert!(!app.documents[0].dirty);
            }
        }
        app.export_png_to(0, &root.0.join("after.png")).unwrap();
        assert_eq!(std::fs::read(root.0.join("after.png")).unwrap(), bytes);
        app.interpolation.resolution = Filter::Area;
        app.begin_sizing(sizing_dialog::Mode::Resample);
        app.sizing_dialog = None;
    }
    #[test]
    fn ordered_views_restore_saved_documents_and_handle_missing_and_unsaved_views() {
        let root = TestDirectory::new();
        let ctx = egui::Context::default();
        let storage = UserStorage::at(root.0.join("userdata")).unwrap();
        let mut app = Drawer::new(&ctx, None)
            .with_storage(UserStorage::at(storage.root().to_path_buf()).unwrap());
        for n in 0..2 {
            app.create_canvas(CanvasSpec {
                width: 20,
                height: 10,
                ..tiny_spec()
            })
            .unwrap();
            app.save_to(n, &root.0.join(format!("{n}.drawer"))).unwrap();
            app.duplicate_view();
        }
        let original: Vec<_> = app
            .documents
            .iter()
            .map(|d| (d.snapshot(), d.dirty, d.history_counts()))
            .collect();
        app.tabs.swap(1, 2);
        app.active_tab = None;
        app.active = None;
        for n in 0..4 {
            app.select_tab(n);
            app.documents[app.active.unwrap()].view = drawer_core::view::View {
                zoom: (n + 1) as f64,
                angle: 37.0,
                flipped: true,
                pan: [n as f64 * 10.0, -30.0],
            };
        }
        app.select_tab(2);
        app.persist_views(1.5);
        let saved = app.saved_layout.clone();
        app.select_tab(8);
        assert_eq!(app.active_tab, Some(2));
        assert_eq!(app.documents.len(), 2);
        app.step_view(1);
        assert_eq!(app.active_tab, Some(3));
        app.step_view(1);
        assert_eq!(app.active_tab, Some(0));
        app.step_view(-1);
        assert_eq!(app.active_tab, Some(3));
        for (doc, before) in app.documents.iter().zip(&original) {
            assert_eq!((doc.snapshot(), doc.dirty, doc.history_counts()), *before);
        }
        let mut restart = Drawer::new(&ctx, None)
            .with_storage(UserStorage::at(storage.root().to_path_buf()).unwrap());
        restart.persist_views(1.5);
        assert_eq!(
            drawer_core::view_layout::Layout::load(&storage).unwrap(),
            saved
        );
        restart.restore_views(saved.clone(), 1.5).unwrap();
        assert_eq!(restart.documents.len(), 2);
        assert_eq!(restart.tabs.len(), 4);
        assert_eq!(restart.active_tab, Some(2));
        let after = restart.capture_views(1.5);
        for (a, b) in after.rows.iter().zip(&saved.rows) {
            assert_eq!(a.path, b.path);
            assert_eq!(
                (a.framing.zoom, a.framing.angle, a.framing.flipped),
                (b.framing.zoom, b.framing.angle, b.framing.flipped)
            );
            for axis in 0..2 {
                assert!((a.framing.center[axis] - b.framing.center[axis]).abs() < 1e-9);
            }
        }
        let mut invalid = saved.clone();
        invalid.rows[0].path = Some(root.0.join("missing.drawer"));
        assert!(
            restart
                .restore_views(invalid, 1.5)
                .unwrap_err()
                .contains("missing")
        );
        assert_eq!(restart.documents.len(), 2);
        restart.create_canvas(tiny_spec()).unwrap();
        let unsaved = restart.capture_views(1.5);
        let mut second = Drawer::new(&ctx, None);
        assert!(
            second
                .restore_views(unsaved, 1.5)
                .unwrap_err()
                .contains("not saved")
        );
        assert_eq!(second.documents.len(), 2);
        // Close the middle view: the remaining sequence has no gap in direct shortcut numbering.
        app.request_close(1);
        app.select_tab(1);
        assert_eq!(app.tabs.len(), 3);
        assert_eq!(app.documents.len(), 2);
        let before = std::fs::read(storage.root().join("settings/views.json")).unwrap();
        std::fs::write(
            storage.root().join("settings/views.json.write.tmp"),
            "foreign temp",
        )
        .unwrap();
        assert!(saved.save(&storage).is_err());
        assert_eq!(
            std::fs::read(storage.root().join("settings/views.json")).unwrap(),
            before
        );
    }
    #[test]
    fn mouse_region_zoom_and_direct_shortcuts_leave_drawing_and_history_unchanged() {
        let ctx = egui::Context::default();
        let mut app = Drawer::new(&ctx, None);
        app.create_canvas(CanvasSpec {
            width: 400,
            height: 300,
            ..tiny_spec()
        })
        .unwrap();
        app.duplicate_view();
        app.duplicate_view();
        app.documents[0].dirty = false;
        let before = app.documents[0].snapshot();
        let history = app.documents[0].history_counts();
        app.canvas_view.area_zoom = true;
        frame(&ctx, &mut app, vec![]);
        let rect = app.canvas_view.viewport.unwrap();
        let a = rect.center() - egui::vec2(40.0, 30.0);
        let b = rect.center() + egui::vec2(40.0, 30.0);
        for events in [
            vec![
                egui::Event::PointerMoved(a),
                egui::Event::PointerButton {
                    pos: a,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: Default::default(),
                },
            ],
            vec![egui::Event::PointerMoved(b)],
            vec![egui::Event::PointerButton {
                pos: b,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: Default::default(),
            }],
        ] {
            frame(&ctx, &mut app, events);
        }
        assert!(app.documents[0].view.zoom > 1.0);
        assert!(app.canvas_view.view_notice.is_none());
        click(
            &ctx,
            &mut app,
            egui::Rect::from_center_size(rect.center(), egui::vec2(2.0, 2.0)),
        );
        assert!(app.canvas_view.view_notice.is_some());
        for (key, expected) in [
            (egui::Key::Num1, 0),
            (egui::Key::Num3, 2),
            (egui::Key::Num9, 2),
        ] {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1100.0, 760.0),
                    )),
                    events: vec![egui::Event::Key {
                        key,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: egui::Modifiers::ALT,
                    }],
                    ..Default::default()
                },
                |ui| app.show(ui),
            );
            output.textures_delta.clear();
            assert_eq!(app.active_tab, Some(expected));
        }
        assert_eq!(app.documents[0].snapshot(), before);
        assert_eq!(app.documents[0].history_counts(), history);
        assert!(!app.documents[0].dirty);
        assert_eq!(app.documents.len(), 1);
    }
    #[test]
    fn navigator_selections_hover_focus_panel_layout_and_history_are_read_only() {
        use drawer_core::{
            navigator_preferences::Preferences,
            selection::{Mask, Run},
        };
        let root = TestDirectory::new();
        let ctx = egui::Context::default();
        let storage = UserStorage::at(root.0.join("userdata")).unwrap();
        let mut app = Drawer::new(&ctx, None)
            .with_storage(UserStorage::at(storage.root().to_path_buf()).unwrap());
        app.create_canvas(CanvasSpec {
            width: 20,
            height: 10,
            ..tiny_spec()
        })
        .unwrap();
        let mut scene = app.documents[0].scene.clone();
        scene.selection_mask = Some(Mask {
            runs: vec![
                Run {
                    x: 2,
                    y: 2,
                    length: 3,
                    alpha: 128,
                },
                Run {
                    x: 12,
                    y: 2,
                    length: 2,
                    alpha: 255,
                },
                Run {
                    x: 2,
                    y: 3,
                    length: 3,
                    alpha: 64,
                },
            ],
        });
        app.documents[0].edit_scene(scene).unwrap();
        app.save_to(0, &root.0.join("selection.drawer")).unwrap();
        assert_eq!(
            drawer_core::files::open(&root.0.join("selection.drawer"), DEFAULT_DOCUMENT_BUDGET)
                .unwrap()
                .snapshot(),
            app.documents[0].snapshot()
        );
        let before = app.documents[0].snapshot();
        let history = app.documents[0].history_counts();
        app.documents[0].view.angle = 37.0;
        app.documents[0].view.flipped = true;
        app.canvas_view.navigator.hover_outline = true;
        frame(&ctx, &mut app, vec![egui::Event::PointerGone]);
        assert!(!app.canvas_view.navigator_outline);
        let rect = app.canvas_view.navigator_rect.unwrap();
        frame(
            &ctx,
            &mut app,
            vec![egui::Event::PointerMoved(rect.center())],
        );
        assert!(app.canvas_view.navigator_outline);
        click(&ctx, &mut app, rect);
        frame(&ctx, &mut app, vec![egui::Event::PointerGone]);
        assert!(app.canvas_view.navigator_outline);
        let viewport = app.canvas_view.viewport.unwrap();
        let poly = drawer_core::navigator::visible_polygon(
            app.documents[0].view,
            [20.0, 10.0],
            [
                [viewport.min.x as f64, viewport.min.y as f64],
                [viewport.max.x as f64, viewport.max.y as f64],
            ],
            ctx.pixels_per_point() as f64,
        );
        for (p, screen) in poly.into_iter().zip([
            viewport.left_top(),
            viewport.right_top(),
            viewport.right_bottom(),
            viewport.left_bottom(),
        ]) {
            let mapped = app.documents[0].view.document_to_screen(
                p,
                [20.0, 10.0],
                [viewport.center().x as f64, viewport.center().y as f64],
                1.0,
            );
            assert!(
                (mapped[0] - screen.x as f64).abs() < 1e-6
                    && (mapped[1] - screen.y as f64).abs() < 1e-6
            );
        }
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1100.0, 760.0),
                )),
                ..Default::default()
            },
            |ui| app.show(ui),
        );
        assert!(output.shapes.iter().any(|shape|matches!(&shape.shape,egui::Shape::Path(path) if path.fill==egui::Color32::from_rgba_unmultiplied(90,130,255,45))));
        output.textures_delta.clear();
        app.canvas_view.navigator.selection_outline = false;
        app.canvas_view.navigator.size = [80.0, 80.0];
        frame(&ctx, &mut app, vec![]);
        assert_eq!(
            app.canvas_view.navigator_rect.unwrap().size(),
            egui::vec2(80.0, 80.0)
        );
        app.canvas_view.navigator.floating = true;
        app.canvas_view.navigator.position = [500.0, 300.0];
        app.canvas_view.navigator_generation += 1;
        frame(&ctx, &mut app, vec![]);
        frame(&ctx, &mut app, vec![]);
        let initial = app.canvas_view.navigator.position;
        let title = egui::pos2(initial[0] + 40.0, initial[1] + 10.0);
        let end = title + egui::vec2(70.0, 30.0);
        for events in [
            vec![egui::Event::PointerMoved(title)],
            vec![egui::Event::PointerButton {
                pos: title,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: Default::default(),
            }],
            vec![egui::Event::PointerMoved(end)],
            vec![egui::Event::PointerButton {
                pos: end,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: Default::default(),
            }],
        ] {
            frame(&ctx, &mut app, events);
        }
        assert_ne!(app.canvas_view.navigator.position, initial);
        let moved = app.canvas_view.navigator;
        app.navigator_history(false).unwrap();
        app.navigator_history(true).unwrap();
        assert_eq!(app.canvas_view.navigator, moved);
        assert_eq!(Preferences::load(&storage).unwrap(), moved);
        let restarted = Drawer::new(&ctx, None)
            .with_storage(UserStorage::at(storage.root().to_path_buf()).unwrap());
        assert_eq!(restarted.canvas_view.navigator, moved);
        assert_eq!(app.documents[0].snapshot(), before);
        assert_eq!(app.documents[0].history_counts(), history);
        assert!(!app.documents[0].dirty);
        let previous = std::fs::read(storage.root().join("settings/navigator.json")).unwrap();
        std::fs::write(
            storage.root().join("settings/navigator.json.write.tmp"),
            "foreign",
        )
        .unwrap();
        assert!(moved.save(&storage).is_err());
        assert_eq!(
            std::fs::read(storage.root().join("settings/navigator.json")).unwrap(),
            previous
        );
        app.documents[0].scene.selection_mask = Some(Mask::default());
        frame(&ctx, &mut app, vec![]);
        assert_eq!(
            canvas_view::selection_status(&app.documents[0]),
            "Empty selection: all pixels blocked"
        );
        app.documents[0].scene.selection_mask = None;
        assert_eq!(
            canvas_view::selection_status(&app.documents[0]),
            "No selection: all pixels allowed"
        );
        std::fs::remove_file(storage.root().join("settings/navigator.json.write.tmp")).unwrap();
        std::fs::write(storage.root().join("settings/navigator.json"), "damaged").unwrap();
        std::fs::File::options()
            .write(true)
            .open(storage.root().join("settings/navigator.json"))
            .unwrap()
            .set_times(std::fs::FileTimes::new().set_modified(std::time::SystemTime::UNIX_EPOCH))
            .unwrap();
        assert!(Preferences::load(&storage).is_err());
        let mut recovered = Drawer::new(&ctx, None)
            .with_storage(UserStorage::at(storage.root().to_path_buf()).unwrap());
        assert_eq!(recovered.canvas_view.navigator, Preferences::default());
        assert!(recovered.status.as_ref().unwrap().contains("Navigator"));
        recovered.persist_navigator();
        assert_eq!(
            std::fs::read_to_string(storage.root().join("settings/navigator.damaged.json"))
                .unwrap(),
            "damaged"
        );
        let settings = Preferences::default();
        assert!(settings.outline_visible(false, false));
        assert!(
            !Preferences {
                hover_outline: true,
                ..settings
            }
            .outline_visible(false, false)
        );
        assert!(
            Preferences {
                hover_outline: true,
                ..settings
            }
            .outline_visible(false, true)
        );
    }
    fn movie_clip() -> drawer_core::animation::Clip {
        use drawer_core::animation::{Cell, Clip, Fps};
        Clip {
            fps: Fps {
                numerator: 2,
                denominator: 1,
            },
            cells: vec![
                Cell {
                    name: "Red".into(),
                    width: 2,
                    height: 1,
                    offset: [0; 2],
                    pixels: Arc::new(vec![[255, 0, 0, 255]; 2]),
                },
                Cell {
                    name: "Blue".into(),
                    width: 2,
                    height: 1,
                    offset: [0; 2],
                    pixels: Arc::new(vec![[0, 0, 255, 255]; 2]),
                },
            ],
            source_times: vec![0.0, 0.5],
            audio: None,
        }
    }
    #[test]
    fn revert_cancel_failure_success_and_denied_association_preserve_current_data() {
        let root = TestDirectory::new();
        let ctx = egui::Context::default();
        let mut app = Drawer::new(&ctx, None);
        app.create_canvas(tiny_spec()).unwrap();
        assert!(app.revert_from(0, true).is_err());
        let path = root.0.join("saved.drawer");
        app.save_to(0, &path).unwrap();
        let saved = app.documents[0].snapshot();
        app.documents[0].edit_pixel(0, 0, 0, [1, 2, 3, 255]);
        app.documents[0].view.angle = 37.0;
        app.duplicate_view();
        let before = app.documents[0].snapshot();
        let view = app.documents[0].view;
        let id = app.documents[0].id;
        let history = app.documents[0].history_counts();
        let original = std::fs::read(&path).unwrap();
        assert!(!app.revert_from(0, false).unwrap());
        assert_eq!(app.documents[0].snapshot(), before);
        assert_eq!(app.documents[0].history_counts(), history);
        std::fs::write(&path, "damaged").unwrap();
        assert!(app.revert_from(0, true).is_err());
        assert_eq!(app.documents[0].snapshot(), before);
        assert!(app.documents[0].dirty);
        std::fs::remove_file(&path).unwrap();
        assert!(app.revert_from(0, true).is_err());
        assert_eq!(app.documents[0].snapshot(), before);
        std::fs::write(&path, &original).unwrap();
        assert!(app.revert_from(0, true).unwrap());
        assert_eq!(app.documents[0].snapshot(), saved);
        assert_eq!(app.documents[0].id, id);
        assert_eq!(app.documents[0].view, view);
        assert_eq!(app.tabs.len(), 2);
        assert_eq!(app.documents[0].history_counts(), (0, 0));
        assert!(!app.documents[0].dirty);
        assert_eq!(std::fs::read(&path).unwrap(), original);
        app.file_association_from(|| Err("denied".into()));
        assert!(app.status.as_ref().unwrap().contains("denied"));
        assert_eq!(app.documents[0].snapshot(), saved);
        assert_eq!(std::fs::read(path).unwrap(), original);
    }
    #[test]
    fn movie_mouse_confirm_cancel_insert_limits_stale_target_and_frame_preview_are_atomic() {
        let root = TestDirectory::new();
        let source = root.0.join("provider.avi");
        std::fs::write(&source, "provider fixture").unwrap();
        let original = std::fs::read(&source).unwrap();
        let ctx = egui::Context::default();
        let mut app = Drawer::new(&ctx, None);
        app.create_canvas(tiny_spec()).unwrap();
        let before = app.documents[0].snapshot();
        app.begin_movie_from(&ctx, &source, false, None).unwrap();
        assert!(app.movie_dialog.as_ref().unwrap().decoder_missing());
        frame(&ctx, &mut app, vec![]);
        frame(&ctx, &mut app, vec![]);
        let button = app.movie_dialog.as_ref().unwrap().cancel_button.unwrap();
        click(&ctx, &mut app, button);
        assert!(app.movie_dialog.is_none());
        assert_eq!(app.documents[0].snapshot(), before);
        assert_eq!(app.documents.len(), 1);
        app.movie_dialog = Some(movie_dialog::MovieDialog::injected(
            &source,
            movie_clip(),
            false,
            None,
        ));
        frame(&ctx, &mut app, vec![]);
        frame(&ctx, &mut app, vec![]);
        let button = app.movie_dialog.as_ref().unwrap().cancel_button.unwrap();
        click(&ctx, &mut app, button);
        assert!(app.movie_dialog.is_none());
        assert_eq!(app.documents[0].snapshot(), before);
        assert_eq!(app.documents.len(), 1);
        app.movie_dialog = Some(movie_dialog::MovieDialog::injected(
            &source,
            movie_clip(),
            false,
            None,
        ));
        frame(&ctx, &mut app, vec![]);
        frame(&ctx, &mut app, vec![]);
        let button = app.movie_dialog.as_ref().unwrap().accept_button.unwrap();
        click(&ctx, &mut app, button);
        assert!(app.movie_dialog.is_none());
        assert_eq!(app.documents.len(), 2);
        assert_eq!(app.documents[0].snapshot(), before);
        assert!(app.documents[1].path.is_none() && app.documents[1].dirty);
        let path = root.0.join("movie.drawer");
        app.save_to(1, &path).unwrap();
        let saved = app.documents[1].snapshot();
        let history = app.documents[1].history_counts();
        app.documents[1].current_frame = 1;
        frame(&ctx, &mut app, vec![]);
        assert_eq!(app.documents[1].composite_pixel(0, 0), [0, 0, 255, 255]);
        assert_eq!(app.documents[1].snapshot(), saved);
        assert_eq!(app.documents[1].history_counts(), history);
        assert!(!app.documents[1].dirty);
        let key = Some((app.documents[1].id, app.documents[1].revision));
        app.movie_dialog = Some(movie_dialog::MovieDialog::injected(
            &source,
            movie_clip(),
            true,
            key,
        ));
        app.movie_dialog.as_mut().unwrap().start_frame = "3".into();
        assert!(app.accept_movie().is_err());
        assert_eq!(app.documents[1].snapshot(), saved);
        app.movie_dialog.as_mut().unwrap().start_frame = "1".into();
        app.documents[1].edit_pixel(0, 0, 0, [20, 30, 40, 255]);
        let changed = app.documents[1].snapshot();
        assert!(app.accept_movie().is_err());
        assert_eq!(app.documents[1].snapshot(), changed);
        app.movie_dialog = None;
        let key = Some((app.documents[1].id, app.documents[1].revision));
        app.movie_dialog = Some(movie_dialog::MovieDialog::injected(
            &source,
            movie_clip(),
            true,
            key,
        ));
        frame(&ctx, &mut app, vec![]);
        frame(&ctx, &mut app, vec![]);
        let button = app.movie_dialog.as_ref().unwrap().accept_button.unwrap();
        click(&ctx, &mut app, button);
        assert!(app.movie_dialog.is_none());
        assert_eq!(app.documents[1].animation.as_ref().unwrap().tracks.len(), 2);
        assert!(app.documents[1].undo());
        assert_eq!(app.documents[1].snapshot(), changed);
        assert_eq!(std::fs::read(source).unwrap(), original);
    }
    #[test]
    fn panel_modes_restore_settings_and_keep_targets_dense_timeline_and_document_unchanged() {
        use drawer_core::{
            panels::{Bar, Kind, Settings, Thumbnail},
            thumbnail::Target,
        };
        let root = TestDirectory::new();
        let ctx = egui::Context::default();
        let storage = UserStorage::at(root.0.join("userdata")).unwrap();
        let mut app = Drawer::new(&ctx, None)
            .with_storage(UserStorage::at(storage.root().to_path_buf()).unwrap());
        let mut doc = movie_clip().document(1000000).unwrap();
        doc.layers.push(drawer_core::document::RasterLayer {
            name: "Outside".into(),
            placement: Some(drawer_core::document::PlacedRaster {
                width: 2,
                height: 1,
                offset: [50, -20],
            }),
            pixels: vec![[250, 0, 0, 128], [4, 5, 6, 0]],
        });
        let a = doc.animation.as_mut().unwrap();
        a.frames = 10000;
        a.tracks[0].end = 10000;
        a.tracks[0].exposures = (0..10000)
            .map(|frame| drawer_core::animation::Exposure {
                frame,
                cell: (frame % 2) as usize,
            })
            .collect();
        app.add_document(doc);
        app.save_to(0, &root.0.join("panels.drawer")).unwrap();
        let id = app.documents[0].id;
        app.panels.targets.insert(id, Target::Raster(1));
        let before = app.documents[0].snapshot();
        let history = app.documents[0].history_counts();
        let view = app.documents[0].view;
        for size in Thumbnail::ALL {
            for content in [false, true] {
                for checker in [false, true] {
                    for kind in [Kind::Layers, Kind::Timeline] {
                        let cfg = &mut app.panels.settings.panels[kind.index()];
                        cfg.thumbnail = size;
                        cfg.content = content;
                        cfg.checker = checker;
                        cfg.properties = Bar::Bottom;
                        cfg.commands = Bar::Top;
                    }
                    app.panels.settings.panels[Kind::Timeline.index()].animation_only = true;
                    frame(&ctx, &mut app, vec![]);
                    assert_eq!(app.panels.targets[&id], Target::Raster(1));
                    assert_eq!(app.documents[0].snapshot(), before);
                    assert_eq!(app.documents[0].view, view);
                    assert_eq!(app.documents[0].history_counts(), history);
                    assert!(!app.documents[0].dirty);
                }
            }
        }
        for panel in &mut app.panels.settings.panels {
            panel.visible = false;
        }
        frame(&ctx, &mut app, vec![]);
        let hidden = app.panels.settings;
        app.panels.settings = Settings::default();
        frame(&ctx, &mut app, vec![]);
        app.panels.history(false, app.storage.as_ref()).unwrap();
        assert_eq!(app.panels.settings, hidden);
        app.panels.history(true, app.storage.as_ref()).unwrap();
        assert_eq!(app.panels.settings, Settings::default());
        let expected = app.panels.settings;
        let restarted = Drawer::new(&ctx, None)
            .with_storage(UserStorage::at(storage.root().to_path_buf()).unwrap());
        assert_eq!(restarted.panels.settings, expected);
        for panel in &mut app.panels.settings.panels {
            panel.visible = false;
        }
        app.panels.settings.panels[Kind::Layers.index()].visible = true;
        frame(&ctx, &mut app, vec![]);
        frame(&ctx, &mut app, vec![]);
        // The resized dock reserves navigator space; scroll the panel to its last track.
        let header = app.layout.panel_rects[&Kind::Layers].min + egui::vec2(12.0, 12.0);
        frame(
            &ctx,
            &mut app,
            vec![
                egui::Event::PointerMoved(header),
                egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    phase: egui::TouchPhase::Move,
                    delta: egui::vec2(0.0, -160.0),
                    modifiers: Default::default(),
                },
            ],
        );
        for _ in 0..8 {
            frame(&ctx, &mut app, vec![]);
        }
        let rect = app.panels.targets_buttons[&Target::Track(0)];
        click(&ctx, &mut app, rect);
        assert_eq!(app.panels.targets[&id], Target::Track(0));
        assert_eq!(app.documents[0].snapshot(), before);
        let mut small = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(640.0, 480.0),
                )),
                ..Default::default()
            },
            |ui| app.show(ui),
        );
        small.textures_delta.clear();
        let viewport = app.canvas_view.viewport.unwrap();
        assert!(
            viewport.min.x >= 0.0
                && viewport.min.y >= 0.0
                && viewport.max.x <= 640.0
                && viewport.max.y <= 480.0,
            "{viewport:?}"
        );
        assert_eq!(app.panels.targets[&id], Target::Track(0));
        assert_eq!(app.documents[0].snapshot(), before);
        let normal = std::fs::read(storage.root().join("settings/panels.json")).unwrap();
        std::fs::write(
            storage.root().join("settings/panels.json.write.tmp"),
            "foreign temp",
        )
        .unwrap();
        assert!(app.panels.settings.save(&storage).is_err());
        assert_eq!(
            std::fs::read(storage.root().join("settings/panels.json")).unwrap(),
            normal
        );
        std::fs::remove_file(storage.root().join("settings/panels.json.write.tmp")).unwrap();
        std::fs::write(storage.root().join("settings/panels.json"), "damaged").unwrap();
        assert!(Settings::load(&storage).is_err());
        Settings::default().save(&storage).unwrap();
        assert_eq!(
            std::fs::read_to_string(storage.root().join("settings/panels.damaged.json")).unwrap(),
            "damaged"
        );
    }
    #[test]
    fn default_shortcut_modifiers_save_undo_tool_selection_and_text_focus() {
        let ctx = egui::Context::default();
        let root = TestDirectory::new();
        let mut app = Drawer::new(&ctx, None);
        app.create_canvas(tiny_spec()).unwrap();
        let path = root.0.join("keys.drawer");
        app.save_to(0, &path).unwrap();
        app.documents[0].edit_pixel(0, 0, 0, [10, 20, 30, 255]);
        let event = |key, modifiers| egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        };
        frame(
            &ctx,
            &mut app,
            vec![event(egui::Key::Z, egui::Modifiers::CTRL)],
        );
        assert_eq!(app.documents[0].layers[0].pixels[0], [0; 4]);
        frame(
            &ctx,
            &mut app,
            vec![event(egui::Key::Y, egui::Modifiers::CTRL)],
        );
        assert_eq!(app.documents[0].layers[0].pixels[0], [10, 20, 30, 255]);
        frame(
            &ctx,
            &mut app,
            vec![event(egui::Key::S, egui::Modifiers::CTRL)],
        );
        assert!(!app.documents[0].dirty);
        assert_eq!(
            drawer_core::files::open(&path, 1000000).unwrap().snapshot(),
            app.documents[0].snapshot()
        );
        for (key, tip) in [
            (egui::Key::N, BrushTip::Pencil),
            (egui::Key::E, BrushTip::Eraser),
            (egui::Key::W, BrushTip::Wash),
            (egui::Key::B, BrushTip::Pen),
        ] {
            frame(&ctx, &mut app, vec![event(key, egui::Modifiers::NONE)]);
            assert_eq!(app.selected, tip);
            frame(
                &ctx,
                &mut app,
                vec![egui::Event::Key {
                    key,
                    physical_key: None,
                    pressed: false,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                }],
            );
        }
        for (key, modifiers, expected) in [
            (egui::Key::S, egui::Modifiers::CTRL, Some(0)),
            (
                egui::Key::S,
                egui::Modifiers::CTRL | egui::Modifiers::SHIFT,
                Some(1),
            ),
            (
                egui::Key::Z,
                egui::Modifiers::CTRL | egui::Modifiers::SHIFT,
                Some(3),
            ),
            (egui::Key::B, egui::Modifiers::ALT, None),
            (egui::Key::B, egui::Modifiers::SHIFT, None),
            (
                egui::Key::S,
                egui::Modifiers::CTRL | egui::Modifiers::COMMAND,
                Some(0),
            ),
        ] {
            let mut action = None;
            let mut output = ctx.run_ui(
                egui::RawInput {
                    events: vec![event(key, modifiers)],
                    ..Default::default()
                },
                |ui| {
                    action = ui.ctx().input_mut(panels::shortcut_action);
                },
            );
            output.textures_delta.clear();
            assert_eq!(action, expected);
        }
        let mut text = String::new();
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            ui.text_edit_singleline(&mut text).request_focus();
        });
        output.textures_delta.clear();
        assert!(ctx.egui_wants_keyboard_input());
        frame(
            &ctx,
            &mut app,
            vec![event(egui::Key::E, egui::Modifiers::NONE)],
        );
        assert_eq!(app.selected, BrushTip::Pen);
    }
    fn dpi_frame(
        ctx: &egui::Context,
        app: &mut Drawer,
        native: f32,
        events: Vec<egui::Event>,
    ) -> f32 {
        let mut input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1100.0, 760.0),
            )),
            events,
            ..Default::default()
        };
        input
            .viewports
            .get_mut(&egui::ViewportId::ROOT)
            .unwrap()
            .native_pixels_per_point = Some(native);
        let mut output = ctx.run_ui(input, |ui| app.show(ui));
        let ppp = output.pixels_per_point;
        output.textures_delta.clear();
        ppp
    }
    #[test]
    fn mixed_monitor_scale_ui_zoom_cursor_and_all_view_centers_preserve_document_data() {
        let root = TestDirectory::new();
        let ctx = egui::Context::default();
        let storage = UserStorage::at(root.0.join("userdata")).unwrap();
        let mut app = Drawer::new(&ctx, None)
            .with_storage(UserStorage::at(storage.root().to_path_buf()).unwrap());
        app.create_canvas(CanvasSpec {
            width: 400,
            height: 300,
            ..tiny_spec()
        })
        .unwrap();
        app.documents[0].view = drawer_core::view::View {
            zoom: 1.2,
            angle: 37.0,
            pan: [43.0, -18.0],
            flipped: true,
        };
        app.duplicate_view();
        app.documents[0].view.pan = [-10.0, 30.0];
        app.sync_active_view();
        app.save_to(0, &root.0.join("dpi.drawer")).unwrap();
        let original = app.documents[0].snapshot();
        let history = app.documents[0].history_counts();
        let centers: Vec<_> = app
            .tabs
            .iter()
            .map(|t| {
                t.view
                    .screen_to_document([0.0; 2], [400.0, 300.0], [0.0; 2], 1.0)
            })
            .collect();
        let mut physical_widths = Vec::new();
        for native in [1.0, 1.5, 2.0, 1.0] {
            dpi_frame(&ctx, &mut app, native, vec![]);
            let ppp = dpi_frame(&ctx, &mut app, native, vec![]);
            assert_eq!(ppp, native);
            let viewport = app.canvas_view.viewport.unwrap();
            assert!(viewport.max.x <= 1100.0 && viewport.max.y <= 760.0);
            let pos = viewport.center();
            dpi_frame(&ctx, &mut app, native, vec![egui::Event::PointerMoved(pos)]);
            let cursor = app.canvas_view.cursor_rect.unwrap();
            assert_eq!(cursor.width() * ppp, 14.0 * native);
            assert_eq!(cursor.center(), pos);
            physical_widths.push(app.new_button.unwrap().width() * ppp);
            for (tab, before) in app.tabs.iter().zip(&centers) {
                let center =
                    tab.view
                        .screen_to_document([0.0; 2], [400.0, 300.0], [0.0; 2], ppp as f64);
                assert!(
                    (center[0] - before[0]).abs() < 1e-6 && (center[1] - before[1]).abs() < 1e-6
                );
                let screen =
                    tab.view
                        .document_to_screen(center, [400.0, 300.0], [0.0; 2], ppp as f64);
                assert!(screen[0].abs() < 1e-6 && screen[1].abs() < 1e-6);
            }
            assert_eq!(app.documents[0].snapshot(), original);
            assert_eq!(app.documents[0].history_counts(), history);
            assert!(!app.documents[0].dirty);
        }
        assert!((physical_widths[1] / physical_widths[0] - 1.5).abs() < 0.05);
        assert!((physical_widths[2] / physical_widths[0] - 2.0).abs() < 0.05);
        app.interface_zoom(&ctx, 1.5).unwrap();
        dpi_frame(&ctx, &mut app, 2.0, vec![]);
        assert_eq!(dpi_frame(&ctx, &mut app, 2.0, vec![]), 3.0);
        app.interface_history(&ctx, false).unwrap();
        dpi_frame(&ctx, &mut app, 2.0, vec![]);
        assert_eq!(dpi_frame(&ctx, &mut app, 2.0, vec![]), 2.0);
        app.interface_history(&ctx, true).unwrap();
        dpi_frame(&ctx, &mut app, 2.0, vec![]);
        dpi_frame(&ctx, &mut app, 2.0, vec![]);
        let settings = drawer_core::ui_scale::Settings::load(&storage).unwrap();
        assert_eq!(settings.zoom, 1.5);
        let new_ctx = egui::Context::default();
        let mut restarted = Drawer::new(&new_ctx, None)
            .with_storage(UserStorage::at(storage.root().to_path_buf()).unwrap());
        restarted
            .create_canvas(CanvasSpec {
                width: 400,
                height: 300,
                ..tiny_spec()
            })
            .unwrap();
        dpi_frame(&new_ctx, &mut restarted, 1.0, vec![]);
        dpi_frame(&new_ctx, &mut restarted, 1.0, vec![]);
        let center =
            restarted.documents[0]
                .view
                .screen_to_document([0.0; 2], [400.0, 300.0], [0.0; 2], 1.5);
        assert!(
            (center[0] - centers[1][0]).abs() < 1e-6 && (center[1] - centers[1][1]).abs() < 1e-6
        );
        let normal = std::fs::read(storage.root().join("settings/interface.json")).unwrap();
        std::fs::write(
            storage.root().join("settings/interface.json.write.tmp"),
            "foreign",
        )
        .unwrap();
        assert!(app.interface_zoom(&ctx, 1.0).is_err());
        assert_eq!(
            std::fs::read(storage.root().join("settings/interface.json")).unwrap(),
            normal
        );
        assert_eq!(app.interface.settings.zoom, 1.5);
        std::fs::remove_file(storage.root().join("settings/interface.json.write.tmp")).unwrap();
        std::fs::write(storage.root().join("settings/interface.json"), "bad").unwrap();
        assert!(drawer_core::ui_scale::Settings::load(&storage).is_err());
        drawer_core::ui_scale::Settings::default()
            .save(&storage)
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(storage.root().join("settings/interface.damaged.json"))
                .unwrap(),
            "bad"
        );
    }
    #[test]
    fn brush_outline_tools_and_first_mouse_point_match_at_minimum_large_and_rotated_sizes() {
        let ctx = egui::Context::default();
        let mut app = Drawer::new(&ctx, None);
        app.create_canvas(CanvasSpec {
            width: 400,
            height: 300,
            ..tiny_spec()
        })
        .unwrap();
        app.documents[0].view.angle = 37.0;
        app.documents[0].view.flipped = true;
        app.documents[0].dirty = false;
        // Isolate cursor/input behavior from painting, which is verified separately.
        app.documents[0].scene.selection_mask = Some(drawer_core::selection::Mask::default());
        let before = app.documents[0].snapshot();
        let history = app.documents[0].history_counts();
        for size in [1.0, 4096.0] {
            for native in [1.0, 1.5, 2.0] {
                for tool in BrushTip::ALL {
                    app.set_tool(tool);
                    app.panels.settings.brush_size = size;
                    app.panels.settings.brush_aspect = 0.3;
                    app.panels.settings.brush_angle = 29.0;
                    dpi_frame(&ctx, &mut app, native, vec![]);
                    let viewport = app.canvas_view.viewport.unwrap();
                    let pos = viewport.center();
                    dpi_frame(&ctx, &mut app, native, vec![egui::Event::PointerMoved(pos)]);
                    dpi_frame(
                        &ctx,
                        &mut app,
                        native,
                        vec![egui::Event::PointerButton {
                            pos,
                            button: egui::PointerButton::Primary,
                            pressed: true,
                            modifiers: Default::default(),
                        }],
                    );
                    let (id, point) = app.canvas_view.first_point.unwrap();
                    assert_eq!(id, app.documents[0].id);
                    let mapped = app.documents[0].view.document_to_screen(
                        point,
                        [400.0, 300.0],
                        [viewport.center().x as f64, viewport.center().y as f64],
                        native as f64,
                    );
                    assert!(
                        (mapped[0] - pos.x as f64).abs() < 1e-6
                            && (mapped[1] - pos.y as f64).abs() < 1e-6
                    );
                    let outline = &app.canvas_view.brush_outline;
                    assert_eq!(outline.len(), 64);
                    for n in 0..32 {
                        let midpoint = outline[n] + (outline[n + 32] - outline[n]) * 0.5;
                        assert!(
                            (midpoint.x - pos.x).abs() < 0.02 && (midpoint.y - pos.y).abs() < 0.02
                        );
                    }
                    dpi_frame(
                        &ctx,
                        &mut app,
                        native,
                        vec![egui::Event::PointerButton {
                            pos,
                            button: egui::PointerButton::Primary,
                            pressed: false,
                            modifiers: Default::default(),
                        }],
                    );
                    assert_eq!(app.documents[0].snapshot(), before);
                    assert_eq!(app.documents[0].history_counts(), history);
                    assert!(!app.documents[0].dirty);
                }
            }
        }
        let icons: Vec<_> = BrushTip::ALL
            .into_iter()
            .map(|tip| drawer_core::assets::tool_icon(tip, 16))
            .collect();
        for n in 0..4 {
            for other in n + 1..4 {
                assert_ne!(icons[n], icons[other]);
            }
        }
    }
}
