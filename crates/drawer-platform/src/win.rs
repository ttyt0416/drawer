use super::*;
use crate::packets::{self, Axis, Contact};
use drawer_core::pointer_input::Attributes;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::{collections::HashMap, ffi::c_void, sync::Arc};
use windows::{
    Win32::{
        Foundation::{FreeLibrary, HMODULE, HWND, POINT},
        Graphics::Gdi::ScreenToClient,
        System::LibraryLoader::{GetProcAddress, LOAD_LIBRARY_SEARCH_SYSTEM32, LoadLibraryExW},
        UI::{
            Input::Pointer::{POINTER_INFO, POINTER_PEN_INFO},
            WindowsAndMessaging::{
                GetMessageExtraInfo, GetSystemMetrics, MSG, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN,
                SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN,
            },
        },
    },
    core::{PCSTR, PCWSTR},
};
use winit::{event_loop::EventLoopBuilder, platform::windows::EventLoopBuilderExtWindows};

struct Module(HMODULE);
impl Module {
    fn load(name: &str) -> Result<Self, String> {
        let wide: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
        // SAFETY: nul-terminated owned UTF-16 lives through the call. Only the OS
        // system directory is searched; no current/project directory DLL loading.
        unsafe { LoadLibraryExW(PCWSTR(wide.as_ptr()), None, LOAD_LIBRARY_SEARCH_SYSTEM32) }
            .map(Self)
            .map_err(|e| format!("{name} unavailable: {e}"))
    }
    fn symbol(&self, name: &'static [u8]) -> Result<unsafe extern "system" fn() -> isize, String> {
        assert_eq!(name.last(), Some(&0));
        // SAFETY: module is live and symbol name is static and nul-terminated.
        unsafe { GetProcAddress(self.0, PCSTR(name.as_ptr())) }.ok_or_else(|| {
            format!(
                "Missing export {}",
                String::from_utf8_lossy(&name[..name.len() - 1])
            )
        })
    }
}
impl Drop for Module {
    fn drop(&mut self) {
        // SAFETY: this is the one reference returned by our successful load.
        let _ = unsafe { FreeLibrary(self.0) };
    }
}
fn client(window: isize, point: [i32; 2]) -> Result<[f64; 2], String> {
    let mut p = POINT {
        x: point[0],
        y: point[1],
    };
    // SAFETY: State retains the owning window Arc; POINT is writable and local.
    if !unsafe { ScreenToClient(HWND(window as *mut c_void), &mut p) }.as_bool() {
        return Err("Pen coordinate conversion failed".into());
    }
    Ok([p.x as f64, p.y as f64])
}
type PointerInfo = unsafe extern "system" fn(u32, *mut POINTER_INFO) -> i32;
type PenInfo = unsafe extern "system" fn(u32, *mut POINTER_PEN_INFO) -> i32;
type PenHistory = unsafe extern "system" fn(u32, *mut u32, *mut POINTER_PEN_INFO) -> i32;
pub(super) struct Pointer {
    info: PointerInfo,
    pen: PenInfo,
    history: PenHistory,
    contacts: HashMap<u32, Event>,
    _module: Module,
}
impl Pointer {
    fn open() -> Result<Self, String> {
        let m = Module::load("user32.dll")?;
        // SAFETY: public User32 exports have these exact documented WINAPI ABIs.
        // Function pointers never outlive the retained module.
        let (info, pen, history) = unsafe {
            (
                std::mem::transmute::<unsafe extern "system" fn() -> isize, PointerInfo>(
                    m.symbol(b"GetPointerInfo\0")?,
                ),
                std::mem::transmute::<unsafe extern "system" fn() -> isize, PenInfo>(
                    m.symbol(b"GetPointerPenInfo\0")?,
                ),
                std::mem::transmute::<unsafe extern "system" fn() -> isize, PenHistory>(
                    m.symbol(b"GetPointerPenInfoHistory\0")?,
                ),
            )
        };
        Ok(Self {
            info,
            pen,
            history,
            contacts: HashMap::new(),
            _module: m,
        })
    }
    fn reset(&mut self) -> Vec<Event> {
        self.contacts
            .drain()
            .map(|(_, mut e)| {
                e.phase = Phase::Cancel;
                e
            })
            .collect()
    }
    fn message(
        &mut self,
        window: isize,
        message: u32,
        wparam: usize,
    ) -> Result<Vec<Event>, String> {
        if !matches!(message, 0x245 | 0x246 | 0x247 | 0x24a | 0x24c) {
            return Ok(vec![]);
        }
        let id = (wparam & 0xffff) as u32;
        if matches!(message, 0x24a | 0x24c) {
            return Ok(self
                .contacts
                .remove(&id)
                .map(|mut e| {
                    e.phase = Phase::Cancel;
                    e
                })
                .into_iter()
                .collect());
        }
        let mut info = POINTER_INFO::default();
        // SAFETY: correctly sized initialized output, synchronous OS query.
        if unsafe { (self.info)(id, &mut info) } == 0 {
            return if self.contacts.contains_key(&id) {
                Err("Windows Pointer lost the active pen".into())
            } else {
                Ok(vec![])
            };
        }
        if info.pointerType.0 != 3 {
            return Ok(vec![]);
        } // PT_PEN, never infer from force.
        let mut current = POINTER_PEN_INFO::default();
        // SAFETY: output matches the documented API layout from windows-rs.
        if unsafe { (self.pen)(id, &mut current) } == 0 {
            return Err("GetPointerPenInfo failed".into());
        }
        let mut count = 0;
        // SAFETY: a null buffer with count=0 queries required element count.
        let _ = unsafe { (self.history)(id, &mut count, std::ptr::null_mut()) };
        if count > 4096 {
            return Err("Windows Pointer history exceeds input limit".into());
        }
        let mut history = vec![POINTER_PEN_INFO::default(); count as usize];
        if count > 0 {
            // SAFETY: buffer contains count elements, API can write at most count.
            if unsafe { (self.history)(id, &mut count, history.as_mut_ptr()) } == 0
                || count as usize > history.len()
            {
                return Err("Windows Pointer history query failed".into());
            }
            history.truncate(count as usize);
            history.reverse(); // Microsoft documents newest first.
        }
        // History already includes current. Use current only when no history exists.
        if history.is_empty() {
            history.push(current);
        }
        let mut out = Vec::new();
        for p in history {
            if p.pointerInfo.pointerId != id || p.pointerInfo.hwndTarget.0 as isize != window {
                continue;
            }
            let flags = p.pointerInfo.pointerFlags.0;
            let active = self.contacts.contains_key(&id);
            let phase = if flags & (32768 | 2097152) != 0 {
                Phase::Cancel
            } else if flags & 262144 != 0 {
                Phase::Up
            } else if flags & 65536 != 0 {
                Phase::Down
            } else if flags & 4 != 0 {
                Phase::Move
            } else {
                Phase::Cancel
            };
            if phase != Phase::Down && !active {
                continue;
            }
            if phase == Phase::Down
                && active
                && let Some(mut old) = self.contacts.remove(&id)
            {
                old.phase = Phase::Cancel;
                out.push(old);
            }
            if self.contacts.len() >= 32 && phase == Phase::Down {
                return Err("Too many active pen contacts".into());
            }
            let mask = p.penMask;
            let event = Event {
                source: Source {
                    kind: Kind::Pen,
                    device: p.pointerInfo.sourceDevice.0 as u64,
                    contact: id as u64,
                },
                phase,
                position: client(
                    window,
                    [
                        p.pointerInfo.ptPixelLocation.x,
                        p.pointerInfo.ptPixelLocation.y,
                    ],
                )?,
                pressure: (mask & 1 != 0).then_some((p.pressure as f64 / 1024.0).clamp(0.0, 1.0)),
                attributes: Attributes {
                    tilt: (mask & 12 == 12)
                        .then_some([(p.tiltX as f64).to_radians(), (p.tiltY as f64).to_radians()]),
                    eraser: Some(p.penFlags & 6 != 0),
                    button: Some(p.penFlags & 1 != 0),
                }
                .normalized(),
                origin: Origin::Unmarked,
            };
            match phase {
                Phase::Down | Phase::Move => {
                    self.contacts.insert(id, event);
                }
                _ => {
                    self.contacts.remove(&id);
                }
            }
            out.push(event);
        }
        Ok(out)
    }
}

type Info = unsafe extern "system" fn(u32, u32, *mut c_void) -> u32;
type Open = unsafe extern "system" fn(HWND, *mut Context, i32) -> *mut c_void;
type Close = unsafe extern "system" fn(*mut c_void) -> i32;
type Enable = unsafe extern "system" fn(*mut c_void, i32) -> i32;
type ReadPacket = unsafe extern "system" fn(*mut c_void, u32, *mut c_void) -> i32;
const MSG_BASE: u32 = 0x8400;
#[derive(Clone, Copy)]
#[repr(C)]
struct Context {
    name: [u16; 40],
    options: u32,
    status: u32,
    locks: u32,
    msg_base: u32,
    device: u32,
    pkt_rate: u32,
    pkt_data: u32,
    pkt_mode: u32,
    move_mask: u32,
    btn_down_mask: u32,
    btn_up_mask: u32,
    in_org: [i32; 3],
    in_ext: [i32; 3],
    out_org: [i32; 3],
    out_ext: [i32; 3],
    sensitivity: [u32; 3],
    sys_mode: i32,
    sys_org: [i32; 2],
    sys_ext: [i32; 2],
    sys_sensitivity: [u32; 2],
}
impl Default for Context {
    fn default() -> Self {
        Self {
            name: [0; 40],
            options: 0,
            status: 0,
            locks: 0,
            msg_base: 0,
            device: 0,
            pkt_rate: 0,
            pkt_data: 0,
            pkt_mode: 0,
            move_mask: 0,
            btn_down_mask: 0,
            btn_up_mask: 0,
            in_org: [0; 3],
            in_ext: [0; 3],
            out_org: [0; 3],
            out_ext: [0; 3],
            sensitivity: [0; 3],
            sys_mode: 0,
            sys_org: [0; 2],
            sys_ext: [0; 2],
            sys_sensitivity: [0; 2],
        }
    }
}
fn info<T: Default>(call: Info, category: u32, index: u32) -> Option<T> {
    // SAFETY: null output is the documented size query, with no memory access.
    if unsafe { call(category, index, std::ptr::null_mut()) } as usize != std::mem::size_of::<T>() {
        return None;
    }
    let mut out = T::default();
    // SAFETY: queried exact output size equals the initialized local ABI type.
    // All callers use integer-only repr(C) data, never references or Rust enums.
    let wrote = unsafe { call(category, index, (&mut out as *mut T).cast()) };
    (wrote as usize == std::mem::size_of::<T>()).then_some(out)
}
pub(super) struct Wintab {
    context: *mut c_void,
    close: Close,
    enable: Enable,
    packet: ReadPacket,
    info: Info,
    mask: u32,
    device: u32,
    default_cursor: u32,
    pressure: Option<Axis>,
    orientation: Option<[Axis; 3]>,
    contact: Contact,
    focused: bool,
    _module: Module,
}
impl Wintab {
    fn open(window: isize) -> Result<Self, String> {
        let m = Module::load("Wintab32.dll")?;
        // SAFETY: Wintab's public Unicode exports use the documented WINAPI ABIs.
        // All outputs below use the public integer-only structures; module retained.
        let (query, open, close, enable, packet) = unsafe {
            (
                std::mem::transmute::<unsafe extern "system" fn() -> isize, Info>(
                    m.symbol(b"WTInfoW\0")?,
                ),
                std::mem::transmute::<unsafe extern "system" fn() -> isize, Open>(
                    m.symbol(b"WTOpenW\0")?,
                ),
                std::mem::transmute::<unsafe extern "system" fn() -> isize, Close>(
                    m.symbol(b"WTClose\0")?,
                ),
                std::mem::transmute::<unsafe extern "system" fn() -> isize, Enable>(
                    m.symbol(b"WTEnable\0")?,
                ),
                std::mem::transmute::<unsafe extern "system" fn() -> isize, ReadPacket>(
                    m.symbol(b"WTPacket\0")?,
                ),
            )
        };
        let mut c = info::<Context>(query, 3, 0).ok_or("No default Wintab tablet context")?;
        let cat = packets::device_category(c.device).ok_or("Invalid Wintab device")?;
        let supported =
            info::<u32>(query, cat, 6).unwrap_or(0) | info::<u32>(query, cat, 8).unwrap_or(0);
        c.pkt_data = supported & packets::SUPPORTED;
        packets::words(c.pkt_data)?;
        c.pkt_mode = 0;
        c.move_mask = c.pkt_data;
        c.btn_down_mask = u32::MAX;
        c.btn_up_mask = u32::MAX;
        c.options = (c.options & !1) | 4; // messages, no system cursor takeover
        c.msg_base = MSG_BASE;
        // SAFETY: these parameterless metric queries read current virtual desktop.
        let (x, y, w, h) = unsafe {
            (
                GetSystemMetrics(SM_XVIRTUALSCREEN),
                GetSystemMetrics(SM_YVIRTUALSCREEN),
                GetSystemMetrics(SM_CXVIRTUALSCREEN),
                GetSystemMetrics(SM_CYVIRTUALSCREEN),
            )
        };
        if w <= 0 || h <= 0 {
            return Err("Invalid virtual desktop dimensions".into());
        }
        c.out_org[0] = x;
        c.out_org[1] = y.checked_add(h).ok_or("Desktop coordinate overflow")?;
        c.out_ext[0] = w;
        c.out_ext[1] = -h;
        c.sys_mode = 0;
        // SAFETY: live owned root HWND and writable complete LOGCONTEXTW. Open
        // disabled; focus must be explicitly granted before accepting any packets.
        let context = unsafe { open(HWND(window as *mut c_void), &mut c, 0) };
        if context.is_null() {
            return Err("WTOpenW declined initialization".into());
        }
        let driver = Self {
            context,
            close,
            enable,
            packet,
            info: query,
            mask: c.pkt_data,
            device: c.device,
            default_cursor: info::<u32>(query, cat, 4).unwrap_or(0),
            pressure: info::<Axis>(query, cat, 15),
            orientation: info::<[Axis; 3]>(query, cat, 17),
            contact: Contact::default(),
            focused: false,
            _module: m,
        };
        packets::words(c.pkt_data)?; // Drop closes context even when validation fails.
        if c.pkt_mode & c.pkt_data != 0
            || c.msg_base != MSG_BASE
            || packets::device_category(c.device) != Some(cat)
        {
            return Err("Wintab changed absolute packet contract".into());
        }
        Ok(driver)
    }
    fn focus(&mut self, focused: bool) -> Result<Vec<Event>, String> {
        if self.focused == focused {
            return Ok(vec![]);
        }
        let cancelled = self.contact.reset();
        // SAFETY: our own live context; callback is on the same UI thread.
        if unsafe { (self.enable)(self.context, i32::from(focused)) } == 0 {
            return Err("WTEnable failed".into());
        }
        self.focused = focused;
        Ok(cancelled)
    }
    fn message(
        &mut self,
        window: isize,
        message: u32,
        wparam: usize,
        lparam: isize,
    ) -> Result<Vec<Event>, String> {
        if message == MSG_BASE + 2 && wparam == self.context as usize {
            return Err("Wintab context closed by driver".into());
        }
        if message == MSG_BASE + 5 {
            // WT_PROXIMITY: hCtx in wParam, low word entering.
            if wparam == self.context as usize && lparam & 0xffff == 0 {
                return Ok(self.contact.reset());
            }
            return Ok(vec![]);
        }
        if message != MSG_BASE || lparam != self.context as isize || !self.focused {
            return Ok(vec![]);
        }
        let count = packets::words(self.mask)?;
        let mut buffer = [0u32; 9];
        // SAFETY: negotiated supported layout fits 9 u32 words, initialized and
        // aligned. WTPacket reads this message's serial only, preserving dispatch
        // order with mouse/Pointer messages instead of draining future packets.
        if unsafe { (self.packet)(self.context, wparam as u32, buffer.as_mut_ptr().cast()) } == 0 {
            return Err("Wintab packet missing; contact cancelled".into());
        }
        let mut p = packets::decode(self.mask, &buffer[..count])?;
        if self.mask & packets::CURSOR == 0 {
            p.cursor = self.default_cursor;
        }
        if p.status.is_some_and(|s| s & 2 != 0) {
            return Err("Wintab driver queue overflow".into());
        }
        if p.status.is_some_and(|s| s & 1 != 0 && s & 8 == 0) {
            return Ok(self.contact.reset());
        }
        let pressure = p
            .pressure
            .and_then(|v| self.pressure.and_then(|a| a.pressure(v)));
        let cursor_cat = 200u32
            .checked_add(p.cursor)
            .ok_or("Invalid Wintab cursor")?;
        let physical = info::<u8>(self.info, cursor_cat, 9);
        let map = info::<[u8; 32]>(self.info, cursor_cat, 7);
        let tip = physical
            .zip(map)
            .and_then(|(id, map)| map.get(id as usize).copied())
            .filter(|v| *v < 32);
        let down = if let Some(button) = tip {
            p.buttons & (1u32 << button) != 0
        } else if let Some(value) = pressure {
            value > 0.0
        } else {
            return Err("Wintab cursor reports neither tip button nor pressure".into());
        };
        Ok(self.contact.sample(
            self.device as u64,
            p.cursor,
            down,
            client(window, p.point)?,
            pressure,
            packets::attributes(&p, self.orientation),
        ))
    }
}
impl Drop for Wintab {
    fn drop(&mut self) {
        // SAFETY: context was opened by this object, is closed exactly once while
        // the window and DLL remain alive; Rc Input prevents thread migration.
        let _ = unsafe { (self.close)(self.context) };
    }
}
pub(super) enum Driver {
    Pointer(Pointer),
    Wintab(Wintab),
}
impl Driver {
    fn open(backend: Backend, window: isize) -> Result<Self, String> {
        match backend {
            Backend::Pointer => Pointer::open().map(Self::Pointer),
            Backend::Wintab => Wintab::open(window).map(Self::Wintab),
            Backend::Mouse => unreachable!(),
        }
    }
    fn focus(&mut self, focused: bool) -> Result<Vec<Event>, String> {
        match self {
            Self::Wintab(w) => w.focus(focused),
            Self::Pointer(p) => Ok(if focused { vec![] } else { p.reset() }),
        }
    }
    fn message(
        &mut self,
        window: isize,
        message: u32,
        wparam: usize,
        lparam: isize,
    ) -> Result<Vec<Event>, String> {
        match self {
            Self::Pointer(p) => p.message(window, message, wparam),
            Self::Wintab(w) => w.message(window, message, wparam, lparam),
        }
    }
}
impl Input {
    pub fn attach(&self, window: &Arc<winit::window::Window>) -> Result<(), String> {
        let RawWindowHandle::Win32(handle) =
            window.window_handle().map_err(|e| e.to_string())?.as_raw()
        else {
            return Err("Expected a Win32 window".into());
        };
        let old = {
            let mut s = self.state.borrow_mut();
            let old = s.driver.take();
            s.window = handle.hwnd.get();
            s.owner = Some(window.clone());
            s.requested = None;
            s.queue.clear();
            old
        };
        drop(old);
        Ok(())
    }
    pub fn configure(&self, preferred: Backend) {
        let (window, old) = {
            let mut s = self.state.borrow_mut();
            s.queue.clear();
            s.down = [false; 3];
            s.blocked = false;
            s.overflow = false;
            s.failure = None;
            (s.window, s.driver.take())
        };
        // Close before opening another backend and outside RefCell borrow: DLL
        // calls may synchronously dispatch window messages. Reentry is forwarded.
        drop(old);
        let (effective, driver, reason) = select(preferred, |b| Driver::open(b, window));
        let mut s = self.state.borrow_mut();
        s.requested = Some(preferred);
        s.effective = effective;
        s.driver = driver;
        s.report = format!(
            "Requested {preferred:?}; active {effective:?}. {reason} Physical pen validation pending."
        );
    }
    pub fn set_focus(&self, focused: bool) {
        let failed = {
            let mut s = self.state.borrow_mut();
            let result = s.driver.as_mut().map(|d| d.focus(focused));
            match result {
                Some(Ok(events)) => {
                    s.queue.extend(events);
                    None
                }
                Some(Err(e)) => {
                    s.failure = Some(e.clone());
                    s.report = format!("{e}; active Mouse. Retry to reconnect.");
                    s.effective = Backend::Mouse;
                    s.driver.take()
                }
                None => None,
            }
        };
        drop(failed);
    }
    pub fn install<T: 'static>(&self, builder: &mut EventLoopBuilder<T>) {
        let input = self.clone();
        builder.with_msg_hook(move |pointer| {
            if pointer.is_null() {
                return false;
            }
            // SAFETY: winit's documented callback provides a live MSG for this
            // immediate call. No reference to it or its pointees is retained.
            let m = unsafe { &*pointer.cast::<MSG>() };
            let failed = {
                let Ok(mut s) = input.state.try_borrow_mut() else {
                    return false;
                };
                let window = m.hwnd.0 as isize;
                if s.window == 0 || window != s.window {
                    return false;
                }
                // SAFETY: no caller pointers; current UI-thread message metadata.
                let extra = unsafe { GetMessageExtraInfo() }.0 as u64;
                s.record(window, m.message, m.wParam.0, m.lParam.0, extra);
                let result = s.driver.as_mut().map(|d| {
                    if m.message == 8 {
                        d.focus(false)
                    } else {
                        d.message(window, m.message, m.wParam.0, m.lParam.0)
                    }
                });
                match result {
                    Some(Ok(events)) => {
                        if s.queue.len() + events.len() > LIMIT {
                            s.queue.clear();
                            s.overflow = true;
                        } else {
                            s.queue.extend(events);
                        }
                        None
                    }
                    Some(Err(e)) => {
                        s.queue.clear();
                        s.failure = Some(e.clone());
                        s.report = format!("{e}; active Mouse. Retry to reconnect.");
                        s.effective = Backend::Mouse;
                        s.driver.take()
                    }
                    None => None,
                }
            };
            drop(failed);
            false // winit/UI always receive original messages.
        });
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn public_wintab_abi_sizes() {
        assert_eq!(std::mem::size_of::<Context>(), 212);
        assert_eq!(std::mem::size_of::<Axis>(), 16);
        assert_eq!(packets::words(packets::SUPPORTED).unwrap(), 9);
    }
}
