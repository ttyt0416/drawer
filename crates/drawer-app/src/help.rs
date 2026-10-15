use super::*;
use drawer_core::help;
#[derive(Default)]
pub(super) struct Help {
    pub korean_font: bool,
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
            ui.horizontal_wrapped(|ui|{ui.label("Search / 검색");let changed=ui.add(egui::TextEdit::singleline(&mut p.query).char_limit(256)).changed();if ui.button("Home / 시작").clicked(){p.query.clear();topic="welcome".into();}
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
