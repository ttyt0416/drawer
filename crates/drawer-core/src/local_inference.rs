//! Frozen sRGB inputs and atomic adoption. Inference weights and paths never enter a document.
use crate::{document::Document, references::LocalImage, sizing::DocumentState};
use serde::{Deserialize, Serialize};
use std::{
    io::{Read, Write},
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
};

pub const MAX_INPUT: usize = 1024 * 1024;
pub const MAX_OUTPUT: usize = 4 * 1024 * 1024;
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Task {
    #[default]
    Body,
    Hand,
    HintColor,
    Upscale,
    Denoise,
}
impl Task {
    pub const ALL: [Self; 5] = [
        Self::Body,
        Self::Hand,
        Self::HintColor,
        Self::Upscale,
        Self::Denoise,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Body => "Body pose (2D, COCO 17)",
            Self::Hand => "Hand pose (2D, 21 joints)",
            Self::HintColor => "Hint colorization",
            Self::Upscale => "Upscale",
            Self::Denoise => "Denoise",
        }
    }
    pub fn joints(self) -> usize {
        match self {
            Self::Body => 17,
            Self::Hand => 21,
            _ => 0,
        }
    }
    pub fn edges(self) -> Vec<[usize; 2]> {
        match self {
            Self::Body => vec![
                [0, 1],
                [0, 2],
                [1, 3],
                [2, 4],
                [5, 6],
                [5, 7],
                [7, 9],
                [6, 8],
                [8, 10],
                [5, 11],
                [6, 12],
                [11, 12],
                [11, 13],
                [13, 15],
                [12, 14],
                [14, 16],
            ],
            Self::Hand => (0..5)
                .flat_map(|f| {
                    let n = f * 4 + 1;
                    [[0, n], [n, n + 1], [n + 1, n + 2], [n + 2, n + 3]]
                })
                .collect(),
            _ => vec![],
        }
    }
}
pub fn checked(cancel: &AtomicBool) -> Result<(), String> {
    if cancel.load(Ordering::Relaxed) {
        Err("Local inference cancelled".into())
    } else {
        Ok(())
    }
}
pub fn valid_image(i: &LocalImage, max: usize) -> bool {
    i.width > 0
        && i.height > 0
        && (i.width as usize)
            .checked_mul(i.height as usize)
            .is_some_and(|n| n <= max && n == i.pixels.len())
}
pub fn read_image(path: &Path, max: usize) -> Result<LocalImage, String> {
    let mut f = std::fs::File::open(path).map_err(|e| e.to_string())?;
    if f.metadata().map_err(|e| e.to_string())?.len() > 8 + max as u64 * 4 {
        return Err("Inference image exceeds memory budget".into());
    }
    let mut header = [0; 8];
    f.read_exact(&mut header).map_err(|e| e.to_string())?;
    let w = u32::from_le_bytes(header[..4].try_into().unwrap()) as usize;
    let h = u32::from_le_bytes(header[4..].try_into().unwrap()) as usize;
    let n = w
        .checked_mul(h)
        .filter(|n| *n > 0 && *n <= max)
        .ok_or("Invalid inference dimensions")?;
    let mut bytes = vec![];
    f.take(n as u64 * 4 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    LocalImage::from_rgba(w, h, &bytes, max * 4).map_err(|e| e.to_string())
}
pub fn write_image(path: &Path, i: &LocalImage, max: usize) -> Result<(), String> {
    if !valid_image(i, max) {
        return Err("Invalid inference image".into());
    }
    let mut f = std::fs::File::create(path).map_err(|e| e.to_string())?;
    for b in [i.width.to_le_bytes(), i.height.to_le_bytes()] {
        f.write_all(&b).map_err(|e| e.to_string())?;
    }
    for row in i.pixels.chunks(4096) {
        let bytes: Vec<u8> = row.iter().flatten().copied().collect();
        f.write_all(&bytes).map_err(|e| e.to_string())?;
    }
    Ok(())
}
pub fn snapshot(d: &Document, selection: bool, cancel: &AtomicBool) -> Result<LocalImage, String> {
    checked(cancel)?;
    let n = d.width as usize * d.height as usize;
    if n > MAX_INPUT {
        return Err("Local inference input exceeds 1,048,576 pixels; crop or resize first".into());
    }
    if selection && d.scene.selection.is_none() && d.scene.selection_mask.is_none() {
        return Err("Select an area first".into());
    }
    let renderer =
        crate::float_render::Renderer::with_effect_progress(d, None, &mut |_, _| checked(cancel))?;
    let mut pixels = Vec::with_capacity(n);
    for y in 0..d.height {
        checked(cancel)?;
        for x in 0..d.width {
            let point = [x as f64 + 0.5, y as f64 + 0.5];
            let mut p = renderer.display_sample(point)?.rgba8();
            if selection {
                p[3] = ((u16::from(p[3]) * u16::from(d.scene.selection_coverage(point)) + 127)
                    / 255) as u8;
            }
            if p[3] == 0 {
                p = [0; 4];
            }
            pixels.push(p);
        }
    }
    Ok(LocalImage {
        width: d.width,
        height: d.height,
        pixels,
    })
}
#[derive(Debug)]
pub struct Output {
    pub task: Task,
    pub image: Option<LocalImage>,
    pub points: Vec<[f32; 3]>,
    pub report: String,
}
impl Output {
    pub fn low_confidence(&self, threshold: f32) -> bool {
        self.points.iter().any(|p| p[2] < threshold)
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.task.joints() > 0 {
            if self.image.is_some()
                || self.points.len() != self.task.joints()
                || self
                    .points
                    .iter()
                    .flatten()
                    .any(|v| !v.is_finite() || !(0. ..=1.).contains(v))
            {
                return Err("Invalid normalized pose/confidence output".into());
            }
        } else if !self.points.is_empty()
            || self
                .image
                .as_ref()
                .is_none_or(|i| !valid_image(i, MAX_OUTPUT))
        {
            return Err("Invalid inference image output".into());
        }
        Ok(())
    }
}
pub fn adopt(
    d: &Document,
    input: &LocalImage,
    result: &Output,
    threshold: f32,
    accept_low: bool,
) -> Result<DocumentState, String> {
    result.validate()?;
    if !valid_image(input, MAX_INPUT) || !threshold.is_finite() || !(0. ..=1.).contains(&threshold)
    {
        return Err("Invalid inference source or confidence threshold".into());
    }
    if result.low_confidence(threshold) && !accept_low {
        return Err("Review and explicitly accept low-confidence joints first".into());
    }
    let mut state = if let Some(image) = &result.image {
        let mut s =
            crate::paste::prepare(d, image, [0, 0], crate::document::DEFAULT_DOCUMENT_BUDGET)?;
        s.layers.last_mut().unwrap().name = format!("Local {}", result.task.label());
        s
    } else {
        let mut s = d.snapshot();
        let layer = s.scene.lines.add_layer(result.task.label())?;
        for [a, b] in result.task.edges() {
            let points = [a, b].map(|n| {
                s.scene.transform.inverse().map([
                    result.points[n][0] as f64 * input.width as f64,
                    result.points[n][1] as f64 * input.height as f64,
                ])
            });
            let uncertain = result.points[a][2] < threshold || result.points[b][2] < threshold;
            s.scene.lines.add_path(
                layer,
                points.to_vec(),
                false,
                2.,
                if uncertain {
                    [255, 160, 0, 255]
                } else {
                    [30, 160, 240, 255]
                },
            )?;
        }
        s
    };
    // Bounds are document coordinates; the source image stays at the top-left.
    state.scene.layer_ids.append(state.layers.len())?;
    crate::layer_catalog::reconcile(d, &mut state)?;
    crate::render_order::reconcile(&mut state);
    if !state.scene.valid()
        || state.memory_bytes().saturating_mul(5) > crate::document::DEFAULT_DOCUMENT_BUDGET
    {
        return Err("Inference result exceeds document/undo memory budget".into());
    }
    Ok(state)
}
