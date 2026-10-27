use crate::{
    audio::{self as core, Raw},
    audio_mix as mix,
    document::{DEFAULT_DOCUMENT_BUDGET as B, Document},
};
use std::{
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};
fn check(ok: bool, message: &str) -> Result<(), String> {
    if ok { Ok(()) } else { Err(message.into()) }
}
pub fn wav(value: i16, count: usize) -> Vec<u8> {
    let mut b = mix::wav_header(count as u64, 8000, 1).unwrap();
    for _ in 0..count {
        b.extend(value.to_le_bytes());
    }
    b
}
fn encoded(kind: u16, bits: u16, data: &[u8]) -> Vec<u8> {
    let mut b = mix::wav_header(1, 8000, 1).unwrap();
    b[4..8].copy_from_slice(&(36u32 + data.len() as u32 + data.len() as u32 % 2).to_le_bytes());
    b[20..22].copy_from_slice(&kind.to_le_bytes());
    b[28..32].copy_from_slice(&(8000 * (bits as u32 / 8)).to_le_bytes());
    b[32..34].copy_from_slice(&(bits / 8).to_le_bytes());
    b[34..36].copy_from_slice(&bits.to_le_bytes());
    b[40..44].copy_from_slice(&(data.len() as u32).to_le_bytes());
    b.extend(data);
    if data.len() % 2 == 1 {
        b.push(0);
    }
    b
}
fn decoder() -> Result<(), String> {
    check(
        core::decode(&encoded(1, 8, &[0, 128, 255]), None)?
            .samples
            .as_ref()
            == &[-32768, 0, 32512],
        "PCM8 normalization",
    )?;
    check(
        core::decode(&encoded(1, 24, &[255, 255, 127, 0, 0, 128]), None)?
            .samples
            .as_ref()
            == &[32767, -32768],
        "Signed PCM24 normalization",
    )?;
    check(
        core::decode(&encoded(1, 32, &i32::MAX.to_le_bytes()), None)?
            .samples
            .as_ref()
            == &[32767],
        "PCM32 normalization",
    )?;
    let b = [
        (-1f32).to_le_bytes(),
        0.5f32.to_le_bytes(),
        1.5f32.to_le_bytes(),
    ]
    .concat();
    let f = core::decode(&encoded(3, 32, &b), None)?;
    check(
        f.quantized && f.samples.as_ref() == &[-32768, 16384, 32767],
        "Float32 normalization / acknowledgement",
    )?;
    check(
        core::decode(&encoded(3, 32, &f32::NAN.to_le_bytes()), None).is_err(),
        "NaN float sample accepted",
    )?;
    let raw = Raw {
        rate: 44100,
        channels: 2,
    };
    check(
        core::decode(&[0, 0, 255, 127], Some(raw))?.samples.as_ref() == &[0, 32767],
        "Explicit raw stereo PCM16",
    )?;
    check(
        core::decode(&[0, 1, 2], Some(raw)).is_err()
            && core::decode(&[0, 0], Some(Raw { channels: 0, ..raw })).is_err(),
        "Raw alignment / zero channels",
    )?;
    let base = encoded(1, 24, &[0, 0, 64]);
    let mut ext = base[..36].to_vec();
    ext[16..20].copy_from_slice(&40u32.to_le_bytes());
    ext[20..22].copy_from_slice(&65534u16.to_le_bytes());
    ext.extend(22u16.to_le_bytes());
    ext.extend(24u16.to_le_bytes());
    ext.extend(4u32.to_le_bytes());
    ext.extend([1, 0, 0, 0, 0, 0, 16, 0, 128, 0, 0, 170, 0, 56, 155, 113]);
    ext.extend(&base[36..]);
    let len = ext.len() as u32 - 8;
    ext[4..8].copy_from_slice(&len.to_le_bytes());
    check(
        core::decode(&ext, None)?.samples.as_ref() == &[16384],
        "Extensible PCM subformat",
    )?;
    let mut invalid = wav(1, 100);
    invalid[20..22].copy_from_slice(&6u16.to_le_bytes());
    check(
        core::decode(&invalid, None).is_err(),
        "Unverified compressed WAV accepted",
    )?;
    invalid = wav(1, 100);
    invalid.pop();
    check(
        core::decode(&invalid, None).is_err(),
        "Truncated WAV accepted",
    )?;
    Ok(())
}
pub fn fixture(root: &Path) -> Result<(Document, u64), String> {
    let file = root.join("sound.wav");
    std::fs::write(&file, wav(10000, 3200)).map_err(|e| e.to_string())?;
    let input = core::load(&file, None, true)?;
    let mut d = crate::keyframes_verify::fixture();
    let state = core::insert(&d, &input, 2, false, B)?;
    d.commit_state(state)?;
    let id = d.scene.timing.audio[0].id;
    Ok((d, id))
}
pub fn verify(root: &Path) -> Result<(), String> {
    let dir = root.join("audio-verify");
    std::fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let result = (|| {
        decoder()?;
        let (mut d, id) = fixture(&dir)?;
        let source = d.animation.as_ref().unwrap().audio[0].samples.clone();
        let count = d.history_counts().0;
        let before = d.snapshot();
        let state = core::edit(&d, id, "Voice, 한글".into(), 4, 50, false, B)?;
        d.commit_state(state)?;
        check(
            d.history_counts().0 == count + 1
                && d.dirty
                && d.undo()
                && d.snapshot() == before
                && d.redo(),
            "Audio edit single Undo/Redo",
        )?;
        check(
            d.animation.as_ref().unwrap().audio[0].samples == source && d.layers == before.layers,
            "Audio controls modified PCM / drawing",
        )?;
        let plan = Arc::new(mix::Plan::capture(&d, 48000, 2)?);
        let stream = plan.clone().stream(0, 0, 24, false)?;
        let start = mix::boundary(4, plan.fps, 48000);
        check(
            stream.block(start - 1, 1)?.0 == [0, 0] && stream.block(start + 1, 2)?.0 == [5000; 4],
            "Delay / gain / mono-to-stereo mix",
        )?;
        check(
            stream.position(0).frame == 0
                && stream.position(stream.duration().unwrap()).finished
                && stream.position(stream.duration().unwrap()).frame == 23,
            "Playback first / last",
        )?;
        let output = dir.join("mix.wav");
        let dest = mix::Destination::capture(&stream, &output, false)?;
        let state = core::edit(&d, id, "Muted".into(), 4, 100, true, B)?;
        d.commit_state(state)?;
        let mut muted = Arc::new(mix::Plan::capture(&d, 48000, 2)?).stream(0, 0, 24, false)?;
        check(muted.block(start + 1, 2)?.0 == [0; 4], "Mute mix")?;
        let clipped = mix::write(&stream, &dest, &AtomicBool::new(false), &AtomicU64::new(0))?;
        check(clipped == 0, "Unexpected mix clipping")?;
        let bytes = std::fs::read(&output).map_err(|e| e.to_string())?;
        let decoded = core::decode(&bytes, None)?;
        check(
            decoded.rate == 48000
                && decoded.channels == 2
                && decoded.frames() == 48000
                && decoded.samples[(start as usize + 1) * 2] == 5000,
            "Actual WAV rate / sample count / isolated snapshot",
        )?;
        let again = dir.join("again.wav");
        let dest2 = mix::Destination::capture(&stream, &again, false)?;
        mix::write(&stream, &dest2, &AtomicBool::new(false), &AtomicU64::new(0))?;
        check(
            std::fs::read(&again).map_err(|e| e.to_string())? == bytes,
            "WAV reproducibility",
        )?;
        check(
            mix::Destination::capture(&stream, &dir.join("sound.wav"), true).is_err()
                && mix::Destination::capture(&stream, &output, false).is_err(),
            "Source / existing WAV overwritten without confirmation",
        )?;
        let stale = mix::Destination::capture(&stream, &output, true)?;
        std::fs::write(&output, b"foreign replacement").map_err(|e| e.to_string())?;
        check(
            mix::write(&stream, &stale, &AtomicBool::new(false), &AtomicU64::new(0)).is_err()
                && std::fs::read(&output).unwrap() == b"foreign replacement",
            "Changed destination not preserved",
        )?;
        let cancelled = dir.join("cancel.wav");
        let dest = mix::Destination::capture(&stream, &cancelled, false)?;
        check(
            mix::write(&stream, &dest, &AtomicBool::new(true), &AtomicU64::new(0)).is_err()
                && !cancelled.exists(),
            "Pre-cancelled WAV published",
        )?;
        muted = muted.limited(4800)?;
        check(
            muted.duration() == Some(4800)
                && muted.block(4096, 4096)?.0.len() == 704 * 2
                && muted.block(4800, 1)?.0.is_empty()
                && muted.position(4800).finished,
            "100ms scrub not bounded independently of UI updates",
        )?;
        // Native metadata and PCM survive missing or changed linked files.
        let native = dir.join("audio.drawer");
        crate::files::save(&mut d, &native).map_err(|e| e.to_string())?;
        let saved = d.snapshot();
        let mut opened = crate::files::open(&native, B).map_err(|e| e.to_string())?;
        check(
            opened.snapshot() == saved && opened.scene.timing.audio[0].sound.mute,
            "Native audio settings / source / PCM roundtrip",
        )?;
        let mut old = std::fs::read(&native).map_err(|e| e.to_string())?;
        old[8..12].copy_from_slice(&101u32.to_le_bytes());
        let spoof = dir.join("old.drawer");
        std::fs::write(&spoof, old).map_err(|e| e.to_string())?;
        check(
            crate::files::open(&spoof, B).is_err(),
            "Native101 accepted mixing / local-link metadata",
        )?;
        let link = opened.scene.timing.audio[0].sound.source.clone().unwrap();
        std::fs::remove_file(&link.path).map_err(|e| e.to_string())?;
        check(
            core::source_status(&link).contains("unavailable")
                && core::refresh(&opened, id, true, B).is_err(),
            "Missing linked file not reported / refreshed",
        )?;
        check(
            crate::files::open(&native, B)
                .map_err(|e| e.to_string())?
                .animation
                .as_ref()
                .unwrap()
                .audio[0]
                .samples
                == source,
            "Missing link lost embedded PCM",
        )?;
        std::fs::write(&link.path, wav(20000, 3200)).map_err(|e| e.to_string())?;
        check(
            core::source_status(&link).contains("changed")
                && core::refresh(&opened, id, false, B).is_err(),
            "Linked change / refresh confirmation",
        )?;
        let state = core::refresh(&opened, id, true, B)?;
        opened.commit_state(state)?;
        check(
            opened.animation.as_ref().unwrap().audio[0].samples[0] == 20000
                && opened.scene.timing.audio[0].sound.mute
                && opened.scene.timing.audio[0].sound.gain == 100,
            "Refresh lost cached PCM / mixing parameters",
        )?;
        let state = core::edit(&opened, id, "Clip".into(), 4, 400, false, B)?;
        opened.commit_state(state)?;
        let loud = Arc::new(mix::Plan::capture(&opened, 48000, 1)?).stream(4, 0, 24, false)?;
        let (pcm, clipped) = loud.block(1, 3)?;
        check(
            pcm == [32767; 3] && clipped == 3,
            "Clipping count / mono output",
        )?;
        // FPS conversion and time splicing retain volume, mute and provenance on every split.
        let sound = opened.scene.timing.audio[0].sound.clone();
        let request = crate::timeline_time_edit::Request::Fps {
            fps: crate::animation::Fps {
                numerator: 30000,
                denominator: 1001,
            },
            keep_time: true,
            labels_linked: true,
        };
        let prepared =
            crate::timeline_time_edit::prepare(&opened, request, &Default::default(), B)?;
        check(prepared.ready(), "Unexpected audio FPS conflict")?;
        prepared.commit(&mut opened)?;
        check(
            opened.scene.timing.audio[0].sound == sound,
            "FPS conversion lost audio settings",
        )?;
        let start_frame = opened.animation.as_ref().unwrap().audio[0].start;
        let request = crate::timeline_time_edit::Request::Splice {
            at: start_frame + 2,
            count: 1,
            remove: false,
            scope: crate::timeline_time_edit::Scope {
                all: true,
                ..Default::default()
            },
            split: true,
            labels_linked: true,
        };
        let prepared =
            crate::timeline_time_edit::prepare(&opened, request, &Default::default(), B)?;
        check(prepared.ready(), "Unexpected audio split conflict")?;
        prepared.commit(&mut opened)?;
        check(
            opened.scene.timing.audio.len() == 2
                && opened.scene.timing.audio.iter().all(|c| c.sound == sound),
            "Split failed to inherit audio provenance / gain",
        )?;
        let selected = opened.scene.timing.audio[0].id;
        let before = opened.snapshot();
        let state = core::remove(&opened, selected, B)?;
        opened.commit_state(state)?;
        check(
            opened.animation.as_ref().unwrap().audio.len() == 1
                && opened.scene.timing.audio[0].index == 0
                && opened.undo()
                && opened.snapshot() == before,
            "Audio remove / reindex / Undo",
        )?;
        let mut locked = opened.snapshot();
        locked.scene.timing.audio[0].locked = true;
        opened.commit_state(locked)?;
        check(
            core::edit(&opened, selected, "Locked".into(), 0, 100, false, B).is_err()
                && core::remove(&opened, selected, B).is_err()
                && core::refresh(&opened, selected, true, B).is_err(),
            "Locked audio edit accepted",
        )?;
        let float = dir.join("float.wav");
        std::fs::write(&float, encoded(3, 32, &0.5f32.to_le_bytes())).map_err(|e| e.to_string())?;
        let input = core::load(&float, None, false)?;
        check(
            core::insert(&d, &input, 0, false, B).is_err()
                && core::insert(&d, &input, 0, true, B).is_ok(),
            "Quantization acknowledgement bypassed",
        )?;
        check(
            core::load(Path::new("https://example.invalid/audio.wav"), None, false).is_err(),
            "Remote audio source accepted",
        )?;
        // Exact rational boundaries have no accumulated frame drift, including start-in-middle loops.
        let mut clock = crate::keyframes_verify::fixture();
        let mut s = clock.snapshot();
        s.animation.as_mut().unwrap().fps = crate::animation::Fps {
            numerator: 30000,
            denominator: 1001,
        };
        s.animation.as_mut().unwrap().frames = 10000;
        clock.commit_state(s)?;
        let plan = Arc::new(mix::Plan::capture(&clock, 48000, 2)?);
        let stream = plan.clone().stream(0, 0, 10000, false)?;
        for frame in 0..10000 {
            let n = mix::boundary(frame, plan.fps, 48000);
            check(
                stream.position(n).frame == frame,
                "Rational sample/frame drift",
            )?;
        }
        let looped = plan.clone().stream(3, 2, 7, true)?;
        let head = mix::boundary(7, plan.fps, 48000) - mix::boundary(3, plan.fps, 48000);
        let length = mix::boundary(7, plan.fps, 48000) - mix::boundary(2, plan.fps, 48000);
        check(
            looped.position(head).frame == 2 && looped.position(head + length * 10000).frame == 2,
            "Loop boundary / accumulated drift",
        )?;
        let dest = mix::Destination::capture(&stream, &dir.join("mid-cancel.wav"), false)?;
        let cancel = Arc::new(AtomicBool::new(false));
        let progress = Arc::new(AtomicU64::new(0));
        let c = cancel.clone();
        let p = progress.clone();
        let worker = std::thread::spawn(move || mix::write(&stream, &dest, &c, &p));
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while progress.load(Ordering::Relaxed) == 0 && std::time::Instant::now() < deadline {
            std::thread::yield_now();
        }
        cancel.store(true, Ordering::Relaxed);
        check(
            worker.join().map_err(|_| "WAV worker panic")?.is_err()
                && !dir.join("mid-cancel.wav").exists(),
            "Mid-render WAV cancellation published partial output",
        )?;
        check(
            !std::fs::read_dir(&dir)
                .map_err(|e| e.to_string())?
                .any(|e| e.is_ok_and(|e| e.file_name().to_string_lossy().ends_with(".tmp"))),
            "WAV temporary file leaked",
        )?;
        check(
            mix::wav_header(u32::MAX as u64, 48000, 2).is_err(),
            "RIFF output overflow accepted",
        )?;
        Ok(())
    })();
    std::fs::remove_dir_all(&dir).map_err(|e| e.to_string())?;
    result
}
#[cfg(test)]
mod tests {
    #[test]
    fn audio_decode_edit_pcm_links_native_mix_export_cancel_and_rational_clock() {
        let root = std::env::temp_dir().join(format!("drawer-audio-core-{}", std::process::id()));
        std::fs::create_dir(&root).unwrap();
        let result = super::verify(&root);
        std::fs::remove_dir(&root).unwrap();
        result.unwrap();
    }
}
