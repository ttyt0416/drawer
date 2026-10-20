use crate::{
    document::{Document, PlacedRaster, RasterLayer},
    references::LocalImage,
    sizing::DocumentState,
};

const MAGIC: &[u8; 8] = b"DRAWCP\r\n";
pub struct ClipboardImage {
    pub color: Option<ClipboardColor>,
    pub image: LocalImage,
    pub origin: Option<[i64; 2]>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct ClipboardColor {
    pub profile: crate::color_profile::RgbProfile,
    pub pixels: Vec<[u16; 4]>,
}
impl ClipboardImage {
    pub fn encode(&self, budget: usize) -> Result<Vec<u8>, String> {
        if self.image.pixels.len() != self.image.width as usize * self.image.height as usize {
            return Err("Invalid clipboard image".into());
        }
        let p = PlacedRaster {
            width: self.image.width,
            height: self.image.height,
            offset: self.origin.unwrap_or([0; 2]),
        };
        let profile = self
            .color
            .as_ref()
            .map(|c| serde_json::to_vec(&c.profile).map_err(|e| e.to_string()))
            .transpose()?;
        let extra = profile.as_ref().map_or(0, |p| {
            p.len() + 4 + self.image.pixels.len().saturating_mul(8)
        });
        if self.color.as_ref().is_some_and(|c| {
            !c.profile.valid()
                || c.pixels.len() != self.image.pixels.len()
                || c.pixels.len() > 4 * 1024 * 1024
                || c.pixels.iter().zip(&self.image.pixels).any(|(p, l)| {
                    (p[3] == 0 && p[..3] != [0; 3])
                        || crate::color_precision::Sample::from16(*p).rgba8() != *l
                })
        }) {
            return Err("Invalid high precision clipboard colors".into());
        }
        if !p.valid()
            || self
                .image
                .pixels
                .len()
                .saturating_mul(4)
                .saturating_add(36)
                .saturating_add(extra)
                > budget
        {
            return Err("Clipboard image exceeds the transfer budget".into());
        }
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(self.image.pixels.len() * 4 + 36 + extra)
            .map_err(|e| e.to_string())?;
        bytes.extend_from_slice(MAGIC);
        bytes.extend_from_slice(&self.image.width.to_le_bytes());
        bytes.extend_from_slice(&self.image.height.to_le_bytes());
        bytes.extend_from_slice(
            &(u32::from(self.origin.is_some()) + 2 * u32::from(self.color.is_some())).to_le_bytes(),
        );
        for n in p.offset {
            bytes.extend_from_slice(&n.to_le_bytes());
        }
        for pixel in &self.image.pixels {
            bytes.extend_from_slice(pixel);
        }
        if let (Some(profile), Some(color)) = (profile, &self.color) {
            bytes.extend_from_slice(&(profile.len() as u32).to_le_bytes());
            bytes.extend_from_slice(&profile);
            for pixel in &color.pixels {
                for value in pixel {
                    bytes.extend_from_slice(&value.to_le_bytes());
                }
            }
        }
        Ok(bytes)
    }
    pub fn decode(bytes: &[u8], budget: usize) -> Result<Self, String> {
        if bytes.len() < 36 || bytes.len() > budget || &bytes[..8] != MAGIC {
            return Err("Invalid Drawer clipboard packet".into());
        }
        let u32at = |offset| u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap());
        let width = u32at(8);
        let height = u32at(12);
        let flag = u32at(16);
        if flag > 3 {
            return Err("Invalid clipboard origin tag".into());
        }
        let offset = [
            i64::from_le_bytes(bytes[20..28].try_into().unwrap()),
            i64::from_le_bytes(bytes[28..36].try_into().unwrap()),
        ];
        if !(PlacedRaster {
            width,
            height,
            offset,
        })
        .valid()
        {
            return Err("Invalid clipboard placement".into());
        }
        let count = width as usize * height as usize;
        let end = 36usize.saturating_add(count.saturating_mul(4));
        if end > bytes.len() {
            return Err("Truncated clipboard pixels".into());
        }
        let image = LocalImage::from_rgba(width as usize, height as usize, &bytes[36..end], budget)
            .map_err(|e| e.to_string())?;
        let color = if flag & 2 != 0 {
            if count > 4 * 1024 * 1024 || bytes.len() < end + 4 {
                return Err("Truncated or oversized precise clipboard".into());
            }
            let size = u32at(end) as usize;
            if size > 20 * 1024 * 1024 || end + 4 + size + count * 8 != bytes.len() {
                return Err("Invalid clipboard color payload length".into());
            }
            let profile: crate::color_profile::RgbProfile =
                serde_json::from_slice(&bytes[end + 4..end + 4 + size])
                    .map_err(|e| e.to_string())?;
            if !profile.valid() {
                return Err("Invalid clipboard RGB profile".into());
            }
            let pixels = bytes[end + 4 + size..]
                .as_chunks::<8>()
                .0
                .iter()
                .map(|p| std::array::from_fn(|n| u16::from_le_bytes([p[n * 2], p[n * 2 + 1]])))
                .collect::<Vec<_>>();
            if pixels.iter().zip(&image.pixels).any(|(p, l)| {
                (p[3] == 0 && p[..3] != [0; 3])
                    || crate::color_precision::Sample::from16(*p).rgba8() != *l
            }) {
                return Err("Invalid clipboard precise preview".into());
            }
            Some(ClipboardColor { profile, pixels })
        } else {
            if end != bytes.len() {
                return Err("Unexpected clipboard bytes".into());
            }
            None
        };
        Ok(Self {
            color,
            image,
            origin: (flag & 1 == 1).then_some(offset),
        })
    }
}
pub fn center(document: &Document, image: &LocalImage) -> [i64; 2] {
    [
        (document.width as i64 - image.width as i64).div_euclid(2),
        (document.height as i64 - image.height as i64).div_euclid(2),
    ]
}
pub fn prepare(
    document: &Document,
    image: &LocalImage,
    offset: [i64; 2],
    budget: usize,
) -> Result<DocumentState, String> {
    let placement = PlacedRaster {
        width: image.width,
        height: image.height,
        offset,
    };
    if !placement.valid()
        || image.pixels.len() != image.width as usize * image.height as usize
        || document.layers.len() >= 256
    {
        return Err("Invalid paste dimensions, position or layer limit".into());
    }
    let bytes = image.pixels.len()
        * if document.color.depth == crate::color_precision::Depth::Sixteen {
            12
        } else {
            4
        };
    let history = document
        .memory_bytes()
        .checked_mul(2)
        .and_then(|n| n.checked_add(bytes))
        .ok_or("Paste size overflow")?;
    if history > crate::document::DEFAULT_DOCUMENT_BUDGET / 4 || history.saturating_mul(3) > budget
    {
        return Err(
            "Paste exceeds the memory/undo budget. Close other drawings or use a smaller image."
                .into(),
        );
    }
    let mut state = document.snapshot();
    state.layers.push(RasterLayer {
        precise: None,
        locked: false,
        name: "Pasted image".into(),
        pixels: image.pixels.clone(),
        placement: Some(placement),
    });
    state.scene.layer_ids.append(state.layers.len())?;
    if document.color != crate::color_storage::Config::legacy() {
        let transform = crate::color_profile::Transform::new(
            &crate::color_profile::RgbProfile::AssumedSrgb,
            &document.color.profile,
        )?;
        let samples = image
            .pixels
            .iter()
            .map(|p| transform.map(crate::color_precision::Sample::from8(*p)))
            .collect::<Result<Vec<_>, _>>()?;
        crate::color_storage::set_samples(
            state.layers.last_mut().unwrap(),
            &document.color,
            placement,
            samples,
        )?;
    }
    Ok(state)
}
pub fn prepare_packet(
    document: &Document,
    packet: &ClipboardImage,
    offset: [i64; 2],
    budget: usize,
) -> Result<DocumentState, String> {
    let mut state = prepare(document, &packet.image, offset, budget)?;
    if let Some(color) = &packet.color {
        if color.pixels.len() != packet.image.pixels.len() || !color.profile.valid() {
            return Err("Invalid precise paste".into());
        }
        let target = state.layers.last_mut().unwrap();
        let extent = target.placement.unwrap();
        let transform =
            crate::color_profile::Transform::new(&color.profile, &document.color.profile)?;
        let samples = color
            .pixels
            .iter()
            .map(|p| transform.map(crate::color_precision::Sample::from16(*p)))
            .collect::<Result<Vec<_>, _>>()?;
        crate::color_storage::set_samples(target, &document.color, extent, samples)?;
    }
    Ok(state)
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        document::{Background, CanvasSpec, DEFAULT_DOCUMENT_BUDGET},
        files,
        sizing::{self, Change, Interpolation},
        test_support::TempDir,
    };
    use std::sync::atomic::AtomicBool;
    #[test]
    fn clipboard_origin_alpha_outside_pixels_reopen_move_and_single_undo() {
        let root = TempDir::new();
        let mut doc = Document::create(
            CanvasSpec {
                width: 3,
                height: 2,
                dpi: 300.0,
                background: Background::Transparent,
            },
            10000,
        )
        .unwrap();
        let before = doc.snapshot();
        let image = LocalImage::from_rgba(2, 1, &[100, 20, 40, 128, 30, 40, 50, 0], 8).unwrap();
        let packet = ClipboardImage {
            color: None,
            image: image.clone(),
            origin: Some([-100, 500]),
        };
        let bytes = packet.encode(1000).unwrap();
        let restored = ClipboardImage::decode(&bytes, 1000).unwrap();
        assert_eq!(restored.origin, packet.origin);
        assert_eq!(restored.image, image);
        assert_eq!(center(&doc, &image), [0, 0]);
        doc.commit_state(
            prepare(
                &doc,
                &image,
                restored.origin.unwrap(),
                DEFAULT_DOCUMENT_BUDGET,
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(doc.composite_pixel(0, 0), [0; 4]);
        assert_eq!(doc.layers[1].pixels, image.pixels);
        let path = root.0.join("outside.drawer");
        files::save(&mut doc, &path).unwrap();
        let mut reopened = files::open(&path, 10000).unwrap();
        assert_eq!(reopened.snapshot(), doc.snapshot());
        let mut moved = reopened.snapshot();
        moved.layers[1].placement.as_mut().unwrap().offset = [1, 1];
        reopened.commit_state(moved).unwrap();
        assert_eq!(reopened.composite_pixel(1, 1), [100, 20, 40, 128]);
        assert!(reopened.edit_pixel(1, 1, 1, [9, 8, 7, 255]));
        assert_eq!(reopened.composite_pixel(1, 1), [9, 8, 7, 255]);
        assert!(doc.undo());
        assert_eq!(doc.snapshot(), before);
        assert_eq!(doc.history_counts(), (0, 1));
        for change in [
            Change::FlipHorizontal,
            Change::Rotate90 { clockwise: true },
            Change::Resample {
                width: 6,
                height: 4,
                dpi: 300.0,
                filter: Interpolation::Nearest,
            },
        ] {
            let output = sizing::prepare(
                &reopened.snapshot(),
                change,
                DEFAULT_DOCUMENT_BUDGET,
                &AtomicBool::new(false),
            )
            .unwrap();
            assert!(output.layers[1].placement.is_some());
            assert_eq!(
                output.layers[1].pixels.len(),
                if matches!(change, Change::Resample { .. }) {
                    8
                } else {
                    2
                }
            );
        }
        assert!(ClipboardImage::decode(&bytes[..bytes.len() - 1], 1000).is_err());
        assert!(ClipboardImage::decode(&bytes, 1).is_err());
        assert!(prepare(&doc, &image, [i64::MAX, 0], DEFAULT_DOCUMENT_BUDGET).is_err());
        assert!(prepare(&doc, &image, [0, 0], 1).is_err());
    }
}
