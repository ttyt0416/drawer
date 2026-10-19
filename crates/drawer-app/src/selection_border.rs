use super::*;
use drawer_core::selection_border::{self as core, Alignment, Settings};
#[derive(Default)]
pub(super) struct Panel {
    pub draft: Option<Draft>,
    apply: Option<egui::Rect>,
}
pub(super) struct Draft {
    id: u64,
    revision: u64,
    frame: u32,
    layer: usize,
    settings: Settings,
}
impl Drawer {
    pub(super) fn begin_selection_border(&mut self) -> Result<(), String> {
        let d = &self.documents[self.active.ok_or("Open a page first")?];
        let drawer_core::thumbnail::Target::Raster(layer) = self
            .panels
            .targets
            .get(&d.id)
            .copied()
            .unwrap_or(drawer_core::thumbnail::Target::Raster(0))
        else {
            return Err("Choose a raster target for the selection border".into());
        };
        self.selection_border.draft = Some(Draft {
            id: d.id,
            revision: d.revision,
            frame: d.current_frame,
            layer,
            settings: Settings {
                color: self.references.session.workspace.color,
                ..Default::default()
            },
        });
        Ok(())
    }
    fn apply_selection_border(&mut self) -> Result<(), String> {
        let e = self
            .selection_border
            .draft
            .as_ref()
            .ok_or("Selection border is closed")?;
        let d = &mut self.documents[self.active.ok_or("Open a page first")?];
        if d.id != e.id || d.revision != e.revision || d.current_frame != e.frame {
            return Err("Page/frame changed; reopen the selection border".into());
        }
        let state = core::prepare(d, e.layer, e.settings)?;
        d.commit_state(state)?;
        self.selection_border.draft = None;
        Ok(())
    }
    pub(super) fn selection_border_window(&mut self, ctx: &egui::Context) {
        let Some(e) = &mut self.selection_border.draft else {
            return;
        };
        let mut open = !ctx.input(|i| i.key_pressed(egui::Key::Escape));
        let mut apply = false;
        let mut cancel = false;
        self.selection_border.apply = None;
        egui::Window::new("Draw selection border").open(&mut open).collapsible(false).show(ctx,|ui| {
            egui::ComboBox::from_id_salt("selection-border-align").selected_text(format!("{:?}",e.settings.alignment)).show_ui(ui,|ui|{for a in Alignment::ALL{ui.selectable_value(&mut e.settings.alignment,a,format!("{a:?}"));}});
            ui.add(egui::Slider::new(&mut e.settings.width,0.0..=128.0).text("Width in document pixels"));super::edit::rgba(ui,&mut e.settings.color);
            ui.label("Draws the nonzero selection boundary, including holes. The original selection stays unchanged. Outside strokes can paint beyond the selection; the result is clipped to the canvas.");
            ui.horizontal(|ui|{let r=ui.button("Draw border");self.selection_border.apply=Some(r.rect);apply=r.clicked();cancel=ui.button("Cancel").clicked();});
        });
        if !open || cancel {
            self.selection_border.draft = None;
        }
        if apply && let Err(e) = self.apply_selection_border() {
            self.status = Some(e);
        }
    }
}
pub(super) fn smoke(root: &std::path::Path) -> Result<(), String> {
    core::verify(root)?;
    let ctx = egui::Context::default();
    let mut app = Drawer::new(&ctx, None);
    app.simple_view = true;
    let mut time = 0.0;
    app.create_canvas(CanvasSpec {
        width: 10,
        height: 10,
        dpi: 300.0,
        background: Background::Transparent,
    })
    .map_err(|e| e.to_string())?;
    app.documents[0].scene.selection = Some(drawer_core::scene::Rect {
        min: [2.0, 2.0],
        max: [8.0, 8.0],
    });
    app.documents[0].view.angle = 90.0;
    app.documents[0].view.zoom = 1.5;
    app.choose_color([200, 40, 20, 128]);
    let step = |app: &mut Drawer, time: &mut f64, events| {
        *time += 0.1;
        super::input_replay::frame(&ctx, app, *time, events);
    };
    step(&mut app, &mut time, vec![]);
    step(&mut app, &mut time, vec![]);
    for alignment in Alignment::ALL {
        let before = app.documents[0].snapshot();
        let history = app.documents[0].history_counts();
        app.execute_command(&ctx, "selection-border");
        app.selection_border
            .draft
            .as_mut()
            .ok_or("Selection border command missing")?
            .settings
            .alignment = alignment;
        let e = app.selection_border.draft.as_ref().unwrap();
        let expected = core::prepare(&app.documents[0], e.layer, e.settings)?;
        step(&mut app, &mut time, vec![]);
        step(&mut app, &mut time, vec![]);
        if app.documents[0].snapshot() != before {
            return Err("Border draft changed page".into());
        }
        let rect = app
            .selection_border
            .apply
            .ok_or("Draw border button missing")?;
        for pressed in [true, false] {
            step(
                &mut app,
                &mut time,
                vec![
                    egui::Event::PointerMoved(rect.center()),
                    egui::Event::PointerButton {
                        pos: rect.center(),
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: Default::default(),
                    },
                ],
            );
        }
        if app.selection_border.draft.is_some()
            || app.documents[0].snapshot() != expected
            || app.documents[0].scene != before.scene
            || app.documents[0].history_counts().0 != history.0 + 1
        {
            return Err(format!(
                "Draw border Apply {alignment:?}: {:?}, draft={}, same={}, scene={}, history={:?}/{history:?}, layer0={:?}/{:?}",
                app.status,
                app.selection_border.draft.is_some(),
                app.documents[0].snapshot() == expected,
                app.documents[0].scene == before.scene,
                app.documents[0].history_counts(),
                app.documents[0].layers[0].pixel(10, 10, 2, 4),
                expected.layers[0].pixel(10, 10, 2, 4)
            ));
        }
        if !app.documents[0].undo() || app.documents[0].snapshot() != before {
            return Err("Border production Undo".into());
        }
        app.execute_command(&ctx, "selection-border");
        step(
            &mut app,
            &mut time,
            vec![egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Default::default(),
            }],
        );
        step(
            &mut app,
            &mut time,
            vec![egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed: false,
                repeat: false,
                modifiers: Default::default(),
            }],
        );
        if app.selection_border.draft.is_some() || app.documents[0].snapshot() != before {
            return Err("Border cancel changed page".into());
        }
    }
    app.execute_command(&ctx, "selection-border");
    app.documents[0].edit_pixel(0, 0, 0, [1, 2, 3, 255]);
    let before = app.documents[0].snapshot();
    if app.apply_selection_border().is_ok() || app.documents[0].snapshot() != before {
        return Err("Stale border applied".into());
    }
    app.selection_border.draft = None;
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn production_border_alignment_apply_selection_cancel_stale() {
        let root = crate::tests::TestDirectory::new();
        super::smoke(&root.0).unwrap();
    }
}
