#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use drawer_core::assets::{APP_NAME, BrushTip, application_icon};
use drawer_core::document::{
    Background, CanvasSpec, CreateError, DEFAULT_DOCUMENT_BUDGET, Document,
};
use drawer_core::storage::UserStorage;
use eframe::egui;
use std::{path::PathBuf, sync::Arc};
mod aid_panel;
mod canvas_view;
mod display_pane;
mod image_import_dialog;
mod new_canvas;
mod reference_pane;
mod sizing_dialog;
mod workspace;
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
    #[cfg(test)]
    new_button: Option<egui::Rect>,
    #[cfg(test)]
    create_button: Option<egui::Rect>,
    #[cfg(test)]
    cancel_button: Option<egui::Rect>,
}

impl Drawer {
    fn new(ctx: &egui::Context, smoke_output: Option<PathBuf>) -> Self {
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
                egui::Grid::new("canvas-fields").show(ui, |ui| {
                    ui.label("Width (pixels)");
                    ui.text_edit_singleline(&mut self.new_canvas.width);
                    ui.end_row();
                    ui.label("Height (pixels)");
                    ui.text_edit_singleline(&mut self.new_canvas.height);
                    ui.end_row();
                    ui.label("DPI");
                    ui.text_edit_singleline(&mut self.new_canvas.dpi);
                    ui.end_row();
                });
                ui.checkbox(&mut self.new_canvas.transparent, "Transparent background");
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
                    let create_response = ui.button("Create");
                    let cancel_response = ui.button("Cancel");
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
        if let Some(active) = self.active {
            self.documents[active].tool = self.selected;
        }
        self.storage = Some(storage);
        self
    }

    fn show(&mut self, ui: &mut egui::Ui) {
        egui::CentralPanel::default().show(ui, |ui| {
            ui.add_enabled_ui(
                self.pending_close.is_none()
                    && self.sizing_dialog.is_none()
                    && self.import_dialog.is_none(),
                |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.heading(APP_NAME);
                        ui.label("Local drawing workspace");
                        let new_response = ui.button("New canvas");
                        #[cfg(test)]
                        {
                            self.new_button = Some(new_response.rect);
                        }
                        if new_response.clicked() {
                            self.new_canvas = NewCanvas::default();
                            self.show_new_canvas = true;
                        }
                        if ui.button("References").clicked() {
                            self.references.session.workspace.visible = true;
                        }
                        if ui.button("Canvas aids").clicked() {
                            self.aids.visible = true;
                        }
                        if ui.button("Display view").clicked() {
                            self.display.settings_open = true;
                        }
                    });
                    self.document_controls(ui);
                    self.sizing_controls(ui);
                    ui.separator();
                    if let Some(status) = &self.status {
                        ui.colored_label(egui::Color32::LIGHT_RED, status);
                    }
                    ui.columns(2, |columns| {
                        columns[0].set_width(170.0);
                        columns[0].heading("Tools");
                        let mut color = self.references.session.workspace.color;
                        if columns[0]
                            .color_edit_button_srgba_unmultiplied(&mut color)
                            .changed()
                        {
                            self.references.session.workspace.color = color;
                        }
                        for tip in BrushTip::ALL {
                            let response =
                                columns[0].selectable_label(self.selected == tip, tip.name());
                            if response.clicked() {
                                self.set_tool(tip);
                            }
                        }
                        columns[0].label("Brush preview");
                        let index = BrushTip::ALL
                            .iter()
                            .position(|&tip| tip == self.selected)
                            .unwrap();
                        columns[0].add(
                            egui::Image::new(&self.previews[index])
                                .fit_to_exact_size(egui::vec2(160.0, 160.0)),
                        );
                        if let Some(active) = self.active {
                            let document = &self.documents[active];
                            columns[1].label(format!(
                                "{} · {} × {} · {} DPI",
                                document.title, document.width, document.height, document.dpi
                            ));
                            let overlay = self.sizing_overlay();
                            let (left, right) = columns.split_at_mut(1);
                            let changed = self.canvas_view.show(
                                &mut right[0],
                                active,
                                &mut self.documents[active],
                                overlay,
                                &mut left[0],
                                self.aids.preferences,
                            );
                            if changed
                                && let Some(storage) = &self.storage
                                && let Err(error) = storage.save_view(self.documents[active].view)
                            {
                                self.status = Some(error.to_string());
                            }
                        } else {
                            columns[1].heading("Create a canvas to begin");
                        }
                    });
                },
            );
        });
        self.new_canvas_dialog(ui.ctx());
        self.close_dialog(ui.ctx());
        self.show_sizing(ui.ctx());
        self.show_references(ui.ctx());
        self.show_aids(ui.ctx());
        self.show_import(ui.ctx());
        self.show_display(ui.ctx());
    }
}

impl eframe::App for Drawer {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        self.display.poll_monitors(ui.ctx(), frame);
        if self.smoke_output.is_none()
            && !self.quit_ready
            && ui.ctx().input(|input| input.viewport().close_requested())
            && (self.documents.iter().any(|doc| doc.dirty)
                || self.sizing_dialog.is_some()
                || self.import_dialog.is_some())
        {
            ui.ctx()
                .send_viewport_cmd(egui::ViewportCommand::CancelClose);
            if self.sizing_dialog.is_none() && self.import_dialog.is_none() {
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
                self.display.preferences.visible = true;
                self.display.preferences.fullscreen = false;
                self.display
                    .preferences
                    .save(self.storage.as_ref().unwrap())
                    .unwrap();
            }
            if self.frames == 3 {
                self.cancel_close();
                self.select_tab(0);
                self.begin_sizing(sizing_dialog::Mode::Dpi);
                self.sizing_dialog.as_mut().unwrap().dpi = "600".into();
            }
            if self.frames == 4 {
                self.smoke_start_sizing(ui.ctx());
            }
            if self.frames > 120 {
                std::process::exit(1);
            }
            if self.frames >= 6 && self.sizing_dialog.is_none() {
                let validated = self.documents.len() == 2
                    && self.tabs.len() == 2
                    && self.pending_close.is_none()
                    && self.documents[0].dpi == 600.0;
                if !validated {
                    std::process::exit(1);
                }
                if !self.documents[0].undo() || self.documents[0].dpi != 300.0 {
                    std::process::exit(1);
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
                    std::process::exit(1);
                }
                self.open_reference_document().unwrap();
                if self.documents[2].layers[0].pixels != [[240, 30, 80, 128], [20, 40, 60, 0]]
                    || self.documents[2].path.is_some()
                    || self.documents[2].history_counts() != (0, 0)
                {
                    std::process::exit(1);
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
                    std::process::exit(1);
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
                    std::process::exit(1);
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
                    std::process::exit(1);
                }
                let recent =
                    drawer_core::recent::RecentFiles::load(self.storage.as_ref().unwrap()).unwrap();
                if recent.paths() != self.recent.paths() || recent.paths().len() != 2 {
                    std::process::exit(1);
                }
                if self.display.monitors.is_empty()
                    || !self.display.rendered_native
                    || self.display.rendered_frames < 2
                    || !self.display.native_windowed
                {
                    std::process::exit(1);
                }
                self.display.preferences.visible = false;
                self.display
                    .preferences
                    .save(self.storage.as_ref().unwrap())
                    .unwrap();
                let report = format!(
                    "app={APP_NAME}\nframes={}\ntools=Pen,Pencil,Eraser,Wash\nembedded_tips={}\ncanvases={}\nviews={}\nclose_cancel={}\nsizing_dpi=true\nsizing_undo=true\nreference_copy=true\nreference_document=true\ngrid_guide_undo=true\nprint_size_view=true\nclipboard_document=true\nsave_as_export=true\nrecent_restart=true\nimage_decode=true\nnative_display_view=true\nmonitor_inventory=true\n",
                    self.frames,
                    self.previews.len(),
                    self.documents.len(),
                    self.tabs.len(),
                    validated,
                );
                if let Err(error) = std::fs::write(path, report) {
                    eprintln!("Smoke receipt: {error}");
                    std::process::exit(1);
                }
                self.quit_ready = true;
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
            } else {
                ui.ctx().request_repaint();
            }
        }
    }
}

fn main() -> eframe::Result {
    let mut args = std::env::args().skip(1);
    let smoke_output = match args.next().as_deref() {
        Some("--smoke-test") => Some(PathBuf::from(args.next().expect("smoke receipt path"))),
        None => None,
        Some(other) => {
            eprintln!("Unknown argument: {other}");
            std::process::exit(2);
        }
    };
    let options = eframe::NativeOptions {
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
            Ok(Box::new(
                Drawer::new(&cc.egui_ctx, smoke_output)
                    .with_storage(UserStorage::from_environment()?),
            ))
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    struct TestDirectory(PathBuf);
    impl TestDirectory {
        fn new() -> Self {
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
    fn tiny_spec() -> CanvasSpec {
        CanvasSpec {
            width: 1,
            height: 1,
            dpi: 300.0,
            background: Background::Transparent,
        }
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

    fn frame(ctx: &egui::Context, app: &mut Drawer, events: Vec<egui::Event>) {
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
    fn click(ctx: &egui::Context, app: &mut Drawer, rect: egui::Rect) {
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
}
