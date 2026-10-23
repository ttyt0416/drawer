use super::*;
pub(super) struct Review {
    id: u64,
    revision: u64,
    frame: u32,
    target: Target,
    keep: bool,
    prepared: Option<core::Prepared>,
    image: Option<egui::TextureHandle>,
    preview: Option<egui::Rect>,
    apply: Option<egui::Rect>,
    cancel: Option<egui::Rect>,
}
pub(super) fn smoke() -> Result<(), String> {
    let ctx = egui::Context::default();
    let mut app = Box::new(Drawer::new(&ctx, None));
    app.simple_view = true;
    app.create_canvas(CanvasSpec {
        width: 17,
        height: 17,
        dpi: 300.,
        background: Background::Transparent,
    })
    .map_err(|e| e.to_string())?;
    app.documents[0].commit_state(drawer_core::quick_filters_verify::fixture()?.snapshot())?;
    let stack = drawer_core::surface_effects::Stack {
        effects: vec![drawer_core::surface_effects::Entry {
            operation: drawer_core::surface_effects::Operation::defaults()[1],
            enabled: true,
            scope: None,
        }],
        ..Default::default()
    };
    let state = drawer_core::surface_effects::prepare(&app.documents[0], 0, stack, |_, _| Ok(()))?;
    app.documents[0].commit_state(state)?;
    let original = app.documents[0].snapshot();
    let target = Target::Raster(app.documents[0].scene.layer_ids.id(0));
    let mut time = 0.;
    let step = |app: &mut Drawer, time: &mut f64, events| {
        *time += 0.1;
        crate::input_replay::frame(&ctx, app, *time, events);
    };
    let settle = |app: &mut Drawer, time: &mut f64| {
        for _ in 0..3 {
            step(app, time, vec![]);
        }
    };
    let click = |app: &mut Drawer, time: &mut f64, r: egui::Rect| {
        for pressed in [true, false] {
            step(
                app,
                time,
                vec![
                    egui::Event::PointerMoved(r.center()),
                    egui::Event::PointerButton {
                        pos: r.center(),
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: Default::default(),
                    },
                ],
            );
        }
    };
    app.layer_command(Command::Delete(target))?;
    settle(&mut app, &mut time);
    if app.documents[0].snapshot() != original {
        return Err("Effect deletion bypassed review".into());
    }
    let cancel = app
        .layer_manager
        .effect_delete
        .as_ref()
        .and_then(|e| e.cancel)
        .ok_or("Delete review cancel")?;
    click(&mut app, &mut time, cancel);
    if app.layer_manager.effect_delete.is_some() || app.documents[0].snapshot() != original {
        return Err("Effect deletion cancel".into());
    }
    for keep in [false, true] {
        app.layer_command(Command::Delete(target))?;
        app.layer_manager
            .effect_delete
            .as_mut()
            .ok_or("Missing delete review")?
            .keep = keep;
        settle(&mut app, &mut time);
        let preview = app
            .layer_manager
            .effect_delete
            .as_ref()
            .and_then(|e| e.preview)
            .ok_or("Delete preview")?;
        click(&mut app, &mut time, preview);
        settle(&mut app, &mut time);
        let apply = app
            .layer_manager
            .effect_delete
            .as_ref()
            .and_then(|e| e.apply)
            .ok_or("Delete confirm")?;
        let count = app.documents[0].history_counts();
        click(&mut app, &mut time, apply);
        if app.layer_manager.effect_delete.is_some()
            || app.documents[0].history_counts().0 != count.0 + 1
            || app.documents[0].layers.len() != if keep { 2 } else { 1 }
        {
            return Err(format!("Effect deletion confirm/retain: {:?}", app.status));
        }
        app.documents[0].undo();
        if app.documents[0].snapshot() != original {
            return Err("Effect deletion review Undo".into());
        }
    }
    Ok(())
}
impl Drawer {
    pub(super) fn review_effect_delete(&mut self, target: Target) -> Result<(), String> {
        let d = &self.documents[self.active.ok_or("Open a page")?];
        let prepared = drawer_core::quick_filters::delete_source(d, target, false)?;
        self.layer_manager.effect_delete = Some(Box::new(Review {
            id: d.id,
            revision: d.revision,
            frame: d.current_frame,
            target,
            keep: false,
            prepared: Some(prepared),
            image: None,
            preview: None,
            apply: None,
            cancel: None,
        }));
        Ok(())
    }
    pub(super) fn effect_delete_window(&mut self, ctx: &egui::Context) {
        let Some(mut e) = self.layer_manager.effect_delete.take() else {
            return;
        };
        let Some(n) = self.active.filter(|n| self.documents[*n].id == e.id) else {
            return;
        };
        let d = &self.documents[n];
        let fresh = d.revision == e.revision && d.current_frame == e.frame;
        let mut open = !ctx.input(|i| i.key_pressed(egui::Key::Escape));
        let (mut prepare, mut apply, mut cancel) = (false, false, false);
        let old = e.keep;
        egui::Window::new("Delete source with attached effects").default_width(500.).open(&mut open).show(ctx,|ui|{
            ui.label(format!("Source: {}",title(d,e.target)));ui.label("Deleting this source also removes its attached live effects and masks. Correction layers elsewhere keep following their compositing order.");
            ui.checkbox(&mut e.keep,"Keep a raster copy of the entire displayed page before deleting");ui.small("The retained copy includes every visible source at the current frame and stays fixed after edits. Other sources stay visible; transparent areas may become denser. Review the resulting page.");
            if e.keep!=old{e.prepared=None;e.image=None;}
            if !fresh{ui.colored_label(egui::Color32::YELLOW,"Document changed; cancel and reopen.");}
            let r=ui.add_enabled(fresh,egui::Button::new("Prepare deletion preview"));e.preview=Some(r.rect);prepare=r.clicked();
            if let Some(image)=&e.image{ui.add(egui::Image::new(image).fit_to_exact_size(egui::vec2(180.,180.)));}
            let r=ui.add_enabled(fresh&&e.prepared.is_some()&&e.image.is_some(),egui::Button::new("Confirm source deletion as one Undo"));e.apply=Some(r.rect);apply=r.clicked();let r=ui.button("Cancel deletion");e.cancel=Some(r.rect);cancel=r.clicked();
        });
        if prepare {
            let result = (|| {
                let p = drawer_core::quick_filters::delete_source(
                    &self.documents[n],
                    e.target,
                    e.keep,
                )?;
                let after = drawer_core::layer_merge::preview(&p.state, e.frame)?;
                let image = crate::layer_merge::image(ctx, &after, "effect-delete-preview");
                Ok::<_, String>((p, image))
            })();
            match result {
                Ok((p, image)) => {
                    e.prepared = Some(p);
                    e.image = Some(image);
                }
                Err(error) => self.status = Some(error),
            }
        }
        if apply
            && fresh
            && let Some(p) = e.prepared.take()
        {
            let old = stamp(
                &self.documents[n],
                self.panels
                    .targets
                    .get(&e.id)
                    .copied()
                    .unwrap_or(drawer_core::thumbnail::Target::Raster(0)),
            );
            match self.documents[n].commit_state(p.state) {
                Ok(_) => {
                    let d = &self.documents[n];
                    remap_tools(&mut self.workspaces.store.current, d, &old);
                    let target =
                        resolve(d, p.selected).unwrap_or(drawer_core::thumbnail::Target::Raster(0));
                    self.panels.targets.insert(d.id, target);
                    self.layer_manager.tracked.insert(d.id, stamp(d, target));
                    self.layer_manager.draft = None;
                    self.status = None;
                    return;
                }
                Err(error) => self.status = Some(error),
            }
        }
        if open && !cancel {
            self.layer_manager.effect_delete = Some(e);
        }
    }
}
#[cfg(test)]
mod tests {
    #[test]
    fn source_effect_delete_mouse_preview_cancel_drop_keep_and_undo() {
        super::smoke().unwrap();
    }
}
