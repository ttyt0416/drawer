//! View-only onion guides. No drawing state, source pixels or visibility is edited.
use crate::{
    color_precision::Sample, document::Document, layer_search::Key, references::LocalImage,
};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Onion {
    pub enabled: bool,
    pub before: u8,
    pub after: u8,
    pub opacity: u8,
    pub colors: [[u8; 3]; 2],
}
impl Default for Onion {
    fn default() -> Self {
        Self {
            enabled: false,
            before: 1,
            after: 1,
            opacity: 30,
            colors: [[220, 60, 65], [45, 115, 230]],
        }
    }
}
pub struct Preview {
    pub image: LocalImage,
    pub overlay: LocalImage,
}
fn over(back: [u8; 4], front: [u8; 4]) -> [u8; 4] {
    Sample::from8(back)
        .over(Sample::from8(front), crate::layer_blend::Mode::Normal, 1.)
        .unwrap()
        .rgba8()
}
fn animation(
    r: &crate::float_render::Renderer<'_>,
    d: &Document,
    track: Option<u64>,
    point: [f64; 2],
) -> Result<[u8; 4], String> {
    let mut out = [0; 4];
    if let Some(a) = &d.animation {
        for n in 0..a.tracks.len() {
            let id = d.scene.catalog.tracks.id(n);
            if track.is_none_or(|t| t == id) {
                out = over(out, r.display_source(Key::Track(id), point)?.rgba8());
            }
        }
    }
    Ok(out)
}
pub fn preview(
    d: &Document,
    track: Option<u64>,
    only: bool,
    onion: Onion,
    side: u32,
    budget: usize,
) -> Result<Preview, String> {
    let a = d.animation.as_ref().ok_or("Choose an animation document")?;
    if d.current_frame >= a.frames
        || side == 0
        || side > 1024
        || onion.before > 8
        || onion.after > 8
        || onion.opacity > 100
        || (only && track.is_none())
    {
        return Err("Invalid timeline view settings".into());
    }
    let mut frames = vec![];
    if onion.enabled {
        for delta in (1..=onion.before as u32).rev() {
            if let Some(f) = d.current_frame.checked_sub(delta) {
                frames.push((f, 0));
            }
        }
        for delta in (1..=onion.after as u32).rev() {
            if d.current_frame + delta < a.frames {
                frames.push((d.current_frame + delta, 1));
            }
        }
    }
    if d.memory_bytes().saturating_mul(frames.len() + 2) > budget {
        return Err(
            "Onion preview exceeds snapshot budget; reduce preceding/following counts".into(),
        );
    }
    let docs = frames
        .iter()
        .map(|(f, _)| crate::layer_merge::preview(&d.snapshot(), *f))
        .collect::<Result<Vec<_>, _>>()?;
    let renderers = docs
        .iter()
        .map(crate::float_render::Renderer::new)
        .collect::<Result<Vec<_>, _>>()?;
    let renderer = crate::float_render::Renderer::new(d)?;
    let scale = (d.width.max(d.height) as f64 / side as f64).max(1.);
    let width = (d.width as f64 / scale).ceil() as u32;
    let height = (d.height as f64 / scale).ceil() as u32;
    if width as usize * height as usize * 8 > budget {
        return Err("Onion output budget".into());
    }
    let mut pixels = Vec::with_capacity((width * height) as usize);
    let mut guides = Vec::with_capacity((width * height) as usize);
    for y in 0..height {
        for x in 0..width {
            let point = [
                (x as f64 + 0.5) * d.width as f64 / width as f64,
                (y as f64 + 0.5) * d.height as f64 / height as f64,
            ];
            let active = animation(&renderer, d, if only { track } else { None }, point)?;
            let current = if only {
                active
            } else {
                renderer.display_sample(point)?.rgba8()
            };
            let mut ghost = [0; 4];
            for ((r, doc), (_, direction)) in renderers.iter().zip(&docs).zip(&frames) {
                let sample = animation(r, doc, if only { track } else { None }, point)?;
                let color = onion.colors[*direction];
                let alpha = (sample[3] as u32 * onion.opacity as u32 / 100) as u8;
                ghost = over(ghost, [color[0], color[1], color[2], alpha]);
            }
            let overlay = [
                ghost[0],
                ghost[1],
                ghost[2],
                (ghost[3] as u16 * (255 - active[3]) as u16 / 255) as u8,
            ];
            guides.push(overlay);
            pixels.push(if only {
                over(ghost, current)
            } else {
                over(current, overlay)
            });
        }
    }
    Ok(Preview {
        image: LocalImage {
            width,
            height,
            pixels,
        },
        overlay: LocalImage {
            width,
            height,
            pixels: guides,
        },
    })
}
