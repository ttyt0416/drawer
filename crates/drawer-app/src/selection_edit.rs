use super::super::precision::Mode;
use super::*;
use drawer_core::selection_edit::{self as core, Brush, Combine, Settings};
pub(super) struct Gesture {
    pub document: u64,
    pub view: u64,
    pub projection: Projection,
    revision: u64,
    frame: u32,
    root: egui::Rect,
    source: drawer_core::pointer_input::Source,
    mode: Mode,
    settings: Settings,
    brush: Option<Brush>,
    preview: Document,
    version: u64,
    layer: Option<usize>,
}
impl Drawing {
    pub(crate) fn selection_edit_preview(&self, id: u64) -> Option<&Document> {
        self.selection_edit
            .as_ref()
            .filter(|s| s.document == id)
            .map(|s| &s.preview)
    }
    #[allow(clippy::too_many_arguments)]
    pub(super) fn begin_selection_edit(
        &mut self,
        doc: &Document,
        view: u64,
        layer: Option<usize>,
        source: drawer_core::pointer_input::Source,
        projection: Projection,
        root: egui::Rect,
        point: Point,
        pressure: f64,
    ) -> Result<(), String> {
        if doc.memory_bytes() > DEFAULT_DOCUMENT_BUDGET / 8
            || doc
                .memory_bytes()
                .saturating_mul(3)
                .saturating_add(64 * 1024 * 1024)
                > self.fill_budget
        {
            return Err("Selection preview exceeds workspace memory budget".into());
        }
        let settings = self.selection_settings;
        let mut brush = if self.mode == Mode::SelectionBrush {
            Some(Brush::begin(doc, settings)?)
        } else {
            None
        };
        let scene = if let Some(b) = &mut brush {
            b.sample(doc, point, pressure)?;
            b.preview(doc)?
        } else {
            if settings.region.reference == drawer_core::bucket::Reference::Active
                && layer.is_none()
            {
                return Err("Select a raster reference or use the composite".into());
            }
            core::wand(doc, layer.unwrap_or(0), point, settings)?
        };
        let version = brush.as_ref().map_or(0, Brush::version);
        let mut state = doc.snapshot();
        state.scene = scene;
        self.selection_edit = Some(Gesture {
            document: doc.id,
            view,
            projection,
            revision: doc.revision,
            frame: doc.current_frame,
            root,
            source,
            mode: self.mode,
            settings,
            brush,
            preview: super::bucket::preview(doc, state)?,
            version,
            layer,
        });
        Ok(())
    }
    pub(super) fn receive_selection_edit(&mut self, index: usize, doc: &mut Document) {
        use drawer_core::pointer_input::Phase;
        if self.events[index].1 {
            return;
        }
        let drawer_core::touch::Effect::Pointer(e) = self.events[index].0 else {
            return;
        };
        let Some(mut s) = self.selection_edit.take() else {
            return;
        };
        if s.document != doc.id || s.source != e.source {
            self.selection_edit = Some(s);
            return;
        }
        self.events[index].1 = true;
        if s.revision != doc.revision
            || s.frame != doc.current_frame
            || s.projection.view != doc.view
            || s.mode != self.mode
            || s.settings != self.selection_settings
            || s.layer != self.move_layer
            || e.phase == Phase::Cancel
        {
            self.ownership.clear();
            return;
        }
        if let Some(b) = &mut s.brush
            && matches!(e.phase, Phase::Move | Phase::Up)
        {
            let p = s
                .projection
                .point(egui::pos2(e.position[0] as f32, e.position[1] as f32));
            if let Err(error) = b.sample(doc, p, e.pressure.unwrap_or(1.0)) {
                self.error = Some(error);
                self.ownership.clear();
                return;
            }
        }
        if e.phase == Phase::Up {
            self.ownership.clear();
            let result = if let Some(b) = s.brush {
                b.preview(doc)
            } else {
                Ok(s.preview.scene)
            }
            .and_then(|scene| doc.edit_scene(scene));
            if let Err(e) = result {
                self.error = Some(e);
            }
            return;
        }
        self.selection_edit = Some(s);
    }
    pub(crate) fn refresh_selection_edit(&mut self, doc: &Document) {
        let Some(s) = self
            .selection_edit
            .as_mut()
            .filter(|s| s.document == doc.id)
        else {
            return;
        };
        let Some(b) = &s.brush else {
            return;
        };
        if s.version == b.version() {
            return;
        }
        match b.preview(doc) {
            Ok(scene) => {
                s.preview.scene = scene;
                s.preview.revision += 1;
                s.version = b.version();
            }
            Err(error) => {
                self.error = Some(error);
                self.cancel();
            }
        }
    }
    pub(super) fn advance_selection_edit(&mut self, ctx: &egui::Context, docs: &mut [Document]) {
        if self.selection_edit.as_ref().is_some_and(|s| {
            s.root != ctx.content_rect()
                || (s.projection.ppp - ctx.pixels_per_point() as f64).abs() > 1e-8
                || s.mode != self.mode
                || s.settings != self.selection_settings
                || s.layer != self.move_layer
                || docs.iter().find(|d| d.id == s.document).is_none_or(|d| {
                    d.revision != s.revision
                        || d.current_frame != s.frame
                        || d.view != s.projection.view
                })
        }) {
            self.cancel();
            return;
        }
        for n in 0..self.events.len() {
            let Some(id) = self.selection_edit.as_ref().map(|s| s.document) else {
                break;
            };
            let Some(doc) = docs.iter_mut().find(|d| d.id == id) else {
                self.cancel();
                break;
            };
            self.receive_selection_edit(n, doc);
        }
        if let Some(id) = self.selection_edit.as_ref().map(|s| s.document)
            && let Some(doc) = docs.iter().find(|d| d.id == id)
        {
            self.refresh_selection_edit(doc);
        }
    }
}
impl Drawer {
    pub(crate) fn selection_edit_controls(&mut self, ui: &mut egui::Ui) {
        ui.collapsing("Wand and selection brush",|ui|{
            let s=&mut self.workspaces.store.current.selection_edit;let before=*s;
            ui.checkbox(&mut s.connected,"Wand: connected area only");
            egui::ComboBox::from_id_salt("wand-combine").selected_text(format!("{:?}",s.combine)).show_ui(ui,|ui|{for op in Combine::ALL{ui.selectable_value(&mut s.combine,op,format!("{op:?}"));}});
            egui::ComboBox::from_id_salt("wand-reference").selected_text(format!("{:?}",s.region.reference)).show_ui(ui,|ui|{
                ui.selectable_value(&mut s.region.reference,drawer_core::bucket::Reference::Active,"Current raster");ui.selectable_value(&mut s.region.reference,drawer_core::bucket::Reference::Composite,"Visible composite");
                if let Some(n)=self.active{for(i,l)in self.documents[n].layers.iter().enumerate(){ui.selectable_value(&mut s.region.reference,drawer_core::bucket::Reference::Layer(i as u16),format!("Raster {}: {}",i+1,l.name));}}
            });
            ui.add(egui::Slider::new(&mut s.region.color_tolerance,0.0..=1.0).text("Color tolerance"));ui.add(egui::Slider::new(&mut s.region.alpha_tolerance,0.0..=1.0).text("Alpha tolerance"));ui.checkbox(&mut s.region.antialias,"Wand edge coverage");
            ui.add(egui::Slider::new(&mut s.diameter,0.5..=4096.0).logarithmic(true).text("Selection brush diameter"));ui.add(egui::Slider::new(&mut s.hardness,0.0..=1.0).text("Hardness"));ui.add(egui::Slider::new(&mut s.strength,0.0..=1.0).text("Strength"));ui.checkbox(&mut s.erase,"Erase selection strength");
            if *s!=before{self.finish_stroke();}
            ui.horizontal_wrapped(|ui|{for(c,l)in[("wand-tool","Wand (H)"),("selection-brush-tool","Add selection (J)"),("selection-erase-tool","Erase selection (Shift+J)")]{if ui.button(l).clicked(){self.execute_command(ui.ctx(),c);}}});
            ui.label("Wand and brush change selection only. Press to preview, release for one Undo; Escape/focus loss cancels. Brush pressure scales strength. Repeated samples in one stroke use maximum coverage.");
        });
    }
}

fn step(ctx: &egui::Context, app: &mut Drawer, time: &mut f64, events: Vec<egui::Event>) {
    *time += 0.1;
    super::super::input_replay::frame(ctx, app, *time, events);
}
fn key(key: egui::Key, pressed: bool, repeat: bool) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed,
        repeat,
        modifiers: Default::default(),
    }
}
pub(crate) fn smoke(root: &std::path::Path) -> Result<(), String> {
    use drawer_core::pointer_input::{Kind, Phase, Source};
    core::verify(root)?;
    for pen in [false, true] {
        let ctx = egui::Context::default();
        let mut app = Drawer::new(&ctx, None);
        app.simple_view = true;
        let mut time = 0.0;
        app.create_canvas(CanvasSpec {
            width: 16,
            height: 16,
            dpi: 300.0,
            background: Background::Transparent,
        })
        .map_err(|e| e.to_string())?;
        app.documents[0].layers[0].pixels.fill([0, 0, 0, 255]);
        for y in 0..16 {
            for x in (2..5).chain(10..13) {
                app.documents[0].layers[0].pixels[y * 16 + x] = [200, 0, 0, 255];
            }
        }
        app.documents[0].view.zoom = 1.5;
        app.documents[0].view.angle = 90.0;
        app.workspaces.store.current.selection_edit.region.antialias = false;
        drawer_core::files::save(
            &mut app.documents[0],
            &root.join(format!("wand-base-{pen}.drawer")),
        )
        .map_err(|e| e.to_string())?;
        let before = app.documents[0].snapshot();
        step(&ctx, &mut app, &mut time, vec![]);
        step(&ctx, &mut app, &mut time, vec![]);
        let pointer = |app: &Drawer, p, phase| {
            let mut e = super::super::precision::pointer(&ctx, app, p, phase);
            if pen {
                e.source = Source {
                    kind: Kind::Pen,
                    device: 91,
                    contact: 2,
                };
                e.pressure = Some(0.5);
            }
            e
        };
        app.drawing
            .virtual_events(vec![pointer(&app, [3.5, 8.5], Phase::Down)], pen);
        step(
            &ctx,
            &mut app,
            &mut time,
            vec![key(egui::Key::H, true, false)],
        );
        if !app.drawing.pending()
            || app.documents[0].snapshot() != before
            || app.documents[0].dirty
            || app
                .drawing
                .selection_edit_preview(app.documents[0].id)
                .is_none()
        {
            return Err("Wand preview/prepared pen changed document".into());
        }
        step(
            &ctx,
            &mut app,
            &mut time,
            vec![key(egui::Key::H, true, true)],
        );
        if !app.drawing.pending() {
            return Err("Repeated wand key cancelled contact".into());
        }
        app.drawing
            .virtual_events(vec![pointer(&app, [3.5, 8.5], Phase::Up)], pen);
        step(
            &ctx,
            &mut app,
            &mut time,
            vec![key(egui::Key::H, false, false)],
        );
        if app.documents[0].scene.selection_coverage([3.5, 8.5]) != 255
            || app.documents[0].scene.selection_coverage([11.5, 8.5]) != 0
            || app.documents[0].layers != before.layers
            || app.documents[0].history_counts() != (1, 0)
        {
            return Err("Connected wand result/history".into());
        }
        app.documents[0].undo();
        app.workspaces.store.current.selection_edit.connected = false;
        app.execute_command(&ctx, "wand-tool");
        for phase in [Phase::Down, Phase::Up] {
            app.drawing
                .virtual_events(vec![pointer(&app, [3.5, 8.5], phase)], pen);
            step(&ctx, &mut app, &mut time, vec![]);
        }
        if app.documents[0].scene.selection_coverage([11.5, 8.5]) != 255
            || app.documents[0].scene.selection_coverage([7.5, 8.5]) != 0
        {
            return Err("Global wand result".into());
        }
        app.documents[0].undo();
        app.workspaces.store.current.selection_edit.diameter = 8.0;
        app.workspaces.store.current.selection_edit.hardness = 1.0;
        app.workspaces.store.current.selection_edit.strength = 0.5;
        app.execute_command(&ctx, "selection-brush-tool");
        for (p, phase) in [([8.0; 2], Phase::Down), ([12.0, 8.0], Phase::Move)] {
            app.drawing
                .virtual_events(vec![pointer(&app, p, phase)], pen);
            step(&ctx, &mut app, &mut time, vec![]);
        }
        let want = if pen { 64 } else { 128 };
        if app.documents[0].snapshot() != before
            || app
                .drawing
                .selection_edit_preview(app.documents[0].id)
                .is_none_or(|d| d.scene.selection_coverage([7.5; 2]) != want)
        {
            return Err("Selection brush preview strength/source".into());
        }
        app.drawing
            .virtual_events(vec![pointer(&app, [12.0, 8.0], Phase::Up)], pen);
        step(&ctx, &mut app, &mut time, vec![]);
        if app.documents[0].scene.selection_coverage([7.5; 2]) != want
            || app.documents[0].layers != before.layers
            || app.documents[0].history_counts() != (1, 0)
            || app.drawing.pending()
        {
            return Err("Selection brush commit/pixel isolation/ownership".into());
        }
        let added = app.documents[0].snapshot();
        app.workspaces.store.current.selection_edit.strength = 1.0;
        app.execute_command(&ctx, "selection-erase-tool");
        for phase in [Phase::Down, Phase::Up] {
            app.drawing
                .virtual_events(vec![pointer(&app, [8.0; 2], phase)], pen);
            step(&ctx, &mut app, &mut time, vec![]);
        }
        if app.documents[0].scene.selection_coverage([7.5; 2]) != if pen { 32 } else { 0 } {
            return Err("Selection brush erase pressure".into());
        }
        app.documents[0].undo();
        if app.documents[0].snapshot() != added {
            return Err("Selection brush erasure Undo".into());
        }
        let path = root.join(format!("selection-tools-{pen}.drawer"));
        drawer_core::files::save(&mut app.documents[0], &path).map_err(|e| e.to_string())?;
        if drawer_core::files::open(&path, DEFAULT_DOCUMENT_BUDGET)
            .map_err(|e| e.to_string())?
            .snapshot()
            != added
        {
            return Err("Selection tools native reopen".into());
        }
        app.documents[0].undo();
        if app.documents[0].snapshot() != before {
            return Err("Selection brush whole-stroke Undo".into());
        }
        app.workspaces.store.current.selection_edit.strength = 0.5;
        for cancel in 0..5 {
            app.documents[0].view.zoom = 1.5;
            app.execute_command(&ctx, "selection-brush-tool");
            app.drawing
                .virtual_events(vec![pointer(&app, [8.0; 2], Phase::Down)], pen);
            step(&ctx, &mut app, &mut time, vec![]);
            let events = match cancel {
                0 => vec![key(egui::Key::Escape, true, false)],
                1 => vec![egui::Event::WindowFocused(false)],
                2 => {
                    app.documents[0].view.zoom = 2.0;
                    vec![]
                }
                3 => {
                    app.workspaces.store.current.selection_edit.strength = 0.25;
                    vec![]
                }
                _ => {
                    app.begin_paste(&ctx, || {
                        Ok(drawer_core::paste::ClipboardImage {
                            image: drawer_core::references::LocalImage {
                                width: 1,
                                height: 1,
                                pixels: vec![[0; 4]],
                            },
                            origin: None,
                        })
                    })?;
                    vec![]
                }
            };
            step(&ctx, &mut app, &mut time, events);
            if app.drawing.pending() || app.documents[0].snapshot() != before {
                return Err(format!("Selection edit cancel {cancel}"));
            }
            app.paste_dialog = None;
            app.workspaces.store.current.selection_edit.strength = 0.5;
            app.drawing
                .virtual_events(vec![pointer(&app, [8.0; 2], Phase::Up)], pen);
            step(
                &ctx,
                &mut app,
                &mut time,
                vec![
                    egui::Event::WindowFocused(true),
                    key(egui::Key::Escape, false, false),
                ],
            );
        }
        app.workspaces.store.current.selection_edit.strength = 0.0;
        app.execute_command(&ctx, "selection-brush-tool");
        let history = app.documents[0].history_counts();
        let dirty = app.documents[0].dirty;
        for phase in [Phase::Down, Phase::Up] {
            app.drawing
                .virtual_events(vec![pointer(&app, [8.0; 2], phase)], pen);
            step(&ctx, &mut app, &mut time, vec![]);
        }
        if app.documents[0].snapshot() != before
            || app.documents[0].history_counts() != history
            || app.documents[0].dirty != dirty
            || !app.workspaces.input_idle()
        {
            return Err("Zero-strength selection brush changed document".into());
        }
        let path = root.join(format!("selection-tool-settings-{pen}.json"));
        app.workspaces
            .store
            .current
            .export(&path)
            .map_err(|e| e.to_string())?;
        if drawer_core::workspaces::Snapshot::read(&path)
            .map_err(|e| e.to_string())?
            .selection_edit
            != app.workspaces.store.current.selection_edit
        {
            return Err("Selection edit settings restart".into());
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn wand_and_brush_mouse_pen_preview_pressure_cancel_undo_restart() {
        let root = crate::tests::TestDirectory::new();
        super::smoke(&root.0).unwrap();
    }
}

#[cfg(test)]
mod storage_budget {
    #[test]
    fn native_app_state_stays_below_96_kib_on_the_one_mib_main_stack() {
        assert!(
            std::mem::size_of::<crate::Drawer>() <= 96 * 1024,
            "Move growing runtime state to heap storage before extending Drawer"
        );
    }
}
