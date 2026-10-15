//! Owned marker, pigment-only smudge, and grid-aligned pixel pen verification.
use crate::{
    assets::BrushTip,
    brush_cursor::Shape,
    document::{Background, CanvasSpec, Document},
    painting::{Stroke, Style},
};
fn document() -> Result<Document, String> {
    Document::create(
        CanvasSpec {
            width: 32,
            height: 16,
            dpi: 300.0,
            background: Background::Transparent,
        },
        8192,
    )
    .map_err(|e| e.to_string())
}
fn style(tip: BrushTip) -> Style {
    let v = crate::basic_brush::values(tip);
    Style {
        pattern: Default::default(),
        dynamics: Default::default(),
        sequence: Default::default(),
        filters: Default::default(),
        profile: Default::default(),
        texture: Default::default(),
        material: crate::basic_brush::material(tip),
        shape: Shape {
            size: v[0],
            aspect: v[1],
            angle: v[2],
        },
        tip,
        color: [230, 40, 20, 128],
        smoothing: 0.0,
        opacity: 1.0,
        hardness: v[5],
        spacing: v[6],
        pressure: crate::pressure::Mapping {
            size: false,
            opacity: false,
            ..Default::default()
        },
    }
}
pub fn verify() -> Result<(), String> {
    let mut one = document()?;
    let mut s = style(BrushTip::Marker);
    s.pressure.opacity = true;
    s.shape.size = 4.0;
    let mut stroke = Stroke::begin(&one, 0, s)?;
    stroke.direct_dab(&one, [8.5; 2], [8.5; 2], 0.5)?;
    stroke.commit(&mut one)?;
    let once = one.composite_pixel(8, 8)[3];
    let mut many = document()?;
    let before = many.snapshot();
    let mut stroke = Stroke::begin(&many, 0, s)?;
    for _ in 0..8 {
        stroke.direct_dab(&many, [8.5; 2], [8.5; 2], 0.5)?;
    }
    stroke.commit(&mut many)?;
    if once == 0
        || many.composite_pixel(8, 8)[3] <= once
        || many.history_counts() != (1, 0)
        || !many.undo()
        || many.snapshot() != before
        || !many.redo()
    {
        return Err("Marker revisit did not build up in one undo".into());
    }
    let mut d = document()?;
    let mut s = style(BrushTip::PixelPen);
    s.color = [20, 60, 220, 128];
    s.shape.size = 1.0;
    s.hardness = 0.0;
    let mut stroke = Stroke::begin(&d, 0, s)?;
    stroke.sample(&d, [2.5, 2.5])?;
    stroke.sample(&d, [12.5, 12.5])?;
    stroke.commit(&mut d)?;
    for y in 0..16 {
        for x in 0..32 {
            let p = d.composite_pixel(x, y);
            let expected = if x == y && (2..=12).contains(&x) {
                [20, 60, 220, 128]
            } else {
                [0; 4]
            };
            if p != expected {
                return Err(format!(
                    "Pixel pen diagonal/transparent boundary diverged at {x},{y}"
                ));
            }
        }
    }
    let mut d = document()?;
    for y in 0..16 {
        for x in 0..9 {
            d.layers[0].pixels[y * 32 + x] = [220, 40, 10, 150];
        }
    }
    let before = d.snapshot();
    let mut s = style(BrushTip::Smudge);
    s.color = [0, 255, 0, 0];
    s.shape.size = 4.0;
    s.material.keep_density = true;
    let mut stroke = Stroke::begin(&d, 0, s)?;
    stroke.sample(&d, [6.5, 8.5])?;
    stroke.sample(&d, [24.5, 8.5])?;
    stroke.commit(&mut d)?;
    let p = d.composite_pixel(20, 8);
    if p[3] == 0
        || p[0] == 0
        || p[1] >= 100
        || d.history_counts() != (1, 0)
        || !d.undo()
        || d.snapshot() != before
        || !d.redo()
    {
        return Err("Smudge added foreground color or lost its undo".into());
    }
    let mut empty = document()?;
    let mut s = style(BrushTip::Smudge);
    s.color = [0, 255, 0, 255];
    let mut stroke = Stroke::with_budget(&empty, 0, s, 65536)?;
    stroke.sample(&empty, [4.5, 8.5])?;
    stroke.sample(&empty, [24.5, 8.5])?;
    if stroke.commit(&mut empty)? || empty.layers[0].pixels.iter().any(|p| p[3] != 0) {
        return Err("Smudge manufactured foreground pigment on an empty layer".into());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn marker_revisit_smudge_transport_pixel_diagonal_and_single_history() {
        verify().unwrap();
    }
    #[test]
    fn pixel_footprints_even_odd_transparent_edges_and_view_are_binary() {
        for size in [0.1, 1.0, 2.0, 3.0, 4.0] {
            let mut d = document().unwrap();
            d.view.angle = 37.0;
            d.view.flipped = true;
            let mut s = style(BrushTip::PixelPen);
            s.shape.size = size;
            s.color = [40, 80, 120, 128];
            let mut stroke = Stroke::begin(&d, 0, s).unwrap();
            for _ in 0..5 {
                stroke.direct_dab(&d, [8.2, 7.8], [8.2, 7.8], 1.0).unwrap();
            }
            stroke.commit(&mut d).unwrap();
            let width = size.round().max(1.0) as usize;
            assert_eq!(
                d.layers[0].pixels.iter().filter(|p| p[3] > 0).count(),
                width * width
            );
            assert!(
                d.layers[0]
                    .pixels
                    .iter()
                    .all(|p| p[3] == 0 || *p == [40, 80, 120, 128])
            );
        }
    }
    #[test]
    fn new_brush_presets_and_native_reopen_keep_tip_and_resolved_pigment() {
        let mut invalid = style(BrushTip::Smudge);
        invalid.material.supply = f64::NAN;
        assert!(Stroke::with_budget(&document().unwrap(), 0, invalid, 65536).is_err());
        let root = crate::test_support::TempDir::new();
        let path = root.0.join("tools.drawer");
        let base = crate::workspaces::Snapshot {
            properties: Some(Default::default()),
            ..Default::default()
        };
        for tip in [BrushTip::Marker, BrushTip::Smudge, BrushTip::PixelPen] {
            let mut next = base.clone();
            let n = BrushTip::ALL.iter().position(|t| *t == tip).unwrap();
            next.input.textures[n] = crate::texture::Pattern {
                source: 2,
                ..Default::default()
            };
            let p = crate::brush_presets::Preset::capture(&next, tip, tip.label()).unwrap();
            let p = crate::brush_presets::Preset::decode(&p.encode().unwrap()).unwrap();
            let applied = p.apply(&base).unwrap();
            assert_eq!(applied.input.textures[n], next.input.textures[n]);
            assert_eq!(applied.tool, tip.name());
            let mut d = document().unwrap();
            let s = style(tip);
            if tip == BrushTip::Smudge {
                for y in 0..16 {
                    for x in 0..9 {
                        d.layers[0].pixels[y * 32 + x] = [200, 60, 20, 255];
                    }
                }
            }
            let mut replay = crate::mouse_pressure::Replay::begin(&d, 0, s, Default::default())
                .unwrap()
                .configure_input(Default::default(), true)
                .unwrap();
            for point in [[6.5, 8.5], [20.5, 8.5]] {
                replay
                    .push_input(
                        crate::mouse_pressure::Sample {
                            point,
                            micros: None,
                        },
                        1.0,
                        0.0,
                    )
                    .unwrap();
            }
            let m = replay.metadata();
            replay
                .render(&d, true)
                .unwrap()
                .commit_mouse(&mut d, m)
                .unwrap();
            let before = d.snapshot();
            crate::files::save(&mut d, &path).unwrap();
            let reopened = crate::files::open(&path, 1_000_000).unwrap();
            assert!(reopened.snapshot() == before);
            assert_eq!(reopened.scene.mouse_strokes[0].style.tip, tip);
        }
    }
    #[test]
    fn rotated_pixel_rectangle_is_not_clipped_by_elliptical_bounds() {
        let mut d = document().unwrap();
        let mut s = style(BrushTip::PixelPen);
        s.shape.size = 12.0;
        s.shape.angle = 45.0;
        s.color = [90, 140, 210, 128];
        let mut stroke = Stroke::begin(&d, 0, s).unwrap();
        stroke
            .direct_dab(&d, [16.5, 8.5], [16.5, 8.5], 1.0)
            .unwrap();
        stroke.commit(&mut d).unwrap();
        assert!(
            d.layers[0]
                .pixels
                .iter()
                .all(|p| p[3] == 0 || *p == s.color)
        );
        assert!(d.composite_pixel(23, 8)[3] > 0);
        assert!(d.composite_pixel(9, 8)[3] > 0);
    }
}
