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
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Options {
    pub first: u32,
    pub last: u32,
    pub format: Format,
    /// Zero is infinite, otherwise the number of complete plays.
    pub plays: u32,
}
#[derive(Serialize)]
struct Manifest {
    version: u8,
    revision: u64,
    width: u32,
    height: u32,
    fps: Fps,
    options: Options,
    rgba_bits: u8,
    files: Vec<String>,
}
pub struct Snapshot {
    document: Box<Document>,
    pub options: Options,
    pub revision: u64,
    budget: usize,
}
impl Snapshot {
    pub fn document(&self) -> &Document {
        &self.document
    }
    pub fn count(&self) -> u32 {
        self.options.last - self.options.first + 1
    }
}
fn delay(fps: Fps) -> Result<(u16, u16), String> {
    let (mut a, mut b) = (fps.numerator, fps.denominator);
    while b != 0 {
        let r = a % b;
        a = b;
        b = r;
    }
    let numerator = u16::try_from(fps.denominator / a)
        .map_err(|_| "This FPS cannot be represented exactly by APNG; use PNG sequence")?;
    let denominator = u16::try_from(fps.numerator / a)
        .map_err(|_| "This FPS cannot be represented exactly by APNG; use PNG sequence")?;
    Ok((numerator, denominator))
}
pub fn prepare(d: &Document, options: Options, budget: usize) -> Result<Snapshot, String> {
    let a = d.animation.as_ref().ok_or("Choose an animation document")?;
    if !a.valid() || options.first > options.last || options.last >= a.frames || !d.scene.valid() {
        return Err("Choose an inclusive frame range inside this timeline".into());
    }
    if options.format == Format::Apng {
        delay(a.fps)?;
    }
    let pixels = (d.width as usize)
        .checked_mul(d.height as usize)
        .ok_or("Output dimensions overflow")?;
    if d.memory_bytes()
        .saturating_mul(3)
        .saturating_add(pixels.saturating_mul(32))
        > budget
    {
        return Err("Animation snapshot and frame buffers exceed the output budget".into());
    }
    let mut document = crate::layer_merge::preview(&d.snapshot(), options.first)?;
    document.path = d.path.clone();
    document.source_path = d.source_path.clone();
    Ok(Snapshot {
        document: Box::new(document),
        options,
        revision: d.revision,
        budget,
    })
}
struct Stage {
    directory: PathBuf,
    files: Vec<PathBuf>,
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
    fn new(parent: &Path) -> Result<Self, String> {
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
    fn create(&mut self, name: &str) -> Result<File, String> {
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
fn existing(path: &Path) -> Result<Option<[u8; 32]>, String> {
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
fn png_data(path: &Path, budget: usize) -> Result<(Vec<u8>, png::Info<'static>), String> {
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
    if s.document.path.as_ref() == Some(&destination)
        || s.document.source_path.as_ref() == Some(&destination)
    {
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
        if !destination
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("apng") || e.eq_ignore_ascii_case("png"))
        {
            return Err("Choose a .apng or .png output file".into());
        }
        let old = existing(&destination)?;
        if old.is_some() && !overwrite {
            return Err("Confirm replacement of the existing APNG file".into());
        }
        old
    };
    let mut stage = Stage::new(&parent)?;
    let mut names = vec![];
    let mut bytes_written = 0u64;
    for frame in s.options.first..=s.options.last {
        check(cancel)?;
        let d = crate::layer_merge::preview(&s.document.snapshot(), frame)?;
        let name = format!("frame-{:06}.png", frame + 1);
        let path = stage.directory.join(&name);
        crate::png_export::export(&d, &path, s.budget).map_err(|e| e.to_string())?;
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
    let manifest = Manifest {
        version: 1,
        revision: s.revision,
        width: s.document.width,
        height: s.document.height,
        fps: s.document.animation.as_ref().unwrap().fps,
        options: s.options,
        rgba_bits: 8,
        files: names.clone(),
    };
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
    let (first, info) = png_data(&stage.files[0], s.budget)?;
    let mut writer = BufWriter::new(stage.create("animation.apng")?);
    {
        let mut encoder = png::Encoder::with_info(&mut writer, info).map_err(|e| e.to_string())?;
        encoder
            .set_animated(s.count(), s.options.plays)
            .map_err(|e| e.to_string())?;
        let (n, d) = delay(manifest.fps)?;
        encoder.set_frame_delay(n, d).map_err(|e| e.to_string())?;
        encoder
            .set_blend_op(png::BlendOp::Source)
            .map_err(|e| e.to_string())?;
        encoder
            .set_dispose_op(png::DisposeOp::None)
            .map_err(|e| e.to_string())?;
        encoder
            .add_text_chunk("Drawer Export".into(), metadata)
            .map_err(|e| e.to_string())?;
        let mut out = encoder.write_header().map_err(|e| e.to_string())?;
        out.write_image_data(&first).map_err(|e| e.to_string())?;
        for path in &stage.files[1..stage.files.len() - 1] {
            check(cancel)?;
            let (bytes, info) = png_data(path, s.budget)?;
            if info.width != manifest.width || info.height != manifest.height {
                return Err("Output frame dimensions changed".into());
            }
            out.write_image_data(&bytes).map_err(|e| e.to_string())?;
        }
        out.finish().map_err(|e| e.to_string())?;
    }
    writer
        .flush()
        .and_then(|_| writer.get_ref().sync_all())
        .map_err(|e| e.to_string())?;
    drop(writer);
    check(cancel)?;
    if existing(&destination)? != old {
        return Err("Destination changed during rendering; existing file preserved".into());
    }
    fs::rename(stage.directory.join("animation.apng"), &destination).map_err(|e| e.to_string())?;
    Ok(Report {
        files: vec![destination],
        frames: s.count(),
    })
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
