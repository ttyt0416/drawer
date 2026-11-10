//! Physical-page print layout and virtual proof, independent of document history and drivers.
use crate::{
    book_io::{self, Page},
    references::LocalImage,
    storage::UserStorage,
};
use serde::{Deserialize, Serialize};
use std::{fs, io::Write, path::Path, sync::atomic::AtomicBool};
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Paper {
    #[default]
    A4,
    Letter,
    Custom,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub version: u32,
    pub paper: Paper,
    pub custom_mm: [f64; 2],
    pub landscape: bool,
    pub margins: [f64; 4],
    pub fit: bool,
    pub percent: f64,
    pub range: String,
    pub icm: bool,
    #[serde(default)]
    pub output: crate::print_options::Options,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            version: 1,
            paper: Paper::A4,
            custom_mm: [210., 297.],
            landscape: false,
            margins: [10.; 4],
            fit: true,
            percent: 100.,
            range: String::new(),
            icm: true,
            output: Default::default(),
        }
    }
}
impl Settings {
    pub fn paper_mm(&self) -> [f64; 2] {
        let mut p = match self.paper {
            Paper::A4 => [210., 297.],
            Paper::Letter => [215.9, 279.4],
            Paper::Custom => self.custom_mm,
        };
        if self.landscape {
            p = [p[0].max(p[1]), p[0].min(p[1])];
        } else {
            p = [p[0].min(p[1]), p[0].max(p[1])];
        }
        p
    }
    pub fn valid(&self) -> bool {
        let p = self.paper_mm();
        self.version == 1
            && self.output.valid()
            && self
                .custom_mm
                .iter()
                .all(|n| n.is_finite() && (1.0..=2000.).contains(n))
            && self
                .margins
                .iter()
                .all(|n| n.is_finite() && (0.0..=500.).contains(n))
            && self.percent.is_finite()
            && (1.0..=400.).contains(&self.percent)
            && self.range.len() <= 4096
            && p[0] > self.margins[0] + self.margins[2]
            && p[1] > self.margins[1] + self.margins[3]
    }
    pub fn load(storage: &UserStorage) -> Result<Self, String> {
        let path = storage.root().join("settings/printing.json");
        let m = match fs::symlink_metadata(&path) {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(e) => return Err(e.to_string()),
        };
        if !m.is_file() || m.file_type().is_symlink() || m.len() > 32768 {
            return Err("Invalid print preferences file".into());
        }
        let s: Self = serde_json::from_slice(&fs::read(path).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        if !s.valid() {
            return Err("Invalid print preference version/values".into());
        }
        Ok(s)
    }
    pub fn save(&self, storage: &UserStorage) -> Result<(), String> {
        if !self.valid() {
            return Err("Invalid print settings; previous preferences preserved".into());
        }
        let path = storage.root().join("settings/printing.json");
        if fs::symlink_metadata(&path).is_ok_and(|m| !m.is_file() || m.file_type().is_symlink()) {
            return Err("Print preferences destination is not a regular file".into());
        }
        let temp = path.with_extension("json.print-write.tmp");
        let mut f = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)
            .map_err(|e| e.to_string())?;
        let result = (|| {
            f.write_all(&serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?;
            f.sync_all().map_err(|e| e.to_string())?;
            drop(f);
            fs::rename(&temp, path).map_err(|e| e.to_string())
        })();
        if result.is_err() {
            let _ = fs::remove_file(temp);
        }
        result
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Area {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}
impl Area {
    pub fn contains(self, x: f64, y: f64) -> bool {
        x >= self.x && y >= self.y && x < self.x + self.width && y < self.y + self.height
    }
}
#[derive(Clone, Debug)]
pub struct Placement {
    pub source: usize,
    pub image: Area,
    pub clip: Area,
    pub clipped: bool,
}
#[derive(Clone, Debug)]
pub struct Plan {
    pub paper_mm: [f64; 2],
    pub pages: Vec<Placement>,
}
pub fn plan(pages: &[Page], s: &Settings) -> Result<Plan, String> {
    book_io::validate(pages, book_io::MAX_BYTES)?;
    if !s.valid() {
        return Err("Paper, margin, scale or settings version is invalid".into());
    }
    let paper = s.paper_mm();
    if s.output.registration
        && paper
            .iter()
            .any(|v| *v < 2. * (s.output.registration_inset_mm + 2.))
    {
        return Err("Paper is too small for the selected registration mark inset".into());
    }
    let clip = Area {
        x: s.margins[0],
        y: s.margins[1],
        width: paper[0] - s.margins[0] - s.margins[2],
        height: paper[1] - s.margins[1] - s.margins[3],
    };
    let selected = book_io::selection(&s.range, pages.len())?;
    let mut out = Vec::new();
    for n in selected {
        let p = &pages[n];
        let w = p.image.width as f64 / p.dpi * 25.4;
        let h = p.image.height as f64 / p.dpi * 25.4;
        let scale = if s.fit {
            (clip.width / w).min(clip.height / h)
        } else {
            s.percent / 100.
        };
        let (width, height) = (w * scale, h * scale);
        let image = Area {
            x: clip.x + (clip.width - width) / 2.,
            y: clip.y + (clip.height - height) / 2.,
            width,
            height,
        };
        let clipped = width > clip.width + 1e-8 || height > clip.height + 1e-8;
        out.push(Placement {
            source: n,
            image,
            clip,
            clipped,
        });
    }
    Ok(Plan {
        paper_mm: paper,
        pages: out,
    })
}
/// Validate the chosen driver's physical sheet and printable origin before StartDoc.
pub fn driver_layout(s: &Settings, paper: [f64; 2], unprintable: [f64; 4]) -> Result<(), String> {
    if !s.valid()
        || paper.iter().any(|v| !v.is_finite() || *v <= 0.)
        || unprintable.iter().any(|v| !v.is_finite() || *v < 0.)
    {
        return Err("No valid printer paper/printable-area metrics".into());
    }
    let requested = s.paper_mm();
    if (0..2).any(|n| (requested[n] - paper[n]).abs() > 1.) {
        return Err(format!(
            "Printer paper is {:.2}×{:.2}mm; preview requires {:.2}×{:.2}mm. Choose matching paper/orientation in the Windows dialog, then retry.",
            paper[0], paper[1], requested[0], requested[1]
        ));
    }
    if (0..4).any(|n| s.margins[n] + 0.05 < unprintable[n]) {
        return Err(format!(
            "Printer cannot reach the reviewed margins. Minimum left/top/right/bottom: {unprintable:.2?}mm; increase margins and preview again."
        ));
    }
    Ok(())
}
pub fn send(
    pages: &[Page],
    s: &Settings,
    sink: &mut impl book_io::PrintSink,
    c: &AtomicBool,
) -> Result<(), String> {
    let plan = plan(pages, s)?;
    book_io::check_cancel(c)?;
    sink.begin(plan.pages.len())?;
    let result = (|| {
        for (n, p) in plan.pages.iter().enumerate() {
            book_io::check_cancel(c)?;
            sink.page(n, &pages[p.source])?;
        }
        book_io::check_cancel(c)?;
        sink.finish()
    })();
    if result.is_err() {
        sink.abort();
    }
    result
}
pub fn virtual_pages(
    pages: &[Page],
    s: &Settings,
    dpi: f64,
    budget: usize,
    c: &AtomicBool,
) -> Result<Vec<Page>, String> {
    if !dpi.is_finite() || !(36.0..=300.).contains(&dpi) {
        return Err("Virtual print proof DPI must be36..300".into());
    }
    let mapping = s.output.channel != crate::print_options::Channel::Composite
        || s.icm && s.output.profile.is_some();
    let mapped = if mapping {
        if pages.iter().map(|p| p.image.memory_bytes()).sum::<usize>() > budget / 2 {
            return Err("Print color/channel copy exceeds proof budget".into());
        }
        Some(crate::print_options::pages(
            pages, &s.output, s.icm, true, c,
        )?)
    } else {
        None
    };
    let pages = mapped.as_deref().unwrap_or(pages);
    let plan = plan(pages, s)?;
    let width = (plan.paper_mm[0] / 25.4 * dpi).round() as u32;
    let height = (plan.paper_mm[1] / 25.4 * dpi).round() as u32;
    let count = width as usize * height as usize;
    if width == 0
        || height == 0
        || count > 4 * 1024 * 1024
        || count.saturating_mul(4).saturating_mul(plan.pages.len()) > budget.min(book_io::MAX_BYTES)
    {
        return Err("Virtual print proof exceeds pixel/memory limit".into());
    }
    let mut out = Vec::new();
    for placement in plan.pages {
        let source = &pages[placement.source].image;
        let mut pixels = vec![[255; 4]; count];
        let step = [
            source.width as f64 / (placement.image.width / 25.4 * dpi),
            source.height as f64 / (placement.image.height / 25.4 * dpi),
        ];
        for y in 0..height {
            book_io::check_cancel(c)?;
            for x in 0..width {
                let mm = [(x as f64 + 0.5) / dpi * 25.4, (y as f64 + 0.5) / dpi * 25.4];
                if !placement.clip.contains(mm[0], mm[1]) || !placement.image.contains(mm[0], mm[1])
                {
                    continue;
                }
                let point = [
                    (mm[0] - placement.image.x) / placement.image.width * source.width as f64 - 0.5,
                    (mm[1] - placement.image.y) / placement.image.height * source.height as f64
                        - 0.5,
                ];
                let p = crate::interpolation::sample(
                    point,
                    crate::interpolation::box_footprint(point, step),
                    crate::interpolation::Filter::Bilinear,
                    |x, y| {
                        source.pixels[y.clamp(0, source.height as i64 - 1) as usize
                            * source.width as usize
                            + x.clamp(0, source.width as i64 - 1) as usize]
                    },
                );
                pixels[(y * width + x) as usize] = [0, 1, 2, 3].map(|n| {
                    if n == 3 {
                        255
                    } else {
                        ((p[n] as u32 * p[3] as u32 + 255 * (255 - p[3] as u32) + 127) / 255) as u8
                    }
                });
            }
        }
        out.push(Page {
            image: LocalImage {
                width,
                height,
                pixels,
            },
            dpi,
        });
    }
    if s.output.registration {
        for page in &mut out {
            crate::print_options::draw_registration(page, s.output.registration_inset_mm);
        }
    }
    book_io::check_cancel(c)?;
    Ok(out)
}
#[path = "printing_verify.rs"]
pub mod verify;
