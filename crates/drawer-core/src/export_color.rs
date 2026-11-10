//! Explicit RGB output transforms, profile tagging and view-only output simulation.
use crate::{
    color_precision::Sample,
    color_profile::{RgbProfile, Transform},
    document::Document,
    raster_io::Format,
    soft_proof::Intent,
};
use std::{path::PathBuf, sync::Arc};
#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    pub convert: bool,
    pub embed: bool,
    pub profile: Option<PathBuf>,
    pub intent: Intent,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            convert: true,
            embed: true,
            profile: None,
            intent: Intent::Relative,
        }
    }
}
pub fn supports_profile(f: Format) -> bool {
    matches!(f, Format::Png | Format::Jpeg | Format::Tiff | Format::Webp)
}
pub fn profile_bytes(p: &RgbProfile) -> Result<Vec<u8>, String> {
    if let RgbProfile::Embedded { bytes, .. } = p {
        return Ok(bytes.clone());
    }
    let mut bytes = moxcms::ColorProfile::new_srgb()
        .encode()
        .map_err(|e| e.to_string())?;
    for (slot, value) in bytes[24..36]
        .as_chunks_mut::<2>()
        .0
        .iter_mut()
        .zip([2000u16, 1, 1, 0, 0, 0])
    {
        slot.copy_from_slice(&value.to_be_bytes());
    }
    Ok(bytes)
}
pub(crate) struct Stage {
    transform: Transform,
    pub display: Arc<Transform>,
    pub from_srgb: Arc<Transform>,
    pub icc: Option<Arc<Vec<u8>>>,
    pub label: String,
    pub notices: Vec<String>,
    pub losses: Vec<String>,
    target: RgbProfile,
    intent: Intent,
}
impl Stage {
    pub fn new(d: &Document, s: &Settings, format: Format) -> Result<Self, String> {
        let embed = s.embed && supports_profile(format);
        let target = if (s.convert || embed)
            && let Some(path) = &s.profile
        {
            crate::file_objects::local_path(path)?;
            let parent = path.parent().ok_or("Missing ICC folder")?;
            let path = crate::asset_catalog::checked(
                parent,
                std::path::Path::new(path.file_name().ok_or("Missing ICC filename")?),
                true,
            )?;
            let bytes = crate::monitor_color::read_profile(&path)?;
            if bytes.get(16..20) == Some(b"CMYK") {
                return Err("CMYK output channels are unsupported by these still-image adapters. Use an RGB output ICC; CMYK is available for view-only soft proof.".into());
            }
            RgbProfile::embedded("Selected output RGB ICC".into(), bytes)?
        } else if s.convert {
            RgbProfile::Srgb
        } else {
            d.color.profile.clone()
        };
        let intent = if s.convert {
            s.intent
        } else {
            Intent::Relative
        };
        let transform = Transform::with_intent(
            &d.color.profile,
            if s.convert { &target } else { &d.color.profile },
            intent,
        )?;
        let display = Arc::new(Transform::with_intent(&target, &RgbProfile::Srgb, intent)?);
        let icc = if embed {
            Some(Arc::new(profile_bytes(&target)?))
        } else {
            None
        };
        let mut losses = vec![];
        if s.convert && !d.color.profile.equivalent(&target) {
            losses.push(format!(
                "Color space: source {} RGB is converted to {}; numeric colors can change",
                d.color.profile.label(),
                target.label()
            ));
        } else if !s.convert && embed && !d.color.profile.equivalent(&target) {
            losses.push("Color space: different ICC assignment preserves numeric RGB but changes its interpretation".into());
        }
        if !embed && !target.equivalent(&RgbProfile::Srgb) {
            losses.push(
                "Color space: omitting the non-sRGB ICC loses the output RGB interpretation".into(),
            );
        }
        let mut notices = vec![
            if s.convert {
                format!(
                    "RGB conversion: {} → {}; rendering intent {:?}",
                    d.color.profile.label(),
                    target.label(),
                    s.intent
                )
            } else {
                "No pixel color conversion. Numeric working RGB is preserved; assigning a different ICC can change its interpreted appearance.".into()
            },
            if embed {
                "Output ICC included (independent of monitor calibration)".into()
            } else {
                "Output ICC omitted; other viewers may assume sRGB".into()
            },
        ];
        if s.embed && !supports_profile(format) {
            notices.push(format!("{:?} cannot embed an ICC in this adapter; the saved include option is inactive for this format",format));
        }
        Ok(Self {
            from_srgb: Arc::new(Transform::with_intent(&RgbProfile::Srgb, &target, intent)?),
            transform,
            display,
            icc,
            label: target.label().into(),
            notices,
            losses,
            target,
            intent,
        })
    }
    pub fn map(&self, p: Sample) -> Result<Sample, String> {
        self.transform.map(p)
    }
    pub fn matte(&self, rgb: [u8; 3]) -> Result<Sample, String> {
        Transform::with_intent(&RgbProfile::Srgb, &self.target, self.intent)?
            .map(Sample::from8([rgb[0], rgb[1], rgb[2], 255]))
    }
}
#[path = "export_color_verify.rs"]
pub mod verify;
