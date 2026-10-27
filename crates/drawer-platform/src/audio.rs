//! Windows PCM sink. Raw device/header ownership stays on one dedicated worker.
use drawer_core::audio_mix::Stream;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use windows::Win32::Media::{Audio::*, MMTIME, TIME_BYTES, TIME_MS, TIME_SAMPLES};
#[derive(Clone, Debug, Default)]
pub struct Status {
    pub played: u64,
    pub queued: u64,
    pub clipped: u64,
    pub finished: bool,
    pub error: Option<String>,
    pub clock: String,
}
pub struct Player {
    stop: Arc<AtomicBool>,
    status: Arc<Mutex<Status>>,
    worker: Option<std::thread::JoinHandle<()>>,
}
impl Player {
    pub fn start(stream: Stream) -> Result<Self, String> {
        if stream.plan.rate != 48000 || stream.plan.channels != 2 {
            return Err("Windows playback requires48kHz stereo PCM".into());
        }
        let stop = Arc::new(AtomicBool::new(false));
        let status = Arc::new(Mutex::new(Status::default()));
        let flag = stop.clone();
        let shared = status.clone();
        let worker = std::thread::Builder::new()
            .name("drawer-audio".into())
            .spawn(move || {
                if let Err(e) = run(&stream, &flag, &shared) {
                    let mut s = shared.lock().unwrap_or_else(|p| p.into_inner());
                    s.error = Some(e);
                    s.finished = true;
                }
            })
            .map_err(|e| e.to_string())?;
        Ok(Self {
            stop,
            status,
            worker: Some(worker),
        })
    }
    pub fn status(&self) -> Status {
        self.status
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }
}
impl Drop for Player {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(w) = self.worker.take() {
            let _ = w.join();
        }
    }
}
struct Slot {
    header: Box<WAVEHDR>,
    pcm: Vec<i16>,
    prepared: bool,
    queued: bool,
}
struct Device {
    handle: HWAVEOUT,
    slots: Vec<Slot>,
}
fn check(code: u32, action: &str) -> Result<(), String> {
    if code == 0 {
        Ok(())
    } else {
        Err(format!(
            "Windows audio {action} failed (code{code}); visual playback / WAV export remain available"
        ))
    }
}
impl Device {
    fn open() -> Result<Self, String> {
        // SAFETY: count query has no pointer arguments or persistent state.
        if unsafe { waveOutGetNumDevs() } == 0 {
            return Err(
                "No Windows audio output device; visual playback / WAV export remain available"
                    .into(),
            );
        }
        let format = WAVEFORMATEX {
            wFormatTag: 1,
            nChannels: 2,
            nSamplesPerSec: 48000,
            nAvgBytesPerSec: 192000,
            nBlockAlign: 4,
            wBitsPerSample: 16,
            cbSize: 0,
        };
        let mut handle = HWAVEOUT::default();
        // SAFETY: output handle and format live throughout the synchronous open; no callback is installed.
        check(
            // SAFETY: output handle and format are initialized locals; CALLBACK_NULL installs no callback.
            unsafe {
                waveOutOpen(
                    Some(&mut handle),
                    WAVE_MAPPER,
                    &format,
                    None,
                    None,
                    CALLBACK_NULL,
                )
            },
            "open",
        )?;
        let slots = (0..4)
            .map(|_| Slot {
                header: Box::new(WAVEHDR::default()),
                pcm: vec![0; 2048],
                prepared: false,
                queued: false,
            })
            .collect();
        Ok(Self { handle, slots })
    }
    fn release(&mut self) -> bool {
        // SAFETY: handle is owned by this worker. Reset synchronously returns queued buffers before freeing.
        if unsafe { waveOutReset(self.handle) } != 0 {
            return false;
        }
        let mut okay = true;
        for slot in &mut self.slots {
            if slot.prepared {
                // SAFETY: reset returned this stable boxed header; each successful prepare is balanced once.
                if unsafe {
                    waveOutUnprepareHeader(
                        self.handle,
                        &mut *slot.header,
                        std::mem::size_of::<WAVEHDR>() as u32,
                    )
                } == 0
                {
                    slot.prepared = false;
                } else {
                    okay = false;
                }
            }
        }
        if !okay {
            return false;
        }
        // SAFETY: no queued/prepared buffers remain, and the owned handle is closed exactly once.
        (unsafe { waveOutClose(self.handle) }) == 0
    }
}
// Retain native-owned allocations on a broken driver instead of releasing live pointers.
struct Owned(Option<Device>);
impl Drop for Owned {
    fn drop(&mut self) {
        if let Some(mut d) = self.0.take()
            && !d.release()
        {
            std::mem::forget(d);
        }
    }
}
#[derive(Default)]
struct Counter {
    unit: Option<u32>,
    last: u32,
    high: u64,
}
impl Counter {
    fn advance(&mut self, raw: u32, unit: u32) -> Result<u64, String> {
        if self.unit.is_some_and(|u| u != unit) {
            return Err("Audio device changed its position clock units".into());
        }
        self.unit = Some(unit);
        if raw < self.last {
            if self.last - raw > u32::MAX / 2 {
                self.high += 1u64 << 32;
            } else {
                return Ok(self.convert(self.high + self.last as u64, unit));
            }
        }
        self.last = raw;
        Ok(self.convert(self.high + raw as u64, unit))
    }
    fn convert(&self, n: u64, unit: u32) -> u64 {
        match unit {
            TIME_BYTES => n / 4,
            TIME_MS => n * 48,
            _ => n,
        }
    }
}
fn run(stream: &Stream, stop: &AtomicBool, status: &Mutex<Status>) -> Result<(), String> {
    let mut owned = Owned(Some(Device::open()?));
    let device = owned.0.as_mut().unwrap();
    let (mut written, mut clipped) = (0, 0);
    let mut counter = Counter::default();
    loop {
        if stop.load(Ordering::Relaxed) {
            return Ok(());
        }
        for slot in &mut device.slots {
            if slot.queued {
                // SAFETY: the driver owns this stable header until unprepare succeeds. We never read
                // asynchronously modified header flags from Rust; STILLPLAYING keeps its ownership intact.
                let code = unsafe {
                    waveOutUnprepareHeader(
                        device.handle,
                        &mut *slot.header,
                        std::mem::size_of::<WAVEHDR>() as u32,
                    )
                };
                if code == WAVERR_STILLPLAYING {
                    continue;
                }
                check(code, "unprepare")?;
                slot.queued = false;
                slot.prepared = false;
            }
            let (pcm, c) = stream.block(written, 1024)?;
            if pcm.is_empty() {
                continue;
            }
            clipped += c;
            let count = pcm.len();
            slot.pcm[..count].copy_from_slice(&pcm);
            *slot.header = WAVEHDR {
                lpData: windows::core::PSTR(slot.pcm.as_mut_ptr().cast()),
                dwBufferLength: (count * 2) as u32,
                ..Default::default()
            };
            // SAFETY: boxed header and PCM allocation remain immovable and unmodified until
            // unprepare/reset confirms the driver has returned both buffers.
            check(
                // SAFETY: stable boxed header points to initialized PCM and remains owned until unprepare/reset.
                unsafe {
                    waveOutPrepareHeader(
                        device.handle,
                        &mut *slot.header,
                        std::mem::size_of::<WAVEHDR>() as u32,
                    )
                },
                "prepare",
            )?;
            slot.prepared = true;
            // SAFETY: this prepared header points to the initialized count*2 bytes of stable PCM.
            check(
                // SAFETY: the successfully prepared header and its PCM allocation stay immovable while queued.
                unsafe {
                    waveOutWrite(
                        device.handle,
                        &mut *slot.header,
                        std::mem::size_of::<WAVEHDR>() as u32,
                    )
                },
                "write",
            )?;
            slot.queued = true;
            written += count as u64 / 2;
        }
        let mut time = MMTIME {
            wType: TIME_SAMPLES,
            ..Default::default()
        };
        // SAFETY: a writable full-sized output structure is passed for this synchronous query.
        check(
            // SAFETY: writable full-sized MMTIME is initialized for this synchronous output query.
            unsafe {
                waveOutGetPosition(
                    device.handle,
                    &mut time,
                    std::mem::size_of::<MMTIME>() as u32,
                )
            },
            "clock",
        )?;
        // SAFETY: wType selects which initialized union field the synchronous device query returned.
        let (raw, unit, label) = unsafe {
            match time.wType {
                TIME_SAMPLES => (time.u.sample, TIME_SAMPLES, "sample position"),
                TIME_BYTES => (time.u.cb, TIME_BYTES, "byte position"),
                TIME_MS => (time.u.ms, TIME_MS, "millisecond position"),
                _ => return Err("Unsupported Windows audio position clock".into()),
            }
        };
        let mut played = counter.advance(raw, unit)?.min(written);
        let finished = stream.duration().is_some_and(|n| written >= n)
            && device.slots.iter().all(|s| !s.queued);
        if finished {
            played = written;
        }
        *status.lock().unwrap_or_else(|p| p.into_inner()) = Status {
            played,
            queued: written - played,
            clipped,
            finished,
            error: None,
            clock: label.into(),
        };
        if finished {
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}
pub fn verify() -> Result<String, String> {
    let d = drawer_core::keyframes_verify::fixture();
    let plan = Arc::new(drawer_core::audio_mix::Plan::capture(&d, 48000, 2)?);
    let stream = plan.stream(0, 0, 6, false)?;
    let player = Player::start(stream)?;
    let start = std::time::Instant::now();
    let (mut last, mut baseline, mut error): (u64, Option<(f64, u64)>, f64) = (0, None, 0.);
    loop {
        let s = player.status();
        if let Some(e) = s.error {
            return if e.contains("No Windows audio output device") || e.contains("open failed") {
                Ok(format!("Unavailable; fallback verified: {e}"))
            } else {
                Err(e)
            };
        }
        if s.played < last {
            return Err("Windows audio position went backwards".into());
        }
        last = s.played;
        if s.played > 0 {
            let elapsed = start.elapsed().as_secs_f64();
            if let Some((t, n)) = baseline {
                let measured = ((elapsed - t) - (s.played - n) as f64 / 48000.).abs() * 1000.;
                error = error.max(measured);
            } else {
                baseline = Some((elapsed, s.played));
            }
        }
        if s.finished {
            if s.played != 12000 {
                return Err("Windows audio completion did not match250ms PCM".into());
            }
            return Ok(format!(
                "Silent PCM12000 frames; {}; observed clock-vs-monotonic error max{error:.2}ms (excludes speaker/display latency)",
                s.clock
            ));
        }
        if start.elapsed().as_secs_f64() > 3. {
            return Err("Windows audio device did not finish the250ms silent stream".into());
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}
#[cfg(test)]
mod tests {
    #[test]
    fn windows_silent_pcm_device_clock_and_cleanup() {
        println!("{}", super::verify().unwrap());
    }
    #[test]
    fn device_counter_extends_wrap_and_rejects_unit_changes() {
        let mut c = super::Counter::default();
        assert_eq!(
            c.advance(u32::MAX - 1, super::TIME_SAMPLES).unwrap(),
            u32::MAX as u64 - 1
        );
        assert_eq!(c.advance(1, super::TIME_SAMPLES).unwrap(), (1u64 << 32) + 1);
        assert!(c.advance(2, super::TIME_BYTES).is_err());
    }
}
