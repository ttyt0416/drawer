use super::*;
use drawer_core::palette_exchange::{Bundle, Policy, Prepared};
use std::collections::BTreeMap;
#[derive(Default)]
pub(super) struct Panel {
    open: bool,
    path: String,
    policy: Policy,
    selected: bool,
    replace_file: bool,
    confirmed: bool,
    prepared: Option<Prepared>,
    message: String,
    controls: BTreeMap<&'static str, egui::Rect>,
}
impl Drawer {
    pub(super) fn begin_palette_exchange(&mut self) {
        self.palette_exchange = Panel {
            open: true,
            ..Default::default()
        };
    }
    fn stage_palette(&self, p: &mut Panel) -> Result<(), String> {
        let b = Bundle::read(std::path::Path::new(&p.path))?;
        p.prepared = Some(b.prepare(&self.workspaces.store.current.color_tables, p.policy)?);
        p.confirmed = false;
        p.message =
            "Palette validated and staged. Apply saves settings; Cancel changes nothing.".into();
        Ok(())
    }
    fn apply_palette(&mut self, p: &Panel) -> Result<(), String> {
        let prepared = p.prepared.as_ref().ok_or("Read a palette first")?;
        prepared.check(&self.workspaces.store.current.color_tables, p.confirmed)?;
        let before = self.workspaces.store.current.color_tables.clone();
        let mut snapshot = self.capture_workspace();
        snapshot.color_tables = prepared.after.clone();
        let mut history = std::mem::take(&mut self.color_tables.history);
        self.stage_workspace(snapshot, self.workspaces.store.active.clone());
        let result = self.apply_workspace(
            drawer_core::workspaces::Scope {
                other: true,
                ..Default::default()
            },
            None,
        );
        if result.is_ok() {
            history.record(before, &self.workspaces.store.current.color_tables);
            self.color_tables.reset_exchange_selection();
        } else {
            self.workspaces.staged = None;
        }
        self.color_tables.history = history;
        result
    }
    pub(super) fn palette_exchange_window(&mut self, ctx: &egui::Context) {
        let mut p = std::mem::take(&mut self.palette_exchange);
        if !p.open {
            return;
        }
        let mut open = !ctx.input(|i| i.key_pressed(egui::Key::Escape));
        let mut browse = false;
        let mut save_as = false;
        let mut read = false;
        let mut export = false;
        let mut apply = false;
        let mut cancel = false;
        p.controls.clear();
        egui::Window::new("Local color table exchange").default_pos(egui::pos2(25.,25.)).default_width(580.).collapsible(false).open(&mut open).show(ctx,|ui|{
            ui.horizontal(|ui|{for (key,label) in [("read","Read palette"),("export","Export palettes"),("apply","Apply imported palettes"),("cancel","Cancel")]{let r=ui.add_enabled(key!="apply"||p.prepared.is_some(),egui::Button::new(label));p.controls.insert(key,r.rect);if r.clicked(){match key{"read"=>read=true,"export"=>export=true,"apply"=>apply=true,_=>cancel=true}}}});
            ui.horizontal(|ui|{ui.label("Local JSON");if ui.text_edit_singleline(&mut p.path).changed(){p.prepared=None;p.replace_file=false;p.confirmed=false;}let r=ui.button("Browse…");p.controls.insert("browse",r.rect);browse=r.clicked();let r=ui.button("Save as…");save_as=r.clicked();});
            ui.horizontal(|ui|{let mut changed=false;let r=ui.selectable_value(&mut p.policy,Policy::Copy,"Independent copies");p.controls.insert("copy",r.rect);changed|=r.changed();let r=ui.selectable_value(&mut p.policy,Policy::ReplaceNames,"Replace matching names");p.controls.insert("names",r.rect);changed|=r.changed();if changed{p.prepared=None;p.confirmed=false;}});
            ui.checkbox(&mut p.selected,"Export selected table only");let r=ui.checkbox(&mut p.replace_file,"Replace this existing export file");p.controls.insert("replace-file",r.rect);
            ui.small("Own palette v1: RGBA8 values, names and layouts. Copies get new IDs and distinct table names. Replacement keeps the target table ID and needs confirmation. No document pixels or drawing history change.");
            if let Some(s)=&p.prepared{ui.label(format!("{} ID mappings; {} existing tables to replace",s.mapping.len(),s.replaced.len()));if !s.replaced.is_empty(){ui.colored_label(egui::Color32::YELLOW,format!("Replace: {}",s.replaced.join(", ")));let r=ui.checkbox(&mut p.confirmed,"Confirm replacement of the listed tables");p.controls.insert("confirm",r.rect);}egui::ScrollArea::vertical().max_height(100.).show(ui,|ui|{for t in &s.after.tables{ui.label(format!("{}: {} colors",t.name,t.colors.len()));}});}
            ui.label(&p.message);
        });
        if browse
            && let Some(path) = rfd::FileDialog::new()
                .add_filter("Drawer palette JSON", &["json"])
                .pick_file()
        {
            p.path = path.display().to_string();
            p.prepared = None;
            p.replace_file = false;
        }
        if save_as
            && let Some(path) = rfd::FileDialog::new()
                .add_filter("Drawer palette JSON", &["json"])
                .set_file_name("color-tables.json")
                .save_file()
        {
            p.path = path.display().to_string();
            p.prepared = None;
            p.replace_file = false;
        }
        let result = if read {
            p.prepared = None;
            self.stage_palette(&mut p)
        } else if export {
            Bundle::capture(&self.workspaces.store.current.color_tables, p.selected)
                .and_then(|b| b.write(std::path::Path::new(&p.path), p.replace_file))
                .map(|_| p.message = "Palette exported; current settings unchanged".into())
        } else if apply {
            self.apply_palette(&p)
        } else {
            Ok(())
        };
        if let Err(e) = result {
            p.message = e;
        } else if apply {
            return;
        }
        if open && !cancel {
            self.palette_exchange = p;
        }
    }
}
#[path = "palette_exchange_verify.rs"]
mod verify;
pub(super) fn smoke(root: &std::path::Path) -> Result<(), String> {
    verify::run(root)
}
#[cfg(test)]
mod tests {
    #[test]
    fn pointer_local_palette_read_copy_replace_confirm_cancel_failure_and_restart() {
        let t = crate::tests::TestDirectory::new();
        super::verify::run(&t.0).unwrap();
    }
}
