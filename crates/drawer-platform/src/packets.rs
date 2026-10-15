//! Safe decoding of the public Wintab packet ABI; no driver memory is borrowed.
#![cfg(any(windows, test))]
use drawer_core::pointer_input::{Attributes, Event, Kind, Phase, Source};
pub const STATUS: u32 = 2;
pub const CURSOR: u32 = 0x20;
pub const BUTTONS: u32 = 0x40;
pub const X: u32 = 0x80;
pub const Y: u32 = 0x100;
pub const PRESSURE: u32 = 0x400;
pub const ORIENTATION: u32 = 0x1000;
pub const REQUIRED: u32 = BUTTONS | X | Y;
pub const SUPPORTED: u32 = REQUIRED | STATUS | CURSOR | PRESSURE | ORIENTATION;
/// Wintab UINT(-1) is the documented aggregate virtual tablet, not a corrupt ID.
pub fn device_category(device: u32) -> Option<u32> {
    match device {
        u32::MAX => Some(99),
        0..=99 => Some(100 + device),
        _ => None,
    }
}
pub fn tablet_point(point: [i32; 2], origin: [i32; 2], extent: [i32; 2]) -> Option<[f64; 2]> {
    if extent.contains(&0) {
        return None;
    }
    Some([
        (point[0] as f64 - origin[0] as f64) / extent[0] as f64,
        1.0 - (point[1] as f64 - origin[1] as f64) / extent[1] as f64,
    ])
}
pub fn touch_event(
    device: u64,
    id: u32,
    flags: u32,
    hundredths: [i32; 2],
    client_offset: [f64; 2],
    frame: u64,
) -> Option<Event> {
    if flags & 64 != 0 {
        return None;
    } // explicit TOUCHEVENTF_PEN is not a finger
    let phase = if flags & 4 != 0 {
        Phase::Up
    } else if flags & 2 != 0 {
        Phase::Down
    } else if flags & 1 != 0 {
        Phase::Move
    } else {
        return None;
    };
    Some(Event {
        source: Source {
            kind: Kind::Touch,
            device,
            contact: id as u64,
        },
        phase,
        position: std::array::from_fn(|n| hundredths[n] as f64 / 100.0 + client_offset[n]),
        pressure: None,
        attributes: Attributes {
            palm: Some(flags & 128 != 0),
            touch_frame: Some(frame),
            ..Default::default()
        },
        origin: Default::default(),
    })
}

#[derive(Clone, Copy, Default)]
#[repr(C)]
pub struct Axis {
    pub min: i32,
    pub max: i32,
    pub units: u32,
    pub resolution: u32,
}
impl Axis {
    pub fn pressure(self, value: u32) -> Option<f64> {
        (self.max > self.min).then(|| {
            ((value as f64 - self.min as f64) / (self.max as f64 - self.min as f64)).clamp(0.0, 1.0)
        })
    }
    fn radians(self, value: i32) -> Option<f64> {
        (self.units == 3 && self.resolution > 0)
            .then(|| value as f64 * std::f64::consts::TAU * 65536.0 / self.resolution as f64)
    }
}
#[derive(Default, Debug)]
pub struct Packet {
    pub status: Option<u32>,
    pub cursor: u32,
    pub buttons: u32,
    pub point: [i32; 2],
    pub pressure: Option<u32>,
    pub orientation: Option<[i32; 3]>,
}
pub fn words(mask: u32) -> Result<usize, String> {
    if mask & REQUIRED != REQUIRED || mask & !SUPPORTED != 0 {
        return Err("Unsupported Wintab packet layout".into());
    }
    Ok(mask.count_ones() as usize + if mask & ORIENTATION != 0 { 2 } else { 0 })
}
pub fn decode(mask: u32, data: &[u32]) -> Result<Packet, String> {
    if data.len() != words(mask)? {
        return Err("Truncated Wintab packet".into());
    }
    let mut n = 0;
    let mut take = || {
        let value = data[n];
        n += 1;
        value
    };
    let mut p = Packet::default();
    if mask & STATUS != 0 {
        p.status = Some(take());
    }
    if mask & CURSOR != 0 {
        p.cursor = take();
    }
    p.buttons = take();
    p.point = [take() as i32, take() as i32];
    if mask & PRESSURE != 0 {
        p.pressure = Some(take());
    }
    if mask & ORIENTATION != 0 {
        p.orientation = Some([take() as i32, take() as i32, take() as i32]);
    }
    Ok(p)
}
pub fn attributes(p: &Packet, axes: Option<[Axis; 3]>) -> Attributes {
    let tilt = p.orientation.zip(axes).and_then(|(raw, axes)| {
        let azimuth = axes[0].radians(raw[0])?;
        let altitude = axes[1]
            .radians(raw[1])?
            .abs()
            .min(std::f64::consts::FRAC_PI_2);
        Some([
            (azimuth.sin() * altitude.cos()).atan2(altitude.sin()),
            (-azimuth.cos() * altitude.cos()).atan2(altitude.sin()),
        ])
    });
    Attributes {
        time_micros: None,
        tilt,
        eraser: p.status.map(|s| s & 0x10 != 0),
        button: Some(p.buttons != 0),
        buttons: Some(p.buttons),
        tablet: None,
        palm: None,
        touch_frame: None,
    }
    .normalized()
}
/// Wintab supplies button state, not a contact lifecycle. Require a lift after
/// focus/backend changes, so a held pen cannot start a second stroke.
#[derive(Default)]
pub struct Contact {
    armed: bool,
    active: Option<Event>,
    sequence: u64,
}
impl Contact {
    pub fn reset(&mut self) -> Vec<Event> {
        self.armed = false;
        self.active
            .take()
            .map(|mut e| {
                e.phase = Phase::Cancel;
                e
            })
            .into_iter()
            .collect()
    }
    pub fn sample(
        &mut self,
        device: u64,
        cursor: u32,
        down: bool,
        point: [f64; 2],
        pressure: Option<f64>,
        attributes: Attributes,
    ) -> Vec<Event> {
        let mut out = Vec::new();
        if self
            .active
            .is_some_and(|e| e.source.contact >> 32 != cursor as u64)
        {
            out.extend(self.reset());
        }
        if !down {
            self.armed = true;
            if let Some(mut e) = self.active.take() {
                e.phase = Phase::Up;
                e.position = point;
                e.pressure = pressure;
                e.attributes = attributes;
                out.push(e);
            }
        } else if let Some(mut e) = self.active {
            e.phase = Phase::Move;
            e.position = point;
            e.pressure = pressure;
            e.attributes = attributes;
            self.active = Some(e);
            out.push(e);
        } else if self.armed {
            self.sequence = (self.sequence + 1) & 0xffff_ffff;
            let e = Event {
                source: Source {
                    kind: Kind::Pen,
                    device,
                    contact: ((cursor as u64) << 32) | self.sequence,
                },
                phase: Phase::Down,
                position: point,
                pressure,
                attributes,
                origin: Default::default(),
            };
            self.active = Some(e);
            out.push(e);
        }
        out
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn explicit_touch_flags_signed_hundredths_identity_palm_and_native_batch() {
        let e = touch_event(0x1234, 7, 2 | 128, [-1250, 2500], [50.0, -20.0], 99).unwrap();
        assert_eq!(
            e.source,
            Source {
                kind: Kind::Touch,
                device: 0x1234,
                contact: 7
            }
        );
        assert_eq!(e.position, [37.5, 5.0]);
        assert_eq!(e.phase, Phase::Down);
        assert_eq!(e.pressure, None);
        assert_eq!(e.attributes.palm, Some(true));
        assert_eq!(e.attributes.touch_frame, Some(99));
        assert!(touch_event(1, 1, 2 | 64, [0; 2], [0.0; 2], 0).is_none());
        assert!(touch_event(1, 1, 8, [0; 2], [0.0; 2], 0).is_none());
        assert_eq!(
            touch_event(1, 1, 4, [0; 2], [0.0; 2], 0).unwrap().phase,
            Phase::Up
        );
    }
    #[test]
    fn tablet_normalization_retains_subpixel_resolution_and_signed_axis_extent() {
        assert_eq!(
            tablet_point([100, 200], [100, 200], [10000, 20000]),
            Some([0.0, 1.0])
        );
        assert_eq!(
            tablet_point([10100, 20200], [100, 200], [10000, 20000]),
            Some([1.0, 0.0])
        );
        let p = tablet_point([101, 201], [100, 200], [10000, 20000]).unwrap();
        assert!((p[0] - 0.0001).abs() < 1e-12);
        assert!((p[1] - 0.99995).abs() < 1e-12);
        assert_eq!(
            tablet_point([-4900, -9800], [100, 200], [-10000, -20000]),
            Some([0.5; 2])
        );
        assert_eq!(tablet_point([0; 2], [0; 2], [0, 1]), None);
    }
    #[test]
    fn optional_packet_fields_signed_coordinates_and_pressure_range() {
        assert_eq!(device_category(u32::MAX), Some(99));
        assert_eq!(device_category(0), Some(100));
        assert_eq!(device_category(100), None);
        let minimal = decode(REQUIRED, &[1, (-50i32) as u32, 20]).unwrap();
        assert_eq!(minimal.point, [-50, 20]);
        assert_eq!(minimal.pressure, None);
        assert_eq!(attributes(&minimal, None).eraser, None);
        let full = decode(SUPPORTED, &[16, 3, 2, 40, 50, 1024, 0, 450, 0]).unwrap();
        assert_eq!(full.cursor, 3);
        assert_eq!(full.pressure, Some(1024));
        let axis = Axis {
            min: 0,
            max: 1024,
            units: 0,
            resolution: 0,
        };
        assert_eq!(axis.pressure(512), Some(0.5));
        assert_eq!(axis.pressure(2048), Some(1.0));
        assert_eq!(Axis::default().pressure(2), None);
        let circle = Axis {
            min: 0,
            max: 3600,
            units: 3,
            resolution: 3600 * 65536,
        };
        let a = attributes(&full, Some([circle; 3]));
        assert_eq!(a.eraser, Some(true));
        assert!((a.tilt.unwrap()[1] + std::f64::consts::FRAC_PI_4).abs() < 1e-10);
        assert!(decode(SUPPORTED, &[0; 8]).is_err());
        assert!(words(REQUIRED | 1).is_err());
        assert!(words(X | Y).is_err());
    }
    #[test]
    fn switching_focus_and_cursor_require_lift_and_preserve_release_identity() {
        let mut c = Contact::default();
        let sample = |c: &mut Contact, cursor, down| {
            c.sample(7, cursor, down, [0.0; 2], Some(0.5), Attributes::default())
        };
        assert!(sample(&mut c, 1, true).is_empty());
        assert!(sample(&mut c, 1, false).is_empty());
        let first = sample(&mut c, 1, true)[0];
        assert_eq!(first.phase, Phase::Down);
        assert_eq!(sample(&mut c, 1, true)[0].phase, Phase::Move);
        assert_eq!(sample(&mut c, 2, true)[0].phase, Phase::Cancel);
        assert!(sample(&mut c, 2, true).is_empty());
        sample(&mut c, 2, false);
        let second = sample(&mut c, 2, true)[0];
        assert_ne!(first.source, second.source);
        assert_eq!(sample(&mut c, 2, false)[0].source, second.source);
        sample(&mut c, 2, true);
        assert_eq!(c.reset()[0].phase, Phase::Cancel);
        assert!(sample(&mut c, 2, true).is_empty());
    }
}
