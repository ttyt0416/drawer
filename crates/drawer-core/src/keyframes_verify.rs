use crate::{
    document::{Background, CanvasSpec, DEFAULT_DOCUMENT_BUDGET as B, Document},
    keyframes::{self as core, Curve, Edit, Keyframe, Owner, Pose},
    layer_properties::{self as props, Target},
    layer_search::Key,
};
use std::path::Path;
fn check(ok: bool, text: &str) -> Result<(), String> {
    if ok { Ok(()) } else { Err(text.into()) }
}
pub fn fixture() -> Document {
    let mut d = Document::create(
        CanvasSpec {
            width: 32,
            height: 24,
            dpi: 300.,
            background: Background::Transparent,
        },
        B,
    )
    .unwrap();
    d.layers[0].pixels[3 * 32 + 2] = [255, 0, 0, 255];
    let s = crate::timeline_editor::create(&d, 24, Default::default()).unwrap();
    d.commit_state(s).unwrap();
    d.current_frame = 4;
    d
}
fn apply(d: &mut Document, o: Owner, e: Edit) -> Result<(), String> {
    let s = core::prepare(d, o, e)?;
    d.commit_state(s)?;
    Ok(())
}
fn put(d: &mut Document, o: Owner, frame: u32, pose: Pose, curve: Curve) -> Result<(), String> {
    apply(
        d,
        o,
        Edit::Put {
            key: Keyframe { frame, pose, curve },
            replace: true,
        },
    )
}
fn pixel(d: &Document, p: [f64; 2]) -> [u8; 4] {
    crate::render::Renderer::new(d).pixel(p)
}
pub fn verify(root: &Path) -> Result<(), String> {
    let dir = root.join("layer-keyframe-verify");
    std::fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let result = (|| {
        let mut d = fixture();
        let key = Key::Layer(props::raster(&d, 0));
        let owner = Owner::Layer(key);
        let mut s = d.snapshot();
        s.scene
            .layer_groups
            .set_frame(key, crate::scene::Affine([1., 0., 0., 1., 2., 3.]));
        d.commit_state(s)?;
        let before = d.snapshot();
        let pixels = d.layers[0].pixels.clone();
        let baseline = pixel(&d, [2.5, 3.5]);
        let count = d.history_counts().0;
        apply(&mut d, owner, Edit::Enable(true))?;
        let t = d.scene.keyframes.get(owner).unwrap();
        check(
            t.keys.len() == 1
                && t.keys[0].frame == 4
                && t.basis.0 == [1., 0., 0., 1., 2., 3.]
                && pixel(&d, [2.5, 3.5]) == baseline,
            "First activation changed base transform/pixels or used frame zero",
        )?;
        check(
            d.history_counts().0 == count + 1 && d.undo() && d.snapshot() == before && d.redo(),
            "Enable Undo/Redo",
        )?;
        put(
            &mut d,
            owner,
            12,
            Pose {
                offset: [8., 0.],
                opacity: 0.5,
                ..Default::default()
            },
            Curve::Linear,
        )?;
        let t = d.scene.keyframes.get(owner).unwrap();
        check(
            t.at(0).offset == [0., 0.]
                && t.at(20).offset == [8., 0.]
                && t.at(8).offset == [4., 0.]
                && t.at(8).opacity == 0.75,
            "Linear range/multiproperty evaluation",
        )?;
        d.current_frame = 8;
        let moved = pixel(&d, [6.5, 3.5]);
        check(
            moved[0] == 255
                && (190..=192).contains(&moved[3])
                && pixel(&d, [2.5, 3.5])[3] == 0
                && d.layers[0].pixels == pixels,
            "Animated raster composition failed or baked source",
        )?;
        let keys = d.scene.keyframes.get(owner).unwrap().keys.clone();
        let image = pixel(&d, [6.5, 3.5]);
        apply(&mut d, owner, Edit::Enable(false))?;
        d.current_frame = 15;
        check(
            pixel(&d, [6.5, 3.5]) == image && d.scene.keyframes.get(owner).unwrap().keys == keys,
            "Disable failed to freeze / retained keys changed",
        )?;
        apply(&mut d, owner, Edit::Enable(true))?;
        check(
            d.scene.keyframes.get(owner).unwrap().keys == keys
                && d.scene.keyframes.get(owner).unwrap().at(15).offset == [8., 0.],
            "Reactivation rewrote original keys",
        )?;
        check(
            core::prepare(
                &d,
                owner,
                Edit::Put {
                    key: keys[0],
                    replace: false,
                },
            )
            .is_err()
                && core::prepare(&d, owner, Edit::Move { from: 4, to: 13 }).is_err()
                && core::prepare(&d, owner, Edit::Move { from: 4, to: 12 }).is_err(),
            "Collision / reverse-time accepted",
        )?;
        let invalid = Curve::Bezier {
            first: [0.9, 0.1],
            second: [0.1, 0.9],
        };
        check(
            !invalid.valid()
                && core::prepare(
                    &d,
                    owner,
                    Edit::Put {
                        key: Keyframe {
                            curve: invalid,
                            ..keys[0]
                        },
                        replace: true,
                    },
                )
                .is_err(),
            "Reversed curve handles accepted",
        )?;
        let curve = Curve::Bezier {
            first: [0.1, 0.],
            second: [0.9, 1.],
        };
        check(
            (curve.weight(0.5) - 0.5).abs() < 1e-10 && curve.weight(0.2) < 0.2,
            "Bezier time-axis inversion / weighting",
        )?;
        put(&mut d, owner, 4, Pose::default(), Curve::Fixed)?;
        check(
            d.scene.keyframes.get(owner).unwrap().at(11).offset == [0., 0.]
                && d.scene.keyframes.get(owner).unwrap().at(12).offset == [8., 0.],
            "Fixed segment exact boundary",
        )?;
        put(&mut d, owner, 4, Pose::default(), curve)?;
        let mut blocked = d.snapshot();
        blocked.layers[0].pixels[0] = [0, 255, 0, 255];
        let unchanged = d.snapshot();
        check(
            d.commit_state(blocked).is_err() && d.snapshot() == unchanged,
            "Animated content edit not guarded",
        )?;
        let counts = d.history_counts();
        core::edit_content(&mut d, Some(owner))?;
        check(
            d.history_counts() == counts
                && d.snapshot() == unchanged
                && pixel(&d, [2.5, 3.5]) == baseline,
            "Base edit mode changed history/state or failed original view",
        )?;
        let original_keys = d.scene.keyframes.clone();
        let mut s = d.snapshot();
        s.layers[0].pixels[0] = [0, 255, 0, 255];
        d.commit_state(s)?;
        check(
            d.scene.keyframes == original_keys
                && d.undo()
                && d.scene.keyframes == original_keys
                && d.redo(),
            "Base content edit changed keys or failed independent Undo",
        )?;
        let exported = crate::render::Renderer::for_export(&d)?.pixel([10.5, 3.5]);
        check(
            exported[0] == 255 && (127..=129).contains(&exported[3]),
            "Base edit view leaked into export",
        )?;
        core::edit_content(&mut d, None)?;
        let saved = d.snapshot();
        let path = dir.join("keys.drawer");
        core::edit_content(&mut d, Some(owner))?;
        crate::files::save(&mut d, &path).map_err(|e| e.to_string())?;
        let mut opened = crate::files::open(&path, B).map_err(|e| e.to_string())?;
        check(
            opened.snapshot() == saved && opened.keyframe_content.is_none(),
            "Native98 lost basis/keys/hold or persisted base-edit view",
        )?;
        core::edit_content(&mut d, None)?;
        let mut bytes = std::fs::read(&path).unwrap();
        bytes[8..12].copy_from_slice(&97u32.to_le_bytes());
        std::fs::write(dir.join("old.drawer"), bytes).unwrap();
        check(
            crate::files::open(&dir.join("old.drawer"), B).is_err(),
            "Old header accepted layer animation",
        )?;
        opened.current_frame = 8;
        let retained = opened.snapshot();
        check(
            core::prepare(&opened, owner, Edit::Clear { confirmed: false }).is_err(),
            "Unconfirmed clear accepted",
        )?;
        apply(&mut opened, owner, Edit::Clear { confirmed: true })?;
        check(
            opened.scene.keyframes.get(owner).unwrap().keys.is_empty()
                && !opened.scene.keyframes.get(owner).unwrap().enabled
                && opened.undo()
                && opened.snapshot() == retained
                && opened.redo(),
            "All-key deletion did not preserve pose/Undo",
        )?;
        let mut s = d.snapshot();
        s.layers[0].locked = true;
        d.commit_state(s)?;
        check(
            core::prepare(&d, owner, Edit::Enable(false)).is_err()
                && core::edit_content(&mut d, Some(owner)).is_err(),
            "Locked animated layer editable",
        )?;
        d.undo();
        // Parent camera contributes once, including when both a folder and its child opt in.
        apply(&mut d, Owner::Camera, Edit::Enable(true))?;
        put(
            &mut d,
            Owner::Camera,
            15,
            Pose {
                offset: [0., 4.],
                ..Default::default()
            },
            Curve::Linear,
        )?;
        apply(&mut d, owner, Edit::ParentCamera(true))?;
        check(
            pixel(&d, [10.5, 7.5])[0] == 255,
            "Parent camera did not compose layer transform",
        )?;
        let mut s = d.snapshot();
        s.scene.paper.folders.push(crate::paper::Folder {
            name: "Parent".into(),
            through: false,
            children: vec![crate::paper::Node::Raster(0)],
            paper: Default::default(),
        });
        s.scene.paper.roots = vec![crate::paper::Node::Folder(0)];
        d.commit_state(s)?;
        let folder = Owner::Layer(Key::Layer(Target::Folder(0)));
        apply(&mut d, folder, Edit::ParentCamera(true))?;
        check(
            pixel(&d, [10.5, 7.5])[0] == 255 && pixel(&d, [10.5, 11.5])[3] == 0,
            "Camera was applied twice through folder",
        )?;
        // Both isolated and pass-through folder rendering, direct source picking and masks.
        let mut g = fixture();
        let raster = Key::Layer(props::raster(&g, 0));
        let mut s = g.snapshot();
        s.scene.paper.folders.push(crate::paper::Folder {
            name: "Animated folder".into(),
            through: true,
            children: vec![crate::paper::Node::Raster(0)],
            paper: Default::default(),
        });
        s.scene.paper.roots = vec![crate::paper::Node::Folder(0)];
        g.commit_state(s)?;
        let folder = Owner::Layer(Key::Layer(Target::Folder(0)));
        apply(&mut g, folder, Edit::Enable(true))?;
        put(
            &mut g,
            folder,
            4,
            Pose {
                offset: [3., 0.],
                opacity: 0.5,
                ..Default::default()
            },
            Curve::Linear,
        )?;
        check(
            pixel(&g, [5.5, 3.5])[0] == 255
                && (127..=129).contains(&pixel(&g, [5.5, 3.5])[3])
                && pixel(&g, [2.5, 3.5])[3] == 0,
            "Pass-through animated folder",
        )?;
        check(
            crate::float_render::Renderer::new(&g)?
                .display_source(raster, [5.5, 3.5])?
                .rgba8()[0]
                == 255,
            "Direct source picking ignored animated parent",
        )?;
        core::edit_content(&mut g, Some(folder))?;
        let mut s = g.snapshot();
        s.scene.paper.folders[0].through = false;
        s.scene
            .layer_effects
            .rasters
            .entry(g.scene.layer_ids.id(0))
            .or_default()
            .mask = Some(crate::layer_effects::Mask {
            space: Default::default(),
            coverage: crate::selection::Mask {
                runs: vec![crate::selection::Run {
                    x: 2,
                    y: 3,
                    length: 1,
                    alpha: 128,
                }],
            },
            offset: [0., 0.],
            enabled: true,
            linked: true,
            inverted: false,
        });
        g.commit_state(s)?;
        core::edit_content(&mut g, None)?;
        check(
            (63..=65).contains(&pixel(&g, [5.5, 3.5])[3]),
            "Animated folder mask did not follow base coordinates",
        )?;
        // Vector and page object paths use the same animation coordinates.
        let mut v = fixture();
        let mut s = v.snapshot();
        s.layers[0].pixels.fill([0; 4]);
        s.scene.lines.layers.push(crate::line_tools::Layer {
            fill: Default::default(),
            id: 1,
            name: "Line".into(),
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
                points: vec![[2., 3.], [2., 7.]],
                closed: false,
                width: 2.,
                color: [0, 255, 0, 255],
            }],
        });
        s.scene.lines.next_id = 3;
        v.commit_state(s)?;
        let o = Owner::Layer(Key::Layer(Target::Vector(1)));
        apply(&mut v, o, Edit::Enable(true))?;
        put(
            &mut v,
            o,
            4,
            Pose {
                offset: [5., 0.],
                ..Default::default()
            },
            Curve::Linear,
        )?;
        check(
            pixel(&v, [7.5, 4.5])[1] == 255 && pixel(&v, [2.5, 4.5])[3] == 0,
            "Vector keyframe rendering",
        )?;
        let mut s = v.snapshot();
        s.scene.objects.push(crate::scene::PageObject::Line {
            start: [3., 12.],
            end: [3., 17.],
            width_px: 2.,
            color: [0, 0, 255, 255],
        });
        v.commit_state(s)?;
        let object = v.scene.catalog.objects.id(0);
        let o = Owner::Layer(Key::Object(object));
        apply(&mut v, o, Edit::Enable(true))?;
        put(
            &mut v,
            o,
            4,
            Pose {
                offset: [6., 0.],
                ..Default::default()
            },
            Curve::Linear,
        )?;
        check(
            pixel(&v, [9.5, 14.5])[2] == 255 && pixel(&v, [3.5, 14.5])[3] == 0,
            "Object keyframe rendering",
        )?;
        // Track+cel transforms and opacity combine once in full composition and source inspection.
        let mut c = fixture();
        let mut s = c.snapshot();
        s.layers[0].pixels.fill([0; 4]);
        s.animation
            .as_mut()
            .unwrap()
            .tracks
            .push(crate::animation::Track {
                name: "Cel".into(),
                cells: vec![crate::animation::Cell {
                    name: "Blue".into(),
                    width: 1,
                    height: 1,
                    offset: [2, 3],
                    pixels: std::sync::Arc::new(vec![[0, 0, 255, 255]]),
                }],
                exposures: vec![crate::animation::Exposure { frame: 0, cell: 0 }],
                end: 24,
            });
        c.commit_state(s)?;
        let track = c.scene.catalog.tracks.id(0);
        let cell = c.scene.catalog.cell_id(track, 0);
        let a = Owner::Layer(Key::Track(track));
        let b = Owner::Layer(Key::Cell { track, cell });
        apply(&mut c, a, Edit::Enable(true))?;
        apply(&mut c, b, Edit::Enable(true))?;
        put(
            &mut c,
            a,
            4,
            Pose {
                offset: [3., 0.],
                opacity: 0.5,
                ..Default::default()
            },
            Curve::Linear,
        )?;
        put(
            &mut c,
            b,
            4,
            Pose {
                offset: [0., 2.],
                opacity: 0.5,
                ..Default::default()
            },
            Curve::Linear,
        )?;
        check(
            pixel(&c, [5.5, 5.5])[2] == 255
                && (63..=65).contains(&pixel(&c, [5.5, 5.5])[3])
                && crate::float_render::Renderer::new(&c)?
                    .display_source(Key::Track(track), [5.5, 5.5])?
                    .rgba8()
                    == pixel(&c, [5.5, 5.5]),
            "Track/cel transform or alpha applied incorrectly",
        )?;
        let copied = crate::layer_move::copy::prepare(&c, &[Key::Track(track)])?;
        let copied_track = copied.keys[0];
        c.commit_state(copied.state)?;
        let cloned = c
            .scene
            .keyframes
            .get(Owner::Layer(copied_track))
            .ok_or("Copied track keys missing")?;
        check(
            cloned.keys == c.scene.keyframes.get(a).unwrap().keys,
            "Track copy changed key values/times",
        )?;
        let delete_keys = crate::layer_groups::content_members(&c, &[copied_track])?;
        let deleted = crate::search_edit::delete_batch(&c, &delete_keys, true)?;
        c.commit_state(deleted)?;
        check(
            c.scene.keyframes.get(Owner::Layer(copied_track)).is_none() && core::valid_document(&c),
            "Deleted track left orphan keys",
        )?;
        let mut held = fixture();
        let o = Owner::Layer(Key::Layer(props::raster(&held, 0)));
        apply(&mut held, o, Edit::Enable(true))?;
        put(
            &mut held,
            o,
            10,
            Pose {
                offset: [8., 0.],
                ..Default::default()
            },
            Curve::Linear,
        )?;
        held.current_frame = 7;
        apply(&mut held, o, Edit::Enable(false))?;
        let held_state = held.snapshot();
        let path = dir.join("disabled.drawer");
        crate::files::save(&mut held, &path).map_err(|e| e.to_string())?;
        let reopened = crate::files::open(&path, B).map_err(|e| e.to_string())?;
        check(
            reopened.snapshot() == held_state && pixel(&reopened, [6.5, 3.5])[0] == 255,
            "Disabled static hold did not survive reload",
        )?;
        // Time editing includes disabled retained keys and exposes per-owner collisions.
        let mut f = fixture();
        let o = Owner::Layer(Key::Layer(props::raster(&f, 0)));
        apply(&mut f, o, Edit::Enable(true))?;
        put(
            &mut f,
            o,
            5,
            Pose {
                offset: [1., 0.],
                ..Default::default()
            },
            Curve::Linear,
        )?;
        apply(&mut f, o, Edit::Enable(false))?;
        let r = crate::timeline_time_edit::Request::Fps {
            fps: crate::animation::Fps {
                numerator: 1,
                denominator: 1,
            },
            keep_time: true,
            labels_linked: true,
        };
        let p = crate::timeline_time_edit::prepare(&f, r.clone(), &Default::default(), B)?;
        check(
            !p.ready() && p.issues.iter().any(|i| i.id.starts_with("layer-key-")),
            "Retained layer key collision omitted from conversion table",
        )?;
        crate::timeline_timing_verify::accept(&mut f, r)?;
        check(
            f.scene.keyframes.get(o).unwrap().keys.len() == 1,
            "FPS layer key selection not applied",
        )?;
        let mut f = fixture();
        let o = Owner::Layer(Key::Layer(props::raster(&f, 0)));
        apply(&mut f, o, Edit::Enable(true))?;
        let original = f.snapshot();
        let r = crate::timeline_time_edit::Request::Splice {
            at: 2,
            count: 3,
            remove: false,
            scope: crate::timeline_time_edit::Scope {
                keyed: vec![o],
                ..Default::default()
            },
            split: false,
            labels_linked: false,
        };
        crate::timeline_timing_verify::accept(&mut f, r)?;
        check(
            f.scene.keyframes.get(o).unwrap().keys[0].frame == 7
                && f.undo()
                && f.snapshot() == original,
            "Selected layer key insertion/Undo",
        )?;
        Ok(())
    })();
    let cleanup = std::fs::remove_dir_all(&dir).map_err(|e| e.to_string());
    result.and(cleanup)
}
#[cfg(test)]
mod tests {
    #[test]
    fn animation_lifecycle_rendering_timing_and_native() {
        let root = std::env::temp_dir().join(format!("drawer-keyframes-{}", std::process::id()));
        std::fs::create_dir(&root).unwrap();
        let r = super::verify(&root);
        std::fs::remove_dir(&root).unwrap();
        r.unwrap();
    }
    #[test]
    fn pose_matrix_and_curve_are_invertible() {
        use super::*;
        let p = Pose {
            offset: [2., 3.],
            scale: [2., 0.5],
            degrees: 90.,
            pivot: [4., 5.],
            opacity: 0.4,
        };
        let m = p.matrix();
        let v = [9., 11.];
        let q = m.inverse().map(m.map(v));
        assert!((q[0] - v[0]).abs() < 1e-10 && (q[1] - v[1]).abs() < 1e-10);
        assert_eq!(m.map(p.pivot), [6., 8.]);
        assert!(
            !Pose {
                scale: [0., 1.],
                ..p
            }
            .valid()
        );
    }
}
