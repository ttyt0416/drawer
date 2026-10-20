//! View-only ICC round-trip simulation and separately committed sRGB tone edits.
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
};
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Intent {
    #[default]
    Perceptual,
    Relative,
    Saturation,
    Absolute,
}
impl Intent {
    fn cms(self) -> moxcms::RenderingIntent {
        match self {
            Self::Perceptual => moxcms::RenderingIntent::Perceptual,
            Self::Relative => moxcms::RenderingIntent::RelativeColorimetric,
            Self::Saturation => moxcms::RenderingIntent::Saturation,
            Self::Absolute => moxcms::RenderingIntent::AbsoluteColorimetric,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tone {
    pub curve: [f64; 5],
    pub black: f64,
    pub white: f64,
    pub gamma: f64,
    pub output: [f64; 2],
}
impl Default for Tone {
    fn default() -> Self {
        Self {
            curve: [0.0, 0.25, 0.5, 0.75, 1.0],
            black: 0.0,
            white: 1.0,
            gamma: 1.0,
            output: [0.0, 1.0],
        }
    }
}
impl Tone {
    pub fn valid(self) -> bool {
        self.curve
            .iter()
            .chain(&self.output)
            .chain([&self.black, &self.white])
            .all(|v| v.is_finite() && (0.0..=1.0).contains(v))
            && self.white - self.black >= 0.0001
            && self.output[0] <= self.output[1]
            && self.gamma.is_finite()
            && (0.1..=10.0).contains(&self.gamma)
    }
    pub fn map_float(
        self,
        color: crate::color_precision::Sample,
    ) -> Result<crate::color_precision::Sample, String> {
        if !self.valid() {
            return Err("Invalid curve or levels".into());
        }
        if self == Self::default() || color.alpha() == 0. {
            return Ok(color);
        }
        color.with_rgb(color.rgb().map(|v| {
            let x = ((v - self.black) / (self.white - self.black))
                .clamp(0., 1.)
                .powf(1. / self.gamma)
                * 4.;
            let i = (x.floor() as usize).min(3);
            let y = self.curve[i] + (self.curve[i + 1] - self.curve[i]) * (x - i as f64);
            self.output[0] + y * (self.output[1] - self.output[0])
        }))
    }
    fn table(self) -> [u8; 256] {
        std::array::from_fn(|i| self.map([i as u8, i as u8, i as u8, 255])[0])
    }
    pub fn map(self, color: [u8; 4]) -> [u8; 4] {
        if color[3] == 0 || self == Self::default() {
            return color;
        }
        let mut c = color;
        for n in 0..3 {
            let x = ((color[n] as f64 / 255.0 - self.black) / (self.white - self.black))
                .clamp(0.0, 1.0)
                .powf(1.0 / self.gamma)
                * 4.0;
            let i = (x.floor() as usize).min(3);
            let y = self.curve[i] + (self.curve[i + 1] - self.curve[i]) * (x - i as f64);
            c[n] = ((self.output[0] + y * (self.output[1] - self.output[0])) * 255.0)
                .round()
                .clamp(0.0, 255.0) as u8;
        }
        c
    }
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub enabled: bool,
    pub profile: Option<PathBuf>,
    pub intent: Intent,
    pub tone: Tone,
}
impl Settings {
    pub fn valid(&self) -> bool {
        self.tone.valid()
            && self
                .profile
                .as_ref()
                .is_none_or(|p| p.is_absolute() && p.as_os_str().len() <= 4096)
    }
}
const N: usize = 33;
pub struct Proof {
    tone: [u8; 256],
    lut: Option<Vec<[u8; 3]>>,
}
fn read_profile(path: &Path) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    fs::File::open(path)
        .map_err(|e| format!("ICC {}: {e}", path.display()))?
        .take(4 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > 4 * 1024 * 1024 {
        return Err("ICC profile exceeds 4 MiB".into());
    }
    Ok(bytes)
}
impl Proof {
    pub fn read(settings: &Settings) -> Result<Self, String> {
        if !settings.valid() {
            return Err("Invalid proof curve/levels or ICC path".into());
        }
        if !settings.enabled {
            return Ok(Self {
                tone: Tone::default().table(),
                lut: None,
            });
        }
        match &settings.profile {
            Some(path) => Self::from_bytes(settings, &read_profile(path)?),
            None => Ok(Self {
                tone: settings.tone.table(),
                lut: None,
            }),
        }
    }
    pub fn from_bytes(settings: &Settings, bytes: &[u8]) -> Result<Self, String> {
        use moxcms::*;
        if !settings.valid() || bytes.len() > 4 * 1024 * 1024 {
            return Err("Invalid proof settings/profile size".into());
        }
        if !settings.enabled {
            return Ok(Self {
                tone: Tone::default().table(),
                lut: None,
            });
        }
        let target = ColorProfile::new_from_slice_with_options(
            bytes,
            ParsingOptions {
                max_profile_size: 4 * 1024 * 1024,
                max_allowed_clut_size: 4 * 1024 * 1024,
                max_allowed_trc_size: 1_000_000,
            },
        )
        .map_err(|e| format!("Invalid proof ICC: {e}"))?;
        let (layout, channels) = match target.color_space {
            DataColorSpace::Rgb => (Layout::Rgb, 3),
            DataColorSpace::Gray => (Layout::Gray, 1),
            DataColorSpace::Cmyk => (Layout::Rgba, 4),
            _ => return Err("Proof profile must be RGB, Gray or CMYK".into()),
        };
        let options = TransformOptions {
            rendering_intent: settings.intent.cms(),
            ..Default::default()
        };
        let working = ColorProfile::new_srgb();
        let forward = working
            .create_transform_8bit(Layout::Rgb, &target, layout, options)
            .map_err(|e| format!("No working-to-proof ICC transform: {e}"))?;
        let reverse = target
            .create_transform_8bit(layout, &working, Layout::Rgb, options)
            .map_err(|e| format!("No proof-to-working ICC transform: {e}"))?;
        let mut input = Vec::with_capacity(N * N * N * 3);
        for r in 0..N {
            for g in 0..N {
                for b in 0..N {
                    input.extend([axis(r), axis(g), axis(b)]);
                }
            }
        }
        let mut stage = vec![0; N * N * N * channels];
        let mut output = vec![0; input.len()];
        forward
            .transform(&input, &mut stage)
            .map_err(|e| e.to_string())?;
        reverse
            .transform(&stage, &mut output)
            .map_err(|e| e.to_string())?;
        Ok(Self {
            tone: settings.tone.table(),
            lut: Some(output.as_chunks::<3>().0.to_vec()),
        })
    }
    pub fn map(&self, c: [u8; 4]) -> [u8; 4] {
        if c[3] == 0 {
            return c;
        }
        let c = [
            self.tone[c[0] as usize],
            self.tone[c[1] as usize],
            self.tone[c[2] as usize],
            c[3],
        ];
        let Some(lut) = &self.lut else {
            return c;
        };
        let coordinates = [c[0], c[1], c[2]].map(|v| {
            let mut i = (v as usize * 32 / 255).min(31);
            if v < axis(i) {
                i = i.saturating_sub(1);
            } else if i < 31 && v > axis(i + 1) {
                i += 1;
            }
            let t = (v as f64 - axis(i) as f64) / (axis(i + 1) - axis(i)) as f64;
            (i, t)
        });
        let mut out = [0.0; 3];
        for r in 0..2 {
            for g in 0..2 {
                for b in 0..2 {
                    let offsets = [r, g, b];
                    let mut weight = 1.0;
                    let mut index = 0;
                    for n in 0..3 {
                        let (i, t) = coordinates[n];
                        weight *= if offsets[n] == 0 { 1.0 - t } else { t };
                        index = index * N + i + offsets[n];
                    }
                    for n in 0..3 {
                        out[n] += lut[index][n] as f64 * weight;
                    }
                }
            }
        }
        [
            out[0].round() as u8,
            out[1].round() as u8,
            out[2].round() as u8,
            c[3],
        ]
    }
}
fn axis(i: usize) -> u8 {
    ((i * 255 + 16) / 32) as u8
}
/// Bake tone/levels only into one selected raster. ICC proofing remains view-only.
pub fn apply(
    doc: &mut crate::document::Document,
    layer: usize,
    tone: Tone,
    cancel: impl Fn() -> bool,
) -> Result<bool, String> {
    if !tone.valid() {
        return Err("Invalid curve or levels".into());
    }
    let raster = doc.layers.get(layer).ok_or("Select a raster layer")?;
    if raster.locked {
        return Err("Selected raster layer is locked".into());
    }
    if doc.color != crate::color_storage::Config::legacy() {
        crate::layer_properties::paintable(doc, layer)?;
        if raster.pixels.len() > 4 * 1024 * 1024
            || doc.memory_bytes() > crate::document::DEFAULT_DOCUMENT_BUDGET / 8
        {
            return Err("Floating tone snapshot/work limit".into());
        }
        if tone == Tone::default() {
            return Ok(false);
        }
        let extent = raster.extent(doc.width, doc.height);
        let space = crate::color_profile::WorkingSpace::new(&doc.color.profile)?;
        let mut samples = Vec::with_capacity(raster.pixels.len());
        for n in 0..raster.pixels.len() {
            if n % 4096 == 0 && cancel() {
                return Err("Tone application cancelled; original pixels preserved".into());
            }
            let original = raster.sample_at(n);
            let point = [
                extent.offset[0] as f64 + (n % extent.width as usize) as f64 + 0.5,
                extent.offset[1] as f64 + (n / extent.width as usize) as f64 + 0.5,
            ];
            let factor = doc.scene.selection_coverage(point) as f64 / 255.;
            let source = space.to_srgb(original)?;
            let mapped = tone.map_float(source)?;
            samples.push(if mapped == source || factor == 0. {
                original
            } else {
                space.from_srgb(source.mix(mapped, factor)?)?
            });
        }
        if cancel() {
            return Err("Tone application cancelled; original pixels preserved".into());
        }
        let mut state = doc.snapshot();
        crate::color_storage::set_samples(&mut state.layers[layer], &doc.color, extent, samples)?;
        return doc.commit_state(state);
    }
    let extent = raster.extent(doc.width, doc.height);
    let table = tone.table();
    let mut changes = Vec::new();
    let budget = (crate::document::DEFAULT_DOCUMENT_BUDGET / 4)
        .min(crate::document::DEFAULT_DOCUMENT_BUDGET.saturating_sub(doc.resident_bytes()));
    for (offset, before) in raster.pixels.iter().copied().enumerate() {
        if offset % 4096 == 0 && cancel() {
            return Err("Tone application cancelled; original pixels preserved".into());
        }
        let p = [
            (offset % extent.width as usize) as f64 + extent.offset[0] as f64 + 0.5,
            (offset / extent.width as usize) as f64 + extent.offset[1] as f64 + 0.5,
        ];
        let coverage = doc.scene.selection_coverage(p) as u32;
        if coverage == 0 || before[3] == 0 {
            continue;
        }
        let target = [
            table[before[0] as usize],
            table[before[1] as usize],
            table[before[2] as usize],
            before[3],
        ];
        let mut after = before;
        for n in 0..3 {
            after[n] = ((before[n] as u32 * (255 - coverage) + target[n] as u32 * coverage + 127)
                / 255) as u8;
        }
        if before != after {
            if (changes.len() + 1) * std::mem::size_of::<crate::history::PixelChange>() + 512
                > budget
            {
                return Err("Tone edit exceeds undo memory budget".into());
            }
            changes.push(crate::history::PixelChange {
                offset,
                before,
                after,
            });
        }
    }
    if cancel() {
        return Err("Tone application cancelled; original pixels preserved".into());
    }
    doc.commit_pixel_changes(layer, changes)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn identity_profile_gray_clipping_alpha_and_missing_icc() {
        let settings = Settings {
            enabled: true,
            ..Default::default()
        };
        let identity = Proof::from_bytes(
            &settings,
            &moxcms::ColorProfile::new_srgb().encode().unwrap(),
        )
        .unwrap();
        for r in (0..=255).step_by(17) {
            for g in (0..=255).step_by(17) {
                for b in (0..=255).step_by(17) {
                    let c = [r, g, b, 128];
                    let out = identity.map(c);
                    for n in 0..3 {
                        assert!(out[n].abs_diff(c[n]) <= 2);
                    }
                    assert_eq!(out[3], 128);
                }
            }
        }
        let gray = Proof::from_bytes(
            &settings,
            &moxcms::ColorProfile::new_gray_with_gamma(2.2)
                .encode()
                .unwrap(),
        )
        .unwrap();
        let out = gray.map([200, 40, 80, 255]);
        assert!(out[0].abs_diff(out[1]) <= 2 && out[1].abs_diff(out[2]) <= 2);
        assert_ne!(out, [200, 40, 80, 255]);
        assert_eq!(gray.map([1, 2, 3, 0]), [1, 2, 3, 0]);
        assert!(Proof::from_bytes(&settings, b"invalid").is_err());
        let root = crate::test_support::TempDir::new();
        assert!(
            Proof::read(&Settings {
                profile: Some(root.0.join("missing.icc")),
                ..settings
            })
            .is_err()
        );
        let t = Tone {
            black: 0.25,
            white: 0.75,
            ..Default::default()
        };
        assert_eq!(t.map([0, 128, 255, 128]), [0, 129, 255, 128]);
        assert!(
            !Tone {
                white: 0.0,
                ..Default::default()
            }
            .valid()
        );
    }
    #[test]
    fn actual_tone_undo_selection_cancel_lock_native_and_profile_bytes() {
        let mut d = crate::document::Document::create(
            crate::document::CanvasSpec {
                width: 3,
                height: 1,
                dpi: 300.0,
                background: crate::document::Background::Transparent,
            },
            4096,
        )
        .unwrap();
        d.layers[0].pixels = vec![[20, 80, 160, 0], [20, 80, 160, 128], [20, 80, 160, 255]];
        d.scene.selection_mask = Some(crate::selection::Mask {
            runs: vec![crate::selection::Run {
                x: 0,
                y: 0,
                length: 3,
                alpha: 128,
            }],
        });
        let before = d.snapshot();
        let tone = Tone {
            gamma: 2.0,
            ..Default::default()
        };
        assert!(apply(&mut d, 0, tone, || true).is_err());
        assert!(d.snapshot() == before);
        d.layers[0].locked = true;
        assert!(apply(&mut d, 0, tone, || false).is_err());
        d.layers[0].locked = false;
        assert!(apply(&mut d, 0, tone, || false).unwrap());
        let after = d.snapshot();
        assert_eq!(d.layers[0].pixels[0], [20, 80, 160, 0]);
        assert_eq!(d.layers[0].pixels[1][3], 128);
        assert_eq!(d.history_counts(), (1, 0));
        assert!(d.undo());
        assert!(d.snapshot() == before);
        assert!(d.redo());
        assert!(d.snapshot() == after);
        let root = crate::test_support::TempDir::new();
        let path = root.0.join("tone.drawer");
        crate::files::save(&mut d, &path).unwrap();
        assert!(crate::files::open(&path, 4096).unwrap().snapshot() == after);
    }
}

/// Generated gray ICC for the installed-app verification fixture.
pub fn gray_fixture() -> Vec<u8> {
    moxcms::ColorProfile::new_gray_with_gamma(2.2)
        .encode()
        .expect("Built-in gray profile is valid")
}

#[cfg(test)]
mod cmyk_tests {
    use super::*;
    fn fixture(xyz: [u16; 3], reverse: bool) -> Vec<u8> {
        use moxcms::*;
        let mut p = ColorProfile::new_srgb();
        p.color_space = DataColorSpace::Cmyk;
        p.profile_class = ProfileClass::OutputDevice;
        p.pcs = DataColorSpace::Xyz;
        p.cicp = None;
        p.red_colorant = Default::default();
        p.green_colorant = Default::default();
        p.blue_colorant = Default::default();
        p.red_trc = None;
        p.green_trc = None;
        p.blue_trc = None;
        let lut = |inputs: u8, output: &[u16]| {
            LutWarehouse::Lut(LutDataType {
                num_input_channels: inputs,
                num_output_channels: output.len() as u8,
                num_clut_grid_points: 2,
                matrix: Matrix3d::IDENTITY,
                num_input_table_entries: 256,
                num_output_table_entries: 256,
                input_table: LutStore::Store16(
                    (0..inputs)
                        .flat_map(|_| (0u16..=255).map(|v| v * 257))
                        .collect(),
                ),
                clut_table: LutStore::Store16(
                    (0..(1usize << inputs))
                        .flat_map(|_| output.iter().copied())
                        .collect(),
                ),
                output_table: LutStore::Store16(
                    (0..output.len())
                        .flat_map(|_| (0u16..=255).map(|v| v * 257))
                        .collect(),
                ),
                lut_type: LutType::Lut16,
            })
        };
        p.lut_b_to_a_perceptual = Some(lut(3, &[13107, 26214, 39321, 52428]));
        if reverse {
            p.lut_a_to_b_perceptual = Some(lut(4, &xyz));
        }
        p.encode().unwrap()
    }
    #[test]
    fn bidirectional_cmyk_simulation_and_missing_direction_rejection() {
        let s = Settings {
            enabled: true,
            ..Default::default()
        };
        let a = Proof::from_bytes(&s, &fixture([16384, 8192, 4096], true)).unwrap();
        let b = Proof::from_bytes(&s, &fixture([4096, 16384, 8192], true)).unwrap();
        let first = a.map([200, 40, 80, 128]);
        assert_ne!(first, b.map([200, 40, 80, 128]));
        assert_eq!(first[3], 128);
        assert_eq!(a.map([10, 20, 30, 255])[..3], first[..3]);
        assert!(Proof::from_bytes(&s, &fixture([0; 3], false)).is_err());
        for intent in [
            Intent::Perceptual,
            Intent::Relative,
            Intent::Saturation,
            Intent::Absolute,
        ] {
            let s = Settings {
                intent,
                ..s.clone()
            };
            let p =
                Proof::from_bytes(&s, &moxcms::ColorProfile::new_srgb().encode().unwrap()).unwrap();
            assert!(p.map([255, 0, 255, 255])[0] >= 253);
        }
    }
}
