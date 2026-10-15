//! Factory brushes are owned numeric definitions; persisted legacy tip IDs stay stable.
use crate::{
    assets::BrushTip,
    tool_properties::{Property, Settings, Tool},
    workspaces::Snapshot,
};
/// Size px, aspect, angle degrees, stabilization, stroke opacity, hardness, diameter spacing.
pub fn values(tip: BrushTip) -> [f64; 7] {
    match tip {
        BrushTip::PixelPen => [1.0, 1.0, 0.0, 0.0, 1.0, 1.0, 0.25],
        BrushTip::Marker => [12.0, 1.0, 0.0, 0.0, 0.65, 1.0, 0.3],
        BrushTip::Smudge => [12.0, 1.0, 0.0, 0.0, 1.0, 0.5, 0.15],
        BrushTip::Wash => [12.0, 1.0, 0.0, 0.0, 0.6, 0.0, 0.1],
        _ => [12.0, 1.0, 0.0, 0.0, 1.0, 1.0, 0.15],
    }
}
pub fn value(tool: Tool, property: Property) -> f64 {
    let tip = match tool {
        Tool::Pen => BrushTip::Pen,
        Tool::Pencil => BrushTip::Pencil,
        Tool::Eraser => BrushTip::Eraser,
        Tool::Wash => BrushTip::Wash,
        Tool::Marker => BrushTip::Marker,
        Tool::Smudge => BrushTip::Smudge,
        Tool::PixelPen => BrushTip::PixelPen,
        _ => BrushTip::Pen,
    };
    values(tip)[Property::ALL.iter().position(|p| *p == property).unwrap()]
}
pub fn restore_values(settings: &mut Settings, tip: BrushTip) -> Result<(), String> {
    if !settings.valid() || settings.version != 2 {
        return Err("Upgrade and validate properties before restoring a factory brush".into());
    }
    let props = settings.tools.get_mut(&Tool::brush(tip)).unwrap();
    for (property, value) in Property::ALL.into_iter().zip(values(tip)) {
        props.get_mut(&property).unwrap().value = value;
    }
    Ok(())
}
pub fn material(tip: BrushTip) -> crate::brush_material::Settings {
    use crate::brush_material::{Mode, Settings};
    if tip == BrushTip::Smudge {
        Settings {
            mode: Mode::Smudge,
            colorless: true,
            supply: 0.0,
            water: 0.0,
            ..Default::default()
        }
    } else {
        Default::default()
    }
}
/// Reset only the chosen brush's input dynamics, keeping device/UI/other brushes intact.
pub fn restore_input(snapshot: &mut Snapshot, tip: BrushTip) {
    let n = BrushTip::ALL.iter().position(|t| *t == tip).unwrap();
    snapshot.input.smoothing[n] = 0.0;
    snapshot.input.ends[n] = Default::default();
    snapshot.input.materials[n] = material(tip);
    snapshot.input.textures[n] = Default::default();
    snapshot.input.profiles[n] = Default::default();
    snapshot.input.filters[n] = Default::default();
    snapshot.input.sequences[n] = Default::default();
    snapshot.input.dynamics[n] = Default::default();
    snapshot.input.mouse_pressure[n] = Default::default();
    snapshot.pressure.tools[n] = Default::default();
}

/// Native smoke and unit tests use the same owned factory definitions and pixel engine.
pub fn verify_pixels() -> Result<(), String> {
    use crate::{
        brush_cursor::Shape,
        document::{Background, CanvasSpec, Document},
        painting::{Stroke, Style},
    };
    let doc = || {
        Document::create(
            CanvasSpec {
                width: 48,
                height: 32,
                dpi: 300.0,
                background: Background::Transparent,
            },
            32768,
        )
        .map_err(|e| e.to_string())
    };
    let mut painted = Vec::new();
    for tip in BrushTip::ALL {
        let mut d = doc()?;
        if tip == BrushTip::Eraser {
            d.layers[0].pixels.fill([80, 90, 100, 255]);
        }
        if tip == BrushTip::Smudge {
            for y in 0..32 {
                for x in 0..16 {
                    d.layers[0].pixels[y * 48 + x] = [210, 70, 20, 180];
                }
            }
        }
        let before = d.snapshot();
        let v = values(tip);
        let style = Style {
            dynamics: Default::default(),
            sequence: Default::default(),
            filters: Default::default(),
            profile: Default::default(),
            texture: Default::default(),
            material: Default::default(),
            shape: Shape {
                size: v[0],
                aspect: v[1],
                angle: v[2],
            },
            smoothing: v[3],
            opacity: v[4],
            hardness: v[5],
            spacing: v[6],
            tip,
            color: [40, 80, 120, 255],
            pressure: Default::default(),
        };
        let mut stroke = Stroke::begin(&d, 0, style)?;
        for x in [10.5, 30.5] {
            stroke.sample(&d, [x, 16.5])?;
        }
        if !stroke.commit(&mut d)? {
            return Err("Factory brush produced no change".into());
        }
        let once = d.layers[0].pixels.clone();
        let mut stroke = Stroke::begin(&d, 0, style)?;
        for x in [10.5, 30.5] {
            stroke.sample(&d, [x, 16.5])?;
        }
        let twice = stroke.commit(&mut d)?;
        if tip == BrushTip::Wash && d.composite_pixel(20, 16)[3] <= once[16 * 48 + 20][3] {
            return Err("Airbrush did not build up across strokes".into());
        }
        if tip == BrushTip::Eraser && (once[16 * 48 + 20][3] != 0 || once[0] != [80, 90, 100, 255])
        {
            return Err("Factory eraser damaged pixels outside its tip".into());
        }
        if (twice && !d.undo()) || !d.undo() || d.snapshot() != before {
            return Err("Factory brush undo failed".into());
        }
        if !d.redo() || d.layers[0].pixels != once {
            return Err("Factory brush redo changed its pixels".into());
        }
        painted.push(once);
    }
    for i in 0..painted.len() {
        for j in i + 1..painted.len() {
            if painted[i] == painted[j] {
                return Err("Factory brush textures are indistinguishable".into());
            }
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn factory_brush_pixels_texture_accumulation_and_history_are_distinct() {
        verify_pixels().unwrap();
    }
    #[test]
    fn reset_preserves_presentation_other_brushes_and_legacy_ids() {
        let mut s = Settings::default();
        let original = s.clone();
        let props = s.tools.get_mut(&Tool::Wash).unwrap();
        props.get_mut(&Property::Size).unwrap().value = 90.0;
        props.get_mut(&Property::Size).unwrap().visible = false;
        restore_values(&mut s, BrushTip::Wash).unwrap();
        assert_eq!(s.tools[&Tool::Wash][&Property::Size].value, 12.0);
        assert!(!s.tools[&Tool::Wash][&Property::Size].visible);
        assert_eq!(s.tools[&Tool::Pen], original.tools[&Tool::Pen]);
        let mut snapshot = Snapshot::default();
        snapshot.input.mouse_pressure[3].enabled = true;
        snapshot.input.mouse_pressure[0].enabled = true;
        snapshot.pressure.tools[3].opacity = true;
        restore_input(&mut snapshot, BrushTip::Wash);
        assert!(!snapshot.input.mouse_pressure[3].enabled);
        assert!(snapshot.input.mouse_pressure[0].enabled);
        assert!(!snapshot.pressure.tools[3].opacity);
        assert_eq!(BrushTip::Wash.name(), "Wash");
        assert_eq!(BrushTip::Wash.label(), "Airbrush");
        assert_eq!(serde_json::to_string(&BrushTip::Wash).unwrap(), "\"Wash\"");
    }
}
