use super::*;
use drawer_core::{
    brush_presets::{Book, Preset},
    tool_properties::{Control, Property, Tool},
};
enum Pending {
    Collection(Preset),
    File(Preset, PathBuf),
}
struct Editor {
    draft: Book,
    name: String,
    selected: Option<usize>,
    restore: Option<Preset>,
    pending: Option<Pending>,
}
#[derive(Default)]
pub(super) struct Presets {
    pub open: bool,
    editor: Option<Editor>,
    #[cfg(test)]
    apply: Option<egui::Rect>,
    #[cfg(test)]
    cancel: Option<egui::Rect>,
    #[cfg(test)]
    confirm: Option<egui::Rect>,
    #[cfg(test)]
    decline: Option<egui::Rect>,
}
impl Drawer {
    pub(super) fn set_quick_brush_size(&mut self, value: f64) -> Result<bool, String> {
        if self.drawing.mode != super::precision::Mode::Paint {
            return Err("This tool has no brush size".into());
        }
        self.sync_tool_properties();
        let current = self.capture_workspace();
        let mut size = drawer_core::numeric::Scalar::new(
            current.panels.brush_size,
            drawer_core::numeric::Range {
                min: 0.1,
                max: 4096.0,
            },
        )?;
        if !size.set(value)? {
            return Ok(false);
        }
        let next = drawer_core::brush_presets::select_size(&current, self.selected, size.value())?;
        self.panels.settings.brush_size = next.panels.brush_size;
        self.workspaces.store.current.properties = next.properties;
        Ok(true)
    }
    pub(super) fn quick_brush_sizes(&mut self, ui: &mut egui::Ui) {
        let capable = self.drawing.mode == super::precision::Mode::Paint;
        let sizes = self.workspaces.store.current.brushes.sizes.clone();
        let mut chosen = None;
        ui.add_enabled_ui(capable, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label("Quick size (px)");
                for size in sizes {
                    if ui
                        .selectable_label(self.panels.settings.brush_size == size, size.to_string())
                        .clicked()
                    {
                        chosen = Some(size);
                    }
                }
            });
            ui.horizontal(|ui| {
                if ui.small_button("Add current size").clicked()
                    && let Err(e) = self
                        .workspaces
                        .store
                        .current
                        .brushes
                        .add_size(self.panels.settings.brush_size)
                {
                    self.status = Some(e);
                }
                let state = self
                    .workspaces
                    .store
                    .current
                    .properties
                    .as_mut()
                    .unwrap()
                    .tools
                    .get_mut(&Tool::brush(self.selected))
                    .unwrap()
                    .get_mut(&Property::Size)
                    .unwrap();
                let mut show = state.control == Control::Slider;
                if ui.checkbox(&mut show, "Size slider").changed() {
                    state.control = if show {
                        Control::Slider
                    } else {
                        Control::Number
                    };
                }
            });
        })
        .response
        .on_disabled_hover_text("Choose a drawing brush to use quick sizes");
        if let Some(value) = chosen
            && let Err(e) = self.set_quick_brush_size(value)
        {
            self.status = Some(e);
        }
        if crate::help::button(ui, "Brush presets…", "tool-properties").clicked() {
            self.finish_stroke();
            self.brush_presets.open = true;
        }
    }
    pub(super) fn brush_presets_window(&mut self, ctx: &egui::Context) {
        if !self.brush_presets.open {
            return;
        }
        self.sync_tool_properties();
        let mut e = self.brush_presets.editor.take().unwrap_or_else(|| Editor {
            draft: self.workspaces.store.current.brushes.clone(),
            name: self.selected.label().into(),
            selected: None,
            restore: None,
            pending: None,
        });
        let mut open = true;
        let mut apply = false;
        let mut cancel = false;
        egui::Window::new("Local brush presets").open(&mut open).vscroll(true).show(ctx,|ui|{
   ui.horizontal(|ui|{let a=ui.add_enabled(e.draft.valid()&&e.pending.is_none(),egui::Button::new("Apply brush presets"));let c=ui.button("Cancel brush presets");apply=a.clicked();cancel=c.clicked();#[cfg(test)]{self.brush_presets.apply=Some(a.rect);self.brush_presets.cancel=Some(c.rect);}});
   ui.label("Presets and size lists are local settings. Restore is staged until Apply; drawing history stays separate.");
   if let Some(p)=&e.pending{
    ui.separator();ui.colored_label(egui::Color32::YELLOW,match p{Pending::Collection(p)=>format!("Replace existing preset ‘{}’?",p.name),Pending::File(_,path)=>format!("Replace local preset file {}?",path.display())});
    let mut accept=false;let mut reject=false;
    ui.horizontal(|ui|{let a=ui.button("Confirm preset replacement");let c=ui.button("Keep existing preset");accept=a.clicked();reject=c.clicked();#[cfg(test)]{self.brush_presets.confirm=Some(a.rect);self.brush_presets.decline=Some(c.rect);}});
    if accept {match e.pending.take().unwrap(){Pending::Collection(p)=>{
        let restoring=e.restore.as_ref().is_some_and(|old|old.name.to_lowercase()==p.name.to_lowercase());
        match e.draft.put(p.clone(),true){Ok(())=>if restoring{e.restore=Some(p);},Err(err)=>self.status=Some(err)}
    },Pending::File(p,path)=>if let Err(err)=p.save(&path,true){self.status=Some(err.to_string());}}}else if reject{e.pending=None;}
   }
   ui.add_enabled_ui(e.pending.is_none(),|ui|{
    ui.label("Preset name");super::edit::text(ui,&mut e.name,"tool-properties");
    if ui.add_enabled(self.drawing.mode==super::precision::Mode::Paint,egui::Button::new("Capture current brush into draft")).clicked(){
     match Preset::capture(&self.capture_workspace(),self.selected,&e.name){Ok(p)=>if e.draft.contains(&p.name){e.pending=Some(Pending::Collection(p));}else if let Err(err)=e.draft.put(p,false){self.status=Some(err);},Err(err)=>self.status=Some(err)}
    }
    ui.separator();
    for(n,p)in e.draft.presets.iter().enumerate(){if ui.selectable_label(e.selected==Some(n),format!("{} — {} · {} px",p.name,p.tip.label(),p.values[0])).clicked(){e.selected=Some(n);}}
    let selected=e.selected.and_then(|n|e.draft.presets.get(n)).cloned();
    ui.add_enabled_ui(selected.is_some(),|ui|{
     ui.horizontal(|ui|{
      if ui.button("Stage selected brush restore").clicked(){e.restore=selected.clone();}
      if ui.button("Delete selected preset from draft").clicked(){let removed=e.draft.presets.remove(e.selected.unwrap());if e.restore.as_ref().is_some_and(|p|p.name==removed.name){e.restore=None;}e.selected=None;}
      if ui.button("Export selected preset…").clicked() && let Some(path)=rfd::FileDialog::new().set_file_name("brush.json").add_filter("Local brush preset",&["json"]).save_file(){let p=selected.clone().unwrap();if path.exists(){e.pending=Some(Pending::File(p,path));}else if let Err(err)=p.save(&path,false){self.status=Some(err.to_string());}}
     });
    });
    if ui.button("Read local brush preset into draft…").clicked() && let Some(path)=rfd::FileDialog::new().add_filter("Local brush preset",&["json"]).pick_file(){match Preset::read(&path){Ok(p)=>if e.draft.contains(&p.name){e.pending=Some(Pending::Collection(p));}else if let Err(err)=e.draft.put(p,false){self.status=Some(err);},Err(err)=>self.status=Some(err.to_string())}}
    if let Some(p)=&e.restore{for source in p.sequence.missing().into_iter().filter(|id|self.materials.library.get(*id,drawer_core::material_library::Kind::Tip).is_none()){ui.colored_label(egui::Color32::YELLOW,drawer_core::tip_sequence::Settings::label(source));}if p.overlay.missing()&&self.materials.library.get(p.overlay.source,drawer_core::material_library::Kind::Texture).is_none(){ui.colored_label(egui::Color32::YELLOW,p.overlay.label());}ui.label(format!("Apply will restore {} ({})",p.name,p.tip.label()));if ui.button("Cancel staged brush restore").clicked(){e.restore=None;}}
    ui.separator();ui.heading("Quick size order");let mut movement=None;let mut remove=None;
    for(n,size)in e.draft.sizes.iter().enumerate(){ui.push_id(("size",n),|ui|{ui.horizontal(|ui|{ui.label(format!("{size} px"));if ui.add_enabled(n>0,egui::Button::new("Up")).clicked(){movement=Some((n,-1));}
    if ui.add_enabled(n+1<e.draft.sizes.len(),egui::Button::new("Down")).clicked(){movement=Some((n,1));}
    if ui.button("Remove from size list").clicked(){remove=Some(n);}});});}
    if let Some((n,d))=movement{e.draft.move_size(n,d);}
    if let Some(n)=remove{e.draft.remove_size(n);}
    if ui.add_enabled(self.drawing.mode==super::precision::Mode::Paint,egui::Button::new("Add current size to draft list")).clicked() && let Err(err)=e.draft.add_size(self.panels.settings.brush_size){self.status=Some(err);}
   });
  });
        if cancel || !open || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.brush_presets.open = false;
        } else if apply && e.pending.is_none() {
            let next = (|| {
                let current = self.capture_workspace();
                let mut next = if let Some(p) = &e.restore {
                    p.apply(&current)?
                } else {
                    current
                };
                next.brushes = e.draft.clone();
                Ok::<_, String>(next)
            })();
            match next.and_then(|snapshot| {
                self.stage_workspace(snapshot, self.workspaces.store.active.clone());
                self.apply_workspace(
                    drawer_core::workspaces::Scope {
                        other: true,
                        ..Default::default()
                    },
                    None,
                )
            }) {
                Ok(()) => {
                    if let Some(p) = &e.restore {
                        self.set_tool(p.tip);
                    }
                    self.brush_presets.open = false;
                }
                Err(err) => {
                    self.workspaces.staged = None;
                    self.status = Some(err);
                    self.brush_presets.editor = Some(e);
                }
            }
        } else {
            self.brush_presets.editor = Some(e);
        }
    }
}

pub(super) fn smoke() -> Result<(), String> {
    let ctx = egui::Context::default();
    let mut app = Drawer::new(&ctx, None);
    app.create_canvas(CanvasSpec {
        width: 2,
        height: 1,
        dpi: 300.0,
        background: Background::Transparent,
    })
    .map_err(|e| e.to_string())?;
    app.sync_tool_properties();
    let before = app.documents[0].snapshot();
    let baseline = app.capture_workspace();
    let p = Preset::capture(&baseline, BrushTip::Pen, "Local ink")?;
    let json = p.encode().map_err(|e| e.to_string())?;
    let restored = Preset::decode(&json).map_err(|e| e.to_string())?;
    let mut book = Book::default();
    book.put(restored.clone(), false)?;
    let saved = book.clone();
    if book.put(restored, false).is_ok() || book != saved {
        return Err("Unconfirmed preset replacement changed collection".into());
    }
    for value in [0.1, 4096.0] {
        if !app.set_quick_brush_size(value)? || app.set_quick_brush_size(value)? {
            return Err("Quick size update/duplicate failed".into());
        }
        if value == 0.1 && app.set_quick_brush_size(value + 1e-14)? {
            return Err("Equivalent size generated a duplicate update".into());
        }
    }
    let mut expected = baseline;
    expected.panels.brush_size = 4096.0;
    expected
        .properties
        .as_mut()
        .unwrap()
        .tools
        .get_mut(&Tool::Pen)
        .unwrap()
        .get_mut(&Property::Size)
        .unwrap()
        .value = 4096.0;
    if app.capture_workspace() != expected {
        return Err("Quick size changed another brush option".into());
    }
    app.drawing.mode = super::precision::Mode::Move;
    if app.set_quick_brush_size(12.0).is_ok() {
        return Err("Non-brush tool accepted quick size".into());
    }
    let snapshot = p.apply(&app.capture_workspace())?;
    app.stage_workspace(snapshot, None);
    app.apply_workspace(
        drawer_core::workspaces::Scope {
            other: true,
            ..Default::default()
        },
        None,
    )?;
    if app.panels.settings.brush_size != 12.0
        || app.documents[0].snapshot() != before
        || app.documents[0].history_counts() != (0, 0)
    {
        return Err("Brush preset restore changed drawing history".into());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{TestDirectory, click, frame, tiny_spec};
    #[test]
    fn quick_size_minimum_maximum_duplicate_other_values_and_non_brush() {
        smoke().unwrap();
    }
    #[test]
    fn named_restore_cancel_apply_backup_restart_and_size_list_order_are_settings() {
        let root = TestDirectory::new();
        let storage = UserStorage::at(root.0.join("user")).unwrap();
        let ctx = egui::Context::default();
        let mut app = Drawer::new(&ctx, None)
            .with_storage(UserStorage::at(storage.root().to_path_buf()).unwrap());
        app.create_canvas(tiny_spec()).unwrap();
        frame(&ctx, &mut app, vec![]);
        frame(&ctx, &mut app, vec![]);
        let baseline = app.capture_workspace();
        let before = app.documents[0].snapshot();
        let dirty = app.documents[0].dirty;
        let mut p = Preset::capture(&baseline, BrushTip::Pencil, "Local pencil").unwrap();
        p.values[0] = 29.0;
        p.values[4] = 0.4;
        p.values[5] = 0.3;
        p.values[6] = 0.2;
        p.ends.start = 9.0;
        p.mouse.seed = 71;
        let mut draft = baseline.brushes.clone();
        draft.put(p.clone(), false).unwrap();
        draft.add_size(4096.0).unwrap();
        draft.move_size(0, 1);
        draft.remove_size(2);
        let editor = || Editor {
            draft: draft.clone(),
            name: "Local pencil".into(),
            selected: Some(0),
            restore: Some(p.clone()),
            pending: None,
        };
        app.brush_presets.open = true;
        app.brush_presets.editor = Some(editor());
        frame(&ctx, &mut app, vec![]);
        frame(&ctx, &mut app, vec![]);
        let rect = app.brush_presets.cancel.unwrap();
        click(&ctx, &mut app, rect);
        assert_eq!(app.capture_workspace(), baseline);
        app.brush_presets.open = true;
        app.brush_presets.editor = Some(editor());
        frame(&ctx, &mut app, vec![]);
        frame(&ctx, &mut app, vec![]);
        let rect = app.brush_presets.apply.unwrap();
        click(&ctx, &mut app, rect);
        assert!(!app.brush_presets.open);
        assert_eq!(app.selected, BrushTip::Pencil);
        assert_eq!(app.panels.settings.brush_size, 29.0);
        assert_eq!(app.workspaces.store.current.brushes, draft);
        assert_eq!(app.workspaces.store.current.input.ends[1].start, 9.0);
        assert_eq!(
            app.workspaces.store.current.input.mouse_pressure[1].seed,
            71
        );
        let backup = drawer_core::workspaces::Snapshot::read(
            &storage.root().join("settings/workspace-before-apply.json"),
        )
        .unwrap();
        assert_eq!(backup, baseline);
        let restarted = Drawer::new(&egui::Context::default(), None)
            .with_storage(UserStorage::at(storage.root().to_path_buf()).unwrap());
        assert_eq!(restarted.workspaces.store.current.brushes, draft);
        assert_eq!(restarted.panels.settings.brush_size, 29.0);
        assert!(app.documents[0].snapshot() == before);
        assert_eq!(app.documents[0].dirty, dirty);
        assert_eq!(app.documents[0].history_counts(), (0, 0));
    }
    #[test]
    fn collection_and_file_overwrite_require_confirmation_and_decline_preserves_previous() {
        let root = TestDirectory::new();
        let ctx = egui::Context::default();
        let mut app = Drawer::new(&ctx, None);
        app.create_canvas(tiny_spec()).unwrap();
        frame(&ctx, &mut app, vec![]);
        let before = app.capture_workspace();
        let original = Preset::capture(&before, BrushTip::Pen, "Ink").unwrap();
        let mut replacement = original.clone();
        replacement.values[0] = 33.0;
        let mut book = Book::default();
        book.put(original.clone(), false).unwrap();
        let editor = |pending| Editor {
            draft: book.clone(),
            name: "Ink".into(),
            selected: Some(0),
            restore: None,
            pending: Some(pending),
        };
        app.brush_presets.open = true;
        app.brush_presets.editor = Some(editor(Pending::Collection(replacement.clone())));
        frame(&ctx, &mut app, vec![]);
        frame(&ctx, &mut app, vec![]);
        let rect = app.brush_presets.decline.unwrap();
        click(&ctx, &mut app, rect);
        assert_eq!(app.brush_presets.editor.as_ref().unwrap().draft, book);
        app.brush_presets.editor = Some(editor(Pending::Collection(replacement.clone())));
        app.brush_presets.editor.as_mut().unwrap().restore = Some(original.clone());
        frame(&ctx, &mut app, vec![]);
        frame(&ctx, &mut app, vec![]);
        let rect = app.brush_presets.confirm.unwrap();
        click(&ctx, &mut app, rect);
        assert_eq!(
            app.brush_presets.editor.as_ref().unwrap().draft.presets[0],
            replacement
        );
        assert_eq!(
            app.brush_presets.editor.as_ref().unwrap().restore,
            Some(replacement.clone())
        );
        assert_eq!(app.capture_workspace(), before);
        let path = root.0.join("preset.json");
        original.save(&path, false).unwrap();
        app.brush_presets.editor = Some(editor(Pending::File(replacement.clone(), path.clone())));
        frame(&ctx, &mut app, vec![]);
        frame(&ctx, &mut app, vec![]);
        let rect = app.brush_presets.decline.unwrap();
        click(&ctx, &mut app, rect);
        assert_eq!(Preset::read(&path).unwrap(), original);
        app.brush_presets.editor = Some(editor(Pending::File(replacement.clone(), path.clone())));
        frame(&ctx, &mut app, vec![]);
        frame(&ctx, &mut app, vec![]);
        let rect = app.brush_presets.confirm.unwrap();
        click(&ctx, &mut app, rect);
        assert_eq!(Preset::read(&path).unwrap(), replacement);
        assert_eq!(app.capture_workspace(), before);
    }
}
