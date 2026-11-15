//! GPU window recovery uses a fresh UI context and moves the live document/history owners.
use crate::*;
use std::{
    cell::RefCell,
    rc::Rc,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use winit::{
    application::ApplicationHandler,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    platform::run_on_demand::EventLoopExtRunOnDemand,
    window::{Window, WindowId},
};
pub(super) type Factory = Box<dyn FnMut(&egui::Context) -> Result<Box<Drawer>, String>>;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Injection {
    Init,
    Context,
    Memory,
}
#[derive(Default)]
struct Bridge {
    app: Option<Box<Drawer>>,
    reason: Option<String>,
    gpu: Option<[String; 3]>,
    gpu_color: bool,
    closed: bool,
}
struct Proxy {
    app: Option<Box<Drawer>>,
    gl: Option<Arc<eframe::glow::Context>>,
    bridge: Rc<RefCell<Bridge>>,
    close_requested: bool,
}
fn prepare_failure(app: &mut Drawer) {
    app.cancel_graphics_input();
    app.drain_background();
    if app.storage.is_some() {
        let _ = app.checkpoint_recovery();
    }
}
impl eframe::App for Proxy {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let error = self
            .gl
            .as_ref()
            .ok_or_else(|| "GPU context unavailable".to_string())
            .and_then(|gl| drawer_platform::graphics::health(gl))
            .err();
        if let Some(reason) = error {
            self.bridge.borrow_mut().reason = Some(reason);
        }
        if self.bridge.borrow().reason.is_some() {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        self.close_requested = ui.ctx().input(|i| i.viewport().close_requested());
        if let Some(app) = &mut self.app {
            eframe::App::ui(app.as_mut(), ui, frame);
            if app.graphics.request_cpu {
                self.bridge.borrow_mut().reason = Some("Software display requested".into());
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
    }
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        if let Some(app) = &mut self.app {
            let closed = self.close_requested || app.quit_ready;
            self.bridge.borrow_mut().closed = closed;
            if self.bridge.borrow().reason.is_some() || !closed {
                prepare_failure(app);
            }
        }
    }
}
impl Drop for Proxy {
    fn drop(&mut self) {
        self.bridge.borrow_mut().app = self.app.take();
    }
}
fn attach(app: &mut Drawer, input: &drawer_platform::Input, window: &Arc<Window>) {
    app.native_dial = Some(drawer_platform::Dial::new(window));
    match input.attach(window) {
        Ok(()) => {
            let cancel = input.clone();
            app.drawing.cancel_check = Some(Box::new(move || cancel.cancel_requested()));
            app.native_mouse = Some(input.clone());
        }
        Err(e) => app.status = Some(e),
    }
}
pub(super) fn transfer(mut old: Box<Drawer>, ctx: &egui::Context) -> Result<Box<Drawer>, String> {
    old.cancel_graphics_input();
    old.stop_timelapse();
    old.settle_work_time();
    old.drain_background();
    let before = old
        .documents
        .iter()
        .map(|d| (d.snapshot(), d.history_counts(), d.path.clone(), d.dirty))
        .collect::<Vec<_>>();
    let snapshot = old.capture_workspace();
    let mut next = Box::new(Drawer::new(ctx, None));
    if let Some(storage) = old.storage.clone() {
        *next = next.with_storage_recovery(storage, false);
    }
    next.workspaces.store = old.workspaces.store.clone();
    next.restore_workspace(&snapshot);
    next.workspaces.store.current = snapshot;
    next.documents = std::mem::take(&mut old.documents);
    for d in &mut next.documents {
        d.history_barrier();
    }
    next.tabs = std::mem::take(&mut old.tabs);
    next.active = old.active;
    next.active_tab = old.active_tab;
    next.next_tab_id = old.next_tab_id;
    next.view_session = old.view_session;
    next.saved_layout = old.saved_layout;
    next.last_layout = old.last_layout;
    next.quality_undo = std::mem::take(&mut old.quality_undo);
    next.quality_redo = std::mem::take(&mut old.quality_redo);
    next.navigator_undo = std::mem::take(&mut old.navigator_undo);
    next.navigator_redo = std::mem::take(&mut old.navigator_redo);
    next.navigator_saved = old.navigator_saved;
    std::mem::swap(&mut next.recent, &mut old.recent);
    next.work_time = std::mem::take(&mut old.work_time);
    next.timelapse = std::mem::take(&mut old.timelapse);
    next.comics_view_session = std::mem::take(&mut old.comics_view_session);
    next.default_view = old.default_view;
    next.interface.ppp = old.interface.ppp;
    next.simple_view = old.simple_view;
    next.panels.targets = std::mem::take(&mut old.panels.targets);
    next.resources.current = old.resources.current.clone();
    next.protected_sources = std::mem::take(&mut old.protected_sources);
    next.page_work = std::mem::take(&mut old.page_work);
    next.page_work.reset_graphics();
    std::mem::swap(&mut next.references.session, &mut old.references.session);
    next.materials.library = old.materials.library.clone();
    next.recovery = std::mem::take(&mut old.recovery);
    next.drawing.mode = old.drawing.mode;
    next.selected = old.selected;
    next.graphics.settings = old.graphics.settings.clone();
    next.graphics.running_cpu = true;
    if next
        .documents
        .iter()
        .map(|d| (d.snapshot(), d.history_counts(), d.path.clone(), d.dirty))
        .collect::<Vec<_>>()
        != before
    {
        return Err("Display recovery altered document/history owners".into());
    }
    ctx.set_zoom_factor(next.interface.settings.zoom);
    Ok(next)
}
pub(super) fn run(
    mut options: eframe::NativeOptions,
    factory: Factory,
    input: drawer_platform::Input,
    strict: bool,
    force_cpu: bool,
    injection: Option<Injection>,
    test_root: Option<PathBuf>,
) -> Result<(), String> {
    let factory = Rc::new(RefCell::new(factory));
    let bridge = Rc::new(RefCell::new(Bridge::default()));
    let preferred = UserStorage::from_environment()
        .ok()
        .and_then(|s| graphics_settings::Settings::read(&s).ok())
        .is_some_and(|s| s.software);
    let mut builder = EventLoop::<eframe::UserEvent>::with_user_event();
    if let Some(configure) = options.event_loop_builder.take() {
        configure(&mut builder);
    }
    let mut event_loop = builder.build().map_err(|e| e.to_string())?;
    if strict || (!force_cpu && !preferred) || injection.is_some() {
        let shared = bridge.clone();
        let create = factory.clone();
        let native_input = input.clone();
        let mut native = eframe::create_native(
            APP_NAME,
            options,
            Box::new(move |cc| {
                let gl = cc
                    .gl
                    .as_deref()
                    .ok_or_else(|| std::io::Error::other("GPU context missing"))?;
                shared.borrow_mut().gpu = drawer_platform::graphics::identity(gl).ok();
                if strict || injection.is_some() {
                    drawer_platform::gpu_color::verify(gl).map_err(std::io::Error::other)?;
                    shared.borrow_mut().gpu_color = true;
                }
                if injection == Some(Injection::Init) {
                    shared.borrow_mut().reason = Some("Injected GPU initialization failure".into());
                    return Err(std::io::Error::other("Injected GPU initialization failure").into());
                }
                let mut app = create.borrow_mut()(&cc.egui_ctx).map_err(std::io::Error::other)?;
                if let Some(window) = cc.winit_window() {
                    attach(&mut app, &native_input, window);
                }
                if matches!(injection, Some(Injection::Context | Injection::Memory)) {
                    let reason = if injection == Some(Injection::Context) {
                        drawer_platform::graphics::detach_current_for_verification()?;
                        drawer_platform::graphics::health(gl)
                            .expect_err("Detached own context must be unavailable")
                    } else {
                        "Injected OpenGL out-of-memory".into()
                    };
                    prepare_failure(&mut app);
                    let mut b = shared.borrow_mut();
                    b.reason = Some(reason);
                    b.app = Some(app);
                    return Err(std::io::Error::other("Owned graphics failure injection").into());
                }
                Ok(Box::new(Proxy {
                    app: Some(app),
                    gl: cc.gl.clone(),
                    bridge: shared.clone(),
                    close_requested: false,
                }))
            }),
            &event_loop,
        );
        let result = event_loop.run_app_on_demand(&mut native);
        drop(native);
        if strict {
            if let Some(mut app) = bridge.borrow_mut().app.take() {
                eframe::App::on_exit(app.as_mut(), None);
            }
            return result.map_err(|e| e.to_string()).and_then(|()| {
                if bridge.borrow().closed {
                    Ok(())
                } else {
                    Err("Native graphics verification did not close normally".into())
                }
            });
        }
        if bridge.borrow().reason.is_none() && bridge.borrow().closed {
            if let Some(mut app) = bridge.borrow_mut().app.take() {
                eframe::App::on_exit(app.as_mut(), None);
            }
            return Ok(());
        }
        if bridge.borrow().reason.is_none() {
            bridge.borrow_mut().reason = Some(result.err().map_or_else(
                || "Graphics window ended unexpectedly".into(),
                |e| e.to_string(),
            ));
        }
    }
    input.detach();
    let mut software = Software::new(factory, bridge, input, test_root);
    event_loop
        .run_app_on_demand(&mut software)
        .map_err(|e| e.to_string())?;
    software.finish()
}
struct Software {
    factory: Rc<RefCell<Factory>>,
    bridge: Rc<RefCell<Bridge>>,
    input: drawer_platform::Input,
    ctx: egui::Context,
    window: Option<Arc<Window>>,
    state: Option<egui_winit::State>,
    app: Option<Box<Drawer>>,
    painter: software_painter::Painter,
    next: Arc<Mutex<Option<Instant>>>,
    dirty: bool,
    close: bool,
    error: Option<String>,
    pending: Vec<egui::Event>,
    test: Option<crate::graphics_verify::Case>,
    frames: u64,
    stopped: bool,
}
impl Software {
    fn new(
        factory: Rc<RefCell<Factory>>,
        bridge: Rc<RefCell<Bridge>>,
        input: drawer_platform::Input,
        test_root: Option<PathBuf>,
    ) -> Self {
        Self {
            factory,
            bridge,
            input,
            ctx: egui::Context::default(),
            window: None,
            state: None,
            app: None,
            painter: Default::default(),
            next: Arc::default(),
            dirty: true,
            close: false,
            error: None,
            pending: vec![],
            test: test_root.map(crate::graphics_verify::Case::new),
            frames: 0,
            stopped: false,
        }
    }
    fn fail(&mut self, event_loop: &ActiveEventLoop, e: String) {
        self.stopped = true;
        self.error = Some(e);
        if let Some(app) = &mut self.app {
            prepare_failure(app);
        }
        event_loop.exit();
    }
    fn redraw(&mut self, event_loop: &ActiveEventLoop) -> Result<(), String> {
        if self.stopped {
            return Ok(());
        }
        let window = self
            .window
            .as_ref()
            .ok_or("Software window unavailable")?
            .clone();
        let size = window.inner_size();
        if size.width == 0 || size.height == 0 {
            return Ok(());
        }
        *self.next.lock().map_err(|e| e.to_string())? = None;
        let state = self.state.as_mut().ok_or("Software input unavailable")?;
        let mut raw = state.take_egui_input(&window);
        if self.close {
            raw.viewports
                .entry(egui::ViewportId::ROOT)
                .or_default()
                .events
                .push(egui::ViewportEvent::Close);
        }
        raw.events.append(&mut self.pending);
        let app = self.app.as_mut().ok_or("Software editor unavailable")?;
        if let Some(test) = &mut self.test {
            raw.events.clear();
            raw.focused = true;
            raw.viewports
                .entry(egui::ViewportId::ROOT)
                .or_default()
                .focused = Some(true);
            test.input(app, self.frames, &mut raw)?;
        }
        let mut info = raw
            .viewports
            .get(&egui::ViewportId::ROOT)
            .cloned()
            .unwrap_or_default();
        let output = self.ctx.run_ui(raw, |ui| app.run_ui(ui, None));
        state.handle_platform_output_with_event_loop(&window, event_loop, output.platform_output);
        self.painter.set(&output.textures_delta)?;
        let primitives = self.ctx.tessellate(output.shapes, output.pixels_per_point);
        let dims = [size.width as usize, size.height as usize];
        let background = self.ctx.style_of(self.ctx.theme()).visuals.panel_fill;
        self.painter
            .paint(dims, output.pixels_per_point, &primitives, background)?;
        window.pre_present_notify();
        drawer_platform::graphics::present(&window, dims, &self.painter.pixels)?;
        self.painter.free(&output.textures_delta);
        let mut close = self.close;
        self.close = false;
        let mut actions = vec![];
        for (id, viewport) in output.viewport_output {
            if id != egui::ViewportId::ROOT {
                continue;
            }
            let mut commands = vec![];
            for c in viewport.commands {
                match c {
                    egui::ViewportCommand::Close => close = true,
                    egui::ViewportCommand::CancelClose => close = false,
                    _ => commands.push(c),
                }
            }
            egui_winit::process_viewport_commands(
                &self.ctx,
                &mut info,
                commands,
                &window,
                &mut actions,
            );
            if viewport.repaint_delay != Duration::MAX {
                let when = Instant::now() + viewport.repaint_delay;
                let mut next = self.next.lock().map_err(|e| e.to_string())?;
                if next.is_none_or(|n| when < n) {
                    *next = Some(when);
                }
            }
        }
        for action in actions {
            match action {
                egui_winit::ActionRequested::Screenshot(user_data) => {
                    let image = egui::ColorImage::new(
                        dims,
                        self.painter
                            .pixels
                            .iter()
                            .map(|p| {
                                egui::Color32::from_rgb((p >> 16) as u8, (p >> 8) as u8, *p as u8)
                            })
                            .collect(),
                    );
                    self.pending.push(egui::Event::Screenshot {
                        viewport_id: egui::ViewportId::ROOT,
                        user_data,
                        image: Arc::new(image),
                    });
                }
                egui_winit::ActionRequested::Cut => self.pending.push(egui::Event::Cut),
                egui_winit::ActionRequested::Copy => self.pending.push(egui::Event::Copy),
                egui_winit::ActionRequested::Paste => {
                    if let Some(text) = state.clipboard_text() {
                        self.pending.push(egui::Event::Paste(text));
                    }
                }
            }
            self.dirty = true;
        }
        if let Some(test) = &mut self.test {
            let bridge = self.bridge.borrow();
            if test.completed(
                app,
                self.frames,
                &self.painter.pixels,
                bridge.gpu.clone(),
                bridge.gpu_color,
                bridge.reason.clone(),
            )? {
                app.quit_ready = true;
                close = true;
            } else {
                self.dirty = true;
            }
        }
        self.frames += 1;
        if close || app.quit_ready {
            self.stopped = true;
            eframe::App::on_exit(app.as_mut(), None);
            event_loop.exit();
        }
        Ok(())
    }
    fn finish(&mut self) -> Result<(), String> {
        self.input.detach();
        if let Some(e) = self.error.take() {
            return Err(e);
        }
        if self.test.as_ref().is_some_and(|t| !t.finished) {
            return Err("Software verification did not finish".into());
        }
        Ok(())
    }
}
impl ApplicationHandler<eframe::UserEvent> for Software {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.stopped {
            return;
        }
        let result = (|| {
            let attrs = Window::default_attributes()
                .with_title(format!("{APP_NAME} — software display"))
                .with_inner_size(winit::dpi::PhysicalSize::new(1100, 760))
                .with_visible(self.test.is_none());
            let window = Arc::new(event_loop.create_window(attrs).map_err(|e| e.to_string())?);
            self.ctx.set_embed_viewports(true);
            let old = self.bridge.borrow_mut().app.take();
            let mut app = if let Some(old) = old {
                transfer(old, &self.ctx)?
            } else {
                self.factory.borrow_mut()(&self.ctx)?
            };
            app.graphics.running_cpu = true;
            app.graphics.settings.software = true;
            let bridge = self.bridge.borrow();
            if let Some(reason) = &bridge.reason {
                app.graphics.settings.last_failure = Some(reason.chars().take(240).collect());
                app.status=Some("Display recovered using local software rendering. Open documents and Undo are retained; unconfirmed previews were cancelled.".into());
            }
            if let Some(gpu) = &bridge.gpu {
                app.graphics.settings.last_gpu = Some(gpu.clone());
            }
            if let Some(storage) = &app.storage
                && let Err(e) = app.graphics.settings.save(storage)
            {
                app.status = Some(e);
            }
            attach(&mut app, &self.input, &window);
            if self.test.is_some() {
                app.native_mouse = None;
                app.drawing.cancel_check = None;
            }
            self.ctx.set_zoom_factor(app.interface.settings.zoom);
            let weak = Arc::downgrade(&window);
            let next = self.next.clone();
            self.ctx.set_request_repaint_callback(move |r| {
                let Some(when) = Instant::now().checked_add(r.delay) else {
                    return;
                };
                if let Ok(mut n) = next.lock()
                    && n.is_none_or(|old| when < old)
                {
                    *n = Some(when);
                }
                if let Some(window) = weak.upgrade() {
                    window.request_redraw();
                }
            });
            self.state = Some(egui_winit::State::new(
                self.ctx.clone(),
                egui::ViewportId::ROOT,
                window.as_ref(),
                Some(window.scale_factor() as f32),
                window.theme(),
                Some(8192),
            ));
            self.window = Some(window);
            self.app = Some(app);
            self.dirty = true;
            Ok(())
        })();
        if let Err(e) = result {
            self.fail(event_loop, e);
        }
    }
    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        if self.stopped {
            return;
        }
        let Some(window) = &self.window else {
            return;
        };
        if window.id() != id {
            return;
        }
        if matches!(event, WindowEvent::CloseRequested) {
            self.close = true;
            window.set_minimized(false);
        }
        if let Some(state) = &mut self.state {
            let response = state.on_window_event(window, &event);
            self.dirty |= response.repaint;
        }
        if matches!(event, WindowEvent::RedrawRequested) {
            let due = self
                .next
                .lock()
                .ok()
                .and_then(|n| *n)
                .is_some_and(|n| n <= Instant::now());
            if self.dirty || due {
                self.dirty = false;
                if let Err(e) = self.redraw(event_loop) {
                    self.fail(event_loop, e);
                }
            }
        }
    }
    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if self.stopped {
            return;
        }
        let next = self.next.lock().ok().and_then(|n| *n);
        if self.dirty || next.is_some_and(|n| n <= Instant::now()) {
            self.dirty = false;
            if let Err(e) = self.redraw(event_loop) {
                self.fail(event_loop, e);
                return;
            }
        }
        if self.test.is_some() && self.dirty {
            event_loop.set_control_flow(ControlFlow::Poll);
        } else {
            let next = self.next.lock().ok().and_then(|n| *n);
            event_loop.set_control_flow(next.map_or(ControlFlow::Wait, ControlFlow::WaitUntil));
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn new_ui_context_retains_document_undo_redo_and_budget_policy() {
        let ctx = egui::Context::default();
        let mut old = Drawer::new(&ctx, None);
        old.create_canvas(CanvasSpec {
            width: 2,
            height: 1,
            dpi: 144.,
            background: Background::Transparent,
        })
        .unwrap();
        old.resources.current.history.steps = 8;
        old.documents[0]
            .configure_history(old.resources.current.history)
            .unwrap();
        let base = old.documents[0].snapshot();
        assert!(old.documents[0].edit_pixel(0, 0, 0, [10, 20, 30, 255]));
        old.documents[0].history_barrier();
        let first = old.documents[0].snapshot();
        assert!(old.documents[0].edit_pixel(0, 1, 0, [70, 80, 90, 255]));
        old.documents[0].history_barrier();
        let second = old.documents[0].snapshot();
        assert!(old.documents[0].undo());
        assert_eq!(old.documents[0].history_counts(), (1, 1));
        old.duplicate_view();
        old.select_tab(0);
        let active_tab = old.active_tab;
        let next_tab_id = old.next_tab_id;
        let view_session = old.view_session;
        let new_ctx = egui::Context::default();
        let mut next = transfer(Box::new(old), &new_ctx).unwrap();
        assert_eq!(next.active_tab, active_tab);
        assert_eq!(next.view_session, view_session);
        next.duplicate_view();
        assert_eq!(next.tabs.last().unwrap().id, next_tab_id + 1);
        let ids = next
            .tabs
            .iter()
            .map(|t| t.id)
            .collect::<std::collections::HashSet<_>>();
        assert_eq!(ids.len(), next.tabs.len());
        crate::input_replay::frame(&new_ctx, &mut next, 0., vec![]);
        assert_eq!(next.documents[0].snapshot(), first);
        assert_eq!(next.documents[0].history_counts(), (1, 1));
        assert_eq!(next.resources.current.history.steps, 8);
        assert!(next.graphics.running_cpu);
        assert!(next.documents[0].redo());
        assert_eq!(next.documents[0].snapshot(), second);
        assert!(next.documents[0].undo());
        assert!(next.documents[0].undo());
        assert_eq!(next.documents[0].snapshot(), base);
    }
}
