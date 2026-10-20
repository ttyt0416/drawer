use super::*;
use drawer_core::{
    layer_masks::{self as core, Command, Policy},
    layer_properties::{self as props, Target},
};
#[derive(Default)]
pub(super) struct Panel {
    draft: Option<Box<Draft>>,
    preview: Option<egui::Rect>,
    apply: Option<egui::Rect>,
    overlay: Option<egui::Rect>,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Operation {
    Outside,
    Inside,
    Invert,
    Enable,
    Link,
    Move,
    Copy,
    Apply,
    Bake,
    Remove,
}
struct Draft {
    id: u64,
    revision: u64,
    frame: u32,
    target: Target,
    operation: Operation,
    policy: Policy,
    source: Target,
    replace: bool,
    enabled: bool,
    linked: bool,
    delta: [f64; 2],
    prepared: Option<core::Prepared>,
    images: Option<[egui::TextureHandle; 2]>,
}
impl Draft {
    fn command(&self) -> Command {
        match self.operation {
            Operation::Outside | Operation::Inside => Command::Selection {
                hide_inside: self.operation == Operation::Inside,
                policy: self.policy,
            },
            Operation::Invert => Command::Invert,
            Operation::Enable => Command::Enabled(self.enabled),
            Operation::Link => Command::Linked(self.linked),
            Operation::Move => Command::Move(self.delta),
            Operation::Copy => Command::Copy {
                source: self.source,
                replace: self.replace,
            },
            Operation::Apply => Command::Apply,
            Operation::Bake => Command::Bake,
            Operation::Remove => Command::Remove,
        }
    }
}
impl Drawer {
    pub(super) fn begin_layer_masks(&mut self) -> Result<(), String> {
        self.finish_stroke();
        let d = &self.documents[self.active.ok_or("Open a page first")?];
        let target = props::from_thumbnail(
            d,
            self.panels
                .targets
                .get(&d.id)
                .copied()
                .unwrap_or(drawer_core::thumbnail::Target::Raster(0)),
        );
        let mask = core::get(d, target);
        let source = props::targets(d)
            .into_iter()
            .find(|t| *t != target && core::get(d, *t).is_some())
            .unwrap_or(target);
        self.layer_masks.draft = Some(Box::new(Draft {
            id: d.id,
            revision: d.revision,
            frame: d.current_frame,
            target,
            operation: Operation::Outside,
            policy: Policy::Create,
            source,
            replace: false,
            enabled: mask.is_none_or(|m| m.enabled),
            linked: mask.is_none_or(|m| m.linked),
            delta: [0.; 2],
            prepared: None,
            images: None,
        }));
        Ok(())
    }
    fn preview_layer_mask(&mut self, ctx: &egui::Context) -> Result<(), String> {
        let e = self
            .layer_masks
            .draft
            .as_ref()
            .ok_or("Mask editor closed")?;
        let d = &self.documents[self.active.ok_or("Open a page first")?];
        if (d.id, d.revision, d.current_frame) != (e.id, e.revision, e.frame) {
            return Err("Page/frame changed; reopen mask controls".into());
        }
        let used: usize = self.documents.iter().map(Document::resident_bytes).sum();
        if used
            .saturating_add(d.memory_bytes().saturating_mul(4))
            .saturating_add(128 * 1024 * 1024)
            > DEFAULT_DOCUMENT_BUDGET
        {
            return Err("Mask preview exceeds workspace memory".into());
        }
        let prepared = core::prepare(d, e.target, e.command())?;
        if used.saturating_add(prepared.state.memory_bytes().saturating_mul(4))
            > DEFAULT_DOCUMENT_BUDGET
        {
            return Err("Mask result exceeds workspace memory".into());
        }
        let after = drawer_core::layer_merge::preview(&prepared.state, e.frame)?;
        let images = [
            super::layer_merge::image(ctx, d, "mask-before"),
            super::layer_merge::image(ctx, &after, "mask-after"),
        ];
        let e = self.layer_masks.draft.as_mut().unwrap();
        e.prepared = Some(prepared);
        e.images = Some(images);
        Ok(())
    }
    fn apply_layer_mask(&mut self) -> Result<(), String> {
        let n = self.active.ok_or("Open a page first")?;
        let e = self
            .layer_masks
            .draft
            .as_mut()
            .ok_or("Mask editor closed")?;
        let d = &self.documents[n];
        if (d.id, d.revision, d.current_frame) != (e.id, e.revision, e.frame) {
            return Err("Page/frame changed; reopen mask controls".into());
        }
        let p = e
            .prepared
            .take()
            .ok_or("Preview the mask operation first")?;
        self.documents[n].commit_state(p.state)?;
        let d = &self.documents[n];
        let key = drawer_core::layer_catalog::key(d, p.selected);
        if let Some(t) = drawer_core::layer_catalog::thumbnail(d, key) {
            self.panels.targets.insert(d.id, t);
        }
        self.layer_masks.draft = None;
        self.sync_layer_identity();
        Ok(())
    }
    pub(super) fn layer_masks_window(&mut self, ctx: &egui::Context) {
        let Some(mut e) = self.layer_masks.draft.take() else {
            return;
        };
        let Some(n) = self.active.filter(|n| self.documents[*n].id == e.id) else {
            return;
        };
        let d = &self.documents[n];
        let fresh = (d.revision, d.current_frame) == (e.revision, e.frame);
        let mut open = !ctx.input(|i| i.key_pressed(egui::Key::Escape));
        let (mut preview, mut apply, mut cancel) = (false, false, false);
        egui::Window::new("Layer mask operations").open(&mut open).default_pos(egui::pos2(25.,20.)).min_width(530.).max_width(530.).collapsible(false).show(ctx,|ui|{
            let r=ui.checkbox(&mut self.workspaces.store.current.layer_mask_overlay,"Show stored mask area on canvas (red hides; view only)");self.layer_masks.overlay=Some(r.rect);
            if !fresh{ui.label("Page/frame changed; cancel and reopen mask controls.");return;}
            let mut changed=false;
            ui.allocate_ui(egui::vec2(490.,230.),|ui|{egui::ScrollArea::vertical().id_salt("mask-options").max_height(230.).show(ui,|ui|{
            let targets=props::targets(d).into_iter().filter(|t|matches!(t,Target::Raster(_)|Target::Vector(_)|Target::Folder(_))).collect::<Vec<_>>();
            egui::ComboBox::from_label("Mask owner").selected_text(super::layer_properties::label(d,e.target)).show_ui(ui,|ui|{for t in &targets{if ui.selectable_value(&mut e.target,*t,super::layer_properties::label(d,*t)).changed(){changed=true;let m=core::get(d,*t);e.enabled=m.is_none_or(|m|m.enabled);e.linked=m.is_none_or(|m|m.linked);}}});
            ui.horizontal_wrapped(|ui|{for(op,label)in[(Operation::Outside,"Hide outside selection"),(Operation::Inside,"Hide inside selection"),(Operation::Invert,"Invert"),(Operation::Enable,"Enable/disable"),(Operation::Link,"Link movement"),(Operation::Move,"Move mask"),(Operation::Copy,"Copy mask"),(Operation::Apply,"Apply mask to alpha"),(Operation::Bake,"Bake masked content"),(Operation::Remove,"Remove")]{changed|=ui.radio_value(&mut e.operation,op,label).changed();}});
            match e.operation{
                Operation::Outside|Operation::Inside=>{ui.horizontal(|ui|{for(p,label)in[(Policy::Create,"Create only"),(Policy::Replace,"Replace"),(Policy::Intersect,"Intersect")]{changed|=ui.radio_value(&mut e.policy,p,label).changed();}});ui.small("Selection strengths become pass alpha (outside hides) or inverted pass alpha (inside hides). Existing masks require an explicit Replace or Intersect.");},
                Operation::Enable=>{changed|=ui.checkbox(&mut e.enabled,"Mask enabled").changed();},
                Operation::Link=>{changed|=ui.checkbox(&mut e.linked,"Move mask with layer").changed();},
                Operation::Move=>{ui.horizontal(|ui|{for(k,label)in["Move mask X ","Move mask Y "].iter().enumerate(){changed|=ui.add(egui::DragValue::new(&mut e.delta[k]).range(-1_000_000. ..=1_000_000.).prefix(*label)).changed();}});},
                Operation::Copy=>{egui::ComboBox::from_label("Copy source").selected_text(super::layer_properties::label(d,e.source)).show_ui(ui,|ui|{for t in &targets{if core::get(d,*t).is_some(){changed|=ui.selectable_value(&mut e.source,*t,super::layer_properties::label(d,*t)).changed();}}});changed|=ui.checkbox(&mut e.replace,"Replace an existing destination mask").changed();ui.small("Copy preserves original-page position, enabled, inversion and movement linkage; source stays independent.");},
                Operation::Apply=>{ui.label("Apply only multiplies stored raster alpha by this mask and removes it. RGB and other live effects/properties remain editable; live effects can render differently from the new alpha. Use ordinary merge or Bake masked content to bake the complete displayed recipe.");},
                Operation::Bake=>{ui.label("Bake masked content rasterizes masks and display effects together. Vector paths and folder structure become raster content. This is separate from applying only a raster alpha mask; one Undo restores editable data.");},
                _=>{}
            }
            });});
            if changed{e.prepared=None;e.images=None;}
            if let Some(m)=core::get(d,e.target){ui.small(format!("Stored mask: enabled={} / linked={} / inverted={}",m.enabled,m.linked,m.inverted));}
            let r=ui.button("Preview mask operation");self.layer_masks.preview=Some(r.rect);preview=r.clicked();
            ui.allocate_ui(egui::vec2(490.,60.),|ui|{egui::ScrollArea::vertical().id_salt("mask-notes").max_height(60.).show(ui,|ui|{if let Some(p)=&e.prepared{for note in &p.notes{ui.small(note);}}});});
            ui.allocate_ui(egui::vec2(300.,118.),|ui|{if let Some(images)=&e.images{ui.horizontal(|ui|{for(label,image)in[("Current",&images[0]),("After mask operation",&images[1])]{ui.vertical(|ui|{ui.label(label);ui.add(egui::Image::new(image).fit_to_exact_size(egui::vec2(96.,96.)));});}});}});
            ui.horizontal(|ui|{let r=ui.add_enabled(e.prepared.is_some(),egui::Button::new("Apply reviewed mask operation"));self.layer_masks.apply=Some(r.rect);apply=r.clicked();cancel=ui.button("Cancel").clicked();});
        });
        if open && !cancel {
            self.layer_masks.draft = Some(e);
        }
        let r = if preview {
            self.preview_layer_mask(ctx)
        } else if apply {
            self.apply_layer_mask()
        } else {
            Ok(())
        };
        if let Err(e) = r {
            self.status = Some(e);
        }
    }
}
pub(super) type OverlayCache = Option<(u64, u64, Target, egui::TextureHandle)>;
pub(super) fn paint_overlay(
    ctx: &egui::Context,
    painter: &egui::Painter,
    d: &Document,
    target: Target,
    center: [f64; 2],
    ppp: f64,
    cache: &mut OverlayCache,
) -> usize {
    let Some(mask) = core::get(d, target) else {
        return 0;
    };
    if cache
        .as_ref()
        .is_none_or(|c| (c.0, c.1, c.2) != (d.id, d.revision, target))
    {
        let size = [d.width.min(256) as usize, d.height.min(256) as usize];
        let mut pixels = Vec::with_capacity(size[0] * size[1]);
        let mut mask = mask.clone();
        mask.enabled = true;
        for y in 0..size[1] {
            for x in 0..size[0] {
                let p = d.scene.transform.inverse().map([
                    (x as f64 + 0.5) * d.width as f64 / size[0] as f64,
                    (y as f64 + 0.5) * d.height as f64 / size[1] as f64,
                ]);
                pixels.push(egui::Color32::from_rgba_unmultiplied(
                    255,
                    35,
                    45,
                    ((1. - mask.factor(p)) * 128.).round() as u8,
                ));
            }
        }
        *cache = Some((
            d.id,
            d.revision,
            target,
            ctx.load_texture(
                "layer-mask-area",
                egui::ColorImage::new(size, pixels),
                egui::TextureOptions::NEAREST,
            ),
        ));
    }
    let size = [d.width as f64, d.height as f64];
    let mut mesh = egui::Mesh::with_texture(cache.as_ref().unwrap().3.id());
    for (p, uv) in [
        ([0., 0.], egui::pos2(0., 0.)),
        ([size[0], 0.], egui::pos2(1., 0.)),
        (size, egui::pos2(1., 1.)),
        ([0., size[1]], egui::pos2(0., 1.)),
    ] {
        let p = d.view.document_to_screen(p, size, center, ppp);
        mesh.vertices.push(egui::epaint::Vertex {
            pos: egui::pos2(p[0] as f32, p[1] as f32),
            uv,
            color: egui::Color32::WHITE,
        });
    }
    mesh.indices.extend([0, 1, 2, 0, 2, 3]);
    painter.add(egui::Shape::mesh(mesh));
    1
}

pub(super) fn smoke(root: &std::path::Path) -> Result<(), String> {
    drawer_core::layer_masks_verify::verify(root)?;
    let ctx = egui::Context::default();
    let mut app = Drawer::new(&ctx, None);
    app.simple_view = true;
    app.create_canvas(CanvasSpec {
        width: 3,
        height: 1,
        dpi: 300.,
        background: Background::Transparent,
    })
    .map_err(|e| e.to_string())?;
    app.documents[0].commit_state(drawer_core::layer_masks_verify::fixture()?.snapshot())?;
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
    step(&mut app, &mut time, vec![]);
    step(&mut app, &mut time, vec![]);
    app.execute_command(&ctx, "layer-masks");
    step(&mut app, &mut time, vec![]);
    step(&mut app, &mut time, vec![]);
    let before = app.documents[0].snapshot();
    let counts = app.documents[0].history_counts();
    let r = app
        .layer_masks
        .preview
        .ok_or("Mask preview button missing")?;
    click(&mut app, &mut time, r);
    if app.documents[0].snapshot() != before
        || app
            .layer_masks
            .draft
            .as_ref()
            .is_none_or(|e| e.prepared.is_none())
    {
        return Err(format!(
            "Mask UI preview changed source or failed: {:?}",
            app.status
        ));
    }
    step(&mut app, &mut time, vec![]);
    let r = app.layer_masks.apply.ok_or("Mask apply button missing")?;
    click(&mut app, &mut time, r);
    if app.layer_masks.draft.is_some()
        || app.documents[0].history_counts().0 != counts.0 + 1
        || app.documents[0].composite_pixel(1, 0)[3] != 64
    {
        return Err(format!(
            "Mask selection UI apply/Undo failed: {:?}",
            app.status
        ));
    }
    let with_mask = app.documents[0].snapshot();
    if !app.documents[0].undo() || app.documents[0].snapshot() != before || !app.documents[0].redo()
    {
        return Err("Mask UI creation Undo/Redo".into());
    }
    app.execute_command(&ctx, "layer-masks");
    app.layer_masks.draft.as_mut().unwrap().operation = Operation::Invert;
    step(&mut app, &mut time, vec![]);
    step(&mut app, &mut time, vec![]);
    let r = app.layer_masks.preview.unwrap();
    click(&mut app, &mut time, r);
    step(&mut app, &mut time, vec![]);
    let r = app.layer_masks.apply.unwrap();
    click(&mut app, &mut time, r);
    if app.documents[0].composite_pixel(0, 0)[3] != 128
        || !app.documents[0].undo()
        || app.documents[0].snapshot() != with_mask
    {
        return Err("Mask inversion UI/Undo".into());
    }
    app.execute_command(&ctx, "layer-masks");
    step(&mut app, &mut time, vec![]);
    step(&mut app, &mut time, vec![]);
    let original = app.documents[0].snapshot();
    let counts = app.documents[0].history_counts();
    let dirty = app.documents[0].dirty;
    let png_before = root.join("mask-overlay-before.png");
    drawer_core::png_export::export(&app.documents[0], &png_before, DEFAULT_DOCUMENT_BUDGET)
        .map_err(|e| e.to_string())?;
    let r = app
        .layer_masks
        .overlay
        .ok_or("Mask overlay checkbox missing")?;
    click(&mut app, &mut time, r);
    step(&mut app, &mut time, vec![]);
    if !app.workspaces.store.current.layer_mask_overlay
        || app.canvas_view.mask_overlay_regions != 1
        || app.documents[0].snapshot() != original
        || app.documents[0].history_counts() != counts
        || app.documents[0].dirty != dirty
    {
        return Err("Mask overlay not drawn or changed document/Undo".into());
    }
    let png_after = root.join("mask-overlay-after.png");
    drawer_core::png_export::export(&app.documents[0], &png_after, DEFAULT_DOCUMENT_BUDGET)
        .map_err(|e| e.to_string())?;
    if std::fs::read(&png_before).map_err(|e| e.to_string())?
        != std::fs::read(&png_after).map_err(|e| e.to_string())?
    {
        return Err("Mask area overlay leaked into PNG".into());
    }
    let pref = root.join("mask-area-workspace.json");
    app.workspaces
        .store
        .current
        .export(&pref)
        .map_err(|e| e.to_string())?;
    let reloaded = drawer_core::workspaces::Snapshot::read(&pref).map_err(|e| e.to_string())?;
    if !reloaded.layer_mask_overlay
        || drawer_core::workspaces::Snapshot::default().layer_mask_overlay
    {
        return Err("Mask area preference restart/reset".into());
    }
    app.layer_masks.draft.as_mut().unwrap().operation = Operation::Apply;
    step(&mut app, &mut time, vec![]);
    step(&mut app, &mut time, vec![]);
    let r = app.layer_masks.preview.unwrap();
    click(&mut app, &mut time, r);
    step(&mut app, &mut time, vec![]);
    step(&mut app, &mut time, vec![]);
    let r = app.layer_masks.apply.unwrap();
    click(&mut app, &mut time, r);
    if core::get(&app.documents[0], Target::Raster(1)).is_some()
        || app.documents[0].layers[0].pixels[1][3] != 64
        || !app.documents[0].undo()
        || app.documents[0].snapshot() != original
    {
        return Err(format!(
            "Apply alpha mask UI/Undo: {:?}, mask={}, alpha={}, open={}",
            app.status,
            core::get(&app.documents[0], Target::Raster(1)).is_some(),
            app.documents[0].layers[0].pixels[1][3],
            app.layer_masks.draft.is_some()
        ));
    }
    app.execute_command(&ctx, "layer-masks");
    app.layer_masks.draft.as_mut().unwrap().operation = Operation::Invert;
    app.preview_layer_mask(&ctx)?;
    app.documents[0].current_frame += 1;
    let before = app.documents[0].snapshot();
    if app.apply_layer_mask().is_ok() || app.documents[0].snapshot() != before {
        return Err("Stale frame mask applied".into());
    }
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
    if app.layer_masks.draft.is_some() {
        return Err("Mask editor Escape failed".into());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn mask_alpha_apply_and_full_bake_buttons_keep_distinct_pixel_and_effect_channels() {
        use super::*;
        for operation in [Operation::Apply, Operation::Bake] {
            let ctx = egui::Context::default();
            let mut app = Drawer::new(&ctx, None);
            app.simple_view = true;
            app.create_canvas(CanvasSpec {
                width: 1,
                height: 1,
                dpi: 300.,
                background: Background::Transparent,
            })
            .unwrap();
            app.documents[0].layers[0].pixels[0] = [200, 100, 50, 128];
            app.documents[0].scene.appearance.rasters.insert(
                1,
                drawer_core::appearance::Style {
                    enabled: true,
                    color: Some([10, 20, 30]),
                    ..Default::default()
                },
            );
            app.documents[0]
                .scene
                .layer_properties
                .rasters
                .entry(1)
                .or_default()
                .opacity = 50;
            app.documents[0]
                .scene
                .layer_effects
                .rasters
                .entry(1)
                .or_default()
                .mask = Some(drawer_core::layer_effects::Mask {
                space: Default::default(),
                coverage: drawer_core::selection::Mask {
                    runs: vec![drawer_core::selection::Run {
                        x: 0,
                        y: 0,
                        length: 1,
                        alpha: 128,
                    }],
                },
                offset: [0.; 2],
                enabled: true,
                linked: true,
                inverted: false,
            });
            let original = app.documents[0].snapshot();
            let mut time = 0.;
            app.begin_layer_masks().unwrap();
            app.layer_masks.draft.as_mut().unwrap().operation = operation;
            let frame = |app: &mut Drawer, time: &mut f64, events| {
                *time += 0.1;
                crate::input_replay::frame(&ctx, app, *time, events);
            };
            let click = |app: &mut Drawer, time: &mut f64, r: egui::Rect| {
                for pressed in [true, false] {
                    frame(
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
            for _ in 0..3 {
                frame(&mut app, &mut time, vec![]);
            }
            let r = app.layer_masks.preview.unwrap();
            click(&mut app, &mut time, r);
            assert!(
                app.layer_masks.draft.as_ref().unwrap().prepared.is_some(),
                "{:?}",
                app.status
            );
            assert_eq!(app.documents[0].snapshot(), original);
            for _ in 0..2 {
                frame(&mut app, &mut time, vec![]);
            }
            let r = app.layer_masks.apply.unwrap();
            click(&mut app, &mut time, r);
            assert!(app.layer_masks.draft.is_none(), "{:?}", app.status);
            assert_eq!(app.documents[0].history_counts(), (1, 0));
            assert_eq!(
                app.documents[0].layers[0].pixels[0],
                if operation == Operation::Apply {
                    [200, 100, 50, 64]
                } else {
                    [10, 20, 30, 64]
                }
            );
            assert_eq!(
                app.documents[0]
                    .scene
                    .appearance
                    .get(Target::Raster(1))
                    .is_some(),
                operation == Operation::Apply
            );
            assert_eq!(
                props::properties(&app.documents[0], Target::Raster(1)).opacity,
                50
            );
            assert!(app.documents[0].undo());
            assert_eq!(app.documents[0].snapshot(), original);
            app.begin_layer_masks().unwrap();
            app.layer_masks.draft.as_mut().unwrap().operation = operation;
            app.preview_layer_mask(&ctx).unwrap();
            app.documents[0].current_frame += 1;
            let changed = app.documents[0].snapshot();
            assert!(app.apply_layer_mask().is_err());
            assert_eq!(app.documents[0].snapshot(), changed);
        }
    }
    #[test]
    fn mask_ui_preview_apply_overlay_output_preferences_undo_stale_escape() {
        let root = crate::tests::TestDirectory::new();
        super::smoke(&root.0).unwrap();
    }
}
