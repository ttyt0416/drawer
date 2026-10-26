//! Editable upper/lower source copies with document-space masks. No source is flattened.
use crate::{
    comics::{Kind, Mask},
    document::Document,
    layer_catalog as c, layer_properties as p,
    layer_search::Key,
    scene::{PageObject, Rect},
    sizing::DocumentState,
};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Band {
    Insert { at: u32, height: u32 },
    Remove { start: u32, end: u32 },
}
fn parameters(d: &Document, b: Band) -> Result<(u32, u32, i64, u32), String> {
    let (a, z, delta) = match b {
        Band::Insert { at, height } => {
            if height == 0 {
                return Err("Insertion height must be positive".into());
            }
            (at, at, height as i64)
        }
        Band::Remove { start, end } => {
            if start >= end {
                return Err("Removal needs a nonempty strip".into());
            }
            (start, end, start as i64 - end as i64)
        }
    };
    let h = d.height as i64 + delta;
    if a > d.height || z > d.height || !(1..=32768).contains(&h) {
        return Err("Band or resulting canvas height outside1–32768".into());
    }
    Ok((a, z, delta, h as u32))
}
pub fn impact(d: &Document, b: Band) -> Result<Vec<String>, String> {
    parameters(d, b)?;
    let mut rows = c::rows(d)?
        .into_iter()
        .filter(|r| {
            matches!(
                r.key,
                Key::Object(_)
                    | Key::World(_)
                    | Key::Gradient(_)
                    | Key::Track(_)
                    | Key::Layer(p::Target::Raster(_) | p::Target::Vector(_))
            )
        })
        .map(|r| {
            format!(
                "{:?}: {}{}",
                r.key,
                r.name,
                if c::target(d, r.key).is_some_and(|t| {
                    let f = p::inherited(d, t);
                    f.locked || f.position_locked || f.pixels_locked
                }) {
                    " [locked: operation blocked]"
                } else {
                    ""
                }
            )
        })
        .collect::<Vec<_>>();
    if d.scene.camera.is_some() {
        rows.push("3D camera: source projection preserved; masks follow the strips".into());
    }
    if d.animation.is_some() {
        rows.push(
            "Animation: every stored cell is shifted/split; exposure timing is preserved".into(),
        );
    }
    Ok(rows)
}
fn mask(s: &mut DocumentState, key: Key, r: Rect) {
    let inv = s.scene.transform.inverse();
    s.scene.comics.masks.push(Mask {
        target: key,
        polygon: vec![r.min, [r.max[0], r.min[1]], r.max, [r.min[0], r.max[1]]]
            .into_iter()
            .map(|p| inv.map(p))
            .collect(),
        space: Default::default(),
    });
}
pub fn prepare(d: &Document, b: Band, confirm_remove: bool) -> Result<DocumentState, String> {
    let (a, z, delta, height) = parameters(d, b)?;
    if matches!(b, Band::Remove { .. }) && !confirm_remove {
        return Err("Confirm the removed strip before applying".into());
    }
    let mut state = d.snapshot();
    state.scene.layer_ids.initialize(d.layers.len())?;
    c::reconcile(d, &mut state)?;
    let base = crate::layer_merge::preview(&state, d.current_frame)?;
    let keys: Vec<_> = (0..base.layers.len())
        .map(|n| Key::Layer(p::raster(&base, n)))
        .chain(
            crate::render_order::sources(&base.scene)
                .into_iter()
                .map(|e| e.key),
        )
        .collect();
    for key in &keys {
        let t = c::target(&base, *key).ok_or("Band source missing")?;
        let f = p::inherited(&base, t);
        if f.locked || f.position_locked || f.pixels_locked {
            return Err(format!("Band source {key:?} is locked"));
        }
    }
    if let Some(timeline) = &d.animation {
        for (t, tr) in timeline.tracks.iter().enumerate() {
            for n in 0..tr.cells.len() {
                let f = p::inherited(
                    d,
                    p::Target::Cell {
                        track: t as u16,
                        cell: n as u16,
                    },
                );
                if f.locked || f.position_locked || f.pixels_locked {
                    return Err("An animation cell is locked".into());
                }
            }
        }
    }
    let (copied, new) = crate::layer_move::copy_sources(&base, &keys)?;
    state = copied;
    let map: std::collections::BTreeMap<_, _> =
        keys.iter().copied().zip(new.iter().copied()).collect();
    for layer in &mut state.layers {
        if layer.placement.is_none() {
            layer.placement = Some(crate::document::PlacedRaster {
                width: d.width,
                height: d.height,
                offset: [0; 2],
            });
        }
    }
    let span = 1e7;
    let upper = Rect {
        min: [-span, -span],
        max: [span, a as f64],
    };
    let lower = Rect {
        min: [-span, z as f64],
        max: [span, span],
    };
    for (old, new) in &map {
        mask(&mut state, *old, upper);
        mask(&mut state, *new, lower);
        if matches!(new, Key::World(_))
            && let Some(camera) = &d.scene.camera
        {
            mask(&mut state, *new, camera.page_bounds);
            mask(&mut state, *old, camera.page_bounds);
        }
        let current = crate::layer_merge::preview(&state, d.current_frame)?;
        // The ordinary move service preserves ruler, file-object, pattern and effect state.
        state = crate::search_move::prepare_unlinked(&current, *new, [0, delta])?;
    }
    for key in &new {
        if let Key::Object(id) = key
            && let Some(n) = state
                .scene
                .catalog
                .objects
                .index(*id, state.scene.objects.len())
            && let PageObject::Comic(o) = &mut state.scene.objects[n]
        {
            match &mut o.kind {
                Kind::Frame(f) => {
                    for k in &mut f.children {
                        if let Some(new) = map.get(k) {
                            *k = *new;
                        }
                    }
                    if f.family != 0 {
                        f.family = match map.get(&Key::Object(f.family)) {
                            Some(Key::Object(id)) => *id,
                            _ => *id,
                        };
                    }
                }
                Kind::Balloon(b) => {
                    if let Some(id) = &mut b.text
                        && let Some(Key::Object(new)) = map.get(&Key::Object(*id))
                    {
                        *id = *new;
                    }
                }
                _ => {}
            }
        }
    }
    if let Some(timeline) = &mut state.animation {
        for tr in &mut timeline.tracks {
            for cell in &mut tr.cells {
                let old = cell.clone();
                let end = old.offset[1] + old.height as i64;
                let map_y = |y: i64| {
                    if y < a as i64 {
                        Some(y)
                    } else if y >= z as i64 {
                        Some(y + delta)
                    } else {
                        None
                    }
                };
                let ys: Vec<_> = (old.offset[1]..end).filter_map(map_y).collect();
                let first = ys.iter().copied().min().unwrap_or(0);
                let last = ys.iter().copied().max().unwrap_or(first);
                let h = (last - first + 1) as u32;
                if h > 32768 {
                    return Err("Shifted animation cell height limit".into());
                }
                let count = old.width as usize * h as usize;
                if count.saturating_mul(4) > crate::document::DEFAULT_DOCUMENT_BUDGET / 16 {
                    return Err("Band animation cell allocation limit".into());
                }
                let mut pixels = vec![[0; 4]; count];
                for y in 0..old.height {
                    if let Some(to) = map_y(old.offset[1] + y as i64) {
                        let dst = (to - first) as usize * old.width as usize;
                        let src = y as usize * old.width as usize;
                        pixels[dst..dst + old.width as usize]
                            .copy_from_slice(&old.pixels[src..src + old.width as usize]);
                    }
                }
                cell.height = h;
                cell.offset[1] = first;
                cell.pixels = std::sync::Arc::new(pixels);
            }
        }
    }
    state.height = height;
    state.scene.comics.webtoon.cuts.retain_mut(|y| {
        if *y < a {
            true
        } else if *y >= z {
            *y = (*y as i64 + delta) as u32;
            *y < height
        } else {
            false
        }
    });
    state.scene.comics.webtoon.cuts.sort_unstable();
    state.scene.comics.webtoon.cuts.dedup();
    crate::layer_groups::finish(d, &state)?;
    Ok(state)
}
