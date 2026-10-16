use super::*;
use drawer_core::related_colors::{Grid, Settings};
#[derive(Default)]
pub(super) struct Related {
    first: [Option<(egui::Id, [u8; 4], egui::Rect)>; 2],
}
fn layout(ui: &mut egui::Ui, value: &mut Grid) {
    ui.horizontal_wrapped(|ui| {
        super::edit::number(
            ui,
            &mut value.rows,
            1.0..=16.0,
            1.0,
            " rows",
            "related-colors",
        );
        super::edit::number(
            ui,
            &mut value.columns,
            1.0..=16.0,
            1.0,
            " columns",
            "related-colors",
        );
        super::edit::number(
            ui,
            &mut value.tile,
            12.0..=64.0,
            1.0,
            " tile px",
            "related-colors",
        );
        ui.checkbox(&mut value.lines, "Grid lines");
    });
}
fn samples(
    ui: &mut egui::Ui,
    id: usize,
    layout: Grid,
    colors: &[[u8; 4]],
    first: &mut Option<(egui::Id, [u8; 4], egui::Rect)>,
) -> Option<[u8; 4]> {
    let mut selected = None;
    egui::ScrollArea::both()
        .id_salt(("related-scroll", id))
        .max_height(300.0)
        .show(ui, |ui| {
            egui::Grid::new(("related-grid", id))
                .spacing(egui::vec2(0.0, 0.0))
                .show(ui, |ui| {
                    for (n, c) in colors.iter().enumerate() {
                        let response = ui
                            .add_sized(
                                [layout.tile as f32; 2],
                                egui::Button::new(" ").frame(layout.lines).fill(
                                    egui::Color32::from_rgba_unmultiplied(c[0], c[1], c[2], c[3]),
                                ),
                            )
                            .on_hover_text(drawer_core::color_selector::hex(*c));
                        if n == 0 {
                            *first = Some((response.id, *c, response.rect));
                        }
                        if response.clicked() {
                            selected = Some(*c)
                        }
                        if (n + 1) % layout.columns as usize == 0 {
                            ui.end_row();
                        }
                    }
                });
        });
    selected
}
impl Drawer {
    pub(super) fn related_colors_window(&mut self, ctx: &egui::Context) {
        if !self.workspaces.store.current.related_colors.visible {
            return;
        }
        let mut s = self.workspaces.store.current.related_colors;
        let mut visible = true;
        let mut selected = None;
        let current = self.references.session.workspace.color;
        let hue = self.workspaces.store.current.color_picker.hue;
        egui::Window::new("Intermediate and similar colors").open(&mut visible).max_width((ctx.content_rect().width()-40.0).max(80.0)).max_height((ctx.content_rect().height()-40.0).max(80.0)).vscroll(true).hscroll(true).show(ctx,|ui|{
        ui.horizontal_wrapped(|ui|{if ui.button("Use current as first color").clicked(){s.endpoints[0]=current;}
            if ui.button("Use current as second color").clicked(){s.endpoints[1]=current;}
            if ui.button("Swap endpoint colors").clicked(){s.endpoints.swap(0,1);}});
        ui.label(format!("{} → {}",drawer_core::color_selector::hex(s.endpoints[0]),drawer_core::color_selector::hex(s.endpoints[1])));
        ui.heading("Intermediate colors");ui.push_id("middle",|ui|layout(ui,&mut s.middle));ui.label("Row-major samples span the two endpoints. Alpha-weighted RGB; transparent endpoints keep stored RGB.");
        if let Ok(colors)=s.middle_colors()&&let Some(c)=samples(ui,0,s.middle,&colors,&mut self.related.first[0]){selected=Some(c)}
        ui.heading("Similar colors");ui.push_id("similar",|ui|layout(ui,&mut s.similar));super::edit::slider(ui,&mut s.span[0],0.0..=1.0,"Saturation neighborhood ±","related-colors");super::edit::slider(ui,&mut s.span[1],0.0..=1.0,"Value neighborhood ±","related-colors");
        ui.label("Columns vary saturation; rows vary value. Hue and alpha stay fixed. Tab then Enter/Space selects a tile.");
        if let Ok(colors)=s.similar_colors(current,hue)&&let Some(c)=samples(ui,1,s.similar,&colors,&mut self.related.first[1]){selected=Some(c)}
    });
        s.visible = visible;
        if s.valid() {
            self.workspaces.store.current.related_colors = s;
        }
        if let Some(c) = selected {
            self.choose_color(c)
        }
    }
}
fn frame(
    ctx: &egui::Context,
    app: &mut Drawer,
    time: f64,
    size: [f32; 2],
    events: Vec<egui::Event>,
) {
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(size[0], size[1]),
            )),
            time: Some(time),
            events,
            ..Default::default()
        },
        |_| app.related_colors_window(ctx),
    );
    output.textures_delta.clear();
}
pub(super) fn smoke(root: &std::path::Path) -> Result<(), String> {
    drawer_core::related_colors::verify()?;
    let ctx = egui::Context::default();
    let user = root.join("related-colors-user");
    let mut app = Drawer::new(&ctx, None)
        .with_storage(UserStorage::at(user.clone()).map_err(|e| e.to_string())?);
    app.create_canvas(CanvasSpec {
        width: 2,
        height: 2,
        dpi: 300.0,
        background: Background::Transparent,
    })
    .map_err(|e| e.to_string())?;
    drawer_core::files::save(
        &mut app.documents[0],
        &root.join("related-keeps-document.drawer"),
    )
    .map_err(|e| e.to_string())?;
    let before = app.documents[0].snapshot();
    let mut s = Settings {
        visible: true,
        ..Default::default()
    };
    s.endpoints = [[12, 34, 56, 0], [220, 30, 60, 128]];
    app.workspaces.store.current.related_colors = s;
    frame(&ctx, &mut app, 0.0, [800.0, 900.0], vec![]);
    frame(&ctx, &mut app, 0.1, [800.0, 900.0], vec![]);
    let (id, color, _) = app.related.first[0].ok_or("Intermediate tile missing")?;
    ctx.memory_mut(|m| m.request_focus(id));
    frame(
        &ctx,
        &mut app,
        0.2,
        [800.0, 900.0],
        vec![egui::Event::Key {
            key: egui::Key::Enter,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Default::default(),
        }],
    );
    if app.references.session.workspace.color != color {
        return Err("Keyboard intermediate-color selection failed".into());
    }
    app.workspaces.store.current.related_colors.middle = Grid {
        rows: 3,
        columns: 3,
        tile: 48,
        lines: false,
    };
    app.workspaces.store.current.related_colors.similar = Grid {
        rows: 9,
        columns: 9,
        tile: 12,
        lines: true,
    };
    frame(&ctx, &mut app, 0.3, [120.0, 140.0], vec![]);
    let expected = app.capture_workspace().related_colors;
    app.persist_workspace()?;
    let restarted = Drawer::new(&egui::Context::default(), None)
        .with_storage(UserStorage::at(user).map_err(|e| e.to_string())?);
    if restarted.capture_workspace().related_colors != expected {
        return Err("Related-color grid settings restart failed".into());
    }
    if app.documents[0].snapshot() != before
        || app.documents[0].dirty
        || app.documents[0].history_counts() != (0, 0)
    {
        return Err("Related-color panels edited the document".into());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn keyboard_small_panel_and_local_restart() {
        let root = crate::tests::TestDirectory::new();
        super::smoke(&root.0).unwrap();
    }
}
