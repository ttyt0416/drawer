//! The small OS boundary. Safe consumers never receive raw message pointers.
#![deny(unsafe_code)]
use drawer_core::pointer_input::{Event, Kind, Origin, Phase, Source, origin_from_extra};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};
const LIMIT: usize = 65536;
#[derive(Default)]
struct State {
    window: isize,
    queue: VecDeque<Event>,
    overflow: bool,
    blocked: bool,
    down: [bool; 3],
}
impl State {
    fn record(&mut self, window: isize, message: u32, wparam: usize, lparam: isize, extra: u64) {
        if self.window == 0 || window != self.window {
            return;
        }
        // WM_MOUSEMOVE, WM_LBUTTONDOWN/UP/DBLCLK, WM_CAPTURECHANGED, WM_KILLFOCUS.
        let mut phase = match message {
            0x200 => Phase::Move,
            0x201 | 0x203 => Phase::Down,
            0x202 => Phase::Up,
            0x215 | 0x8 => Phase::Cancel,
            _ => return,
        };
        if self.blocked {
            if matches!(phase, Phase::Up | Phase::Cancel) {
                self.blocked = false;
                self.down = [false; 3];
            }
            return;
        }
        if self.queue.len() >= LIMIT {
            self.queue.clear();
            self.overflow = true;
            self.blocked = true;
            self.down = [false; 3];
            return;
        }
        let origin = origin_from_extra(extra);
        let key = match origin {
            Origin::Unmarked => 0,
            Origin::PenCompatibility => 1,
            Origin::TouchCompatibility => 2,
        };
        if phase == Phase::Move && self.down[key] && wparam & 1 == 0 {
            phase = Phase::Cancel;
        }
        match phase {
            Phase::Down => self.down[key] = true,
            Phase::Up | Phase::Cancel => self.down[key] = false,
            _ => {}
        }
        let source = if origin == Origin::TouchCompatibility {
            Source {
                kind: Kind::Touch,
                device: 0,
                contact: extra & 0x7f,
            }
        } else {
            Source::MOUSE
        };
        let point = lparam as u32;
        self.queue.push_back(Event {
            source,
            phase,
            position: [
                point as u16 as i16 as f64,
                (point >> 16) as u16 as i16 as f64,
            ],
            pressure: None,
            attributes: Default::default(),
            origin,
        });
    }
}
#[derive(Clone, Default)]
pub struct Input {
    state: Arc<Mutex<State>>,
}
impl Input {
    pub fn drain(&self, pixels_per_point: f64) -> Result<Vec<Event>, String> {
        if !pixels_per_point.is_finite() || pixels_per_point <= 0.0 {
            return Err("Invalid input display scale".into());
        }
        let mut state = self
            .state
            .lock()
            .map_err(|_| "Native input queue unavailable")?;
        if std::mem::take(&mut state.overflow) {
            state.queue.clear();
            return Err("Native input overflow; pending stroke cancelled".into());
        }
        Ok(state
            .queue
            .drain(..)
            .map(|mut e| {
                e.position = e.position.map(|p| p / pixels_per_point);
                e
            })
            .collect())
    }
    pub fn attached(&self) -> bool {
        self.state.lock().is_ok_and(|s| s.window != 0)
    }
}
// These two FFI operations are the sole exception to the application's unsafe
// prohibition. The module owns the winit callback and exposes a safe queue API.
#[cfg(windows)]
#[allow(unsafe_code)]
mod win {
    use super::*;
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use windows::Win32::UI::WindowsAndMessaging::{GetMessageExtraInfo, MSG};
    use winit::{event_loop::EventLoopBuilder, platform::windows::EventLoopBuilderExtWindows};
    impl Input {
        pub fn attach(&self, window: &impl HasWindowHandle) -> Result<(), String> {
            let RawWindowHandle::Win32(handle) =
                window.window_handle().map_err(|e| e.to_string())?.as_raw()
            else {
                return Err("Expected a Win32 window".into());
            };
            let mut s = self
                .state
                .lock()
                .map_err(|_| "Native input queue unavailable")?;
            s.window = handle.hwnd.get();
            s.queue.clear();
            s.overflow = false;
            s.blocked = false;
            s.down = [false; 3];
            Ok(())
        }
        pub fn install<T: 'static>(&self, builder: &mut EventLoopBuilder<T>) {
            let input = self.clone();
            builder.with_msg_hook(move |pointer| {
                if pointer.is_null() {
                    return false;
                }
                // SAFETY: winit invokes this callback immediately before dispatch
                // with a live MSG pointer (its documented with_msg_hook contract).
                // The reference is never stored or allowed to escape this call.
                let message = unsafe { &*pointer.cast::<MSG>() };
                let Ok(mut s) = input.state.lock() else {
                    return false;
                };
                let window = message.hwnd.0 as isize;
                if window != s.window
                    || s.window == 0
                    || !matches!(message.message, 0x200 | 0x201 | 0x202 | 0x203 | 0x215 | 0x8)
                {
                    return false;
                }
                // SAFETY: this parameterless User32 query reads only the current
                // UI thread's message metadata. It dereferences no caller memory.
                let extra = unsafe { GetMessageExtraInfo() }.0 as u64;
                s.record(
                    window,
                    message.message,
                    message.wParam.0,
                    message.lParam.0,
                    extra,
                );
                false // UI and winit still receive every message.
            });
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn promoted_pen_release_does_not_release_real_mouse_and_missing_up_cancels() {
        let input = Input::default();
        {
            let mut s = input.state.lock().unwrap();
            s.window = 1;
            s.record(1, 0x201, 1, 0, 0);
            s.record(1, 0x201, 1, 0, 0xff51_5701);
            s.record(1, 0x202, 0, 0, 0xff51_5701);
            s.record(1, 0x200, 1, 0, 0);
            s.record(1, 0x200, 0, 0, 0);
        }
        let e = input.drain(1.0).unwrap();
        assert_eq!(e[3].phase, Phase::Move);
        assert_eq!(e[4].phase, Phase::Cancel);
        assert_eq!(e[4].origin, Origin::Unmarked);
    }
    #[test]
    fn own_window_signed_coordinates_order_origin_and_drain_scaling() {
        let input = Input::default();
        {
            let mut s = input.state.lock().unwrap();
            s.window = 17;
            s.record(18, 0x201, 0, 0, 0);
            s.record(17, 0x100, 0, 0, 0);
            s.record(
                17,
                0x201,
                1,
                ((20u32 << 16) | (-10i16 as u16 as u32)) as isize,
                0xff51_5701,
            );
            s.record(17, 0x202, 0, ((40u32 << 16) | 30) as isize, 0xff51_5701);
            s.record(17, 0x201, 0, 0, 0);
            s.record(17, 0x202, 0, 0, 0);
        }
        let events = input.drain(2.0).unwrap();
        assert_eq!(events.len(), 4);
        assert_eq!(events[0].position, [-5.0, 10.0]);
        assert_eq!(events[0].origin, Origin::PenCompatibility);
        assert_eq!(events[1].phase, Phase::Up);
        assert_eq!(events[2].origin, Origin::Unmarked);
        assert!(input.drain(1.0).unwrap().is_empty());
        assert!(input.drain(0.0).is_err());
    }
    #[test]
    fn queue_overflow_discards_contact_until_release_without_growing_memory() {
        let input = Input::default();
        {
            let mut s = input.state.lock().unwrap();
            s.window = 1;
            for _ in 0..=LIMIT {
                s.record(1, 0x200, 0, 0, 0);
            }
            assert!(s.queue.is_empty());
            assert!(s.blocked);
            s.record(1, 0x201, 0, 0, 0);
            s.record(1, 0x202, 0, 0, 0);
            assert!(!s.blocked);
        }
        assert!(input.drain(1.0).is_err());
        {
            let mut s = input.state.lock().unwrap();
            s.record(1, 0x201, 0, 0, 0);
            s.record(1, 0x202, 0, 0, 0);
        }
        assert_eq!(input.drain(1.0).unwrap().len(), 2);
    }
}
