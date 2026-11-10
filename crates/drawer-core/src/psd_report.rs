//! Per-file interchange outcomes; extension spelling never identifies the wire format.
use super::*;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResultKind {
    Preserved,
    Rasterized,
    Unsupported,
}
impl ResultKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Preserved => "Preserved",
            Self::Rasterized => "Raster replacement",
            Self::Unsupported => "Unsupported",
        }
    }
}
#[derive(Clone, Debug)]
pub struct Outcome {
    pub feature: String,
    pub result: ResultKind,
    pub detail: String,
}
#[derive(Clone, Debug)]
pub struct Report {
    pub format: Format,
    pub width: u32,
    pub height: u32,
    pub outcomes: Vec<Outcome>,
    pub notes: Vec<String>,
}
pub fn header(bytes: &[u8]) -> Result<Format, String> {
    if bytes.get(..4) != Some(b"8BPS") {
        return Err("Invalid PSD/PSB signature".into());
    }
    match bytes.get(4..6) {
        Some([0, 1]) => Ok(Format::Psd),
        Some([0, 2]) => Ok(Format::Psb),
        _ => Err("Unsupported or truncated PSD/PSB version".into()),
    }
}
impl Format {
    pub fn label(self) -> &'static str {
        match self {
            Self::Psd => "PSD v1 (32-bit section/channel lengths)",
            Self::Psb => "PSB v2 (64-bit section/channel lengths)",
        }
    }
}
impl Report {
    pub(super) fn image(image: &Image, format: Format, substitute: bool, notes: &[String]) -> Self {
        let mut r = Self {
            format,
            width: image.width,
            height: image.height,
            outcomes: vec![],
            notes: notes.to_vec(),
        };
        r.add(
            "Composite / RGB8 / DPI",
            ResultKind::Preserved,
            "sRGB pixel composite and resolution",
        );
        let result = if substitute {
            ResultKind::Rasterized
        } else {
            ResultKind::Preserved
        };
        for layer in &image.layers {
            if layer.section == 3 {
                continue;
            }
            let kind = if layer.section == 0 {
                "Raster layer"
            } else {
                "Group"
            };
            r.add(
                format!("{kind}: {}", layer.name),
                result,
                if substitute {
                    "Only the merged appearance; separate editing lost"
                } else {
                    "Name, bounds, order, opacity, visibility, clipping and alpha lock"
                },
            );
            r.add(
                format!("Blend: {}", layer.name),
                result,
                format!(
                    "{} ({})",
                    String::from_utf8_lossy(&layer.blend),
                    if layer.blend == *b"pass" {
                        "pass-through group"
                    } else if blend_mode(layer.blend).is_ok() {
                        "verified RGB8 key"
                    } else {
                        "unsupported key; merged appearance only"
                    }
                ),
            );
            if let Some(mask) = &layer.mask {
                r.add(format!("Raster mask: {}",layer.name),result,format!("Bounds {:?}; default {}; flags {}; {} samples; PSD link/transform editing is not retained",mask.bounds,mask.default,mask.flags,mask.pixels.len()));
            }
        }
        r.add("Unverified editing / input variants",ResultKind::Unsupported,"CMYK/Lab/indexed/Gray/16/32-bit and ZIP/prediction input are refused. Editable vectors, text, smart objects and effects require cached raster or the merged substitute; mask density/feather/real-mask parameters require merged substitution.");
        r
    }
    fn add(&mut self, feature: impl Into<String>, result: ResultKind, detail: impl Into<String>) {
        self.outcomes.push(Outcome {
            feature: feature.into(),
            result,
            detail: detail.into(),
        });
    }
    pub fn lines(&self) -> Vec<String> {
        let mut lines = vec![format!(
            "{} — {} × {}",
            self.format.label(),
            self.width,
            self.height
        )];
        lines.extend(
            self.outcomes
                .iter()
                .map(|o| format!("{} | {} | {}", o.result.label(), o.feature, o.detail)),
        );
        lines.extend(self.notes.iter().map(|n| format!("Conversion note: {n}")));
        lines
    }
}
impl Plan {
    pub fn report_with_preview(
        &self,
        format: Format,
        flatten: bool,
        merged_preview: bool,
    ) -> Result<Report, String> {
        let mut r = self.report(format, flatten)?;
        if !merged_preview {
            r.outcomes[0].result = ResultKind::Unsupported;
            r.outcomes[0].detail="Real merged preview omitted; required image data is an explicitly flagged placeholder. Verified raster layers retain the appearance; readers requiring a real merged image may refuse it.".into();
        }
        Ok(r)
    }
}
impl Plan {
    pub fn report(&self, format: Format, flatten: bool) -> Result<Report, String> {
        let image = if flatten {
            &self.flattened
        } else {
            self.partial
                .as_ref()
                .ok_or("Partial exchange is unavailable; review whole-composite replacement")?
        };
        let mut report = Report::image(image, format, false, &self.losses);
        let ranges = if flatten {
            vec![self.whole_names.clone()]
        } else {
            self.ranges.clone()
        };
        for range in ranges {
            report.add(
                range.join(" → "),
                ResultKind::Rasterized,
                "Original separate editing replaced by the confirmed raster dependency range",
            );
        }
        if let Some(reason) = &self.reason {
            report.notes.push(reason.clone());
        }
        Ok(report)
    }
}
pub fn verify(root: &Path) -> Result<(), String> {
    use crate::document::DEFAULT_DOCUMENT_BUDGET as B;
    let cancel = AtomicBool::new(false);
    let d = verify::fixture()?;
    let before = d.snapshot();
    let plan = prepare(&d, B, &cancel)?;
    for format in [Format::Psd, Format::Psb] {
        let image = plan.partial.as_ref().ok_or("Missing editable fixture")?;
        let bytes = io::encode(image, format, B, &cancel)?;
        if header(&bytes)? != format {
            return Err("Header format mismatch".into());
        }
        // Intentionally opposite extension: the header must win.
        let path = root.join(if format == Format::Psd {
            "report-disguised.psb"
        } else {
            "report-disguised.psd"
        });
        std::fs::write(&path, &bytes).map_err(|e| e.to_string())?;
        let (loaded, warnings, report) = super::open_detailed(&path, B)?;
        if !warnings.is_empty()
            || report.format != format
            || loaded.layers.len() != d.layers.len()
            || report
                .outcomes
                .iter()
                .filter(|o| {
                    o.feature.starts_with("Raster mask:") && o.result == ResultKind::Preserved
                })
                .count()
                != 1
        {
            return Err("Per-file preserved structure report mismatch".into());
        }
        let blends = report
            .outcomes
            .iter()
            .filter(|o| o.feature.starts_with("Blend:"))
            .count();
        if blends != 4
            || !plan
                .report(format, true)?
                .outcomes
                .iter()
                .any(|o| o.result == ResultKind::Rasterized)
        {
            return Err("Blend/raster outcome mismatch".into());
        }
        if io::decode(&bytes, 0, &cancel).is_ok() {
            return Err("PSD report ignored memory limit".into());
        }
        if format == Format::Psb {
            let resources = u32::from_be_bytes(bytes[30..34].try_into().unwrap()) as usize;
            let mut huge = bytes.clone();
            let at = 34 + resources;
            huge[at..at + 8].copy_from_slice(&u64::MAX.to_be_bytes());
            if io::decode(&huge, B, &cancel).is_ok() {
                return Err("Huge PSB section length accepted".into());
            }
        }
        let mut unsupported = bytes.clone();
        let at = unsupported
            .windows(4)
            .position(|w| w == b"luni")
            .ok_or("Missing layer block")?;
        unsupported[at..at + 4].copy_from_slice(b"vmsk");
        std::fs::write(&path, &unsupported).map_err(|e| e.to_string())?;
        let (_, warnings, report) = super::open_detailed(&path, B)?;
        if !warnings.iter().any(|w| w.contains("Whole merged"))
            || !report.outcomes.iter().any(|o| {
                o.feature.starts_with("Raster mask:") && o.result == ResultKind::Rasterized
            })
        {
            return Err("Unsupported mask falsely reported preserved".into());
        }
        let mut unknown = bytes.clone();
        let blend_at = unknown
            .windows(4)
            .position(|w| w == b"norm")
            .ok_or("Missing blend key")?;
        unknown[blend_at..blend_at + 4].copy_from_slice(b"zzzz");
        std::fs::write(&path, &unknown).map_err(|e| e.to_string())?;
        let (_, _, report) = super::open_detailed(&path, B)?;
        if !report.outcomes.iter().any(|o| {
            o.feature.starts_with("Blend:")
                && o.result == ResultKind::Rasterized
                && o.detail.contains("unsupported key")
        }) {
            return Err("Unknown blend was described as verified".into());
        }
        std::fs::remove_file(path).map_err(|e| e.to_string())?;
    }
    let supported = Mode::ALL
        .into_iter()
        .filter(|m| blend_key(*m).is_some())
        .count();
    if supported != 25 || d.snapshot() != before || header(b"8BPS\0\x03").is_ok() {
        return Err("Verified mode list / source changed".into());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn headers_results_disguised_extensions_masks_modes_and_huge_psb_lengths() {
        let t = crate::test_support::TempDir::new();
        super::verify(&t.0).unwrap();
    }
}
