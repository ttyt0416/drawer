//! Document-specific view preferences and output-independent onion guides.
use crate::{
    color_precision::Sample, document::Document, layer_search::Key, references::LocalImage,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ColorMode {
    Original,
    Half,
    #[default]
    Solid,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Onion {
    #[serde(skip)]
    pub enabled: bool,
    pub before: u8,
    pub after: u8,
    pub opacity: u8,
    pub colors: [[u8; 3]; 2],
    pub mode: ColorMode,
    pub fade: u8,
    pub skip_repeats: bool,
}
impl Default for Onion {
    fn default() -> Self {
        Self {
            enabled: false,
            before: 1,
            after: 1,
            opacity: 30,
            colors: [[220, 60, 65], [45, 115, 230]],
            mode: ColorMode::Solid,
            fade: 0,
            skip_repeats: false,
        }
    }
}
impl Onion {
    pub fn valid(self) -> bool {
        self.before <= 8 && self.after <= 8 && self.opacity <= 100 && self.fade <= 100
    }
    pub fn preferences(mut self) -> Self {
        self.enabled = false;
        self
    }
    pub fn tint(self, pixel: [u8; 4], direction: usize, distance: u32) -> [u8; 4] {
        let strength = (self.opacity as u32)
            .saturating_sub((self.fade as u32).saturating_mul(distance.saturating_sub(1)));
        let color = self.colors[direction];
        let rgb = match self.mode {
            ColorMode::Original => [pixel[0], pixel[1], pixel[2]],
            ColorMode::Half => {
                [0, 1, 2].map(|i| (pixel[i] as u16 + color[i] as u16).div_ceil(2) as u8)
            }
            ColorMode::Solid => color,
        };
        [
            rgb[0],
            rgb[1],
            rgb[2],
            (pixel[3] as u32 * strength / 100) as u8,
        ]
    }
}
pub fn set_preferences(d: &mut Document, onion: Onion) -> Result<(), String> {
    if !onion.valid() {
        return Err("Onion counts must be0..8; opacity and fade0..100".into());
    }
    d.onion = onion.preferences();
    Ok(())
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
fn sources(d: &Document, track: Option<u64>, only: bool) -> Vec<Key> {
    let mut keys = d
        .animation
        .iter()
        .flat_map(|a| 0..a.tracks.len())
        .map(|n| Key::Track(d.scene.catalog.tracks.id(n)))
        .filter(|k| track.is_none_or(|id| !only || *k == Key::Track(id)))
        .collect::<Vec<_>>();
    if !only {
        let animated = d
            .scene
            .keyframes
            .tracks
            .iter()
            .filter_map(|t| match t.owner {
                crate::keyframes::Owner::Layer(k)
                    if !matches!(k, Key::Track(_) | Key::Cell { .. }) =>
                {
                    Some(k)
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        keys.extend(crate::layer_groups::roots(d, &animated));
    }
    if let Ok(rows) = crate::layer_catalog::rows(d) {
        let order = rows
            .into_iter()
            .enumerate()
            .map(|(i, r)| (r.key, i))
            .collect::<BTreeMap<_, _>>();
        keys.sort_by_key(|k| order.get(k).copied().unwrap_or(usize::MAX));
    }
    keys
}
struct Parent {
    inverse: crate::scene::Affine,
    target: crate::layer_properties::Target,
    opacity: f64,
    masked: bool,
}
struct Source {
    key: Key,
    parents: Vec<Parent>,
}
fn source_plan(d: &Document, key: Key) -> Source {
    Source {
        key,
        parents: crate::keyframes::ancestors(d, key)
            .into_iter()
            .rev()
            .filter_map(|parent| {
                let target = crate::layer_catalog::target(d, parent)?;
                Some(Parent {
                    inverse: crate::keyframes::local(d, parent, false).0.inverse(),
                    target,
                    opacity: crate::layer_properties::factor(d, target),
                    masked: d
                        .scene
                        .layer_effects
                        .get(target)
                        .is_some_and(|e| e.mask.is_some()),
                })
            })
            .collect(),
    }
}
fn animation(
    r: &crate::float_render::Renderer<'_>,
    d: &Document,
    keys: &[Source],
    point: [f64; 2],
) -> Result<[u8; 4], String> {
    let mut out = [0; 4];
    for source in keys {
        let mut p = point;
        let mut weight = 1.;
        for parent in &source.parents {
            p = parent.inverse.map(p);
            weight *= parent.opacity;
            if parent.masked {
                weight *= d
                    .scene
                    .layer_effects
                    .mask(parent.target, d.scene.transform.inverse().map(p))
                    as f64;
            }
        }
        if weight > 0. {
            out = over(
                out,
                r.display_source(source.key, point)?
                    .coverage(weight)?
                    .rgba8(),
            );
        }
    }
    Ok(out)
}
/// Source identity plus evaluated placement: held cels are deduplicated; moving poses remain useful.
fn signature(d: &Document, key: Key) -> Option<Vec<u64>> {
    let mut out = vec![];
    if let Key::Track(id) = key {
        let a = d.animation.as_ref()?;
        let n = d.scene.catalog.tracks.index(id, a.tracks.len())?;
        let (_, i, c) = a
            .indexed_visible_cells(d.current_frame)
            .find(|(t, _, _)| *t == n)?;
        let cell = d.scene.catalog.cell_id(id, i);
        out.push(d.scene.cels.source(id, cell).unwrap_or(cell));
        out.extend(c.offset.map(|v| v as u64));
        out.extend([c.width as u64, c.height as u64]);
        let (m, alpha) = crate::keyframes::combined(d, Key::Cell { track: id, cell }, false);
        out.extend(m.0.map(f64::to_bits));
        out.push(alpha.to_bits());
    } else {
        let members = crate::layer_groups::content_members(d, &[key]).ok()?;
        for member in members {
            let (m, alpha) = crate::keyframes::combined(d, member, false);
            out.extend(m.0.map(f64::to_bits));
            out.push(alpha.to_bits());
        }
    }
    Some(out)
}
struct Guide {
    frame: u32,
    direction: usize,
    distance: u32,
    keys: Vec<Source>,
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
        || !onion.valid()
        || only && track.is_none()
    {
        return Err("Invalid timeline view settings".into());
    }
    let keys = sources(d, track, only)
        .into_iter()
        .filter(|k| {
            crate::layer_catalog::target(d, *k)
                .is_some_and(|t| !crate::layer_properties::inherited(d, t).hidden)
        })
        .collect::<Vec<_>>();
    let current_sources = keys.iter().map(|k| source_plan(d, *k)).collect::<Vec<_>>();
    let mut seen: BTreeMap<Key, BTreeSet<Vec<u64>>> = BTreeMap::new();
    if onion.skip_repeats {
        for k in &keys {
            if let Some(s) = signature(d, *k) {
                seen.entry(*k).or_default().insert(s);
            }
        }
    }
    let mut candidates = vec![];
    if onion.enabled {
        for distance in 1..=u32::from(onion.before.max(onion.after)) {
            if distance <= onion.before as u32
                && let Some(frame) = d.current_frame.checked_sub(distance)
            {
                candidates.push((frame, 0, distance));
            }
            if distance <= onion.after as u32 && d.current_frame + distance < a.frames {
                candidates.push((d.current_frame + distance, 1, distance));
            }
        }
    }
    if d.memory_bytes().saturating_mul(candidates.len() + 2) > budget {
        return Err(
            "Onion preview exceeds snapshot budget; reduce preceding/following counts".into(),
        );
    }
    let mut docs = vec![];
    let mut guides = vec![];
    for (frame, direction, distance) in candidates {
        if onion.tint([0, 0, 0, 255], direction, distance)[3] == 0 {
            continue;
        }
        let mut doc = crate::layer_merge::preview(&d.snapshot(), frame)?;
        doc.keyframe_content = d.keyframe_content;
        let kept = keys
            .iter()
            .copied()
            .filter(|k| {
                !onion.skip_repeats
                    || signature(&doc, *k).is_some_and(|s| seen.entry(*k).or_default().insert(s))
            })
            .collect::<Vec<_>>();
        if kept.is_empty() {
            continue;
        }
        let planned = kept.into_iter().map(|k| source_plan(&doc, k)).collect();
        docs.push(doc);
        guides.push(Guide {
            frame,
            direction,
            distance,
            keys: planned,
        });
    }
    // Draw distant references first, so nearby ones stay legible. Equal distances prefer the preceding side.
    let mut order = (0..guides.len()).collect::<Vec<_>>();
    order.sort_by_key(|i| {
        (
            std::cmp::Reverse(guides[*i].distance),
            guides[*i].direction,
            guides[*i].frame,
        )
    });
    let scale = (d.width.max(d.height) as f64 / side as f64).max(1.);
    let width = (d.width as f64 / scale).ceil() as u32;
    let height = (d.height as f64 / scale).ceil() as u32;
    if width as usize * height as usize * 8 > budget {
        return Err("Onion output budget".into());
    }
    let work = guides
        .iter()
        .map(|g| g.keys.iter().map(|s| s.parents.len() + 1).sum::<usize>())
        .sum::<usize>()
        .saturating_add(
            current_sources
                .iter()
                .map(|s| s.parents.len() + 1)
                .sum::<usize>()
                + 1,
        )
        .saturating_mul(width as usize * height as usize);
    if work > 64 * 1024 * 1024 {
        return Err("Onion sample budget exceeded; reduce frame counts or select one track".into());
    }
    let renderers = docs
        .iter()
        .map(crate::float_render::Renderer::new)
        .collect::<Result<Vec<_>, _>>()?;
    let renderer = crate::float_render::Renderer::new(d)?;
    let mut pixels = Vec::with_capacity((width * height) as usize);
    let mut overlay = Vec::with_capacity((width * height) as usize);
    for y in 0..height {
        for x in 0..width {
            let point = [
                (x as f64 + 0.5) * d.width as f64 / width as f64,
                (y as f64 + 0.5) * d.height as f64 / height as f64,
            ];
            let active = animation(&renderer, d, &current_sources, point)?;
            let current = if only {
                active
            } else {
                renderer.display_sample(point)?.rgba8()
            };
            let mut ghost = [0; 4];
            for i in &order {
                let g = &guides[*i];
                let sample = animation(&renderers[*i], &docs[*i], &g.keys, point)?;
                ghost = over(ghost, onion.tint(sample, g.direction, g.distance));
            }
            let guide = [
                ghost[0],
                ghost[1],
                ghost[2],
                (ghost[3] as u16 * (255 - active[3]) as u16 / 255) as u8,
            ];
            overlay.push(guide);
            pixels.push(if only {
                over(ghost, current)
            } else {
                over(current, guide)
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
            pixels: overlay,
        },
    })
}
