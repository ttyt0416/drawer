//! The small OS boundary. Safe consumers never receive raw message pointers.
#![deny(unsafe_code)]
use drawer_core::pointer_input::{
    Backend, Event, Kind, Origin, Phase, Source, backend_candidates, origin_from_extra,
};
use std::{cell::RefCell, collections::VecDeque, rc::Rc};
const LIMIT: usize = 65536;
#[derive(Default)]
struct State {
    window: isize,
    queue: VecDeque<Event>,
    overflow: bool,
    blocked: bool,
    down: [bool; 3],
    requested: Option<Backend>,
    effective: Backend,
    report: String,
    failure: Option<String>,
    #[cfg(windows)]
    driver: Option<win::Driver>,
    #[cfg(windows)]
    touch: Option<touch::Hook>,
    #[cfg(windows)]
    touch_contacts: Vec<Event>,
    #[cfg(windows)]
    touch_frame: u64,
    #[cfg(windows)]
    touch_error: Option<String>,
    // Last: release native context and DLL before the window's owner.
    #[cfg(windows)]
    owner: Option<std::sync::Arc<winit::window::Window>>,
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
    state: Rc<RefCell<State>>,
}
impl Input {
    pub fn drain(&self, pixels_per_point: f64) -> Result<Vec<Event>, String> {
        if !pixels_per_point.is_finite() || pixels_per_point <= 0.0 {
            return Err("Invalid input display scale".into());
        }
        let mut state = self
            .state
            .try_borrow_mut()
            .map_err(|_| "Native input queue unavailable")?;
        if let Some(error) = state.failure.take() {
            state.queue.clear();
            return Err(error);
        }
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
        self.state.try_borrow().is_ok_and(|s| s.window != 0)
    }
    pub fn requested(&self) -> Option<Backend> {
        self.state.borrow().requested
    }
    pub fn effective(&self) -> Backend {
        self.state.borrow().effective
    }
    pub fn report(&self) -> String {
        self.state.borrow().report.clone()
    }
}
/// Only a successfully initialized backend is retained. Mouse needs no pen driver.
fn select<T>(
    preferred: Backend,
    mut attempt: impl FnMut(Backend) -> Result<T, String>,
) -> (Backend, Option<T>, String) {
    let mut reasons = Vec::new();
    for &candidate in backend_candidates(preferred) {
        if candidate == Backend::Mouse {
            return (candidate, None, reasons.join("; "));
        }
        match attempt(candidate) {
            Ok(driver) => return (candidate, Some(driver), reasons.join("; ")),
            Err(e) => reasons.push(format!("{candidate:?}: {e}")),
        }
    }
    unreachable!("Every candidate list ends with Mouse")
}
mod packets;
// Raw pointers, DLL and context lifetimes stay inside this UI-thread-only boundary.
#[cfg(windows)]
#[allow(unsafe_code)]
mod acquire;
#[cfg(windows)]
#[allow(unsafe_code)]
mod touch;
#[cfg(windows)]
#[allow(unsafe_code)]
mod win;
#[cfg(windows)]
pub use acquire::{acquisition_capture, acquisition_list};
#[cfg(windows)]
#[allow(unsafe_code)]
mod child_job;
#[cfg(windows)]
#[allow(unsafe_code)]
mod dial;
#[cfg(windows)]
pub use child_job::acquisition_child_job;
#[cfg(windows)]
pub use child_job::acquisition_child_job as local_inference_child_job;
#[cfg(windows)]
pub use dial::Dial;
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn failed_initialization_falls_back_without_opening_parallel_drivers() {
        let mut tried = Vec::new();
        let (effective, driver, reason) = select(Backend::Wintab, |b| {
            tried.push(b);
            if b == Backend::Wintab {
                Err("not installed".into())
            } else {
                Ok(17)
            }
        });
        assert_eq!(tried, vec![Backend::Wintab, Backend::Pointer]);
        assert_eq!(effective, Backend::Pointer);
        assert_eq!(driver, Some(17));
        assert!(reason.contains("not installed"));
        let (effective, driver, reason) =
            select::<()>(Backend::Wintab, |_| Err("initialization failed".into()));
        assert_eq!(effective, Backend::Mouse);
        assert!(driver.is_none());
        assert!(reason.contains("Pointer"));
        let (_, driver, _) = select::<()>(Backend::Mouse, |_| {
            panic!("Mouse must not open a pen driver")
        });
        assert!(driver.is_none());
        let mut count = 0;
        let (effective, _, _) = select(Backend::Pointer, |_| {
            count += 1;
            Ok(())
        });
        assert_eq!(effective, Backend::Pointer);
        assert_eq!(count, 1);
    }
    #[test]
    fn promoted_pen_release_does_not_release_real_mouse_and_missing_up_cancels() {
        let input = Input::default();
        {
            let mut s = input.state.borrow_mut();
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
            let mut s = input.state.borrow_mut();
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
            let mut s = input.state.borrow_mut();
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
            let mut s = input.state.borrow_mut();
            s.record(1, 0x201, 0, 0, 0);
            s.record(1, 0x202, 0, 0, 0);
        }
        assert_eq!(input.drain(1.0).unwrap().len(), 2);
    }
}

#[cfg(windows)]
#[allow(unsafe_code)]
mod screen_sample;
#[cfg(windows)]
pub use screen_sample::{ScreenSample, verify_screen_boundary};

#[cfg(windows)]
#[allow(unsafe_code)]
pub mod gpu_color;

#[allow(unsafe_code)]
pub mod local_files;

#[cfg(windows)]
#[allow(unsafe_code)]
pub mod book_pdf;
#[cfg(windows)]
#[allow(unsafe_code)]
pub mod book_print;

#[cfg(windows)]
#[allow(unsafe_code)]
pub mod audio;
