//! Only locally self-tested encoder/container/pixel combinations are offered.
use crate::{
    animation::Fps,
    animation_export::{Snapshot, Stage},
    movie::Decoder,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    io::Read,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicBool, AtomicU64},
};
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Codec {
    #[default]
    Ffv1,
    H264,
}
impl Codec {
    pub fn label(self) -> &'static str {
        match self {
            Self::Ffv1 => "FFV1 · MKV · lossless BGRA + PCM",
            Self::H264 => "H.264 · MP4 · opaque 4:2:0 + AAC",
        }
    }
    pub fn extension(self) -> &'static str {
        match self {
            Self::Ffv1 => "mkv",
            Self::H264 => "mp4",
        }
    }
    pub fn notice(self) -> &'static str {
        match self {
            Self::Ffv1 => {
                "FFV1/MKV: lossless8-bit BGRA alpha; sRGB conversion, no embedded ICC;48kHz PCM sound. MKV timestamps round to1ms."
            }
            Self::H264 => {
                "H.264/MP4: alpha flattened onto selected matte; lossy8-bit4:2:0 sRGB/SDR conversion, no ICC; AAC sound is lossy."
            }
        }
    }
    fn encoder(self) -> &'static str {
        match self {
            Self::Ffv1 => "ffv1",
            Self::H264 => "libx264",
        }
    }
    fn pixel(self) -> &'static str {
        match self {
            Self::Ffv1 => "bgra",
            Self::H264 => "yuv420p",
        }
    }
    fn mux(self) -> &'static str {
        match self {
            Self::Ffv1 => "matroska",
            Self::H264 => "mp4",
        }
    }
}
#[derive(Clone)]
pub struct Verified {
    pub decoder: Decoder,
    pub codecs: Vec<Codec>,
    pub version: String,
    hashes: [[u8; 32]; 2],
}
fn hash(path: &Path) -> Result<[u8; 32], String> {
    if !crate::font_library::local(path) {
        return Err("Encoder must be local".into());
    }
    let mut f = std::fs::File::open(path).map_err(|e| e.to_string())?;
    if f.metadata().map_err(|e| e.to_string())?.len() > 512 * 1024 * 1024 {
        return Err("Encoder executable exceeds512MiB".into());
    }
    let mut hash = Sha256::new();
    let mut buffer = [0; 65536];
    loop {
        let n = f.read(&mut buffer).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
    }
    Ok(hash.finalize().into())
}
impl Verified {
    pub fn description(&self) -> String {
        format!(
            "{}; {}; verified={:?}; SHA256={:x?}",
            self.decoder.ffmpeg.display(),
            self.version,
            self.codecs,
            self.hashes
        )
    }
    fn unchanged(&self) -> Result<(), String> {
        if [hash(&self.decoder.ffmpeg)?, hash(&self.decoder.ffprobe)?] != self.hashes {
            return Err("Local encoder changed; verify it again".into());
        }
        Ok(())
    }
}
#[allow(clippy::too_many_arguments)]
fn command(
    decoder: &Decoder,
    codec: Codec,
    pattern: &Path,
    first: u32,
    count: u32,
    fps: Fps,
    wav: Option<&Path>,
    out: &Path,
) -> Command {
    let mut c = Command::new(&decoder.ffmpeg);
    c.args([
        "-nostdin",
        "-v",
        "error",
        "-protocol_whitelist",
        "file,pipe",
        "-framerate",
    ])
    .arg(format!("{}/{}", fps.numerator, fps.denominator))
    .arg("-start_number")
    .arg(first.to_string())
    .arg("-i")
    .arg(pattern);
    if let Some(wav) = wav {
        c.args(["-protocol_whitelist", "file,pipe", "-i"]).arg(wav);
    }
    c.args(["-map", "0:v:0", "-frames:v"])
        .arg(count.to_string())
        .args([
            "-c:v",
            codec.encoder(),
            "-pix_fmt",
            codec.pixel(),
            "-threads",
            "1",
        ]);
    match codec {
        Codec::Ffv1 => {
            c.args(["-level", "3"]);
        }
        Codec::H264 => {
            c.args([
                "-preset",
                "medium",
                "-crf",
                "18",
                "-vf",
                "scale=in_range=full:out_range=tv:out_color_matrix=bt709",
                "-color_range",
                "tv",
                "-colorspace",
                "bt709",
                "-color_primaries",
                "bt709",
                "-color_trc",
                "iec61966-2-1",
            ]);
        }
    }
    if wav.is_some() {
        c.args([
            "-map",
            "1:a:0",
            "-c:a",
            if codec == Codec::Ffv1 {
                "pcm_s16le"
            } else {
                "aac"
            },
            "-ar",
            "48000",
            "-ac",
            "2",
        ]);
    } else {
        c.arg("-an");
    }
    c.args([
        "-map_metadata",
        "-1",
        "-fflags",
        "+bitexact",
        "-flags:v",
        "+bitexact",
        "-fs",
        "2147483648",
        "-f",
        codec.mux(),
        "-n",
    ])
    .arg(out);
    c
}
pub fn discover() -> Option<Decoder> {
    actual_decoder(Decoder::discover()?).ok()
}
fn actual_decoder(decoder: Decoder) -> Result<Decoder, String> {
    let length = std::fs::metadata(&decoder.ffmpeg)
        .map_err(|e| e.to_string())?
        .len();
    if length >= 65536 {
        return Ok(decoder);
    }
    if let Some(root) = decoder.ffmpeg.parent().and_then(Path::parent) {
        let target = root.join("lib/ffmpeg/tools/ffmpeg/bin/ffmpeg.exe");
        if target.is_file() && std::fs::metadata(&target).map_err(|e| e.to_string())?.len() >= 65536
        {
            return Decoder::at(target);
        }
    }
    Err("Select the actual local FFmpeg executable instead of a small launcher/wrapper".into())
}
pub fn verify(decoder: Decoder, cancel: &AtomicBool) -> Result<Verified, String> {
    let decoder = actual_decoder(decoder)?;
    let version = decoder.verify(cancel)?;
    let hashes = [hash(&decoder.ffmpeg)?, hash(&decoder.ffprobe)?];
    let available = String::from_utf8(crate::movie::run(
        Command::new(&decoder.ffmpeg).args(["-v", "error", "-encoders"]),
        1024 * 1024,
        cancel,
    )?)
    .map_err(|e| e.to_string())?;
    let root = std::env::temp_dir()
        .canonicalize()
        .map_err(|e| e.to_string())?;
    let mut stage = Stage::new(&root)?;
    let fps = Fps {
        numerator: 30000,
        denominator: 1001,
    };
    let mut originals = Vec::new();
    for n in 0..2 {
        let data = (0..256)
            .map(|i| {
                if n == 0 {
                    [180, 40 + (i % 8) as u8, 30, 128]
                } else {
                    [20, 80, 160, 255]
                }
            })
            .collect::<Vec<_>>();
        let b = data.iter().flatten().copied().collect::<Vec<_>>();
        let p = stage.directory.join(format!("frame-{n:06}.png"));
        stage.files.push(p.clone());
        image::save_buffer_with_format(
            p,
            &b,
            16,
            16,
            image::ColorType::Rgba8,
            image::ImageFormat::Png,
        )
        .map_err(|e| e.to_string())?;
        originals.extend(b);
    }
    // Silent stereo PCM verifies both video and audio mux/decoder availability.
    let wav = stage.directory.join("test.wav");
    stage.files.push(wav.clone());
    let samples = crate::audio_mix::boundary(2, fps, 48000);
    let mut pcm = crate::audio_mix::wav_header(samples, 48000, 2)?;
    pcm.resize(pcm.len() + samples as usize * 4, 0);
    std::fs::write(&wav, pcm).map_err(|e| e.to_string())?;
    let mut codecs = vec![];
    for codec in [Codec::Ffv1, Codec::H264] {
        if !available
            .lines()
            .any(|l| l.split_whitespace().nth(1) == Some(codec.encoder()))
        {
            continue;
        }
        let out = stage.directory.join(format!("test.{}", codec.extension()));
        stage.files.push(out.clone());
        let test = (|| {
            let mut c = command(
                &decoder,
                codec,
                &stage.directory.join("frame-%06d.png"),
                0,
                2,
                fps,
                Some(&wav),
                &out,
            );
            crate::movie::run(&mut c, 65536, cancel)?;
            verify_streams(&decoder, codec, &out, 16, 16, 2, fps, true, cancel)?;
            let sound = crate::movie::run(
                Command::new(&decoder.ffmpeg)
                    .args(["-nostdin", "-v", "error", "-i"])
                    .arg(&out)
                    .args([
                        "-map", "0:a:0", "-f", "s16le", "-ac", "2", "-ar", "48000", "pipe:1",
                    ]),
                (samples as usize + 2048) * 4,
                cancel,
            )?;
            if codec == Codec::Ffv1 && sound.len() != samples as usize * 4
                || codec == Codec::H264
                    && !(samples as usize * 4..=(samples as usize + 2048) * 4)
                        .contains(&sound.len())
                || sound.iter().any(|b| *b != 0)
            {
                return Err(
                    "Encoder self-test audio sample length / silent PCM verification".into(),
                );
            }
            let bytes = decode(&decoder, &out, 256 * 4 * 2, cancel)?;
            if bytes.len() != originals.len() {
                return Err("Encoder self-test frame length".into());
            }
            if codec == Codec::Ffv1 && bytes != originals {
                return Err("FFV1 self-test did not preserve RGBA".into());
            }
            if codec == Codec::H264
                && bytes
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .zip(originals.as_chunks::<4>().0.iter())
                    .any(|(a, b)| {
                        a[3] != 255 || a[..3].iter().zip(&b[..3]).any(|(a, b)| a.abs_diff(*b) > 45)
                    })
            {
                return Err("H.264 self-test exceeds the SDR color error bound".into());
            }
            Ok::<_, String>(())
        })();
        if test.is_ok() {
            codecs.push(codec);
        }
        crate::book_io::check_cancel(cancel)?;
    }
    if codecs.is_empty() {
        return Err(
            "No local encoder combination passed the encode/decode test. Use PNG sequence + WAV."
                .into(),
        );
    }
    let v = Verified {
        decoder,
        version,
        codecs,
        hashes,
    };
    v.unchanged()?;
    Ok(v)
}
fn decode(d: &Decoder, p: &Path, limit: usize, cancel: &AtomicBool) -> Result<Vec<u8>, String> {
    crate::movie::run(
        Command::new(&d.ffmpeg)
            .args([
                "-nostdin",
                "-v",
                "error",
                "-protocol_whitelist",
                "file,pipe",
                "-i",
            ])
            .arg(p)
            .args([
                "-map", "0:v:0", "-f", "rawvideo", "-pix_fmt", "rgba", "-threads", "1", "pipe:1",
            ]),
        limit,
        cancel,
    )
}
#[allow(clippy::too_many_arguments)]
fn verify_streams(
    d: &Decoder,
    codec: Codec,
    p: &Path,
    width: u32,
    height: u32,
    count: u32,
    fps: Fps,
    audio: bool,
    cancel: &AtomicBool,
) -> Result<(), String> {
    let bytes = crate::movie::run(
        Command::new(&d.ffprobe)
            .args([
                "-v",
                "error",
                "-protocol_whitelist",
                "file,pipe",
                "-count_frames",
                "-show_streams",
                "-of",
                "json",
            ])
            .arg(p),
        1024 * 1024,
        cancel,
    )?;
    let v: serde_json::Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    let streams = v["streams"]
        .as_array()
        .ok_or("Missing output stream report")?;
    let video = streams
        .iter()
        .find(|v| v["codec_type"] == "video")
        .ok_or("Movie has no video")?;
    let rate = video["avg_frame_rate"]
        .as_str()
        .unwrap_or("")
        .split('/')
        .filter_map(|v| v.parse::<u64>().ok())
        .collect::<Vec<_>>();
    if video["width"].as_u64() != Some(width as u64)
        || video["height"].as_u64() != Some(height as u64)
        || video["nb_read_frames"]
            .as_str()
            .and_then(|v| v.parse::<u32>().ok())
            != Some(count)
        || video["codec_name"] != if codec == Codec::Ffv1 { "ffv1" } else { "h264" }
        || video["pix_fmt"] != codec.pixel()
        || rate.len() != 2
        || rate[0] * fps.denominator as u64 != rate[1] * fps.numerator as u64
    {
        return Err("Movie dimensions/count/codec/pixel/FPS verification failed".into());
    }
    let sound = streams.iter().find(|v| v["codec_type"] == "audio");
    if audio
        && !sound.is_some_and(|a| {
            a["channels"] == 2
                && a["sample_rate"] == "48000"
                && a["codec_name"]
                    == if codec == Codec::Ffv1 {
                        "pcm_s16le"
                    } else {
                        "aac"
                    }
        })
        || !audio && sound.is_some()
    {
        return Err("Movie audio stream verification failed".into());
    }
    Ok(())
}
pub(crate) fn encode(
    s: &Snapshot,
    frames: &[PathBuf],
    out: &Path,
    cancel: &AtomicBool,
) -> Result<(), String> {
    let encoder = s
        .encoder
        .as_ref()
        .ok_or("Verify a local encoder; use PNG+WAV when unavailable")?;
    encoder.unchanged()?;
    if !encoder.codecs.contains(&s.options.codec) {
        return Err("This encoder combination was not verified".into());
    }
    let parent = out.parent().ok_or("Missing private output directory")?;
    // Keep temporary audio in a separately owned stage, cleaned on every exit path.
    let mut stage = Stage::new(parent)?;
    let wav = if s.options.audio {
        let plan = std::sync::Arc::new(crate::audio_mix::Plan::capture(s.document(), 48000, 2)?);
        let stream = plan.stream(s.options.first, s.options.first, s.options.last + 1, false)?;
        let path = stage.directory.join("sound.wav");
        stage.files.push(path.clone());
        let dest = crate::audio_mix::Destination::capture(&stream, &path, false)?;
        crate::audio_mix::write(&stream, &dest, cancel, &AtomicU64::new(0))?;
        Some(path)
    } else {
        None
    };
    let pattern = frames[0]
        .parent()
        .ok_or("Missing captured frames")?
        .join("frame-%06d.png");
    let mut c = command(
        &encoder.decoder,
        s.options.codec,
        &pattern,
        s.options.first + 1,
        s.count(),
        s.fps(),
        wav.as_deref(),
        out,
    );
    crate::movie::run(&mut c, 65536, cancel)?;
    verify_streams(
        &encoder.decoder,
        s.options.codec,
        out,
        s.layout.width,
        s.layout.height,
        s.count(),
        s.fps(),
        s.options.audio,
        cancel,
    )?;
    // Actual encoding has no external input other than private captured PNG/WAV files.
    // Verify a full decode by null sink without holding all decoded movie frames in memory.
    crate::movie::run(
        Command::new(&encoder.decoder.ffmpeg)
            .args([
                "-nostdin",
                "-v",
                "error",
                "-xerror",
                "-protocol_whitelist",
                "file,pipe",
                "-i",
            ])
            .arg(out)
            .args(["-f", "null", "-"]),
        65536,
        cancel,
    )?;
    encoder.unchanged()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cancel_running_local_encoder_reaps_process_promptly() {
        let Some(d) = discover() else {
            return;
        };
        let flag = std::sync::Arc::new(AtomicBool::new(false));
        let cancel = flag.clone();
        let worker = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(100));
            cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        });
        let begin = std::time::Instant::now();
        let result = crate::movie::run(
            Command::new(&d.ffmpeg).args([
                "-nostdin",
                "-v",
                "error",
                "-re",
                "-f",
                "lavfi",
                "-i",
                "color=c=black:s=16x16:r=24",
                "-t",
                "10",
                "-f",
                "null",
                "-",
            ]),
            65536,
            &flag,
        );
        worker.join().unwrap();
        assert!(result.is_err());
        assert!(begin.elapsed() < std::time::Duration::from_secs(3));
    }
}
