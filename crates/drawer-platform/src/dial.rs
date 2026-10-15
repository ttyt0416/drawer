//! OS RadialController, restricted to the retained root window. No HID guessing.
use drawer_core::controller::{Event, Kind};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::{
    collections::VecDeque,
    marker::PhantomData,
    rc::Rc,
    sync::{Arc, Mutex},
};
use windows::{
    Foundation::TypedEventHandler,
    UI::Input::{
        RadialController, RadialControllerMenuItem, RadialControllerMenuKnownIcon,
        RadialControllerRotationChangedEventArgs,
    },
    Win32::{
        Foundation::HWND,
        System::WinRT::{RO_INIT_SINGLETHREADED, RoInitialize, RoUninitialize},
        UI::{Input::Radial::IRadialControllerInterop, WindowsAndMessaging::GetForegroundWindow},
    },
    core::HSTRING,
};
use winit::window::Window;

struct Runtime;
impl Runtime {
    fn new() -> windows::core::Result<Self> {
        // SAFETY: root UI thread; successful S_OK/S_FALSE balanced after objects.
        unsafe { RoInitialize(RO_INIT_SINGLETHREADED) }?;
        Ok(Self)
    }
}
impl Drop for Runtime {
    fn drop(&mut self) {
        // SAFETY: Dial is !Send and drops this guard on the initializing UI thread.
        unsafe { RoUninitialize() };
    }
}
#[derive(Default)]
struct Queue {
    events: VecDeque<Event>,
    serial: u64,
    alive: bool,
    acquired: bool,
    focus: Option<bool>,
    error: Option<String>,
}
impl Queue {
    fn push(&mut self, kind: Kind) {
        if !self.alive || self.error.is_some() {
            return;
        }
        match kind {
            Kind::Acquired => self.acquired = true,
            Kind::Lost => self.acquired = false,
            _ => {}
        }
        self.emit(kind);
    }
    fn emit(&mut self, kind: Kind) {
        if !self.alive || self.error.is_some() {
            return;
        }
        let Some(serial) = self.serial.checked_add(1) else {
            self.fail("Dial sequence limit; retry adapter".into());
            return;
        };
        if self.events.len() >= 256 {
            self.fail("Dial queue overflow; retry adapter".into());
            return;
        }
        self.serial = serial;
        self.events.push_back(Event { serial, kind });
    }
    fn fail(&mut self, error: String) {
        self.events.clear();
        self.acquired = false;
        self.error = Some(error);
    }
    fn focus(&mut self, focused: bool) {
        if self.focus != Some(focused) {
            self.focus = Some(focused);
            self.events.clear();
            self.emit(if self.acquired && focused {
                Kind::Acquired
            } else {
                Kind::Lost
            });
        }
    }
}
struct Native {
    controller: RadialController,
    item: RadialControllerMenuItem,
    tokens: [Option<i64>; 4],
}
impl Drop for Native {
    fn drop(&mut self) {
        if let Some(t) = self.tokens[0] {
            let _ = self.controller.RemoveRotationChanged(t);
        }
        if let Some(t) = self.tokens[1] {
            let _ = self.controller.RemoveButtonClicked(t);
        }
        if let Some(t) = self.tokens[2] {
            let _ = self.controller.RemoveControlAcquired(t);
        }
        if let Some(t) = self.tokens[3] {
            let _ = self.controller.RemoveControlLost(t);
        }
        if let Ok(items) = self.controller.Menu().and_then(|m| m.Items()) {
            let mut index = 0;
            if items.IndexOf(&self.item, &mut index) == Ok(true) {
                let _ = items.RemoveAt(index);
            }
        }
    }
}
/// Thread-bound owner: callbacks contain only a bounded Mutex queue and Weak window.
pub struct Dial {
    native: Option<Native>,
    queue: Arc<Mutex<Queue>>,
    report: String,
    runtime: Option<Runtime>,
    _thread: PhantomData<Rc<()>>,
    owner: Arc<Window>,
}
impl Dial {
    pub fn new(window: &Arc<Window>) -> Self {
        let mut result = Self {
            native: None,
            queue: Arc::new(Mutex::new(Queue {
                alive: true,
                ..Default::default()
            })),
            report: String::new(),
            runtime: None,
            _thread: PhantomData,
            owner: window.clone(),
        };
        result.report = match result.open() {
            Ok(supported) => format!(
                "Windows dial adapter attached; OS reports connected wheel: {supported}. Physical device validation pending."
            ),
            Err(e) => {
                if let Ok(mut q) = result.queue.lock() {
                    q.alive = false;
                }
                result.native.take();
                result.runtime.take();
                format!(
                    "Windows dial adapter unavailable: {e}. Keyboard commands remain available."
                )
            }
        };
        result
    }
    fn open(&mut self) -> Result<String, String> {
        let runtime = Runtime::new().map_err(|e| e.to_string())?;
        // Retain immediately so partial subscriptions are dropped before COM.
        self.runtime = Some(runtime);
        let RawWindowHandle::Win32(handle) = self
            .owner
            .window_handle()
            .map_err(|e| e.to_string())?
            .as_raw()
        else {
            return Err("Expected own Win32 root window".into());
        };
        let hwnd = handle.hwnd.get();
        let factory = windows::core::factory::<RadialController, IRadialControllerInterop>()
            .map_err(|e| e.to_string())?;
        // SAFETY: typed OS factory, live own HWND retained until after controller.
        let controller: RadialController =
            unsafe { factory.CreateForWindow(HWND(hwnd as *mut _)) }.map_err(|e| e.to_string())?;
        controller
            .SetRotationResolutionInDegrees(1.0)
            .map_err(|e| e.to_string())?;
        let item = RadialControllerMenuItem::CreateFromKnownIcon(
            &HSTRING::from("Drawer"),
            RadialControllerMenuKnownIcon::InkThickness,
        )
        .map_err(|e| e.to_string())?;
        controller
            .Menu()
            .and_then(|m| m.Items())
            .and_then(|items| items.Append(&item))
            .map_err(|e| e.to_string())?;
        self.native = Some(Native {
            controller: controller.clone(),
            item: item.clone(),
            tokens: [None; 4],
        });
        let owner = Arc::downgrade(&self.owner);
        let send = move |q: Arc<Mutex<Queue>>| {
            let window = owner.clone();
            move |event: Result<Kind, String>| {
                if let Ok(mut state) = q.lock() {
                    match event {
                        Ok(kind) => {
                            // SAFETY: read-only OS foreground query, HWND used as value.
                            let foreground = unsafe { GetForegroundWindow() }.0 as isize == hwnd;
                            if matches!(kind, Kind::Acquired | Kind::Lost) || foreground {
                                state.push(kind);
                            } else {
                                state.push(Kind::Lost);
                            }
                        }
                        Err(e) => state.fail(e),
                    }
                }
                if let Some(window) = window.upgrade() {
                    window.request_redraw();
                }
            }
        };
        let rotation = send(self.queue.clone());
        let token = controller
            .RotationChanged(&TypedEventHandler::new(
                move |_, args: windows::core::Ref<RadialControllerRotationChangedEventArgs>| {
                    rotation(
                        args.as_ref()
                            .ok_or_else(|| "Missing dial rotation arguments".to_string())
                            .and_then(|a| {
                                a.RotationDeltaInDegrees()
                                    .map(Kind::Rotation)
                                    .map_err(|e| e.to_string())
                            }),
                    );
                    Ok(())
                },
            ))
            .map_err(|e| e.to_string())?;
        self.native.as_mut().unwrap().tokens[0] = Some(token);
        let click = send(self.queue.clone());
        let token = controller
            .ButtonClicked(&TypedEventHandler::new(move |_, _| {
                click(Ok(Kind::Click));
                Ok(())
            }))
            .map_err(|e| e.to_string())?;
        self.native.as_mut().unwrap().tokens[1] = Some(token);
        let acquired = send(self.queue.clone());
        let token = controller
            .ControlAcquired(&TypedEventHandler::new(move |_, _| {
                acquired(Ok(Kind::Acquired));
                Ok(())
            }))
            .map_err(|e| e.to_string())?;
        self.native.as_mut().unwrap().tokens[2] = Some(token);
        let lost = send(self.queue.clone());
        let token = controller
            .ControlLost(&TypedEventHandler::new(move |_, _| {
                lost(Ok(Kind::Lost));
                Ok(())
            }))
            .map_err(|e| e.to_string())?;
        self.native.as_mut().unwrap().tokens[3] = Some(token);
        controller
            .Menu()
            .and_then(|m| m.SelectMenuItem(&item))
            .map_err(|e| e.to_string())?;
        Ok(match RadialController::IsSupported() {
            Ok(supported) => supported.to_string(),
            Err(e) => format!("query unavailable ({e})"),
        })
    }
    pub fn report(&self) -> &str {
        &self.report
    }
    pub fn attached(&self) -> bool {
        self.native.is_some() && self.queue.lock().is_ok_and(|q| q.error.is_none())
    }
    pub fn drain(&mut self, focused: bool) -> Result<Vec<Event>, String> {
        let mut q = self.queue.lock().map_err(|_| "Dial queue unavailable")?;
        if let Some(error) = &q.error {
            self.report = format!("{error}; retry adapter");
            return Err(error.clone());
        }
        q.focus(focused);
        Ok(q.events.drain(..).collect())
    }
    pub fn retry(&mut self) {
        if let Ok(mut q) = self.queue.lock() {
            q.alive = false;
        }
        self.native.take();
        self.runtime.take();
        let replacement = Self::new(&self.owner);
        *self = replacement;
    }
}
impl Drop for Dial {
    fn drop(&mut self) {
        if let Ok(mut q) = self.queue.lock() {
            q.alive = false;
            q.events.clear();
        }
        // Callbacks that race removal can only see the dead queue and Weak window.
        self.native.take();
        self.runtime.take();
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn queue_is_bounded_latches_fault_and_clears_on_focus_and_teardown() {
        let mut q = Queue {
            alive: true,
            ..Default::default()
        };
        q.push(Kind::Acquired);
        q.push(Kind::Rotation(1.0));
        q.focus(true);
        assert_eq!(q.events.len(), 1);
        assert!(matches!(q.events[0].kind, Kind::Acquired));
        q.push(Kind::Click);
        q.focus(false);
        assert_eq!(q.events.len(), 1);
        assert!(matches!(q.events[0].kind, Kind::Lost));
        q.focus(true);
        assert!(matches!(q.events[0].kind, Kind::Acquired));
        for _ in 0..256 {
            q.push(Kind::Click);
        }
        assert!(q.error.is_some());
        assert!(q.events.is_empty());
        assert!(!q.acquired);
        q.push(Kind::Acquired);
        assert!(q.events.is_empty());
        let mut q = Queue {
            alive: false,
            ..Default::default()
        };
        q.push(Kind::Click);
        assert!(q.events.is_empty());
    }
}
