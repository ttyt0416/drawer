use super::*;
use drawer_core::{
    brush_presets::Preset,
    color_tables::Pixels,
    workspaces::{PAD_SIZE, Scratchpad},
};
#[derive(Default)]
pub(super) struct Runtime {
    paint: Option<Box<super::workspaces::PadPaint>>,
    texture: Option<egui::TextureHandle>,
    pub history: Pixels,
    pick: bool,
    pub rect: Option<egui::Rect>,
    buttons: [Option<egui::Rect>; 5],
}
impl Runtime {
    pub fn original(&self) -> Option<&[[u8; 4]]> {
        self.paint.as_ref().map(|p| p.original())
    }
    pub fn cancel(&mut self, pad: &mut Scratchpad) {
        if let Some(p) = self.paint.take() {
            pad.pixels = (*p).cancel();
        }
    }
    fn finish(&mut self, pad: &mut Scratchpad) -> Option<String> {
        let p = self.paint.take()?;
        let before = p.original().to_vec();
        let (pixels, error) = (*p).finish();
        if error.is_none() {
            self.history.record(before, &pixels)
        }
        pad.pixels = pixels;
        error
    }
}
fn image(pad: &Scratchpad) -> egui::ColorImage {
    egui::ColorImage::new(
        PAD_SIZE,
        pad.pixels
            .iter()
            .map(|p| {
                let c = drawer_core::brush_composite::Mode::Normal.compose(
                    pad.background,
                    *p,
                    1.0,
                    [0; 2],
                    0,
                );
                egui::Color32::from_rgba_unmultiplied(c[0], c[1], c[2], c[3])
            })
            .collect(),
    )
}
impl Drawer {
    pub(super) fn mixing_pad(&mut self, ctx: &egui::Context) {
        if !self.workspaces.store.current.mixer.visible {
            self.mixer.cancel(&mut self.workspaces.store.current.mixer);
            return;
        }
        let mut runtime = std::mem::take(&mut self.mixer);
        let snapshot = self.capture_workspace();
        let mut pad = self.workspaces.store.current.mixer.clone();
        let mut open = true;
        let mut chosen = None;
        let mut message = None;
        let allowed = !self.tool_shortcuts_blocked(ctx) && self.workspaces.is_focused();
        egui::Window::new("Local mixing palette").open(&mut open).resizable(false).vscroll(true).max_height((ctx.content_rect().height()-40.0).max(180.0)).show(ctx,|ui|{
        ui.label("Independent colors and undo. Shared mode freezes the canvas tool when a stroke starts.");
        let old_zoom=pad.zoom;
        ui.horizontal(|ui|{super::edit::slider(ui,&mut pad.zoom,0.25..=2.0,"Mixing view zoom","color-tables");let reset=ui.button("Reset mixing view");runtime.buttons[3]=Some(reset.rect);if reset.clicked(){pad.zoom=1.0}});
        if pad.zoom!=old_zoom{runtime.cancel(&mut pad);}
        ui.horizontal(|ui|{for(label,bg)in[("Transparent",[0;4]),("White",[255;4]),("Gray",[128,128,128,255]),("Black",[0,0,0,255])]{if ui.selectable_label(pad.background==bg,label).clicked(){pad.background=bg;}}});
        egui::ComboBox::from_id_salt("mixing-tool").selected_text(pad.tool_slot.and_then(|n|pad.slots.get(n)).map_or("Shared canvas tool",|p|p.name.as_str())).show_ui(ui,|ui|{ui.selectable_value(&mut pad.tool_slot,None,"Shared canvas tool");for(n,p)in pad.slots.iter().enumerate(){ui.selectable_value(&mut pad.tool_slot,Some(n),&p.name);}});
        ui.horizontal(|ui|{
            let register=ui.add_enabled(pad.slots.len()<8&&self.drawing.mode==super::precision::Mode::Paint,egui::Button::new("Register current tool in new mixing slot"));runtime.buttons[4]=Some(register.rect);if register.clicked(){if snapshot.input.clone_stamp.enabled{message=Some("Dedicated mixing slots do not store canvas clone sources".into())}else{match Preset::capture(&snapshot,self.selected,&format!("Mixing tool {}",pad.slots.len()+1)){Ok(mut p)=>{p.values[0]=p.values[0].min(64.0);pad.slots.push(p);pad.tool_slot=Some(pad.slots.len()-1);},Err(e)=>message=Some(e)}}}
            if let Some(n)=pad.tool_slot && ui.button("Replace mixing slot from current tool").clicked(){if snapshot.input.clone_stamp.enabled{message=Some("Choose a color tool before registering a dedicated mixing slot".into())}else{match Preset::capture(&snapshot,self.selected,&pad.slots[n].name){Ok(mut p)=>{p.values[0]=p.values[0].min(64.0);pad.slots[n]=p;},Err(e)=>message=Some(e)}}}
        });
        if let Some(n)=pad.tool_slot{super::edit::number(ui,&mut pad.slots[n].values[0],0.1..=64.0,0.1," mixing tool px size","color-tables");}
        ui.checkbox(&mut runtime.pick,"Pick mixed RGBA instead of painting");
        ui.horizontal(|ui|{let undo=ui.button("Undo mixing stroke");runtime.buttons[0]=Some(undo.rect);if undo.clicked(){runtime.cancel(&mut pad);runtime.history.undo(&mut pad.pixels);}let redo=ui.button("Redo mixing stroke");runtime.buttons[1]=Some(redo.rect);if redo.clicked(){runtime.cancel(&mut pad);runtime.history.redo(&mut pad.pixels);}let clear=ui.button("Clear mixing pixels");runtime.buttons[2]=Some(clear.rect);if clear.clicked(){runtime.cancel(&mut pad);let before=pad.pixels.clone();pad.pixels.fill([0;4]);runtime.history.record(before,&pad.pixels);}});
        let texture=runtime.texture.get_or_insert_with(||ctx.load_texture("local-mixing-pixels",image(&pad),egui::TextureOptions::NEAREST));texture.set(image(&pad),egui::TextureOptions::NEAREST);
        let scale=3.0*pad.zoom;let response=ui.add(egui::Image::new(&*texture).fit_to_exact_size(egui::vec2(PAD_SIZE[0]as f32*scale as f32,PAD_SIZE[1]as f32*scale as f32)).sense(egui::Sense::click_and_drag()));runtime.rect=Some(response.rect);
        let point=|pos:egui::Pos2|[(pos.x-response.rect.min.x)as f64/scale,(pos.y-response.rect.min.y)as f64/scale];
        let owning=response.is_pointer_button_down_on()||response.drag_stopped_by(egui::PointerButton::Primary)||response.clicked();
        if !allowed||ui.input(|i|!i.focused||i.key_pressed(egui::Key::Escape)||i.events.iter().any(|e|matches!(e,egui::Event::WindowFocused(false)|egui::Event::PointerGone))){runtime.cancel(&mut pad);}
        else if runtime.pick{runtime.cancel(&mut pad);if response.clicked()&&let Some(pos)=response.interact_pointer_pos(){let p=point(pos);let x=(p[0].floor()as usize).min(PAD_SIZE[0]-1);let y=(p[1].floor()as usize).min(PAD_SIZE[1]-1);chosen=Some(pad.pixels[y*PAD_SIZE[0]+x]);}}
        else if owning{for event in ui.input(|i|i.events.clone()){
            let mut sample=None;let mut finish=false;
            match event{
                egui::Event::PointerButton{pos,button:egui::PointerButton::Primary,pressed:true,..}if response.rect.contains(pos)=>{
                    runtime.cancel(&mut pad);
                    let preset=pad.tool_slot.and_then(|n|pad.slots.get(n)).cloned().map(Ok).unwrap_or_else(||Preset::capture(&snapshot,self.selected,"Shared mixing tool"));
                    match preset.and_then(|p|{let mut style=p.style(snapshot.color);if pad.tool_slot.is_none(){style.clone_stamp=snapshot.input.clone_stamp.resolve(point(pos))?;}style.pressure.device_response=snapshot.pressure.calibration(drawer_core::pointer_input::Source::MOUSE);style.shape.size=style.pattern.behavior.document_size(style.shape.size,scale*ctx.pixels_per_point()as f64)?;super::workspaces::PadPaint::begin(&pad.pixels,style,self.materials.library.clone())}){Ok(p)=>{runtime.paint=Some(Box::new(p));sample=Some(pos)},Err(e)=>message=Some(e)}
                },
                egui::Event::PointerMoved(pos)if runtime.paint.is_some()=>sample=Some(pos),
                egui::Event::PointerButton{pos,button:egui::PointerButton::Primary,pressed:false,..}if runtime.paint.is_some()=>{sample=Some(pos);finish=true},_=>{}
            }
            if let Some(pos)=sample&&let Some(p)=&mut runtime.paint{match p.sample(point(pos),ui.input(|i|(i.time.max(0.0)*1e6)as u64)){Ok(())=>pad.pixels=p.preview(),Err(e)=>{message=Some(e);runtime.cancel(&mut pad);}}}
            if finish&&let Some(e)=runtime.finish(&mut pad){message=Some(e)}
        }}
        else if runtime.paint.is_some(){runtime.cancel(&mut pad);}
        if let Some(p)=&mut runtime.paint{ctx.request_repaint_after(std::time::Duration::from_millis(16));match p.tick(ui.input(|i|(i.time.max(0.0)*1e6)as u64)){Ok(true)=>pad.pixels=p.preview(),Ok(false)=>{},Err(e)=>{message=Some(e);runtime.cancel(&mut pad);}}}
    });
        if !open {
            runtime.cancel(&mut pad)
        }
        pad.visible = open;
        if !pad.valid() {
            runtime.cancel(&mut pad);
            message = Some("Invalid mixing palette options; previous settings kept".into());
        } else {
            self.workspaces.store.current.mixer = pad;
        }
        self.mixer = runtime;
        if let Some(color) = chosen {
            self.choose_color(color)
        }
        if let Some(e) = message {
            self.status = Some(e)
        }
    }
}

fn click(p: egui::Pos2, pressed: bool) -> Vec<egui::Event> {
    vec![
        egui::Event::PointerMoved(p),
        egui::Event::PointerButton {
            pos: p,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        },
    ]
}
fn step(ctx: &egui::Context, app: &mut Drawer, time: &mut f64, events: Vec<egui::Event>) {
    *time += 0.1;
    super::input_replay::frame(ctx, app, *time, events);
}
fn button(ctx: &egui::Context, app: &mut Drawer, time: &mut f64, n: usize) -> Result<(), String> {
    let p = app.mixer.buttons[n]
        .ok_or("Mixing action button missing")?
        .center();
    step(ctx, app, time, click(p, true));
    step(ctx, app, time, click(p, false));
    Ok(())
}
pub(super) fn smoke(root: &std::path::Path) -> Result<(), String> {
    drawer_core::color_tables::verify()?;
    verify_mixer(root)?;
    verify_sampler()?;
    super::workspaces::verify_test_history(root)
}
fn verify_mixer(root: &std::path::Path) -> Result<(), String> {
    let ctx = egui::Context::default();
    let user = root.join("mixing-user");
    let mut app = Drawer::new(&ctx, None)
        .with_storage(UserStorage::at(user.clone()).map_err(|e| e.to_string())?);
    app.simple_view = true;
    app.create_canvas(CanvasSpec {
        width: 16,
        height: 16,
        dpi: 300.0,
        background: Background::Transparent,
    })
    .map_err(|e| e.to_string())?;
    drawer_core::files::save(
        &mut app.documents[0],
        &root.join("mixing-keeps-document.drawer"),
    )
    .map_err(|e| e.to_string())?;
    let original = app.documents[0].snapshot();
    app.references.session.workspace.color = [220, 70, 30, 128];
    app.workspaces.store.current.mixer.visible = true;
    let mut t = 0.0;
    step(&ctx, &mut app, &mut t, vec![]);
    step(&ctx, &mut app, &mut t, vec![]);
    let rect = app.mixer.rect.ok_or("Mixing canvas missing")?;
    let a = rect.min + egui::vec2(25.5, 25.5);
    let b = a + egui::vec2(12.0, 0.0);
    step(&ctx, &mut app, &mut t, click(a, true));
    if app.mixer.paint.is_none()
        || app
            .capture_workspace()
            .mixer
            .pixels
            .iter()
            .any(|p| *p != [0; 4])
    {
        return Err("Mixing preview was not isolated from saved pixels".into());
    }
    step(&ctx, &mut app, &mut t, vec![egui::Event::PointerMoved(b)]);
    step(&ctx, &mut app, &mut t, click(b, false));
    let painted = app.workspaces.store.current.mixer.pixels.clone();
    if !painted.iter().any(|p| p[3] > 0 && p[3] < 255) || app.mixer.history.counts() != (1, 0) {
        return Err("Mixing stroke/alpha/own undo missing".into());
    }
    button(&ctx, &mut app, &mut t, 0)?;
    if app
        .workspaces
        .store
        .current
        .mixer
        .pixels
        .iter()
        .any(|p| *p != [0; 4])
    {
        return Err("Mixing Undo failed".into());
    }
    button(&ctx, &mut app, &mut t, 1)?;
    if app.workspaces.store.current.mixer.pixels != painted {
        return Err("Mixing Redo failed".into());
    }
    button(&ctx, &mut app, &mut t, 2)?;
    button(&ctx, &mut app, &mut t, 0)?;
    if app.workspaces.store.current.mixer.pixels != painted {
        return Err("Clear mixing pixels was not one undo step".into());
    }
    app.workspaces.store.current.mixer.background = [255; 4];
    app.workspaces.store.current.mixer.zoom = 0.5;
    step(&ctx, &mut app, &mut t, vec![]);
    if app.workspaces.store.current.mixer.pixels != painted {
        return Err("Background/zoom changed mixing RGBA".into());
    }
    button(&ctx, &mut app, &mut t, 3)?;
    if app.workspaces.store.current.mixer.zoom != 1.0 {
        return Err("Reset mixing view failed".into());
    }
    button(&ctx, &mut app, &mut t, 4)?;
    if app.workspaces.store.current.mixer.slots.len() != 1 {
        return Err("Dedicated mixing tool not registered".into());
    }
    let dedicated = app.workspaces.store.current.mixer.slots[0].clone();
    app.set_tool(BrushTip::Pencil);
    step(&ctx, &mut app, &mut t, vec![]);
    if app.workspaces.store.current.mixer.slots[0] != dedicated
        || app.workspaces.store.current.mixer.tool_slot != Some(0)
    {
        return Err("Canvas tool switch overwrote dedicated mixing slot".into());
    }
    let rect = app.mixer.rect.unwrap();
    let p = rect.center();
    step(&ctx, &mut app, &mut t, click(p, true));
    step(
        &ctx,
        &mut app,
        &mut t,
        vec![egui::Event::WindowFocused(false)],
    );
    if app.workspaces.store.current.mixer.pixels != painted || app.mixer.paint.is_some() {
        return Err("Focus loss committed an unfinished mixing stroke".into());
    }
    step(
        &ctx,
        &mut app,
        &mut t,
        vec![egui::Event::WindowFocused(true)],
    );
    app.workspaces.store.current.mixer.zoom = 0.75;
    app.persist_workspace()?;
    let restart = Drawer::new(&egui::Context::default(), None)
        .with_storage(UserStorage::at(user).map_err(|e| e.to_string())?);
    if restart.capture_workspace().mixer != app.capture_workspace().mixer {
        return Err("Mixing pixels/tool/background/view restart mismatch".into());
    }
    if app.documents[0].snapshot() != original
        || app.documents[0].dirty
        || app.documents[0].history_counts() != (0, 0)
    {
        return Err("Mixing palette edited the real document".into());
    }
    Ok(())
}
fn verify_sampler() -> Result<(), String> {
    use drawer_core::pointer_input::Phase;
    let ctx = egui::Context::default();
    let mut app = Drawer::new(&ctx, None);
    app.simple_view = true;
    app.create_canvas(CanvasSpec {
        width: 16,
        height: 16,
        dpi: 300.0,
        background: Background::Transparent,
    })
    .map_err(|e| e.to_string())?;
    app.documents[0].layers[0].pixels[4 * 16 + 4] = [20, 80, 160, 128];
    app.documents[0].layers[0].pixels[8 * 16 + 8] = [220, 30, 40, 255];
    let before = app.documents[0].snapshot();
    app.references.session.workspace.color = [1, 2, 3, 255];
    app.workspaces.store.current.color_tables.auto_sample = true;
    app.workspaces.store.current.eyedropper.settings.source =
        drawer_core::eyedropper::Source::LayerRgba;
    let mut t = 0.0;
    step(&ctx, &mut app, &mut t, vec![]);
    step(&ctx, &mut app, &mut t, vec![]);
    app.execute_command(&ctx, "Eyedropper");
    let event = super::precision::pointer(&ctx, &app, [4.0, 4.0], Phase::Down);
    app.drawing.virtual_events(vec![event], false);
    step(&ctx, &mut app, &mut t, vec![]);
    if app.references.session.workspace.color != [1, 2, 3, 255]
        || !app.workspaces.store.current.color_tables.tables[0]
            .colors
            .is_empty()
    {
        return Err("Sampler registered an unconfirmed pointer-down preview".into());
    }
    let event = super::precision::pointer(&ctx, &app, [4.0, 4.0], Phase::Up);
    app.drawing.virtual_events(vec![event], false);
    step(&ctx, &mut app, &mut t, vec![]);
    step(&ctx, &mut app, &mut t, vec![]);
    if app.references.session.workspace.color != [20, 80, 160, 128]
        || app.workspaces.store.current.color_tables.tables[0]
            .colors
            .len()
            != 1
    {
        return Err("Confirmed sampler did not register one RGBA".into());
    }
    let event = super::precision::pointer(&ctx, &app, [8.0, 8.0], Phase::Down);
    app.drawing.virtual_events(vec![event], false);
    step(&ctx, &mut app, &mut t, vec![]);
    step(
        &ctx,
        &mut app,
        &mut t,
        vec![egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }],
    );
    let event = super::precision::pointer(&ctx, &app, [8.0, 8.0], Phase::Up);
    app.drawing.virtual_events(vec![event], false);
    step(&ctx, &mut app, &mut t, vec![]);
    step(&ctx, &mut app, &mut t, vec![]);
    if app.references.session.workspace.color != [20, 80, 160, 128]
        || app.workspaces.store.current.color_tables.tables[0]
            .colors
            .len()
            != 1
    {
        return Err("Cancelled sampler added a color".into());
    }
    let event = super::precision::pointer(&ctx, &app, [8.0, 8.0], Phase::Down);
    app.drawing.virtual_events(vec![event], false);
    step(&ctx, &mut app, &mut t, vec![]);
    app.documents[0].revision += 1;
    let event = super::precision::pointer(&ctx, &app, [8.0, 8.0], Phase::Up);
    app.drawing.virtual_events(vec![event], false);
    step(&ctx, &mut app, &mut t, vec![]);
    step(&ctx, &mut app, &mut t, vec![]);
    if app.references.session.workspace.color != [20, 80, 160, 128]
        || app.workspaces.store.current.color_tables.tables[0]
            .colors
            .len()
            != 1
    {
        return Err("A stale sampler revision was committed".into());
    }
    if app.documents[0].snapshot() != before || app.documents[0].history_counts() != (0, 0) {
        return Err("Sampler registration changed the document".into());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn independent_mixing_test_history_and_confirmed_sampler() {
        let root = crate::tests::TestDirectory::new();
        super::smoke(&root.0).unwrap();
    }
}
