use super::*;
use drawer_core::help;
#[derive(Default)]
pub(super) struct Help {
    pub korean_font: bool,
    pending: bool,
    notices: Vec<&'static str>,
    #[cfg(test)]
    pub links: std::collections::HashMap<String, egui::Rect>,
    pub rendered_topic: Option<String>,
}
impl Help {
    pub fn new(korean_font: bool) -> Self {
        Self {
            korean_font,
            ..Default::default()
        }
    }
    pub fn notices_count(&self) -> usize {
        self.notices.len()
    }
}
#[derive(Clone)]
struct Target {
    id: egui::Id,
    rect: egui::Rect,
    topic: &'static str,
}
pub(super) fn tag(ctx: &egui::Context, response: &egui::Response, topic: &'static str) {
    ctx.data_mut(|d| {
        let id = egui::Id::new("drawer-help-targets");
        d.get_temp_mut_or_default::<Vec<Target>>(id).push(Target {
            id: response.id,
            rect: response.interact_rect,
            topic,
        });
    });
}
pub(super) fn has_context(ctx: &egui::Context) -> bool {
    ctx.data(|d| d.get_temp::<Vec<Target>>(egui::Id::new("drawer-help-targets")))
        .is_some_and(|t| t.len() > 10)
}
pub(super) fn button(
    ui: &mut egui::Ui,
    label: impl Into<String>,
    topic: &'static str,
) -> egui::Response {
    let response = ui.button(label.into());
    tag(ui.ctx(), &response, topic);
    response
}
pub(super) fn selectable(
    ui: &mut egui::Ui,
    selected: bool,
    label: impl Into<String>,
    topic: &'static str,
) -> egui::Response {
    let response = ui.selectable_label(selected, label.into());
    tag(ui.ctx(), &response, topic);
    response
}
pub(super) fn checkbox(
    ui: &mut egui::Ui,
    value: &mut bool,
    label: impl Into<String>,
    topic: &'static str,
) -> egui::Response {
    let response = ui.checkbox(value, label.into());
    tag(ui.ctx(), &response, topic);
    response
}
/// Read a locally installed system font only; never copy it into the package.
pub(super) fn setup_ui_font(ctx: &egui::Context) -> bool {
    let id = egui::Id::new("drawer-local-korean-font");
    if let Some(loaded) = ctx.data(|d| d.get_temp::<bool>(id)) {
        return loaded;
    }
    let loaded = (|| -> Option<()> {
        let path = PathBuf::from(std::env::var_os("WINDIR")?).join("Fonts/malgun.ttf");
        if std::fs::metadata(&path).ok()?.len() > 32 * 1024 * 1024 {
            return None;
        }
        let bytes = std::fs::read(path).ok()?;
        if !help::supports_korean(&bytes) {
            return None;
        }
        let mut fonts = egui::FontDefinitions::default();
        fonts.font_data.insert(
            "local-korean".into(),
            Arc::new(egui::FontData::from_owned(bytes)),
        );
        for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
            fonts
                .families
                .entry(family)
                .or_default()
                .push("local-korean".into());
        }
        ctx.set_fonts(fonts);
        Some(())
    })()
    .is_some();
    ctx.data_mut(|d| d.insert_temp(id, loaded));
    loaded
}
impl Drawer {
    pub(super) fn begin_context_help(&mut self, ctx: &egui::Context) {
        ctx.data_mut(|d| d.insert_temp(egui::Id::new("drawer-help-targets"), Vec::<Target>::new()));
        if !ctx.input(|i| i.focused) {
            return;
        }
        let bindings = self
            .workspaces
            .store
            .current
            .bindings
            .iter()
            .filter(|b| b.command == "help")
            .cloned()
            .collect::<Vec<_>>();
        self.help.pending |= ctx
            .input_mut(|i| super::workspaces::shortcut(i, &bindings))
            .is_some();
    }
    pub(super) fn resolve_context_help(&mut self, ctx: &egui::Context) {
        if !std::mem::take(&mut self.help.pending) {
            return;
        }
        let targets = ctx
            .data(|d| d.get_temp::<Vec<Target>>(egui::Id::new("drawer-help-targets")))
            .unwrap_or_default();
        let focused = ctx.memory(|m| m.focused());
        let pointer = ctx.pointer_latest_pos();
        let focused_point = focused
            .and_then(|id| ctx.read_response(id))
            .map(|r| r.rect.center());
        let closest = |point: egui::Pos2| {
            targets
                .iter()
                .filter(|t| t.rect.contains(point))
                .min_by(|a, b| a.rect.area().total_cmp(&b.rect.area()))
        };
        let topic = focused
            .and_then(|id| targets.iter().find(|t| t.id == id))
            .or_else(|| focused_point.and_then(closest))
            .or_else(|| pointer.and_then(closest))
            .map(|t| t.topic)
            .unwrap_or("welcome");
        self.open_help(if help::find(topic).is_some() {
            topic
        } else {
            "welcome"
        });
    }
    pub(super) fn open_help(&mut self, topic: &str) {
        let p = &mut self.workspaces.store.current.help;
        p.visible = true;
        p.topic = topic.into();
        p.query.clear();
    }
    pub(super) fn help_window(&mut self, ctx: &egui::Context) {
        if !self.workspaces.store.current.help.visible {
            return;
        }
        let mut p = self.workspaces.store.current.help.clone();
        let mut visible = true;
        let mut topic = p.topic.clone();
        egui::Window::new("Local help — Drawer").open(&mut visible).default_size([800.0,600.0]).constrain(true).show(ctx,|ui|{
            ui.horizontal_wrapped(|ui|{ui.label("Search / 검색");let changed=super::edit::text(ui,&mut p.query,"welcome").changed();if ui.button("Home / 시작").clicked(){p.query.clear();topic="welcome".into();}
if ui.button("About / 버전").clicked(){topic="version".into();p.query.clear();}
                if changed{let found=help::search(&p.query,&self.capture_workspace());if !found.iter().any(|t|t.id==topic)&&let Some(first)=found.first(){topic=first.id.into();}}
            });
            if !self.help.korean_font{ui.label("Korean system font unavailable. Search still accepts Korean; no font is downloaded.");}
            let found=help::search(&p.query,&self.capture_workspace());
            ui.columns(2,|columns|{
                columns[0].set_width(200.0);egui::ScrollArea::vertical().scroll_bar_visibility(crate::ui_behavior::scroll(ctx)).id_salt("help-topics").max_height((ctx.content_rect().height()*0.7).max(100.0)).show(&mut columns[0],|ui|{
                    if found.is_empty(){ui.label("검색 결과 없음 / No matches");}
                    for t in found{if ui.selectable_label(t.id==topic,t.title).clicked(){topic=t.id.into();}}
                });
                let current=help::find(&topic).unwrap_or_else(||help::find("welcome").unwrap());
                self.help.rendered_topic=Some(current.id.into());
                columns[1].heading(current.title);
                if current.id!=topic{columns[1].label("This saved topic is unavailable. Showing the local start page.");}
                egui::ScrollArea::both().scroll_bar_visibility(crate::ui_behavior::scroll(ctx)).id_salt("help-body").max_height((ctx.content_rect().height()*0.6).max(100.0)).show(&mut columns[1],|ui|{
                    if current.id=="version"{ui.label(format!("Drawer {} · Windows x64",help::VERSION));}
                    for line in current.body.lines(){
                        if let Some((label,target))=help::link(line){let r=ui.button(label);#[cfg(test)]{self.help.links.insert(target.into(),r.rect);}if r.clicked(){topic=target.into();}}
                        else if line.is_empty(){ui.add_space(8.0);}else{ui.label(line);}
                    }
                    if current.id=="shortcuts"{
                        let s=self.capture_workspace();for b in &s.bindings{ui.label(format!("{} — {}",b.label(),s.actions.label(&b.command)));}ui.label("Ctrl+PageUp / Ctrl+PageDown — previous / next view");ui.label("Alt+1…9 — view in current order");
                    }
                    if current.id=="licenses"{
                        if self.help.notices.is_empty(){self.help.notices=help::NOTICES.lines().collect();}
                        let row=ui.text_style_height(&egui::TextStyle::Body)+ui.spacing().item_spacing.y;
                        egui::ScrollArea::both().scroll_bar_visibility(crate::ui_behavior::scroll(ctx)).id_salt("help-notices").max_height(280.0).show_rows(ui,row,self.help.notices.len(),|ui,range|{for n in range{ui.add(egui::Label::new(self.help.notices[n]).wrap_mode(egui::TextWrapMode::Extend));}});
                    }
                });
            });
        });
        p.visible = visible;
        p.topic = topic;
        self.workspaces.store.current.help = p;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{TestDirectory, click, frame, tiny_spec};
    #[test]
    fn context_help_prefers_focused_control_then_hover_then_local_home() {
        let ctx = egui::Context::default();
        let mut app = Drawer::new(&ctx, None);
        app.create_canvas(tiny_spec()).unwrap();
        let before = app.documents[0].snapshot();
        let history = app.documents[0].history_counts();
        let mut text = "name".to_string();
        let mut field = egui::Id::NULL;
        let mut hover = egui::Pos2::ZERO;
        let mut render = |app: &mut Drawer, events: Vec<egui::Event>, topic: &'static str| {
            let mut out = ctx.run_ui(
                egui::RawInput {
                    events,
                    ..Default::default()
                },
                |ui| {
                    app.begin_context_help(ui.ctx());
                    let r = crate::edit::text(ui, &mut text, topic);
                    field = r.id;
                    let button = super::button(ui, "Other control", "tools");
                    hover = button.rect.center();
                    app.resolve_context_help(ui.ctx());
                },
            );
            out.textures_delta.clear();
            (field, hover)
        };
        let (field, hover) = render(&mut app, vec![], "size");
        ctx.memory_mut(|m| m.request_focus(field));
        let f1 = |pressed| egui::Event::Key {
            key: egui::Key::F1,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers: Default::default(),
        };
        render(
            &mut app,
            vec![egui::Event::PointerMoved(hover), f1(true)],
            "size",
        );
        assert_eq!(app.workspaces.store.current.help.topic, "size");
        ctx.memory_mut(|m| m.surrender_focus(field));
        render(&mut app, vec![f1(false), f1(true)], "size");
        assert_eq!(app.workspaces.store.current.help.topic, "tools");
        ctx.memory_mut(|m| m.request_focus(field));
        render(&mut app, vec![f1(false), f1(true)], "missing-topic");
        assert_eq!(app.workspaces.store.current.help.topic, "welcome");
        ctx.memory_mut(|m| m.surrender_focus(field));
        render(
            &mut app,
            vec![egui::Event::PointerGone, f1(false), f1(true)],
            "size",
        );
        assert_eq!(app.workspaces.store.current.help.topic, "welcome");
        assert_eq!(app.documents[0].snapshot(), before);
        assert_eq!(app.documents[0].history_counts(), history);
    }
    #[test]
    fn local_help_links_search_licenses_restart_and_missing_topic_do_not_edit_document() {
        let root = TestDirectory::new();
        let storage = UserStorage::at(root.0.join("user")).unwrap();
        let ctx = egui::Context::default();
        let mut app = Drawer::new(&ctx, None)
            .with_storage(UserStorage::at(storage.root().to_path_buf()).unwrap());
        app.create_canvas(tiny_spec()).unwrap();
        frame(&ctx, &mut app, vec![]);
        let before = app.documents[0].snapshot();
        let history = app.documents[0].history_counts();
        let dirty = app.documents[0].dirty;
        app.open_help("welcome");
        frame(&ctx, &mut app, vec![]);
        frame(&ctx, &mut app, vec![]);
        let link = app.help.links["shortcuts"];
        click(&ctx, &mut app, link);
        assert_eq!(app.workspaces.store.current.help.topic, "shortcuts");
        frame(&ctx, &mut app, vec![]);
        assert_eq!(app.help.rendered_topic.as_deref(), Some("shortcuts"));
        app.workspaces.store.current.help.query = "작업공간".into();
        frame(&ctx, &mut app, vec![]);
        assert!(
            help::search("작업공간", &app.capture_workspace())
                .iter()
                .any(|t| t.id == "workspace")
        );
        app.open_help("licenses");
        frame(&ctx, &mut app, vec![]);
        assert!(app.help.notices_count() > 1000);
        assert_eq!(app.help.rendered_topic.as_deref(), Some("licenses"));
        app.open_help("unknown-saved-topic");
        frame(&ctx, &mut app, vec![]);
        assert_eq!(app.help.rendered_topic.as_deref(), Some("welcome"));
        app.workspaces.store.current.help.query = "no match 8675309".into();
        frame(&ctx, &mut app, vec![]);
        assert!(
            help::search(
                &app.workspaces.store.current.help.query,
                &app.capture_workspace()
            )
            .is_empty()
        );
        let restarted = Drawer::new(&egui::Context::default(), None)
            .with_storage(UserStorage::at(storage.root().to_path_buf()).unwrap());
        assert_eq!(
            restarted.workspaces.store.current.help,
            app.workspaces.store.current.help
        );
        assert_eq!(app.documents[0].snapshot(), before);
        assert_eq!(app.documents[0].history_counts(), history);
        assert_eq!(app.documents[0].dirty, dirty);
    }
    #[test]
    fn korean_system_font_is_local_optional_and_invalid_font_is_rejected() {
        assert!(!help::supports_korean(b"invalid font"));
        let ctx = egui::Context::default();
        let loaded = setup_ui_font(&ctx);
        assert_eq!(setup_ui_font(&ctx), loaded);
        if let Some(windir) = std::env::var_os("WINDIR") {
            let path = PathBuf::from(windir).join("Fonts/malgun.ttf");
            if path.exists() {
                assert!(loaded);
                assert!(help::supports_korean(&std::fs::read(path).unwrap()));
            }
        }
    }
}
