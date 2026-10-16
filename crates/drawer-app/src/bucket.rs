use super::*;
use drawer_core::bucket::{self as core, Mode, Reference, Settings};
pub(super) struct Gesture {
    pub document: u64,
    pub view: u64,
    pub projection: Projection,
    revision: u64,
    frame: u32,
    source: drawer_core::pointer_input::Source,
    root: egui::Rect,
    layer: usize,
    settings: Settings,
    preview: Document,
    config: ([u8; 4], bool, bool),
}
fn preview(doc: &Document, state: drawer_core::sizing::DocumentState) -> Result<Document, String> {
    let mut p = Document::create(
        CanvasSpec {
            width: 1,
            height: 1,
            dpi: state.dpi,
            background: Background::Transparent,
        },
        DEFAULT_DOCUMENT_BUDGET,
    )
    .map_err(|e| e.to_string())?;
    p.width = state.width;
    p.height = state.height;
    p.layers = state.layers;
    p.scene = state.scene;
    p.animation = state.animation;
    p.current_frame = doc.current_frame;
    p.view = doc.view;
    p.tool = doc.tool;
    Ok(p)
}
impl Drawing {
    pub(crate) fn bucket_preview(&self, id: u64) -> Option<&Document> {
        self.bucket
            .as_ref()
            .filter(|s| s.document == id)
            .map(|s| &s.preview)
    }
    #[allow(clippy::too_many_arguments)]
    pub(super) fn begin_bucket(
        &mut self,
        doc: &Document,
        view: u64,
        layer: Option<usize>,
        source: drawer_core::pointer_input::Source,
        projection: Projection,
        root: egui::Rect,
        point: Point,
        style: Style,
    ) -> Result<(), String> {
        let layer = layer.ok_or("Select a raster fill target")?;
        if doc.memory_bytes().saturating_mul(3) > self.fill_budget {
            return Err("Fill preview exceeds available workspace memory".into());
        }
        let state = core::prepare(
            doc,
            layer,
            Some(point),
            style.color,
            self.transparent_mode || style.profile.transparent,
            style.material.alpha_lock,
            self.fill_settings,
        )?;
        if state
            .memory_bytes()
            .saturating_mul(2)
            .saturating_add(doc.memory_bytes())
            > self.fill_budget
        {
            return Err("Fill preview exceeds available workspace memory".into());
        }
        self.bucket = Some(Gesture {
            document: doc.id,
            view,
            projection,
            revision: doc.revision,
            frame: doc.current_frame,
            source,
            root,
            layer,
            settings: self.fill_settings,
            config: (
                style.color,
                self.transparent_mode || style.profile.transparent,
                style.material.alpha_lock,
            ),
            preview: preview(doc, state)?,
        });
        Ok(())
    }
    pub(super) fn receive_bucket(&mut self, index: usize, doc: &mut Document) {
        use drawer_core::pointer_input::Phase;
        if self.events[index].1 {
            return;
        }
        let drawer_core::touch::Effect::Pointer(e) = self.events[index].0 else {
            return;
        };
        let Some(s) = self.bucket.take() else {
            return;
        };
        if s.document != doc.id || s.source != e.source {
            self.bucket = Some(s);
            return;
        }
        self.events[index].1 = true;
        if s.revision != doc.revision
            || s.frame != doc.current_frame
            || s.projection.view != doc.view
            || self.mode != super::super::precision::Mode::Fill
            || s.settings != self.fill_settings
            || s.config != self.fill_config
            || Some(s.layer) != self.move_layer
            || e.phase == Phase::Cancel
        {
            self.ownership.clear();
            return;
        }
        if e.phase == Phase::Up {
            self.ownership.clear();
            let p = s
                .projection
                .point(egui::pos2(e.position[0] as f32, e.position[1] as f32));
            if p[0] >= 0.0
                && p[1] >= 0.0
                && p[0] < doc.width as f64
                && p[1] < doc.height as f64
                && let Err(error) = doc.commit_state(s.preview.snapshot())
            {
                self.error = Some(error);
            }
            return;
        }
        self.bucket = Some(s);
    }
    pub(super) fn advance_bucket(&mut self, ctx: &egui::Context, docs: &mut [Document]) {
        if self.bucket.as_ref().is_some_and(|s| {
            s.root != ctx.content_rect()
                || (s.projection.ppp - ctx.pixels_per_point() as f64).abs() > 1e-8
                || s.settings != self.fill_settings
                || s.config != self.fill_config
                || Some(s.layer) != self.move_layer
                || self.mode != super::super::precision::Mode::Fill
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
            let Some(id) = self.bucket.as_ref().map(|s| s.document) else {
                break;
            };
            let Some(doc) = docs.iter_mut().find(|d| d.id == id) else {
                self.cancel();
                break;
            };
            self.receive_bucket(n, doc);
        }
    }
}
impl Drawer {
    pub(crate) fn fill_selection(&mut self) -> Result<(), String> {
        let n = self.active.ok_or("Open a page first")?;
        let id = self.documents[n].id;
        let drawer_core::thumbnail::Target::Raster(layer) = self
            .panels
            .targets
            .get(&id)
            .copied()
            .unwrap_or(drawer_core::thumbnail::Target::Raster(0))
        else {
            return Err("Select a raster fill target".into());
        };
        let slot = BrushTip::ALL
            .iter()
            .position(|t| *t == self.selected)
            .unwrap();
        let settings = self.workspaces.store.current.input.fills[slot];
        let available = DEFAULT_DOCUMENT_BUDGET
            .saturating_sub(self.documents.iter().map(Document::resident_bytes).sum());
        if self.documents[n].memory_bytes().saturating_mul(3) > available {
            return Err("Selection fill exceeds available workspace memory".into());
        }
        let state = core::prepare(
            &self.documents[n],
            layer,
            None,
            self.references.session.workspace.color,
            self.workspaces.store.current.color_modes.transparent
                || self.workspaces.store.current.input.profiles[slot].transparent,
            self.workspaces.store.current.input.materials[slot].alpha_lock,
            settings,
        )?;
        if state
            .memory_bytes()
            .saturating_mul(2)
            .saturating_add(self.documents[n].memory_bytes())
            > available
        {
            return Err("Selection fill exceeds available workspace memory".into());
        }
        self.documents[n].commit_state(state)?;
        Ok(())
    }
    pub(crate) fn bucket_controls(&mut self, ui: &mut egui::Ui) {
        ui.collapsing("Fill options",|ui|{
            let slot=BrushTip::ALL.iter().position(|t|*t==self.selected).unwrap();
            let s=&mut self.workspaces.store.current.input.fills[slot];let before=*s;
            ui.horizontal(|ui|{ui.selectable_value(&mut s.mode,Mode::Strict,"Strict");ui.selectable_value(&mut s.mode,Mode::CloseGaps,"Close gaps");});
            egui::ComboBox::from_id_salt("bucket-reference").selected_text(format!("{:?}",s.reference)).show_ui(ui,|ui|{
                ui.selectable_value(&mut s.reference,Reference::Active,"Current raster");ui.selectable_value(&mut s.reference,Reference::Composite,"Visible composite");
                if let Some(n)=self.active{for(i,l)in self.documents[n].layers.iter().enumerate(){ui.selectable_value(&mut s.reference,Reference::Layer(i as u16),format!("Raster {}: {}",i+1,l.name));}}
            });
            ui.add(egui::Slider::new(&mut s.color_tolerance,0.0..=1.0).text("Color tolerance"));ui.add(egui::Slider::new(&mut s.alpha_tolerance,0.0..=1.0).text("Alpha tolerance"));
            ui.add_enabled_ui(s.mode==Mode::CloseGaps,|ui|{ui.add(egui::Slider::new(&mut s.gap,0..=32).text("Maximum gap pixels"));});
            ui.add(egui::Slider::new(&mut s.expand,0..=32).text("Expand pixels"));ui.checkbox(&mut s.antialias,"Antialias partial-alpha edges");
            if *s!=before{self.finish_stroke();}
            ui.checkbox(&mut self.workspaces.store.current.input.materials[slot].alpha_lock,"Protect target alpha");
            ui.horizontal(|ui|{for(c,label)in[("bucket-tool","Fill tool (G)"),("fill-selection","Fill entire selection")]{if ui.button(label).clicked(){self.execute_command(ui.ctx(),c);}}});
            ui.label("Press to preview; release inside the canvas to fill once. Escape/focus loss cancels. Alpha/color thresholds, gap closure, expansion and edge coverage are independent. Options are stored per tool preset.");
        });
    }
}

fn step(ctx: &egui::Context, app: &mut Drawer, time: &mut f64, events: Vec<egui::Event>) {
    *time += 0.1;
    super::super::input_replay::frame(ctx, app, *time, events);
}
fn key(pressed: bool, repeat: bool) -> egui::Event {
    egui::Event::Key {
        key: egui::Key::G,
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
            height: 12,
            dpi: 300.0,
            background: Background::Transparent,
        })
        .map_err(|e| e.to_string())?;
        let mut lines = app.documents[0].layers[0].clone();
        lines.name = "Reference".into();
        for y in 2..10 {
            for x in 3..13 {
                if x == 3 || x == 12 || y == 2 || y == 9 {
                    lines.pixels[y * 16 + x] = [0, 0, 0, 255];
                }
            }
        }
        lines.pixels[2 * 16 + 7] = [0; 4];
        app.documents[0].layers.push(lines);
        app.workspaces.store.current.input.fills[0] = Settings {
            mode: Mode::CloseGaps,
            reference: Reference::Layer(1),
            gap: 1,
            antialias: false,
            ..Default::default()
        };
        app.choose_color([70, 90, 110, 128]);
        app.documents[0].view.zoom = 1.5;
        app.documents[0].view.angle = 90.0;
        drawer_core::files::save(
            &mut app.documents[0],
            &root.join(format!("bucket-base-{pen}.drawer")),
        )
        .map_err(|e| e.to_string())?;
        step(&ctx, &mut app, &mut time, vec![]);
        step(&ctx, &mut app, &mut time, vec![]);
        let before = app.documents[0].snapshot();
        let pointer = |app: &Drawer, p, phase| {
            let mut e = super::super::precision::pointer(&ctx, app, p, phase);
            if pen {
                e.source = Source {
                    kind: Kind::Pen,
                    device: 81,
                    contact: 4,
                };
                e.pressure = Some(0.5);
            }
            e
        };
        app.drawing
            .virtual_events(vec![pointer(&app, [7.5, 5.5], Phase::Down)], pen);
        step(&ctx, &mut app, &mut time, vec![key(true, false)]);
        if !app.drawing.pending()
            || app.documents[0].snapshot() != before
            || app.documents[0].dirty
            || app
                .drawing
                .bucket_preview(app.documents[0].id)
                .is_none_or(|p| p.layers[0].pixel(16, 12, 7, 5) != [70, 90, 110, 128])
        {
            return Err("Bucket preview/prepared contact changed source".into());
        }
        step(&ctx, &mut app, &mut time, vec![key(true, true)]);
        if !app.drawing.pending() {
            return Err("Repeated fill shortcut cancelled pen contact".into());
        }
        app.drawing
            .virtual_events(vec![pointer(&app, [7.5, 5.5], Phase::Up)], pen);
        step(&ctx, &mut app, &mut time, vec![key(false, false)]);
        let d = &mut app.documents[0];
        if d.layers[0].pixel(16, 12, 7, 5) != [70, 90, 110, 128]
            || d.layers[0].pixel(16, 12, 0, 0)[3] != 0
            || d.layers[1] != before.layers[1]
            || d.history_counts() != (1, 0)
        {
            return Err("Bucket commit/ref/one undo".into());
        }
        let after = d.snapshot();
        d.undo();
        if d.snapshot() != before || d.dirty {
            return Err("Bucket Undo".into());
        }
        d.redo();
        if d.snapshot() != after {
            return Err("Bucket Redo".into());
        }
        d.undo();
        if app.drawing.pending() || !app.workspaces.input_idle() {
            return Err("Bucket ownership not released".into());
        }
        for cancel in 0..6 {
            app.documents[0].view.zoom = 1.5;
            app.execute_command(&ctx, "bucket-tool");
            app.drawing
                .virtual_events(vec![pointer(&app, [7.5, 5.5], Phase::Down)], pen);
            step(&ctx, &mut app, &mut time, vec![]);
            if !app.drawing.pending() {
                return Err(format!("Missing bucket cancel fixture {cancel}"));
            }
            let events = match cancel {
                0 => vec![egui::Event::Key {
                    key: egui::Key::Escape,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: Default::default(),
                }],
                1 => vec![egui::Event::WindowFocused(false)],
                2 => {
                    app.documents[0].view.zoom = 2.0;
                    vec![]
                }
                3 => {
                    app.drawing
                        .virtual_events(vec![pointer(&app, [-5.0, -5.0], Phase::Up)], pen);
                    vec![]
                }
                4 => {
                    app.workspaces.store.current.input.fills[0].reference = Reference::Active;
                    vec![]
                }
                _ => {
                    app.drawing
                        .virtual_events(vec![pointer(&app, [7.5, 5.5], Phase::Cancel)], pen);
                    vec![]
                }
            };
            step(&ctx, &mut app, &mut time, events);
            if app.drawing.pending() || app.documents[0].snapshot() != before {
                return Err(format!("Bucket cancellation {cancel}"));
            }
            // The physical contact must end after cancellation before another Down.
            app.drawing
                .virtual_events(vec![pointer(&app, [7.5, 5.5], Phase::Up)], pen);
            app.workspaces.store.current.input.fills[0].reference = Reference::Layer(1);
            step(
                &ctx,
                &mut app,
                &mut time,
                vec![
                    egui::Event::WindowFocused(true),
                    egui::Event::Key {
                        key: egui::Key::Escape,
                        physical_key: None,
                        pressed: false,
                        repeat: false,
                        modifiers: Default::default(),
                    },
                ],
            );
        }
        app.documents[0].scene.selection_mask = Some(drawer_core::selection::Mask {
            runs: vec![drawer_core::selection::Run {
                x: 1,
                y: 1,
                length: 2,
                alpha: 128,
            }],
        });
        let selected = app.documents[0].snapshot();
        app.execute_command(&ctx, "fill-selection");
        if app.documents[0].layers[0].pixel(16, 12, 1, 1) != [70, 90, 110, 64]
            || app.documents[0].layers[0].pixel(16, 12, 3, 1)[3] != 0
            || app.documents[0].history_counts() != (1, 0)
        {
            return Err("Whole selection fill did not bypass reference/seed".into());
        }
        app.documents[0].undo();
        if app.documents[0].snapshot() != selected {
            return Err("Whole selection fill Undo".into());
        }
        let p = root.join(format!("bucket-settings-{pen}.json"));
        app.workspaces
            .store
            .current
            .export(&p)
            .map_err(|e| e.to_string())?;
        if drawer_core::workspaces::Snapshot::read(&p)
            .map_err(|e| e.to_string())?
            .input
            .fills
            != app.workspaces.store.current.input.fills
        {
            return Err("Fill settings reopen".into());
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn mouse_pen_preview_release_cancel_references_selected_fill_and_restart() {
        let root = crate::tests::TestDirectory::new();
        super::smoke(&root.0).unwrap();
    }
}
