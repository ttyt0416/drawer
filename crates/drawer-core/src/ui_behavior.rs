use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub version: u32,
    pub auto_hide: bool,
    pub hide_delay_ms: u32,
    pub scrollbars: bool,
    pub brush_cursor: bool,
    pub edge_pan: bool,
    pub edge_margin: f64,
    pub edge_speed: f64,
    pub prevent_sleep: bool,
    pub keep_display: bool,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            version: 1,
            auto_hide: false,
            hide_delay_ms: 800,
            scrollbars: true,
            brush_cursor: true,
            edge_pan: false,
            edge_margin: 24.0,
            edge_speed: 300.0,
            prevent_sleep: false,
            keep_display: false,
        }
    }
}
impl Settings {
    pub fn valid(self) -> bool {
        self.version == 1
            && self.hide_delay_ms <= 5000
            && (1.0..=128.0).contains(&self.edge_margin)
            && (1.0..=2000.0).contains(&self.edge_speed)
    }
}
#[derive(Clone, Copy, Default)]
pub struct AutoHide {
    since: Option<f64>,
    pub collapsed: bool,
}
impl AutoHide {
    pub fn update(&mut self, enabled: bool, interactive: bool, now: f64, delay_ms: u32) -> bool {
        if !enabled || interactive || !now.is_finite() {
            self.reveal();
            return true;
        }
        let since = self.since.get_or_insert(now);
        if now < *since {
            *since = now;
        }
        self.collapsed = (now - *since) * 1000.0 + 1e-7 >= delay_ms as f64;
        !self.collapsed
    }
    pub fn reveal(&mut self) {
        self.since = None;
        self.collapsed = false;
    }
}
pub fn edge_delta(
    point: [f64; 2],
    rect: crate::scene::Rect,
    ppp: f64,
    dt: f64,
    s: Settings,
) -> [f64; 2] {
    if !s.edge_pan
        || !s.valid()
        || !ppp.is_finite()
        || ppp <= 0.0
        || !dt.is_finite()
        || !point.iter().all(|v| v.is_finite())
    {
        return [0.0; 2];
    }
    let mut result = [0.0; 2];
    for axis in 0..2 {
        let width = rect.max[axis] - rect.min[axis];
        if !width.is_finite() || width <= 0.0 {
            continue;
        }
        let margin = (s.edge_margin / ppp).min(width / 2.0);
        let positive = ((rect.min[axis] + margin - point[axis]) / margin).clamp(0.0, 1.0);
        let negative = ((point[axis] - rect.max[axis] + margin) / margin).clamp(0.0, 1.0);
        result[axis] = (positive - negative) * s.edge_speed * dt.clamp(0.0, 0.05) / ppp;
    }
    result
}
/// A single lease; disabled/idle transitions and Drop release its owned token.
pub struct Wake<T> {
    token: Option<T>,
    failed: bool,
}
impl<T> Default for Wake<T> {
    fn default() -> Self {
        Self {
            token: None,
            failed: false,
        }
    }
}
impl<T> Wake<T> {
    pub fn active(&self) -> bool {
        self.token.is_some()
    }
    pub fn reset(&mut self) {
        self.token = None;
        self.failed = false;
    }
    pub fn update(
        &mut self,
        enabled: bool,
        working: bool,
        create: impl FnOnce() -> Result<T, String>,
    ) -> Result<(), String> {
        if !enabled || !working {
            self.reset();
            return Ok(());
        }
        if self.token.is_none() && !self.failed {
            match create() {
                Ok(t) => self.token = Some(t),
                Err(e) => {
                    self.failed = true;
                    return Err(e);
                }
            }
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn auto_hide_delay_focus_recovery_bad_clock_and_disabled() {
        let mut h = AutoHide::default();
        assert!(h.update(true, false, 1.0, 800));
        assert!(h.update(true, false, 1.799, 800));
        assert!(!h.update(true, false, 1.8, 800));
        assert!(h.update(true, true, 2.0, 800));
        assert!(h.update(false, false, 9.0, 800));
        assert!(h.update(true, false, f64::NAN, 800));
    }
    #[test]
    fn edge_pan_physical_speed_corners_dt_and_disabled() {
        let rect = crate::scene::Rect {
            min: [0.0; 2],
            max: [500.0; 2],
        };
        let s = Settings {
            edge_pan: true,
            ..Default::default()
        };
        for ppp in [1.0, 1.5, 2.0] {
            let d = edge_delta([500.0, 0.0], rect, ppp, 0.02, s);
            assert!((d[0] * ppp + 6.0).abs() < 1e-9 && (d[1] * ppp - 6.0).abs() < 1e-9);
            assert_eq!(edge_delta([250.0; 2], rect, ppp, 0.02, s), [0.0; 2]);
        }
        assert_eq!(edge_delta([500.0; 2], rect, 1.0, f64::NAN, s), [0.0; 2]);
        assert_eq!(
            edge_delta([500.0; 2], rect, 1.0, 1.0, Settings::default()),
            [0.0; 2]
        );
    }
    #[test]
    fn sleep_leases_disabled_completion_error_cancel_and_drop() {
        use std::{cell::Cell, rc::Rc};
        struct Token(Rc<Cell<usize>>);
        impl Drop for Token {
            fn drop(&mut self) {
                self.0.set(self.0.get() - 1);
            }
        }
        let count = Rc::new(Cell::new(0));
        let mut wake = Wake::default();
        let create = || {
            count.set(count.get() + 1);
            Ok(Token(count.clone()))
        };
        wake.update(false, true, create).unwrap();
        assert_eq!(count.get(), 0);
        wake.update(true, true, create).unwrap();
        wake.update(true, true, create).unwrap();
        assert_eq!(count.get(), 1);
        wake.update(true, false, create).unwrap();
        assert_eq!(count.get(), 0);
        assert!(wake.update(true, true, || Err("failure".into())).is_err());
        wake.update(true, true, create).unwrap();
        assert_eq!(count.get(), 0);
        wake.update(false, false, create).unwrap();
        wake.update(true, true, create).unwrap();
        drop(wake);
        assert_eq!(count.get(), 0);
    }
}
