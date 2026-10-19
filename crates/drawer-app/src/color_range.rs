use super::*;
use drawer_core::color_range::{self as core, Reference, Settings};
#[derive(Default)]
pub(super) struct Panel {
    pub draft: Option<Draft>,
    apply: Option<egui::Rect>,
}
pub(super) struct Draft {
    id: u64,
    revision: u64,
    frame: u32,
    layer: Option<usize>,
    settings: Settings,
}
impl Drawer {
    pub(super) fn begin_color_range(&mut self) -> Result<(), String> {
        let d = &self.documents[self.active.ok_or("Open a page first")?];
        let layer = match self
            .panels
            .targets
            .get(&d.id)
            .copied()
            .unwrap_or(drawer_core::thumbnail::Target::Raster(0))
        {
            drawer_core::thumbnail::Target::Raster(n) => Some(n),
            _ => None,
        };
        self.color_range.draft = Some(Draft {
            id: d.id,
            revision: d.revision,
            frame: d.current_frame,
            layer,
            settings: self.workspaces.store.current.color_range,
        });
        Ok(())
    }
    fn apply_color_range(&mut self) -> Result<(), String> {
        let e = self
            .color_range
            .draft
            .as_ref()
            .ok_or("Color range is closed")?;
        let d = &mut self.documents[self.active.ok_or("Open a page first")?];
        if d.id != e.id || d.revision != e.revision || d.current_frame != e.frame {
            return Err("Page/frame changed; reopen Color range".into());
        }
        let scene = core::prepare(d, e.layer, e.settings)?;
        d.edit_scene(scene)?;
        self.workspaces.store.current.color_range = e.settings;
        self.color_range.draft = None;
        Ok(())
    }
    pub(super) fn color_range_window(&mut self, ctx: &egui::Context) {
        let Some(e) = &mut self.color_range.draft else {
            return;
        };
        let mut open = !ctx.input(|i| i.key_pressed(egui::Key::Escape));
        let mut apply = false;
        let mut cancel = false;
        self.color_range.apply = None;
        egui::Window::new("Color range selection").open(&mut open).collapsible(false).show(ctx,|ui| {
            ui.label("Sample a color with the eyedropper, then use Current drawing color here. RGB distance uses the largest channel difference; source alpha above zero does not change the match.");
            super::edit::rgba(ui,&mut e.settings.color);
            if ui.button("Use current drawing color").clicked(){e.settings.color=self.references.session.workspace.color;}
            ui.add(egui::Slider::new(&mut e.settings.tolerance,0..=255).text("RGB tolerance"));ui.checkbox(&mut e.settings.include_transparent,"Include fully transparent pixels (any hidden RGB)");
            egui::ComboBox::from_id_salt("color-range-reference").selected_text(format!("{:?}",e.settings.reference)).show_ui(ui,|ui|{for r in [Reference::Current,Reference::Composite]{ui.selectable_value(&mut e.settings.reference,r,format!("{r:?}"));}});
            egui::ComboBox::from_id_salt("color-range-combine").selected_text(format!("{:?}",e.settings.combine)).show_ui(ui,|ui|{for op in drawer_core::selection_edit::Combine::ALL{ui.selectable_value(&mut e.settings.combine,op,format!("{op:?}"));}});
            ui.horizontal(|ui|{let r=ui.button("Apply color range");self.color_range.apply=Some(r.rect);apply=r.clicked();cancel=ui.button("Cancel").clicked();});
        });
        if !open || cancel {
            self.color_range.draft = None;
        }
        if apply && let Err(e) = self.apply_color_range() {
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
        width: 8,
        height: 8,
        dpi: 300.0,
        background: Background::Transparent,
    })
    .map_err(|e| e.to_string())?;
    for y in 0..8 {
        for x in [1, 5] {
            app.documents[0].layers[0].pixels[y * 8 + x] = [10, 20, 30, 255];
        }
    }
    let step = |app: &mut Drawer, time: &mut f64, events| {
        *time += 0.1;
        super::input_replay::frame(&ctx, app, *time, events);
    };
    step(&mut app, &mut time, vec![]);
    step(&mut app, &mut time, vec![]);
    for reference in [Reference::Current, Reference::Composite] {
        let before = app.documents[0].snapshot();
        let history = app.documents[0].history_counts();
        app.execute_command(&ctx, "color-range");
        app.color_range
            .draft
            .as_mut()
            .ok_or("Color range command missing")?
            .settings = Settings {
            color: [10, 20, 30, 128],
            reference,
            ..Default::default()
        };
        step(&mut app, &mut time, vec![]);
        step(&mut app, &mut time, vec![]);
        if app.documents[0].snapshot() != before {
            return Err("Color range draft changed page".into());
        }
        let rect = app
            .color_range
            .apply
            .ok_or("Color range Apply button missing")?;
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
        if app.color_range.draft.is_some()
            || app.documents[0].scene.selection_coverage([1.5, 4.5]) != 255
            || app.documents[0].scene.selection_coverage([5.5, 4.5]) != 255
            || app.documents[0].scene.selection_coverage([3.5, 4.5]) != 0
            || app.documents[0].history_counts().0 != history.0 + 1
            || app.documents[0].snapshot().layers != before.layers
        {
            return Err(format!("Color range Apply {reference:?}: {:?}", app.status));
        }
        app.documents[0].undo();
        if app.documents[0].snapshot() != before {
            return Err("Color range production Undo".into());
        }
        app.execute_command(&ctx, "color-range");
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
        if app.color_range.draft.is_some() || app.documents[0].snapshot() != before {
            return Err("Color range cancel changed page".into());
        }
    }
    app.execute_command(&ctx, "color-range");
    app.documents[0].edit_pixel(0, 0, 0, [1, 2, 3, 255]);
    let before = app.documents[0].snapshot();
    if app.apply_color_range().is_ok() || app.documents[0].snapshot() != before {
        return Err("Stale color range applied".into());
    }
    app.color_range.draft = None;
    let path = root.join("color-range-settings.json");
    app.workspaces
        .store
        .current
        .export(&path)
        .map_err(|e| e.to_string())?;
    if drawer_core::workspaces::Snapshot::read(&path)
        .map_err(|e| e.to_string())?
        .color_range
        != app.workspaces.store.current.color_range
    {
        return Err("Color range local settings reopen".into());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn production_color_range_apply_references_cancel_stale_settings() {
        let root = crate::tests::TestDirectory::new();
        super::smoke(&root.0).unwrap();
    }
}
