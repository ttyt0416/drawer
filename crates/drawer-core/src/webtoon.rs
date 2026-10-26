//! Row-exact local PNG strips; staged output never overwrites an existing file.
use crate::{comics::Webtoon, document::Document, render::Renderer};
use image::ImageEncoder;
use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};
static NEXT: AtomicU64 = AtomicU64::new(1);
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Strip {
    pub start: u32,
    pub end: u32,
    pub name: String,
}
pub fn plan(height: u32, s: &Webtoon) -> Result<Vec<Strip>, String> {
    if height == 0 || height > 32768 || !s.valid() || s.cuts.iter().any(|c| *c >= height) {
        return Err("Invalid webtoon split settings or boundary outside the page".into());
    }
    let mut boundaries = vec![0];
    let mut y = s.slice;
    while y < height {
        boundaries.push(y);
        y += s.slice;
    }
    boundaries.extend(&s.cuts);
    boundaries.push(height);
    boundaries.sort_unstable();
    boundaries.dedup();
    if boundaries.len() > 4097 {
        return Err("Webtoon output limit4096 strips".into());
    }
    Ok(boundaries
        .windows(2)
        .enumerate()
        .map(|(i, b)| Strip {
            start: b[0],
            end: b[1],
            name: format!("{}-{:04}.png", s.prefix, i + 1),
        })
        .collect())
}
struct Stage {
    path: PathBuf,
    published: Vec<PathBuf>,
    done: bool,
}
impl Drop for Stage {
    fn drop(&mut self) {
        if !self.done {
            for p in &self.published {
                let _ = fs::remove_file(p);
            }
        }
        let _ = fs::remove_dir_all(&self.path);
    }
}
pub fn export(
    d: &Document,
    s: &Webtoon,
    directory: &Path,
    tick: &mut impl FnMut(usize, usize) -> Result<(), String>,
) -> Result<Vec<PathBuf>, String> {
    export_with_publisher(d, s, directory, tick, &mut |a, b| {
        fs::hard_link(a, b).map_err(|e| e.to_string())
    })
}
pub fn export_with_publisher(
    d: &Document,
    s: &Webtoon,
    directory: &Path,
    tick: &mut impl FnMut(usize, usize) -> Result<(), String>,
    publisher: &mut impl FnMut(&Path, &Path) -> Result<(), String>,
) -> Result<Vec<PathBuf>, String> {
    if !(1..=32768).contains(&d.width)
        || !d.dpi.is_finite()
        || d.dpi <= 0.
        || !d.scene.valid()
        || !d.scene.paper.valid_for(d.layers.len())
        || !d.color.valid_layers(&d.layers)
        || d.layers.iter().any(|l| !l.valid(d.width, d.height))
        || !d.scene.catalog.valid_animation(d.animation.as_ref())
        || d.animation.as_ref().is_some_and(|a| !a.valid())
    {
        return Err("Invalid document; no webtoon files were created".into());
    }
    let strips = plan(d.height, s)?;
    let directory = directory.canonicalize().map_err(|e| e.to_string())?;
    if !crate::font_library::local(&directory) || !directory.is_dir() {
        return Err("Choose a local output folder".into());
    }
    let outputs: Vec<_> = strips.iter().map(|s| directory.join(&s.name)).collect();
    if outputs.iter().any(|p| p.try_exists().unwrap_or(true)) {
        return Err("An output filename already exists; change the prefix".into());
    }
    let path = directory.join(format!(
        ".drawer-strips-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&path).map_err(|e| e.to_string())?;
    let mut stage = Stage {
        path,
        published: vec![],
        done: false,
    };
    let renderer = Renderer::for_export(d)?;
    if let Some(error) = renderer.error() {
        return Err(error.into());
    }
    let mut completed = 0;
    let total = d.height as usize + strips.len();
    for strip in &strips {
        let count = d.width as usize * (strip.end - strip.start) as usize;
        let budget = crate::document::DEFAULT_DOCUMENT_BUDGET.saturating_sub(d.resident_bytes());
        if count.saturating_mul(4) > budget / 2 {
            return Err("Strip output memory limit; use a smaller strip height".into());
        }
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(count * 4)
            .map_err(|_| "Strip allocation failed")?;
        for y in strip.start..strip.end {
            tick(completed, total)?;
            completed += 1;
            for x in 0..d.width {
                bytes.extend(renderer.pixel([x as f64 + 0.5, y as f64 + 0.5]));
            }
        }
        let output = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(stage.path.join(&strip.name))
            .map_err(|e| e.to_string())?;
        image::codecs::png::PngEncoder::new(&output)
            .write_image(
                &bytes,
                d.width,
                strip.end - strip.start,
                image::ExtendedColorType::Rgba8,
            )
            .map_err(|e| e.to_string())?;
        output.sync_all().map_err(|e| e.to_string())?;
    }
    for (strip, output) in strips.iter().zip(&outputs) {
        tick(completed, total)?;
        completed += 1;
        publisher(&stage.path.join(&strip.name), output)
            .map_err(|e| format!("Could not publish without overwriting: {e}"))?;
        stage.published.push(output.clone());
    }
    tick(total, total)?;
    stage.done = true;
    Ok(outputs)
}
