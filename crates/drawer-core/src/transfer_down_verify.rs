use crate::{
    document::{Background, CanvasSpec, DEFAULT_DOCUMENT_BUDGET, Document},
    layer_properties::{self as p, Target},
    selection::{Mask, Run},
    transfer_down as core,
};
fn check(ok: bool, message: &str) -> Result<(), String> {
    if ok { Ok(()) } else { Err(message.into()) }
}
pub fn fixture() -> Result<Document, String> {
    let mut d = Document::create(
        CanvasSpec {
            width: 3,
            height: 1,
            dpi: 300.,
            background: Background::Transparent,
        },
        DEFAULT_DOCUMENT_BUDGET,
    )
    .map_err(|e| e.to_string())?;
    d.layers[0].name = "Lower".into();
    let mut upper = d.layers[0].clone();
    upper.name = "Keep upper".into();
    upper.pixels = vec![[200, 50, 100, 128]; 3];
    d.layers.push(upper);
    d.scene.selection_mask = Some(Mask {
        runs: vec![
            Run {
                x: 1,
                y: 0,
                length: 1,
                alpha: 128,
            },
            Run {
                x: 2,
                y: 0,
                length: 1,
                alpha: 255,
            },
        ],
    });
    Ok(d)
}
pub fn verify(root: &std::path::Path) -> Result<(), String> {
    let mut d = fixture()?;
    let original = d.snapshot();
    let counts = d.history_counts();
    let result = core::prepare(&d, 2)?;
    check(
        d.snapshot() == original
            && result.lower == Some(Target::Raster(1))
            && result.changed == 2
            && result.maximum_delta <= 1,
        "Transfer preflight/target/count",
    )?;
    d.commit_state(result.state)?;
    check(
        d.layers.len() == 2
            && d.layers[1].name == "Keep upper"
            && d.layers[0].name == "Lower"
            && d.scene.layer_ids.id(1) == 2,
        "Transfer deleted or renamed a layer",
    )?;
    check(
        d.layers[1].pixels[0] == original.layers[1].pixels[0]
            && d.layers[1].pixels[1] == [200, 50, 100, 64]
            && d.layers[1].pixels[2] == [0; 4],
        "Selection source cut/outer preservation",
    )?;
    check(
        d.layers[0].pixel(3, 1, 0, 0) == [0; 4]
            && d.layers[0].pixel(3, 1, 1, 0) == [200, 50, 100, 85]
            && d.layers[0].pixel(3, 1, 2, 0) == [200, 50, 100, 128],
        "Independent soft-edge inverse composition values",
    )?;
    check(
        (0..3).all(|x| d.composite_pixel(x, 0) == [200, 50, 100, 128])
            && d.history_counts().0 == counts.0 + 1,
        "Transfer appearance/one Undo",
    )?;
    let after = d.snapshot();
    check(
        d.undo() && d.snapshot() == original && d.redo() && d.snapshot() == after,
        "Two-layer Undo/Redo",
    )?;
    let path = root.join("transfer-down.drawer");
    crate::files::save(&mut d, &path).map_err(|e| e.to_string())?;
    check(
        crate::files::open(&path, DEFAULT_DOCUMENT_BUDGET)
            .map_err(|e| e.to_string())?
            .snapshot()
            == after,
        "Transfer native roundtrip",
    )?;
    let mut d = fixture()?;
    d.scene.selection_mask = None;
    d.commit_state(core::prepare(&d, 2)?.state)?;
    check(
        d.layers.len() == 2
            && d.layers[1].pixels.iter().all(|p| *p == [0; 4])
            && d.layers[1].name == "Keep upper"
            && d.layers[0].pixels.iter().all(|p| *p == [200, 50, 100, 128]),
        "Whole transfer did not retain empty upper layer",
    )?;
    let mut d = fixture()?;
    d.scene.selection_mask = Some(Mask::default());
    let original = d.snapshot();
    let counts = d.history_counts();
    let result = core::prepare(&d, 2)?;
    check(
        result.changed == 0 && result.state == original,
        "Empty selection changed transfer",
    )?;
    d.commit_state(result.state)?;
    check(
        d.history_counts() == counts && d.snapshot() == original,
        "Empty selection added Undo",
    )?;
    let mut d = fixture()?;
    d.layers[0].locked = true;
    let original = d.snapshot();
    check(
        core::prepare(&d, 2).is_err() && d.snapshot() == original,
        "Locked lower layer partially changed transfer",
    )?;
    let mut d = fixture()?;
    d.layers.remove(1);
    check(
        core::prepare(&d, 1).is_err(),
        "Missing lower layer accepted",
    )?;
    for patch in [
        p::Patch {
            opacity: Some(40),
            ..Default::default()
        },
        p::Patch {
            blend: Some(crate::layer_blend::Mode::Multiply),
            ..Default::default()
        },
        p::Patch {
            hidden: Some(true),
            ..Default::default()
        },
        p::Patch {
            pixels_locked: Some(true),
            ..Default::default()
        },
    ] {
        let mut d = fixture()?;
        d.commit_state(p::prepare_patch(&d, &[Target::Raster(1)], patch)?)?;
        let original = d.snapshot();
        check(
            core::prepare(&d, 2).is_err() && d.snapshot() == original,
            "Ineligible destination transfer was not atomic",
        )?;
    }
    // Preserve signed, off-canvas content and lower pixels outside the selected region.
    let mut d = fixture()?;
    d.layers[0].placement = Some(crate::document::PlacedRaster {
        offset: [-2, 4],
        width: 3,
        height: 1,
    });
    d.layers[0].pixels = vec![[10, 20, 30, 255]; 3];
    d.layers[1].placement = Some(crate::document::PlacedRaster {
        offset: [-1, 4],
        width: 3,
        height: 1,
    });
    d.scene.selection_mask = None;
    d.scene.selection = Some(crate::scene::Rect {
        min: [-1., 4.],
        max: [0., 5.],
    });
    let outside = d.layers[0].pixel(3, 1, -2, 4);
    let src_outside = d.layers[1].pixel(3, 1, 0, 4);
    d.commit_state(core::prepare(&d, 2)?.state)?;
    check(
        d.layers[0].pixel(3, 1, -2, 4) == outside
            && d.layers[1].pixel(3, 1, 0, 4) == src_outside
            && d.layers[1].pixel(3, 1, -1, 4) == [0; 4]
            && d.layers[0].pixel(3, 1, -1, 4) == [105, 35, 65, 255],
        "Signed source/destination bounds or unselected preservation",
    )?;
    // All source/destination alpha endpoints and soft/full coverage preserve composition.
    for ua in [0, 1, 64, 128, 254, 255] {
        for ba in [0, 1, 128, 255] {
            for c in [1, 128, 255] {
                let mut d = fixture()?;
                d.layers[0].pixels = vec![[20, 100, 220, ba]; 3];
                d.layers[1].pixels = vec![[180, 60, 120, ua]; 3];
                d.scene.selection_mask = Some(Mask {
                    runs: vec![Run {
                        x: 0,
                        y: 0,
                        length: 3,
                        alpha: c,
                    }],
                });
                let original = (0..3).map(|x| d.composite_pixel(x, 0)).collect::<Vec<_>>();
                let p = core::prepare(&d, 2)?;
                check(p.maximum_delta <= 1, "Alpha-grid transfer comparison")?;
                d.commit_state(p.state)?;
                for (x, want) in original.into_iter().enumerate() {
                    check(
                        d.composite_pixel(x as u32, 0)
                            .iter()
                            .zip(want)
                            .all(|(a, b)| a.abs_diff(b) <= 1),
                        "Alpha-grid appearance changed",
                    )?;
                }
            }
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn transfer_selection_full_empty_locks_signed_alpha_native_undo() {
        let root = crate::test_support::TempDir::new();
        super::verify(&root.0).unwrap();
    }
}
