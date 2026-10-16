use super::*;
use drawer_core::soft_proof::{Intent, Proof, Settings, Tone};
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Target {
    View(u64),
    Display,
}
struct Editor {
    target: Target,
    draft: Settings,
}
#[derive(Default)]
pub(super) struct Runtime {
    editor: Option<Editor>,
    preview: Option<(Target, Settings)>,
    buttons: [Option<egui::Rect>; 4],
    error: Option<String>,
}
impl Drawer {
    fn stored_proof(&self, target: Target) -> Settings {
        match target {
            Target::View(id) => self
                .tabs
                .iter()
                .find(|t| t.id == id)
                .map(|t| t.proof.clone())
                .unwrap_or_default(),
            Target::Display => self.display.preferences.proof.clone(),
        }
    }
    pub(super) fn proof_for(&self, target: Target) -> Settings {
        self.proof_editor
            .preview
            .as_ref()
            .filter(|(t, _)| *t == target)
            .map_or_else(|| self.stored_proof(target), |(_, s)| s.clone())
    }
    pub(super) fn prepare_proof_views(&mut self) {
        let main = self
            .active_tab
            .map(|i| self.proof_for(Target::View(self.tabs[i].id)))
            .unwrap_or_default();
        self.canvas_view.set_proof(&main);
        self.display
            .canvas
            .set_proof(&self.proof_for(Target::Display));
    }
    pub(super) fn open_proof(&mut self) {
        let target = self
            .active_tab
            .map_or(Target::Display, |i| Target::View(self.tabs[i].id));
        self.edit_proof(target);
    }
    fn edit_proof(&mut self, target: Target) {
        self.proof_editor = Runtime {
            editor: Some(Editor {
                target,
                draft: self.stored_proof(target),
            }),
            ..Default::default()
        };
    }
    fn reload_proof_target(&mut self, target: Target) {
        match target {
            Target::View(id) => {
                self.canvas_view.reload_proof();
                self.layout.reload_proof(id);
            }
            Target::Display => self.display.canvas.reload_proof(),
        }
    }
    fn save_proof(&mut self, target: Target, settings: Settings) -> Result<(), String> {
        Proof::read(&settings)?;
        self.reload_proof_target(target);
        match target {
            Target::View(id) => {
                self.tabs
                    .iter_mut()
                    .find(|t| t.id == id)
                    .ok_or("The preview view was closed")?
                    .proof = settings;
            }
            Target::Display => {
                let mut next = self.display.preferences.clone();
                next.proof = settings;
                self.display.apply(next, self.storage.as_ref(), true)?;
            }
        }
        Ok(())
    }
    fn bake_tone(&mut self, target: Target, tone: Tone) -> Result<(), String> {
        if let Target::View(id) = target
            && self.active_tab.is_none_or(|i| self.tabs[i].id != id)
        {
            return Err("Activate the preview's canvas view before applying tones".into());
        }
        self.finish_stroke();
        let index = self
            .active
            .ok_or("Open a document and select a raster layer")?;
        let doc = &mut self.documents[index];
        let layer = match self
            .panels
            .targets
            .get(&doc.id)
            .copied()
            .unwrap_or(drawer_core::thumbnail::Target::Raster(0))
        {
            drawer_core::thumbnail::Target::Raster(n) => n,
            _ => return Err("Select a raster layer for tone application".into()),
        };
        #[cfg(windows)]
        let native = self.native_mouse.clone();
        let changed = drawer_core::soft_proof::apply(doc, layer, tone, || {
            #[cfg(windows)]
            {
                native.as_ref().is_some_and(|n| n.cancel_requested())
            }
            #[cfg(not(windows))]
            {
                false
            }
        })?;
        // Baking tones must not show the same adjustment twice. ICC proof stays view-only.
        let mut s = self.stored_proof(target);
        s.tone = Tone::default();
        let mut warning = None;
        match target {
            Target::View(id) => {
                if let Some(t) = self.tabs.iter_mut().find(|t| t.id == id) {
                    t.proof = s;
                }
            }
            Target::Display => {
                let mut next = self.display.preferences.clone();
                next.proof = s;
                if let Err(e) = self
                    .display
                    .apply(next.clone(), self.storage.as_ref(), true)
                {
                    self.display.preferences = next;
                    warning = Some(e);
                }
            }
        }
        self.status = Some(
            if changed {
                "Tone/levels applied to selected raster as one drawing Undo; ICC profile unchanged"
            } else {
                "Tone/levels made no pixel changes"
            }
            .into(),
        );
        if let Some(e) = warning {
            self.status = Some(format!(
                "Tone pixels applied; preview settings save failed: {e}"
            ));
        }
        Ok(())
    }
    pub(super) fn proof_window(&mut self, ctx: &egui::Context) {
        let Some(mut editor) = self.proof_editor.editor.take() else {
            return;
        };
        let mut open = true;
        let mut action = None;
        let mut target = None;
        egui::Window::new("View color proof and tone preview").id(egui::Id::new("soft-proof-editor")).open(&mut open).vscroll(true).default_size([660.0,1000.0]).max_height((ctx.content_rect().height()-40.0).max(160.0)).resizable(true).show(ctx,|ui|{
            ui.horizontal_wrapped(|ui|{if ui.button("Edit active canvas view").clicked()&&let Some(i)=self.active_tab{target=Some(Target::View(self.tabs[i].id));}
            if ui.button("Edit separate display view").clicked(){target=Some(Target::Display);}});
            ui.label(match editor.target{Target::View(id)=>format!("Canvas view {id} · working space sRGB"),Target::Display=>"Separate display view · working space sRGB".into()});
            let s=&mut editor.draft;ui.checkbox(&mut s.enabled,"Enable view-only proof / tones");
            ui.horizontal_wrapped(|ui|{ui.label(s.profile.as_ref().map_or("ICC: none (sRGB working color)".into(),|p|p.display().to_string()));if ui.button("Choose local ICC…").clicked()&&let Some(p)=rfd::FileDialog::new().add_filter("ICC profile",&["icc","icm"]).pick_file(){s.profile=Some(p);}
            if ui.button("Clear ICC").clicked(){s.profile=None;}});
            egui::ComboBox::from_id_salt("proof-intent").selected_text(format!("{:?}",s.intent)).show_ui(ui,|ui|{for intent in [Intent::Perceptual,Intent::Relative,Intent::Saturation,Intent::Absolute]{ui.selectable_value(&mut s.intent,intent,format!("{intent:?}"));}});
            ui.label("ICC is a view-only working → target → sRGB round trip. Profiles need both directions; no assumed fallback.");
            ui.heading("Tone curve (five fixed input positions)");for (n,v) in s.tone.curve.iter_mut().enumerate(){super::edit::slider(ui,v,0.0..=1.0,format!("Input {}% output",n*25),"soft-proof");}
            ui.heading("Levels");for (v,label,range) in [(&mut s.tone.black,"Input black",0.0..=1.0),(&mut s.tone.white,"Input white",0.0..=1.0),(&mut s.tone.gamma,"Gamma",0.1..=10.0),(&mut s.tone.output[0],"Output black",0.0..=1.0)]{super::edit::slider(ui,v,range,label,"soft-proof");}
            super::edit::slider(ui,&mut s.tone.output[1],0.0..=1.0,"Output white","soft-proof");
            if ui.button("Reset tones to identity").clicked(){s.tone=Tone::default();}
            ui.label("Preview draft changes display only. Save keeps this view's settings locally. Cancel restores the saved view. Applying tones edits only the selected raster/selection, preserves alpha and sRGB, and resets this view's tone preview; ICC simulation is never baked.");
            for (n,label) in ["Preview draft in this view","Save view preview settings","Cancel preview","Apply tone/levels to selected raster"].iter().enumerate(){let r=ui.add_enabled(n==2||s.valid(),egui::Button::new(*label));self.proof_editor.buttons[n]=Some(r.rect);if r.clicked(){action=Some(n);}}
            if let Some(e)=&self.proof_editor.error{ui.colored_label(egui::Color32::LIGHT_RED,e);}
        });
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) || !open {
            action = Some(2);
        }
        if let Some(t) = target {
            self.edit_proof(t);
            ctx.request_repaint();
            return;
        }
        let result = match action {
            Some(0) => Proof::read(&editor.draft).map(|_| {
                self.reload_proof_target(editor.target);
                self.proof_editor.error = None;
                self.proof_editor.preview = Some((editor.target, editor.draft.clone()));
            }),
            Some(1) => self.save_proof(editor.target, editor.draft.clone()),
            Some(3) => self.bake_tone(editor.target, editor.draft.tone),
            _ => Ok(()),
        };
        if let Err(e) = result {
            self.proof_editor.error = Some(e);
            self.proof_editor.editor = Some(editor);
        } else if matches!(action, Some(1..=3)) {
            self.proof_editor = Default::default();
        } else {
            self.proof_editor.editor = Some(editor);
        }
        if action.is_some() {
            ctx.request_repaint();
        }
    }
}

fn frame(ctx: &egui::Context, app: &mut Drawer, time: f64, events: Vec<egui::Event>) {
    let mut result = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1000.0, 1400.0),
            )),
            time: Some(time),
            events,
            ..Default::default()
        },
        |ui| app.show(ui),
    );
    result.textures_delta.clear();
}
fn click(p: egui::Pos2, pressed: bool) -> Vec<egui::Event> {
    vec![
        egui::Event::PointerMoved(p),
        egui::Event::PointerButton {
            pos: p,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        },
    ]
}
fn press(
    ctx: &egui::Context,
    app: &mut Drawer,
    time: &mut f64,
    action: usize,
) -> Result<(), String> {
    frame(ctx, app, *time, vec![]);
    *time += 0.1;
    frame(ctx, app, *time, vec![]);
    *time += 0.1;
    let p = app.proof_editor.buttons[action]
        .ok_or("Proof action missing")?
        .center();
    frame(ctx, app, *time, click(p, true));
    *time += 0.1;
    frame(ctx, app, *time, click(p, false));
    *time += 0.1;
    Ok(())
}
pub(super) fn smoke(root: &std::path::Path) -> Result<(), String> {
    let ctx = egui::Context::default();
    let user = root.join("soft-proof-user");
    let profile = root.join("own-gray-proof.icc");
    let bytes = mox_profile();
    std::fs::write(&profile, &bytes).map_err(|e| e.to_string())?;
    let mut app = Drawer::new(&ctx, None)
        .with_storage(UserStorage::at(user.clone()).map_err(|e| e.to_string())?);
    app.create_canvas(CanvasSpec {
        width: 8,
        height: 8,
        dpi: 300.0,
        background: Background::Transparent,
    })
    .map_err(|e| e.to_string())?;
    let color = [200, 40, 80, 128];
    app.documents[0].layers[0].pixels.fill(color);
    drawer_core::files::save(&mut app.documents[0], &root.join("proof-original.drawer"))
        .map_err(|e| e.to_string())?;
    let before = app.documents[0].snapshot();
    app.duplicate_view();
    let proof = Settings {
        enabled: true,
        profile: Some(profile.clone()),
        tone: Tone {
            gamma: 1.25,
            ..Default::default()
        },
        ..Default::default()
    };
    app.tabs[0].proof = proof.clone();
    app.select_tab(0);
    let expected = Proof::read(&proof)?.map(color);
    let mut time = 0.0;
    frame(&ctx, &mut app, time, vec![]);
    time += 0.1;
    frame(&ctx, &mut app, time, vec![]);
    time += 0.1;
    if app.canvas_view.preview_sample != Some((color, expected)) || expected == color {
        return Err("Actual view ICC/tone preview did not affect texture pixels".into());
    }
    app.workspaces.store.current.layout.arrangement = drawer_core::layout::Arrangement::Tile;
    frame(&ctx, &mut app, time, vec![]);
    time += 0.1;
    frame(&ctx, &mut app, time, vec![]);
    time += 0.1;
    if app.layout.proof_sample(app.tabs[0].id) != Some((color, expected))
        || app.layout.proof_sample(app.tabs[1].id) != Some((color, color))
    {
        return Err("Tiled views shared proof settings".into());
    }
    app.workspaces.store.current.layout.arrangement = drawer_core::layout::Arrangement::Tabs;
    app.interpolation.view = drawer_core::interpolation::Filter::Bilinear;
    frame(&ctx, &mut app, time, vec![]);
    time += 0.1;
    if app
        .canvas_view
        .preview_sample
        .is_none_or(|(raw, shown)| shown != Proof::read(&proof).unwrap().map(raw))
    {
        return Err("Interpolated canvas skipped view proof".into());
    }
    let raw = drawer_core::eyedropper::Settings::default().sample(
        &app.documents[0],
        None,
        [2.5, 2.5],
        [0; 4],
    )?;
    if raw != Some(color) {
        return Err("View proof changed canvas sampling".into());
    }
    app.interpolation.view = drawer_core::interpolation::Filter::Nearest;
    app.select_tab(1);
    frame(&ctx, &mut app, time, vec![]);
    time += 0.1;
    if app.canvas_view.preview_sample != Some((color, color)) {
        return Err("Independent view reused proofed texture".into());
    }
    app.select_tab(0);
    app.open_proof();
    app.proof_editor.editor.as_mut().unwrap().draft.tone.curve = [1.0; 5];
    press(&ctx, &mut app, &mut time, 0)?;
    frame(&ctx, &mut app, time, vec![]);
    time += 0.1;
    if app.proof_editor.preview.is_none()
        || app.tabs[0].proof != proof
        || app.capture_views(1.0).rows[0].proof != proof
    {
        return Err(format!(
            "Draft preview/saved settings: preview={}, tab_kept={}, layout_kept={}, error={:?}, button={:?}",
            app.proof_editor.preview.is_some(),
            app.tabs[0].proof == proof,
            app.capture_views(1.0).rows[0].proof == proof,
            app.proof_editor.error,
            app.proof_editor.buttons[0]
        ));
    }
    press(&ctx, &mut app, &mut time, 2)?;
    frame(&ctx, &mut app, time, vec![]);
    time += 0.1;
    if app.proof_editor.editor.is_some()
        || app.canvas_view.preview_sample != Some((color, expected))
    {
        return Err("Cancel preview did not restore prior display".into());
    }
    app.open_proof();
    app.proof_editor.editor.as_mut().unwrap().draft.enabled = false;
    press(&ctx, &mut app, &mut time, 1)?;
    frame(&ctx, &mut app, time, vec![]);
    time += 0.1;
    if app.tabs[0].proof.enabled || app.canvas_view.preview_sample != Some((color, color)) {
        return Err("Saving disabled proof did not restore document display".into());
    }
    app.open_proof();
    app.proof_editor.editor.as_mut().unwrap().draft = proof.clone();
    press(&ctx, &mut app, &mut time, 1)?;
    let other = Target::View(app.tabs[1].id);
    if app
        .bake_tone(
            other,
            Tone {
                gamma: 2.0,
                ..Default::default()
            },
        )
        .is_ok()
    {
        return Err("Tone application accepted another view's active document".into());
    }
    if app.documents[0].snapshot() != before
        || app.documents[0].history_counts() != (0, 0)
        || app.documents[0].dirty
    {
        return Err("Proof preview changed document or drawing history".into());
    }
    app.open_proof();
    app.proof_editor.editor.as_mut().unwrap().draft.profile = Some(root.join("missing-proof.icc"));
    press(&ctx, &mut app, &mut time, 1)?;
    if app.proof_editor.error.is_none() || app.tabs[0].proof != proof {
        return Err("Missing ICC silently replaced saved preview".into());
    }
    press(&ctx, &mut app, &mut time, 2)?;
    app.select_tab(1);
    app.open_proof();
    let tone = Tone {
        gamma: 2.0,
        ..Default::default()
    };
    app.proof_editor.editor.as_mut().unwrap().draft.tone = tone;
    press(&ctx, &mut app, &mut time, 3)?;
    if app.proof_editor.editor.is_some()
        || app.documents[0].layers[0].pixels[0] != tone.map(color)
        || app.documents[0].history_counts() != (1, 0)
    {
        return Err(format!("Actual apply tone action failed: {:?}", app.status));
    }
    let after = app.documents[0].snapshot();
    if !app.documents[0].undo()
        || app.documents[0].snapshot() != before
        || !app.documents[0].redo()
        || app.documents[0].snapshot() != after
    {
        return Err("Explicit tone command undo/redo failed".into());
    }
    if std::fs::read(&profile).map_err(|e| e.to_string())? != bytes {
        return Err("Preview/application wrote the ICC source".into());
    }
    drawer_core::files::save(&mut app.documents[0], &root.join("proof-applied.drawer"))
        .map_err(|e| e.to_string())?;
    let mut display = app.display.preferences.clone();
    display.proof = Settings {
        enabled: true,
        tone: Tone {
            curve: [1.0, 0.75, 0.5, 0.25, 0.0],
            ..Default::default()
        },
        ..Default::default()
    };
    app.display
        .apply(display.clone(), app.storage.as_ref(), true)?;
    app.prepare_proof_views();
    app.persist_views(1.0);
    app.persist_workspace()?;
    let layout = app.capture_views(1.0);
    let mut restarted = Drawer::new(&egui::Context::default(), None)
        .with_storage(UserStorage::at(user).map_err(|e| e.to_string())?);
    restarted.restore_views(layout.clone(), 1.0)?;
    if restarted.tabs.len() != 2
        || restarted.tabs[0].proof != layout.rows[0].proof
        || restarted.tabs[1].proof != layout.rows[1].proof
        || restarted.display.preferences.proof != display.proof
    {
        return Err("Independent view/display proof restart mismatch".into());
    }
    let mut missing = proof;
    missing.profile = Some(root.join("missing-again.icc"));
    restarted.canvas_view.set_proof(&missing);
    if restarted.canvas_view.proof_error.is_none() {
        return Err("Missing ICC lacks visible canvas diagnostic".into());
    }
    Ok(())
}
fn mox_profile() -> Vec<u8> {
    drawer_core::soft_proof::gray_fixture()
}
#[cfg(test)]
mod tests {
    #[test]
    fn actual_view_preview_cancel_missing_apply_undo_and_restart() {
        let root = crate::tests::TestDirectory::new();
        super::smoke(&root.0).unwrap();
    }
}
