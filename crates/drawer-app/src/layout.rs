use super::*;
use drawer_core::{
    layout::{Arrangement, Presentation, Settings},
    panels::Kind,
    workspaces::Dock,
};
use std::collections::HashMap;
#[derive(Default)]
pub(super) struct Layout {
    pub open: bool,
    pub generation: u64,
    draft: Option<Settings>,
    confirm_reset: bool,
    hide_reset: bool,
    popup: Option<Kind>,
    views: HashMap<u64, canvas_view::CanvasView>,
    pub rendered_windows: usize,
    #[cfg(test)]
    pub panel_rects: HashMap<Kind, egui::Rect>,
    #[cfg(test)]
    pub canvas_rects: Vec<egui::Rect>,
    #[cfg(test)]
    pub dock_rects: [Option<egui::Rect>; 2],
    #[cfg(test)]
    height_handles: [Option<egui::Id>; 2],
    #[cfg(test)]
    apply_button: Option<egui::Rect>,
    #[cfg(test)]
    cancel_button: Option<egui::Rect>,
}
impl Drawer {
    pub(super) fn layout_settings(&mut self, ctx: &egui::Context) {
        if !self.layout.open {
            return;
        }
        let mut draft = self
            .layout
            .draft
            .take()
            .unwrap_or_else(|| self.workspaces.store.current.layout.clone());
        let mut open = true;
        let mut apply = false;
        let mut cancel = false;
        egui::Window::new("Dock and canvas layout")
            .open(&mut open)
            .constrain(true)
            .vscroll(true)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    let a = crate::help::button(ui, "Apply layout", "workspace");
                    let c = crate::help::button(ui, "Cancel layout", "workspace");
                    apply = a.clicked();
                    cancel = c.clicked();
                    #[cfg(test)]
                    {
                        self.layout.apply_button = Some(a.rect);
                        self.layout.cancel_button = Some(c.rect);
                    }
                    if crate::help::button(ui, "Reset layout draft", "workspace").clicked() {
                        if self
                            .workspaces
                            .store
                            .current
                            .preferences
                            .suppressed
                            .contains(&drawer_core::preferences::Confirmation::ResetLayout)
                        {
                            draft = Default::default();
                        } else {
                            self.layout.confirm_reset = true;
                        }
                    }
                });
                if self.layout.confirm_reset {
                    ui.label(
                        "Reset the layout draft? Apply is still required to change your workspace.",
                    );
                    crate::help::checkbox(
                        ui,
                        &mut self.layout.hide_reset,
                        "Do not show this reset confirmation again",
                        "preferences",
                    );
                    ui.horizontal(|ui| {
                        if crate::help::button(ui, "Confirm layout draft reset", "preferences")
                            .clicked()
                        {
                            draft = Default::default();
                            self.layout.confirm_reset = false;
                            if self.layout.hide_reset {
                                self.workspaces
                                    .store
                                    .current
                                    .preferences
                                    .suppressed
                                    .insert(drawer_core::preferences::Confirmation::ResetLayout);
                            }
                        }
                        if crate::help::button(ui, "Keep layout draft", "preferences").clicked() {
                            self.layout.confirm_reset = false;
                            self.layout.hide_reset = false;
                        }
                    });
                }
                crate::help::checkbox(ui, &mut draft.titles, "Show title bars", "workspace");
                crate::help::checkbox(ui, &mut draft.menu, "Show command menu line", "workspace");
                ui.label(
                    "The Layout button and screen switch stay available when bars are hidden.",
                );
                for n in 0..2 {
                    ui.push_id(n, |ui| {
                        ui.heading(["Left dock", "Right dock"][n]);
                        ui.horizontal(|ui| {
                            ui.label("Width / height (screen pixels)");
                            super::edit::number(
                                ui,
                                &mut draft.width[n],
                                80.0..=4096.0,
                                1.0,
                                "",
                                "workspace",
                            );
                            super::edit::number(
                                ui,
                                &mut draft.height[n],
                                80.0..=4096.0,
                                1.0,
                                "",
                                "workspace",
                            );
                        });
                        lock_ui(ui, &mut draft.dock_locks[n]);
                        ui.horizontal(|ui| {
                            for mode in
                                [Presentation::Stack, Presentation::Tabs, Presentation::Popup]
                            {
                                ui.selectable_value(
                                    &mut draft.presentation[n],
                                    mode,
                                    format!("{mode:?}"),
                                );
                            }
                        });
                    });
                }
                ui.heading("Floating panel locks");
                for kind in Kind::ALL {
                    ui.push_id(kind.index() + 2, |ui| {
                        ui.label(kind.label());
                        lock_ui(ui, &mut draft.panel_locks[kind.index()]);
                    });
                }
                ui.heading("Canvas arrangement");
                ui.horizontal(|ui| {
                    for mode in [Arrangement::Tabs, Arrangement::Cascade, Arrangement::Tile] {
                        ui.selectable_value(&mut draft.arrangement, mode, format!("{mode:?}"));
                    }
                });
            });
        if cancel || !open || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.layout.open = false;
            self.layout.draft = None;
            self.layout.confirm_reset = false;
            self.layout.hide_reset = false;
        } else if apply {
            let mut snapshot = self.capture_workspace();
            snapshot.layout = draft.clone();
            self.stage_workspace(snapshot, self.workspaces.store.active.clone());
            match self.apply_workspace(
                drawer_core::workspaces::Scope {
                    layout: true,
                    ..Default::default()
                },
                None,
            ) {
                Ok(()) => {
                    self.layout.open = false;
                    self.layout.draft = None;
                }
                Err(e) => {
                    self.status = Some(e);
                    self.workspaces.staged = None;
                    self.layout.draft = Some(draft);
                }
            }
        } else {
            self.layout.draft = Some(draft);
        }
    }
    pub(super) fn dock_contents(&mut self, ui: &mut egui::Ui, dock: Dock) {
        let n = usize::from(dock == Dock::Right);
        let mode = self.workspaces.store.current.layout.presentation[n];
        let kinds = self
            .workspaces
            .store
            .current
            .order
            .into_iter()
            .filter(|kind| {
                self.panels.settings.panels[kind.index()].visible
                    && self.workspaces.store.current.placements[kind.index()].dock == dock
            })
            .collect::<Vec<_>>();
        if mode != Presentation::Stack {
            ui.horizontal_wrapped(|ui| {
                for (index, kind) in kinds.iter().enumerate() {
                    if crate::help::selectable(
                        ui,
                        self.workspaces.store.current.layout.selected[n] == index,
                        kind.label(),
                        "workspace",
                    )
                    .clicked()
                    {
                        self.workspaces.store.current.layout.selected[n] = index;
                        if mode == Presentation::Popup {
                            self.layout.popup = Some(*kind);
                        }
                    }
                }
            });
        }
        if mode == Presentation::Popup {
            return;
        }
        egui::ScrollArea::vertical()
            .id_salt(("dock-scroll", n))
            .scroll_bar_visibility(crate::ui_behavior::scroll(ui.ctx()))
            .auto_shrink([false, false])
            .max_height(ui.available_height())
            .show(ui, |ui| {
                for (index, kind) in kinds.iter().enumerate() {
                    if mode == Presentation::Stack
                        || index
                            == self.workspaces.store.current.layout.selected[n]
                                .min(kinds.len().saturating_sub(1))
                    {
                        #[cfg(test)]
                        let top = ui.cursor().min;
                        self.show_panel(ui, *kind);
                        #[cfg(test)]
                        self.layout
                            .panel_rects
                            .insert(*kind, egui::Rect::from_min_max(top, ui.min_rect().max));
                    }
                }
            });
    }
    pub(super) fn docked_workspace(&mut self, ui: &mut egui::Ui) {
        self.canvas_view.dock_navigator = true;
        let ppp = ui.ctx().pixels_per_point() as f64;
        let width = ui.available_width();
        for (n, dock) in [(0, Dock::Left), (1, Dock::Right)] {
            if !(dock == Dock::Left
                && self.canvas_view.navigator.visible
                && !self.canvas_view.navigator.floating)
                && !self.workspaces.store.current.order.iter().any(|k| {
                    self.panels.settings.panels[k.index()].visible
                        && self.workspaces.store.current.placements[k.index()].dock == dock
                })
            {
                continue;
            }
            let cfg = self.workspaces.store.current.layout.clone();
            let wanted = (cfg.width[n] / ppp) as f32;
            let width = wanted.clamp((80.0 / ppp) as f32, (width * 0.4).max((80.0 / ppp) as f32));
            let panel = if n == 0 {
                egui::Panel::left(egui::Id::new((
                    "drawer-dock-left",
                    self.layout.generation,
                    ppp.to_bits(),
                )))
            } else {
                egui::Panel::right(egui::Id::new((
                    "drawer-dock-right",
                    self.layout.generation,
                    ppp.to_bits(),
                )))
            };
            let panel = panel
                .resizable(!cfg.dock_locks[n].width)
                .size_range(
                    (80.0 / ppp) as f32..=(ui.available_width() * 0.6).max((80.0 / ppp) as f32),
                )
                .default_size(width);
            let panel = if cfg.dock_locks[n].width {
                panel.exact_size(width)
            } else {
                panel
            };
            let response = panel.show(ui, |ui| {
                let reserve = if dock == Dock::Left
                    && self.canvas_view.navigator.visible
                    && !self.canvas_view.navigator.floating
                {
                    self.canvas_view.navigator.size[1] + 90.0
                } else {
                    0.0
                };
                let limit = (ui.available_height() - reserve).max((80.0 / ppp) as f32);
                let wanted = ((cfg.height[n] / ppp) as f32).min(limit);
                let fixed_width = ui.available_width();
                let mut height = wanted;
                let resize_id = ui.make_persistent_id((
                    "dock-height",
                    n,
                    self.layout.generation,
                    ppp.to_bits(),
                ));
                #[cfg(test)]
                {
                    self.layout.height_handles[n] = Some(resize_id.with("__resize_corner"));
                }
                egui::Resize::default()
                    .id(resize_id)
                    .default_size([fixed_width, wanted])
                    .min_size([
                        fixed_width,
                        if cfg.dock_locks[n].height {
                            wanted
                        } else {
                            (80.0 / ppp) as f32
                        },
                    ])
                    .max_size([
                        fixed_width,
                        if cfg.dock_locks[n].height {
                            wanted
                        } else {
                            limit
                        },
                    ])
                    .resizable([false, !cfg.dock_locks[n].height])
                    .show(ui, |ui| {
                        self.dock_contents(ui, dock);
                        height = ui.min_rect().height();
                    });
                if !cfg.dock_locks[n].height
                    && ui.input(|i| i.pointer.primary_down() || i.pointer.primary_released())
                    && (height - wanted).abs() > 0.5
                {
                    self.workspaces.store.current.layout.height[n] =
                        (height as f64 * ppp).clamp(80.0, 4096.0);
                }
                if dock == Dock::Left
                    && self.canvas_view.navigator.visible
                    && !self.canvas_view.navigator.floating
                    && let Some(active) = self.active
                {
                    self.canvas_view
                        .navigator_in_dock(ui, &mut self.documents[active], ppp);
                }
            });
            #[cfg(test)]
            {
                self.layout.dock_rects[n] = Some(response.response.rect);
            }
            if !cfg.dock_locks[n].width {
                self.workspaces.store.current.layout.width[n] =
                    (response.response.rect.width() as f64 * ppp).clamp(80.0, 4096.0);
            }
        }
        if self.workspaces.store.current.layout.arrangement != Arrangement::Tabs {
            ui.label("Canvas windows are arranged in this workspace.");
            return;
        }
        self.canvas_view.dock_navigator = true;
        self.simple_canvas(ui);
    }
    pub(super) fn layout_windows(&mut self, ctx: &egui::Context) {
        let cfg = self.workspaces.store.current.layout.clone();
        ctx.send_viewport_cmd(egui::ViewportCommand::Decorations(cfg.titles));
        if let Some(kind) = self.layout.popup {
            let mut open = true;
            let mut dismiss = false;
            egui::Window::new(format!("{} popup", kind.label()))
                .open(&mut open)
                .title_bar(cfg.titles)
                .constrain(true)
                .show(ctx, |ui| {
                    dismiss = crate::help::button(ui, "Close popup", "workspace").clicked();
                    self.show_panel(ui, kind);
                });
            if !open || dismiss {
                self.layout.popup = None;
            }
        }
        self.layout
            .views
            .retain(|id, _| self.tabs.iter().any(|t| t.id == *id));
        self.layout.rendered_windows = 0;
        #[cfg(test)]
        self.layout.canvas_rects.clear();
        if cfg.arrangement == Arrangement::Tabs || self.simple_view {
            return;
        }
        self.sync_active_view();
        let area = ctx.content_rect().shrink2(egui::vec2(8.0, 48.0));
        let rects = drawer_core::layout::arrange(
            cfg.arrangement,
            self.tabs.len(),
            [area.width() as f64, area.height() as f64],
            ctx.pixels_per_point() as f64,
        );
        let mut activate = None;
        let mut close = None;
        for (index, rect) in rects.iter().enumerate() {
            let tab = &mut self.tabs[index];
            let Some(doc) = self.documents.iter().find(|d| d.id == tab.document_id) else {
                continue;
            };
            let canvas = self
                .layout
                .views
                .entry(tab.id)
                .or_insert_with(|| canvas_view::CanvasView::new(ctx));
            canvas.filter = self.interpolation.view;
            let frame = egui::Rect::from_min_size(
                area.min + egui::vec2(rect.position[0] as f32, rect.position[1] as f32),
                egui::vec2(rect.size[0] as f32, rect.size[1] as f32),
            );
            let mut open = true;
            let response = egui::Window::new(format!(
                "{}{}",
                doc.title,
                if doc.dirty { " *" } else { "" }
            ))
            .id(egui::Id::new(("arranged-canvas", tab.id)))
            .open(&mut open)
            .title_bar(cfg.titles)
            .fixed_rect(frame)
            .min_size([1.0, 1.0])
            .show(ctx, |ui| {
                canvas.show_only(ui, doc, &mut tab.view, true);
                if ui.rect_contains_pointer(ui.min_rect())
                    && ui.input(|i| i.pointer.primary_pressed())
                {
                    activate = Some(index);
                }
            });
            if !open {
                close = Some(index);
            }
            if let Some(_response) = response {
                self.layout.rendered_windows += 1;
                #[cfg(test)]
                self.layout.canvas_rects.push(_response.response.rect);
            }
        }
        if let (Some(n), Some(t)) = (self.active, self.active_tab) {
            self.documents[n].view = self.tabs[t].view;
        }
        if let Some(n) = activate {
            self.select_tab(n);
        }
        if let Some(n) = close {
            self.request_close(n);
        }
    }
}
fn lock_ui(ui: &mut egui::Ui, lock: &mut drawer_core::layout::Lock) {
    ui.horizontal(|ui| {
        crate::help::checkbox(ui, &mut lock.width, "Lock width", "workspace");
        crate::help::checkbox(ui, &mut lock.height, "Lock height", "workspace");
        crate::help::checkbox(ui, &mut lock.position, "Lock position/docking", "workspace");
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{TestDirectory, click, frame, tiny_spec};
    #[test]
    fn stack_tabs_popup_bars_and_arranged_views_preserve_documents_and_close_safely() {
        let ctx = egui::Context::default();
        let mut app = Drawer::new(&ctx, None);
        app.create_canvas(tiny_spec()).unwrap();
        app.duplicate_view();
        app.create_canvas(tiny_spec()).unwrap();
        frame(&ctx, &mut app, vec![]);
        let before = app
            .documents
            .iter()
            .map(|d| (d.snapshot(), d.history_counts(), d.dirty))
            .collect::<Vec<_>>();
        assert!(app.layout.panel_rects.len() > 1);
        app.workspaces.store.current.layout.presentation[0] = Presentation::Tabs;
        app.workspaces.store.current.layout.selected[0] = 1;
        frame(&ctx, &mut app, vec![]);
        assert_eq!(app.layout.panel_rects.len(), 1);
        assert!(app.layout.panel_rects.contains_key(&Kind::Color));
        app.workspaces.store.current.layout.presentation[0] = Presentation::Popup;
        app.layout.popup = Some(Kind::Brush);
        app.workspaces.store.current.layout.titles = false;
        app.workspaces.store.current.layout.menu = false;
        frame(&ctx, &mut app, vec![]);
        assert!(app.layout.panel_rects.is_empty());
        assert!(app.commands_button.is_some());
        assert!(app.screen_button.is_some());
        app.layout.popup = None;
        app.workspaces.store.current.layout.titles = true;
        for mode in [Arrangement::Tile, Arrangement::Cascade] {
            app.workspaces.store.current.layout.arrangement = mode;
            frame(&ctx, &mut app, vec![]);
            frame(&ctx, &mut app, vec![]);
            assert_eq!(app.layout.rendered_windows, 3);
            assert_eq!(app.layout.views.len(), 3);
            for rect in &app.layout.canvas_rects {
                assert!(rect.min.x >= 0.0 && rect.min.y >= 0.0);
            }
        }
        app.request_close(0);
        frame(&ctx, &mut app, vec![]);
        assert_eq!(app.tabs.len(), 2);
        assert_eq!(app.layout.views.len(), 2);
        assert_eq!(
            app.documents
                .iter()
                .map(|d| (d.snapshot(), d.history_counts(), d.dirty))
                .collect::<Vec<_>>(),
            before
        );
        app.workspaces.store.current.layout.arrangement = Arrangement::Tabs;
        frame(&ctx, &mut app, vec![]);
        assert_eq!(app.layout.rendered_windows, 0);
    }
    #[test]
    fn locked_dock_drag_and_layout_cancel_apply_backup_restart_are_local() {
        let root = TestDirectory::new();
        let storage = UserStorage::at(root.0.join("user")).unwrap();
        let ctx = egui::Context::default();
        let mut app = Drawer::new(&ctx, None)
            .with_storage(UserStorage::at(storage.root().to_path_buf()).unwrap());
        app.create_canvas(tiny_spec()).unwrap();
        app.workspaces.store.current.layout.dock_locks[0] = drawer_core::layout::Lock {
            width: true,
            height: true,
            position: true,
        };
        frame(&ctx, &mut app, vec![]);
        frame(&ctx, &mut app, vec![]);
        let snapshot = app.documents[0].snapshot();
        let history = app.documents[0].history_counts();
        let width = app.workspaces.store.current.layout.width[0];
        let rect = app.layout.dock_rects[0].unwrap();
        let start = egui::pos2(rect.right(), rect.center().y);
        let end = start + egui::vec2(100.0, 0.0);
        for events in [
            vec![egui::Event::PointerMoved(start)],
            vec![egui::Event::PointerButton {
                pos: start,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: Default::default(),
            }],
            vec![egui::Event::PointerMoved(end)],
            vec![egui::Event::PointerButton {
                pos: end,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: Default::default(),
            }],
        ] {
            frame(&ctx, &mut app, events);
        }
        assert_eq!(app.workspaces.store.current.layout.width[0], width);
        assert!((app.layout.dock_rects[0].unwrap().width() as f64 - width).abs() < 2.0);
        assert!(
            ctx.read_response(app.layout.height_handles[0].unwrap())
                .is_none()
        );
        app.workspaces.store.current.layout.dock_locks[0].height = false;
        app.workspaces.store.current.layout.height[0] = 180.0;
        app.layout.generation += 1;
        frame(&ctx, &mut app, vec![]);
        frame(&ctx, &mut app, vec![]);
        let corner = ctx
            .read_response(app.layout.height_handles[0].unwrap())
            .unwrap()
            .rect
            .center();
        let end = corner - egui::vec2(0.0, 30.0);
        for events in [
            vec![egui::Event::PointerMoved(corner)],
            vec![egui::Event::PointerButton {
                pos: corner,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: Default::default(),
            }],
            vec![egui::Event::PointerMoved(end)],
            vec![egui::Event::PointerMoved(end)],
            vec![egui::Event::PointerButton {
                pos: end,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: Default::default(),
            }],
        ] {
            frame(&ctx, &mut app, events);
        }
        assert!(app.workspaces.store.current.layout.height[0] < 180.0);
        app.workspaces.store.current.layout.dock_locks[0].height = true;
        app.layout.generation += 1;
        frame(&ctx, &mut app, vec![]);
        assert!(
            ctx.read_response(app.layout.height_handles[0].unwrap())
                .is_none()
        );
        let current = app.capture_workspace();
        let mut draft = current.layout.clone();
        draft.width = [80.0, 4096.0];
        draft.height = [4096.0, 80.0];
        draft.presentation = [Presentation::Tabs, Presentation::Popup];
        app.layout.open = true;
        app.layout.draft = Some(draft.clone());
        frame(&ctx, &mut app, vec![]);
        frame(&ctx, &mut app, vec![]);
        let button = app.layout.cancel_button.unwrap();
        click(&ctx, &mut app, button);
        assert_eq!(app.capture_workspace(), current);
        app.layout.open = true;
        app.layout.draft = Some(draft.clone());
        frame(&ctx, &mut app, vec![]);
        frame(&ctx, &mut app, vec![]);
        let button = app.layout.apply_button.unwrap();
        click(&ctx, &mut app, button);
        assert_eq!(app.workspaces.store.current.layout, draft);
        let restarted = Drawer::new(&egui::Context::default(), None)
            .with_storage(UserStorage::at(storage.root().to_path_buf()).unwrap());
        assert_eq!(restarted.workspaces.store.current.layout, draft);
        assert_eq!(
            drawer_core::workspaces::Snapshot::read(
                &storage.root().join("settings/workspace-before-apply.json")
            )
            .unwrap()
            .layout,
            current.layout
        );
        assert_eq!(app.documents[0].snapshot(), snapshot);
        assert_eq!(app.documents[0].history_counts(), history);
    }
}
