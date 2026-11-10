//! Reviewed pixel/DPI/name plans and per-file, non-replacing local publication.
use crate::{
    asset_catalog as assets,
    document::Document,
    raster_io::{self, Options},
};
use std::{
    collections::BTreeSet,
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
};
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Geometry {
    pub resize: bool,
    pub width: u32,
    pub height: u32,
    pub resolution: bool,
    pub dpi: f64,
}
impl Default for Geometry {
    fn default() -> Self {
        Self {
            resize: false,
            width: 1920,
            height: 1080,
            resolution: false,
            dpi: 300.,
        }
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    pub color: crate::export_color::Settings,
    pub geometry: Geometry,
    pub encoding: Options,
    pub pattern: String,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            color: Default::default(),
            geometry: Default::default(),
            encoding: Options {
                format: raster_io::Format::Png,
                alpha: true,
                ..Default::default()
            },
            pattern: "{title}-{index}".into(),
        }
    }
}
pub fn filename(
    pattern: &str,
    title: &str,
    index: usize,
    extension: &str,
) -> Result<String, String> {
    if pattern.is_empty() || pattern.len() > 512 || index == 0 || index > 256 {
        return Err("Filename pattern/index limit".into());
    }
    let title = Path::new(title)
        .file_stem()
        .and_then(|s| s.to_str())
        .ok_or("Invalid source title")?;
    let stem = pattern
        .replace("{title}", title)
        .replace("{index}", &format!("{index:04}"));
    let device = stem
        .split('.')
        .next()
        .unwrap_or_default()
        .trim_end()
        .replace('¹', "1")
        .replace('²', "2")
        .replace('³', "3")
        .to_ascii_uppercase();
    if stem.is_empty()
        || stem != stem.trim()
        || stem.ends_with('.')
        || stem.encode_utf16().count() > 180
        || stem
            .chars()
            .any(|c| c.is_control() || "<>:\"/\\|?*{}".contains(c))
        || ["CON", "PRN", "AUX", "NUL", "CLOCK$", "CONIN$", "CONOUT$"].contains(&device.as_str())
        || (1..=9).any(|n| device == format!("COM{n}") || device == format!("LPT{n}"))
        || extension.is_empty()
        || extension.len() > 8
        || !extension.bytes().all(|b| b.is_ascii_alphanumeric())
    {
        return Err(
            "Use a local filename without reserved characters/device names or unknown placeholders"
                .into(),
        );
    }
    Ok(format!("{stem}.{extension}"))
}
pub struct File {
    pub path: PathBuf,
    pub width: u32,
    pub height: u32,
    pub dpi: f64,
    pub bytes: Vec<u8>,
    pub preview: Vec<[u8; 4]>,
    pub source_preview: Vec<[u8; 4]>,
    pub preview_size: [usize; 2],
    pub notices: Vec<String>,
}
pub struct Plan {
    pub files: Vec<File>,
    sources: Vec<(u64, u64, u32)>,
    protected: Vec<PathBuf>,
}
#[derive(Debug)]
pub struct ResultFile {
    pub path: PathBuf,
    pub result: Result<(), String>,
}
#[derive(Debug)]
pub struct Report {
    pub files: Vec<ResultFile>,
}
pub fn prepare(
    docs: &[Document],
    folder: &Path,
    s: &Settings,
    budget: usize,
    c: &AtomicBool,
) -> Result<Plan, String> {
    crate::file_objects::local_path(folder)?;
    if docs.is_empty() || docs.len() > 32 || !s.encoding.valid() {
        return Err("Choose 1..32 source documents and supported codec options".into());
    }
    let g = s.geometry;
    if g.resize
        && (g.width == 0
            || g.height == 0
            || g.width > 32768
            || g.height > 32768
            || u64::from(g.width) * u64::from(g.height) > 4 * 1024 * 1024)
        || g.resolution && (!g.dpi.is_finite() || !(1.0..=4800.).contains(&g.dpi))
    {
        return Err("Choose nonzero output pixels (max4M), and1..4800 output DPI".into());
    }
    let root = folder.canonicalize().map_err(|e| e.to_string())?;
    let mut files = Vec::new();
    let mut names = BTreeSet::new();
    let mut used = 0usize;
    let mut protected = Vec::new();
    for (n, d) in docs.iter().enumerate() {
        assets::canceled(c)?;
        let name = filename(&s.pattern, &d.title, n + 1, s.encoding.format.extension())?;
        if !names.insert(name.to_lowercase()) {
            return Err("Filename pattern creates duplicate output names; add {index}".into());
        }
        let path = assets::checked(&root, Path::new(&name), false)?;
        for p in d.path.iter().chain(d.source_path.iter()) {
            let p = p.canonicalize().unwrap_or_else(|_| p.clone());
            protected.push(p);
        }
        files.push(File {
            path,
            width: 0,
            height: 0,
            dpi: 0.,
            bytes: vec![],
            preview: vec![],
            source_preview: vec![],
            preview_size: [0; 2],
            notices: vec![],
        });
    }
    for f in &files {
        if protected.iter().any(|p| {
            p.to_string_lossy()
                .eq_ignore_ascii_case(&f.path.to_string_lossy())
        }) {
            return Err("Output would replace an input document".into());
        }
    }
    for (f, d) in files.iter_mut().zip(docs) {
        assets::canceled(c)?;
        let available = budget.min(128 * 1024 * 1024).saturating_sub(used);
        let p = raster_io::prepare_managed(d, available, c, &s.color, s.encoding)?;
        let g = s.geometry;
        let (width, height) = if g.resize {
            (g.width, g.height)
        } else {
            (d.width, d.height)
        };
        let dpi = if g.resolution { g.dpi } else { d.dpi };
        let p = p.resized(width, height, dpi, available, c)?;
        let bytes = p.encode(s.encoding, available, c)?;
        used = used.saturating_add(bytes.len());
        if used > budget.min(64 * 1024 * 1024) {
            return Err("Prepared batch exceeds 64MiB output budget".into());
        }
        let step = width.div_ceil(160).max(height.div_ceil(100)).max(1);
        let (w, h) = (
            width.div_ceil(step) as usize,
            height.div_ceil(step) as usize,
        );
        let mut preview = Vec::with_capacity(w * h);
        for y in 0..h {
            for x in 0..w {
                preview.push(
                    p.preview_sample(
                        y * step as usize * width as usize + x * step as usize,
                        s.encoding,
                    )?
                    .rgba8(),
                );
            }
        }
        let source_renderer = crate::render::Renderer::for_export(d)?;
        let mut source_preview = Vec::with_capacity(w * h);
        for y in 0..h {
            assets::canceled(c)?;
            for x in 0..w {
                let point = [
                    (x as f64 + 0.5) / w as f64 * d.width as f64,
                    (y as f64 + 0.5) / h as f64 * d.height as f64,
                ];
                let mut sample = source_renderer.display_sample(point)?;
                if !s.encoding.alpha {
                    sample = crate::color_precision::Sample::from8([
                        s.encoding.background[0],
                        s.encoding.background[1],
                        s.encoding.background[2],
                        255,
                    ])
                    .over(sample, crate::layer_blend::Mode::Normal, 1.)?;
                }
                source_preview.push(sample.rgba8());
            }
        }
        f.source_preview = source_preview;
        f.width = width;
        f.height = height;
        f.dpi = dpi;
        f.bytes = bytes;
        f.preview = preview;
        f.preview_size = [w, h];
        f.notices = p.losses(s.encoding);
    }
    assets::canceled(c)?;
    Ok(Plan {
        files,
        sources: docs
            .iter()
            .map(|d| (d.id, d.revision, d.current_frame))
            .collect(),
        protected,
    })
}
impl Plan {
    pub fn matches(&self, docs: &[Document]) -> bool {
        self.sources.iter().all(|s| {
            docs.iter()
                .any(|d| (d.id, d.revision, d.current_frame) == *s)
        })
    }
    pub fn protect(&self, paths: &std::collections::HashSet<PathBuf>) -> Result<(), String> {
        if self.files.iter().any(|f| {
            paths.iter().any(|p| {
                p.canonicalize()
                    .unwrap_or_else(|_| p.clone())
                    .to_string_lossy()
                    .eq_ignore_ascii_case(&f.path.to_string_lossy())
            })
        }) {
            Err("Output would replace a protected imported source".into())
        } else {
            Ok(())
        }
    }
    /// A canceled batch preserves completed new files and reports every remaining file as canceled.
    pub fn publish(self, c: &AtomicBool) -> Report {
        let files = self
            .files
            .into_iter()
            .map(|f| {
                let result = (|| {
                    assets::canceled(c)?;
                    if self.protected.iter().any(|p| {
                        p.to_string_lossy()
                            .eq_ignore_ascii_case(&f.path.to_string_lossy())
                    }) {
                        return Err("Protected source destination".into());
                    }
                    let parent = f.path.parent().ok_or("Missing output folder")?;
                    let path = assets::checked(
                        parent,
                        Path::new(f.path.file_name().ok_or("Missing output name")?),
                        false,
                    )?;
                    if path.exists() {
                        return Err("Destination exists; no file was replaced".into());
                    }
                    let temp = path.with_file_name(format!(
                        "{}.output-write.tmp",
                        path.file_name().unwrap().to_string_lossy()
                    ));
                    let mut file = fs::OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .open(&temp)
                        .map_err(|e| e.to_string())?;
                    let result = (|| {
                        for b in f.bytes.chunks(65536) {
                            assets::canceled(c)?;
                            file.write_all(b).map_err(|e| e.to_string())?;
                        }
                        file.sync_all().map_err(|e| e.to_string())?;
                        drop(file);
                        assets::canceled(c)?;
                        fs::hard_link(&temp, &path).map_err(|e| e.to_string())
                    })();
                    let cleanup = fs::remove_file(&temp).map_err(|e| e.to_string());
                    result.and(cleanup)
                })();
                ResultFile {
                    path: f.path,
                    result,
                }
            })
            .collect();
        Report { files }
    }
}
#[path = "output_batch_verify.rs"]
pub mod verify;
