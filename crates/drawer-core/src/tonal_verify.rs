use crate::{
    assets::BrushTip,
    brush_presets::Preset,
    document::{Background, CanvasSpec, DEFAULT_DOCUMENT_BUDGET, Document},
    painting::{Stroke, Style},
    tonal::{Band, Mode, Settings},
    workspaces::Snapshot,
};
fn style(s: Settings) -> Style {
    Style {
        clone_stamp: Default::default(),
        variation: Default::default(),
        pattern: Default::default(),
        dynamics: Default::default(),
        sequence: Default::default(),
        filters: Default::default(),
        profile: Default::default(),
        texture: Default::default(),
        material: crate::brush_material::Settings {
            tonal: s,
            ..Default::default()
        },
        shape: crate::brush_cursor::Shape {
            size: 1.0,
            aspect: 1.0,
            angle: 0.0,
        },
        tip: BrushTip::Pen,
        color: [0; 4],
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
pub fn verify(root: &std::path::Path) -> Result<(), String> {
    for mode in [Mode::Dodge, Mode::Burn] {
        for band in Band::ALL {
            let mut d = Document::create(
                CanvasSpec {
                    width: 256,
                    height: 3,
                    dpi: 300.0,
                    background: Background::Transparent,
                },
                DEFAULT_DOCUMENT_BUDGET,
            )
            .map_err(|e| e.to_string())?;
            for (y, alpha) in [0, 128, 255].into_iter().enumerate() {
                for x in 0..256 {
                    d.layers[0].pixels[y * 256 + x] = [x as u8, x as u8, x as u8, alpha];
                }
            }
            let before = d.snapshot();
            let s = Settings {
                mode,
                band,
                strength: 0.5,
            };
            let mut stroke = Stroke::begin(&d, 0, style(s))?;
            for _ in 0..2 {
                for y in 0..3 {
                    for x in 0..256 {
                        let p = [x as f64 + 0.5, y as f64 + 0.5];
                        stroke.direct_dab(&d, p, p, 1.0)?;
                    }
                }
            }
            stroke.commit(&mut d)?;
            for (y, alpha) in [0, 128, 255].into_iter().enumerate() {
                for x in 0..256 {
                    let b = if x < 85 {
                        Band::Shadows
                    } else if x < 170 {
                        Band::Midtones
                    } else {
                        Band::Highlights
                    };
                    let expected = if b == band && alpha > 0 {
                        match mode {
                            Mode::Dodge => ((x as f64 + 255.0) / 2.0).round() as u8,
                            Mode::Burn => (x as f64 / 2.0).round() as u8,
                            _ => unreachable!(),
                        }
                    } else {
                        x as u8
                    };
                    if d.layers[0].pixels[y * 256 + x] != [expected, expected, expected, alpha] {
                        return Err(format!(
                            "Tone band/alpha/repeated dab failed at {mode:?}/{band:?}/{x}/{alpha}"
                        ));
                    }
                }
            }
            let after = d.snapshot();
            if d.history_counts() != (1, 0)
                || !d.undo()
                || d.snapshot() != before
                || !d.redo()
                || d.snapshot() != after
            {
                return Err("Tone stroke undo/redo mismatch".into());
            }
            let path = root.join(format!("tone-{mode:?}-{band:?}.drawer"));
            crate::files::save(&mut d, &path).map_err(|e| e.to_string())?;
            if crate::files::open(&path, DEFAULT_DOCUMENT_BUDGET)
                .map_err(|e| e.to_string())?
                .snapshot()
                != after
            {
                return Err("Tone document reopen mismatch".into());
            }
        }
    }
    verify_locks(root)
}
pub fn verify_locks(root: &std::path::Path) -> Result<(), String> {
    use crate::{
        tool_locks as locks,
        tool_properties::{Property, Tool},
    };
    let mut s = Snapshot {
        properties: Some(Default::default()),
        ..Default::default()
    };
    let baseline = Preset::capture(&s, BrushTip::Pen, "Baseline")?;
    s = locks::set(&s, BrushTip::Pen, true)?;
    s.input.materials[0].tonal = Settings {
        mode: Mode::Burn,
        band: Band::Highlights,
        strength: 0.77,
    };
    s.panels.brush_size = 31.0;
    s.properties
        .as_mut()
        .unwrap()
        .tools
        .get_mut(&Tool::Pen)
        .unwrap()
        .get_mut(&Property::Size)
        .unwrap()
        .value = 31.0;
    let live = s.clone();
    let disk = locks::for_storage(&s)?;
    if s != live
        || disk.panels.brush_size != baseline.values[0]
        || disk.input.materials[0] != baseline.material
    {
        return Err("Locked baseline replaced runtime values or persisted temporary values".into());
    }
    if locks::restore_current(&s, BrushTip::Pen)?.input.materials[0] != baseline.material {
        return Err("Locked builtin restoration failed".into());
    }
    let path = root.join("locked-workspace.json");
    s.export(&path).map_err(|e| e.to_string())?;
    let restored = Snapshot::read(&path).map_err(|e| e.to_string())?;
    if restored.input.materials[0] != baseline.material {
        return Err("Export persisted locked temporary values".into());
    }
    let mut original = baseline.clone();
    original.name = "Protected".into();
    original.locked = true;
    s.brushes.put(original.clone(), false)?;
    let mut changed = original.clone();
    changed.values[0] = 99.0;
    let original_book = s.brushes.clone();
    if s.brushes.put(changed.clone(), true).is_ok() || s.brushes != original_book {
        return Err("Locked stored preset was overwritten".into());
    }
    changed.name = "Separate copy".into();
    changed.locked = false;
    s.brushes.put(changed, false)?;
    let file = root.join("protected-brush.json");
    original.save(&file, false).map_err(|e| e.to_string())?;
    let bytes = std::fs::read(&file).map_err(|e| e.to_string())?;
    if original.save(&file, true).is_ok()
        || std::fs::read(&file).map_err(|e| e.to_string())? != bytes
    {
        return Err("Locked file overwritten".into());
    }
    s = locks::set(&s, BrushTip::Pen, false)?;
    let id = s
        .user_tools
        .create_tool(1, baseline.clone(), Property::ALL.to_vec())?;
    s = s.user_tools.activate(&s, id)?;
    s = locks::set(&s, BrushTip::Pen, true)?;
    let original = s.user_tools.entry(id).unwrap().preset.clone();
    s.input.materials[0].tonal.mode = Mode::Dodge;
    let p = Preset::capture(&s, BrushTip::Pen, &original.name)?;
    if s.user_tools.update_active(p).is_ok() || s.user_tools.entry(id).unwrap().preset != original {
        return Err("Locked user tool auto-saved temporary values".into());
    }
    if s.user_tools.reset_tool(id).is_ok() {
        return Err("Locked user tool reset bypassed lock".into());
    }
    let disk = locks::for_storage(&s)?;
    if disk.input.materials[0] != original.material {
        return Err("User tool temporary values persisted".into());
    }
    let mut library = s.user_tools.clone();
    let copied = library.duplicate(&[id], 1)?;
    if copied.len() != 1 || library.entry(id).unwrap().preset != original {
        return Err("Separate clone altered locked original".into());
    }
    s = locks::set(&s, BrushTip::Pen, false)?;
    let p = Preset::capture(&s, BrushTip::Pen, &original.name)?;
    s.user_tools.update_active(p)?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tonal_bands_alpha_undo_native_and_lock_storage() {
        let root = crate::test_support::TempDir::new();
        verify(&root.0).unwrap();
    }
    #[test]
    fn numeric_band_edges_strength_and_transparent_pixels() {
        for mode in [Mode::Dodge, Mode::Burn] {
            for band in Band::ALL {
                let s = Settings {
                    mode,
                    band,
                    strength: 0.5,
                };
                for n in [0u8, 84, 85, 169, 170, 255] {
                    let input = [n, n, n, 128];
                    let b = if n < 85 {
                        Band::Shadows
                    } else if n < 170 {
                        Band::Midtones
                    } else {
                        Band::Highlights
                    };
                    let v = if b == band {
                        match (mode, n) {
                            (Mode::Dodge, 0) => 128,
                            (Mode::Dodge, 84) => 170,
                            (Mode::Dodge, 85) => 170,
                            (Mode::Dodge, 169) => 212,
                            (Mode::Dodge, 170) => 213,
                            (Mode::Dodge, 255) => 255,
                            (Mode::Burn, 0) => 0,
                            (Mode::Burn, 84) => 42,
                            (Mode::Burn, 85) => 43,
                            (Mode::Burn, 169) => 85,
                            (Mode::Burn, 170) => 85,
                            (Mode::Burn, 255) => 128,
                            _ => unreachable!(),
                        }
                    } else {
                        n
                    };
                    assert_eq!(s.apply(input, 1.0), [v, v, v, 128]);
                    assert_eq!(s.apply([n, n, n, 0], 1.0), [n, n, n, 0]);
                    assert_eq!(Settings { strength: 0.0, ..s }.apply(input, 1.0), input);
                }
            }
        }
    }
    #[test]
    fn bad_settings_and_legacy_defaults() {
        assert!(
            !Settings {
                strength: f64::NAN,
                ..Default::default()
            }
            .valid()
        );
        assert!(
            !Settings {
                strength: 1.01,
                ..Default::default()
            }
            .valid()
        );
        let mut value = serde_json::to_value(crate::brush_material::Settings::default()).unwrap();
        value.as_object_mut().unwrap().remove("tonal");
        assert_eq!(
            serde_json::from_value::<crate::brush_material::Settings>(value)
                .unwrap()
                .tonal,
            Settings::default()
        );
        let s = Snapshot::default();
        let mut value = serde_json::to_value(s).unwrap();
        value.as_object_mut().unwrap().remove("locks");
        assert!(
            serde_json::from_value::<Snapshot>(value)
                .unwrap()
                .locks
                .empty()
        );
    }
    #[test]
    fn user_lock_cannot_be_bypassed_by_capabilities_or_reset() {
        let mut s = Snapshot {
            properties: Some(Default::default()),
            ..Default::default()
        };
        let p = Preset::capture(&s, BrushTip::Pen, "Protected").unwrap();
        let id = s
            .user_tools
            .create_tool(1, p, crate::tool_properties::Property::ALL.to_vec())
            .unwrap();
        s.user_tools.lock_tool(id, true, None).unwrap();
        let before = s.user_tools.clone();
        assert!(s.user_tools.capabilities(id, vec![]).is_err());
        assert!(s.user_tools.reset_tool(id).is_err());
        assert_eq!(s.user_tools, before);
    }
    #[test]
    fn selection_opacity_and_other_tool_precedence() {
        let mut d = Document::create(
            CanvasSpec {
                width: 1,
                height: 1,
                dpi: 300.0,
                background: Background::Color([128; 3]),
            },
            DEFAULT_DOCUMENT_BUDGET,
        )
        .unwrap();
        d.layers[0].pixels[0][3] = 128;
        d.scene.selection_mask = Some(crate::selection::Mask {
            runs: vec![crate::selection::Run {
                x: 0,
                y: 0,
                length: 1,
                alpha: 128,
            }],
        });
        let mut brush = style(Settings {
            mode: Mode::Dodge,
            band: Band::Midtones,
            strength: 0.5,
        });
        brush.opacity = 0.5;
        let mut stroke = Stroke::begin(&d, 0, brush).unwrap();
        stroke.sample(&d, [0.5, 0.5]).unwrap();
        stroke.commit(&mut d).unwrap();
        assert_eq!(d.layers[0].pixels[0], [144, 144, 144, 128]);
        assert!(d.undo());
        d.scene.selection_mask = None;
        brush.opacity = 1.0;
        brush.tip = BrushTip::Eraser;
        let mut stroke = Stroke::begin(&d, 0, brush).unwrap();
        stroke.sample(&d, [0.5, 0.5]).unwrap();
        stroke.commit(&mut d).unwrap();
        assert_eq!(d.layers[0].pixels[0], [128, 128, 128, 0]);
        assert!(d.undo());
        brush.tip = BrushTip::PixelPen;
        brush.profile.pixel.overwrite = true;
        brush.color = [9, 8, 7, 255];
        let mut stroke = Stroke::begin(&d, 0, brush).unwrap();
        stroke.sample(&d, [0.5, 0.5]).unwrap();
        stroke.commit(&mut d).unwrap();
        assert_eq!(d.layers[0].pixels[0], [9, 8, 7, 255]);
    }
    #[test]
    fn locked_named_restore_unlock_and_inactive_baseline_storage() {
        let mut s = Snapshot {
            properties: Some(Default::default()),
            ..Default::default()
        };
        let mut p = Preset::capture(&s, BrushTip::Pen, "Named lock").unwrap();
        p.values[0] = 19.0;
        p.locked = true;
        s.brushes.put(p.clone(), false).unwrap();
        s = crate::tool_locks::apply_preset(&s, &p).unwrap();
        assert_eq!(s.panels.brush_size, 19.0);
        s.input.materials[0].tonal.mode = Mode::Burn;
        let mut other = Preset::capture(&s, BrushTip::Pencil, "Pencil").unwrap();
        other.values[0] = 7.0;
        s = crate::tool_locks::apply_preset(&s, &other).unwrap();
        assert_eq!(s.input.materials[0], p.material);
        let disk = crate::tool_locks::for_storage(&s).unwrap();
        assert_eq!(disk.tool, BrushTip::Pencil.name());
        assert_eq!(disk.panels.brush_size, 7.0);
        s = crate::tool_locks::apply_preset(&s, &p).unwrap();
        s = crate::tool_locks::set(&s, BrushTip::Pen, false).unwrap();
        assert!(!s.brushes.presets[0].locked);
        let mut changed = p;
        changed.values[0] = 25.0;
        changed.locked = false;
        s.brushes.put(changed, true).unwrap();
        let mut value = serde_json::to_value(&s.brushes.presets[0]).unwrap();
        value.as_object_mut().unwrap().remove("locked");
        assert!(!serde_json::from_value::<Preset>(value).unwrap().locked);
    }
}
