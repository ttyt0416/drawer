//! Frozen local animation output. Publication never switches the live document frame.
use crate::{animation::Fps, document::Document};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    borrow::Cow,
    fs::{self, File},
    io::{BufReader, BufWriter, Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
};
static NEXT: AtomicU64 = AtomicU64::new(1);
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Format {
    #[default]
    PngSequence,
    Apng,
    Gif,
    Webp,
    Movie,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Options {
    pub first: u32,
    pub last: u32,
    pub format: Format,
    /// Zero is infinite, otherwise the number of complete plays (image animations only).
    pub plays: u32,
    pub view: crate::animation_render::View,
    pub codec: crate::movie_export::Codec,
    pub audio: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Preset {
    pub version: u8,
    pub options: Options,
    pub watermark: Option<PathBuf>,
    pub encoder: Option<PathBuf>,
}
impl Preset {
    pub fn decode(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() > 65536 {
            return Err("Output preset exceeds64KiB".into());
        }
        let p: Self = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
        if p.version != 1
            || p.options.first > p.options.last
            || p.options.last >= 10000
            || p.watermark
                .iter()
                .chain(p.encoder.iter())
                .any(|p| !crate::font_library::local(p))
        {
            return Err("Invalid / unsupported local output preset".into());
        }
        Ok(p)
    }
    pub fn encode(&self) -> Result<Vec<u8>, String> {
        let b = serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?;
        Self::decode(&b)?;
        Ok(b)
    }
}
#[derive(Serialize)]
struct Manifest {
    version: u8,
    revision: u64,
    width: u32,
    height: u32,
    origin: [i64; 2],
    fps: Fps,
    options: Options,
    rgba_bits: u8,
    watermark: Option<[u8; 32]>,
    sources: Vec<(u64, [u8; 32])>,
    profile_hash: [u8; 32],
    selection_hash: [u8; 32],
    material_hashes: Vec<(String, [u8; 32])>,
    files: Vec<String>,
}
#[derive(Clone)]
pub struct Snapshot {
    document: Box<Document>,
    pub options: Options,
    pub revision: u64,
    pub layout: crate::animation_render::Layout,
    pub(crate) watermark: Option<crate::animation_render::Watermark>,
    pub(crate) encoder: Option<crate::movie_export::Verified>,
    budget: usize,
    protected: Vec<PathBuf>,
}
impl Snapshot {
    pub fn document(&self) -> &Document {
        &self.document
    }
    pub fn count(&self) -> u32 {
        self.options.last - self.options.first + 1
    }
    pub fn fps(&self) -> Fps {
        self.document.animation.as_ref().unwrap().fps
    }
    pub fn watermark_hash(&self) -> Option<[u8; 32]> {
        self.watermark.as_ref().map(|w| w.hash)
    }
    fn manifest(&self, files: Vec<String>) -> Manifest {
        Manifest {
            version: 2,
            revision: self.revision,
            width: self.layout.width,
            height: self.layout.height,
            origin: self.layout.origin,
            fps: self.fps(),
            options: self.options,
            rgba_bits: 8,
            watermark: self.watermark_hash(),
            sources: self
                .document
                .scene
                .cels
                .definitions
                .iter()
                .map(|p| (p.object, p.source_hash()))
                .collect(),
            profile_hash: Sha256::digest(serde_json::to_vec(&self.document.color.profile).unwrap())
                .into(),
            selection_hash: Sha256::digest(
                serde_json::to_vec(&(
                    &self.document.scene.selection,
                    &self.document.scene.selection_mask,
                    self.document.scene.selection_translation,
                ))
                .unwrap(),
            )
            .into(),
            material_hashes: self
                .document
                .scene
                .file_objects
                .entries
                .iter()
                .map(|e| {
                    let mut h = Sha256::new();
                    h.update(serde_json::to_vec(&e.cache).unwrap());
                    if let Some(image) = e.cache.source.image() {
                        for p in image.pixels.iter() {
                            h.update(p);
                        }
                    }
                    (e.path.to_string_lossy().into_owned(), h.finalize().into())
                })
                .chain(
                    self.document
                        .scene
                        .objects
                        .iter()
                        .enumerate()
                        .filter_map(|(i, o)| {
                            if let crate::scene::PageObject::Material(m) = o {
                                let mut h = Sha256::new();
                                h.update(serde_json::to_vec(m).unwrap());
                                if let Some(image) = m.source.image() {
                                    for p in image.pixels.iter() {
                                        h.update(p);
                                    }
                                }
                                Some((format!("embedded-object-{i}"), h.finalize().into()))
                            } else {
                                None
                            }
                        }),
                )
                .collect(),
            files,
        }
    }
    pub fn warnings(&self) -> String {
        let timing = match self.options.format {
            Format::Gif => {
                "GIF: up to256 colors per frame; alpha<128 becomes transparent, other alpha becomes opaque; no ICC, converted to sRGB. Timing is cumulative-rounded to10ms; viewers may impose their own minimum delay."
            }
            Format::Webp => {
                "Lossless Animated WebP: RGBA alpha retained; timing cumulative-rounded to1ms. Viewer timing may differ."
            }
            Format::Apng => "APNG: RGBA alpha and exact rational frame delays retained.",
            Format::Movie => self.options.codec.notice(),
            Format::PngSequence => {
                "PNG sequence: RGBA alpha retained; exact rational FPS in animation.json. Completed frames are retained on cancellation."
            }
        };
        format!(
            "{timing} All output is8-bit SDR. {} {}",
            if self.options.view.icc {
                "ICC retained in PNG/APNG/WebP."
            } else {
                "ICC omitted; pixels converted to sRGB."
            },
            if self.options.format != Format::Movie {
                "Image animations contain no audio; use WAV output for sound."
            } else {
                "Audio uses48kHz stereo PCM from the captured mix when enabled."
            }
        )
    }
}
pub(crate) fn delay(fps: Fps) -> Result<(u16, u16), String> {
    let (mut a, mut b) = (fps.numerator, fps.denominator);
    while b != 0 {
        let r = a % b;
        a = b;
        b = r;
    }
    Ok((
        u16::try_from(fps.denominator / a).map_err(|_| "Use PNG sequence for this exact FPS")?,
        u16::try_from(fps.numerator / a).map_err(|_| "Use PNG sequence for this exact FPS")?,
    ))
}
pub fn prepare(d: &Document, o: Options, budget: usize) -> Result<Snapshot, String> {
    prepare_with(d, o, budget, None, None)
}
pub fn prepare_with(
    d: &Document,
    options: Options,
    budget: usize,
    watermark: Option<&Path>,
    encoder: Option<&crate::movie_export::Verified>,
) -> Result<Snapshot, String> {
    let a = d.animation.as_ref().ok_or("Choose an animation document")?;
    if !a.valid() || options.first > options.last || options.last >= a.frames || !d.scene.valid() {
        return Err("Choose an inclusive frame range inside this timeline".into());
    }
    if options.format == Format::Apng {
        delay(a.fps)?;
    }
    if matches!(options.format, Format::Gif | Format::Webp) {
        let units = if options.format == Format::Gif {
            100
        } else {
            1000
        };
        for n in 0..options.last - options.first + 1 {
            crate::animation_codecs::duration(a.fps, n, units)?;
        }
        if options.plays
            > if options.format == Format::Gif {
                65536
            } else {
                65535
            }
        {
            return Err("Loop count exceeds the selected format".into());
        }
    }
    if options.format == Format::Movie
        && !encoder.is_some_and(|v| v.codecs.contains(&options.codec))
    {
        return Err(
            "Verify a local encoder first. Without one use PNG sequence plus WAV output.".into(),
        );
    }
    let mut document = crate::layer_merge::preview(&d.snapshot(), options.first)?;
    document.path = d.path.clone();
    document.source_path = d.source_path.clone();
    crate::animation_render::apply(&mut document, options.view, true)?;
    let layout = crate::animation_render::layout(&document, options, budget)?;
    if d.memory_bytes()
        .saturating_mul(6)
        .saturating_add(layout.pixels().saturating_mul(48))
        > budget
    {
        return Err("Animation snapshot and frame buffers exceed the output budget".into());
    }
    if options.format == Format::Webp && (layout.width > 16384 || layout.height > 16384) {
        return Err("Lossless WebP frame dimensions must be at most16384".into());
    }
    if options.format == Format::Movie
        && options.codec == crate::movie_export::Codec::H264
        && (layout.width % 2 != 0 || layout.height % 2 != 0)
    {
        return Err(
            "H.264 4:2:0 needs even output dimensions; choose FFV1 for odd dimensions".into(),
        );
    }
    let protected = d
        .path
        .iter()
        .chain(d.source_path.iter())
        .cloned()
        .chain(d.scene.file_objects.entries.iter().map(|e| e.path.clone()))
        .chain(
            d.scene
                .timing
                .audio
                .iter()
                .filter_map(|c| c.sound.source.as_ref().map(|s| s.path.clone())),
        )
        .chain(watermark.map(Path::to_path_buf))
        .collect();
    let watermark = watermark
        .map(|p| crate::animation_render::Watermark::read(p, budget))
        .transpose()?;
    Ok(Snapshot {
        document: Box::new(document),
        options,
        revision: d.revision,
        layout,
        watermark,
        encoder: encoder.cloned(),
        budget,
        protected,
    })
}
pub(crate) struct Stage {
    pub(crate) directory: PathBuf,
    pub(crate) files: Vec<PathBuf>,
    keep: bool,
}
impl Drop for Stage {
    fn drop(&mut self) {
        if !self.keep {
            for p in &self.files {
                let _ = fs::remove_file(p);
            }
            let _ = fs::remove_dir(&self.directory);
        }
    }
}
impl Stage {
    pub(crate) fn new(parent: &Path) -> Result<Self, String> {
        for _ in 0..32 {
            let directory = parent.join(format!(
                ".Drawer-animation-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            match fs::create_dir(&directory) {
                Ok(()) => {
                    return Ok(Self {
                        directory,
                        files: vec![],
                        keep: false,
                    });
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(e) => return Err(e.to_string()),
            }
        }
        Err("Unable to reserve an owned animation output folder".into())
    }
    pub(crate) fn create(&mut self, name: &str) -> Result<File, String> {
        let path = self.directory.join(name);
        let f = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|e| e.to_string())?;
        self.files.push(path);
        Ok(f)
    }
}
pub struct Report {
    pub files: Vec<PathBuf>,
    pub frames: u32,
}
fn check(cancel: &AtomicBool) -> Result<(), String> {
    crate::book_io::check_cancel(cancel)
}
pub(crate) fn existing(path: &Path) -> Result<Option<[u8; 32]>, String> {
    match fs::symlink_metadata(path) {
        Ok(m) if m.is_file() && !m.file_type().is_symlink() && m.len() <= 512 * 1024 * 1024 => {
            let mut file = File::open(path).map_err(|e| e.to_string())?;
            let mut digest = Sha256::new();
            let mut buffer = [0; 65536];
            let mut total = 0u64;
            loop {
                let n = file.read(&mut buffer).map_err(|e| e.to_string())?;
                if n == 0 {
                    break;
                }
                total += n as u64;
                if total > 512 * 1024 * 1024 {
                    return Err("Destination grew beyond the replacement limit".into());
                }
                digest.update(&buffer[..n]);
            }
            Ok(Some(digest.finalize().into()))
        }
        Ok(_) => Err("Destination must be a regular local file, at most512MiB".into()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}
pub(crate) fn png_data(
    path: &Path,
    budget: usize,
) -> Result<(Vec<u8>, png::Info<'static>), String> {
    let decoder = png::Decoder::new(BufReader::new(File::open(path).map_err(|e| e.to_string())?));
    let mut reader = decoder.read_info().map_err(|e| e.to_string())?;
    let size = reader
        .output_buffer_size()
        .filter(|n| *n <= budget / 4)
        .ok_or("PNG frame decode budget")?;
    let mut bytes = vec![0; size];
    let frame = reader.next_frame(&mut bytes).map_err(|e| e.to_string())?;
    if frame.color_type != png::ColorType::Rgba || frame.bit_depth != png::BitDepth::Eight {
        return Err("Expected RGBA8 frame".into());
    }
    bytes.truncate(frame.buffer_size());
    let mut info = png::Info::with_size(frame.width, frame.height);
    info.color_type = png::ColorType::Rgba;
    info.bit_depth = png::BitDepth::Eight;
    info.icc_profile = reader
        .info()
        .icc_profile
        .as_ref()
        .map(|p| Cow::Owned(p.to_vec()));
    info.pixel_dims = reader.info().pixel_dims;
    Ok((bytes, info))
}
/// All frame files are prepared privately before one atomic publication.
/// Existing APNG replacement requires explicit authorization; folders are never merged.
pub fn write(
    s: &Snapshot,
    destination: &Path,
    overwrite: bool,
    cancel: &AtomicBool,
    progress: &AtomicU32,
) -> Result<Report, String> {
    write_with(s, destination, overwrite, cancel, progress, |_| {})
}
fn write_with(
    s: &Snapshot,
    destination: &Path,
    overwrite: bool,
    cancel: &AtomicBool,
    progress: &AtomicU32,
    mut after_frame: impl FnMut(u32),
) -> Result<Report, String> {
    check(cancel)?;
    let parent = destination
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
        .canonicalize()
        .map_err(|e| e.to_string())?;
    if !crate::font_library::local(&parent) || !parent.is_dir() {
        return Err("Choose a local output folder".into());
    }
    let name = destination.file_name().ok_or("Choose an output name")?;
    let destination = parent.join(name);
    if protected(s, &destination) {
        return Err("Preserve the original document; choose another output name".into());
    }
    let old = if s.options.format == Format::PngSequence {
        if fs::symlink_metadata(&destination).is_ok() {
            return Err(
                "Choose a new PNG sequence folder; existing folders are never merged or replaced"
                    .into(),
            );
        }
        None
    } else {
        if !valid_extension(s, &destination) {
            return Err("Choose the selected animation format's file extension".into());
        }
        let old = existing(&destination)?;
        if old.is_some() && !overwrite {
            return Err("Confirm replacement of the existing animation file".into());
        }
        old
    };
    let mut stage = Stage::new(&parent)?;
    let mut names = vec![];
    let mut bytes_written = 0u64;
    for frame in s.options.first..=s.options.last {
        check(cancel)?;
        let name = format!("frame-{:06}.png", frame + 1);
        let path = stage.directory.join(&name);
        let pixels = crate::animation_render::frame(s, frame, cancel)?;
        write_png(s, &path, &pixels)?;
        stage.files.push(path.clone());
        names.push(name);
        bytes_written =
            bytes_written.saturating_add(fs::metadata(path).map_err(|e| e.to_string())?.len());
        if bytes_written > 2 * 1024 * 1024 * 1024 {
            return Err("Animation output exceeds2GiB; split the range".into());
        }
        progress.store(frame - s.options.first + 1, Ordering::Relaxed);
        after_frame(frame);
        check(cancel)?;
    }
    let manifest = s.manifest(names.clone());
    let metadata = serde_json::to_string_pretty(&manifest).map_err(|e| e.to_string())?;
    if s.options.format == Format::PngSequence {
        let mut f = stage.create("animation.json")?;
        f.write_all(metadata.as_bytes())
            .and_then(|_| f.sync_all())
            .map_err(|e| e.to_string())?;
        drop(f);
        check(cancel)?;
        if fs::symlink_metadata(&destination).is_ok() {
            return Err("Output folder appeared during rendering; existing data preserved".into());
        }
        fs::rename(&stage.directory, &destination).map_err(|e| e.to_string())?;
        stage.keep = true;
        let mut files = names
            .into_iter()
            .map(|n| destination.join(n))
            .collect::<Vec<_>>();
        files.push(destination.join("animation.json"));
        return Ok(Report {
            files,
            frames: s.count(),
        });
    }
    let output = stage.directory.join("animation-output");
    stage.files.push(output.clone());
    crate::animation_codecs::encode(
        s,
        &stage.files[..stage.files.len() - 1],
        &output,
        &metadata,
        cancel,
    )?;
    check(cancel)?;
    if existing(&destination)? != old {
        return Err("Destination changed during rendering; existing file preserved".into());
    }
    fs::rename(&output, &destination).map_err(|e| e.to_string())?;
    Ok(Report {
        files: vec![destination],
        frames: s.count(),
    })
}

fn valid_extension(s: &Snapshot, p: &Path) -> bool {
    p.extension().is_some_and(|e| match s.options.format {
        Format::Apng => e.eq_ignore_ascii_case("apng") || e.eq_ignore_ascii_case("png"),
        Format::Gif => e.eq_ignore_ascii_case("gif"),
        Format::Webp => e.eq_ignore_ascii_case("webp"),
        Format::Movie => e.eq_ignore_ascii_case(s.options.codec.extension()),
        Format::PngSequence => true,
    })
}
fn protected(s: &Snapshot, path: &Path) -> bool {
    let candidate = path.canonicalize().unwrap_or_else(|_| path.to_owned());
    s.protected
        .iter()
        .any(|p| p.canonicalize().unwrap_or_else(|_| p.clone()) == candidate)
}
fn write_png(s: &Snapshot, path: &Path, pixels: &[u8]) -> Result<(), String> {
    let file = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)
        .map_err(|e| e.to_string())?;
    let result = (|| {
        let mut out = BufWriter::new(file);
        let mut info = png::Info::with_size(s.layout.width, s.layout.height);
        info.color_type = png::ColorType::Rgba;
        info.bit_depth = png::BitDepth::Eight;
        info.icc_profile = crate::animation_render::icc(s)?.map(Cow::Owned);
        let e = png::Encoder::with_info(&mut out, info).map_err(|e| e.to_string())?;
        let mut w = e.write_header().map_err(|e| e.to_string())?;
        w.write_image_data(pixels).map_err(|e| e.to_string())?;
        w.finish().map_err(|e| e.to_string())?;
        out.flush()
            .and_then(|_| out.get_ref().sync_all())
            .map_err(|e| e.to_string())?;
        drop(out);
        let (decoded, info) = png_data(path, s.budget)?;
        if decoded != pixels || info.width != s.layout.width || info.height != s.layout.height {
            return Err("PNG verification failed".into());
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(path);
    }
    result
}
#[derive(Debug)]
pub struct Outcome {
    pub files: Vec<PathBuf>,
    pub frames: u32,
    pub error: Option<String>,
    pub log: Option<PathBuf>,
}
/// User-facing delivery records completed files even when a later frame fails or is canceled.
/// Single-file animations remain atomic; PNG frames publish one verified file at a time.
pub fn publish(
    s: &Snapshot,
    destination: &Path,
    overwrite: bool,
    cancel: &AtomicBool,
    progress: &AtomicU32,
) -> Outcome {
    publish_with(s, destination, overwrite, cancel, progress, |_| {})
}
fn publish_with(
    s: &Snapshot,
    destination: &Path,
    overwrite: bool,
    cancel: &AtomicBool,
    progress: &AtomicU32,
    mut after: impl FnMut(u32),
) -> Outcome {
    let mut outcome = Outcome {
        files: vec![],
        frames: 0,
        error: None,
        log: None,
    };
    let result = (|| {
        let parent = destination
            .parent()
            .unwrap_or(Path::new("."))
            .canonicalize()
            .map_err(|e| e.to_string())?;
        if !crate::font_library::local(&parent) || !parent.is_dir() {
            return Err("Choose a local output directory".into());
        }
        let destination = parent.join(destination.file_name().ok_or("Missing output name")?);
        if protected(s, &destination) {
            return Err("Preserve the captured document / asset source".into());
        }
        // A unique persistent receipt belongs to this export only. It never replaces a user file.
        let (log, mut receipt) = loop {
            let p = parent.join(format!(
                "drawer-output-{}-{}.json",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            match fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .read(true)
                .open(&p)
            {
                Ok(f) => break (p, f),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e.to_string()),
            }
        };
        outcome.log = Some(log);
        let result = (|| {
            check(cancel)?;
            if s.options.format != Format::PngSequence {
                let report = write(s, &destination, overwrite, cancel, progress)?;
                outcome.frames = report.frames;
                outcome.files = report.files;
                return Ok(());
            }
            // create_dir is exclusive: no merge, resume or deletion of preexisting folders.
            fs::create_dir(&destination)
                .map_err(|e| format!("Choose a new sequence folder: {e}"))?;
            let mut stage = Stage::new(&parent)?;
            let mut bytes_written = 0u64;
            for f in s.options.first..=s.options.last {
                check(cancel)?;
                let name = format!("frame-{:06}.png", f + 1);
                let private = stage.directory.join(&name);
                let pixels = crate::animation_render::frame(s, f, cancel)?;
                write_png(s, &private, &pixels)?;
                stage.files.push(private.clone());
                bytes_written = bytes_written
                    .saturating_add(fs::metadata(&private).map_err(|e| e.to_string())?.len());
                if bytes_written > 2 * 1024 * 1024 * 1024 {
                    return Err("Sequence exceeds2GiB; split the range".into());
                }
                check(cancel)?;
                let target = destination.join(name);
                // Hard-link publication is exclusive and never overwrites a late foreign file.
                fs::hard_link(&private, &target).map_err(|e| e.to_string())?;
                outcome.files.push(target);
                outcome.frames += 1;
                progress.store(outcome.frames, Ordering::Relaxed);
                after(f);
            }
            let metadata = serde_json::to_vec_pretty(
                &s.manifest(
                    outcome
                        .files
                        .iter()
                        .map(|p| p.file_name().unwrap().to_string_lossy().into())
                        .collect(),
                ),
            )
            .map_err(|e| e.to_string())?;
            let mut f = stage.create("animation.json")?;
            f.write_all(&metadata)
                .and_then(|_| f.sync_all())
                .map_err(|e| e.to_string())?;
            drop(f);
            fs::hard_link(
                stage.directory.join("animation.json"),
                destination.join("animation.json"),
            )
            .map_err(|e| e.to_string())?;
            outcome.files.push(destination.join("animation.json"));
            Ok(())
        })();
        let pending = if s.options.format == Format::PngSequence {
            (s.options.first + outcome.frames..=s.options.last)
                .map(|f| destination.join(format!("frame-{:06}.png", f + 1)))
                .collect::<Vec<_>>()
        } else if result.is_err() {
            vec![destination.clone()]
        } else {
            vec![]
        };
        let record = serde_json::json!({"version":1,"snapshot":s.manifest(vec![]),"destination":destination,
            "encoder":s.encoder.as_ref().map(|v|v.description()),"completed":outcome.files,"not_completed":pending,
            "completed_frames":outcome.frames,"error":result.as_ref().err(),"status":if result.is_ok(){"completed"}else if cancel.load(Ordering::Relaxed){"canceled"}else{"failed"}});
        let b = serde_json::to_vec_pretty(&record).map_err(|e| e.to_string())?;
        receipt
            .write_all(&b)
            .and_then(|_| receipt.sync_all())
            .map_err(|e| e.to_string())?;
        result
    })();
    outcome.error = result.err();
    outcome
}

/// The save dialog supplies replacement confirmation; late destination changes still abort.
pub fn save_preset(path: &Path, bytes: &[u8]) -> Result<(), String> {
    Preset::decode(bytes)?;
    if !crate::font_library::local(path)
        || !path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("json"))
    {
        return Err("Choose a local .json preset".into());
    }
    let parent = path
        .parent()
        .ok_or("Choose preset directory")?
        .canonicalize()
        .map_err(|e| e.to_string())?;
    let path = parent.join(path.file_name().ok_or("Choose preset filename")?);
    let before = existing(&path)?;
    let mut stage = Stage::new(&parent)?;
    let mut f = stage.create("preset.json")?;
    f.write_all(bytes)
        .and_then(|_| f.sync_all())
        .map_err(|e| e.to_string())?;
    drop(f);
    if existing(&path)? != before {
        return Err("Preset changed while saving".into());
    }
    std::fs::rename(stage.directory.join("preset.json"), path).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    #[test]
    fn snapshot_ranges_rgba_timing_replacement_cancel_failure_and_native_sources() {
        let root = crate::test_support::TempDir::new();
        crate::animation_export_verify::verify(&root.0).unwrap();
    }
    #[test]
    fn cancellation_and_destination_races_preserve_existing_data() {
        use super::*;
        let root = crate::test_support::TempDir::new();
        let d = crate::animation_export_verify::fixture();
        let s = prepare(
            &d,
            Options {
                first: 0,
                last: 2,
                format: Format::Apng,
                plays: 1,
                ..Default::default()
            },
            crate::document::DEFAULT_DOCUMENT_BUDGET,
        )
        .unwrap();
        let path = root.0.join("out.apng");
        fs::write(&path, b"original").unwrap();
        let cancel = AtomicBool::new(false);
        let progress = AtomicU32::new(0);
        assert!(
            write_with(&s, &path, true, &cancel, &progress, |_| {
                cancel.store(true, Ordering::Relaxed);
            })
            .is_err()
        );
        assert_eq!(fs::read(&path).unwrap(), b"original");
        assert_eq!(fs::read_dir(&root.0).unwrap().count(), 1);
        cancel.store(false, Ordering::Relaxed);
        assert!(
            write_with(&s, &path, true, &cancel, &progress, |_| {
                fs::write(&path, b"changed during render").unwrap();
            })
            .is_err()
        );
        assert_eq!(fs::read(&path).unwrap(), b"changed during render");
        assert_eq!(fs::read_dir(&root.0).unwrap().count(), 1);
        let sequence = prepare(
            &d,
            Options {
                format: Format::PngSequence,
                ..s.options
            },
            crate::document::DEFAULT_DOCUMENT_BUDGET,
        )
        .unwrap();
        let folder = root.0.join("appeared");
        assert!(
            write_with(&sequence, &folder, false, &cancel, &progress, |_| {
                if !folder.exists() {
                    fs::create_dir(&folder).unwrap();
                    fs::write(folder.join("foreign.txt"), b"preserve").unwrap();
                }
            })
            .is_err()
        );
        assert_eq!(fs::read(folder.join("foreign.txt")).unwrap(), b"preserve");
        assert!(fs::read_dir(&root.0).unwrap().all(|e| {
            !e.unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".Drawer-animation")
        }));
        let mut unusual = d.clone();
        unusual.animation.as_mut().unwrap().fps = Fps {
            numerator: 65537,
            denominator: 1000,
        };
        assert!(
            prepare(
                &unusual,
                s.options,
                crate::document::DEFAULT_DOCUMENT_BUDGET
            )
            .is_err()
        );
    }
}

#[cfg(test)]
#[path = "animation_delivery_tests.rs"]
mod delivery_tests;
