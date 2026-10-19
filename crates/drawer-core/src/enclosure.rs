//! Shape fill and bounded, truly enclosed transparent components.
use crate::{document::Document, scene::Point, sizing::DocumentState};
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Lasso,
    Enclosed,
    SmallGaps,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub maximum_area: u32,
}
impl Default for Settings {
    fn default() -> Self {
        Self { maximum_area: 64 }
    }
}
impl Settings {
    pub fn valid(self) -> bool {
        (1..=4 * 1024 * 1024).contains(&self.maximum_area)
    }
}
#[allow(clippy::too_many_arguments)]
pub fn prepare(
    doc: &Document,
    layer: usize,
    points: &[Point],
    mode: Mode,
    settings: Settings,
    reference: crate::bucket::Settings,
    color: [u8; 4],
    erase: bool,
    alpha_lock: bool,
) -> Result<DocumentState, String> {
    if !settings.valid() {
        return Err("Small-area maximum must be 1–4194304 pixels".into());
    }
    let scope = if points.is_empty() {
        if mode != Mode::SmallGaps {
            return Err("Trace a closed lasso first".into());
        }
        None
    } else {
        Some(crate::selection_tools::prepare_lasso(doc, points)?)
    };
    let count = doc.width as usize * doc.height as usize;
    if count > 4 * 1024 * 1024 {
        return Err("Enclosed fill exceeds work limit".into());
    }
    let mut coverage = vec![0u8; count];
    for y in 0..doc.height {
        for x in 0..doc.width {
            let p = [x as f64 + 0.5, y as f64 + 0.5];
            let a = scope.as_ref().map_or(255, |s| s.selection_coverage(p));
            let b = doc.scene.selection_coverage(p);
            coverage[y as usize * doc.width as usize + x as usize] =
                ((a as u32 * b as u32 + 127) / 255) as u8;
        }
    }
    if mode != Mode::Lasso {
        let sampler = crate::fill_reference::Sampler::new(doc, layer, reference)?;
        let mut eligible = vec![false; count];
        for y in 0..doc.height {
            for x in 0..doc.width {
                eligible[y as usize * doc.width as usize + x as usize] =
                    sampler.pixel(x, y)[3] as f64 / 255.0 <= reference.alpha_tolerance;
            }
        }
        let mut seen = vec![false; count];
        let mut accepted = vec![false; count];
        let mut queue = Vec::<u32>::new();
        queue
            .try_reserve(count)
            .map_err(|_| "Enclosed component allocation failed")?;
        for start in 0..count {
            if seen[start] || !eligible[start] {
                continue;
            }
            queue.clear();
            queue.push(start as u32);
            seen[start] = true;
            let mut head = 0;
            let mut outside = false;
            while head < queue.len() {
                let p = queue[head] as usize;
                head += 1;
                let x = p % doc.width as usize;
                let y = p / doc.width as usize;
                outside |= x == 0
                    || y == 0
                    || x + 1 == doc.width as usize
                    || y + 1 == doc.height as usize
                    || coverage[p] == 0;
                for q in crate::bucket::neighbors(p, doc.width as usize, doc.height as usize)
                    .into_iter()
                    .flatten()
                {
                    if !seen[q] && eligible[q] {
                        seen[q] = true;
                        queue.push(q as u32);
                    }
                }
            }
            if !outside
                && (mode != Mode::SmallGaps || queue.len() <= settings.maximum_area as usize)
            {
                for &p in &queue {
                    accepted[p as usize] = true;
                }
            }
        }
        for (value, ok) in coverage.iter_mut().zip(accepted) {
            if !ok {
                *value = 0;
            }
        }
    }
    crate::bucket::paint_coverage(doc, layer, color, erase, alpha_lock, |x, y| {
        coverage[y as usize * doc.width as usize + x as usize]
    })
}
pub fn verify(root: &std::path::Path) -> Result<(), String> {
    use crate::{
        bucket::Reference,
        document::{Background, CanvasSpec, DEFAULT_DOCUMENT_BUDGET},
    };
    let mut d = Document::create(
        CanvasSpec {
            width: 7,
            height: 7,
            dpi: 300.0,
            background: Background::Transparent,
        },
        DEFAULT_DOCUMENT_BUDGET,
    )
    .map_err(|e| e.to_string())?;
    d.layers.push(d.layers[0].clone());
    for v in 1..6 {
        for (x, y) in [(v, 1), (v, 5), (1, v), (5, v)] {
            d.layers[1].pixels[y * 7 + x] = [0, 0, 0, 255];
        }
    }
    let p = [[0.0, 0.0], [7.0, 0.0], [7.0, 7.0], [0.0, 7.0]];
    let r = crate::bucket::Settings {
        reference: Reference::Layer(1),
        ..Default::default()
    };
    let before = d.snapshot();
    for (mode, max, filled) in [
        (Mode::Enclosed, 1, true),
        (Mode::SmallGaps, 9, true),
        (Mode::SmallGaps, 8, false),
    ] {
        let state = prepare(
            &d,
            0,
            &p,
            mode,
            Settings { maximum_area: max },
            r,
            [200, 40, 20, 128],
            false,
            false,
        )?;
        if state.layers[0].pixel(7, 7, 3, 3)[3] != if filled { 128 } else { 0 }
            || state.layers[0].pixel(7, 7, 0, 0)[3] != 0
            || state.scene != before.scene
            || d.snapshot() != before
        {
            return Err("Closed component/area threshold/background protection".into());
        }
    }
    let state = prepare(
        &d,
        0,
        &p,
        Mode::Enclosed,
        Default::default(),
        r,
        [200, 40, 20, 128],
        false,
        false,
    )?;
    d.commit_state(state)?;
    let after = d.snapshot();
    if !d.undo() || d.snapshot() != before || !d.redo() || d.snapshot() != after {
        return Err("Enclosure Undo/Redo".into());
    }
    let path = root.join("enclosed-fill.drawer");
    crate::files::save(&mut d, &path).map_err(|e| e.to_string())?;
    if crate::files::open(&path, DEFAULT_DOCUMENT_BUDGET)
        .map_err(|e| e.to_string())?
        .snapshot()
        != after
    {
        return Err("Enclosed fill native reopen".into());
    }
    d.undo();
    d.layers[1].pixels[7 + 3] = [0; 4];
    let before = d.snapshot();
    if prepare(
        &d,
        0,
        &p,
        Mode::Enclosed,
        Default::default(),
        r,
        [200, 40, 20, 255],
        false,
        false,
    )? != before
    {
        return Err("One-pixel gap leaked into the background".into());
    }
    let state = prepare(
        &d,
        0,
        &[[0.5, 0.5], [2.5, 0.5], [2.5, 2.5], [0.5, 2.5]],
        Mode::Lasso,
        Default::default(),
        r,
        [200, 40, 20, 128],
        false,
        false,
    )?;
    if state.layers[0].pixel(7, 7, 0, 0)[3] != 32
        || state.layers[0].pixel(7, 7, 1, 1)[3] != 128
        || state.layers[0].pixel(7, 7, 3, 3)[3] != 0
        || state.scene != d.scene
    {
        return Err("Lasso edge coverage/outside preservation".into());
    }
    if prepare(
        &d,
        0,
        &[[1.0; 2]; 3],
        Mode::Lasso,
        Default::default(),
        r,
        [0; 4],
        false,
        false,
    )
    .is_ok()
    {
        return Err("Degenerate lasso accepted".into());
    }
    if prepare(
        &d,
        0,
        &p,
        Mode::SmallGaps,
        Settings { maximum_area: 0 },
        r,
        [0; 4],
        false,
        false,
    )
    .is_ok()
    {
        return Err("Invalid area limit accepted".into());
    }
    d.layers[0].locked = true;
    if prepare(
        &d,
        0,
        &p,
        Mode::Enclosed,
        Default::default(),
        r,
        [0; 4],
        false,
        false,
    )
    .is_ok()
    {
        return Err("Locked enclosure target accepted".into());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn closed_lasso_small_area_gap_background_edges_history_native() {
        let root = crate::test_support::TempDir::new();
        super::verify(&root.0).unwrap();
    }
}
