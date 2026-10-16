use super::*;
use drawer_core::{
    paper::{Graph, Node},
    texture::Pattern,
};
#[derive(Default)]
pub(super) struct Textures {
    pub open: bool,
    editor: Option<Editor>,
    pub export: Option<Export>,
    #[cfg(test)]
    buttons: [Option<egui::Rect>; 2],
}
struct Editor {
    id: u64,
    revision: u64,
    draft: Graph,
    node: Node,
}
pub(super) struct Export {
    id: u64,
    revision: u64,
    path: PathBuf,
    replace: bool,
}
impl Textures {
    pub fn modal(&self) -> bool {
        self.open || self.export.is_some()
    }
}
pub(super) fn pattern_ui(ui: &mut egui::Ui, p: &mut Pattern) {
    pattern_ui_inner(ui, p, None);
}
pub(super) fn brush_pattern_ui(
    ui: &mut egui::Ui,
    p: &mut Pattern,
    library: &drawer_core::material_library::Library,
) {
    pattern_ui_inner(ui, p, Some(library));
}
fn pattern_ui_inner(
    ui: &mut egui::Ui,
    p: &mut Pattern,
    library: Option<&drawer_core::material_library::Library>,
) {
    egui::ComboBox::from_id_salt("texture-source")
        .selected_text(
            library
                .and_then(|l| l.label(p.source, drawer_core::material_library::Kind::Texture))
                .unwrap_or_else(|| p.label()),
        )
        .show_ui(ui, |ui| {
            for n in 0..=3 {
                let candidate = Pattern { source: n, ..*p };
                ui.selectable_value(&mut p.source, n, candidate.label());
            }
            if let Some(library) = library {
                for a in library
                    .assets()
                    .iter()
                    .filter(|a| a.kind == drawer_core::material_library::Kind::Texture)
                {
                    ui.selectable_value(&mut p.source, a.id, format!("{} (#{})", a.name, a.id));
                }
            }
        });
    if p.missing()
        && library.is_none_or(|l| {
            l.get(p.source, drawer_core::material_library::Kind::Texture)
                .is_none()
        })
    {
        ui.colored_label(egui::Color32::YELLOW, p.label());
    }
    super::edit::number(
        ui,
        &mut p.scale,
        0.01..=1024.0,
        0.1,
        " px texture scale",
        "textures",
    );
    super::edit::slider(
        ui,
        &mut p.strength,
        0.0..=1.0,
        "Texture strength",
        "textures",
    );
    super::edit::number(
        ui,
        &mut p.angle,
        -360.0..=360.0,
        1.0,
        "° texture rotation",
        "textures",
    );
    for a in 0..2 {
        ui.push_id(a, |ui| {
            super::edit::number(
                ui,
                &mut p.offset[a],
                -1e9..=1e9,
                1.0,
                if a == 0 {
                    " px texture X"
                } else {
                    " px texture Y"
                },
                "textures",
            );
        });
    }
}
impl Drawer {
    pub(super) fn texture_windows(&mut self, ctx: &egui::Context) {
        if self.textures.open {
            if self.textures.editor.is_none() {
                self.finish_stroke();
                if let Some(n) = self.active {
                    let d = &self.documents[n];
                    let layer = match self.panels.targets.get(&d.id) {
                        Some(drawer_core::thumbnail::Target::Raster(n)) => *n,
                        _ => 0,
                    };
                    self.textures.editor = Some(Editor {
                        id: d.id,
                        revision: d.revision,
                        draft: d.scene.paper.clone(),
                        node: Node::Raster(layer),
                    });
                }
            }
            if let Some(mut e) = self.textures.editor.take() {
                let Some(n) = self.documents.iter().position(|d| d.id == e.id) else {
                    self.textures.open = false;
                    return;
                };
                let d = &self.documents[n];
                let mut open = true;
                let mut apply = false;
                let mut cancel = false;
                let mut bake = false;
                let mut layer = None;
                egui::Window::new("Layer and folder paper").open(&mut open).vscroll(true).show(ctx,|ui|{
                    layer=Some(ui.layer_id());
                    ui.horizontal(|ui|{
                        let a=ui.add_enabled(e.draft.valid_for(d.layers.len()) && !super::edit::invalid_layer(ctx,ui.layer_id()),egui::Button::new("Apply paper"));let c=ui.button("Cancel paper");apply=a.clicked();cancel=c.clicked();#[cfg(test)]{self.textures.buttons=[Some(a.rect),Some(c.rect)];}
                    });
                    ui.label("Nondestructive paper shades color, preserves alpha, and follows document coordinates. A folder applies its paper once after its children are composed.");
                    egui::ComboBox::from_id_salt("paper-target").selected_text(match e.node{Node::Raster(n)=>format!("Layer {}",n+1),Node::Folder(n)=>format!("Folder {}",n+1)}).show_ui(ui,|ui|{
                        for(n,l)in d.layers.iter().enumerate(){ui.selectable_value(&mut e.node,Node::Raster(n),format!("Layer {}: {}",n+1,l.name));}
                        for(n,f)in e.draft.folders.iter().enumerate().filter(|(_,f)|!f.children.is_empty()){ui.selectable_value(&mut e.node,Node::Folder(n),format!("Folder {}: {}",n+1,f.name));}
                    });
                    if ui.button("Group all roots in a new folder draft").clicked(){match e.draft.group_all(d.layers.len(),&format!("Paper folder {}",e.draft.folders.len()+1)){Ok(n)=>e.node=Node::Folder(n),Err(err)=>self.status=Some(err)}}
                    if let Node::Folder(n)=e.node {
                        let f=&mut e.draft.folders[n];
                        let changed=ui.checkbox(&mut f.through,"Through: compose children with the outside layers").changed();
                        if changed && f.through {f.paper=Default::default();}
                        ui.label("Through removes this folder's own effect in this draft. Isolated folders combine their children before applying their effect.");
                    }
                    let through=matches!(e.node,Node::Folder(n) if e.draft.folders[n].through);
                    let paper=match e.node{Node::Raster(n)=>e.draft.layers.entry(n).or_default(),Node::Folder(n)=>&mut e.draft.folders[n].paper};
                    ui.add_enabled_ui(!through,|ui|{
                        egui::ComboBox::from_id_salt("paper-blend").selected_text(paper.blend.label()).show_ui(ui,|ui|{
                            for mode in drawer_core::brush_composite::Mode::ALL {ui.selectable_value(&mut paper.blend,mode,mode.label());}
                        });
                        ui.push_id(format!("{:?}",e.node),|ui|pattern_ui(ui,&mut paper.pattern));
                        super::edit::number(ui,&mut paper.seed,0.0..=u32::MAX as f64,1.0," composition seed","brush-composite");
                    });
                    if *paper==Default::default() && let Node::Raster(n)=e.node{e.draft.layers.remove(&n);}
                    ui.separator();ui.label("Raster composition preview");
                    let (rect,_)=ui.allocate_exact_size(egui::vec2(192.0,96.0),egui::Sense::hover());
                    for y in 0..32 {for x in 0..64 {let p=e.draft.preview_pixel(d,x*d.width/64,y*d.height/32);let alpha=p[3] as f32/255.0;let bg=if(x/4+y/4)%2==0{180.0}else{220.0};let c=egui::Color32::from_rgb((p[0] as f32*alpha+bg*(1.0-alpha))as u8,(p[1] as f32*alpha+bg*(1.0-alpha))as u8,(p[2] as f32*alpha+bg*(1.0-alpha))as u8);let min=rect.min+egui::vec2(x as f32*3.0,y as f32*3.0);ui.painter().rect_filled(egui::Rect::from_min_size(min,egui::vec2(3.0,3.0)),0.0,c);}}
                    bake=ui.add_enabled(e.draft==d.scene.paper,egui::Button::new("Bake saved layer / merge saved folder")).clicked();
                    ui.label("Apply a changed draft before baking. Baking replaces the chosen subtree with its rendered raster; Undo restores the original parameters and layers, including off-canvas content.");
                    for message in e.draft.missing(){ui.colored_label(egui::Color32::YELLOW,message);}
                });
                if cancel || !open || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
                    self.textures.open = false;
                    if let Some(layer) = layer {
                        super::edit::discard_layer(ctx, layer);
                    }
                } else if apply || bake {
                    let result = if self.documents[n].revision != e.revision {
                        Err("The drawing changed; close and reopen the paper editor".into())
                    } else if bake {
                        drawer_core::paper::bake(&mut self.documents[n], e.node)
                    } else {
                        let mut scene = self.documents[n].scene.clone();
                        scene.paper = e.draft.clone();
                        self.documents[n].edit_scene(scene)
                    };
                    match result {
                        Ok(_) => {
                            self.drawing.forget_endpoints(e.id);
                            if bake {
                                self.panels
                                    .targets
                                    .insert(e.id, drawer_core::thumbnail::Target::Raster(0));
                            }
                            self.textures.open = false;
                            if let Some(layer) = layer {
                                super::edit::discard_layer(ctx, layer);
                            }
                        }
                        Err(err) => {
                            self.status = Some(err);
                            self.textures.editor = Some(e);
                        }
                    }
                } else {
                    self.textures.editor = Some(e);
                }
            } else {
                self.textures.open = false;
            }
        } else {
            self.textures.editor = None;
        }
        if let Some(mut e) = self.textures.export.take() {
            let Some(n) = self.documents.iter().position(|d| d.id == e.id) else {
                return;
            };
            let d = &self.documents[n];
            let mut open = true;
            let mut choice = None;
            egui::Window::new("PSD rendered export").open(&mut open).show(ctx,|ui|{
                ui.label(drawer_core::psd::LOSS);ui.label(e.path.display().to_string());
                if e.path.exists(){ui.checkbox(&mut e.replace,"Replace this existing PSD file");}
                ui.add_enabled_ui(!e.path.exists() || e.replace,|ui|{
                    if ui.add_enabled(drawer_core::psd::layers_supported(d),egui::Button::new("Export rendered root layers / folders")).clicked(){choice=Some(drawer_core::psd::Choice::RenderedLayers);}
                    if ui.button("Export rendered whole image").clicked(){choice=Some(drawer_core::psd::Choice::RenderedImage);}
                });
                if !drawer_core::psd::layers_supported(d){ui.label("Page objects or animation require the rendered whole image to preserve the current appearance.");}
                if ui.button("Cancel PSD export").clicked(){choice=Some(drawer_core::psd::Choice::Cancel);}
            });
            if let Some(choice) = choice {
                if self.documents[n].revision != e.revision {
                    self.status = Some("The drawing changed; choose the export again".into());
                } else if let Err(err) = self.export_psd_to(n, &e.path, choice, e.replace) {
                    self.status = Some(err);
                }
            } else if open && !ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
                self.textures.export = Some(e);
            }
        }
    }
    pub(super) fn request_psd(&mut self) {
        self.finish_stroke();
        if let Some(n) = self.active
            && let Some(path) = rfd::FileDialog::new()
                .add_filter("Rendered PSD", &["psd"])
                .set_file_name("drawing.psd")
                .save_file()
        {
            let d = &self.documents[n];
            self.textures.export = Some(Export {
                id: d.id,
                revision: d.revision,
                path,
                replace: false,
            });
        }
    }
    pub(super) fn export_psd_to(
        &mut self,
        n: usize,
        path: &std::path::Path,
        choice: drawer_core::psd::Choice,
        replace: bool,
    ) -> Result<bool, String> {
        if choice == drawer_core::psd::Choice::Cancel {
            return Ok(false);
        }
        self.finish_stroke();
        let _awake = self.synchronous_power();
        let target = self.export_target(n, path)?;
        let available = DEFAULT_DOCUMENT_BUDGET
            .saturating_sub(self.documents.iter().map(Document::resident_bytes).sum());
        drawer_core::psd::export(&self.documents[n], &target, choice, replace, available)
            .map_err(|e| e.to_string())
    }
}

pub(super) fn smoke(root: &std::path::Path) -> Result<(), String> {
    use drawer_core::{input_replay::Sequence, psd::Choice};
    use std::sync::atomic::{AtomicBool, AtomicUsize};
    drawer_core::paper::verify()?;
    let cancel = AtomicBool::new(false);
    let progress = AtomicUsize::new(0);
    for mouse in [false, true] {
        let mut s = Sequence::default();
        s.textures[0] = Pattern {
            source: 1,
            strength: 1.0,
            ..Default::default()
        };
        if mouse {
            for sample in &mut s.samples {
                sample.event.source = drawer_core::pointer_input::Source::MOUSE;
                sample.event.pressure = None;
            }
        }
        let a = super::input_replay::run(&s, &cancel, &progress)?;
        let b = super::input_replay::run(&s, &cancel, &progress)?;
        if !a.record.passed || a.pixels != b.pixels || a.pixels.iter().all(|p| p[3] == 0) {
            return Err("Texture mouse/pen replay failed".into());
        }
        s.textures = [Default::default(); 7];
        let plain = super::input_replay::run(&s, &cancel, &progress)?;
        if plain.pixels == a.pixels {
            return Err("Brush texture had no effect".into());
        }
    }
    let ctx = egui::Context::default();
    let mut app = Drawer::new(&ctx, None);
    app.create_canvas(CanvasSpec {
        width: 9,
        height: 5,
        dpi: 300.0,
        background: Background::Color([210, 130, 70]),
    })
    .map_err(|e| e.to_string())?;
    let d = &mut app.documents[0];
    d.layers[0].pixels[0] = [210, 130, 70, 77];
    let n = d.scene.paper.group_all(1, "Owned paper")?;
    d.scene.paper.folders[n].paper.pattern = Pattern {
        source: 2,
        strength: 0.8,
        ..Default::default()
    };
    let before = d.snapshot();
    let shown = drawer_core::render::Renderer::new(d)
        .output(d.dpi, 100000)?
        .2;
    let path = root.join("paper-smoke.psd");
    if app.export_psd_to(0, &path, Choice::Cancel, false)? || path.exists() {
        return Err("PSD cancellation wrote a file".into());
    }
    app.export_psd_to(0, &path, Choice::RenderedLayers, false)?;
    let reopened = drawer_core::files::open(&path, 100000).map_err(|e| e.to_string())?;
    if reopened.layers[0].pixels != shown
        || app.documents[0].snapshot() != before
        || app.documents[0].history_counts() != (0, 0)
    {
        return Err("Rendered PSD changed the source or preview".into());
    }
    app.protected_sources
        .insert(path.canonicalize().map_err(|e| e.to_string())?);
    if app
        .export_psd_to(0, &path, Choice::RenderedImage, true)
        .is_ok()
    {
        return Err("PSD overwrote a protected source".into());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{TestDirectory, click, frame};
    #[test]
    fn replay_export_cancel_reopen_and_source_protection() {
        let root = TestDirectory::new();
        smoke(&root.0).unwrap();
    }
    #[test]
    fn paper_ui_cancel_apply_one_undo_and_stale_document_rejected() {
        let ctx = egui::Context::default();
        let mut app = Drawer::new(&ctx, None);
        app.create_canvas(CanvasSpec {
            width: 8,
            height: 4,
            dpi: 300.0,
            background: Background::Color([210, 100, 50]),
        })
        .unwrap();
        frame(&ctx, &mut app, vec![]);
        frame(&ctx, &mut app, vec![]);
        let before = app.documents[0].snapshot();
        let id = app.documents[0].id;
        let mut graph = Graph::default();
        graph.layers.insert(
            0,
            drawer_core::paper::Paper {
                pattern: Pattern {
                    source: 1,
                    ..Default::default()
                },
                ..Default::default()
            },
        );
        let editor = |revision| Editor {
            id,
            revision,
            draft: graph.clone(),
            node: Node::Raster(0),
        };
        app.textures.open = true;
        app.textures.editor = Some(editor(0));
        frame(&ctx, &mut app, vec![]);
        frame(&ctx, &mut app, vec![]);
        assert!(app.tool_shortcuts_blocked(&ctx));
        let cancel = app.textures.buttons[1].unwrap();
        click(&ctx, &mut app, cancel);
        assert!(app.documents[0].snapshot() == before);
        assert_eq!(app.documents[0].history_counts(), (0, 0));
        app.textures.open = true;
        app.textures.editor = Some(editor(0));
        frame(&ctx, &mut app, vec![]);
        frame(&ctx, &mut app, vec![]);
        let apply = app.textures.buttons[0].unwrap();
        click(&ctx, &mut app, apply);
        assert!(!app.textures.open);
        assert_eq!(app.documents[0].scene.paper, graph);
        assert_eq!(app.documents[0].layers, before.layers);
        assert_eq!(app.documents[0].history_counts(), (1, 0));
        assert!(app.documents[0].undo());
        assert!(app.documents[0].snapshot() == before);
        assert!(app.documents[0].redo());
        let after = app.documents[0].snapshot();
        app.textures.open = true;
        app.textures.editor = Some(editor(0));
        frame(&ctx, &mut app, vec![]);
        frame(&ctx, &mut app, vec![]);
        let apply = app.textures.buttons[0].unwrap();
        click(&ctx, &mut app, apply);
        assert!(app.textures.open);
        assert!(app.status.as_ref().unwrap().contains("changed"));
        assert!(app.documents[0].snapshot() == after);
    }
    #[test]
    fn brush_texture_preset_settings_backup_and_restart_are_self_contained() {
        let root = TestDirectory::new();
        let storage = UserStorage::at(root.0.join("user")).unwrap();
        let ctx = egui::Context::default();
        let mut app =
            Drawer::new(&ctx, None).with_storage(UserStorage::at(storage.root().into()).unwrap());
        app.create_canvas(crate::tests::tiny_spec()).unwrap();
        let before = app.documents[0].snapshot();
        app.sync_tool_properties();
        let baseline = app.capture_workspace();
        let mut next = baseline.clone();
        next.input.textures[1] = Pattern {
            source: 2,
            scale: 2.5,
            strength: 0.8,
            offset: [-2.0, 3.0],
            ..Default::default()
        };
        let preset = drawer_core::brush_presets::Preset::capture(
            &next,
            BrushTip::Pencil,
            "Owned linen pencil",
        )
        .unwrap();
        let bytes = preset.encode().unwrap();
        let restored = drawer_core::brush_presets::Preset::decode(&bytes).unwrap();
        let applied = restored.apply(&baseline).unwrap();
        assert_eq!(applied.input.textures[1], next.input.textures[1]);
        app.stage_workspace(applied, None);
        app.apply_workspace(
            drawer_core::workspaces::Scope {
                other: true,
                ..Default::default()
            },
            None,
        )
        .unwrap();
        let restarted = Drawer::new(&egui::Context::default(), None)
            .with_storage(UserStorage::at(storage.root().into()).unwrap());
        assert_eq!(
            restarted.workspaces.store.current.input.textures[1],
            next.input.textures[1]
        );
        let backup = drawer_core::workspaces::Snapshot::read(
            &storage.root().join("settings/workspace-before-apply.json"),
        )
        .unwrap();
        assert_eq!(backup.input.textures, baseline.input.textures);
        assert!(app.documents[0].snapshot() == before);
        assert_eq!(app.documents[0].history_counts(), (0, 0));
    }
}
