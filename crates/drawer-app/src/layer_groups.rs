use super::*;
use drawer_core::{
    layer_catalog as catalog,
    layer_groups::{self as core, Command},
    layer_move::{Settings, Source},
    layer_search::Key,
};
use std::collections::BTreeSet;
#[derive(Default)]
pub(super) struct Panel {
    draft: Option<Box<Draft>>,
    preview: Option<egui::Rect>,
    apply: Option<egui::Rect>,
    rows: std::collections::BTreeMap<Key, egui::Rect>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Action {
    Save,
    Delete,
    Move,
    Transform,
    Filter,
    Show,
    Hide,
    Solo,
    Restore,
}
struct Draft {
    id: u64,
    revision: u64,
    frame: u32,
    group: Option<u64>,
    name: String,
    members: BTreeSet<Key>,
    linked: [bool; 3],
    action: Action,
    delta: [i64; 2],
    pivot: [f64; 2],
    angle: f64,
    scale: f64,
    flip: bool,
    filter: drawer_core::layer_effects::Adjustment,
    prepared: Option<Box<drawer_core::sizing::DocumentState>>,
    images: Option<[egui::TextureHandle; 2]>,
}
impl Drawer {
    pub(super) fn begin_layer_groups(&mut self) -> Result<(), String> {
        self.finish_stroke();
        let d = &self.documents[self.active.ok_or("Open a page first")?];
        let current = self
            .panels
            .targets
            .get(&d.id)
            .copied()
            .and_then(|t| catalog::from_thumbnail(d, t))
            .unwrap_or(Key::Layer(drawer_core::layer_properties::raster(d, 0)));
        let group = d
            .scene
            .layer_groups
            .groups
            .iter()
            .find(|g| g.members.contains(&current));
        self.layer_groups.draft = Some(Box::new(Draft {
            id: d.id,
            revision: d.revision,
            frame: d.current_frame,
            group: group.map(|g| g.id),
            name: group.map_or("Linked layers".into(), |g| g.name.clone()),
            members: group.map_or_else(
                || [current].into_iter().collect(),
                |g| g.members.iter().copied().collect(),
            ),
            linked: group.map_or([true; 3], |g| g.linked),
            action: if group.is_some() {
                Action::Move
            } else {
                Action::Save
            },
            delta: [0; 2],
            pivot: [d.width as f64 / 2., d.height as f64 / 2.],
            angle: 90.,
            scale: 1.,
            flip: false,
            filter: Default::default(),
            prepared: None,
            images: None,
        }));
        Ok(())
    }
    fn preview_layer_group(&mut self, ctx: &egui::Context) -> Result<(), String> {
        let e = self
            .layer_groups
            .draft
            .as_ref()
            .ok_or("Linked layer editor closed")?;
        let d = &self.documents[self.active.ok_or("Open a page first")?];
        if (d.id, d.revision, d.current_frame) != (e.id, e.revision, e.frame) {
            return Err("Page/frame changed; reopen linked layers".into());
        }
        let used: usize = self.documents.iter().map(Document::resident_bytes).sum();
        if used
            .saturating_add(d.memory_bytes().saturating_mul(5))
            .saturating_add(64 * 1024 * 1024)
            > DEFAULT_DOCUMENT_BUDGET
        {
            return Err("Linked preview exceeds workspace memory".into());
        }
        let keys = e.members.iter().copied().collect::<Vec<_>>();
        let state = match e.action {
            Action::Save => core::prepare(
                d,
                Command::Save {
                    id: e.group,
                    name: e.name.clone(),
                    members: keys,
                    linked: e.linked,
                },
            )?,
            Action::Delete => {
                core::prepare(d, Command::Delete(e.group.ok_or("Choose a saved group")?))?
            }
            Action::Move => core::move_linked(d, &keys, e.delta)?,
            Action::Transform => drawer_core::group_edits::transform(
                d,
                &keys,
                drawer_core::group_edits::matrix(e.pivot, e.angle, e.scale, e.flip)?,
            )?,
            Action::Filter => drawer_core::group_edits::filter(d, &keys, e.filter)?,
            Action::Show | Action::Hide => core::prepare(
                d,
                Command::Visibility {
                    members: keys,
                    hidden: e.action == Action::Hide,
                },
            )?,
            Action::Solo => core::prepare(d, Command::Solo(keys))?,
            Action::Restore => core::prepare(d, Command::Restore)?,
        };
        if used.saturating_add(state.memory_bytes().saturating_mul(4)) > DEFAULT_DOCUMENT_BUDGET {
            return Err("Linked result exceeds workspace memory".into());
        }
        let after = drawer_core::layer_merge::preview(&state, e.frame)?;
        let images = [
            super::layer_merge::image(ctx, d, "linked-before"),
            super::layer_merge::image(ctx, &after, "linked-after"),
        ];
        let e = self.layer_groups.draft.as_mut().unwrap();
        e.prepared = Some(Box::new(state));
        e.images = Some(images);
        Ok(())
    }
    fn apply_layer_group(&mut self) -> Result<(), String> {
        let n = self.active.ok_or("Open a page first")?;
        let e = self
            .layer_groups
            .draft
            .as_mut()
            .ok_or("Linked layer editor closed")?;
        let d = &self.documents[n];
        if (d.id, d.revision, d.current_frame) != (e.id, e.revision, e.frame) {
            return Err("Page/frame changed; reopen linked layers".into());
        }
        let state = e
            .prepared
            .take()
            .ok_or("Preview the linked operation first")?;
        self.documents[n].commit_state(*state)?;
        self.layer_groups.draft = None;
        self.sync_layer_identity();
        Ok(())
    }
    pub(super) fn layer_groups_window(&mut self, ctx: &egui::Context) {
        let Some(mut e) = self.layer_groups.draft.take() else {
            return;
        };
        let Some(n) = self.active.filter(|n| self.documents[*n].id == e.id) else {
            return;
        };
        let d = &self.documents[n];
        let fresh = (d.revision, d.current_frame) == (e.revision, e.frame);
        let mut open = !ctx.input(|i| i.key_pressed(egui::Key::Escape));
        let (mut preview, mut apply, mut cancel) = (false, false, false);
        egui::Window::new("Linked layers, transforms and visibility").open(&mut open).default_pos(egui::pos2(25.,15.)).default_width(560.).collapsible(false).show(ctx,|ui|{
            if !fresh{ui.label("Page/frame changed; cancel and reopen linked layers.");return;}
            let mut changed=false;
            egui::ScrollArea::vertical().id_salt("linked-controls-body").max_height(330.).show(ui,|ui|{
            ui.horizontal(|ui|{egui::ComboBox::from_label("Group").selected_text(e.group.and_then(|id|d.scene.layer_groups.groups.iter().find(|g|g.id==id)).map_or("New group",|g|g.name.as_str())).show_ui(ui,|ui|{for g in &d.scene.layer_groups.groups{if ui.selectable_label(e.group==Some(g.id),&g.name).clicked(){e.group=Some(g.id);e.name=g.name.clone();e.members=g.members.iter().copied().collect();e.linked=g.linked;changed=true;}}});if ui.button("New group").clicked(){e.group=None;e.name="Linked layers".into();changed=true;}});
            changed|=ui.text_edit_singleline(&mut e.name).changed();
            ui.horizontal(|ui|{for(i,label)in["Link movement","Link transforms","Link ink filters"].iter().enumerate(){changed|=ui.checkbox(&mut e.linked[i],*label).changed();}});
            self.layer_groups.rows.clear();
            if let Ok(rows)=catalog::rows(d){egui::ScrollArea::vertical().max_height(110.).show(ui,|ui|{for row in rows.into_iter().filter(|r|core::canonical(r.key)){
                let mut selected=e.members.contains(&row.key);let r=ui.checkbox(&mut selected,format!("{}  [{:?}]",row.name,row.kind));self.layer_groups.rows.insert(row.key,r.rect);if r.changed(){changed=true;if selected{e.members.insert(row.key);}else{e.members.remove(&row.key);}}
            }});}
            ui.small("One layer/subtree belongs to one group. Choose a folder or its children, rather than both. Linked members outside the checked set join that operation; directly selected hidden layers or any locked member reject it as a whole. Movement/transforms retain hidden folder children; show them before ink filtering. Save group to record changed membership/link flags.");
            ui.horizontal_wrapped(|ui|{for(action,label)in[(Action::Save,"Save group"),(Action::Delete,"Delete group"),(Action::Move,"Move"),(Action::Transform,"Transform"),(Action::Filter,"Ink filter"),(Action::Show,"Show all"),(Action::Hide,"Hide all"),(Action::Solo,"Solo"),(Action::Restore,"Restore visibility")]{changed|=ui.radio_value(&mut e.action,action,label).changed();}});
            match e.action{
                Action::Move=>{ui.horizontal(|ui|{for(k,label)in["Move X ","Move Y "].iter().enumerate(){changed|=ui.add(egui::DragValue::new(&mut e.delta[k]).range(-1_000_000..=1_000_000).prefix(*label)).changed();}});},
                Action::Transform=>{ui.horizontal(|ui|{changed|=ui.add(egui::DragValue::new(&mut e.angle).range(-360. ..=360.).prefix("Degrees ")).changed();changed|=ui.add(egui::DragValue::new(&mut e.scale).range(0.0625..=16.).prefix("Scale ")).changed();changed|=ui.checkbox(&mut e.flip,"Flip X").changed();});ui.horizontal(|ui|{for(k,label)in["Pivot X ","Pivot Y "].iter().enumerate(){changed|=ui.add(egui::DragValue::new(&mut e.pivot[k]).range(-1_000_000. ..=1_000_000.).prefix(*label)).changed();}});ui.small("Raster pixels use nearest sampling. Vector points, material/gradient frames, linked masks, texture and rulers follow the transform. Rasterize unsupported editable text/3D/animation members first.");},
                Action::Filter=>{for(v,label,range)in[(&mut e.filter.brightness,"Brightness",-1. ..=1.),(&mut e.filter.contrast,"Contrast",-1. ..=1.),(&mut e.filter.hue,"Hue",-180. ..=180.),(&mut e.filter.saturation,"Saturation",-1. ..=1.)]{changed|=ui.add(egui::Slider::new(v,range).text(label)).changed();}ui.small("Adjusts stored raster and vector ink RGB, keeping alpha and geometry. Other source kinds need rasterization first.");},
                Action::Solo|Action::Restore=>{ui.small("Solo preserves a folder's hidden children and restores recorded states by stable ID. New layers keep their current state; deleted layers are skipped.");},_=>{}
            }
            });
            if changed{e.prepared=None;e.images=None;}
            let r=ui.button("Preview linked operation");self.layer_groups.preview=Some(r.rect);preview=r.clicked();
            ui.allocate_ui(egui::vec2(ui.available_width(),110.),|ui|{if let Some(images)=&e.images{ui.horizontal(|ui|{for(label,image)in[("Current",&images[0]),("After linked operation",&images[1])]{ui.vertical(|ui|{ui.label(label);ui.add(egui::Image::new(image).fit_to_exact_size(egui::vec2(80.,80.)));});}});}else{ui.label("Preview appears here.");}});
            ui.horizontal(|ui|{let r=ui.add_enabled(e.prepared.is_some(),egui::Button::new("Apply linked operation"));self.layer_groups.apply=Some(r.rect);apply=r.clicked();cancel=ui.button("Cancel").clicked();});
        });
        if open && !cancel {
            self.layer_groups.draft = Some(e);
        }
        let r = if preview {
            self.preview_layer_group(ctx)
        } else if apply {
            self.apply_layer_group()
        } else {
            Ok(())
        };
        if let Err(e) = r {
            self.status = Some(e);
        }
    }
    pub(super) fn layer_move_controls(&mut self, ui: &mut egui::Ui) {
        if self.drawing.mode != super::precision::Mode::Move {
            return;
        }
        let mut s = self.workspaces.store.current.layer_move;
        let old = s;
        ui.collapsing("Layer movement settings",|ui|{
            ui.add_enabled_ui(self.precision.target==drawer_core::precision::Target::Layer,|ui|{ui.horizontal(|ui|{for(value,label)in[(Source::Current,"Current layer"),(Source::Click,"Clicked layer"),(Source::Area,"Layers touching selection")]{ui.radio_value(&mut s.source,value,label);}});});
            ui.horizontal(|ui|{ui.checkbox(&mut s.auto_select,"Auto-select moved layer");ui.add_enabled(self.precision.target!=drawer_core::precision::Target::Boundary,egui::Checkbox::new(&mut s.keep_original,"Keep original; move a new copy"));});
            ui.horizontal_wrapped(|ui|{for(value,label)in[(&mut s.exclude_text,"Exclude text"),(&mut s.exclude_balloon,"Exclude balloons"),(&mut s.exclude_draft,"Exclude drafts"),(&mut s.exclude_locked,"Exclude locked"),(&mut s.exclude_texture,"Exclude texture/effects"),(&mut s.exclude_image,"Exclude image materials"),(&mut s.exclude_fill,"Exclude fill layers")]{ui.checkbox(value,label);}});
            ui.small("Layer targeting moves entire chosen layers, including parts outside the selection. Selected-pixel movement edits only the current raster selection; Keep original copies those pixels to a new layer. Escape/focus or setting changes cancel the preview.");
            if ui.button("Reset movement settings").clicked(){s=Settings::default();}
        });
        if s != old {
            self.finish_stroke();
            self.workspaces.store.current.layer_move = s;
        }
    }
}

pub(super) fn smoke(root: &std::path::Path) -> Result<(), String> {
    drawer_core::layer_groups_verify::verify(root)?;
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
    app.documents[0].commit_state(drawer_core::layer_groups_verify::fixture()?.snapshot())?;
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
    app.execute_command(&ctx, "layer-groups");
    step(&mut app, &mut time, vec![]);
    step(&mut app, &mut time, vec![]);
    let keys = drawer_core::layer_groups_verify::keys(&app.documents[0]);
    let r = *app
        .layer_groups
        .rows
        .get(&keys[1])
        .ok_or("Group member checkbox missing")?;
    click(&mut app, &mut time, r);
    let original = app.documents[0].snapshot();
    let counts = app.documents[0].history_counts();
    let r = app.layer_groups.preview.ok_or("Group preview missing")?;
    click(&mut app, &mut time, r);
    if app.documents[0].snapshot() != original
        || app
            .layer_groups
            .draft
            .as_ref()
            .is_none_or(|e| e.prepared.is_none())
    {
        return Err(format!("Group UI preview/creation: {:?}", app.status));
    }
    step(&mut app, &mut time, vec![]);
    step(&mut app, &mut time, vec![]);
    let r = app.layer_groups.apply.ok_or("Group apply missing")?;
    click(&mut app, &mut time, r);
    if app.layer_groups.draft.is_some()
        || app.documents[0].scene.layer_groups.groups.len() != 1
        || app.documents[0].history_counts().0 != counts.0 + 1
    {
        return Err(format!(
            "Group UI save/Undo: {:?}; open={} groups={} counts={:?}/{:?}, apply={:?}",
            app.status,
            app.layer_groups.draft.is_some(),
            app.documents[0].scene.layer_groups.groups.len(),
            app.documents[0].history_counts(),
            counts,
            app.layer_groups.apply
        ));
    }
    let saved = app.documents[0].snapshot();
    if !app.documents[0].undo()
        || app.documents[0].snapshot() != original
        || !app.documents[0].redo()
    {
        return Err("Group UI save Undo/Redo".into());
    }
    for action in [Action::Move, Action::Transform, Action::Filter] {
        app.execute_command(&ctx, "layer-groups");
        {
            let e = app.layer_groups.draft.as_mut().unwrap();
            e.action = action;
            e.delta = [2, 1];
            e.angle = 90.;
            e.pivot = [4., 4.];
            e.filter.brightness = 0.1;
        }
        step(&mut app, &mut time, vec![]);
        step(&mut app, &mut time, vec![]);
        let r = app.layer_groups.preview.unwrap();
        click(&mut app, &mut time, r);
        step(&mut app, &mut time, vec![]);
        if app
            .layer_groups
            .draft
            .as_ref()
            .is_none_or(|e| e.prepared.is_none())
            || app.documents[0].snapshot() != saved
        {
            return Err(format!(
                "Group operation preview mutated original or failed: {:?}",
                app.status
            ));
        }
        step(&mut app, &mut time, vec![]);
        let r = app.layer_groups.apply.unwrap();
        click(&mut app, &mut time, r);
        if app.layer_groups.draft.is_some()
            || app.documents[0].snapshot() == saved
            || !app.documents[0].undo()
            || app.documents[0].snapshot() != saved
        {
            return Err(format!(
                "Group UI operation/Undo: {:?}; action={action:?}, open={}, counts={:?}, rect={:?}",
                app.status,
                app.layer_groups.draft.is_some(),
                app.documents[0].history_counts(),
                app.layer_groups.apply
            ));
        }
    }
    app.execute_command(&ctx, "layer-groups");
    app.layer_groups.draft.as_mut().unwrap().action = Action::Solo;
    app.preview_layer_group(&ctx)?;
    app.apply_layer_group()?;
    if !drawer_core::layer_properties::properties(
        &app.documents[0],
        drawer_core::layer_properties::Target::Raster(2),
    )
    .hidden
    {
        return Err("Solo UI did not hide outsider".into());
    }
    app.execute_command(&ctx, "layer-groups");
    app.layer_groups.draft.as_mut().unwrap().action = Action::Restore;
    app.preview_layer_group(&ctx)?;
    app.apply_layer_group()?;
    if app.documents[0].snapshot() != saved {
        return Err("Solo UI restore did not restore original states".into());
    }
    app.execute_command(&ctx, "layer-groups");
    app.layer_groups.draft.as_mut().unwrap().delta = [1, 0];
    app.preview_layer_group(&ctx)?;
    app.documents[0].current_frame += 1;
    let original = app.documents[0].snapshot();
    if app.apply_layer_group().is_ok() || app.documents[0].snapshot() != original {
        return Err("Stale group operation applied".into());
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
    if app.layer_groups.draft.is_some() {
        return Err("Group Escape failed".into());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn linked_ui_save_move_transform_filter_solo_undo_stale_escape() {
        let root = crate::tests::TestDirectory::new();
        super::smoke(&root.0).unwrap();
    }
}
