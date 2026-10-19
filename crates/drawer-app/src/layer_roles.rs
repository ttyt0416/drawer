use super::*;
use drawer_core::{
    layer_catalog as catalog,
    layer_roles::{self as core, Flags},
    layer_search::Key,
};
#[derive(Default)]
pub(super) struct Panel {
    draft: Option<Box<Draft>>,
    apply: Option<egui::Rect>,
    reference: Option<egui::Rect>,
    draft_flag: Option<egui::Rect>,
    output: Option<egui::Rect>,
}
struct Draft {
    id: u64,
    revision: u64,
    frame: u32,
    key: Key,
    flags: Flags,
    include_drafts: bool,
}
impl Drawer {
    pub(super) fn begin_layer_roles(&mut self) -> Result<(), String> {
        let d = &self.documents[self.active.ok_or("Open a page first")?];
        let key = catalog::from_thumbnail(
            d,
            self.panels
                .targets
                .get(&d.id)
                .copied()
                .unwrap_or(drawer_core::thumbnail::Target::Raster(0)),
        )
        .ok_or("Select a layer")?;
        self.layer_roles.draft = Some(Box::new(Draft {
            id: d.id,
            revision: d.revision,
            frame: d.current_frame,
            key,
            flags: core::own(d, catalog::target(d, key).ok_or("Missing role source")?),
            include_drafts: d.scene.include_drafts_in_output,
        }));
        Ok(())
    }
    fn apply_layer_roles(&mut self) -> Result<(), String> {
        self.finish_stroke();
        let e = self
            .layer_roles
            .draft
            .as_ref()
            .ok_or("Roles draft closed")?;
        let n = self.active.ok_or("Open a page first")?;
        let d = &self.documents[n];
        if (d.id, d.revision, d.current_frame) != (e.id, e.revision, e.frame) {
            return Err("Page/frame changed; reopen layer roles".into());
        }
        let state = core::prepare(d, e.key, e.flags, Some(e.include_drafts))?;
        let used: usize = self.documents.iter().map(Document::resident_bytes).sum();
        if used.saturating_add(state.memory_bytes().saturating_mul(2)) > DEFAULT_DOCUMENT_BUDGET {
            return Err("Layer role change exceeds workspace memory".into());
        }
        self.documents[n].commit_state(state)?;
        self.layer_roles.draft = None;
        Ok(())
    }
    pub(super) fn layer_roles_window(&mut self, ctx: &egui::Context) {
        let Some(mut e) = self.layer_roles.draft.take() else {
            return;
        };
        let Some(n) = self.active.filter(|n| self.documents[*n].id == e.id) else {
            return;
        };
        let d = &self.documents[n];
        let fresh = (d.revision, d.current_frame) == (e.revision, e.frame);
        let mut open = !ctx.input(|i| i.key_pressed(egui::Key::Escape));
        let mut apply = false;
        let mut cancel = false;
        egui::Window::new("Reference and draft layer roles").open(&mut open).default_width(480.).collapsible(false).default_pos(egui::pos2(45.,25.)).show(ctx,|ui|{
            if !fresh{ui.label("Page/frame changed; cancel and reopen roles.");return;}
            if let Ok(rows)=catalog::rows(d){egui::ComboBox::from_label("Layer").selected_text(rows.iter().find(|r|r.key==e.key).map_or("Missing layer",|r|r.name.as_str())).show_ui(ui,|ui|{for row in rows{if ui.selectable_value(&mut e.key,row.key,&row.name).changed(){e.flags=core::own(d,catalog::target(d,e.key).unwrap());}}});}
            let r=ui.checkbox(&mut e.flags.reference,"Reference for fill / color selection");self.layer_roles.reference=Some(r.rect);
            let r=ui.checkbox(&mut e.flags.draft,"Draft layer");self.layer_roles.draft_flag=Some(r.rect);
            ui.small("Folder and animation-folder flags apply to descendants. Original content and the canvas stay unchanged. A child cannot override an inherited flag.");
            let r=ui.checkbox(&mut e.include_drafts,"Include draft layers in rendered output");self.layer_roles.output=Some(r.rect);
            ui.small("PNG and rendered-image PSD follow this page setting. Normal .drawer saves always keep draft content and flags. Fill and wand use their own reference, hidden-layer and draft-exclusion settings.");
            ui.horizontal(|ui|{let r=ui.button("Apply layer roles");self.layer_roles.apply=Some(r.rect);apply=r.clicked();cancel=ui.button("Cancel").clicked();});
        });
        if open && !cancel {
            self.layer_roles.draft = Some(e);
        }
        if apply && let Err(e) = self.apply_layer_roles() {
            self.status = Some(e);
        }
    }
}
pub(super) fn smoke(root: &std::path::Path) -> Result<(), String> {
    drawer_core::layer_roles_verify::verify(root)?;
    reference_input()?;
    let ctx = egui::Context::default();
    let mut app = Drawer::new(&ctx, None);
    app.simple_view = true;
    app.create_canvas(CanvasSpec {
        width: 8,
        height: 8,
        dpi: 300.,
        background: Background::Transparent,
    })
    .map_err(|e| e.to_string())?;
    app.documents[0].layers[0].pixels.fill([200, 40, 20, 128]);
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
    let original = app.documents[0].snapshot();
    let history = app.documents[0].history_counts();
    app.execute_command(&ctx, "layer-roles");
    step(&mut app, &mut time, vec![]);
    step(&mut app, &mut time, vec![]);
    let r = app
        .layer_roles
        .reference
        .ok_or("Reference checkbox missing")?;
    click(&mut app, &mut time, r);
    let r = app.layer_roles.draft_flag.ok_or("Draft checkbox missing")?;
    click(&mut app, &mut time, r);
    if app.documents[0].snapshot() != original || app.documents[0].history_counts() != history {
        return Err("Role dialog controls edited source before Apply".into());
    }
    let r = app.layer_roles.apply.ok_or("Role Apply missing")?;
    click(&mut app, &mut time, r);
    let key = Key::Layer(drawer_core::layer_properties::Target::Raster(1));
    let flags = core::own(
        &app.documents[0],
        catalog::target(&app.documents[0], key).unwrap(),
    );
    if app.layer_roles.draft.is_some()
        || flags
            != (Flags {
                reference: true,
                draft: true,
            })
        || drawer_core::render::Renderer::new(&app.documents[0]).pixel([3.5; 2])
            != [200, 40, 20, 128]
        || drawer_core::render::Renderer::for_export(&app.documents[0])?.pixel([3.5; 2]) != [0; 4]
    {
        return Err(format!("Role Apply failed: {flags:?}, {:?}", app.status));
    }
    let after = app.documents[0].snapshot();
    if !app.documents[0].undo()
        || app.documents[0].snapshot() != original
        || !app.documents[0].redo()
        || app.documents[0].snapshot() != after
    {
        return Err("Role Apply Undo/Redo".into());
    }
    app.execute_command(&ctx, "layer-roles");
    step(&mut app, &mut time, vec![]);
    step(&mut app, &mut time, vec![]);
    let r = app
        .layer_roles
        .output
        .ok_or("Output policy checkbox missing")?;
    click(&mut app, &mut time, r);
    let r = app.layer_roles.apply.unwrap();
    click(&mut app, &mut time, r);
    if !app.documents[0].scene.include_drafts_in_output
        || drawer_core::render::Renderer::for_export(&app.documents[0])?.pixel([3.5; 2])
            != [200, 40, 20, 128]
    {
        return Err("Role output inclusion control failed".into());
    }
    let file = root.join("roles-ui.drawer");
    drawer_core::files::save(&mut app.documents[0], &file).map_err(|e| e.to_string())?;
    if drawer_core::files::open(&file, DEFAULT_DOCUMENT_BUDGET)
        .map_err(|e| e.to_string())?
        .snapshot()
        != app.documents[0].snapshot()
    {
        return Err("Role UI native reopen".into());
    }
    let saved = app.documents[0].snapshot();
    app.execute_command(&ctx, "layer-roles");
    app.documents[0].current_frame = 1;
    if app.apply_layer_roles().is_ok() {
        return Err("Stale role frame accepted".into());
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
    if app.layer_roles.draft.is_some() || app.documents[0].snapshot() != saved {
        return Err("Role Escape changed source".into());
    }
    Ok(())
}
fn reference_input() -> Result<(), String> {
    use drawer_core::{
        bucket::{Reference, Settings},
        pointer_input::{Kind, Phase, Source},
    };
    for pen in [false, true] {
        for wand in [false, true] {
            for hidden in [false, true] {
                let ctx = egui::Context::default();
                let mut app = Drawer::new(&ctx, None);
                app.simple_view = true;
                let mut d = drawer_core::layer_roles_verify::wall()?;
                d.view.zoom = 8.;
                d.view.angle = 90.;
                app.add_document(d);
                app.choose_color([200, 40, 20, 255]);
                let s = Settings {
                    reference: Reference::Marked,
                    include_hidden: hidden,
                    antialias: false,
                    ..Default::default()
                };
                app.workspaces.store.current.input.fills[0] = s;
                app.workspaces.store.current.selection_edit.region = s;
                let mut time = 0.;
                let step = |app: &mut Drawer, time: &mut f64| {
                    *time += 0.1;
                    super::input_replay::frame(&ctx, app, *time, vec![]);
                };
                step(&mut app, &mut time);
                step(&mut app, &mut time);
                app.execute_command(&ctx, if wand { "wand-tool" } else { "bucket-tool" });
                let before = app.documents[0].snapshot();
                let counts = app.documents[0].history_counts();
                for phase in [Phase::Down, Phase::Up] {
                    let mut e = super::precision::pointer(&ctx, &app, [3.5; 2], phase);
                    if pen {
                        e.source = Source {
                            kind: Kind::Pen,
                            device: 124,
                            contact: 1,
                        };
                        e.pressure = Some(0.5);
                    }
                    app.drawing.virtual_events(vec![e], pen);
                    step(&mut app, &mut time);
                    if phase == Phase::Down && app.documents[0].snapshot() != before {
                        return Err("Hidden reference preview changed source".into());
                    }
                }
                let expected = if hidden { 0 } else { 255 };
                let d = &app.documents[0];
                let outside = if wand {
                    d.scene.selection_coverage([0.5; 2])
                } else {
                    d.layers[0].pixels[0][3]
                };
                let inside = if wand {
                    d.scene.selection_coverage([3.5; 2])
                } else {
                    d.layers[0].pixels[3 * 8 + 3][3]
                };
                if outside != expected
                    || inside != 255
                    || d.layers[1].pixels != before.layers[1].pixels
                    || !drawer_core::layer_properties::properties(
                        d,
                        drawer_core::layer_properties::Target::Raster(2),
                    )
                    .hidden
                    || d.history_counts().0 != counts.0 + 1
                {
                    return Err(format!(
                        "Hidden reference input {pen}/{wand}/{hidden}: {outside}/{inside}, {:?}",
                        app.status
                    ));
                }
                if !app.documents[0].undo() || app.documents[0].snapshot() != before {
                    return Err("Hidden reference input Undo".into());
                }
            }
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn actual_role_checkboxes_apply_output_undo_and_native() {
        let root = crate::tests::TestDirectory::new();
        super::smoke(&root.0).unwrap();
    }
}
