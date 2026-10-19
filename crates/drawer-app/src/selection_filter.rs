use super::*;
use drawer_core::selection_filter::{self as core, Operation};
#[derive(Default)]
pub(super) struct Panel {
    pub draft: Option<Draft>,
    apply: Option<egui::Rect>,
}
pub(super) struct Draft {
    id: u64,
    revision: u64,
    frame: u32,
    op: Operation,
    radius: u8,
}
impl Drawer {
    pub(super) fn begin_selection_filter(&mut self, op: Operation) -> Result<(), String> {
        let d = &self.documents[self.active.ok_or("Open a page first")?];
        self.selection_filter.draft = Some(Draft {
            id: d.id,
            revision: d.revision,
            frame: d.current_frame,
            op,
            radius: 1,
        });
        Ok(())
    }
    fn apply_selection_filter(&mut self) -> Result<(), String> {
        let e = self
            .selection_filter
            .draft
            .as_ref()
            .ok_or("Selection filter is closed")?;
        let n = self.active.ok_or("Open a page first")?;
        let d = &mut self.documents[n];
        if d.id != e.id || d.revision != e.revision || d.current_frame != e.frame {
            return Err("Page/frame changed; reopen the selection filter".into());
        }
        let scene = core::prepare(d, e.op, e.radius)?;
        d.edit_scene(scene)?;
        self.selection_filter.draft = None;
        Ok(())
    }
    pub(super) fn selection_filter_window(&mut self, ctx: &egui::Context) {
        let Some(e) = &mut self.selection_filter.draft else {
            return;
        };
        let mut open = !ctx.input(|i| i.key_pressed(egui::Key::Escape));
        let mut apply = false;
        let mut cancel = false;
        self.selection_filter.apply = None;
        egui::Window::new("Selection boundary").open(&mut open).collapsible(false).show(ctx,|ui| {
            egui::ComboBox::from_id_salt("selection-filter-operation").selected_text(format!("{:?}",e.op)).show_ui(ui,|ui|{for op in Operation::ALL{ui.selectable_value(&mut e.op,op,format!("{op:?}"));}});
            ui.add(egui::Slider::new(&mut e.radius,0..=64).text("Radius in document pixels"));
            ui.label("Expand/Shrink use a square neighborhood; Feather uses a box average. Radius 0 keeps the exact mask. Apply changes only the selection, as one Undo.");
            ui.horizontal(|ui| {let r=ui.button("Apply selection boundary");self.selection_filter.apply=Some(r.rect);apply=r.clicked();cancel=ui.button("Cancel").clicked();});
        });
        if !open || cancel {
            self.selection_filter.draft = None;
        }
        if apply && let Err(e) = self.apply_selection_filter() {
            self.status = Some(e);
        }
    }
}
pub(super) fn smoke(root: &std::path::Path) -> Result<(), String> {
    core::verify(root)?;
    let ctx = egui::Context::default();
    let mut app = Drawer::new(&ctx, None);
    app.simple_view = true;
    app.create_canvas(CanvasSpec {
        width: 8,
        height: 8,
        dpi: 300.0,
        background: Background::Transparent,
    })
    .map_err(|e| e.to_string())?;
    app.documents[0].scene.selection_mask = Some(drawer_core::selection::Mask {
        runs: vec![drawer_core::selection::Run {
            x: 3,
            y: 3,
            length: 1,
            alpha: 128,
        }],
    });
    let mut time = 0.0;
    let step = |app: &mut Drawer, time: &mut f64, events| {
        *time += 0.1;
        super::input_replay::frame(&ctx, app, *time, events);
    };
    step(&mut app, &mut time, vec![]);
    step(&mut app, &mut time, vec![]);
    for command in ["selection-expand", "selection-shrink", "selection-feather"] {
        let before = app.documents[0].snapshot();
        let history = app.documents[0].history_counts();
        app.execute_command(&ctx, command);
        app.selection_filter
            .draft
            .as_mut()
            .ok_or("Selection filter command missing")?
            .radius = 0;
        app.apply_selection_filter()?;
        if app.documents[0].snapshot() != before || app.documents[0].history_counts() != history {
            return Err("Zero-radius UI changed history".into());
        }
        app.execute_command(&ctx, command);
        let e = app.selection_filter.draft.as_ref().unwrap();
        let expected = core::prepare(&app.documents[0], e.op, e.radius)?;
        step(&mut app, &mut time, vec![]);
        step(&mut app, &mut time, vec![]);
        if app.documents[0].snapshot() != before {
            return Err("Filter draft changed page".into());
        }
        let rect = app
            .selection_filter
            .apply
            .ok_or("Filter Apply button missing")?;
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
        if app.selection_filter.draft.is_some()
            || app.documents[0].scene.selection_mask != expected.selection_mask
            || app.documents[0].history_counts().0 != history.0 + 1
            || app.documents[0].snapshot().layers != before.layers
        {
            return Err(format!("Filter Apply {command}: {:?}", app.status));
        }
        if !app.documents[0].undo() || app.documents[0].snapshot() != before {
            return Err("Filter UI Undo".into());
        }
        app.execute_command(&ctx, command);
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
        if app.selection_filter.draft.is_some() || app.documents[0].snapshot() != before {
            return Err("Filter cancel changed page".into());
        }
        app.execute_command(&ctx, command);
        app.documents[0].edit_pixel(0, 0, 0, [20, 40, 60, 255]);
        let changed = app.documents[0].snapshot();
        if app.apply_selection_filter().is_ok() || app.documents[0].snapshot() != changed {
            return Err("Stale filter changed document".into());
        }
        app.selection_filter.draft = None;
        app.documents[0].undo();
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn production_boundary_dialog_zero_apply_cancel_stale() {
        let root = crate::tests::TestDirectory::new();
        super::smoke(&root.0).unwrap();
    }
}
