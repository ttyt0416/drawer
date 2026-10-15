use crate::{
    document::{Document, PlacedRaster, RasterLayer},
    references::LocalImage,
    sizing::DocumentState,
};

const MAGIC: &[u8; 8] = b"DRAWCP\r\n";
pub struct ClipboardImage {
    pub image: LocalImage,
    pub origin: Option<[i64; 2]>,
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
        if !p.valid() || self.image.pixels.len().saturating_mul(4).saturating_add(36) > budget {
            return Err("Clipboard image exceeds the transfer budget".into());
        }
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(self.image.pixels.len() * 4 + 36)
            .map_err(|e| e.to_string())?;
        bytes.extend_from_slice(MAGIC);
        bytes.extend_from_slice(&self.image.width.to_le_bytes());
        bytes.extend_from_slice(&self.image.height.to_le_bytes());
        bytes.extend_from_slice(&u32::from(self.origin.is_some()).to_le_bytes());
        for n in p.offset {
            bytes.extend_from_slice(&n.to_le_bytes());
        }
        for pixel in &self.image.pixels {
            bytes.extend_from_slice(pixel);
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
        if flag > 1 {
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
        let image = LocalImage::from_rgba(width as usize, height as usize, &bytes[36..], budget)
            .map_err(|e| e.to_string())?;
        Ok(Self {
            image,
            origin: (flag == 1).then_some(offset),
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
    let bytes = image.pixels.len() * 4;
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
        locked: false,
        name: "Pasted image".into(),
        pixels: image.pixels.clone(),
        placement: Some(placement),
    });
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
