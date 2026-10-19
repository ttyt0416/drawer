//! Draw the geometric boundary while preserving the original selection verbatim.
use crate::{document::Document, scene::Point, sizing::DocumentState};
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Alignment {
    Inside,
    #[default]
    Center,
    Outside,
}
impl Alignment {
    pub const ALL: [Self; 3] = [Self::Inside, Self::Center, Self::Outside];
}
#[derive(Clone, Copy, Debug)]
pub struct Settings {
    pub alignment: Alignment,
    pub width: f64,
    pub color: [u8; 4],
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            alignment: Alignment::Center,
            width: 1.0,
            color: [0, 0, 0, 255],
        }
    }
}
pub fn prepare(doc: &Document, layer: usize, s: Settings) -> Result<DocumentState, String> {
    if !s.width.is_finite() || !(0.0..=128.0).contains(&s.width) || !doc.scene.valid() {
        return Err("Border width must be 0–128 document pixels".into());
    }
    let mut edges = if let Some(mask) = &doc.scene.selection_mask {
        mask.boundaries()
    } else if let Some(r) = doc.scene.selection {
        let p = [r.min, [r.max[0], r.min[1]], r.max, [r.min[0], r.max[1]]];
        (0..4).map(|i| [p[i], p[(i + 1) % 4]]).collect()
    } else {
        return Err("Select an area before drawing its border".into());
    };
    for line in &mut edges {
        for p in line {
            *p = doc.scene.selection_point(*p);
        }
    }
    if edges
        .iter()
        .flatten()
        .flatten()
        .any(|v| !v.is_finite() || v.abs() > 1e9)
    {
        return Err("Selection border coordinate limit".into());
    }
    let count = doc.width as usize * doc.height as usize;
    if count > 4 * 1024 * 1024
        || count.saturating_mul(16).saturating_mul(edges.len()) > 64 * 1024 * 1024
    {
        return Err("Selection border sampling work limit".into());
    }
    let radius = if s.alignment == Alignment::Center {
        s.width / 2.0
    } else {
        s.width
    };
    let coverage = |x: u32, y: u32| -> u8 {
        if radius == 0.0 || s.color[3] == 0 {
            return 0;
        }
        let mut hits = 0u32;
        for sy in 0..4 {
            for sx in 0..4 {
                let p = [
                    x as f64 + (sx as f64 + 0.5) / 4.0,
                    y as f64 + (sy as f64 + 0.5) / 4.0,
                ];
                let inside = doc.scene.selection_coverage(p) > 0;
                if ((s.alignment == Alignment::Inside && !inside)
                    || (s.alignment == Alignment::Outside && inside))
                    || !near(p, &edges, radius)
                {
                    continue;
                }
                hits += 1;
            }
        }
        ((hits * 255 + 8) / 16) as u8
    };
    crate::bucket::paint_coverage(doc, layer, s.color, false, false, coverage)
}
fn near(p: Point, edges: &[[Point; 2]], radius: f64) -> bool {
    edges
        .iter()
        .any(|[a, b]| crate::line_tools::distance(p, *a, *b).0 <= radius)
}
pub fn verify(root: &std::path::Path) -> Result<(), String> {
    use crate::{
        document::{Background, CanvasSpec, DEFAULT_DOCUMENT_BUDGET},
        scene::Rect,
        selection::{Mask, Run},
    };
    let make = || {
        Document::create(
            CanvasSpec {
                width: 10,
                height: 10,
                dpi: 300.0,
                background: Background::Transparent,
            },
            DEFAULT_DOCUMENT_BUDGET,
        )
        .map_err(|e| e.to_string())
    };
    for alignment in Alignment::ALL {
        let mut d = make()?;
        d.scene.selection = Some(Rect {
            min: [2.0, 2.0],
            max: [8.0, 8.0],
        });
        let before = d.snapshot();
        let s = Settings {
            alignment,
            color: [200, 40, 20, 255],
            ..Default::default()
        };
        let result = prepare(&d, 0, s)?;
        let (outside, inside) = match alignment {
            Alignment::Inside => (0, 255),
            Alignment::Center => (128, 128),
            Alignment::Outside => (255, 0),
        };
        if result.layers[0].pixel(10, 10, 1, 4)[3] != outside
            || result.layers[0].pixel(10, 10, 2, 4)[3] != inside
            || result.layers[0].pixel(10, 10, 4, 4)[3] != 0
            || result.scene != before.scene
            || d.snapshot() != before
        {
            return Err(format!(
                "Border one-pixel independent alignment {alignment:?}"
            ));
        }
        d.commit_state(result)?;
        let after = d.snapshot();
        if d.history_counts() != (1, 0)
            || !d.undo()
            || d.snapshot() != before
            || !d.redo()
            || d.snapshot() != after
        {
            return Err("Border Undo/Redo".into());
        }
        let path = root.join("selection-border.drawer");
        crate::files::save(&mut d, &path).map_err(|e| e.to_string())?;
        if crate::files::open(&path, DEFAULT_DOCUMENT_BUDGET)
            .map_err(|e| e.to_string())?
            .snapshot()
            != after
        {
            return Err("Border native reopen".into());
        }
    }
    let mut d = make()?;
    let mut runs = vec![];
    for y in 1..9 {
        if (3..7).contains(&y) {
            runs.push(Run {
                x: 1,
                y,
                length: 2,
                alpha: 128,
            });
            runs.push(Run {
                x: 7,
                y,
                length: 2,
                alpha: 255,
            });
        } else {
            runs.push(Run {
                x: 1,
                y,
                length: 8,
                alpha: 128,
            });
        }
    }
    d.scene.selection_mask = Some(Mask { runs });
    let scene = d.scene.clone();
    let out = prepare(
        &d,
        0,
        Settings {
            alignment: Alignment::Outside,
            color: [200, 40, 20, 128],
            ..Default::default()
        },
    )?;
    if out.layers[0].pixel(10, 10, 3, 4)[3] != 128
        || out.layers[0].pixel(10, 10, 4, 4)[3] != 0
        || out.layers[0].pixel(10, 10, 2, 4)[3] != 0
        || out.scene != scene
    {
        return Err("Border hole/soft selection preservation".into());
    }
    d.scene.selection_mask = None;
    d.scene.selection = Some(Rect {
        min: [-3.0, 2.0],
        max: [8.0, 8.0],
    });
    let out = prepare(
        &d,
        0,
        Settings {
            alignment: Alignment::Inside,
            ..Default::default()
        },
    )?;
    if out.layers[0].pixel(10, 10, 0, 4)[3] != 0 || out.layers[0].pixel(10, 10, 7, 4)[3] != 255 {
        return Err("Open visible border acquired a false canvas-edge segment".into());
    }
    for width in [0.0, 1.0, 2.5, 128.0] {
        let _ = prepare(
            &d,
            0,
            Settings {
                width,
                ..Default::default()
            },
        )?;
    }
    for width in [f64::NAN, -1.0, 129.0] {
        if prepare(
            &d,
            0,
            Settings {
                width,
                ..Default::default()
            },
        )
        .is_ok()
        {
            return Err("Invalid border width accepted".into());
        }
    }
    d.scene.selection = Some(Rect {
        min: [1.0, 1.0],
        max: [4.0, 4.0],
    });
    d.scene.transform = crate::scene::Affine([2.0, 0.0, 0.0, 2.0, 0.0, 0.0]);
    d.scene.selection_translation = [1.0, 0.0];
    let out = prepare(
        &d,
        0,
        Settings {
            alignment: Alignment::Inside,
            ..Default::default()
        },
    )?;
    if out.layers[0].pixel(10, 10, 3, 4)[3] != 255
        || out.layers[0].pixel(10, 10, 4, 4)[3] != 0
        || out.scene != d.scene
    {
        return Err("Border transform/translation/document width".into());
    }
    d.scene.selection = None;
    d.scene.selection_mask = Some(Mask::default());
    if prepare(&d, 0, Default::default())? != d.snapshot() {
        return Err("Empty border changed document".into());
    }
    d.layers[0].locked = true;
    if prepare(&d, 0, Default::default()).is_ok() {
        return Err("Locked border target accepted".into());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn alignments_holes_open_empty_widths_selection_history_native() {
        let root = crate::test_support::TempDir::new();
        super::verify(&root.0).unwrap();
    }
}
