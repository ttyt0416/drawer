//! Print-only content, channel, RGB ICC and physical-screen options.
use crate::{
    book_io::{self, Page},
    document::{DEFAULT_DOCUMENT_BUDGET as B, Document},
    layer_properties as properties,
    references::LocalImage,
    scene::PageObject,
    soft_proof::Intent,
};
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
};
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Channel {
    #[default]
    Composite,
    Red,
    Green,
    Blue,
    Alpha,
}
impl Channel {
    pub const ALL: [Self; 5] = [
        Self::Composite,
        Self::Red,
        Self::Green,
        Self::Blue,
        Self::Alpha,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Composite => "Composite RGB",
            Self::Red => "Red as gray",
            Self::Green => "Green as gray",
            Self::Blue => "Blue as gray",
            Self::Alpha => "Alpha coverage (opaque = black)",
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Calibration {
    pub pixels_per_mm: f64,
    pub label: String,
}
impl Calibration {
    pub fn measure(pixels: f64, millimeters: f64, label: String) -> Result<Self, String> {
        let value = Self {
            pixels_per_mm: pixels / millimeters,
            label,
        };
        if !pixels.is_finite()
            || pixels <= 0.
            || !millimeters.is_finite()
            || millimeters <= 0.
            || !value.valid()
        {
            return Err("Measure the tick centers in millimeters; supported density0.5..100 physical pixels/mm".into());
        }
        Ok(value)
    }
    pub fn valid(&self) -> bool {
        self.pixels_per_mm.is_finite()
            && (0.5..=100.).contains(&self.pixels_per_mm)
            && self.label.chars().count() <= 64
            && !self.label.chars().any(char::is_control)
    }
    pub fn zoom(&self, dpi: f64) -> Result<f64, String> {
        let zoom = self.pixels_per_mm * 25.4 / dpi;
        if !self.valid() || !dpi.is_finite() || dpi <= 0. || !(0.01..=64.).contains(&zoom) {
            return Err("Physical-size view exceeds the supported view zoom for this DPI".into());
        }
        Ok(zoom)
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Options {
    pub include_drafts: bool,
    pub include_text: bool,
    pub crop_guides: bool,
    pub registration: bool,
    pub registration_inset_mm: f64,
    pub channel: Channel,
    pub profile: Option<PathBuf>,
    pub intent: Intent,
    pub calibration: Option<Calibration>,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            include_drafts: false,
            include_text: true,
            crop_guides: false,
            registration: false,
            registration_inset_mm: 5.,
            channel: Channel::Composite,
            profile: None,
            intent: Intent::Relative,
            calibration: None,
        }
    }
}
impl Options {
    pub fn valid(&self) -> bool {
        self.registration_inset_mm.is_finite()
            && (3.0..=100.).contains(&self.registration_inset_mm)
            && self
                .profile
                .as_ref()
                .is_none_or(|p| crate::file_browser::lexical(p).is_ok())
            && self.calibration.as_ref().is_none_or(Calibration::valid)
    }
    pub fn same_content(&self, other: &Self) -> bool {
        (self.include_drafts, self.include_text, self.crop_guides)
            == (other.include_drafts, other.include_text, other.crop_guides)
    }
}
pub fn compose(
    d: &Document,
    options: &Options,
    dpi: f64,
    budget: usize,
    c: &AtomicBool,
) -> Result<Vec<Page>, String> {
    if !options.valid() || d.memory_bytes().saturating_mul(3) > budget.min(B) {
        return Err("Print content options/snapshot exceed the workspace budget".into());
    }
    let mut state = d.snapshot();
    state.scene.include_drafts_in_output = options.include_drafts;
    state.scene.comics.page.include_guides = options.crop_guides;
    if !options.include_text {
        state.scene.comics.page.include_number = false;
        for target in properties::targets(d) {
            let key = crate::layer_catalog::key(d, target);
            let text = crate::layer_catalog::source_index(d, key)
                .is_some_and(|n| matches!(d.scene.objects.get(n), Some(PageObject::Text { .. })))
                || d.scene
                    .catalog
                    .info(key)
                    .is_some_and(|i| i.kind == Some(crate::layer_search::Kind::Text));
            if text {
                let mut p = properties::properties(d, target);
                p.hidden = true;
                properties::set_state(d, &mut state, target, p)?;
            }
        }
    }
    let copy = crate::layer_merge::preview(&state, d.current_frame)?;
    book_io::compose(&[&copy], dpi, budget.saturating_sub(copy.memory_bytes()), c)
}
pub fn profile_bytes(options: &Options, enabled: bool) -> Result<Option<Vec<u8>>, String> {
    if !enabled {
        return Ok(None);
    }
    let Some(path) = &options.profile else {
        return Ok(None);
    };
    let path = crate::file_browser::checked(path)?;
    let bytes = crate::asset_catalog::bytes(&path, 4 * 1024 * 1024)?;
    crate::color_profile::RgbProfile::embedded("Print RGB ICC".into(), bytes.clone())?;
    Ok(Some(bytes))
}
pub fn mapped(
    image: &LocalImage,
    options: &Options,
    icm: bool,
    proof: bool,
    c: &AtomicBool,
) -> Result<LocalImage, String> {
    if !image.valid() || !options.valid() {
        return Err("Invalid print source/options".into());
    }
    let stage = if proof {
        profile_bytes(options, icm)?
            .map(|bytes| {
                use crate::color_profile::{RgbProfile, Transform};
                let target = RgbProfile::embedded("Print proof RGB ICC".into(), bytes)?;
                Ok::<_, String>((
                    Transform::with_intent(&RgbProfile::Srgb, &target, options.intent)?,
                    Transform::with_intent(&target, &RgbProfile::Srgb, options.intent)?,
                ))
            })
            .transpose()?
    } else {
        None
    };
    let mut pixels = Vec::with_capacity(image.pixels.len());
    for row in image.pixels.chunks(image.width as usize) {
        book_io::check_cancel(c)?;
        for p in row {
            let mut value = match options.channel {
                Channel::Composite => *p,
                Channel::Red | Channel::Green | Channel::Blue => {
                    let n = match options.channel {
                        Channel::Red => 0,
                        Channel::Green => 1,
                        _ => 2,
                    };
                    [p[n], p[n], p[n], p[3]]
                }
                Channel::Alpha => [255 - p[3], 255 - p[3], 255 - p[3], 255],
            };
            if let Some((to_target, from_target)) = &stage {
                let quantized = to_target
                    .map(crate::color_precision::Sample::from8(value))?
                    .rgba8();
                value = from_target
                    .map(crate::color_precision::Sample::from8(quantized))?
                    .rgba8();
            }
            pixels.push(value);
        }
    }
    Ok(LocalImage {
        width: image.width,
        height: image.height,
        pixels,
    })
}
pub fn pages(
    pages: &[Page],
    options: &Options,
    icm: bool,
    proof: bool,
    c: &AtomicBool,
) -> Result<Vec<Page>, String> {
    book_io::validate(pages, book_io::MAX_BYTES)?;
    pages
        .iter()
        .map(|p| {
            Ok(Page {
                image: mapped(&p.image, options, icm, proof, c)?,
                dpi: p.dpi,
            })
        })
        .collect()
}
pub fn apply_view(d: &mut Document, calibration: &Calibration, ppp: f64) -> Result<(), String> {
    if !ppp.is_finite() || ppp <= 0. {
        return Err("Invalid display scale".into());
    }
    let zoom = calibration.zoom(d.dpi)?;
    let size = [d.width as f64, d.height as f64];
    let center = d.view.screen_to_document([0.; 2], size, [0.; 2], ppp);
    d.view.zoom = zoom;
    d.view.center_on(center, size, [0.; 2], ppp);
    Ok(())
}
/// Physical crosses in the margin, independent of document guide visibility.
pub fn registration_lines(paper: [f64; 2], inset: f64) -> Vec<([f64; 2], [f64; 2])> {
    let mut lines = vec![];
    for center in [
        [paper[0] / 2., inset],
        [paper[0] / 2., paper[1] - inset],
        [inset, paper[1] / 2.],
        [paper[0] - inset, paper[1] / 2.],
    ] {
        for axis in 0..2 {
            let mut a = center;
            let mut b = center;
            a[axis] -= 2.;
            b[axis] += 2.;
            if a[0] >= 0. && a[1] >= 0. && b[0] <= paper[0] && b[1] <= paper[1] {
                lines.push((a, b));
            }
        }
    }
    lines
}
pub fn draw_registration(page: &mut Page, inset: f64) {
    let size = [
        page.image.width as f64 / page.dpi * 25.4,
        page.image.height as f64 / page.dpi * 25.4,
    ];
    for (a, b) in registration_lines(size, inset) {
        let a = a.map(|v| v / 25.4 * page.dpi);
        let b = b.map(|v| v / 25.4 * page.dpi);
        let steps = (b[0] - a[0]).abs().max((b[1] - a[1]).abs()).ceil().max(1.) as usize;
        for n in 0..=steps {
            let t = n as f64 / steps as f64;
            let x = (a[0] + (b[0] - a[0]) * t).round() as i64;
            let y = (a[1] + (b[1] - a[1]) * t).round() as i64;
            if x >= 0 && y >= 0 && x < page.image.width as i64 && y < page.image.height as i64 {
                page.image.pixels[y as usize * page.image.width as usize + x as usize] =
                    [0, 0, 0, 255];
            }
        }
    }
}
pub fn gdi_intent(intent: Intent) -> u32 {
    match intent {
        Intent::Saturation => 1,
        Intent::Relative => 2,
        Intent::Perceptual => 4,
        Intent::Absolute => 8,
    }
}

pub struct FrozenProfile {
    path: PathBuf,
    file: Option<std::fs::File>,
    pub sha256: String,
}
impl FrozenProfile {
    pub fn capture(root: &Path, options: &Options, enabled: bool) -> Result<Option<Self>, String> {
        use std::{
            fs,
            io::Write,
            sync::atomic::{AtomicU64, Ordering},
        };
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let Some(bytes) = profile_bytes(options, enabled)? else {
            return Ok(None);
        };
        let root = crate::file_browser::folder(root)?;
        let path = root.join(format!(
            ".drawer-print-{}-{}.icc",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let mut f = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|e| e.to_string())?;
        let saved = (|| {
            f.write_all(&bytes).map_err(|e| e.to_string())?;
            f.sync_all().map_err(|e| e.to_string())
        })();
        drop(f);
        if let Err(e) = saved {
            let _ = fs::remove_file(&path);
            return Err(e);
        }
        let mut options = fs::OpenOptions::new();
        options.read(true);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options.share_mode(1);
        }
        let file = match options.open(&path) {
            Ok(f) => f,
            Err(e) => {
                let _ = fs::remove_file(&path);
                return Err(e.to_string());
            }
        };
        Ok(Some(Self {
            path,
            file: Some(file),
            sha256: crate::asset_catalog::hash(&bytes),
        }))
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
}
impl Drop for FrozenProfile {
    fn drop(&mut self) {
        self.file = None;
        let _ = std::fs::remove_file(&self.path);
    }
}
#[path = "print_options_verify.rs"]
pub mod verify;
