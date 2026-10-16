//! User-command-only display sampling. Raw OS handles stay on the owning thread.
use drawer_core::screen_sample::{Bounds, mean};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};
use windows::{
    Win32::{
        Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, POINT, WPARAM},
        Graphics::{Dwm::DwmFlush, Gdi::*},
        System::LibraryLoader::GetModuleHandleW,
        UI::{
            HiDpi::*,
            Input::KeyboardAndMouse::{SetFocus, VK_ESCAPE},
            WindowsAndMessaging::*,
        },
    },
    core::w,
};
type Outcome = Result<Option<[u8; 3]>, String>;
static ACTIVE: AtomicBool = AtomicBool::new(false);
struct Active;
impl Drop for Active {
    fn drop(&mut self) {
        ACTIVE.store(false, Ordering::Release);
    }
}
pub struct ScreenSample {
    receiver: mpsc::Receiver<Outcome>,
    cancel: Arc<AtomicBool>,
}
impl ScreenSample {
    pub fn poll(&mut self) -> Option<Outcome> {
        match self.receiver.try_recv() {
            Ok(v) => Some(v),
            Err(mpsc::TryRecvError::Empty) => None,
            Err(_) => Some(Err("Screen sampler closed before returning a result".into())),
        }
    }
}
impl Drop for ScreenSample {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}
impl super::Input {
    pub fn screen_sample(
        &self,
        radius: u8,
        hide: bool,
        wake: Arc<dyn Fn() + Send + Sync>,
    ) -> Result<ScreenSample, String> {
        if radius > 4 {
            return Err("Screen sample radius must be 0–4".into());
        }
        let s = self.state.borrow();
        let owner = s.owner.clone().ok_or("Native window is unavailable")?;
        let window = s.window;
        if ACTIVE
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err("Previous screen sampler is still restoring its window".into());
        }
        let (sender, receiver) = mpsc::sync_channel(1);
        let cancel = Arc::new(AtomicBool::new(false));
        let stop = cancel.clone();
        std::thread::Builder::new()
            .name("local-screen-color".into())
            .spawn(move || {
                let _owner = owner;
                let active = Active;
                let result = std::panic::catch_unwind(|| run(window, radius, hide, &stop))
                    .unwrap_or_else(|_| {
                        Err("Screen sampler failed; window restoration attempted".into())
                    });
                drop(active);
                let _ = sender.send(result);
                wake();
            })
            .map_err(|e| {
                ACTIVE.store(false, Ordering::Release);
                e.to_string()
            })?;
        Ok(ScreenSample { receiver, cancel })
    }
}
struct Dpi(DPI_AWARENESS_CONTEXT);
impl Dpi {
    fn physical() -> Result<Self, String> {
        // SAFETY: changes only this sampling thread, with a documented constant.
        let previous =
            unsafe { SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
        if previous.0.is_null() {
            return Err("Physical display coordinates are unavailable".into());
        }
        Ok(Self(previous))
    }
}
impl Drop for Dpi {
    fn drop(&mut self) {
        // SAFETY: restores the context returned on this same thread.
        unsafe { SetThreadDpiAwarenessContext(self.0) };
    }
}
struct Window(HWND);
impl Drop for Window {
    fn drop(&mut self) {
        // SAFETY: created on this thread, never shared, destroyed exactly once.
        let _ = unsafe { DestroyWindow(self.0) };
    }
}
struct Restore {
    window: HWND,
    placement: WINDOWPLACEMENT,
    visible: bool,
    focused: bool,
    active: bool,
}
impl Restore {
    fn begin(window: HWND, hide: bool) -> Result<Self, String> {
        let mut placement = WINDOWPLACEMENT {
            length: std::mem::size_of::<WINDOWPLACEMENT>() as u32,
            ..Default::default()
        };
        // SAFETY: retained own window; writable correctly sized output.
        unsafe { GetWindowPlacement(window, &mut placement) }.map_err(|e| e.to_string())?;
        // SAFETY: read-only queries of the retained owner and OS foreground value.
        let (visible, focused) = unsafe {
            (
                IsWindowVisible(window).as_bool(),
                GetForegroundWindow() == window,
            )
        };
        let r = Self {
            window,
            placement,
            visible,
            focused,
            active: true,
        };
        if hide && visible {
            // SAFETY: only the retained app window is hidden; r restores it on every exit.
            let _ = unsafe { ShowWindow(window, SW_HIDE) };
        }
        Ok(r)
    }
    fn finish(&mut self) -> Result<(), String> {
        if !self.active {
            return Ok(());
        }
        // SAFETY: owner lives until after restoration; placement was captured by this guard.
        let placed = unsafe { SetWindowPlacement(self.window, &self.placement) }
            .map_err(|e| format!("Window placement restore failed: {e}"));
        // SAFETY: enforce original visibility even if placement restoration failed.
        let _ = unsafe {
            ShowWindow(
                self.window,
                if !self.visible {
                    SW_HIDE
                } else if placed.is_err() {
                    SW_SHOW
                } else {
                    SHOW_WINDOW_CMD(self.placement.showCmd as i32)
                },
            )
        };
        self.active = false;
        placed?;
        if self.focused && self.visible {
            // SAFETY: restores only this app's originally focused window, never another process.
            if !unsafe { SetForegroundWindow(self.window) }.as_bool() {
                return Err("Window restored; Windows declined focus restoration".into());
            }
        }
        Ok(())
    }
}
impl Drop for Restore {
    fn drop(&mut self) {
        let _ = self.finish();
    }
}
fn guarded(window: HWND, hide: bool, operation: impl FnOnce() -> Outcome) -> Outcome {
    let mut restore = Restore::begin(window, hide)?;
    let result = operation();
    restore.finish()?;
    result
}
fn desktop() -> Result<Bounds, String> {
    // SAFETY: read-only desktop metrics under per-monitor physical DPI context.
    let b = unsafe {
        let x = GetSystemMetrics(SM_XVIRTUALSCREEN);
        let y = GetSystemMetrics(SM_YVIRTUALSCREEN);
        Bounds {
            min: [x, y],
            max: [
                x.saturating_add(GetSystemMetrics(SM_CXVIRTUALSCREEN)),
                y.saturating_add(GetSystemMetrics(SM_CYVIRTUALSCREEN)),
            ],
        }
    };
    if !b.valid() {
        return Err("Display bounds are unavailable".into());
    }
    Ok(b)
}
unsafe extern "system" fn overlay_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if message == WM_NCHITTEST {
        return LRESULT(HTCLIENT as isize);
    }
    if message == WM_SETCURSOR {
        // SAFETY: documented shared system cursor; never released by the app.
        if let Ok(cursor) = unsafe { LoadCursorW(None, IDC_CROSS) } {
            // SAFETY: borrowed shared cursor handle remains live.
            unsafe { SetCursor(Some(cursor)) };
        }
        return LRESULT(1);
    }
    // SAFETY: same-thread OS callback parameters forwarded unchanged to default handling.
    unsafe { DefWindowProcW(window, message, wparam, lparam) }
}
fn overlay_class() -> Result<HINSTANCE, String> {
    static CLASS: std::sync::OnceLock<Result<(), String>> = std::sync::OnceLock::new();
    // SAFETY: read-only own executable module query, no foreign DLL loading.
    let module = HINSTANCE(
        unsafe { GetModuleHandleW(None) }
            .map_err(|e| e.to_string())?
            .0,
    );
    CLASS
        .get_or_init(|| {
            let class = WNDCLASSW {
                lpfnWndProc: Some(overlay_proc),
                hInstance: module,
                lpszClassName: w!("DrawerLocalColorSelector"),
                ..Default::default()
            };
            // SAFETY: static class name/callback, module lives until process shutdown.
            if unsafe { RegisterClassW(&class) } == 0 {
                Err("Screen selection class registration failed".into())
            } else {
                Ok(())
            }
        })
        .clone()?;
    Ok(module)
}
fn create_overlay(b: Bounds) -> Result<Window, String> {
    let module = overlay_class()?;
    // SAFETY: own registered class/callback, no foreign parent or external pointers.
    let window = unsafe {
        CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
            w!("DrawerLocalColorSelector"),
            w!("Drawer: click to sample display RGB; Escape cancels"),
            WS_POPUP,
            b.min[0],
            b.min[1],
            b.max[0] - b.min[0],
            b.max[1] - b.min[1],
            None,
            None,
            Some(module),
            None,
        )
    }
    .map_err(|e| e.to_string())?;
    let window = Window(window);
    // SAFETY: this thread owns window. Alpha 1 receives input across the desktop.
    // It is hidden and the compositor flushed before any RGB acquisition.
    unsafe { SetLayeredWindowAttributes(window.0, COLORREF(0), 1, LWA_ALPHA) }
        .map_err(|e| e.to_string())?;
    Ok(window)
}
fn run(window: isize, radius: u8, hide: bool, cancel: &AtomicBool) -> Outcome {
    let _dpi = Dpi::physical()?;
    let bounds = desktop()?;
    guarded(HWND(window as *mut _), hide, || {
        let overlay = create_overlay(bounds)?;
        // SAFETY: own overlay, on its UI thread. No global hooks or injected input.
        unsafe {
            let _ = ShowWindow(overlay.0, SW_SHOW);
            let _ = SetForegroundWindow(overlay.0);
            let _ = SetFocus(Some(overlay.0));
        }
        // SAFETY: read-only OS foreground query; reject if overlay could not take focus.
        if unsafe { GetForegroundWindow() } != overlay.0 {
            return Err("Screen sampler could not receive input".into());
        }
        let start = Instant::now();
        let mut armed = false;
        loop {
            if cancel.load(Ordering::Relaxed) {
                return Ok(None);
            }
            if start.elapsed() > Duration::from_secs(300) {
                return Err("Screen sampler timed out; no color selected".into());
            }
            // SAFETY: own overlay remains live; lose-focus cancellation avoids a hidden session.
            if unsafe { GetForegroundWindow() } != overlay.0 {
                return Ok(None);
            }
            let mut msg = MSG::default();
            // SAFETY: writable MSG, drains only this worker's queue.
            while unsafe { PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE) }.as_bool() {
                if msg.message == WM_QUIT
                    || msg.message == WM_CLOSE
                    || msg.message == WM_RBUTTONUP
                    || ((msg.message == WM_KEYDOWN || msg.message == WM_SYSKEYDOWN)
                        && msg.wParam.0 == VK_ESCAPE.0 as usize)
                {
                    return Ok(None);
                }
                if msg.message == WM_LBUTTONDOWN {
                    armed = true;
                    continue;
                }
                if msg.message == WM_LBUTTONUP && armed {
                    let mut point = POINT::default();
                    // SAFETY: writable POINT; OS returns physical cursor coordinates.
                    unsafe { GetPhysicalCursorPos(&mut point) }.map_err(|e| e.to_string())?;
                    // SAFETY: own overlay only. Flush removal before reading underlying display.
                    unsafe {
                        let _ = ShowWindow(overlay.0, SW_HIDE);
                        DwmFlush()
                    }
                    .map_err(|e| format!("Display synchronization failed: {e}"))?;
                    return display_mean([point.x, point.y], radius, bounds).map(Some);
                }
                // SAFETY: queue messages belong to this thread's own registered window.
                unsafe { DispatchMessageW(&msg) };
            }
            std::thread::sleep(Duration::from_millis(8));
        }
    })
}
struct DesktopDc(HDC);
impl Drop for DesktopDc {
    fn drop(&mut self) {
        // SAFETY: matches this thread's one successful GetDC(None).
        unsafe { ReleaseDC(None, self.0) };
    }
}
fn dc_mean(
    dc: HDC,
    center: [i32; 2],
    radius: u8,
    bounds: Bounds,
    display: bool,
) -> Result<[u8; 3], String> {
    mean(center, radius, bounds, |p| {
        if display {
            // SAFETY: physical coordinates, read-only monitor query; gaps are excluded.
            if unsafe { MonitorFromPoint(POINT { x: p[0], y: p[1] }, MONITOR_DEFAULTTONULL) }
                .0
                .is_null()
            {
                return Ok(None);
            }
        }
        // SAFETY: live DC owned by caller, bounded coordinates; no pixels are written.
        let c = unsafe { GetPixel(dc, p[0], p[1]) }.0;
        if c == u32::MAX {
            return Err(
                "Display pixel is unavailable (protected content or display changed)".into(),
            );
        }
        Ok(Some([c as u8, (c >> 8) as u8, (c >> 16) as u8]))
    })
}
fn display_mean(center: [i32; 2], radius: u8, bounds: Bounds) -> Result<[u8; 3], String> {
    // SAFETY: read-only physical center/monitor check; do not sample monitor gaps.
    if unsafe {
        MonitorFromPoint(
            POINT {
                x: center[0],
                y: center[1],
            },
            MONITOR_DEFAULTTONULL,
        )
    }
    .0
    .is_null()
    {
        return Err("No display at the selected physical point".into());
    }
    // SAFETY: explicit user-confirmation path only, no stored screenshot or background reads.
    let dc = unsafe { GetDC(None) };
    if dc.0.is_null() {
        return Err("Display color acquisition failed".into());
    }
    let dc = DesktopDc(dc);
    dc_mean(dc.0, center, radius, bounds, true)
}
/// Own hidden window and memory-bitmap fixtures only; never reads desktop pixels.
pub fn verify_screen_boundary() -> Result<(), String> {
    drawer_core::screen_sample::verify()?;
    let _dpi = Dpi::physical()?;
    // SAFETY: create a hidden, private built-in-class fixture on this thread.
    let window = Window(
        unsafe {
            CreateWindowExW(
                WS_EX_TOOLWINDOW,
                w!("STATIC"),
                w!("Drawer sampler fixture"),
                WS_POPUP,
                30,
                40,
                8,
                8,
                None,
                None,
                None,
                None,
            )
        }
        .map_err(|e| e.to_string())?,
    );
    let mut before = WINDOWPLACEMENT {
        length: std::mem::size_of::<WINDOWPLACEMENT>() as u32,
        ..Default::default()
    };
    // SAFETY: live fixture and writable complete placement structure.
    unsafe { GetWindowPlacement(window.0, &mut before) }.map_err(|e| e.to_string())?;
    for mode in 0..3 {
        let result = guarded(window.0, true, || match mode {
            0 => Ok(None),
            1 => Err("fixture acquisition failure".into()),
            _ => Ok(Some([20, 80, 160])),
        });
        if (mode == 1) != result.is_err() {
            return Err("Screen failure/cancel outcome changed".into());
        }
        let mut after = before;
        // SAFETY: live own fixture; read-only state queries.
        unsafe { GetWindowPlacement(window.0, &mut after) }.map_err(|e| e.to_string())?;
        // SAFETY: read-only visibility query for the retained own fixture.
        let visible = unsafe { IsWindowVisible(window.0) }.as_bool();
        if before.rcNormalPosition != after.rcNormalPosition
            || before.showCmd != after.showCmd
            || visible
        {
            return Err(
                "Screen cancellation/failure changed original window placement or visibility"
                    .into(),
            );
        }
    }
    let overlay = create_overlay(Bounds {
        min: [30, 40],
        max: [38, 48],
    })?;
    // SAFETY: synchronous hit-test message to our hidden fixture, no pointer payload.
    let hit = unsafe {
        SendMessageW(
            overlay.0,
            WM_NCHITTEST,
            Some(WPARAM(0)),
            Some(LPARAM((41 << 16) | 31)),
        )
    };
    if hit.0 != HTCLIENT as isize {
        return Err("Screen overlay lets mouse input pass through".into());
    }
    verify_memory_dc()
}
fn verify_memory_dc() -> Result<(), String> {
    struct Memory {
        dc: HDC,
        bitmap: HBITMAP,
        old: HGDIOBJ,
    }
    impl Drop for Memory {
        fn drop(&mut self) {
            // SAFETY: deselect before deleting owned bitmap, then delete owned memory DC.
            unsafe {
                SelectObject(self.dc, self.old);
                let _ = DeleteObject(self.bitmap.into());
                let _ = DeleteDC(self.dc);
            }
        }
    }
    // SAFETY: memory-only DC and an independently allocated 3×3 bitmap, no screen read.
    let dc = unsafe { CreateCompatibleDC(None) };
    if dc.0.is_null() {
        return Err("Fixture memory DC failed".into());
    }
    // SAFETY: bounded 32-bit bitmap allocation without a pixel pointer.
    let bitmap = unsafe { CreateBitmap(3, 3, 1, 32, None) };
    if bitmap.0.is_null() {
        // SAFETY: own memory DC, bitmap allocation failed before selection.
        let _ = unsafe { DeleteDC(dc) };
        return Err("Fixture bitmap failed".into());
    }
    // SAFETY: select owned compatible bitmap into the owned memory DC.
    let old = unsafe { SelectObject(dc, bitmap.into()) };
    let m = Memory { dc, bitmap, old };
    for y in 0..3 {
        for x in 0..3 {
            // SAFETY: writes only the 3×3 own fixture, never the display.
            if !unsafe { SetPixelV(m.dc, x, y, COLORREF(if x == 1 { 255 } else { 255 << 16 })) }
                .as_bool()
            {
                return Err("Fixture pixel write failed".into());
            }
        }
    }
    if dc_mean(
        m.dc,
        [1, 1],
        1,
        Bounds {
            min: [0, 0],
            max: [3, 3],
        },
        false,
    )? != [85, 0, 170]
    {
        return Err("Native GDI RGB channel/mean mismatch".into());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn hidden_window_cancel_failure_restore_and_memory_dc() {
        super::verify_screen_boundary().unwrap();
    }
}
