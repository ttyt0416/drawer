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
use new_canvas::NewCanvas;

struct Drawer {
    selected: BrushTip,
    previews: Vec<egui::TextureHandle>,
    smoke_output: Option<PathBuf>,
    frames: usize,
    storage: Option<UserStorage>,
    status: Option<String>,
    documents: Vec<Document>,
    active: Option<usize>,
    new_canvas: NewCanvas,
    show_new_canvas: bool,
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
            active: None,
            new_canvas: NewCanvas::default(),
            show_new_canvas: false,
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
        let used: usize = self.documents.iter().map(Document::memory_bytes).sum();
        let mut document = Document::create(spec, DEFAULT_DOCUMENT_BUDGET.saturating_sub(used))?;
        document.view = self.default_view;
        self.documents
            .try_reserve(1)
            .map_err(|_| CreateError::Allocation)?;
        self.documents.push(document);
        self.active = Some(self.documents.len() - 1);
        Ok(())
    }

    fn new_canvas_dialog(&mut self, ctx: &egui::Context) {
        if !self.show_new_canvas {
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
        self.storage = Some(storage);
        self
    }

    fn show(&mut self, ui: &mut egui::Ui) {
        egui::CentralPanel::default().show(ui, |ui| {
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
            ui.separator();
            if let Some(status) = &self.status {
                ui.colored_label(egui::Color32::LIGHT_RED, status);
            }
            ui.columns(2, |columns| {
                columns[0].set_width(170.0);
                columns[0].heading("Tools");
                for tip in BrushTip::ALL {
                    let response = columns[0].selectable_label(self.selected == tip, tip.name());
                    if response.clicked() {
                        self.selected = tip;
                        if let Some(storage) = &self.storage
                            && let Err(error) = storage.save_brush(tip)
                        {
                            self.status = Some(error.to_string());
                        }
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
                        "Untitled {} · {} × {} · {} DPI · Unsaved",
                        active + 1,
                        document.width,
                        document.height,
                        document.dpi
                    ));
                    let changed =
                        self.canvas_view
                            .show(&mut columns[1], active, &mut self.documents[active]);
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
        });
        self.new_canvas_dialog(ui.ctx());
    }
}

impl eframe::App for Drawer {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.show(ui);
        if let Some(path) = &self.smoke_output {
            self.frames += 1;
            if self.frames >= 4 {
                let report = format!(
                    "app={APP_NAME}\nframes={}\ntools=Pen,Pencil,Eraser,Wash\nembedded_tips={}\ncanvases={}\n",
                    self.frames,
                    self.previews.len(),
                    self.documents.len(),
                );
                if let Err(error) = std::fs::write(path, report) {
                    eprintln!("Smoke receipt: {error}");
                    std::process::exit(1);
                }
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
}
