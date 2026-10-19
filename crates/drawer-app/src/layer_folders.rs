use super::*;
use drawer_core::{
    layer_folders::{self as core, Command},
    layer_properties::{self as props, Target},
    scene::Rect,
};
#[derive(Default)]
pub(super) struct Panel {
    draft: Option<Box<Draft>>,
    create: Option<egui::Rect>,
    wrap: Option<egui::Rect>,
    unwrap: Option<egui::Rect>,
    mask: Option<egui::Rect>,
    remove: Option<egui::Rect>,
}
struct Draft {
    id: u64,
    revision: u64,
    frame: u32,
    selected: Vec<Target>,
    name: String,
    parent: Option<usize>,
    position: usize,
    through: bool,
    gray: u8,
    bounds: Rect,
}
impl Drawer {
    pub(super) fn begin_layer_folders(&mut self) -> Result<(), String> {
        let d = &self.documents[self.active.ok_or("Open a page first")?];
        let target = props::from_thumbnail(
            d,
            self.panels
                .targets
                .get(&d.id)
                .copied()
                .unwrap_or(drawer_core::thumbnail::Target::Raster(0)),
        );
        let through = if let Target::Folder(n) = target {
            d.scene.paper.folders[n as usize].through
        } else {
            true
        };
        self.layer_folders.draft = Some(Box::new(Draft {
            id: d.id,
            revision: d.revision,
            frame: d.current_frame,
            selected: vec![target],
            name: "Layer folder".into(),
            parent: None,
            position: 0,
            through,
            gray: 255,
            bounds: Rect {
                min: [0.; 2],
                max: [d.width as f64, d.height as f64],
            },
        }));
        Ok(())
    }
    fn folder_command(&mut self, c: Command) -> Result<(), String> {
        let e = self
            .layer_folders
            .draft
            .as_ref()
            .ok_or("Folder/mask draft is closed")?;
        let n = self.active.ok_or("Open a page first")?;
        let d = &self.documents[n];
        if (d.id, d.revision, d.current_frame) != (e.id, e.revision, e.frame) {
            return Err("Page/frame changed; reopen folder/mask controls".into());
        }
        let state = core::prepare(d, c)?;
        let used: usize = self.documents.iter().map(Document::resident_bytes).sum();
        if used.saturating_add(state.state.memory_bytes().saturating_mul(2))
            > DEFAULT_DOCUMENT_BUDGET
        {
            return Err("Folder/mask edit exceeds workspace memory".into());
        }
        self.documents[n].commit_state(state.state)?;
        self.sync_layer_identity();
        let d = &self.documents[n];
        let selected = state.selected;
        let shown = match selected {
            Target::Raster(id) => {
                drawer_core::layers::index(d, id).map(drawer_core::thumbnail::Target::Raster)
            }
            Target::Folder(n) => Some(drawer_core::thumbnail::Target::Folder(n as usize)),
            Target::Vector(n) => Some(drawer_core::thumbnail::Target::Vector(n)),
            _ => None,
        };
        if let Some(target) = shown {
            self.panels.targets.insert(d.id, target);
        }
        let e = self.layer_folders.draft.as_mut().unwrap();
        e.revision = d.revision;
        e.selected = vec![selected];
        if let Target::Folder(n) = selected {
            e.through = d.scene.paper.folders[n as usize].through;
        }
        Ok(())
    }
    pub(super) fn layer_folder_window(&mut self, ctx: &egui::Context) {
        let Some(mut e) = self.layer_folders.draft.take() else {
            return;
        };
        let Some(n) = self.active.filter(|n| self.documents[*n].id == e.id) else {
            return;
        };
        let d = &self.documents[n];
        let fresh = (d.revision, d.current_frame) == (e.revision, e.frame);
        let targets = props::targets(d)
            .into_iter()
            .filter(|t| matches!(t, Target::Raster(_) | Target::Folder(_) | Target::Vector(_)))
            .collect::<Vec<_>>();
        let folder = if let [Target::Folder(n)] = e.selected.as_slice() {
            Some(*n as usize)
        } else {
            None
        };
        let mut open = !ctx.input(|i| i.key_pressed(egui::Key::Escape));
        let mut cancel = false;
        let mut action = None;
        let mut raster = false;
        egui::Window::new("Layer folders and grayscale masks").open(&mut open).collapsible(false).resizable(false).default_width(560.).default_pos(egui::pos2(45.,20.)).show(ctx,|ui|{
   if !fresh{ui.label("Page/frame changed; cancel and reopen these controls.");cancel=ui.button("Cancel").clicked();return;}
   egui::ScrollArea::vertical().id_salt("folder-mask-targets").max_height(130.).show_rows(ui,22.,targets.len(),|ui,rows|{for n in rows{let t=targets[n];let mut selected=e.selected.contains(&t);if ui.checkbox(&mut selected,super::layer_properties::label(d,t)).changed(){if selected{e.selected.push(t);}else{e.selected.retain(|v|*v!=t);}
if let [Target::Folder(n)]=e.selected.as_slice(){e.through=d.scene.paper.folders[*n as usize].through;}}}});
   ui.horizontal(|ui|{ui.label("New folder name");ui.text_edit_singleline(&mut e.name);});
   egui::ComboBox::from_label("Destination parent").selected_text(e.parent.map_or("Root".into(),|n|format!("Folder {}",n+1))).show_ui(ui,|ui|{ui.selectable_value(&mut e.parent,None,"Root");for (n,f) in d.scene.paper.folders.iter().enumerate().filter(|(n,_)|d.scene.paper.folder_active(*n)){ui.selectable_value(&mut e.parent,Some(n),&f.name);}});
   ui.horizontal(|ui|{let r=ui.button("Create empty folder");self.layer_folders.create=Some(r.rect);if r.clicked(){action=Some(Command::Create{name:e.name.clone(),parent:e.parent});}let r=ui.add_enabled(!e.selected.is_empty(),egui::Button::new("Wrap selected in folder"));self.layer_folders.wrap=Some(r.rect);if r.clicked(){action=Some(Command::Wrap{name:e.name.clone(),targets:e.selected.clone()});}});
   ui.add_enabled_ui(folder.is_some(),|ui|{ui.horizontal(|ui|{ui.add(egui::DragValue::new(&mut e.position).range(0..=512).prefix("Slot "));if ui.button("Move folder").clicked(){action=Some(Command::Move{folder:folder.unwrap(),parent:e.parent,position:e.position});}});
    ui.horizontal(|ui|{ui.checkbox(&mut e.through,"Through folder");if ui.button("Apply folder mode").clicked(){action=Some(Command::Mode{folder:folder.unwrap(),through:e.through});}let r=ui.button("Unwrap folder");self.layer_folders.unwrap=Some(r.rect);if r.clicked(){action=Some(Command::Unwrap(folder.unwrap()));}});
   });
   ui.small("Through folders include their incoming backdrop. Normal folders isolate their children. Unwrap preserves child order; masks/effects may require raster preservation.");
   raster=ui.button("Open raster preservation preview…").clicked();
   ui.separator();ui.label("Replace one selected layer/folder mask with a gray rectangle");ui.add(egui::Slider::new(&mut e.gray,0..=255).text("Visible strength (0 hides / 255 shows)"));
   ui.horizontal(|ui|{for(k,name)in["Left","Top"].iter().enumerate(){ui.add(egui::DragValue::new(&mut e.bounds.min[k]).range(-1000000. ..=1000000.).prefix(format!("{name} ")));}for(k,name)in["Right","Bottom"].iter().enumerate(){ui.add(egui::DragValue::new(&mut e.bounds.max[k]).range(-1000000. ..=1000000.).prefix(format!("{name} ")));}});
   if let Some(t)=e.selected.first(){ui.label(if d.scene.layer_effects.get(*t).is_some_and(|v|v.mask.is_some()){"Selected target has a mask"}else{"Selected target has no mask"});}
   ui.horizontal(|ui|{let r=ui.add_enabled(e.selected.len()==1,egui::Button::new("Replace rectangle mask"));self.layer_folders.mask=Some(r.rect);if r.clicked(){match core::rectangle_mask(d,e.bounds,e.gray){Ok(mask)=>action=Some(Command::Mask{target:e.selected[0],mask:Some(mask)}),Err(err)=>self.status=Some(err)}}let r=ui.add_enabled(e.selected.len()==1,egui::Button::new("Remove mask"));self.layer_folders.remove=Some(r.rect);if r.clicked(){action=Some(Command::Mask{target:e.selected[0],mask:None});}cancel=ui.button("Close").clicked();});
   ui.small("Each button applies one Undo. Mask strength multiplies layer and parent alpha. Layer movement carries the linked mask; page rotation preserves its geometry.");
  });
        if open && !cancel {
            self.layer_folders.draft = Some(e);
        }
        if let Some(command) = action
            && let Err(err) = self.folder_command(command)
        {
            self.status = Some(err);
        }
        if raster {
            if let Err(err) = self.begin_layer_merge() {
                self.status = Some(err);
            } else {
                self.layer_folders.draft = None;
            }
        }
    }
}
pub(super) fn smoke(root: &std::path::Path) -> Result<(), String> {
    drawer_core::layer_folders_verify::verify(root)?;
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
    app.documents[0].layers[0].pixels.fill([200, 100, 50, 255]);
    let mut l = app.documents[0].layers[0].clone();
    l.pixels.fill([0; 4]);
    app.documents[0].layers.push(l);
    let mut time = 0.;
    let step = |app: &mut Drawer, time: &mut f64, events| {
        *time += 0.1;
        super::input_replay::frame(&ctx, app, *time, events);
    };
    step(&mut app, &mut time, vec![]);
    step(&mut app, &mut time, vec![]);
    app.execute_command(&ctx, "layer-folders");
    let before = app.documents[0].snapshot();
    let e = app
        .layer_folders
        .draft
        .as_mut()
        .ok_or("Folder command missing")?;
    e.selected = vec![Target::Raster(1), Target::Raster(2)];
    step(&mut app, &mut time, vec![]);
    step(&mut app, &mut time, vec![]);
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
    let r = app.layer_folders.wrap.ok_or("Wrap button missing")?;
    click(&mut app, &mut time, r);
    if app.documents[0].scene.paper.folders.len() != 1 {
        return Err(format!("Actual Wrap failed: {:?}", app.status));
    }
    let grouped = app.documents[0].snapshot();
    if !app.documents[0].undo()
        || app.documents[0].snapshot() != before
        || !app.documents[0].redo()
        || app.documents[0].snapshot() != grouped
    {
        return Err("Wrap UI structure Undo/Redo".into());
    }
    app.layer_folders.draft = None;
    app.execute_command(&ctx, "layer-folders");
    {
        let e = app.layer_folders.draft.as_mut().unwrap();
        e.selected = vec![Target::Folder(0)];
        e.gray = 128;
    }
    step(&mut app, &mut time, vec![]);
    step(&mut app, &mut time, vec![]);
    let r = app.layer_folders.mask.ok_or("Mask button missing")?;
    click(&mut app, &mut time, r);
    if app.documents[0].composite_pixel(0, 0) != [200, 100, 50, 128] {
        return Err(format!("Actual gray mask failed: {:?}", app.status));
    }
    let masked = app.documents[0].snapshot();
    let file = root.join("folder-mask-ui.drawer");
    drawer_core::files::save(&mut app.documents[0], &file).map_err(|e| e.to_string())?;
    if drawer_core::files::open(&file, DEFAULT_DOCUMENT_BUDGET)
        .map_err(|e| e.to_string())?
        .snapshot()
        != masked
    {
        return Err("Folder/mask UI reopen".into());
    }
    let r = app
        .layer_folders
        .remove
        .ok_or("Remove mask button missing")?;
    click(&mut app, &mut time, r);
    if app.documents[0].snapshot() != grouped {
        return Err("Remove mask left unexpected state".into());
    }
    let r = app.layer_folders.unwrap.ok_or("Unwrap button missing")?;
    click(&mut app, &mut time, r);
    if app.documents[0].scene.paper.folder_active(0) {
        return Err(format!("Actual Unwrap failed: {:?}", app.status));
    }
    app.layer_folders.draft.as_mut().unwrap().parent = None;
    let r = app
        .layer_folders
        .create
        .ok_or("Create folder button missing")?;
    click(&mut app, &mut time, r);
    if app.documents[0].scene.paper.folders.len() != 2
        || !app.documents[0].scene.paper.folder_active(1)
    {
        return Err("Actual empty folder create failed".into());
    }
    let before = app.documents[0].snapshot();
    app.documents[0].current_frame += 1;
    if app.folder_command(Command::Unwrap(1)).is_ok() || app.documents[0].snapshot() != before {
        return Err("Stale folder frame accepted".into());
    }
    app.documents[0].current_frame = 0;
    app.layer_folders.draft = None;
    app.execute_command(&ctx, "layer-folders");
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
    if app.layer_folders.draft.is_some() || app.documents[0].snapshot() != before {
        return Err("Folder/mask Escape changed document".into());
    }
    app.execute_command(&ctx, "layer-folders");
    app.documents.clear();
    app.active = None;
    app.layer_folder_window(&ctx);
    if app.layer_folders.draft.is_some() {
        return Err("Closed folder document retained draft".into());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn actual_folder_mask_buttons_undo_reopen_and_close() {
        let root = crate::tests::TestDirectory::new();
        super::smoke(&root.0).unwrap();
    }
}
