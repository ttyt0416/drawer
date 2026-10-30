//! Local optional decoder. The distribution contains no codec executable and never downloads one.
use crate::{
    animation::{self, Audio, Cell, Clip, Fps},
    document::MAX_DIMENSION,
};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

#[derive(Clone)]
pub struct Decoder {
    pub ffmpeg: PathBuf,
    pub ffprobe: PathBuf,
}
impl Decoder {
    pub fn discover() -> Option<Self> {
        std::env::split_paths(&std::env::var_os("PATH")?)
            .find_map(|dir| Self::at(dir.join("ffmpeg.exe")).ok())
    }
    pub fn at(ffmpeg: PathBuf) -> Result<Self, String> {
        let ffmpeg = ffmpeg
            .canonicalize()
            .map_err(|e| format!("Local FFmpeg missing: {e}"))?;
        let ffprobe = ffmpeg
            .parent()
            .ok_or("Missing codec directory")?
            .join("ffprobe.exe");
        if !ffmpeg.is_file() || !ffprobe.is_file() {
            return Err(
                "Select a local FFmpeg executable with FFprobe in the same directory".into(),
            );
        }
        Ok(Self { ffmpeg, ffprobe })
    }
    pub fn verify(&self, cancel: &AtomicBool) -> Result<String, String> {
        let mut versions = Vec::new();
        for (path, name) in [(&self.ffmpeg, "ffmpeg"), (&self.ffprobe, "ffprobe")] {
            let result = run(Command::new(path).arg("-version"), 65536, cancel)?;
            let text = String::from_utf8(result).map_err(|_| "Invalid codec version response")?;
            let version = text.split_whitespace().nth(2).unwrap_or_default();
            if !text.starts_with(&format!("{name} version "))
                || !(version == "7.1" || version.starts_with("7.1-") || version.starts_with("7.1."))
            {
                return Err("This local codec version has no verified import support. Choose FFmpeg/FFprobe 7.1.".into());
            }
            versions.push(text.lines().next().unwrap_or_default().to_owned());
        }
        Ok(versions.join("; "))
    }
}
#[derive(Clone)]
pub struct Source {
    pub path: PathBuf,
    pub bytes: Arc<Vec<u8>>,
    extension: String,
}
impl Source {
    pub fn read(path: &Path, budget: usize) -> Result<Self, String> {
        let path = path.canonicalize().map_err(|e| e.to_string())?;
        let extension = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or_default()
            .to_lowercase();
        if !["mp4", "mov", "mkv", "webm", "avi"].contains(&extension.as_str()) {
            return Err("Verified movie containers: MP4/MOV, MKV/WebM and AVI".into());
        }
        let mut file = fs::File::open(&path).map_err(|e| e.to_string())?;
        let length = file.metadata().map_err(|e| e.to_string())?.len();
        if length == 0 || length > 64 * 1024 * 1024 || length > budget as u64 {
            return Err("Movie snapshot exceeds the 64 MiB input / available memory limit".into());
        }
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(length as usize)
            .map_err(|e| e.to_string())?;
        Read::by_ref(&mut file)
            .take(length + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        if bytes.len() != length as usize {
            return Err("Movie changed while it was being read".into());
        }
        Ok(Self {
            path,
            bytes: Arc::new(bytes),
            extension,
        })
    }
    fn temporary(&self) -> Result<Temporary, String> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "drawer-movie-{}-{nonce}-{}.{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed),
            self.extension
        ));
        let mut file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&path)
            .map_err(|e| e.to_string())?;
        let temp = Temporary(path);
        file.write_all(&self.bytes)
            .and_then(|_| file.sync_all())
            .map_err(|e| e.to_string())?;
        Ok(temp)
    }
}
struct Temporary(PathBuf);
impl Drop for Temporary {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}
#[derive(Clone, Debug)]
pub struct Info {
    pub width: u32,
    pub height: u32,
    pub times: Vec<f64>,
    pub fps: String,
    pub has_audio: bool,
    pub audio_codec: Option<String>,
    pub audio_channels: u16,
    pub video_start: f64,
    pub format_start: f64,
}
impl Info {
    pub fn last_time(&self) -> f64 {
        *self.times.last().unwrap_or(&0.0)
    }
}
fn input(command: &mut Command, path: &Path) {
    command
        .args([
            "-v",
            "error",
            "-protocol_whitelist",
            "file,pipe",
            "-format_whitelist",
            "mov,matroska,webm,avi",
            "-i",
        ])
        .arg(path);
}
pub fn inspect(source: &Source, decoder: &Decoder, cancel: &AtomicBool) -> Result<Info, String> {
    decoder.verify(cancel)?;
    let file = source.temporary()?;
    let mut probe = Command::new(&decoder.ffprobe);
    input(&mut probe, &file.0);
    probe.args([
        "-show_streams",
        "-show_format",
        "-show_entries",
        "stream=codec_type,codec_name,channels,pix_fmt,color_transfer,width,height,avg_frame_rate:format=start_time",
        "-of",
        "json",
    ]);
    let json: serde_json::Value = serde_json::from_slice(&run(&mut probe, 1024 * 1024, cancel)?)
        .map_err(|e| e.to_string())?;
    let streams = json["streams"].as_array().ok_or("Missing movie streams")?;
    let video = streams
        .iter()
        .find(|v| v["codec_type"] == "video")
        .ok_or("No video stream in this file")?;
    let codec = video["codec_name"].as_str().unwrap_or_default();
    if !["h264", "ffv1", "rawvideo", "vp9"].contains(&codec) {
        return Err(format!(
            "No verified import support for the local video codec {codec}"
        ));
    }
    let format = video["pix_fmt"].as_str().unwrap_or_default();
    let pixel_support = match codec {
        "h264" | "vp9" => format == "yuv420p",
        "ffv1" | "rawvideo" => format == "bgra",
        _ => false,
    };
    let transfer = video["color_transfer"].as_str().unwrap_or("unknown");
    if !pixel_support || !["unknown", "bt709", "smpte170m", "iec61966-2-1"].contains(&transfer) {
        return Err(format!(
            "No verified import support for {codec}/{format}/{transfer}; convert locally to a supported 8-bit SDR movie"
        ));
    }
    let format_start = json["format"]["start_time"]
        .as_str()
        .and_then(|v| v.parse::<f64>().ok())
        .filter(|v| v.is_finite())
        .unwrap_or(0.0);
    let width = video["width"]
        .as_u64()
        .and_then(|v| u32::try_from(v).ok())
        .ok_or("Missing video width")?;
    let height = video["height"]
        .as_u64()
        .and_then(|v| u32::try_from(v).ok())
        .ok_or("Missing video height")?;
    if !(1..=MAX_DIMENSION).contains(&width)
        || !(1..=MAX_DIMENSION).contains(&height)
        || width as u64 * height as u64 * 4 > 64 * 1024 * 1024
    {
        return Err("Unsupported video dimensions".into());
    }
    let mut probe = Command::new(&decoder.ffprobe);
    input(&mut probe, &file.0);
    probe.args([
        "-select_streams",
        "v:0",
        "-show_frames",
        "-show_entries",
        "frame=best_effort_timestamp_time",
        "-of",
        "json",
    ]);
    let json: serde_json::Value =
        serde_json::from_slice(&run(&mut probe, 4 * 1024 * 1024, cancel)?)
            .map_err(|e| e.to_string())?;
    let frames = json["frames"]
        .as_array()
        .ok_or("Missing decoded frame timestamps")?;
    if frames.is_empty() || frames.len() > animation::MAX_FRAMES as usize {
        return Err(
            "The movie exceeds the 10,000 source-frame limit or has no decodable frames".into(),
        );
    }
    let mut times = Vec::new();
    for frame in frames {
        let value = frame["best_effort_timestamp_time"]
            .as_str()
            .and_then(|v| v.parse::<f64>().ok())
            .filter(|v| v.is_finite())
            .ok_or("The movie has unsupported frame timestamps")?;
        times.push(value);
    }
    let video_start = times[0];
    for time in &mut times {
        *time -= video_start;
    }
    if times.windows(2).any(|p| p[0] > p[1]) || times.iter().any(|t| *t < 0.0 || *t > 3600.0) {
        return Err("Unsupported movie timing or duration over one hour".into());
    }
    Ok(Info {
        width,
        height,
        times,
        fps: video["avg_frame_rate"].as_str().unwrap_or("unknown").into(),
        has_audio: streams.iter().any(|s| s["codec_type"] == "audio"),
        audio_codec: streams
            .iter()
            .find(|s| s["codec_type"] == "audio")
            .and_then(|s| s["codec_name"].as_str())
            .map(str::to_owned),
        audio_channels: streams
            .iter()
            .find(|s| s["codec_type"] == "audio")
            .and_then(|s| s["channels"].as_u64())
            .and_then(|n| u16::try_from(n).ok())
            .unwrap_or(0),
        video_start,
        format_start,
    })
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Options {
    pub start: f64,
    pub end: f64,
    pub fps: Fps,
    pub width: u32,
    pub height: u32,
    pub audio: bool,
    pub filter: crate::interpolation::Filter,
}
pub fn sampling(info: &Info, options: Options) -> Result<Vec<usize>, String> {
    if info.times.is_empty()
        || info.times.len() > animation::MAX_FRAMES as usize
        || info.times.iter().any(|t| !t.is_finite() || *t < 0.0)
        || info.times.windows(2).any(|t| t[0] > t[1])
        || !(1..=MAX_DIMENSION).contains(&info.width)
        || !(1..=MAX_DIMENSION).contains(&info.height)
    {
        return Err("Invalid source timing or dimensions".into());
    }
    if !options.start.is_finite()
        || !options.end.is_finite()
        || options.start < 0.0
        || options.end < options.start
        || options.end > info.last_time() + 1e-8
        || !options.fps.valid()
        || !(1..=MAX_DIMENSION).contains(&options.width)
        || !(1..=MAX_DIMENSION).contains(&options.height)
    {
        return Err("Invalid range, frame rate or output size".into());
    }
    let count = ((options.end - options.start) * options.fps.value() + 1e-8).floor() + 1.0;
    if count > animation::MAX_FRAMES as f64 {
        return Err("The selected range exceeds the 10,000 sampled-frame limit".into());
    }
    Ok((0..count as usize)
        .map(|n| {
            let time = options.start + n as f64 / options.fps.value();
            info.times
                .partition_point(|t| *t <= time + 1e-8)
                .saturating_sub(1)
        })
        .collect())
}
pub fn decode(
    source: &Source,
    decoder: &Decoder,
    info: &Info,
    options: Options,
    budget: usize,
    cancel: &AtomicBool,
) -> Result<Clip, String> {
    let mapping = sampling(info, options)?;
    if cancel.load(Ordering::Relaxed) {
        return Err("Movie import canceled".into());
    }
    let first = mapping[0];
    let last = *mapping.last().unwrap();
    let pixels = options.width as usize * options.height as usize;
    let raw_pixels = info.width as usize * info.height as usize;
    let raw_frame_bytes = raw_pixels.checked_mul(4).ok_or("Video size overflow")?;
    let frame_bytes = pixels.checked_mul(4).ok_or("Video size overflow")?;
    let limit = raw_frame_bytes
        .checked_mul(last - first + 1)
        .ok_or("Video size overflow")?;
    let output_bytes = frame_bytes
        .checked_mul(mapping.len())
        .ok_or("Video size overflow")?;
    if limit
        .saturating_add(output_bytes)
        .saturating_add(source.bytes.len())
        .saturating_mul(2)
        > budget
    {
        return Err(
            "Decoded range exceeds the available memory budget; reduce its resolution or duration"
                .into(),
        );
    }
    let file = source.temporary()?;
    let mut command = Command::new(&decoder.ffmpeg);
    command.args(["-nostdin", "-noautorotate"]);
    input(&mut command, &file.0);
    command
        .args(["-map", "0:v:0", "-an", "-vf"])
        .arg(format!("select=between(n\\,{first}\\,{last})"))
        .args(["-fps_mode", "passthrough", "-frames:v"])
        .arg((last - first + 1).to_string())
        .args(["-pix_fmt", "rgba", "-f", "rawvideo", "pipe:1"]);
    let bytes = run(&mut command, limit, cancel)?;
    if bytes.len() != limit {
        return Err("The decoder returned an incomplete frame range; nothing was imported".into());
    }
    let mut frames: Vec<Arc<Vec<[u8; 4]>>> = Vec::new();
    for frame in bytes.chunks_exact(raw_frame_bytes) {
        let input = frame.as_chunks::<4>().0;
        if info.width == options.width && info.height == options.height {
            frames.push(Arc::new(input.to_vec()));
            continue;
        }
        let mut output = Vec::new();
        output
            .try_reserve_exact(pixels)
            .map_err(|e| e.to_string())?;
        let step = [
            info.width as f64 / options.width as f64,
            info.height as f64 / options.height as f64,
        ];
        for y in 0..options.height {
            if cancel.load(Ordering::Relaxed) {
                return Err("Movie import canceled".into());
            }
            for x in 0..options.width {
                let point = [
                    (x as f64 + 0.5) * step[0] - 0.5,
                    (y as f64 + 0.5) * step[1] - 0.5,
                ];
                output.push(crate::interpolation::sample(
                    point,
                    crate::interpolation::box_footprint(point, step),
                    options.filter,
                    |x, y| {
                        input[(y.clamp(0, info.height as i64 - 1) as usize * info.width as usize)
                            + x.clamp(0, info.width as i64 - 1) as usize]
                    },
                ));
            }
        }
        frames.push(Arc::new(output));
    }
    let mut cells = Vec::new();
    let mut source_times = Vec::new();
    for (n, index) in mapping.into_iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            return Err("Movie import canceled".into());
        }
        cells.push(Cell {
            name: format!("Movie cell {}", n + 1),
            width: options.width,
            height: options.height,
            offset: [0; 2],
            pixels: frames[index - first].clone(),
        });
        source_times.push(info.times[index]);
    }
    let audio = if options.audio && info.has_audio {
        if !info
            .audio_codec
            .as_deref()
            .is_some_and(|codec| ["pcm_s16le", "aac"].contains(&codec))
        {
            return Err("No verified audio import support for this movie; import video only or convert audio locally to AAC / signed 16-bit PCM".into());
        }
        if !(1..=8).contains(&info.audio_channels) {
            return Err("Unsupported audio channel count".into());
        }
        let duration = cells.len() as f64 / options.fps.value();
        let cap = (duration * 48000.0).ceil() as usize * 2 * info.audio_channels as usize;
        if cap > 64 * 1024 * 1024
            || cap
                .saturating_add(output_bytes)
                .saturating_add(source.bytes.len())
                .saturating_mul(2)
                > budget
        {
            return Err("Audio exceeds the import memory budget".into());
        }
        let mut command = Command::new(&decoder.ffmpeg);
        command.args(["-nostdin"]);
        input(&mut command, &file.0);
        command
            .arg("-ss")
            .arg(
                (options.start + info.video_start - info.format_start)
                    .max(0.0)
                    .to_string(),
            )
            .arg("-t")
            .arg(duration.to_string())
            .args([
                "-map",
                "0:a:0",
                "-vn",
                "-ac",
                &info.audio_channels.to_string(),
                "-ar",
                "48000",
                "-f",
                "s16le",
                "pipe:1",
            ]);
        let bytes = run(&mut command, cap, cancel)?;
        if bytes.is_empty() || !bytes.len().is_multiple_of(2) {
            return Err("Unsupported or empty audio in the selected range".into());
        }
        let samples = bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|b| i16::from_le_bytes(*b))
            .collect::<Vec<_>>();
        Some(Audio {
            name: "Imported movie audio".into(),
            start: 0,
            rate: 48000,
            channels: info.audio_channels,
            sample_count: samples.len() as u32,
            samples: Arc::new(samples),
        })
    } else {
        None
    };
    Ok(Clip {
        fps: options.fps,
        cells,
        source_times,
        audio,
    })
}
pub(crate) fn run(
    command: &mut Command,
    limit: usize,
    cancel: &AtomicBool,
) -> Result<Vec<u8>, String> {
    if cancel.load(Ordering::Relaxed) {
        return Err("Movie import canceled".into());
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Cannot run the local decoder: {e}"))?;
    let output = child.stdout.take().unwrap();
    let errors = child.stderr.take().unwrap();
    let exceeded = Arc::new(AtomicBool::new(false));
    let flag = exceeded.clone();
    let stdout = thread::spawn(move || {
        let mut bytes = Vec::new();
        let result = output.take(limit as u64 + 1).read_to_end(&mut bytes);
        if bytes.len() > limit {
            flag.store(true, Ordering::Relaxed);
        }
        result.map(|_| bytes)
    });
    let flag = exceeded.clone();
    let stderr = thread::spawn(move || {
        let mut bytes = Vec::new();
        let result = errors.take(65537).read_to_end(&mut bytes);
        if bytes.len() > 65536 {
            flag.store(true, Ordering::Relaxed);
        }
        result.map(|_| bytes)
    });
    let started = Instant::now();
    let mut failure = None;
    let status = loop {
        if cancel.load(Ordering::Relaxed)
            || exceeded.load(Ordering::Relaxed)
            || started.elapsed() > Duration::from_secs(120)
        {
            failure = Some(if cancel.load(Ordering::Relaxed) {
                "Movie import canceled"
            } else {
                "Local decoder exceeded its output or time limit"
            });
            let _ = child.kill();
            break child.wait();
        }
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) => thread::sleep(Duration::from_millis(10)),
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                break Err(e);
            }
        }
    };
    let output = stdout
        .join()
        .map_err(|_| "Decoder output reader failed")?
        .map_err(|e| e.to_string())?;
    let errors = stderr
        .join()
        .map_err(|_| "Decoder error reader failed")?
        .map_err(|e| e.to_string())?;
    if let Some(failure) = failure {
        return Err(failure.into());
    }
    if output.len() > limit || errors.len() > 65536 {
        return Err("Local decoder output exceeds the limit".into());
    }
    let status = status.map_err(|e| e.to_string())?;
    if !status.success() {
        return Err(format!(
            "Unsupported movie or decoder failure: {}",
            String::from_utf8_lossy(&errors)
        ));
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{document::DEFAULT_DOCUMENT_BUDGET, interpolation::Filter, test_support::TempDir};
    fn decoder() -> Decoder {
        Decoder::discover()
            .expect("Movie integration tests require the locally installed FFmpeg/FFprobe 7.1")
    }
    fn fixture(
        root: &Path,
        decoder: &Decoder,
        name: &str,
        codec: &str,
        format: &str,
        timing: (bool, bool),
        count: usize,
    ) -> PathBuf {
        let (vfr, audio) = timing;
        let raw = root.join(format!("{name}.rgba"));
        let mut bytes = Vec::new();
        for (n, color) in [[255, 0, 0, 255], [0, 255, 0, 255], [0, 0, 255, 255]]
            .iter()
            .take(count)
            .enumerate()
        {
            let mut color = *color;
            if codec == "ffv1" {
                color[3] = [128, 0, 255][n];
            }
            for _ in 0..16 * 16 {
                bytes.extend_from_slice(&color);
            }
        }
        fs::write(&raw, bytes).unwrap();
        let output = root.join(format!("{name}.{format}"));
        let mut command = Command::new(&decoder.ffmpeg);
        command
            .args([
                "-v",
                "error",
                "-nostdin",
                "-f",
                "rawvideo",
                "-pixel_format",
                "rgba",
                "-video_size",
                "16x16",
                "-framerate",
                "10",
                "-i",
            ])
            .arg(raw);
        if audio {
            command.args([
                "-f",
                "lavfi",
                "-i",
                "aevalsrc=sin(2*PI*440*t)|sin(2*PI*660*t):s=48000:d=1",
                "-ac",
                "2",
                "-c:a",
                if format == "mp4" || format == "mov" {
                    "aac"
                } else {
                    "pcm_s16le"
                },
            ]);
        }
        if vfr {
            command.args([
                "-vf",
                r"settb=1/1000,setpts=if(eq(N\,0)\,0\,if(eq(N\,1)\,150\,600))",
                "-fps_mode",
                "passthrough",
                "-enc_time_base",
                "1:1000",
            ]);
        }
        command.args(["-c:v", codec]);
        if codec == "libx264" || codec == "libvpx-vp9" {
            command.args(["-pix_fmt", "yuv420p"]);
        } else if codec == "rawvideo" {
            command.args(["-pix_fmt", "bgra"]);
        }
        command.arg(output.clone());
        run(&mut command, 0, &AtomicBool::new(false)).unwrap();
        output
    }
    fn options(info: &Info) -> Options {
        Options {
            start: 0.0,
            end: info.last_time(),
            fps: Fps {
                numerator: 10,
                denominator: 1,
            },
            width: 16,
            height: 16,
            audio: false,
            filter: Filter::Nearest,
        }
    }
    #[test]
    #[ignore = "Requires locally installed FFmpeg/FFprobe 7.1"]
    fn real_decoder_vfr_audio_endpoints_single_frame_containers_preserve_original_and_cancel() {
        let root = TempDir::new();
        let decoder = decoder();
        let flag = AtomicBool::new(false);
        let path = fixture(
            &root.0,
            &decoder,
            "variable",
            "ffv1",
            "mkv",
            (true, true),
            3,
        );
        let original = fs::read(&path).unwrap();
        let source = Source::read(&path, DEFAULT_DOCUMENT_BUDGET).unwrap();
        let info = inspect(&source, &decoder, &flag).unwrap();
        assert_eq!(info.times, [0.0, 0.15, 0.6]);
        assert!(info.has_audio);
        let mut settings = options(&info);
        settings.audio = true;
        let clip = decode(
            &source,
            &decoder,
            &info,
            settings,
            DEFAULT_DOCUMENT_BUDGET,
            &flag,
        )
        .unwrap();
        assert_eq!(clip.source_times, [0.0, 0.0, 0.15, 0.15, 0.15, 0.15, 0.6]);
        assert_eq!(clip.cells[0].pixels[0], [255, 0, 0, 128]);
        assert_eq!(clip.cells[2].pixels[0], [0, 255, 0, 0]);
        assert_eq!(clip.cells[6].pixels[0], [0, 0, 255, 255]);
        assert!(!clip.audio.as_ref().unwrap().samples.is_empty());
        assert_eq!(clip.audio.as_ref().unwrap().channels, 2);
        let audio = clip.audio.as_ref().unwrap();
        assert!((audio.samples.len() as f64 / 2.0 / 48000.0 - 0.7).abs() < 1.0 / 48000.0);
        assert!(
            audio
                .samples
                .as_chunks::<2>()
                .0
                .iter()
                .any(|pair| pair[0] != pair[1])
        );
        assert!(
            clip.document(DEFAULT_DOCUMENT_BUDGET)
                .unwrap()
                .animation
                .unwrap()
                .valid()
        );
        settings.start = 0.6;
        settings.end = 0.6;
        settings.audio = false;
        assert_eq!(
            decode(
                &source,
                &decoder,
                &info,
                settings,
                DEFAULT_DOCUMENT_BUDGET,
                &flag
            )
            .unwrap()
            .cells
            .len(),
            1
        );
        assert!(decode(&source, &decoder, &info, options(&info), 1, &flag).is_err());
        let cancel = Arc::new(AtomicBool::new(false));
        let worker = cancel.clone();
        let stop = thread::spawn(move || {
            thread::sleep(Duration::from_millis(20));
            worker.store(true, Ordering::Relaxed);
        });
        settings = options(&info);
        settings.width = 512;
        settings.height = 512;
        let result = decode(
            &source,
            &decoder,
            &info,
            settings,
            DEFAULT_DOCUMENT_BUDGET,
            &cancel,
        );
        stop.join().unwrap();
        assert!(result.unwrap_err().contains("canceled"));
        assert_eq!(fs::read(&path).unwrap(), original);
        for (name, codec, container, count) in [
            ("one", "rawvideo", "avi", 1),
            ("mp4", "libx264", "mp4", 3),
            ("mov", "libx264", "mov", 3),
            ("webm", "libvpx-vp9", "webm", 3),
        ] {
            let path = fixture(
                &root.0,
                &decoder,
                name,
                codec,
                container,
                (false, name == "mp4" || name == "mov"),
                count,
            );
            let source = Source::read(&path, DEFAULT_DOCUMENT_BUDGET).unwrap();
            let info = inspect(&source, &decoder, &flag).unwrap();
            assert_eq!(info.times.len(), count);
            let mut settings = options(&info);
            settings.audio = true;
            settings.width = 8;
            settings.height = 8;
            settings.filter = Filter::Area;
            let clip = decode(
                &source,
                &decoder,
                &info,
                settings,
                DEFAULT_DOCUMENT_BUDGET,
                &flag,
            )
            .unwrap();
            assert_eq!(clip.cells.len(), count);
            assert_eq!(clip.cells[0].pixels.len(), 64);
            assert_eq!(clip.audio.is_some(), name == "mp4" || name == "mov");
        }
        let damaged = root.0.join("bad.mkv");
        fs::write(&damaged, [0u8; 128]).unwrap();
        let source = Source::read(&damaged, 1000).unwrap();
        assert!(inspect(&source, &decoder, &flag).is_err());
        assert_eq!(fs::read(damaged).unwrap(), [0u8; 128]);
        assert!(Decoder::at(root.0.join("missing.exe")).is_err());
    }
    #[test]
    fn frame_limits_bad_ranges_empty_timestamps_and_pre_cancellation_are_rejected() {
        let info = Info {
            width: 1,
            height: 1,
            times: vec![0.0, 0.15, 0.6],
            fps: "variable".into(),
            has_audio: false,
            audio_codec: None,
            audio_channels: 0,
            video_start: 0.0,
            format_start: 0.0,
        };
        let mut settings = options(&info);
        assert_eq!(sampling(&info, settings).unwrap(), [0, 0, 1, 1, 1, 1, 2]);
        settings.end = 0.61;
        assert!(sampling(&info, settings).is_err());
        settings.end = f64::NAN;
        assert!(sampling(&info, settings).is_err());
        settings = options(&info);
        settings.fps.denominator = 0;
        assert!(sampling(&info, settings).is_err());
        let long = Info {
            times: vec![0.0, 3600.0],
            ..info.clone()
        };
        settings = options(&long);
        assert!(sampling(&long, settings).is_err());
        let empty = Info {
            times: Vec::new(),
            ..info
        };
        assert!(sampling(&empty, settings).is_err());
        let flag = AtomicBool::new(true);
        assert!(
            run(&mut Command::new("does-not-exist"), 1, &flag)
                .unwrap_err()
                .contains("canceled")
        );
    }
}
