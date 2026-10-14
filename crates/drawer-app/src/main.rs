#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use drawer_core::assets::{APP_NAME, BrushTip, application_icon};
use drawer_core::storage::UserStorage;
use eframe::egui;
use std::{path::PathBuf, sync::Arc};

struct Drawer {
    selected: BrushTip,
    previews: Vec<egui::TextureHandle>,
    smoke_output: Option<PathBuf>,
    frames: usize,
    storage: Option<UserStorage>,
    status: Option<String>,
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
        Self {
            selected: BrushTip::Pen,
            previews,
            smoke_output,
            frames: 0,
            storage: None,
            status: None,
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
        self.storage = Some(storage);
        self
    }

    fn show(&mut self, ui: &mut egui::Ui) {
        egui::CentralPanel::default().show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.heading(APP_NAME);
                ui.label("Local drawing workspace");
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
                columns[1].heading("Brush preview");
                let index = BrushTip::ALL
                    .iter()
                    .position(|&tip| tip == self.selected)
                    .unwrap();
                columns[1].add(
                    egui::Image::new(&self.previews[index])
                        .fit_to_exact_size(egui::vec2(160.0, 160.0)),
                );
            });
        });
    }
}

impl eframe::App for Drawer {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.show(ui);
        if let Some(path) = &self.smoke_output {
            self.frames += 1;
            if self.frames >= 4 {
                let report = format!(
                    "app={APP_NAME}\nframes={}\ntools=Pen,Pencil,Eraser,Wash\nembedded_tips={}\n",
                    self.frames,
                    self.previews.len()
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
}
