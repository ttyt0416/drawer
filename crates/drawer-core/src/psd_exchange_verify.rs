use super::*;
use crate::document::DEFAULT_DOCUMENT_BUDGET as B;
fn check(ok: bool, msg: &str) -> Result<(), String> {
    if ok { Ok(()) } else { Err(msg.into()) }
}
pub fn fixture() -> Result<Document, String> {
    let mut d = Document::create(
        CanvasSpec {
            width: 8,
            height: 8,
            dpi: 144.,
            background: Background::Transparent,
        },
        B,
    )
    .map_err(|e| e.to_string())?;
    for (name, color) in [
        ("Base", [30, 80, 150, 255]),
        ("Alpha", [190, 40, 100, 128]),
        ("Hidden", [10, 200, 80, 255]),
    ] {
        let mut l = d.layers[0].clone();
        l.name = name.into();
        l.pixels.fill(color);
        d.layers.push(l);
    }
    d.layers.remove(0);
    d.scene.layer_ids.initialize(3)?;
    d.scene.paper.folders.push(Folder {
        name: "그룹 ✏".into(),
        through: false,
        children: vec![Node::Raster(1)],
        paper: Default::default(),
    });
    d.scene.paper.roots = vec![Node::Raster(0), Node::Folder(0), Node::Raster(2)];
    d.scene.paper.layers.entry(2).or_default().hidden = true;
    d.scene.layer_properties.set(
        Target::Raster(2),
        props::Properties {
            opacity: 65,
            blend: Mode::Multiply,
            ..Default::default()
        },
    );
    d.scene.layer_effects.folders.insert(
        0,
        crate::layer_effects::Effect {
            mask: Some(crate::layer_effects::Mask {
                space: Default::default(),
                coverage: crate::selection::Mask {
                    runs: (0..8)
                        .map(|y| crate::selection::Run {
                            x: 1,
                            y,
                            length: 5,
                            alpha: 128,
                        })
                        .collect(),
                },
                offset: [0.; 2],
                enabled: true,
                linked: true,
                inverted: false,
            }),
            adjustment: None,
        },
    );
    Ok(d)
}
pub fn roundtrips(root: &Path) -> Result<(), String> {
    for through in [false, true] {
        let mut d = fixture()?;
        d.scene.paper.folders[0].through = through;
        let before = d.snapshot();
        let history = d.history_counts();
        let plan = prepare(&d, B, &AtomicBool::new(false))?;
        check(
            plan.partial.is_some() && plan.ranges.is_empty(),
            "Supported structure was flattened",
        )?;
        for f in [Format::Psd, Format::Psb] {
            let path = root.join(format!(
                "layers-{through}.{}",
                if f == Format::Psd { "psd" } else { "psb" }
            ));
            plan.write(&d, &path, f, false, true, false, B, &AtomicBool::new(false))?;
            let (imported, w) = open_report(&path, B)?;
            check(w.is_empty(), &format!("Own layered file fell back: {w:?}"))?;
            check(
                imported.layers.len() == 3
                    && imported.scene.paper.folders.len() == 1
                    && imported.scene.paper.folders[0].through == through
                    && imported.scene.paper.folders[0].name == "그룹 ✏"
                    && crate::layer_masks::get(&imported, Target::Folder(0)).is_some()
                    && (imported.dpi - 144.).abs() < 0.001,
                "Editable structure / DPI lost",
            )?;
            check(
                difference(
                    &render(&d, B, &AtomicBool::new(false))?,
                    &render(&imported, B, &AtomicBool::new(false))?,
                ) <= 1,
                "Layered composite changed",
            )?;
            check(
                d.snapshot() == before && d.history_counts() == history,
                "Export changed source or history",
            )?;
            let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
            check(
                plan.write(&d, &path, f, false, true, false, B, &AtomicBool::new(false))
                    .is_err()
                    && std::fs::read(&path).unwrap() == bytes,
                "Existing export replaced without confirmation",
            )?;
            let mut name = path.file_name().unwrap().to_owned();
            name.push(".psd-exchange.tmp");
            let temp = path.with_file_name(name);
            std::fs::write(&temp, b"foreign").map_err(|e| e.to_string())?;
            check(
                plan.write(&d, &path, f, false, true, true, B, &AtomicBool::new(false))
                    .is_err()
                    && std::fs::read(&temp).unwrap() == b"foreign"
                    && std::fs::read(&path).unwrap() == bytes,
                "Foreign temp / old file changed",
            )?;
            std::fs::remove_file(temp).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}
pub fn dependencies(root: &Path) -> Result<(), String> {
    let mut d = fixture()?;
    d.scene.paper.folders[0].through = true;
    d.scene.layer_effects.folders.clear();
    d.scene.layer_effects.rasters.insert(
        2,
        crate::layer_effects::Effect {
            adjustment: Some(crate::layer_effects::Adjustment {
                brightness: 0.2,
                ..Default::default()
            }),
            mask: None,
        },
    );
    let original = d.snapshot();
    let p = prepare(&d, B, &AtomicBool::new(false))?;
    check(
        p.partial.is_some(),
        &format!(
            "Pass-through correction did not find dependency range: {:?}",
            p.reason
        ),
    )?;
    check(
        p.ranges
            .iter()
            .any(|r| r.iter().any(|s| s == "Base") && r.iter().any(|s| s == "Alpha")),
        "Pass-through correction omitted its backdrop range",
    )?;
    check(
        p.ranges.iter().all(|r| !r.iter().any(|s| s == "Hidden")),
        "Unrelated hidden sibling was rasterized",
    )?;
    p.write(
        &d,
        &root.join("through-correction.psd"),
        Format::Psd,
        false,
        true,
        false,
        B,
        &AtomicBool::new(false),
    )?;
    check(d.snapshot() == original, "Dependency export changed source")?;
    let mut mixed = crate::raster_conversion_verify::mixed()?;
    for o in &mut mixed.scene.objects {
        if let crate::scene::PageObject::Text { layout, text, .. } = o {
            *text = "한".into();
            *layout = Some(Box::new(crate::text_layout::Style {
                ruby: vec![crate::text_layout::Ruby {
                    start: 0,
                    end: 1,
                    text: "X".into(),
                    font: crate::text_layout::Font::Ubuntu,
                    ..Default::default()
                }],
                ..Default::default()
            }));
        }
    }
    let before = mixed.snapshot();
    let plan = prepare(&mixed, B, &AtomicBool::new(false))?;
    check(
        plan.partial.is_some(),
        &format!(
            "Text/ruby/vector/3D mixed document did not preserve partial composition: {:?}",
            plan.reason
        ),
    )?;
    let path = root.join("mixed-text-ruby-3d.psb");
    plan.write(
        &mixed,
        &path,
        Format::Psb,
        false,
        true,
        false,
        B,
        &AtomicBool::new(false),
    )?;
    let (imported, w) = open_report(&path, B)?;
    check(
        w.is_empty()
            && imported.layers.len() > 1
            && imported.scene.objects.is_empty()
            && imported.scene.worlds.is_empty()
            && plan.losses.iter().any(|l| l.contains("ruby"))
            && plan.ranges.len() >= 3,
        "Mixed source conversion/loss list mismatch",
    )?;
    check(
        difference(
            &render(&mixed, B, &AtomicBool::new(false))?,
            &render(&imported, B, &AtomicBool::new(false))?,
        ) <= 1
            && mixed.snapshot() == before,
        "Mixed roundtrip changed appearance or source",
    )?;
    let mut shadow = crate::surface_effects_verify::fixture()?;
    let mut top = shadow.layers[0].clone();
    top.name = "Clipped shadow".into();
    shadow.layers[0].name = "Clip base".into();
    top.pixels.fill([60, 150, 220, 128]);
    shadow.layers.push(top);
    shadow.scene.layer_ids.initialize(2)?;
    shadow.scene.clipping.rasters.insert(2, Target::Raster(1));
    shadow.scene.surface_effects.rasters.insert(
        2,
        crate::surface_effects::Stack {
            effects: vec![crate::surface_effects::Entry {
                operation: crate::surface_effects::Operation::Shadow {
                    blur: 2,
                    direction: 45.,
                    distance: 2.,
                    color: [0; 3],
                    opacity: 0.5,
                },
                enabled: true,
                scope: None,
            }],
            ..Default::default()
        },
    );
    let p = prepare(&shadow, B, &AtomicBool::new(false))?;
    check(
        p.partial.is_some(),
        &format!(
            "Clipped shadow dependency conversion failed: {:?}",
            p.reason
        ),
    )?;
    check(
        p.ranges
            .iter()
            .any(|r| r.iter().any(|s| s == "Clip base") && r.iter().any(|s| s == "Clipped shadow")),
        "Clipped shadow range lost its base",
    )?;
    p.write(
        &shadow,
        &root.join("clipped-shadow.psd"),
        Format::Psd,
        false,
        true,
        false,
        B,
        &AtomicBool::new(false),
    )?;
    Ok(())
}
pub fn files(root: &Path) -> Result<(), String> {
    // Independent tiny RLE files: no layer records and no code from our writer.
    for version in [1u16, 2] {
        let mut raw = b"8BPS".to_vec();
        raw.extend(version.to_be_bytes());
        raw.extend([0; 6]);
        raw.extend(3u16.to_be_bytes());
        raw.extend(1u32.to_be_bytes());
        raw.extend(2u32.to_be_bytes());
        raw.extend(8u16.to_be_bytes());
        raw.extend(3u16.to_be_bytes());
        raw.extend(0u32.to_be_bytes());
        raw.extend(0u32.to_be_bytes());
        raw.extend(vec![0; if version == 1 { 4 } else { 8 }]);
        raw.extend(1u16.to_be_bytes());
        for _ in 0..3 {
            if version == 1 {
                raw.extend(3u16.to_be_bytes());
            } else {
                raw.extend(3u32.to_be_bytes());
            }
        }
        raw.extend([1, 10, 20, 1, 30, 40, 1, 50, 60]);
        let (image, _, _) = io::decode(&raw, B, &AtomicBool::new(false))?;
        check(
            image.merged == vec![[10, 30, 50, 255], [20, 40, 60, 255]],
            "Independent PSD/PSB PackBits image decoded incorrectly",
        )?;
    }
    let mut auxiliary = b"8BPS".to_vec();
    auxiliary.extend(1u16.to_be_bytes());
    auxiliary.extend([0; 6]);
    auxiliary.extend(4u16.to_be_bytes());
    auxiliary.extend(1u32.to_be_bytes());
    auxiliary.extend(2u32.to_be_bytes());
    auxiliary.extend(8u16.to_be_bytes());
    auxiliary.extend(3u16.to_be_bytes());
    auxiliary.extend([0; 12]);
    auxiliary.extend(0u16.to_be_bytes());
    auxiliary.extend([10, 20, 30, 40, 50, 60, 0, 64]);
    let (auxiliary, warnings, _) = io::decode(&auxiliary, B, &AtomicBool::new(false))?;
    check(
        auxiliary.merged == vec![[10, 30, 50, 255], [20, 40, 60, 255]]
            && warnings.iter().any(|s| s.contains("auxiliary alpha")),
        "Auxiliary alpha was treated as transparency or omitted without a loss notice",
    )?;
    let mut d = fixture()?;
    let p = prepare(&d, B, &AtomicBool::new(false))?;
    let image = p.partial.as_ref().unwrap();
    let bytes = io::encode(image, Format::Psd, B, &AtomicBool::new(false))?;
    let name_position = |name: &str| {
        let utf16 = name
            .encode_utf16()
            .flat_map(u16::to_be_bytes)
            .collect::<Vec<_>>();
        bytes.windows(utf16.len()).position(|w| w == utf16).unwrap()
    };
    check(
        name_position("Base") < name_position("</Group>")
            && name_position("</Group>") < name_position("Alpha")
            && name_position("Alpha") < name_position("그룹 ✏")
            && name_position("그룹 ✏") < name_position("Hidden"),
        "PSD wire stack must run bottom to top, with bounding divider before group children",
    )?;
    for n in (0..bytes.len()).step_by(19) {
        check(
            io::decode(&bytes[..n], B, &AtomicBool::new(false)).is_err(),
            "Truncated PSD accepted",
        )?;
    }
    for (index, value) in [(4, 99), (13, 2), (23, 16), (25, 4)] {
        let mut bad = bytes.clone();
        bad[index] = value;
        check(
            io::decode(&bad, B, &AtomicBool::new(false)).is_err(),
            "Invalid PSD header variant accepted",
        )?;
    }
    for (data, want) in [
        (vec![2, 10, 20, 30], vec![10, 20, 30]),
        (vec![254, 9], vec![9, 9, 9]),
        (vec![128, 2, 1, 2, 3], vec![1, 2, 3]),
    ] {
        check(
            io::unpack(&data, 3)? == want,
            "Independent PackBits values differ",
        )?;
    }
    for bad in [vec![3, 1, 2], vec![253, 1], vec![0], vec![255, 1]] {
        check(
            io::unpack(&bad, 3).is_err(),
            "Damaged PackBits row accepted",
        )?;
    }
    let path = root.join("cancel.psd");
    check(
        p.write(
            &d,
            &path,
            Format::Psd,
            false,
            false,
            false,
            B,
            &AtomicBool::new(false),
        )
        .is_err()
            && !path.exists(),
        "Unconfirmed export created a file",
    )?;
    check(
        p.write(
            &d,
            &path,
            Format::Psd,
            false,
            true,
            false,
            B,
            &AtomicBool::new(true),
        )
        .is_err()
            && !path.exists(),
        "Canceled export created a file",
    )?;
    d.revision += 1;
    check(
        p.write(
            &d,
            &path,
            Format::Psd,
            false,
            true,
            false,
            B,
            &AtomicBool::new(false),
        )
        .is_err()
            && !path.exists(),
        "Stale preview exported",
    )?;
    d.revision -= 1;
    d.source_path = Some(path.clone());
    check(
        p.write(
            &d,
            &path,
            Format::Psd,
            false,
            true,
            true,
            B,
            &AtomicBool::new(false),
        )
        .is_err(),
        "Original source overwrite allowed",
    )?;
    d.source_path = None;
    check(
        prepare(&d, 0, &AtomicBool::new(false)).is_err()
            && prepare(&d, B, &AtomicBool::new(true)).is_err(),
        "Preparation ignored budget/cancel",
    )?;
    // Unknown required editing blocks must use the actual merged image, never an invented empty layer.
    let mut altered = bytes.clone();
    let at = altered.windows(4).position(|w| w == b"luni").unwrap();
    altered[at..at + 4].copy_from_slice(b"vmsk");
    let path = root.join("unsupported-vector-mask.psd");
    std::fs::write(&path, altered).map_err(|e| e.to_string())?;
    let (imported, w) = open_report(&path, B)?;
    check(
        imported.layers.len() == 1
            && w.iter().any(|s| s.contains("vmsk"))
            && w.iter().any(|s| s.contains("Whole merged")),
        "Unsupported source was silently dropped / invented",
    )?;
    Ok(())
}
pub fn verify(root: &Path) -> Result<(), String> {
    let dir = root.join("psd-exchange-core");
    std::fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let result = roundtrips(&dir)
        .and_then(|_| dependencies(&dir))
        .and_then(|_| files(&dir))
        .and_then(|_| report::verify(&dir));
    result.and(std::fs::remove_dir_all(dir).map_err(|e| e.to_string()))
}
#[cfg(test)]
mod tests {
    #[test]
    fn groups_masks_blends_hidden_layers_dpi_and_psd_psb_roundtrip() {
        let t = crate::test_support::TempDir::new();
        super::roundtrips(&t.0).unwrap();
    }
    #[test]
    fn minimum_pass_through_correction_clipped_shadow_text_ruby_and_3d_ranges() {
        let t = crate::test_support::TempDir::new();
        super::dependencies(&t.0).unwrap();
    }
    #[test]
    fn damaged_lengths_packbits_color_modes_cancel_confirm_stale_and_original_protection() {
        let t = crate::test_support::TempDir::new();
        super::files(&t.0).unwrap();
    }
}
