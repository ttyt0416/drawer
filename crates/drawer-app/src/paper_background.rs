use super::*;
use drawer_core::paper_background as core;
#[derive(Default)]
pub(super) struct Panel {
    draft: Option<Box<Draft>>,
    preview: Option<egui::Rect>,
    apply: Option<egui::Rect>,
    display: Option<egui::Rect>,
}
struct Draft {
    source: Option<(u64, u64, u32)>,
    color: [u8; 3],
    visible: bool,
    checker: [[u8; 3]; 2],
    state: Option<drawer_core::sizing::DocumentState>,
    images: Option<[egui::TextureHandle; 2]>,
}
impl Drawer {
    pub(super) fn begin_paper_background(&mut self) -> Result<(), String> {
        self.finish_stroke();
        let d = self.active.map(|n| &self.documents[n]);
        let paper = d.and_then(|d| {
            d.scene
                .paper_background
                .and_then(|id| drawer_core::layers::index(d, id).map(|n| (d, n)))
        });
        self.paper_background.draft = Some(Box::new(Draft {
            source: d.map(|d| (d.id, d.revision, d.current_frame)),
            color: paper.map_or([255; 3], |(d, n)| {
                let p = d.layers[n].pixels[0];
                [p[0], p[1], p[2]]
            }),
            visible: paper.is_none_or(|(d, n)| {
                !drawer_core::layer_properties::properties(
                    d,
                    drawer_core::layer_properties::raster(d, n),
                )
                .hidden
            }),
            checker: self.workspaces.store.current.transparency_colors,
            state: None,
            images: None,
        }));
        Ok(())
    }
    fn preview_paper_background(&mut self, ctx: &egui::Context) -> Result<(), String> {
        let e = self
            .paper_background
            .draft
            .as_ref()
            .ok_or("Paper editor closed")?;
        let d = &self.documents[self.active.ok_or("Open a page for an output paper layer")?];
        if e.source != Some((d.id, d.revision, d.current_frame)) {
            return Err("Page/frame changed; reopen paper editor".into());
        }
        let used: usize = self.documents.iter().map(Document::resident_bytes).sum();
        if used
            .saturating_add(d.memory_bytes().saturating_mul(4))
            .saturating_add((d.width as usize).saturating_mul(d.height as usize) * 16)
            > DEFAULT_DOCUMENT_BUDGET
        {
            return Err("Paper preview exceeds workspace memory".into());
        }
        let state = core::prepare(d, e.color, e.visible)?;
        let after = drawer_core::layer_merge::preview(&state, d.current_frame)?;
        let images = [
            super::layer_merge::image(ctx, d, "paper-before"),
            super::layer_merge::image(ctx, &after, "paper-after"),
        ];
        let e = self.paper_background.draft.as_mut().unwrap();
        e.state = Some(state);
        e.images = Some(images);
        Ok(())
    }
    fn apply_paper_background(&mut self) -> Result<(), String> {
        let n = self.active.ok_or("Open a page first")?;
        let e = self
            .paper_background
            .draft
            .as_mut()
            .ok_or("Paper editor closed")?;
        let d = &mut self.documents[n];
        if e.source != Some((d.id, d.revision, d.current_frame)) {
            return Err("Page/frame changed; reopen paper editor".into());
        }
        d.commit_state(e.state.take().ok_or("Preview the paper layer first")?)?;
        self.paper_background.draft = None;
        Ok(())
    }
    fn apply_transparency_colors(&mut self) -> Result<(), String> {
        let colors = self
            .paper_background
            .draft
            .as_ref()
            .ok_or("Paper editor closed")?
            .checker;
        let old = self.workspaces.store.current.transparency_colors;
        self.workspaces.store.current.transparency_colors = colors;
        if let Err(e) = self.persist_workspace() {
            self.workspaces.store.current.transparency_colors = old;
            return Err(e);
        }
        self.canvas_view.set_checker(colors);
        self.paper_background.draft = None;
        Ok(())
    }
    pub(super) fn paper_background_window(&mut self, ctx: &egui::Context) {
        let Some(mut e) = self.paper_background.draft.take() else {
            return;
        };
        let fresh = self.active.is_some_and(|n| {
            e.source
                == Some((
                    self.documents[n].id,
                    self.documents[n].revision,
                    self.documents[n].current_frame,
                ))
        });
        let mut open = !ctx.input(|i| i.key_pressed(egui::Key::Escape));
        let (mut preview, mut apply, mut display, mut cancel) = (false, false, false, false);
        egui::Window::new("Paper and transparency display").open(&mut open).default_pos(egui::pos2(30.,25.)).default_width(515.).collapsible(false).show(ctx,|ui|{
            ui.label("Output paper layer");
            let mut changed=ui.color_edit_button_srgb(&mut e.color).changed();
            changed|=ui.checkbox(&mut e.visible,"Show paper in the page and exported images").changed();
            ui.small("Creates an ordinary raster below existing ink. Replacing its color replaces its full pixel rectangle. You can hide/delete/rename it in Layers; hiding restores the ink's original transparency. Other layer settings remain editable.");
            if changed{e.state=None;e.images=None;}
            let r=ui.add_enabled(fresh,egui::Button::new("Preview output paper"));self.paper_background.preview=Some(r.rect);preview=r.clicked();
            ui.allocate_ui(egui::vec2(300.,112.),|ui|{if let Some(images)=&e.images{ui.horizontal(|ui|{for(label,image)in[("Current page",&images[0]),("Output paper",&images[1])]{ui.vertical(|ui|{ui.label(label);ui.add(egui::Image::new(image).fit_to_exact_size(egui::vec2(96.,96.)));});}});}});
            let r=ui.add_enabled(fresh&&e.state.is_some(),egui::Button::new("Apply paper layer (one Undo)"));self.paper_background.apply=Some(r.rect);apply=r.clicked();
            if !fresh{ui.label("Open a page, or reopen this editor after page/frame changes.");}
            ui.separator();ui.label("Transparency display only (local preferences)");
            ui.horizontal(|ui|{for(index,color)in e.checker.iter_mut().enumerate(){ui.label(if index==0{"First check"}else{"Second check"});ui.color_edit_button_srgb(color);}});
            if ui.button("Reset transparency colors").clicked(){e.checker=core::checker_default();}
            let c=e.checker.map(|c|egui::Color32::from_rgb(c[0],c[1],c[2]));
            let(r,_)=ui.allocate_exact_size(egui::vec2(128.,32.),egui::Sense::hover());for y in 0..2{for x in 0..8{ui.painter().rect_filled(egui::Rect::from_min_size(r.min+egui::vec2(x as f32*16.,y as f32*16.),egui::vec2(16.,16.)),0.,c[(x+y)%2]);}}
            ui.small("Check colors never enter document pixels, PNG/PSD output, document dirty state or Undo history. These buttons apply separately.");
            ui.horizontal(|ui|{let r=ui.button("Apply display colors");self.paper_background.display=Some(r.rect);display=r.clicked();cancel=ui.button("Cancel").clicked();});
        });
        if open && !cancel {
            self.paper_background.draft = Some(e);
        }
        let result = if preview {
            self.preview_paper_background(ctx)
        } else if apply {
            self.apply_paper_background()
        } else if display {
            self.apply_transparency_colors()
        } else {
            Ok(())
        };
        if let Err(err) = result {
            self.status = Some(err);
        }
    }
}
pub(super) fn smoke(root: &std::path::Path) -> Result<(), String> {
    drawer_core::paper_background_verify::verify(root)?;
    let ctx = egui::Context::default();
    let mut app = Drawer::new(&ctx, None);
    app.simple_view = true;
    app.create_canvas(CanvasSpec {
        width: 4,
        height: 4,
        dpi: 300.,
        background: Background::Transparent,
    })
    .map_err(|e| e.to_string())?;
    app.documents[0].layers[0].pixels[5] = [200, 100, 50, 128];
    let original = app.documents[0].snapshot();
    let counts = app.documents[0].history_counts();
    let dirty = app.documents[0].dirty;
    let mut time = 0.;
    let step = |app: &mut Drawer, time: &mut f64, events| {
        *time += 0.1;
        super::input_replay::frame(&ctx, app, *time, events);
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
    for _ in 0..2 {
        step(&mut app, &mut time, vec![]);
    }
    app.execute_command(&ctx, "paper-background");
    let checker = [[2, 190, 80], [200, 20, 240]];
    app.paper_background
        .draft
        .as_mut()
        .ok_or("Paper command missing")?
        .checker = checker;
    for _ in 0..3 {
        step(&mut app, &mut time, vec![]);
    }
    let r = app
        .paper_background
        .display
        .ok_or("Display colors button missing")?;
    click(&mut app, &mut time, r);
    if app.paper_background.draft.is_some()
        || app.canvas_view.checker_colors != checker
        || app.documents[0].snapshot() != original
        || app.documents[0].history_counts() != counts
        || app.documents[0].dirty != dirty
    {
        return Err(format!(
            "Display colors touched document or failed: {:?}",
            app.status
        ));
    }
    // A new/separate canvas receives the persisted preference through the regular frame path.
    let mut view = super::canvas_view::CanvasView::new(&ctx);
    view.set_checker(app.workspaces.store.current.transparency_colors);
    if view.checker_colors != checker {
        return Err("Separate canvas checker colors".into());
    }
    app.execute_command(&ctx, "paper-background");
    app.paper_background.draft.as_mut().unwrap().color = [20, 40, 60];
    for _ in 0..3 {
        step(&mut app, &mut time, vec![]);
    }
    let r = app
        .paper_background
        .preview
        .ok_or("Paper preview missing")?;
    click(&mut app, &mut time, r);
    for _ in 0..3 {
        step(&mut app, &mut time, vec![]);
    }
    if app.documents[0].snapshot() != original {
        return Err("Paper preview changed original".into());
    }
    let r = app.paper_background.apply.ok_or("Paper Apply missing")?;
    click(&mut app, &mut time, r);
    if app.paper_background.draft.is_some()
        || app.documents[0].scene.paper_background.is_none()
        || app.documents[0].history_counts().0 != counts.0 + 1
        || drawer_core::render::Renderer::new(&app.documents[0]).pixel([0.5; 2])
            != [20, 40, 60, 255]
    {
        return Err(format!("Paper Apply failed: {:?}", app.status));
    }
    let applied = app.documents[0].snapshot();
    if !app.documents[0].undo()
        || app.documents[0].snapshot() != original
        || !app.documents[0].redo()
        || app.documents[0].snapshot() != applied
    {
        return Err("Paper UI Undo/Redo".into());
    }
    app.execute_command(&ctx, "paper-background");
    app.documents[0].current_frame = 1;
    if app.preview_paper_background(&ctx).is_ok() {
        return Err("Stale paper frame accepted".into());
    }
    app.documents[0].current_frame = 0;
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
    if app.paper_background.draft.is_some() || app.documents[0].snapshot() != applied {
        return Err("Paper Escape changed page".into());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn actual_paper_preview_apply_checker_preferences_and_undo() {
        let root = crate::tests::TestDirectory::new();
        super::smoke(&root.0).unwrap();
    }
}
