use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub slots: [[u8; 4]; 2],
    pub active: u8,
    pub transparent: bool,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            slots: [[0, 0, 0, 255], [255; 4]],
            active: 0,
            transparent: false,
        }
    }
}
impl Settings {
    pub fn valid(self) -> bool {
        self.active < 2
    }
    pub fn observe(&mut self, color: [u8; 4]) {
        if let Some(slot) = self.slots.get_mut(self.active as usize) {
            *slot = color
        }
    }
    pub fn select(&mut self, slot: u8, current: [u8; 4]) -> Result<[u8; 4], String> {
        if !self.valid() || slot > 1 {
            return Err("Invalid primary/secondary color slot".into());
        }
        self.observe(current);
        self.active = slot;
        self.transparent = false;
        Ok(self.slots[slot as usize])
    }
    pub fn swap(&mut self, current: [u8; 4]) -> Result<[u8; 4], String> {
        if !self.valid() {
            return Err("Invalid drawing color slots".into());
        }
        self.observe(current);
        self.slots.swap(0, 1);
        self.transparent = false;
        Ok(self.slots[self.active as usize])
    }
}

fn style(tip: crate::assets::BrushTip) -> crate::painting::Style {
    let s = crate::workspaces::Snapshot {
        properties: Some(Default::default()),
        ..Default::default()
    };
    let mut p = crate::brush_presets::Preset::capture(&s, tip, "Transparent shape fixture")
        .unwrap()
        .style([200, 50, 30, 255]);
    p.shape = crate::brush_cursor::Shape {
        size: 8.0,
        aspect: 1.0,
        angle: 0.0,
    };
    p.material = Default::default();
    p.pressure.size = false;
    p.pressure.opacity = false;
    p.hardness = 1.0;
    p.opacity = 1.0;
    p.smoothing = 0.0;
    p
}
fn document(opaque: bool) -> Result<crate::document::Document, String> {
    crate::document::Document::create(
        crate::document::CanvasSpec {
            width: 24,
            height: 24,
            dpi: 300.0,
            background: if opaque {
                crate::document::Background::Color([80, 90, 100])
            } else {
                crate::document::Background::Transparent
            },
        },
        crate::document::DEFAULT_DOCUMENT_BUDGET,
    )
    .map_err(|e| e.to_string())
}
pub fn verify(root: &std::path::Path) -> Result<(), String> {
    use crate::{assets::BrushTip, painting::Stroke, tip_profile::Image};
    let lib = crate::material_library::Library::default()
        .register_asset(
            crate::material_library::Asset {
                id: 1024,
                name: "Owned asymmetric fixture".into(),
                kind: crate::material_library::Kind::Tip,
                width: 3,
                height: 3,
                values: vec![255, 0, 0, 0, 128, 0, 0, 0, 64],
                density: Default::default(),
                fibres: None,
            },
            false,
        )
        .map_err(|e| e.to_string())?;
    let tip_id = lib.assets()[0].id;
    let lib = std::sync::Arc::new(lib);
    for tip in BrushTip::ALL {
        let mut d = document(true)?;
        let before = d.snapshot();
        let mut s = style(tip);
        s.color[3] = 0;
        s.profile.transparent = true;
        s.material.mode = crate::brush_material::Mode::Water;
        s.material.water = 1.0;
        s.material.edge = 1.0;
        s.material.alpha_lock = true;
        s.material.tonal.mode = crate::tonal::Mode::Burn;
        s.material.composition.mode = crate::brush_composite::Mode::ReplaceAlpha;
        let mut stroke = Stroke::begin(&d, 0, s)?;
        stroke.direct_dab(&d, [12.5, 12.5], [12.5, 12.5], 1.0)?;
        stroke.commit(&mut d)?;
        if !d.layers[0].pixels.iter().any(|p| p[3] < 255)
            || d.layers[0].pixels.iter().any(|p| p[..3] != [80, 90, 100])
        {
            return Err(format!(
                "Transparent {tip:?} did not erase alpha with original RGB"
            ));
        }
        let after = d.snapshot();
        if d.history_counts() != (1, 0)
            || !d.undo()
            || d.snapshot() != before
            || !d.redo()
            || d.snapshot() != after
        {
            return Err("Transparent stroke undo/redo mismatch".into());
        }
    }
    for tip in [
        BrushTip::Pen,
        BrushTip::Pencil,
        BrushTip::Wash,
        BrushTip::Marker,
        BrushTip::PixelPen,
    ] {
        for image in [Image::Round, Image::Square] {
            for bitmap in [false, true] {
                let mut paint = document(false)?;
                let mut erase = document(true)?;
                let mut s = style(tip);
                s.profile.image = image;
                if bitmap {
                    s.sequence.tips[0] = tip_id;
                }
                let mut a = Stroke::begin(&paint, 0, s)?.with_materials(lib.clone())?;
                a.set_seed(13);
                a.direct_dab(&paint, [12.5, 12.5], [12.5, 12.5], 1.0)?;
                a.commit(&mut paint)?;
                s.profile.transparent = true;
                s.color[3] = 0;
                let mut b = Stroke::begin(&erase, 0, s)?.with_materials(lib.clone())?;
                b.set_seed(13);
                b.direct_dab(&erase, [12.5, 12.5], [12.5, 12.5], 1.0)?;
                b.commit(&mut erase)?;
                for (a, b) in paint.layers[0].pixels.iter().zip(&erase.layers[0].pixels) {
                    if (a[3] as i32 + b[3] as i32 - 255).abs() > 1 || b[..3] != [80, 90, 100] {
                        return Err(format!(
                            "Transparent tip shape/density mismatch: {tip:?}/{image:?}/{bitmap}"
                        ));
                    }
                }
            }
        }
    }
    let mut d = document(true)?;
    d.scene.lines.settings.clip_paint = true;
    let mut s = style(BrushTip::Pen);
    s.profile.image = Image::Square;
    s.profile.transparent = true;
    s.color[3] = 0;
    let before = d.snapshot();
    let mut replay = crate::mouse_pressure::Replay::begin(&d, 0, s, Default::default())?;
    replay.push_input(
        crate::mouse_pressure::Sample {
            point: [12.5, 12.5],
            micros: Some(0),
        },
        1.0,
        0.0,
    )?;
    replay
        .render(&d, true)?
        .commit_mouse(&mut d, replay.metadata())?;
    if d.layers[0].pixel(24, 24, 15, 15)[3] != 0
        || d.scene
            .mouse_strokes
            .last()
            .is_none_or(|m| !m.style.profile.transparent || m.style.tip != BrushTip::Pen)
    {
        return Err("Square transparent corner/replay metadata mismatch".into());
    }
    let after = d.snapshot();
    let file = root.join("transparent-shape.drawer");
    crate::files::save(&mut d, &file).map_err(|e| e.to_string())?;
    let reopened = crate::files::open(&file, crate::document::DEFAULT_DOCUMENT_BUDGET)
        .map_err(|e| e.to_string())?;
    if reopened.snapshot() != after
        || !d.undo()
        || d.snapshot() != before
        || !d.redo()
        || d.snapshot() != after
    {
        return Err("Transparent native/undo metadata mismatch".into());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn colors_switch_swap_alpha_and_invalid_slot() {
        let mut s = Settings::default();
        let red = [220, 30, 40, 0];
        assert_eq!(s.select(1, red).unwrap(), [255; 4]);
        assert_eq!(s.select(0, [20, 80, 160, 128]).unwrap(), red);
        assert_eq!(s.swap(red).unwrap(), [20, 80, 160, 128]);
        assert_eq!(s.slots[1], red);
        let before = s;
        assert!(s.select(2, [0; 4]).is_err());
        assert_eq!(s, before);
    }
    #[test]
    fn transparent_shapes_density_native_and_undo() {
        let root = crate::test_support::TempDir::new();
        verify(&root.0).unwrap();
    }
    #[test]
    fn transparent_selection_opacity_and_legacy_default() {
        let mut d = document(true).unwrap();
        d.layers[0].pixels.fill([80, 90, 100, 128]);
        d.scene.selection_mask = Some(crate::selection::Mask {
            runs: vec![crate::selection::Run {
                x: 12,
                y: 12,
                length: 1,
                alpha: 128,
            }],
        });
        let mut s = style(crate::assets::BrushTip::Pen);
        s.profile.transparent = true;
        s.shape.size = 1.0;
        s.opacity = 0.5;
        s.material.alpha_lock = true;
        s.color[3] = 0;
        let mut stroke = crate::painting::Stroke::begin(&d, 0, s).unwrap();
        stroke.sample(&d, [12.5, 12.5]).unwrap();
        stroke.commit(&mut d).unwrap();
        assert_eq!(d.layers[0].pixel(24, 24, 12, 12), [80, 90, 100, 96]);
        assert_eq!(d.layers[0].pixel(24, 24, 11, 12), [80, 90, 100, 128]);
        let mut value = serde_json::to_value(crate::tip_profile::Settings::default()).unwrap();
        value.as_object_mut().unwrap().remove("transparent");
        assert!(
            !serde_json::from_value::<crate::tip_profile::Settings>(value)
                .unwrap()
                .transparent
        );
    }
}
