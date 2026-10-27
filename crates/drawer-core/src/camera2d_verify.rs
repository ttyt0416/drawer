use crate::{
    camera2d::{self as camera, Guides},
    document::{DEFAULT_DOCUMENT_BUDGET as B, Document},
    keyframes::{self as keys, Curve, Edit, Keyframe, Owner, Pose},
    layer_search::Key,
};
use std::path::Path;
fn check(ok: bool, message: &str) -> Result<(), String> {
    if ok { Ok(()) } else { Err(message.into()) }
}
fn apply(d: &mut Document, o: Owner, e: Edit) -> Result<(), String> {
    let s = keys::prepare(d, o, e)?;
    d.commit_state(s)?;
    Ok(())
}
fn put(d: &mut Document, frame: u32, pose: Pose, curve: Curve) -> Result<(), String> {
    apply(
        d,
        Owner::Camera,
        Edit::Put {
            key: Keyframe { frame, pose, curve },
            replace: true,
        },
    )
}
fn pixel(d: &Document, p: [f64; 2]) -> [u8; 4] {
    crate::render::Renderer::new(d).pixel(p)
}
fn export_pixel(d: &Document, p: [f64; 2]) -> [u8; 4] {
    crate::render::Renderer::for_export(d).unwrap().pixel(p)
}
pub fn verify(root: &Path) -> Result<(), String> {
    let dir = root.join("camera2d-verify");
    std::fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let result = (|| {
        let mut d = crate::keyframes_verify::fixture();
        let key = Key::Layer(crate::layer_properties::raster(&d, 0));
        let red = [255, 0, 0, 255];
        let original = d.snapshot();
        let counts = d.history_counts().0;
        let s = camera::enable(&d, true)?;
        d.commit_state(s)?;
        check(
            d.scene.keyframes.camera_output
                && pixel(&d, [2.5, 3.5]) == red
                && d.history_counts().0 == counts + 1,
            "Camera enable changed identity image / Undo grouping",
        )?;
        check(
            d.undo() && d.snapshot() == original && d.redo(),
            "Camera enable Undo/Redo",
        )?;
        let p = Pose {
            offset: [2., 1.],
            ..Default::default()
        };
        put(&mut d, 4, p, Curve::Linear)?;
        check(
            pixel(&d, [0.5, 2.5]) == red
                && pixel(&d, [2.5, 3.5])[3] == 0
                && d.layers == original.layers,
            "Camera pan failed / baked source",
        )?;
        let bounds =
            crate::layer_catalog::bounds(&d, key).ok_or("Missing camera-space layer bounds")?;
        check(
            bounds.min == [0., 2.],
            "Selection bounds did not follow camera",
        )?;
        check(
            crate::float_render::Renderer::new(&d)?
                .display_source(key, [0.5, 2.5])?
                .rgba8()
                == red,
            "Direct source inspection ignored camera",
        )?;
        let mut bad = p;
        bad.scale[0] = 0.;
        check(
            keys::prepare(
                &d,
                Owner::Camera,
                Edit::Put {
                    key: Keyframe {
                        frame: 4,
                        pose: bad,
                        curve: Curve::Linear,
                    },
                    replace: true,
                },
            )
            .is_err(),
            "Zero camera scale accepted",
        )?;
        put(
            &mut d,
            12,
            Pose {
                offset: [10., 1.],
                ..p
            },
            Curve::Linear,
        )?;
        let t = d.scene.keyframes.get(Owner::Camera).unwrap();
        check(
            t.at(0).offset == [2., 1.]
                && t.at(8).offset == [6., 1.]
                && t.at(999).offset == [10., 1.],
            "Camera outside-range clamp / interpolation",
        )?;
        put(&mut d, 4, p, Curve::Fixed)?;
        check(
            d.scene.keyframes.get(Owner::Camera).unwrap().at(8).offset == p.offset,
            "Fixed camera curve",
        )?;
        put(
            &mut d,
            4,
            p,
            Curve::Bezier {
                first: [0.2, 0.],
                second: [0.8, 1.],
            },
        )?;
        check(
            (d.scene.keyframes.get(Owner::Camera).unwrap().at(8).offset[0] - 6.).abs() < 1e-9,
            "Bezier camera timing",
        )?;
        put(
            &mut d,
            4,
            Pose {
                scale: [0.5; 2],
                ..Default::default()
            },
            Curve::Linear,
        )?;
        check(pixel(&d, [5., 7.]) == red, "Camera zoom failed")?;
        put(
            &mut d,
            4,
            Pose {
                degrees: 90.,
                pivot: [4., 4.],
                ..Default::default()
            },
            Curve::Linear,
        )?;
        check(
            pixel(&d, [3.5, 5.5]) == red,
            "Camera rotation / pivot failed",
        )?;
        put(&mut d, 4, p, Curve::Linear)?;
        // Camera-parented layers move in world space and stay fixed in camera coordinates.
        apply(&mut d, Owner::Layer(key), Edit::ParentCamera(true))?;
        check(
            pixel(&d, [2.5, 3.5]) == red && pixel(&d, [0.5, 2.5])[3] == 0,
            "Parent rig and final inverse camera did not cancel",
        )?;
        apply(&mut d, Owner::Layer(key), Edit::ParentCamera(false))?;
        // Source edits are explicit, in a camera-free base-content view; exports always use the camera.
        let mut s = d.snapshot();
        s.layers[0].pixels[0] = [0, 0, 255, 255];
        check(
            d.commit_state(s).is_err() && keys::guard(&d, key).is_err(),
            "Camera-space source edit accepted",
        )?;
        keys::edit_content(&mut d, Some(Owner::Layer(key)))?;
        check(
            pixel(&d, [2.5, 3.5]) == red && export_pixel(&d, [0.5, 2.5]) == red,
            "Base view or forced camera export failed",
        )?;
        let mut s = d.snapshot();
        s.layers[0].pixels[0] = [0, 0, 255, 255];
        d.commit_state(s)?;
        d.undo();
        keys::edit_content(&mut d, None)?;
        // Guides are document preferences, outside drawing snapshots and dirty/history.
        d.mark_saved(dir.join("camera.drawer"));
        // Three unevenly spaced keys and a rotated final framing guide.
        put(
            &mut d,
            6,
            Pose {
                offset: [4., 0.],
                degrees: 45.,
                pivot: [4., 4.],
                ..Default::default()
            },
            Curve::Linear,
        )?;
        put(
            &mut d,
            12,
            Pose {
                offset: [10., 1.],
                degrees: 90.,
                pivot: [4., 4.],
                ..Default::default()
            },
            Curve::Linear,
        )?;
        d.mark_saved(dir.join("camera.drawer"));
        let snapshot = d.snapshot();
        let history = d.history_counts();
        let revision = d.revision;
        let g = Guides {
            enabled: true,
            first: 4,
            last: 12,
            colors: [[0, 0, 0], [240, 80, 40]],
            corners: true,
            layer: Some(key),
            ..Default::default()
        };
        camera::set_guides(&mut d, g)?;
        check(
            !d.dirty
                && d.snapshot() == snapshot
                && d.history_counts() == history
                && d.revision == revision,
            "Guide view mutated drawing / dirty / history",
        )?;
        check(
            g.color(4) == [0; 3] && g.color(6) == [60, 20, 10] && g.color(12) == [240, 80, 40],
            "Guide colors based on key index instead of frame time",
        )?;
        let frames = camera::paths(&d)?;
        check(
            frames[0].corners == [[0., 0.], [32., 0.], [32., 24.], [0., 24.]],
            "Current filming guide is not aligned to output frame",
        )?;
        check(
            frames
                .iter()
                .filter(|f| f.camera)
                .map(|f| f.frame)
                .collect::<Vec<_>>()
                == [4, 6, 12]
                && frames.iter().any(|f| !f.camera),
            "Camera / layer key-only guides",
        )?;
        camera::set_guides(
            &mut d,
            Guides {
                keys_only: false,
                ..g
            },
        )?;
        let frames = camera::paths(&d)?;
        check(
            frames.len() == 18 && frames[2].frame == 6 && frames[2].color == [60, 20, 10],
            "All-frame guide range / uneven timing",
        )?;
        let rotated = frames.iter().find(|f| f.camera && f.frame == 12).unwrap();
        check(
            (rotated.corners[0][0] - 16.).abs() < 1e-9
                && rotated.corners[0][1].abs() < 1e-9
                && rotated.center == [12., 4.],
            "Rotated future camera frame / center projection",
        )?;
        // Guide points follow the same zoom/rotation/mirroring transform as the canvas.
        for flipped in [false, true] {
            let view = crate::view::View {
                zoom: 2.75,
                angle: 37.,
                pan: [13., -4.],
                flipped,
            };
            for p in frames[0].corners {
                let q = view.document_to_screen(p, [32., 24.], [300., 200.], 1.5);
                let back = view.screen_to_document(q, [32., 24.], [300., 200.], 1.5);
                check(
                    (back[0] - p[0]).abs() < 1e-9 && (back[1] - p[1]).abs() < 1e-9,
                    "Guide view projection round trip",
                )?;
            }
        }
        let dest = dir.join("camera.drawer");
        crate::files::save(&mut d, &dest).map_err(|e| e.to_string())?;
        let mut opened = crate::files::open(&dest, B).map_err(|e| e.to_string())?;
        opened.current_frame = 4;
        check(
            opened.scene.keyframes == d.scene.keyframes
                && opened.camera_guides == d.camera_guides
                && opened.scene.camera_guides.is_none()
                && opened.keyframe_content.is_none()
                && pixel(&opened, [0.5, 2.5]) == red,
            "Native camera / guides / session transport roundtrip",
        )?;
        let mut bytes = std::fs::read(&dest).map_err(|e| e.to_string())?;
        bytes[8..12].copy_from_slice(&100u32.to_le_bytes());
        let spoof = dir.join("old.drawer");
        std::fs::write(&spoof, &bytes).map_err(|e| e.to_string())?;
        check(
            crate::files::open(&spoof, B).is_err(),
            "Native100 accepted final camera / preferences",
        )?;
        // Actual PNG pixels and animation frames contain camera output, never paths or onions.
        let png = dir.join("camera.png");
        crate::png_export::export(&d, &png, B).map_err(|e| e.to_string())?;
        let image = image::open(&png).map_err(|e| e.to_string())?.into_rgba8();
        check(
            image.get_pixel(0, 2).0 == red
                && image.get_pixel(2, 3).0[3] == 0
                && image.pixels().filter(|p| p.0[3] > 0).count() == 1,
            "PNG camera / guide separation",
        )?;
        let capture = crate::animation_export::prepare(
            &d,
            crate::animation_export::Options {
                first: 4,
                last: 4,
                format: Default::default(),
                plays: 1,
            },
            B,
        )?;
        let output = dir.join("frames");
        crate::animation_export::write(
            &capture,
            &output,
            false,
            &Default::default(),
            &Default::default(),
        )?;
        let image = image::open(output.join("frame-000005.png"))
            .map_err(|e| e.to_string())?
            .into_rgba8();
        check(
            image.get_pixel(0, 2).0 == red && image.pixels().filter(|p| p.0[3] > 0).count() == 1,
            "Animation PNG camera / guide separation",
        )?;
        let settings = d.camera_guides;
        let s = camera::enable(&d, false)?;
        d.commit_state(s)?;
        check(
            d.undo() && d.camera_guides == settings && d.redo() && d.camera_guides == settings,
            "Camera Undo discarded view preferences",
        )?;
        // Dense paths are bounded, static and single-key cameras remain visible.
        let mut dense = crate::keyframes_verify::fixture();
        let mut s = camera::enable(&dense, true)?;
        s.animation.as_mut().unwrap().frames = 10000;
        dense.commit_state(s)?;
        camera::set_guides(
            &mut dense,
            Guides {
                enabled: true,
                keys_only: false,
                ..Default::default()
            },
        )?;
        check(
            camera::paths(&dense)?.len() == 10000,
            "Dense camera path limit",
        )?;
        let guide_prefs = Guides {
            keys_only: true,
            ..dense.camera_guides
        };
        camera::set_guides(&mut dense, guide_prefs)?;
        check(camera::paths(&dense)?.len() == 1, "Single-key camera guide")?;
        apply(&mut dense, Owner::Camera, Edit::Clear { confirmed: true })?;
        check(camera::paths(&dense)?.len() == 1, "Stationary camera guide")?;
        check(
            camera::set_guides(
                &mut dense,
                Guides {
                    first: 8,
                    last: 7,
                    ..Default::default()
                },
            )
            .is_err(),
            "Reversed path range accepted",
        )?;
        // Stored off-canvas source data can enter the filming frame.
        let mut off = crate::keyframes_verify::fixture();
        off.layers[0].placement = Some(crate::document::PlacedRaster {
            width: 1,
            height: 1,
            offset: [-2, 0],
        });
        off.layers[0].pixels = vec![red];
        let s = camera::enable(&off, true)?;
        off.commit_state(s)?;
        put(
            &mut off,
            4,
            Pose {
                offset: [-2., 0.],
                ..Default::default()
            },
            Curve::Linear,
        )?;
        check(
            pixel(&off, [0.5, 0.5]) == red,
            "Camera clipped world content at old canvas boundary",
        )?;
        // A single final camera samples ordered vector/object sources as well as raster trees.
        let mut mixed = crate::keyframes_verify::fixture();
        let mut s = mixed.snapshot();
        s.layers[0].pixels.fill([0; 4]);
        s.scene.objects.push(crate::scene::PageObject::Line {
            start: [3., 12.],
            end: [3., 17.],
            width_px: 2.,
            color: [0, 0, 255, 255],
        });
        s.scene.lines.layers.push(crate::line_tools::Layer {
            fill: Default::default(),
            id: 1,
            name: "Camera vector".into(),
            hidden: false,
            locked: false,
            paths: vec![crate::line_tools::Path {
                svg: None,
                tip: Default::default(),
                pressure: Default::default(),
                fill: None,
                stroke: true,
                definition: None,
                id: 2,
                points: vec![[6., 3.], [6., 7.]],
                closed: false,
                width: 2.,
                color: [0, 255, 0, 255],
            }],
        });
        s.scene.lines.next_id = 3;
        mixed.commit_state(s)?;
        let s = camera::enable(&mixed, true)?;
        mixed.commit_state(s)?;
        put(
            &mut mixed,
            4,
            Pose {
                offset: [2., 1.],
                ..Default::default()
            },
            Curve::Linear,
        )?;
        check(
            pixel(&mixed, [1.5, 13.5]) == [0, 0, 255, 255]
                && pixel(&mixed, [4.5, 3.5]) == [0, 255, 0, 255]
                && pixel(&mixed, [6.5, 4.5])[3] == 0,
            "Final camera omitted ordered vector / page-object sources",
        )?;
        let invalid = Guides {
            first: 5,
            last: 4,
            ..Default::default()
        };
        mixed.camera_guides = invalid;
        let bad_path = dir.join("invalid.drawer");
        check(
            crate::files::save(&mut mixed, &bad_path).is_err() && !bad_path.exists(),
            "Native save accepted invalid path range",
        )?;
        // Onion ghosts follow each evaluated camera without changing final composition.
        let mut onion = crate::onion_verify::fixture();
        let s = camera::enable(&onion, true)?;
        onion.commit_state(s)?;
        put(
            &mut onion,
            3,
            Pose {
                offset: [2., 0.],
                ..Default::default()
            },
            Curve::Linear,
        )?;
        put(&mut onion, 2, Pose::default(), Curve::Linear)?;
        put(
            &mut onion,
            4,
            Pose {
                offset: [4., 0.],
                ..Default::default()
            },
            Curve::Linear,
        )?;
        onion.onion = crate::timeline_view::Onion {
            enabled: true,
            skip_repeats: true,
            ..Default::default()
        };
        let guide = crate::timeline_view::preview(&onion, None, false, onion.onion, 10, B)?;
        check(
            guide.image.pixels[0] == [40, 60, 220, 255]
                && export_pixel(&onion, [0.5, 0.5]) == [40, 60, 220, 255]
                && guide.overlay.pixels[2][3] > 0
                && export_pixel(&onion, [2.5, 0.5])[3] == 0,
            "Onion / camera composition",
        )?;
        Ok(())
    })();
    std::fs::remove_dir_all(&dir).map_err(|e| e.to_string())?;
    result
}
#[cfg(test)]
mod tests {
    #[test]
    fn final_camera_curves_guides_native_outputs_and_source_guard() {
        let p = std::env::temp_dir().join(format!("drawer-camera-core-{}", std::process::id()));
        std::fs::create_dir(&p).unwrap();
        let result = super::verify(&p);
        std::fs::remove_dir(&p).unwrap();
        result.unwrap();
    }
}
