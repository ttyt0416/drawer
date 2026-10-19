use super::*;
use drawer_core::{
    layer_catalog as catalog,
    layer_defaults::{self as core, Data, Fields},
    layer_search::Key,
};
#[derive(Default)]
pub(super) struct Panel {
    draft: Option<Box<Draft>>,
    undo: Vec<Data>,
    observed: Option<Data>,
    save: Option<egui::Rect>,
    reset: Option<egui::Rect>,
}
struct Draft {
    id: u64,
    key: Key,
    fields: Fields,
}
pub(super) fn apply(
    d: &Document,
    state: &mut drawer_core::sizing::DocumentState,
    data: &Data,
    used: usize,
) -> Result<(), String> {
    if !data.is_default()
        && used.saturating_add(state.memory_bytes().saturating_mul(4)) > DEFAULT_DOCUMENT_BUDGET
    {
        return Err("New layer defaults exceed workspace memory".into());
    }
    core::apply_new(d, state, data)
}
impl Drawer {
    pub(super) fn begin_layer_defaults(&mut self) -> Result<(), String> {
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
        let class = core::class(d, key)?;
        self.layer_defaults.draft = Some(Box::new(Draft {
            id: d.id,
            key,
            fields: Fields::supported(class),
        }));
        Ok(())
    }
    fn change_layer_defaults(&mut self, next: Data) {
        let before = self.workspaces.store.current.layer_defaults.clone();
        if before != next {
            self.layer_defaults.undo.push(before);
            if self.layer_defaults.undo.len() > 32 {
                self.layer_defaults.undo.remove(0);
            }
            self.layer_defaults.observed = Some(next.clone());
            self.workspaces.store.current.layer_defaults = next;
        }
    }
    pub(super) fn layer_defaults_window(&mut self, ctx: &egui::Context) {
        let Some(mut e) = self.layer_defaults.draft.take() else {
            return;
        };
        let Some(n) = self.active.filter(|n| self.documents[*n].id == e.id) else {
            return;
        };
        let d = &self.documents[n];
        let mut class = match core::class(d, e.key) {
            Ok(c) => c,
            Err(err) => {
                self.status = Some(err);
                return;
            }
        };
        if self.layer_defaults.observed.as_ref()
            != Some(&self.workspaces.store.current.layer_defaults)
        {
            self.layer_defaults.undo.clear();
            self.layer_defaults.observed =
                Some(self.workspaces.store.current.layer_defaults.clone());
        }
        let mut open = !ctx.input(|i| i.key_pressed(egui::Key::Escape));
        let mut save = false;
        let mut reset = false;
        let mut undo = false;
        egui::Window::new("New layer property defaults").open(&mut open).default_width(500.).collapsible(false).default_pos(egui::pos2(50.,25.)).show(ctx,|ui|{
            if let Ok(rows)=catalog::rows(d){egui::ComboBox::from_label("Applied layer properties").selected_text(rows.iter().find(|r|r.key==e.key).map_or("Missing layer",|r|r.name.as_str())).show_ui(ui,|ui|{egui::ScrollArea::vertical().max_height(180.).show_rows(ui,22.,rows.len(),|ui,range|{for i in range{let row=&rows[i];if ui.selectable_value(&mut e.key,row.key,&row.name).changed(){class=core::class(d,e.key).unwrap();e.fields=Fields::supported(class);}}});});}
            ui.label(format!("Defaults for new {} layers",class.label()));ui.add_enabled_ui(class.appearance(),|ui|{ui.checkbox(&mut e.fields.appearance,"Display color, border and watercolor edge");ui.checkbox(&mut e.fields.expression,"Expression color");});ui.add_enabled_ui(class.opacity(),|ui|{ui.checkbox(&mut e.fields.opacity,"Layer opacity");});ui.add_enabled_ui(class.blend(),|ui|{ui.checkbox(&mut e.fields.blend,"Layer blend mode");});ui.checkbox(&mut e.fields.color,"Identification color");
            ui.label(if self.workspaces.store.current.layer_defaults.get(class).is_some(){"This kind has saved defaults."}else{"This kind uses its standard defaults."});
            ui.horizontal(|ui|{let r=ui.button("Save selected defaults");self.layer_defaults.save=Some(r.rect);save=r.clicked();let r=ui.button("Reset this kind");self.layer_defaults.reset=Some(r.rect);reset=r.clicked();undo=ui.add_enabled(!self.layer_defaults.undo.is_empty(),egui::Button::new("Undo default change")).clicked();});
            ui.small("Only selected supported properties are copied into local settings. Existing layers, names, IDs, source files, pixels, masks, area selections and locks stay unchanged. New layers of this kind use the saved values; duplication and file opening retain their own properties.");
        });
        if save {
            let result = (|| {
                let entry = core::capture(d, e.key, e.fields)?;
                let mut next = self.workspaces.store.current.layer_defaults.clone();
                next.set(entry)?;
                Ok::<_, String>(next)
            })();
            match result {
                Ok(next) => self.change_layer_defaults(next),
                Err(err) => self.status = Some(err),
            }
        } else if reset {
            let mut next = self.workspaces.store.current.layer_defaults.clone();
            next.reset(class);
            self.change_layer_defaults(next);
        } else if undo && let Some(previous) = self.layer_defaults.undo.pop() {
            self.workspaces.store.current.layer_defaults = previous.clone();
            self.layer_defaults.observed = Some(previous);
        }
        if open {
            self.layer_defaults.draft = Some(e);
        }
    }
}
pub(super) fn smoke(root: &std::path::Path) -> Result<(), String> {
    drawer_core::layer_defaults_verify::verify(root)?;
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
    let s = drawer_core::appearance::prepare_expression(
        &app.documents[0],
        drawer_core::layer_properties::Target::Raster(1),
        drawer_core::appearance::Style {
            enabled: true,
            color: Some([40, 80, 120]),
            border: 1,
            ..Default::default()
        },
        Some(drawer_core::layer_search::Expression::Gray),
    )?;
    app.documents[0].commit_state(s)?;
    let s = drawer_core::layer_properties::prepare_patch(
        &app.documents[0],
        &[drawer_core::layer_properties::Target::Raster(1)],
        drawer_core::layer_properties::Patch {
            opacity: Some(42),
            ..Default::default()
        },
    )?;
    app.documents[0].commit_state(s)?;
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
    let before = app.documents[0].snapshot();
    let counts = app.documents[0].history_counts();
    app.execute_command(&ctx, "layer-defaults");
    step(&mut app, &mut time, vec![]);
    step(&mut app, &mut time, vec![]);
    let r = app
        .layer_defaults
        .save
        .ok_or("Save layer defaults missing")?;
    click(&mut app, &mut time, r);
    if app.workspaces.store.current.layer_defaults.entries.len() != 1
        || app.documents[0].snapshot() != before
        || app.documents[0].history_counts() != counts
    {
        return Err("Saving defaults changed existing document".into());
    }
    app.layer_defaults.draft = None;
    app.layer_quick_command("new-raster-layer")?;
    let d = &app.documents[0];
    let t = drawer_core::layer_properties::Target::Raster(d.scene.layer_ids.id(1));
    if drawer_core::layer_properties::properties(d, t).opacity != 42
        || drawer_core::appearance::expression(d, t) != drawer_core::layer_search::Expression::Gray
        || d.scene.appearance.get(t).is_none()
    {
        return Err("New raster command ignored saved defaults".into());
    }
    if !app.documents[0].undo() || app.documents[0].snapshot() != before {
        return Err("New layer/defaults Undo".into());
    }
    app.panels.targets.insert(
        app.documents[0].id,
        drawer_core::thumbnail::Target::Raster(0),
    );
    let s = drawer_core::layer_properties::prepare_patch(
        &app.documents[0],
        &[drawer_core::layer_properties::Target::Raster(1)],
        drawer_core::layer_properties::Patch {
            opacity: Some(88),
            ..Default::default()
        },
    )?;
    app.documents[0].commit_state(s)?;
    app.layer_quick_command("duplicate-layer")?;
    let d = &app.documents[0];
    let t = drawer_core::layer_properties::Target::Raster(d.scene.layer_ids.id(1));
    if drawer_core::layer_properties::properties(d, t).opacity != 88 {
        return Err("Creation defaults overwrote duplicate properties".into());
    }
    let settings = app.capture_workspace();
    let mut restart = Drawer::new(&ctx, None);
    restart.workspaces.store.current = settings.clone();
    restart.restore_workspace(&settings);
    restart
        .create_canvas(CanvasSpec {
            width: 8,
            height: 8,
            dpi: 300.,
            background: Background::Transparent,
        })
        .map_err(|e| e.to_string())?;
    if drawer_core::layer_properties::properties(
        &restart.documents[0],
        drawer_core::layer_properties::Target::Raster(1),
    )
    .opacity
        != 42
        || restart.documents[0].history_counts() != (0, 0)
    {
        return Err("New document/restart ignored layer defaults".into());
    }
    app.execute_command(&ctx, "layer-defaults");
    step(&mut app, &mut time, vec![]);
    step(&mut app, &mut time, vec![]);
    let before = app.documents[0].snapshot();
    let r = app
        .layer_defaults
        .reset
        .ok_or("Reset type defaults missing")?;
    click(&mut app, &mut time, r);
    if !app.workspaces.store.current.layer_defaults.is_default()
        || app.documents[0].snapshot() != before
    {
        return Err("Reset type defaults changed document or failed".into());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn actual_default_save_new_layer_duplicate_reset_restart_and_document() {
        let root = crate::tests::TestDirectory::new();
        super::smoke(&root.0).unwrap();
    }
}
