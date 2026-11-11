use super::*;
#[derive(Default)]
pub(super) struct Panel {
    pub open: bool,
    confirm: Option<u64>,
    pub clear: Option<egui::Rect>,
    pub accept: Option<egui::Rect>,
    pub cancel: Option<egui::Rect>,
}
impl Drawer {
    fn clear_history_decision(&mut self, id: u64, accept: bool) -> Result<(), String> {
        if self.history_navigation.confirm != Some(id) {
            return Err("History confirmation expired".into());
        }
        self.history_navigation.confirm = None;
        if accept {
            let doc = self
                .documents
                .iter_mut()
                .find(|d| d.id == id)
                .ok_or("Document closed")?;
            doc.clear_session_history();
        }
        Ok(())
    }
    pub(super) fn history_navigation_window(&mut self, ctx: &egui::Context) {
        self.history_navigation.clear = None;
        self.history_navigation.accept = None;
        self.history_navigation.cancel = None;
        let mut open = self.history_navigation.open;
        let mut jump = None;
        let mut clear = None;
        if open {
            egui::Window::new("Document history").open(&mut open).default_width(350.).show(ctx,|ui|{
            ui.label("Session history · saving writes only the current state. A new edit drops the redo branch.");
            if let Some(n)=self.active{let d=&self.documents[n];let busy=self.drawing.pending();
                ui.small(format!("History {} bytes / {}MiB; {} oldest Undo entries removed by limits",d.history_bytes(),d.history_limits().memory_mib,d.history_pruned()));
                egui::ScrollArea::vertical().max_height(320.).show(ui,|ui|{for e in d.history_entries(){let text=format!("{} {}{}",if e.current{"●"}else{"○"},e.label,if e.redo{" (redo branch)"}else{""});if ui.add_enabled(!busy,egui::Button::new(text).selected(e.current)).clicked(){jump=Some((d.id,e.token));}}});
                let r=ui.add_enabled(!busy,egui::Button::new("Clear Undo/Redo history…"));self.history_navigation.clear=Some(r.rect);if r.clicked(){clear=Some(d.id);}
            }else{ui.label("Open a document to view its history.");}
        });
        }
        self.history_navigation.open = open;
        if let Some((id, token)) = jump
            && let Some(d) = self.documents.iter_mut().find(|d| d.id == id)
            && let Err(e) = d.jump_history(token)
        {
            self.status = Some(e);
        }
        if let Some(id) = clear {
            self.history_navigation.confirm = Some(id);
        }
        if let Some(id) = self.history_navigation.confirm {
            let mut yes = false;
            let mut no = false;
            egui::Window::new("Clear session history?").collapsible(false).show(ctx,|ui|{ui.label("Keep the current picture, selection, path and saved/modified state. Delete all Undo/Redo records for this document. This cannot be undone.");let r=ui.button("Clear history");self.history_navigation.accept=Some(r.rect);yes=r.clicked();let r=ui.button("Cancel / keep history");self.history_navigation.cancel=Some(r.rect);no=r.clicked();});
            if (yes || no)
                && let Err(e) = self.clear_history_decision(id, yes)
            {
                self.status = Some(e);
            }
        }
    }
}
pub(super) fn run(root: &std::path::Path) -> Result<(), String> {
    std::fs::create_dir_all(root).map_err(|e| e.to_string())?;
    drawer_core::history_navigation::verify::run()?;
    let ctx = egui::Context::default();
    let mut app = Drawer::new(&ctx, None);
    app.simple_view = true;
    app.create_canvas(CanvasSpec {
        width: 4,
        height: 4,
        dpi: 300.,
        background: Background::Transparent,
    })
    .map_err(|e| e.to_string())?;
    app.documents[0].edit_pixel(0, 0, 0, [1, 2, 3, 255]);
    app.history_navigation.open = true;
    let mut t = 0.;
    let mut frame = |app: &mut Drawer, events| {
        t += 0.05;
        input_replay::frame(&ctx, app, t, events);
    };
    let click = |p: egui::Pos2| {
        vec![
            egui::Event::PointerMoved(p),
            egui::Event::PointerButton {
                pos: p,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: Default::default(),
            },
            egui::Event::PointerButton {
                pos: p,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: Default::default(),
            },
        ]
    };
    for _ in 0..3 {
        frame(&mut app, vec![]);
    }
    let state = app.documents[0].snapshot();
    let dirty = app.documents[0].dirty;
    let rect = app
        .history_navigation
        .clear
        .ok_or("Clear history button absent")?;
    frame(&mut app, click(rect.center()));
    frame(&mut app, vec![]);
    let rect = app
        .history_navigation
        .cancel
        .ok_or("Cancel history confirmation absent")?;
    frame(&mut app, click(rect.center()));
    if app.documents[0].history_counts() != (1, 0) {
        return Err("Cancel cleared history".into());
    }
    frame(&mut app, vec![]);
    let rect = app.history_navigation.clear.ok_or("Clear button absent")?;
    frame(&mut app, click(rect.center()));
    frame(&mut app, vec![]);
    let rect = app
        .history_navigation
        .accept
        .ok_or("Confirm clear absent")?;
    frame(&mut app, click(rect.center()));
    if app.documents[0].history_counts() != (0, 0)
        || app.documents[0].snapshot() != state
        || app.documents[0].dirty != dirty
    {
        return Err("Confirmed clear changed drawing".into());
    }
    let path = root.join("history-current.drawer");
    drawer_core::files::save(&mut app.documents[0], &path).map_err(|e| e.to_string())?;
    let d = drawer_core::files::open(&path, DEFAULT_DOCUMENT_BUDGET).map_err(|e| e.to_string())?;
    if d.history_counts() != (0, 0) || d.snapshot() != state {
        return Err("Save persisted history or changed current state".into());
    }
    std::fs::remove_file(path).map_err(|e| e.to_string())?;
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn mouse_confirm_cancel_and_current_state_save() {
        let dir = std::env::temp_dir().join(format!("drawer-history-nav-{}", std::process::id()));
        std::fs::create_dir(&dir).unwrap();
        let r = super::run(&dir);
        std::fs::remove_dir_all(dir).unwrap();
        r.unwrap();
    }
}

pub(super) fn limits_smoke(root: &std::path::Path) -> Result<(), String> {
    drawer_core::history_limits::verify::run()?;
    std::fs::create_dir_all(root).map_err(|e| e.to_string())?;
    let dir = root.join("history-limits-ui");
    std::fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let result = (|| -> Result<(), String> {
        let storage = UserStorage::at(dir.join("data")).map_err(|e| e.to_string())?;
        let settings = drawer_core::resource_settings::Settings {
            history: drawer_core::history_limits::Settings {
                steps: 2,
                memory_mib: 1,
                group_ms: 0,
            },
            ..Default::default()
        };
        settings.save(&storage).map_err(|e| e.to_string())?;
        let ctx = egui::Context::default();
        let mut app = Drawer::new(&ctx, None).with_storage(storage);
        app.create_canvas(CanvasSpec {
            width: 2,
            height: 2,
            dpi: 300.,
            background: Background::Transparent,
        })
        .map_err(|e| e.to_string())?;
        input_replay::frame(&ctx, &mut app, 0.1, vec![]);
        for n in 1..=4 {
            app.documents[0].edit_pixel(0, 0, 0, [n, 0, 0, 255]);
        }
        let before = app.documents[0].snapshot();
        if app.documents[0].history_counts() != (2, 0) || app.documents[0].history_pruned() != 2 {
            return Err("Persisted limits were not applied".into());
        }
        app.documents[0].undo();
        app.documents[0].redo();
        if app.documents[0].snapshot() != before {
            return Err("Pruned history changed current picture".into());
        }
        drop(app);
        Ok(())
    })();
    std::fs::remove_dir_all(dir).map_err(|e| e.to_string())?;
    result
}
#[cfg(test)]
mod limits_tests {
    #[test]
    fn persisted_limits_apply_to_session() {
        let root =
            std::env::temp_dir().join(format!("drawer-history-limits-ui-{}", std::process::id()));
        std::fs::create_dir(&root).unwrap();
        let r = super::limits_smoke(&root);
        std::fs::remove_dir_all(root).unwrap();
        r.unwrap();
    }
}
