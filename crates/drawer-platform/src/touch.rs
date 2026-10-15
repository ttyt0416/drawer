//! Root-window-only subclass reads WM_TOUCH before winit closes its borrowed
//! touch handle. Every message still reaches the original window procedure.
use super::*;
use std::{cell::RefCell, rc::Weak};
use windows::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, WPARAM},
    UI::{
        Input::Touch::{GetTouchInputInfo, HTOUCHINPUT, TOUCHINPUT},
        Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass},
        WindowsAndMessaging::IsWindow,
    },
};
const ID: usize = 0x44525754;
pub(super) struct Hook {
    window: isize,
    context: Option<Box<Weak<RefCell<State>>>>,
}
impl Hook {
    pub(super) fn new(window: isize, state: Weak<RefCell<State>>) -> Result<Self, String> {
        let context = Box::new(state);
        let data = (&*context as *const Weak<RefCell<State>>) as usize;
        // SAFETY: own root HWND on its UI thread. Stable Box allocation remains
        // live until removal; callback/reference data never escape this module.
        if !unsafe { SetWindowSubclass(HWND(window as *mut _), Some(callback), ID, data) }.as_bool()
        {
            return Err("Touch window adapter could not attach".into());
        }
        Ok(Self {
            window,
            context: Some(context),
        })
    }
}
impl Drop for Hook {
    fn drop(&mut self) {
        // SAFETY: same UI-thread root and callback/ID pair used at installation.
        let removed =
            unsafe { RemoveWindowSubclass(HWND(self.window as *mut _), Some(callback), ID) }
                .as_bool();
        // SAFETY: numeric HWND validity query; no borrowed application memory.
        let alive = unsafe { IsWindow(Some(HWND(self.window as *mut _))) }.as_bool();
        if !removed
            && alive
            && let Some(context) = self.context.take()
        {
            // A provider-declined removal must never leave a dangling callback data
            // pointer. Retain this small Weak allocation until process exit instead.
            let _ = Box::leak(context);
        }
    }
}
fn cancel(s: &mut State) {
    if s.queue.len() + s.touch_contacts.len() > LIMIT {
        s.queue.clear();
        s.overflow = true;
    }
    for mut event in s.touch_contacts.drain(..) {
        event.phase = Phase::Cancel;
        s.queue.push_back(event);
    }
}
fn read(s: &mut State, wparam: usize, lparam: isize) -> Result<(), String> {
    let count = wparam & 0xffff;
    if count > 256 {
        return Err("Touch batch exceeds 256 contacts".into());
    }
    if count == 0 {
        return Ok(());
    }
    let mut values = vec![TOUCHINPUT::default(); count];
    // SAFETY: WM_TOUCH supplies a live borrowed handle during this callback;
    // initialized output capacity and ABI element size match. Do not close the
    // handle: DefSubclassProc forwards it to winit, which owns final cleanup.
    unsafe {
        GetTouchInputInfo(
            HTOUCHINPUT(lparam as *mut _),
            &mut values,
            std::mem::size_of::<TOUCHINPUT>() as i32,
        )
    }
    .map_err(|e| e.to_string())?;
    let offset = super::win::client(s.window, [0, 0])?;
    s.touch_frame = s.touch_frame.wrapping_add(1);
    for p in values {
        let Some(event) = super::packets::touch_event(
            p.hSource.0 as u64,
            p.dwID,
            p.dwFlags.0,
            [p.x, p.y],
            offset,
            s.touch_frame,
        ) else {
            continue;
        };
        if let Some(n) = s
            .touch_contacts
            .iter()
            .position(|e| e.source == event.source)
        {
            if matches!(event.phase, Phase::Up | Phase::Cancel) {
                s.touch_contacts.remove(n);
            } else {
                s.touch_contacts[n] = event;
            }
        } else if event.phase == Phase::Down {
            if s.touch_contacts.len() >= 32 {
                return Err("Too many simultaneous touch contacts".into());
            }
            s.touch_contacts.push(event);
        }
        if s.queue.len() >= LIMIT {
            s.queue.clear();
            s.overflow = true;
            cancel(s);
            return Ok(());
        }
        s.queue.push_back(event);
    }
    Ok(())
}
unsafe extern "system" fn callback(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    data: usize,
) -> LRESULT {
    // SAFETY: installation supplies exactly this stable Box pointee. Hook owns
    // it through callback removal (or retains it on removal failure). Weak avoids
    // keeping dropped state alive; all callbacks execute on the window UI thread.
    let weak = unsafe { &*(data as *const Weak<RefCell<State>>) };
    if let Some(state) = weak.upgrade()
        && let Ok(mut s) = state.try_borrow_mut()
        && s.window == hwnd.0 as isize
    {
        if message == 0x240 {
            match read(&mut s, wparam.0, lparam.0) {
                Ok(()) => s.touch_error = None,
                Err(e) => {
                    cancel(&mut s);
                    s.failure = Some(format!("Touch input failed; pending stroke cancelled: {e}"));
                    s.touch_error = Some(e);
                }
            }
        } else if matches!(message, 8 | 0x1f | 0x215) {
            cancel(&mut s);
        }
    }
    // SAFETY: original OS-provided arguments forwarded exactly once. winit's
    // original procedure retains ownership of any input handle in the message.
    unsafe { DefSubclassProc(hwnd, message, wparam, lparam) }
}
