#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use drawer_core::assets::{APP_NAME, BrushTip, application_icon};
use drawer_core::document::{
    Background, CanvasSpec, CreateError, DEFAULT_DOCUMENT_BUDGET, Document,
};
use drawer_core::storage::UserStorage;
use eframe::egui;
use std::{path::PathBuf, sync::Arc};
mod canvas_view;
mod new_canvas;
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
                self.pending_close.is_none() && self.sizing_dialog.is_none(),
                |ui| {
                    ui.horizontal(|ui| {
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
                            let changed = self.canvas_view.show(
                                &mut columns[1],
                                active,
                                &mut self.documents[active],
                                overlay,
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
    }
}

impl eframe::App for Drawer {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        if self.smoke_output.is_none()
            && !self.quit_ready
            && ui.ctx().input(|input| input.viewport().close_requested())
            && (self.documents.iter().any(|doc| doc.dirty) || self.sizing_dialog.is_some())
        {
            ui.ctx()
                .send_viewport_cmd(egui::ViewportCommand::CancelClose);
            if self.sizing_dialog.is_none() {
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
                let report = format!(
                    "app={APP_NAME}\nframes={}\ntools=Pen,Pencil,Eraser,Wash\nembedded_tips={}\ncanvases={}\nviews={}\nclose_cancel={}\nsizing_dpi=true\nsizing_undo=true\n",
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
}
