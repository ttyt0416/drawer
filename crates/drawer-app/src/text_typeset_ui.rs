use super::*;
use drawer_core::text_typeset as ty;
#[derive(Default)]
pub(super) struct Panel {
    tool: Option<Box<ty::Tool>>,
    first: usize,
    last: usize,
    error: Option<String>,
    pub enable: Option<egui::Rect>,
}
fn choice<T: Copy + PartialEq + std::fmt::Debug>(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut T,
    values: &[T],
) {
    egui::ComboBox::from_id_salt(label)
        .selected_text(format!("{label}: {value:?}"))
        .show_ui(ui, |ui| {
            for v in values {
                ui.selectable_value(value, *v, format!("{v:?}"));
            }
        });
}
fn options(ui: &mut egui::Ui, s: &mut Settings, p: &mut Panel) {
    let Some(o) = &mut s.style.typeset else {
        return;
    };
    choice(
        ui,
        "Kerning",
        &mut o.kerning,
        &[ty::Kerning::Manual, ty::Kerning::Metrics],
    );
    ui.add_enabled(
        o.kerning == ty::Kerning::Manual,
        egui::DragValue::new(&mut o.manual)
            .range(-0.5..=4.)
            .speed(0.01)
            .prefix("Manual kerning em "),
    );
    for f in [ty::Feature::Ligatures, ty::Feature::ContextualAlternates] {
        let mut selected =
            o.features.contains(&f) || (f == ty::Feature::Ligatures && s.style.ligatures);
        if ui.checkbox(&mut selected, format!("{f:?}")).changed() {
            if f == ty::Feature::Ligatures {
                s.style.ligatures = selected;
            }
            if selected {
                o.features.push(f)
            } else {
                o.features.retain(|v| *v != f)
            }
        }
    }
    ui.checkbox(&mut o.justify, "Justify wrapped lines");
    choice(
        ui,
        "Last line",
        &mut o.last,
        &[
            ty::LastLine::Left,
            ty::LastLine::Center,
            ty::LastLine::Right,
            ty::LastLine::Top,
            ty::LastLine::Bottom,
        ],
    );
    ui.checkbox(
        &mut o.half_punctuation,
        "Half-em Japanese punctuation spacing",
    );
    ui.horizontal(|ui| {
        ui.label("Character background RGBA");
        ui.color_edit_button_srgba_unmultiplied(&mut o.background);
    });
    ui.label("Background alpha is independent of text and outline. Zero alpha hides it.");
    ui.horizontal(|ui| {
        ui.add(
            egui::DragValue::new(&mut p.first)
                .range(0..=4096)
                .prefix("Horizontal-in-vertical start "),
        );
        ui.add(
            egui::DragValue::new(&mut p.last)
                .range(0..=4096)
                .prefix("end "),
        );
    });
    if ui
        .add_enabled(
            s.style.flow == core::Flow::Vertical && p.first < p.last,
            egui::Button::new("Add horizontal range"),
        )
        .clicked()
    {
        let mut candidate = o.as_ref().clone();
        let (a, b) = fmt_text::expand(&s.text, p.first, p.last);
        candidate.horizontal.push([a, b]);
        candidate.horizontal.sort_unstable();
        if candidate.valid(&s.text)
            && candidate.horizontal.iter().all(|r| {
                s.style
                    .ruby
                    .iter()
                    .all(|a| r[1] <= a.start || r[0] >= a.end)
            })
        {
            **o = candidate;
            p.error = None
        } else {
            p.error = Some(
                "Choose nonoverlapping graphemes (1–16 scalars), outside ruby and hard newlines"
                    .into(),
            );
        }
    }
    let mut remove = None;
    for (i, r) in o.horizontal.iter().enumerate() {
        ui.horizontal(|ui| {
            ui.label(format!("Horizontal in vertical: {}..{}", r[0], r[1]));
            if ui.button("Remove range").clicked() {
                remove = Some(i);
            }
        });
    }
    if let Some(i) = remove {
        o.horizontal.remove(i);
    }
}
pub(super) fn controls(ui: &mut egui::Ui, e: &mut Draft, p: &mut Panel, blocked: bool) {
    egui::CollapsingHeader::new("Advanced typography and character background").show(ui,|ui|{ui.add_enabled_ui(!blocked,|ui|{
        if e.settings.style.typeset.is_none(){ui.label("Explicitly enables current layout. Prepare the preview; Cancel preserves the original.");let r=ui.button("Enable advanced typography");p.enable=Some(r.rect);if r.clicked(){match drawer_core::text_version::upgrade(&e.settings.style,&e.settings.text){Ok(mut s)=>{s.typeset=Some(Box::default());e.settings.style=s;},Err(err)=>p.error=Some(err)}}}
        options(ui,&mut e.settings,p);
        if let Some(error)=&p.error{ui.colored_label(egui::Color32::YELLOW,error);}
    });});
}
impl Drawer {
    pub(crate) fn begin_text_tool_settings(&mut self) {
        self.text_editor.typeset.tool = Some(self.workspaces.store.current.text_tool.clone());
    }
    pub(crate) fn toggle_text_tool(&mut self) {
        self.finish_stroke();
        self.drawing.text_tool_active = !self.drawing.text_tool_active;
        self.status=Some(if self.drawing.text_tool_active{"Text tool: click or drag the canvas; Escape cancels capture. Configure actions with text-tool-settings."}else{"Text tool off"}.into());
    }
    fn save_text_tool(&mut self, ctx: &egui::Context, tool: Box<ty::Tool>) -> Result<(), String> {
        if !tool.valid() {
            return Err("Invalid new-text defaults".into());
        }
        if self
            .text_editor
            .draft
            .as_ref()
            .is_some_and(|e| e.composition.is_some())
        {
            return Err("Finish the IME composition first".into());
        }
        if tool.target == ty::ApplyTarget::SelectedText
            && self
                .text_editor
                .draft
                .as_ref()
                .is_none_or(|e| e.id.is_none())
        {
            if self.text_editor.draft.is_some() {
                return Err(
                    "Apply or cancel the new text draft before targeting selected text".into(),
                );
            }
            let d = &self.documents[self.active.ok_or("Select text first")?];
            let selected = self
                .panels
                .targets
                .get(&d.id)
                .and_then(|t| drawer_core::layer_catalog::from_thumbnail(d, *t))
                .filter(|key| matches!(key, drawer_core::layer_search::Key::Object(_)))
                .and_then(|key| drawer_core::layer_catalog::source_index(d, key))
                .and_then(|n| d.scene.objects.get(n))
                .and_then(Settings::from_object);
            if selected.is_none() {
                return Err(
                    "Select an existing text object before saving SelectedText settings".into(),
                );
            }
        }
        let old = std::mem::replace(&mut self.workspaces.store.current.text_tool, tool);
        if let Err(error) = self.persist_workspace() {
            self.workspaces.store.current.text_tool = old;
            return Err(error);
        }
        if self.workspaces.store.current.text_tool.target == ty::ApplyTarget::SelectedText {
            if self.text_editor.draft.is_none() {
                let action = self.workspaces.store.current.text_tool.create;
                self.workspaces.store.current.text_tool.create = ty::CreateAction::DetectPosition;
                let result = self.begin_text_editor(ctx);
                self.workspaces.store.current.text_tool.create = action;
                result?;
            }
            if let Some(e) = &mut self.text_editor.draft
                && e.id.is_some()
            {
                e.settings =
                    ty::apply_defaults(&self.workspaces.store.current.text_tool, true, &e.settings);
                e.prepared = None;
                e.image = None;
            }
        }
        Ok(())
    }
    pub(crate) fn text_tool_window(&mut self, ctx: &egui::Context) {
        self.drawing.text_quality = self.workspaces.store.current.text_tool.quality;
        self.poll_text_tool(ctx);
        let Some(mut tool) = self.text_editor.typeset.tool.take() else {
            return;
        };
        let mut open = true;
        let (mut save, mut cancel) = (false, false);
        egui::Window::new("Local text tool settings").default_width(470.).open(&mut open).show(ctx,|ui|{
            ui.label("Saving defaults does not overwrite existing text unless SelectedText is chosen. That target opens a draft to preview and apply as one Undo.");
            choice(ui,"Create action",&mut tool.create,&[ty::CreateAction::CreateLayerAlways,ty::CreateAction::AddToSelectedText,ty::CreateAction::DetectPosition]);
            choice(ui,"Drag action",&mut tool.drag,&[ty::DragAction::CreateTextBox,ty::DragAction::SelectText]);
            choice(ui,"Apply target",&mut tool.target,&[ty::ApplyTarget::None,ty::ApplyTarget::NewTextOnly,ty::ApplyTarget::SelectedText]);
            choice(ui,"Preview quality",&mut tool.quality,&[ty::Quality::Fast,ty::Quality::High,ty::Quality::Auto]);
            ui.label("Quality changes display sampling only; saved geometry and final output are identical.");
            font_ui::picker(ui,"new-text-font",&mut tool.defaults.style.font);ui.add(egui::DragValue::new(&mut tool.defaults.size).range(0.1..=4096.).prefix("New text size "));ui.color_edit_button_srgba_unmultiplied(&mut tool.defaults.color);
            if ui.button("Copy formatting from current text draft").clicked()&&let Some(e)=&self.text_editor.draft {tool.defaults=e.settings.clone();tool.defaults.text.clear();tool.defaults.baseline=[0.;2];tool.defaults.style.frame=Default::default();tool.defaults.style.ruby.clear();if let Some(r)=&mut tool.defaults.style.rich{r.spans.clear();}
                if let Some(o)=&mut tool.defaults.style.typeset{o.horizontal.clear();}}
            if let Some(error)=&self.text_editor.typeset.error{ui.colored_label(egui::Color32::YELLOW,error);}
            ui.horizontal(|ui|{save=ui.add_enabled(tool.valid(),egui::Button::new("Save tool settings")).clicked();cancel=ui.button("Cancel settings").clicked();});
        });
        if cancel || !open || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            return;
        }
        if save {
            if let Err(error) = self.save_text_tool(ctx, tool.clone()) {
                self.text_editor.typeset.error = Some(error);
            } else {
                return;
            }
        }
        self.text_editor.typeset.tool = Some(tool);
    }
    fn poll_text_tool(&mut self, ctx: &egui::Context) {
        let Some(request) = self.drawing.text_tool_request.take() else {
            return;
        };
        let result = (|| -> Result<(), String> {
            let n = self.active.ok_or("Open the captured page")?;
            let d = &self.documents[n];
            if d.id != request.document
                || d.revision != request.revision
                || d.current_frame != request.frame
            {
                return Err("Text capture source changed".into());
            }
            let inv = d.scene.transform.inverse();
            let a = inv.map(request.start);
            let b = inv.map(request.end);
            let region = drawer_core::scene::Rect {
                min: std::array::from_fn(|n| a[n].min(b[n])),
                max: std::array::from_fn(|n| a[n].max(b[n])),
            };
            let tool = self.workspaces.store.current.text_tool.clone();
            let dragging = (a[0] - b[0]).hypot(a[1] - b[1]) > 2.;
            let order = drawer_core::render_order::normalized(&d.scene);
            let hit = order.iter().rev().find_map(|entry| {
                let drawer_core::layer_search::Key::Object(id) = entry.key else {
                    return None;
                };
                let n = drawer_core::layer_catalog::source_index(d, entry.key)?;
                let o = &d.scene.objects[n];
                if !matches!(o, drawer_core::scene::PageObject::Text { .. })
                    || drawer_core::layer_properties::inherited(
                        d,
                        drawer_core::layer_properties::Target::Object(id),
                    )
                    .hidden
                {
                    return None;
                }
                let r = o.bounds();
                let hit = if dragging && tool.drag == ty::DragAction::SelectText {
                    (0..2).all(|n| r.min[n] <= region.max[n] && r.max[n] >= region.min[n])
                } else {
                    (0..2).all(|n| b[n] >= r.min[n] && b[n] <= r.max[n])
                };
                hit.then_some(id)
            });
            let selected = self
                .panels
                .targets
                .get(&d.id)
                .and_then(|t| drawer_core::layer_catalog::from_thumbnail(d, *t))
                .and_then(|k| match k {
                    drawer_core::layer_search::Key::Object(id) => {
                        drawer_core::layer_catalog::source_index(d, k)
                            .and_then(|n| d.scene.objects.get(n))
                            .and_then(Settings::from_object)
                            .map(|_| id)
                    }
                    _ => None,
                });
            let id = if dragging && tool.drag == ty::DragAction::SelectText {
                Some(hit.ok_or("No text object in the dragged region")?)
            } else {
                match tool.create {
                    ty::CreateAction::CreateLayerAlways => None,
                    ty::CreateAction::AddToSelectedText => {
                        Some(selected.ok_or("Select a text object to add text")?)
                    }
                    ty::CreateAction::DetectPosition => hit,
                }
            };
            self.panels.targets.insert(
                d.id,
                id.map_or(drawer_core::thumbnail::Target::Raster(0), |id| {
                    drawer_core::thumbnail::Target::Source(drawer_core::layer_search::Key::Object(
                        id,
                    ))
                }),
            );
            // The capture has already resolved the explicit create action; avoid re-resolving it from stale selection.
            let old = self.workspaces.store.current.text_tool.create;
            self.workspaces.store.current.text_tool.create = ty::CreateAction::DetectPosition;
            let result = self.begin_text_editor(ctx);
            self.workspaces.store.current.text_tool.create = old;
            result?;
            if id.is_none()
                && let Some(e) = &mut self.text_editor.draft
            {
                e.settings.baseline = if dragging {
                    [region.min[0], region.min[1] + e.settings.size]
                } else {
                    b
                };
                if dragging {
                    e.settings.style.wrap_width = (region.max[0] - region.min[0]).max(1.);
                }
            }
            Ok(())
        })();
        if let Err(error) = result {
            self.status = Some(error);
        }
    }
}
pub(super) fn smoke(root: &std::path::Path) -> Result<(), String> {
    verify::smoke(root)
}
#[path = "text_typeset_ui_verify.rs"]
mod verify;
