use super::*;
use fmt_text::{Attr, Border, Group, Mapping, Presets, Rule};
pub(super) struct Panel {
    pub open: bool,
    pub first: usize,
    pub last: usize,
    pub font: Font,
    pub ratio: f64,
    pub color: [u8; 4],
    pub border: bool,
    pub width: f64,
    pub edge: [u8; 4],
    pub mapping: Mapping,
    pub name: String,
    pub selected: String,
    pub error: Option<String>,
    pub toggle: Option<egui::Rect>,
    pub enable: Option<egui::Rect>,
    pub apply: Option<egui::Rect>,
    pub map_apply: Option<egui::Rect>,
}
impl Default for Panel {
    fn default() -> Self {
        Self {
            open: false,
            first: 1,
            last: 1,
            font: Font::Korean,
            ratio: 1.,
            color: [0, 0, 0, 255],
            border: false,
            width: 2.,
            edge: [255, 255, 255, 255],
            mapping: Mapping::default(),
            name: String::new(),
            selected: String::new(),
            error: None,
            toggle: None,
            enable: None,
            apply: None,
            map_apply: None,
        }
    }
}
pub(super) enum Action {
    Create(String, Mapping),
    Update(String, String, Mapping),
    Delete(String),
}
pub(super) fn preset(app: &mut Drawer, action: Action) -> Result<(), String> {
    let old = app.workspaces.store.current.text_mix.clone();
    let mut next = old.clone();
    match action {
        Action::Create(n, m) => next.save(n, m, false)?,
        Action::Update(old, name, mapping) => {
            let n = next
                .values
                .iter()
                .position(|(n, _)| *n == old)
                .ok_or("Select an existing mixed-font preset")?;
            if next
                .values
                .iter()
                .enumerate()
                .any(|(i, (v, _))| i != n && v.trim().to_lowercase() == name.trim().to_lowercase())
            {
                return Err("Another mixed-font preset already uses that name".into());
            }
            next.values[n] = (name, mapping);
        }
        Action::Delete(n) => next.values.retain(|(v, _)| *v != n),
    }
    if !next.valid() {
        return Err(
            "Use unique nonempty preset names up to64 characters and valid mappings".into(),
        );
    }
    if next == old {
        return Ok(());
    }
    app.workspaces.store.current.text_mix = next;
    if let Err(error) = app.persist_workspace() {
        app.workspaces.store.current.text_mix = old;
        return Err(error);
    }
    Ok(())
}
pub(super) fn controls(
    ui: &mut egui::Ui,
    e: &mut Draft,
    p: &mut Panel,
    presets: &Presets,
    blocked: bool,
) -> Option<Action> {
    let toggle = ui.checkbox(&mut p.open, "Character formatting and mixed fonts");
    p.toggle = Some(toggle.rect);
    if !p.open {
        return None;
    }
    let body_id = composition::widget_id(e);
    let mut action = None;
    let composing = e.composition.is_some() || blocked;
    if composing {
        ui.label("Finish or cancel IME composition before changing character formatting.");
    }
    ui.add_enabled_ui(!composing,|ui|{
        if let Some(rich)=&mut e.settings.style.rich{
            let bounds=fmt_text::boundaries(&e.settings.text);let count=bounds.len()-1;
            ui.horizontal(|ui|{ui.add(egui::DragValue::new(&mut p.first).range(1..=count.max(1)).prefix("From grapheme "));ui.add(egui::DragValue::new(&mut p.last).range(1..=count.max(1)).prefix("Through "));if ui.button("Use text selection").clicked()&&let Some(range)=egui::text_edit::TextEditState::load(ui.ctx(),body_id).and_then(|s|s.cursor.char_range()){let a=usize::from(range.primary.index).min(usize::from(range.secondary.index));let b=usize::from(range.primary.index).max(usize::from(range.secondary.index));let(a,b)=fmt_text::expand(&e.settings.text,a,b);p.first=bounds.iter().position(|v|*v==a).unwrap_or(0)+1;p.last=bounds.iter().position(|v|*v==b).unwrap_or(p.first).max(p.first).min(count);}});
            font_picker(ui,"range-font",&mut p.font);
            ui.horizontal(|ui|{ui.add(egui::DragValue::new(&mut p.ratio).range(0.1..=4.).speed(0.01).prefix("Size ratio "));ui.color_edit_button_srgba_unmultiplied(&mut p.color);ui.checkbox(&mut p.border,"Outline");});
            if p.border{ui.horizontal(|ui|{ui.add(egui::DragValue::new(&mut p.width).range(0. ..=128.).prefix("Outline px "));ui.color_edit_button_srgba_unmultiplied(&mut p.edge);});}
            let valid=p.first>0&&p.first<=p.last&&p.last<=count;
            let r=ui.add_enabled(valid,egui::Button::new("Apply format to graphemes"));p.apply=Some(r.rect);
            if r.clicked(){let attr=Attr{font:Some(p.font),ratio:Some(p.ratio),color:Some(p.color),border:p.border.then_some(Border{width:p.width,color:p.edge})};if let Err(error)=rich.set(&e.settings.text,bounds[p.first-1],bounds[p.last],Some(attr)){p.error=Some(error);}}
            if ui.add_enabled(valid,egui::Button::new("Clear selected range formatting")).clicked(){let _=rich.set(&e.settings.text,bounds[p.first-1],bounds[p.last],None);}
        }
        else{let r=ui.button("Enable grapheme formatting");p.enable=Some(r.rect);if r.clicked(){e.settings.style.rich=Some(Box::default());}ui.small("Enabling prepares a grapheme-aware layout preview. Apply commits it once.");}
        ui.separator();ui.label("Mixed-font mapping editor (first matching rule wins)");
        font_picker(ui,"mapping-base",&mut p.mapping.base);
        let mut move_rule=None;let mut remove=None;
        egui::ScrollArea::vertical().id_salt("mixed-font-rules").max_height(150.).show(ui,|ui|{for n in 0..p.mapping.rules.len(){ui.push_id(n,|ui|{let r=&mut p.mapping.rules[n];ui.horizontal(|ui|{egui::ComboBox::from_id_salt("script").selected_text(r.group.label()).show_ui(ui,|ui|{for group in Group::ALL{ui.selectable_value(&mut r.group,group,group.label());}});if ui.small_button("↑").clicked()&&n>0{move_rule=Some((n,n-1));}
if ui.small_button("↓").clicked(){move_rule=Some((n,n+1));}
if ui.small_button("Remove").clicked(){remove=Some(n);}});font_picker(ui,"rule-font",&mut r.font);ui.add(egui::DragValue::new(&mut r.ratio).range(0.1..=4.).speed(0.01).prefix("Script size ratio "));if r.group==Group::Custom{ui.horizontal(|ui|{ui.add(egui::DragValue::new(&mut r.from).range(0..=0x10ffff).hexadecimal(6,false,true).prefix("U+ "));ui.add(egui::DragValue::new(&mut r.through).range(0..=0x10ffff).hexadecimal(6,false,true).prefix("through U+ "));});}ui.separator();});}});
        if let Some(n)=remove{p.mapping.rules.remove(n);}else if let Some((a,b))=move_rule&&b<p.mapping.rules.len(){p.mapping.rules.swap(a,b);}
        if ui.add_enabled(p.mapping.rules.len()<16,egui::Button::new("Add custom range rule")).clicked(){p.mapping.rules.push(Rule{group:Group::Custom,from:0x1f600,through:0x1f64f,font:Font::Emoji,ratio:1.});}
        let r=ui.add_enabled(p.mapping.valid(),egui::Button::new("Apply mapping snapshot to this text"));p.map_apply=Some(r.rect);if r.clicked(){e.settings.style.rich.get_or_insert_with(Box::default).mapping=Some(p.mapping.clone());}
        if e.settings.style.rich.as_ref().is_some_and(|r|r.mapping.is_some())&&ui.button("Remove mapping from this text").clicked(){e.settings.style.rich.as_mut().unwrap().mapping=None;}
        ui.small("Editing, renaming or deleting a local preset leaves existing text unchanged. Apply mapping explicitly to copy its values into this text.");
        egui::ComboBox::from_id_salt("mixed-font-presets").selected_text(if p.selected.is_empty(){"Choose local preset"}else{&p.selected}).show_ui(ui,|ui|{for (name,_)in &presets.values{ui.selectable_value(&mut p.selected,name.clone(),name);}});
        if ui.button("Load preset into mapping editor").clicked()&&let Some((name,m))=presets.values.iter().find(|(n,_)|*n==p.selected){p.name=name.clone();p.mapping=m.clone();}
        ui.add(egui::TextEdit::singleline(&mut p.name).char_limit(64).hint_text("Mixed-font preset name"));
        ui.horizontal_wrapped(|ui|{if ui.button("Create local preset").clicked(){action=Some(Action::Create(p.name.clone(),p.mapping.clone()));}let selected=presets.values.iter().any(|(n,_)|*n==p.selected);if ui.add_enabled(selected,egui::Button::new("Update / rename selected preset")).clicked(){action=Some(Action::Update(p.selected.clone(),p.name.clone(),p.mapping.clone()));}
if ui.add_enabled(selected,egui::Button::new("Delete selected preset")).clicked(){action=Some(Action::Delete(p.selected.clone()));}});
    });
    if let Some(error) = &p.error {
        ui.colored_label(egui::Color32::YELLOW, error);
    }
    action
}
fn font_picker(ui: &mut egui::Ui, id: &str, font: &mut Font) {
    super::font_ui::picker(ui, id, font);
}
pub(super) fn normalize(ctx: &egui::Context, e: &mut Draft) -> Option<String> {
    e.settings.style.rich.as_ref()?;
    let field = std::iter::once(composition::Field::Body)
        .chain((0..e.settings.style.ruby.len()).map(composition::Field::Reading))
        .find(|f| ctx.memory(|m| m.has_focus(composition::field_id(e, *f))))?;
    let text = match field {
        composition::Field::Body => &e.settings.text,
        composition::Field::Reading(n) => &e.settings.style.ruby[n].text,
    };
    let id = composition::field_id(e, field);
    let mut state = egui::text_edit::TextEditState::load(ctx, id)?;
    let range = state.cursor.char_range()?;
    let a = usize::from(range.primary.index).min(usize::from(range.secondary.index));
    let b = usize::from(range.primary.index).max(usize::from(range.secondary.index));
    let selected = b - a;
    let limit = if field == composition::Field::Body {
        4096
    } else {
        1024
    };
    let mut added = 0;
    let reject = ctx.input(|i| {
        i.events.iter().any(|v| {
            let text = match v {
                egui::Event::Text(t)
                | egui::Event::Paste(t)
                | egui::Event::Ime(egui::ImeEvent::Commit(t)) => {
                    added += t.chars().count();
                    Some(added)
                }
                egui::Event::Ime(egui::ImeEvent::Preedit { text, .. }) => {
                    Some(text.chars().count())
                }
                _ => None,
            };
            text.is_some_and(|count| text_count(e, field).saturating_sub(selected) + count > limit)
        })
    });
    if reject {
        composition::cancel(ctx, e);
        ctx.input_mut(|i| {
            i.events.retain(|v| {
                !matches!(
                    v,
                    egui::Event::Text(_) | egui::Event::Paste(_) | egui::Event::Ime(_)
                )
            })
        });
        return Some(format!(
            "Input exceeds {limit} Unicode characters; the complete input was rejected without splitting a grapheme"
        ));
    }
    if e.composition.is_some() {
        return None;
    }
    let boundaries = fmt_text::boundaries(text);
    let arrow = ctx.input(|i| {
        !i.modifiers.command
            && !i.modifiers.shift
            && (i.key_pressed(egui::Key::ArrowLeft) || i.key_pressed(egui::Key::ArrowRight))
    });
    let (a, b) = if a != b {
        fmt_text::expand(text, a, b)
    } else if arrow {
        let n = if ctx.input(|i| i.key_pressed(egui::Key::ArrowLeft)) {
            boundaries
                .iter()
                .copied()
                .take_while(|v| *v < a)
                .last()
                .unwrap_or(0)
        } else {
            boundaries.iter().copied().find(|v| *v > a).unwrap_or(a)
        };
        ctx.input_mut(|i| {
            i.events.retain(|v| {
                !matches!(
                    v,
                    egui::Event::Key {
                        key: egui::Key::ArrowLeft | egui::Key::ArrowRight,
                        ..
                    }
                )
            })
        });
        (n, n)
    } else if ctx.input(|i| {
        !i.modifiers.command
            && (i.key_pressed(egui::Key::Backspace) || i.key_pressed(egui::Key::Delete))
    }) {
        if !boundaries.contains(&a) {
            fmt_text::expand(text, a, a)
        } else if ctx.input(|i| i.key_pressed(egui::Key::Backspace)) {
            (
                boundaries
                    .iter()
                    .copied()
                    .take_while(|v| *v < a)
                    .last()
                    .unwrap_or(0),
                a,
            )
        } else {
            (a, boundaries.iter().copied().find(|v| *v > a).unwrap_or(a))
        }
    } else {
        let n = boundaries
            .iter()
            .copied()
            .find(|v| *v >= a)
            .unwrap_or(*boundaries.last().unwrap());
        (n, n)
    };
    state
        .cursor
        .set_char_range(Some(egui::text::CCursorRange::two(
            egui::text::CCursor::new(a),
            egui::text::CCursor::new(b),
        )));
    state.store(ctx, id);
    None
}
fn text_count(e: &Draft, field: composition::Field) -> usize {
    match field {
        composition::Field::Body => e.settings.text.chars().count(),
        composition::Field::Reading(n) => e.settings.style.ruby[n].text.chars().count(),
    }
}

#[path = "text_format_ui_verify.rs"]
mod verify;
pub(super) fn smoke(root: &std::path::Path) -> Result<(), String> {
    verify::smoke(root)?;
    verify::editing()
}
