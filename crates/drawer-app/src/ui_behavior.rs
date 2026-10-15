use super::*;
use drawer_core::ui_behavior::{AutoHide, Settings, Wake};
#[cfg(windows)]
type Token = std::rc::Rc<keepawake::KeepAwake>;
#[cfg(not(windows))]
type Token = ();
#[derive(Default)]
pub(super) struct Behavior {
    pub open: bool,
    draft: Option<Settings>,
    pub hidden: [AutoHide; 4],
    pub rectangles: [Option<egui::Rect>; 4],
    pub resize_skip: [u8; 4],
    pub wake: Wake<Token>,
    display: Option<bool>,
    pub error: Option<String>,
    #[cfg(test)]
    pub cancel_button: Option<egui::Rect>,
}
pub(super) fn scroll(ctx: &egui::Context) -> egui::scroll_area::ScrollBarVisibility {
    if ctx
        .data(|d| d.get_temp::<bool>(egui::Id::new("drawer-scrollbars")))
        .unwrap_or(true)
    {
        egui::scroll_area::ScrollBarVisibility::VisibleWhenNeeded
    } else {
        egui::scroll_area::ScrollBarVisibility::AlwaysHidden
    }
}
fn create_token(display: bool) -> Result<Token, String> {
    #[cfg(windows)]
    {
        keepawake::Builder::default()
            .idle(true)
            .display(display)
            .sleep(false)
            .app_name("Drawer")
            .reason("Local drawing or processing")
            .create()
            .map(std::rc::Rc::new)
            .map_err(|e| e.to_string())
    }
    #[cfg(not(windows))]
    {
        let _ = display;
        Err("Windows sleep prevention is unavailable on this platform".into())
    }
}
impl Drawer {
    pub(super) fn update_power(&mut self) {
        let s = self.workspaces.store.current.behavior;
        let working = self.canvas_view.working
            || self.workspaces.painting()
            || self.sizing_dialog.as_ref().is_some_and(|d| d.working())
            || self.import_dialog.as_ref().is_some_and(|d| d.working())
            || self.movie_dialog.as_ref().is_some_and(|d| d.working());
        if self.ui_behavior.display != Some(s.keep_display) {
            self.ui_behavior.wake.reset();
            self.ui_behavior.display = Some(s.keep_display);
        }
        if let Err(e) = self
            .ui_behavior
            .wake
            .update(s.prevent_sleep, working, || create_token(s.keep_display))
        {
            self.status = Some("Could not prevent idle sleep. Work can continue.".into());
            self.ui_behavior.error = Some(format!("Sleep prevention unavailable: {e}"));
        }
        if !s.prevent_sleep || !working {
            self.ui_behavior.error = None;
        }
    }
    pub(super) fn synchronous_power(&mut self) -> Option<Token> {
        let s = self.workspaces.store.current.behavior;
        if !s.prevent_sleep || self.ui_behavior.wake.active() {
            None
        } else {
            match create_token(s.keep_display) {
                Ok(t) => Some(t),
                Err(e) => {
                    self.status = Some("Could not prevent idle sleep. Work can continue.".into());
                    self.ui_behavior.error = Some(format!("Sleep prevention unavailable: {e}"));
                    None
                }
            }
        }
    }
    pub(super) fn panel_auto_hide(
        &mut self,
        ui: &egui::Ui,
        kind: drawer_core::panels::Kind,
    ) -> bool {
        let n = kind.index();
        let s = self.workspaces.store.current.behavior;
        let rect = self.ui_behavior.rectangles[n];
        let interactive = ui.ctx().egui_wants_keyboard_input()
            || ui.input(|i| {
                i.pointer.any_down()
                    || rect.is_some_and(|r| i.pointer.latest_pos().is_some_and(|p| r.contains(p)))
            });
        self.ui_behavior.hidden[n].update(
            s.auto_hide,
            interactive,
            ui.input(|i| i.time),
            s.hide_delay_ms,
        )
    }
    pub(super) fn behavior_settings(&mut self, ctx: &egui::Context) {
        if !self.ui_behavior.open {
            return;
        }
        let mut s = self
            .ui_behavior
            .draft
            .unwrap_or(self.workspaces.store.current.behavior);
        let mut open = true;
        let mut apply = false;
        let mut cancel = false;
        egui::Window::new("Panels, cursor and power").open(&mut open).show(ctx,|ui|{
            crate::help::checkbox(ui,&mut s.auto_hide,"Auto-hide panel contents when idle","behavior");super::edit::slider(ui,&mut s.hide_delay_ms,0..=5000,"Hide delay (ms)","behavior");crate::help::checkbox(ui,&mut s.scrollbars,"Show scroll bars","behavior");crate::help::checkbox(ui,&mut s.brush_cursor,"Show brush outline and tool cursor","behavior");
            crate::help::checkbox(ui,&mut s.edge_pan,"Move view near canvas edges while drawing","behavior");super::edit::slider(ui,&mut s.edge_margin,1.0..=128.0,"Edge margin (screen pixels)","behavior");super::edit::slider(ui,&mut s.edge_speed,1.0..=2000.0,"Edge speed (screen pixels/sec)","behavior");
            crate::help::checkbox(ui,&mut s.prevent_sleep,"Prevent automatic idle sleep while working","behavior");ui.add_enabled_ui(s.prevent_sleep,|ui|{crate::help::checkbox(ui,&mut s.keep_display,"Also keep the display on while working","behavior");});ui.label("Inactive work, cancellation and completion release the request. Manual sleep remains under Windows control.");
            if let Some(e)=&self.ui_behavior.error{ui.colored_label(egui::Color32::LIGHT_RED,e);}
            ui.horizontal(|ui|{apply=crate::help::button(ui,"Apply behavior","behavior").clicked();let c=crate::help::button(ui,"Cancel behavior","behavior");cancel=c.clicked();#[cfg(test)]{self.ui_behavior.cancel_button=Some(c.rect);}if crate::help::button(ui,"Reset draft","behavior").clicked(){s=Default::default();}});
            if ui.add_enabled(self.storage.is_some(),egui::Button::new("Load before-apply backup")).clicked(){match drawer_core::workspaces::Snapshot::read(&self.storage.as_ref().unwrap().root().join("settings/workspace-before-apply.json")){Ok(p)=>s=p.behavior,Err(e)=>self.status=Some(e.to_string())}}
        });
        if cancel || !open || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.ui_behavior.open = false;
            self.ui_behavior.draft = None;
        } else if apply {
            let mut snapshot = self.capture_workspace();
            snapshot.behavior = s;
            self.stage_workspace(snapshot, self.workspaces.store.active.clone());
            match self.apply_workspace(
                drawer_core::workspaces::Scope {
                    other: true,
                    ..Default::default()
                },
                None,
            ) {
                Ok(()) => {
                    self.ui_behavior.open = false;
                    self.ui_behavior.draft = None;
                }
                Err(e) => {
                    self.status = Some(e);
                    self.workspaces.staged = None;
                    self.ui_behavior.draft = Some(s);
                }
            }
        } else {
            self.ui_behavior.draft = Some(s);
        }
    }
    pub(super) fn verify_native_power(&mut self) -> Result<(), String> {
        self.ui_behavior
            .wake
            .update(true, true, || create_token(false))?;
        if !self.ui_behavior.wake.active() {
            return Err("Sleep lease did not activate".into());
        }
        self.ui_behavior
            .wake
            .update(false, false, || create_token(false))?;
        if self.ui_behavior.wake.active() {
            return Err("Sleep lease did not release".into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{TestDirectory, click, frame, tiny_spec};
    #[test]
    fn panel_hide_hover_cursor_scrollbars_edge_pan_release_and_cancel_are_read_only() {
        let ctx = egui::Context::default();
        let mut app = Drawer::new(&ctx, None);
        app.create_canvas(tiny_spec()).unwrap();
        frame(&ctx, &mut app, vec![]);
        let before = app.documents[0].snapshot();
        let history = app.documents[0].history_counts();
        let s = &mut app.workspaces.store.current.behavior;
        s.auto_hide = true;
        s.hide_delay_ms = 0;
        s.scrollbars = false;
        s.brush_cursor = false;
        s.edge_pan = true;
        frame(&ctx, &mut app, vec![]);
        assert!(app.ui_behavior.hidden[0].collapsed);
        let header = app.ui_behavior.rectangles[0].unwrap().center();
        frame(&ctx, &mut app, vec![egui::Event::PointerMoved(header)]);
        assert!(!app.ui_behavior.hidden[0].collapsed);
        assert_eq!(
            scroll(&ctx),
            egui::scroll_area::ScrollBarVisibility::AlwaysHidden
        );
        let rect = app.canvas_view.viewport.unwrap();
        let point = egui::pos2(rect.max.x - 2.0, rect.center().y);
        frame(&ctx, &mut app, vec![egui::Event::PointerMoved(point)]);
        assert!(app.canvas_view.brush_outline.is_empty());
        let old = app.documents[0].view.pan;
        frame(
            &ctx,
            &mut app,
            vec![egui::Event::PointerButton {
                pos: point,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: Default::default(),
            }],
        );
        frame(&ctx, &mut app, vec![egui::Event::PointerMoved(point)]);
        assert!(app.documents[0].view.pan[0] < old[0]);
        assert!(app.canvas_view.working);
        frame(
            &ctx,
            &mut app,
            vec![egui::Event::PointerButton {
                pos: egui::Pos2::ZERO,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: Default::default(),
            }],
        );
        let stopped = app.documents[0].view.pan;
        frame(&ctx, &mut app, vec![]);
        assert_eq!(app.documents[0].view.pan, stopped);
        assert!(!app.canvas_view.working);
        assert!(!app.ui_behavior.wake.active());
        let current = app.workspaces.store.current.behavior;
        app.ui_behavior.open = true;
        frame(&ctx, &mut app, vec![]);
        app.ui_behavior.draft.as_mut().unwrap().auto_hide = false;
        frame(&ctx, &mut app, vec![]);
        let cancel = app.ui_behavior.cancel_button.unwrap();
        click(&ctx, &mut app, cancel);
        assert!(!app.ui_behavior.open);
        assert_eq!(app.workspaces.store.current.behavior, current);
        assert_eq!(app.documents[0].snapshot(), before);
        assert_eq!(app.documents[0].history_counts(), history);
    }
    #[test]
    fn behavior_restart_scoped_apply_corruption_and_backup_preserve_document() {
        let root = TestDirectory::new();
        let storage = UserStorage::at(root.0.join("user")).unwrap();
        let ctx = egui::Context::default();
        let mut app = Drawer::new(&ctx, None)
            .with_storage(UserStorage::at(storage.root().to_path_buf()).unwrap());
        app.create_canvas(tiny_spec()).unwrap();
        frame(&ctx, &mut app, vec![]);
        let before = app.capture_workspace();
        let document = app.documents[0].snapshot();
        let mut incoming = before.clone();
        incoming.behavior.auto_hide = true;
        incoming.behavior.brush_cursor = false;
        incoming.behavior.edge_pan = true;
        incoming.behavior.hide_delay_ms = 100;
        app.stage_workspace(incoming.clone(), None);
        app.apply_workspace(
            drawer_core::workspaces::Scope {
                other: true,
                ..Default::default()
            },
            None,
        )
        .unwrap();
        assert_eq!(
            drawer_core::workspaces::Snapshot::read(
                &storage.root().join("settings/workspace-before-apply.json")
            )
            .unwrap(),
            before
        );
        let restarted = Drawer::new(&egui::Context::default(), None)
            .with_storage(UserStorage::at(storage.root().to_path_buf()).unwrap());
        assert_eq!(
            restarted.workspaces.store.current.behavior,
            incoming.behavior
        );
        let saved = app.capture_workspace();
        incoming.behavior.edge_speed = f64::NAN;
        app.stage_workspace(incoming, None);
        assert!(
            app.apply_workspace(drawer_core::workspaces::Scope::ALL, None)
                .is_err()
        );
        assert_eq!(app.capture_workspace(), saved);
        assert_eq!(app.documents[0].snapshot(), document);
        assert_eq!(app.documents[0].history_counts(), (0, 0));
    }
}
