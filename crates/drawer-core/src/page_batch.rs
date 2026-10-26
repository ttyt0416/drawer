//! Immutable local input snapshots and atomic prepared page batches.
use crate::{
    book_io::{self, Page},
    document::{DEFAULT_DOCUMENT_BUDGET, Document},
    sizing::{self, Change},
};
use sha2::{Digest, Sha256};
use std::{cmp::Ordering, path::PathBuf, sync::atomic::AtomicBool};
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Size {
    #[default]
    Original,
    Fit {
        width: u32,
        height: u32,
        dpi: f64,
    },
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stamp {
    pub path: PathBuf,
    pub hash: [u8; 32],
}
pub struct Prepared {
    pub pages: Vec<Document>,
    pub sources: Vec<Stamp>,
    pub rejected: Vec<(PathBuf, String)>,
}
impl Prepared {
    pub fn fresh(&self) -> Result<(), String> {
        for s in &self.sources {
            let bytes = book_io::read_local(&s.path, 64 * 1024 * 1024)?;
            if <[u8; 32]>::from(Sha256::digest(&bytes)) != s.hash {
                return Err(format!("Source changed: {}", s.path.display()));
            }
        }
        Ok(())
    }
}
fn natural(a: &str, b: &str) -> Ordering {
    let a = a.to_lowercase();
    let b = b.to_lowercase();
    let (a, b) = (a.as_bytes(), b.as_bytes());
    let (mut i, mut j) = (0, 0);
    while i < a.len() && j < b.len() {
        if a[i].is_ascii_digit() && b[j].is_ascii_digit() {
            let (ai, bj) = (i, j);
            while i < a.len() && a[i].is_ascii_digit() {
                i += 1;
            }
            while j < b.len() && b[j].is_ascii_digit() {
                j += 1;
            }
            let x = &a[ai..i];
            let y = &b[bj..j];
            let xz = x.iter().position(|c| *c != b'0').unwrap_or(x.len());
            let yz = y.iter().position(|c| *c != b'0').unwrap_or(y.len());
            let cmp = (x.len() - xz)
                .cmp(&(y.len() - yz))
                .then_with(|| x[xz..].cmp(&y[yz..]))
                .then_with(|| x.len().cmp(&y.len()));
            if cmp != Ordering::Equal {
                return cmp;
            }
        } else {
            let cmp = a[i].cmp(&b[j]);
            if cmp != Ordering::Equal {
                return cmp;
            }
            i += 1;
            j += 1;
        }
    }
    a.len().cmp(&b.len())
}
pub fn natural_sort(paths: &mut [PathBuf]) {
    paths.sort_by(|a, b| {
        natural(
            &a.file_name().unwrap_or_default().to_string_lossy(),
            &b.file_name().unwrap_or_default().to_string_lossy(),
        )
        .then_with(|| a.cmp(b))
    });
}
fn square(x: f64, y: f64) -> Result<f64, String> {
    if !x.is_finite() || !y.is_finite() || x <= 0. || y <= 0. || (x - y).abs() > 1e-4 * x.max(y) {
        return Err("Unsupported non-square or invalid source DPI".into());
    }
    Ok(x)
}
fn tiff_dpi(bytes: &[u8], page: usize) -> Result<Option<f64>, String> {
    let little = match bytes.get(..4) {
        Some(b"II\x2a\0") => true,
        Some(b"MM\0\x2a") => false,
        _ => return Ok(None),
    };
    let u16at = |p: usize| -> Result<u16, String> {
        let b = bytes
            .get(p..p + 2)
            .ok_or("Truncated resolution metadata")?
            .try_into()
            .unwrap();
        Ok(if little {
            u16::from_le_bytes(b)
        } else {
            u16::from_be_bytes(b)
        })
    };
    let u32at = |p: usize| -> Result<u32, String> {
        let b = bytes
            .get(p..p + 4)
            .ok_or("Truncated resolution metadata")?
            .try_into()
            .unwrap();
        Ok(if little {
            u32::from_le_bytes(b)
        } else {
            u32::from_be_bytes(b)
        })
    };
    let mut offset = u32at(4)? as usize;
    for n in 0..=page {
        let count = u16at(offset)? as usize;
        if count > 4096 {
            return Err("TIFF resolution directory limit".into());
        }
        let mut xy = [None, None];
        let mut unit = 2;
        for k in 0..count {
            let p = offset + 2 + k * 12;
            let tag = u16at(p)?;
            if tag == 296 && u16at(p + 2)? == 3 && u32at(p + 4)? == 1 {
                unit = u16at(p + 8)?;
            }
            if [282, 283].contains(&tag) && u16at(p + 2)? == 5 && u32at(p + 4)? == 1 {
                let at = u32at(p + 8)? as usize;
                let denominator = u32at(at + 4)?;
                if denominator == 0 {
                    return Err("Zero resolution denominator".into());
                }
                xy[(tag - 282) as usize] = Some(u32at(at)? as f64 / denominator as f64);
            }
        }
        if n == page {
            if !(1..=3).contains(&unit) {
                return Err("Unsupported TIFF/EXIF resolution unit".into());
            }
            return match xy {
                [Some(x), Some(y)] => {
                    if unit == 1 {
                        Ok(None)
                    } else {
                        square(x, y).map(|v| Some(v * if unit == 3 { 2.54 } else { 1. }))
                    }
                }
                [None, None] => Ok(None),
                _ => Err("Incomplete source DPI metadata".into()),
            };
        }
        offset = u32at(offset + 2 + count * 12)? as usize;
        if offset == 0 {
            return Err("TIFF resolution page missing".into());
        }
    }
    Ok(None)
}
pub fn image_dpi(bytes: &[u8], page: usize) -> Result<f64, String> {
    let mut dpi = None;
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        let mut at = 8usize;
        while at + 12 <= bytes.len() {
            let len = u32::from_be_bytes(bytes[at..at + 4].try_into().unwrap()) as usize;
            let end = at
                .checked_add(12)
                .and_then(|n| n.checked_add(len))
                .ok_or("PNG metadata size overflow")?;
            if end > bytes.len() {
                return Err("Truncated PNG metadata".into());
            }
            if &bytes[at + 4..at + 8] == b"pHYs" && len == 9 && bytes[at + 16] == 1 {
                dpi = Some(
                    square(
                        u32::from_be_bytes(bytes[at + 8..at + 12].try_into().unwrap()) as f64,
                        u32::from_be_bytes(bytes[at + 12..at + 16].try_into().unwrap()) as f64,
                    )? * 0.0254,
                );
                break;
            }
            at = end;
        }
    } else if bytes.starts_with(b"BM") && bytes.len() >= 46 {
        let x = i32::from_le_bytes(bytes[38..42].try_into().unwrap());
        let y = i32::from_le_bytes(bytes[42..46].try_into().unwrap());
        if x > 0 && y > 0 {
            dpi = Some(square(x as f64, y as f64)? * 0.0254);
        }
    } else if bytes.starts_with(b"\xff\xd8") {
        let mut at = 2usize;
        while at + 4 <= bytes.len() {
            if bytes[at] != 0xff {
                break;
            }
            let marker = bytes[at + 1];
            if marker == 0xda || marker == 0xd9 {
                break;
            }
            let len = u16::from_be_bytes([bytes[at + 2], bytes[at + 3]]) as usize;
            if len < 2 || at + 2 + len > bytes.len() {
                return Err("Truncated JPEG metadata".into());
            }
            let data = &bytes[at + 4..at + 2 + len];
            if marker == 0xe0
                && data.starts_with(b"JFIF\0")
                && data.len() >= 12
                && [1, 2].contains(&data[7])
            {
                dpi = Some(
                    square(
                        u16::from_be_bytes([data[8], data[9]]) as f64,
                        u16::from_be_bytes([data[10], data[11]]) as f64,
                    )? * if data[7] == 2 { 2.54 } else { 1. },
                );
            }
            if marker == 0xe1
                && data.starts_with(b"Exif\0\0")
                && let Some(value) = tiff_dpi(&data[6..], 0)?
            {
                dpi = Some(value);
                break;
            }
            at += 2 + len;
        }
    } else {
        dpi = tiff_dpi(bytes, page)?;
    }
    let dpi = dpi.unwrap_or(300.);
    if !dpi.is_finite() || dpi <= 0. {
        return Err("Invalid source DPI".into());
    }
    Ok(dpi)
}
pub fn prepare(
    paths: &[PathBuf],
    size: Size,
    budget: usize,
    cancel: &AtomicBool,
    pdf: &mut impl FnMut(&[u8], usize, &AtomicBool) -> Result<Vec<Page>, String>,
) -> Result<Prepared, String> {
    if paths.is_empty() || paths.len() > 512 {
        return Err("Choose 1..512 local inputs".into());
    }
    let mut result = Prepared {
        pages: vec![],
        sources: vec![],
        rejected: vec![],
    };
    let mut used = 0usize;
    for path in paths {
        book_io::check_cancel(cancel)?;
        let operation = (|| {
            let path = path.canonicalize().map_err(|e| e.to_string())?;
            if result.sources.iter().any(|s| s.path == path) {
                return Err("Duplicate input file; only the first occurrence is prepared".into());
            }
            let remaining = budget.saturating_sub(used);
            let bytes = book_io::read_local(&path, (64 * 1024 * 1024).min(remaining / 2))?;
            let hash = Sha256::digest(&bytes).into();
            let ext = path
                .extension()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            let mut pages = match ext.as_str() {
                "drawer" => vec![
                    crate::files::read_native_snapshot(
                        &path,
                        &bytes,
                        remaining.saturating_sub(bytes.len()),
                    )
                    .map_err(|e| e.to_string())?,
                ],
                "epub" => crate::book_epub::decode(&bytes, "", remaining / 2, cancel)?
                    .into_iter()
                    .map(|p| p.document(remaining))
                    .collect::<Result<Vec<_>, _>>()?,
                "pdf" => pdf(&bytes, remaining / 2, cancel)?
                    .into_iter()
                    .map(|p| p.document(remaining))
                    .collect::<Result<Vec<_>, _>>()?,
                _ => {
                    let source = crate::image_import::ImageSource::from_encoded(
                        path.clone(),
                        bytes,
                        remaining,
                        cancel,
                    )
                    .map_err(|e| e.to_string())?;
                    let mut pages = vec![];
                    let mut local = source.encoded_snapshot().len();
                    for n in 0..source.pages.len() {
                        book_io::check_cancel(cancel)?;
                        let image = source
                            .decode(n, true, remaining.saturating_sub(local) / 2, cancel)
                            .map_err(|e| e.to_string())?
                            .image;
                        let mut d = image
                            .document(remaining.saturating_sub(local) / 2)
                            .map_err(|e| e.to_string())?;
                        d.dpi = image_dpi(&source.encoded_snapshot(), n)?;
                        local = local.saturating_add(d.resident_bytes());
                        pages.push(d);
                    }
                    pages
                }
            };
            for d in &mut pages {
                book_io::check_cancel(cancel)?;
                if let Size::Fit { width, height, dpi } = size {
                    let physical = [
                        width as f64 / d.width as f64 * d.dpi / dpi,
                        height as f64 / d.height as f64 * d.dpi / dpi,
                    ];
                    let state = sizing::prepare(
                        &d.snapshot(),
                        Change::Resample {
                            width,
                            height,
                            dpi,
                            filter: crate::interpolation::Filter::Bilinear,
                        },
                        remaining,
                        cancel,
                    )?;
                    state.restore(d);
                    for (n, scale) in physical.into_iter().enumerate() {
                        d.scene.comics.page.trim_mm[n] *= scale;
                    }
                    let margin = physical[0].min(physical[1]);
                    d.scene.comics.page.bleed_mm *= margin;
                    d.scene.comics.page.safe_mm *= margin;
                }
                d.title = path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned();
                d.source_path = Some(path.clone());
            }
            let bytes = pages.iter().map(Document::resident_bytes).sum::<usize>();
            if used.saturating_add(bytes) > budget
                || result.pages.len().saturating_add(pages.len()) > 512
            {
                return Err("Prepared page count/memory limit".into());
            }
            used += bytes;
            result.pages.append(&mut pages);
            result.sources.push(Stamp { path, hash });
            Ok(())
        })();
        if let Err(error) = operation {
            book_io::check_cancel(cancel)?;
            result.rejected.push((path.clone(), error));
        }
    }
    book_io::check_cancel(cancel)?;
    Ok(result)
}
pub fn default_budget() -> usize {
    DEFAULT_DOCUMENT_BUDGET
}
