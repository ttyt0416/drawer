//! Color-range selection is independent of connectivity and view color effects.
use crate::{
    document::{DEFAULT_DOCUMENT_BUDGET, Document},
    scene::Scene,
    selection_edit::{Combine, Grid},
};
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Reference {
    #[default]
    Current,
    Composite,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub color: [u8; 4],
    pub tolerance: u8,
    pub include_transparent: bool,
    pub reference: Reference,
    pub combine: Combine,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            color: [0, 0, 0, 255],
            tolerance: 0,
            include_transparent: false,
            reference: Reference::Current,
            combine: Combine::Replace,
        }
    }
}
pub fn prepare(doc: &Document, layer: Option<usize>, s: Settings) -> Result<Scene, String> {
    let count = doc.width as usize * doc.height as usize;
    if count > 4 * 1024 * 1024
        || doc
            .resident_bytes()
            .saturating_add(count.saturating_mul(16))
            > DEFAULT_DOCUMENT_BUDGET
    {
        return Err("Color range exceeds work/memory limit".into());
    }
    let raster = if s.reference == Reference::Current {
        Some(
            doc.layers
                .get(layer.ok_or("Choose a raster layer or the visible composite")?)
                .filter(|l| l.valid(doc.width, doc.height))
                .ok_or("Invalid color-range reference")?,
        )
    } else {
        None
    };
    if raster.is_none() {
        let cost = doc.layers.len()
            + doc.scene.objects.len()
            + doc
                .scene
                .lines
                .layers
                .iter()
                .flat_map(|l| &l.paths)
                .map(|p| p.points.len())
                .sum::<usize>()
            + 1;
        if count.saturating_mul(cost) > 64 * 1024 * 1024 {
            return Err("Composite color range exceeds work limit".into());
        }
    }
    let renderer = raster.is_none().then(|| crate::render::Renderer::new(doc));
    let mut values = vec![0u8; count];
    for y in 0..doc.height {
        for x in 0..doc.width {
            let p = raster.map_or_else(
                || {
                    renderer
                        .as_ref()
                        .unwrap()
                        .pixel([x as f64 + 0.5, y as f64 + 0.5])
                },
                |r| r.pixel(doc.width, doc.height, x as i64, y as i64),
            );
            let matches = if p[3] == 0 {
                s.include_transparent
            } else {
                (0..3).all(|i| p[i].abs_diff(s.color[i]) <= s.tolerance)
            };
            values[y as usize * doc.width as usize + x as usize] = if matches { 255 } else { 0 };
        }
    }
    let grid = Grid::new(doc)?;
    let incoming = grid.sample(doc, true, |p| {
        values[p[1].floor() as usize * doc.width as usize + p[0].floor() as usize]
    });
    let out = grid
        .base(doc, s.combine == Combine::Add)
        .into_iter()
        .zip(incoming)
        .map(|(a, b)| s.combine.coverage(a, b))
        .collect::<Vec<_>>();
    grid.scene(doc, &out)
}
pub fn verify(root: &std::path::Path) -> Result<(), String> {
    use crate::{
        document::{Background, CanvasSpec},
        selection::{Mask, Run},
    };
    let mut d = Document::create(
        CanvasSpec {
            width: 5,
            height: 1,
            dpi: 300.0,
            background: Background::Transparent,
        },
        DEFAULT_DOCUMENT_BUDGET,
    )
    .map_err(|e| e.to_string())?;
    d.layers[0].pixels = vec![
        [10, 20, 30, 255],
        [11, 20, 30, 128],
        [200, 0, 0, 255],
        [10, 20, 30, 255],
        [99, 80, 60, 0],
    ];
    let s = Settings {
        color: [10, 20, 30, 100],
        ..Default::default()
    };
    let before = d.snapshot();
    for (t, alpha, expected) in [
        (0, false, [255, 0, 0, 255, 0]),
        (1, false, [255, 255, 0, 255, 0]),
        (255, false, [255, 255, 255, 255, 0]),
        (0, true, [255, 0, 0, 255, 255]),
        (255, true, [255; 5]),
    ] {
        let scene = prepare(
            &d,
            Some(0),
            Settings {
                tolerance: t,
                include_transparent: alpha,
                ..s
            },
        )?;
        for (x, alpha) in expected.into_iter().enumerate() {
            if scene.selection_coverage([x as f64 + 0.5, 0.5]) != alpha {
                return Err(format!("Color range threshold/transparent {t}/{alpha}/{x}"));
            }
        }
        if d.snapshot() != before {
            return Err("Color range discovery mutated pixels".into());
        }
    }
    let scene = prepare(&d, Some(0), s)?;
    d.edit_scene(scene)?;
    let after = d.snapshot();
    if after.layers != before.layers
        || !d.undo()
        || d.snapshot() != before
        || !d.redo()
        || d.snapshot() != after
    {
        return Err("Color range history/pixels".into());
    }
    let path = root.join("color-range.drawer");
    crate::files::save(&mut d, &path).map_err(|e| e.to_string())?;
    if crate::files::open(&path, DEFAULT_DOCUMENT_BUDGET)
        .map_err(|e| e.to_string())?
        .snapshot()
        != d.snapshot()
    {
        return Err("Color range native reopen".into());
    }
    d.scene.selection_mask = Some(Mask {
        runs: vec![Run {
            x: 0,
            y: 0,
            length: 5,
            alpha: 128,
        }],
    });
    for (op, a, b) in [
        (Combine::Add, 255, 128),
        (Combine::Subtract, 0, 128),
        (Combine::Intersect, 128, 0),
    ] {
        let scene = prepare(&d, Some(0), Settings { combine: op, ..s })?;
        if scene.selection_coverage([0.5, 0.5]) != a || scene.selection_coverage([2.5, 0.5]) != b {
            return Err("Color range selection composition".into());
        }
    }
    d.scene.paper.layers.entry(0).or_default().hidden = true;
    let composite = prepare(
        &d,
        None,
        Settings {
            reference: Reference::Composite,
            ..s
        },
    )?;
    if composite.selection_coverage([0.5, 0.5]) != 0 {
        return Err("Composite included hidden raster".into());
    }
    if prepare(&d, None, s).is_ok() || prepare(&d, Some(999), s).is_ok() {
        return Err("Missing color-range reference accepted".into());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn disconnected_colors_thresholds_transparency_composite_history() {
        let root = crate::test_support::TempDir::new();
        super::verify(&root.0).unwrap();
    }
}
