//! Tip-local texture coordinates and coverage modes, independent of layer blending.
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Space {
    #[default]
    Document,
    Tip,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Mode {
    Normal,
    #[default]
    Multiply,
    Subtract,
    Compare,
    Outline,
    Overlay,
    ColorDodge,
    ColorBurn,
    HardMix,
    Height,
}
impl Mode {
    pub const ALL: [Self; 10] = [
        Self::Normal,
        Self::Multiply,
        Self::Subtract,
        Self::Compare,
        Self::Outline,
        Self::Overlay,
        Self::ColorDodge,
        Self::ColorBurn,
        Self::HardMix,
        Self::Height,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Normal => "Normal",
            Self::Multiply => "Multiply",
            Self::Subtract => "Subtract",
            Self::Compare => "Compare",
            Self::Outline => "Outline",
            Self::Overlay => "Overlay",
            Self::ColorDodge => "Color dodge",
            Self::ColorBurn => "Color burn",
            Self::HardMix => "Hard mix",
            Self::Height => "Height",
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub pattern: crate::texture::Pattern,
    pub space: Space,
    pub flip: [bool; 2],
    pub brightness: f64,
    pub contrast: f64,
    pub mode: Mode,
}
impl Settings {
    pub fn valid(self) -> bool {
        self.pattern.valid()
            && (-1.0..=1.0).contains(&self.brightness)
            && (-1.0..=1.0).contains(&self.contrast)
    }
    pub fn cost(self) -> u64 {
        if self.mode == Mode::Outline { 5 } else { 1 }
    }
    fn adjusted(self, v: f64) -> f64 {
        ((v - 0.5) * (1.0 + self.contrast) + 0.5 + self.brightness).clamp(0.0, 1.0)
    }
    pub fn value(self, a: f64, b: f64, gradient: f64) -> f64 {
        use crate::brush_variation::Blend;
        match self.mode {
            Mode::Normal => b,
            Mode::Multiply => a * b,
            Mode::Subtract => (a - b).max(0.0),
            Mode::Compare => a.max(b),
            Mode::Outline => a * gradient,
            Mode::Overlay => Blend::Overlay.channel(a, b),
            Mode::ColorDodge => Blend::ColorDodge.channel(a, b),
            Mode::ColorBurn => Blend::ColorBurn.channel(a, b),
            Mode::HardMix => Blend::HardMix.channel(a, b),
            Mode::Height => (a + b) * 0.5,
        }
    }
    pub fn apply(
        self,
        a: f64,
        point: [f64; 2],
        tip: [f64; 2],
        library: &crate::material_library::Library,
    ) -> f64 {
        if a <= 0.0 || self.pattern.strength == 0.0 {
            return a;
        }
        let mut p = match self.space {
            Space::Document => point,
            Space::Tip => tip.map(|v| v * 64.0),
        };
        for (n, v) in p.iter_mut().enumerate() {
            if self.flip[n] {
                *v = -*v;
            }
        }
        let Some(b) = library.raw_texture(self.pattern, p) else {
            return a;
        };
        let b = self.adjusted(b);
        let gradient = if self.mode == Mode::Outline {
            let sample = |q| self.adjusted(library.raw_texture(self.pattern, q).unwrap_or(1.0));
            (sample([p[0] + 1.0, p[1]]) - sample([p[0] - 1.0, p[1]]))
                .abs()
                .max((sample([p[0], p[1] + 1.0]) - sample([p[0], p[1] - 1.0])).abs())
                .min(1.0)
        } else {
            0.0
        };
        (a + (self.value(a, b, gradient) - a) * self.pattern.strength).clamp(0.0, 1.0)
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Slot {
    pub flip: [bool; 2],
    pub texture: Option<Settings>,
}
impl Slot {
    pub fn valid(self) -> bool {
        self.texture.is_none_or(|s| s.valid())
    }
    pub fn point(self, mut q: [f64; 2]) -> [f64; 2] {
        for (n, v) in q.iter_mut().enumerate() {
            if self.flip[n] {
                *v = -*v;
            }
        }
        q
    }
}

pub fn references(
    sequence: &crate::tip_sequence::Settings,
    dual: &crate::brush_variation::Controls,
    pattern: crate::texture::Pattern,
) -> Vec<(u32, crate::material_library::Kind)> {
    use crate::material_library::Kind;
    sequence
        .tips
        .iter()
        .map(|id| (*id, Kind::Tip))
        .chain(std::iter::once((pattern.source, Kind::Texture)))
        .chain(
            sequence
                .slots
                .iter()
                .filter_map(|s| s.texture)
                .map(|s| (s.pattern.source, Kind::Texture)),
        )
        .chain(dual.texture.map(|s| (s.pattern.source, Kind::Texture)))
        .collect()
}

fn style() -> crate::painting::Style {
    crate::painting::Style {
        clone_stamp: Default::default(),
        variation: Default::default(),
        pattern: Default::default(),
        dynamics: Default::default(),
        sequence: Default::default(),
        filters: Default::default(),
        profile: Default::default(),
        texture: Default::default(),
        material: Default::default(),
        shape: crate::brush_cursor::Shape {
            size: 8.0,
            aspect: 1.0,
            angle: 0.0,
        },
        tip: crate::assets::BrushTip::Pen,
        color: [180, 100, 50, 255],
        opacity: 1.0,
        hardness: 1.0,
        spacing: 0.15,
        smoothing: 0.0,
        pressure: crate::pressure::Mapping {
            size: false,
            opacity: false,
            ..Default::default()
        },
    }
}
fn document() -> Result<crate::document::Document, String> {
    crate::document::Document::create(
        crate::document::CanvasSpec {
            width: 32,
            height: 16,
            dpi: 300.0,
            background: crate::document::Background::Transparent,
        },
        32768,
    )
    .map_err(|e| e.to_string())
}
fn paint(
    s: crate::painting::Style,
    materials: std::sync::Arc<crate::material_library::Library>,
    points: &[[f64; 2]],
) -> Result<crate::document::Document, String> {
    let mut d = document()?;
    let mut stroke = crate::painting::Stroke::begin(&d, 0, s)?.with_materials(materials)?;
    for p in points {
        stroke.direct_dab(&d, *p, *p, 1.0)?;
    }
    stroke.commit(&mut d)?;
    Ok(d)
}
pub fn verify(root: &std::path::Path) -> Result<crate::material_library::Library, String> {
    use crate::{
        material_library::{Kind, Library},
        painting::Stroke,
    };
    use std::sync::Arc;
    let library = Library::default()
        .register_mask(
            "Asymmetric tip",
            Kind::Tip,
            3,
            3,
            vec![255, 64, 0, 128, 255, 64, 0, 128, 255],
        )
        .map_err(|e| e.to_string())?
        .register_mask(
            "Asymmetric weave",
            Kind::Texture,
            2,
            2,
            vec![0, 64, 128, 255],
        )
        .map_err(|e| e.to_string())?;
    let mask = library.assets()[0].id;
    let texture = library.assets()[1].id;
    let materials = Arc::new(library.clone());
    let mut s = style();
    s.sequence.enabled = true;
    s.sequence.tips[0] = mask;
    let plain = paint(s, materials.clone(), &[[16.5, 8.5]])?;
    for axis in 0..2 {
        let mut flip = s;
        flip.sequence.slots[0].flip[axis] = true;
        let mirrored = paint(flip, materials.clone(), &[[16.5, 8.5]])?;
        if mirrored.layers[0].pixels == plain.layers[0].pixels {
            return Err("Asymmetric tip flip had no effect".into());
        }
        for dy in -4i32..=4 {
            for dx in -4i32..=4 {
                let p = [16 + dx, 8 + dy];
                let q = if axis == 0 {
                    [16 - dx, 8 + dy]
                } else {
                    [16 + dx, 8 - dy]
                };
                if mirrored.layers[0].pixels[(p[1] * 32 + p[0]) as usize]
                    != plain.layers[0].pixels[(q[1] * 32 + q[0]) as usize]
                {
                    return Err("Tip mirror did not match independent pixel reflection".into());
                }
            }
        }
    }
    let a = Settings {
        pattern: crate::texture::Pattern {
            source: texture,
            strength: 1.0,
            scale: 3.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let b = Settings {
        pattern: crate::texture::Pattern {
            source: texture,
            strength: 1.0,
            scale: 1.5,
            angle: 90.0,
            ..Default::default()
        },
        space: Space::Tip,
        ..Default::default()
    };
    let expected = [
        (Mode::Normal, 0.7),
        (Mode::Multiply, 0.28),
        (Mode::Subtract, 0.0),
        (Mode::Compare, 0.7),
        (Mode::Outline, 0.08),
        (Mode::Overlay, 0.56),
        (Mode::ColorDodge, 1.0),
        (Mode::ColorBurn, 1.0 - 0.6 / 0.7),
        (Mode::HardMix, 1.0),
        (Mode::Height, 0.55),
    ];
    for (mode, value) in expected {
        if ((Settings { mode, ..a }).value(0.4, 0.7, 0.2) - value).abs() > 1e-12 {
            return Err(format!("Texture numeric mode {mode:?}"));
        }
    }
    let mut sequences = s;
    s.shape.size = 4.0;
    sequences.shape.size = 4.0;
    sequences.sequence.count = 2;
    sequences.sequence.tips[1] = mask;
    sequences.sequence.slots[0].texture = Some(a);
    sequences.sequence.slots[1].texture = Some(b);
    let combined = paint(sequences, materials.clone(), &[[8.5, 8.5], [24.5, 8.5]])?;
    let mut reference = document()?;
    for (p, texture) in [([8.5, 8.5], a), ([24.5, 8.5], b)] {
        let mut single = s;
        single.sequence.slots[0].texture = Some(texture);
        let mut stroke = Stroke::begin(&reference, 0, single)?.with_materials(materials.clone())?;
        stroke.direct_dab(&reference, p, p, 1.0)?;
        stroke.commit(&mut reference)?;
    }
    if combined.layers[0].pixels != reference.layers[0].pixels
        || combined.history_counts() != (1, 0)
    {
        return Err("Two slots did not keep independent texture transforms / one history".into());
    }
    let mut altered = sequences;
    altered.sequence.slots[0].texture = Some(Settings {
        flip: [true, false],
        brightness: 0.2,
        contrast: 0.4,
        ..a
    });
    if paint(altered, materials.clone(), &[[8.5, 8.5], [24.5, 8.5]])?.layers[0].pixels
        == combined.layers[0].pixels
    {
        return Err("Texture flip/brightness/contrast had no effect".into());
    }
    let mut dual = style();
    dual.color[3] = 0;
    dual.variation.dual = crate::brush_variation::Dual {
        enabled: true,
        blend: crate::brush_variation::Blend::Normal,
        scale: 1.0,
        mask: crate::brush_variation::Mask::Square,
        controls: crate::brush_variation::Controls {
            rgb: true,
            color: [0, 0, 255, 255],
            texture: Some(Settings {
                space: Space::Tip,
                ..a
            }),
            ..Default::default()
        },
        ..Default::default()
    };
    let first = paint(dual, materials.clone(), &[[16.5, 8.5]])?;
    dual.sequence.slots[0].flip = [true, false];
    if paint(dual, materials.clone(), &[[16.5, 8.5]])?.layers[0].pixels != first.layers[0].pixels {
        return Err("Primary tip flip changed the independent secondary tip".into());
    }
    dual.variation.dual.controls.flip = [true, false];
    if paint(dual, materials.clone(), &[[16.5, 8.5]])?.layers[0].pixels == first.layers[0].pixels {
        return Err("Secondary tip flip did not mirror its local texture".into());
    }
    verify_edges(root)?;
    let file = root.join("tip-texture-library.json");
    library.save(&file).map_err(|e| e.to_string())?;
    if Library::read(&file).map_err(|e| e.to_string())? != library {
        return Err("Tip texture materials lost on restart".into());
    }
    Ok(library)
}
fn verify_edges(root: &std::path::Path) -> Result<(), String> {
    use crate::{
        brush_material::Mode as Pigment,
        mouse_pressure::{Replay, Sample},
        painting::Stroke,
    };
    let mut d = document()?;
    let before = d.snapshot();
    let mut s = style();
    s.shape.size = 4.0;
    s.material.mode = Pigment::Water;
    s.material.edge = 1.0;
    s.material.supply = 1.0;
    s.material.mixing = 0.0;
    s.material.retention = 0.0;
    s.material.water = 0.0;
    let mut stroke = Stroke::begin(&d, 0, s)?;
    stroke.direct_dab(&d, [16.5, 8.5], [16.5, 8.5], 1.0)?;
    let before_edge = stroke
        .pixel(&d, 0, &d.layers[0], 16, 6)
        .ok_or("Edge preview unavailable")?;
    stroke.finish_material(&d, || false)?;
    let finished = stroke
        .pixel(&d, 0, &d.layers[0], 16, 6)
        .ok_or("Finished edge unavailable")?;
    if finished[0] >= before_edge[0]
        || finished[3] != before_edge[3]
        || stroke.pixel(&d, 0, &d.layers[0], 16, 8).unwrap()[0] != 180
    {
        return Err("Water edge did not darken only the rim while retaining alpha".into());
    }
    stroke.finish_material(&d, || false)?;
    if stroke.pixel(&d, 0, &d.layers[0], 16, 6) != Some(finished) {
        return Err("Water edge ran twice".into());
    }
    stroke.commit(&mut d)?;
    let after = d.snapshot();
    if d.history_counts() != (1, 0)
        || !d.undo()
        || d.snapshot() != before
        || !d.redo()
        || d.snapshot() != after
    {
        return Err("Water edge was outside the stroke history".into());
    }
    let mut cancelled = Stroke::begin(&d, 0, s)?;
    cancelled.direct_dab(&d, [6.5, 8.5], [6.5, 8.5], 1.0)?;
    if cancelled.finish_material(&d, || true).is_ok() || d.snapshot() != after {
        return Err("Cancelled water edge published pixels".into());
    }
    let mut r = Replay::begin(&d, 0, s, Default::default())?;
    r.push(Sample {
        point: [6.5, 8.5],
        micros: Some(0),
    })?;
    r.render(&d, true)?.commit_mouse(&mut d, r.metadata())?;
    crate::files::save(&mut d, &root.join("tip-texture-edges.drawer"))
        .map_err(|e| e.to_string())?;
    if crate::files::open(&root.join("tip-texture-edges.drawer"), 32768)
        .map_err(|e| e.to_string())?
        .snapshot()
        != d.snapshot()
    {
        return Err("Water edge native pixels/settings changed".into());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn per_tip_modes_flips_secondary_and_water_edges() {
        let root = crate::test_support::TempDir::new();
        super::verify(&root.0).unwrap();
    }
    #[test]
    fn legacy_bounds_and_disabled_slots() {
        let mut value = serde_json::to_value(crate::tip_sequence::Settings::default()).unwrap();
        value.as_object_mut().unwrap().remove("slots");
        assert_eq!(
            serde_json::from_value::<crate::tip_sequence::Settings>(value)
                .unwrap()
                .slots,
            [Default::default(); 8]
        );
        for v in [-1.1, 1.1, f64::NAN] {
            assert!(
                !super::Settings {
                    brightness: v,
                    ..Default::default()
                }
                .valid()
            );
            assert!(
                !super::Settings {
                    contrast: v,
                    ..Default::default()
                }
                .valid()
            );
        }
    }
    #[test]
    fn transforms_each_change_and_portable_dependencies() {
        use crate::{
            assets::BrushTip,
            brush_presets::Preset,
            material_library::{Kind, Library},
            tool_properties::Property,
            user_tools,
            workspaces::Snapshot,
        };
        let root = crate::test_support::TempDir::new();
        let lib = super::verify(&root.0).unwrap();
        let texture = lib.assets()[1].id;
        let base = super::Settings {
            pattern: crate::texture::Pattern {
                source: texture,
                strength: 1.0,
                scale: 3.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let sample = |s: super::Settings| {
            (0..32)
                .map(|n| {
                    s.apply(
                        0.7,
                        [n as f64 + 0.5, 7.5],
                        [n as f64 / 16.0 - 1.0, 0.25],
                        &lib,
                    )
                })
                .collect::<Vec<_>>()
        };
        let baseline = sample(base);
        for changed in [
            super::Settings {
                flip: [true, false],
                ..base
            },
            super::Settings {
                flip: [false, true],
                ..base
            },
            super::Settings {
                brightness: 0.25,
                ..base
            },
            super::Settings {
                contrast: -1.0,
                ..base
            },
            super::Settings {
                space: super::Space::Tip,
                ..base
            },
            super::Settings {
                pattern: crate::texture::Pattern {
                    angle: 90.0,
                    ..base.pattern
                },
                ..base
            },
            super::Settings {
                pattern: crate::texture::Pattern {
                    scale: 1.5,
                    ..base.pattern
                },
                ..base
            },
        ] {
            assert_ne!(sample(changed), baseline);
        }
        let snap = Snapshot {
            properties: Some(Default::default()),
            ..Default::default()
        };
        let mut preset = Preset::capture(&snap, BrushTip::Pen, "Textured tips").unwrap();
        preset.sequence.tips[0] = lib.assets()[0].id;
        preset.sequence.slots[0].texture = Some(base);
        preset.variation.dual.controls.texture = Some(base);
        let mut tools = user_tools::Library::default();
        tools
            .create_tool(1, preset, Property::ALL.to_vec())
            .unwrap();
        let bundle = root.0.join("textured-bundle");
        crate::tool_bundle::export(&bundle, &tools, &lib).unwrap();
        let existing = Library::default()
            .register_mask("Existing", Kind::Texture, 1, 1, vec![100])
            .unwrap();
        let (imported, materials) =
            crate::tool_bundle::import(&bundle, &user_tools::Library::default(), &existing)
                .unwrap();
        let tool = &imported.tools[0];
        for p in [&tool.preset, &tool.default] {
            let a = p.sequence.slots[0].texture.unwrap().pattern.source;
            let b = p.variation.dual.controls.texture.unwrap().pattern.source;
            assert_eq!(a, b);
            assert_ne!(a, texture);
            assert!(materials.get(a, Kind::Texture).is_some());
        }
    }
    #[test]
    fn tiny_negative_rotated_tile_coordinates_stay_in_bounds() {
        use crate::material_library::{Kind, Library};
        let lib = Library::default()
            .register_mask("Boundary", Kind::Texture, 2, 2, vec![0, 64, 128, 255])
            .unwrap();
        let a = &lib.assets()[0];
        assert_eq!(a.tile([-f64::EPSILON / 4.0, 0.0]), 64.0 / 255.0);
        assert_eq!(a.tile([0.0, -f64::EPSILON / 4.0]), 128.0 / 255.0);
        assert_eq!(a.tile([2.0, 2.0]), 0.0);
        let p = crate::texture::Pattern {
            source: 1,
            strength: 1.0,
            ..Default::default()
        };
        assert_eq!(p.factor([-f64::EPSILON / 4.0, 0.0]), p.factor([-1e-9, 0.0]));
    }
    #[test]
    fn full_and_quarter_turns_match_exact_coordinate_rotations() {
        use crate::{
            material_library::{Kind, Library},
            texture::Pattern,
        };
        let lib = Library::default()
            .register_mask("Turn grid", Kind::Texture, 2, 2, vec![0, 64, 128, 255])
            .unwrap();
        for source in [1, lib.assets()[0].id] {
            let p = Pattern {
                source,
                scale: 0.5,
                strength: 1.0,
                ..Default::default()
            };
            for angle in [360.0, -360.0] {
                assert_eq!(
                    lib.factor(Pattern { angle, ..p }, [0.5, 0.5]),
                    lib.factor(p, [0.5, 0.5])
                );
            }
            for (angle, point) in [
                (90.0, [0.5, 0.5]),
                (180.0, [0.5, -0.5]),
                (270.0, [-0.5, -0.5]),
            ] {
                assert_eq!(
                    lib.factor(Pattern { angle, ..p }, [-0.5, 0.5]),
                    lib.factor(p, point)
                );
            }
        }
    }
}
