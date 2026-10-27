use crate::{
    animation::{Cell, Exposure, Timeline, Track},
    document::{Background, CanvasSpec, DEFAULT_DOCUMENT_BUDGET as B, Document},
    keyframes::{self, Curve, Edit, Keyframe, Owner, Pose},
    layer_search::Key,
    timeline_view::{self as view, ColorMode, Onion},
};
use std::{path::Path, sync::Arc};
fn check(ok: bool, message: &str) -> Result<(), String> {
    if ok { Ok(()) } else { Err(message.into()) }
}
pub fn fixture() -> Document {
    let mut d = Document::create(
        CanvasSpec {
            width: 10,
            height: 1,
            dpi: 300.,
            background: Background::Transparent,
        },
        B,
    )
    .unwrap();
    let mut s = d.snapshot();
    s.animation = Some(Timeline {
        fps: Default::default(),
        frames: 7,
        audio: vec![],
        tracks: vec![Track {
            name: "Colors".into(),
            end: 7,
            cells: vec![
                Cell {
                    name: "A".into(),
                    width: 1,
                    height: 1,
                    offset: [0, 0],
                    pixels: Arc::new(vec![[200, 20, 40, 255]]),
                },
                Cell {
                    name: "B".into(),
                    width: 1,
                    height: 1,
                    offset: [2, 0],
                    pixels: Arc::new(vec![[40, 60, 220, 255]]),
                },
                Cell {
                    name: "C".into(),
                    width: 1,
                    height: 1,
                    offset: [4, 0],
                    pixels: Arc::new(vec![[20, 180, 70, 255]]),
                },
            ],
            exposures: vec![
                Exposure { frame: 0, cell: 0 },
                Exposure { frame: 2, cell: 1 },
                Exposure { frame: 4, cell: 2 },
                Exposure { frame: 6, cell: 0 },
            ],
        }],
    });
    d.commit_state(s).unwrap();
    d.current_frame = 3;
    d.mark_saved("onion-fixture.drawer".into());
    d
}
pub fn settings() -> Onion {
    Onion {
        enabled: true,
        before: 3,
        after: 3,
        opacity: 80,
        colors: [[80, 10, 250], [240, 220, 50]],
        mode: ColorMode::Solid,
        fade: 20,
        skip_repeats: true,
    }
}
pub fn verify(root: &Path) -> Result<(), String> {
    let dir = root.join("onion-verify");
    std::fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let result = (|| {
        let mut d = fixture();
        let id = d.scene.catalog.tracks.id(0);
        let saved = d.snapshot();
        let h = d.history_counts();
        let path = d.path.clone();
        let dirty = d.dirty;
        let onion = settings();
        let p = view::preview(&d, Some(id), false, onion, 64, B)?;
        check(
            p.overlay.pixels[0] == [80, 10, 250, 153]
                && p.overlay.pixels[4] == [240, 220, 50, 204]
                && p.overlay.pixels[2][3] == 0,
            "Onion nearest held cel / directional color / distance fade",
        )?;
        for (mode, expected) in [
            (ColorMode::Original, [200, 20, 40, 153]),
            (ColorMode::Half, [140, 15, 145, 153]),
        ] {
            let p = view::preview(&d, None, false, Onion { mode, ..onion }, 64, B)?;
            check(
                p.overlay.pixels[0] == expected,
                "Original / half-tint guide color",
            )?;
        }
        let repeated = view::preview(
            &d,
            None,
            false,
            Onion {
                skip_repeats: false,
                ..onion
            },
            64,
            B,
        )?;
        check(
            repeated.overlay.pixels[0][3] > p.overlay.pixels[0][3],
            "Repeated exposures were not independently switchable",
        )?;
        check(
            Onion { fade: 100, ..onion }.tint([200, 30, 40, 255], 0, 2)[3] == 0,
            "Opacity subtraction became negative",
        )?;
        for frame in [0, 6] {
            d.current_frame = frame;
            let edge = Onion {
                before: if frame == 0 { 8 } else { 0 },
                after: if frame == 6 { 8 } else { 0 },
                ..onion
            };
            let p = view::preview(&d, None, false, edge, 64, B)?;
            check(
                p.overlay.pixels.iter().all(|p| p[3] == 0),
                "Onion wrapped beyond a timeline endpoint",
            )?;
        }
        d.current_frame = 3;
        let p = view::preview(
            &d,
            None,
            false,
            Onion {
                before: 0,
                after: 0,
                ..onion
            },
            64,
            B,
        )?;
        check(
            p.overlay.pixels.iter().all(|p| p[3] == 0),
            "Zero-count onion not empty",
        )?;
        let mut s = d.snapshot();
        s.scene.layer_properties.tracks.entry(0).or_default().hidden = true;
        d.commit_state(s)?;
        let p = view::preview(&d, None, false, onion, 64, B)?;
        check(
            p.overlay.pixels.iter().all(|p| p[3] == 0),
            "Hidden track leaked into onion",
        )?;
        d.undo();
        let preference_revision = d.revision;
        view::set_preferences(&mut d, onion)?;
        check(
            d.onion == onion.preferences()
                && d.snapshot() == saved
                && d.revision == preference_revision
                && d.history_counts().0 == h.0
                && d.path == path
                && d.dirty == dirty,
            "Stored view preferences altered drawing Undo/dirty/path",
        )?;
        let now = d.revision;
        let counts = d.history_counts();
        let p = view::preview(&d, None, false, onion, 64, B)?;
        check(
            d.snapshot() == saved
                && d.revision == now
                && d.history_counts() == counts
                && d.current_frame == 3
                && p.overlay.pixels.iter().any(|p| p[3] > 0),
            "Guide evaluation edited current document",
        )?;
        let dest = dir.join("preferences.drawer");
        crate::files::save(&mut d, &dest).map_err(|e| e.to_string())?;
        let mut opened = crate::files::open(&dest, B).map_err(|e| e.to_string())?;
        check(
            opened.onion == onion.preferences()
                && !opened.onion.enabled
                && opened.scene.onion.is_none()
                && opened.snapshot() == d.snapshot(),
            "Native99 did not preserve independent preferences",
        )?;
        let mut bytes = std::fs::read(&dest).unwrap();
        bytes[8..12].copy_from_slice(&98u32.to_le_bytes());
        std::fs::write(dir.join("old.drawer"), bytes).unwrap();
        check(
            crate::files::open(&dir.join("old.drawer"), B).is_err(),
            "Old header accepted view preference transport",
        )?;
        let mut s = opened.snapshot();
        s.layers[0].pixels[9] = [10, 20, 30, 255];
        opened.commit_state(s)?;
        view::set_preferences(&mut opened, Onion { fade: 37, ..onion })?;
        let settings = opened.onion;
        check(
            opened.undo() && opened.onion == settings && opened.redo() && opened.onion == settings,
            "Drawing Undo reverted onion preferences",
        )?;
        // Exports render the same pixels irrespective of all guide/view preferences.
        let before = (0..d.width)
            .map(|x| {
                crate::render::Renderer::for_export(&d)
                    .unwrap()
                    .pixel([x as f64 + 0.5, 0.5])
            })
            .collect::<Vec<_>>();
        view::set_preferences(
            &mut d,
            Onion {
                before: 8,
                after: 8,
                opacity: 100,
                fade: 0,
                mode: ColorMode::Original,
                ..onion
            },
        )?;
        let after = (0..d.width)
            .map(|x| {
                crate::render::Renderer::for_export(&d)
                    .unwrap()
                    .pixel([x as f64 + 0.5, 0.5])
            })
            .collect::<Vec<_>>();
        check(
            before == after && after[0][3] == 0 && after[4][3] == 0,
            "Onion colors leaked into export",
        )?;
        let capture = crate::animation_export::prepare(
            &d,
            crate::animation_export::Options {
                first: 3,
                last: 3,
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
            &std::sync::atomic::AtomicBool::new(false),
            &std::sync::atomic::AtomicU32::new(0),
        )?;
        let png = image::open(output.join("frame-000004.png"))
            .map_err(|e| e.to_string())?
            .into_rgba8();
        check(
            png.get_pixel(0, 0).0[3] == 0
                && png.get_pixel(4, 0).0[3] == 0
                && png.get_pixel(2, 0).0 == [40, 60, 220, 255],
            "Onion leaked into actual PNG output",
        )?;
        // An animated common layer also contributes reference poses without baking source pixels.
        let mut motion = crate::keyframes_verify::fixture();
        let key = Key::Layer(crate::layer_properties::raster(&motion, 0));
        let owner = Owner::Layer(key);
        let s = keyframes::prepare(&motion, owner, Edit::Enable(true))?;
        motion.commit_state(s)?;
        let s = keyframes::prepare(
            &motion,
            owner,
            Edit::Put {
                key: Keyframe {
                    frame: 8,
                    pose: Pose {
                        offset: [8., 0.],
                        ..Default::default()
                    },
                    curve: Curve::Linear,
                },
                replace: false,
            },
        )?;
        motion.commit_state(s)?;
        motion.current_frame = 6;
        let p = view::preview(
            &motion,
            None,
            false,
            Onion {
                enabled: true,
                before: 2,
                after: 2,
                skip_repeats: true,
                ..Default::default()
            },
            64,
            B,
        )?;
        check(
            p.overlay.pixels.iter().any(|p| p[3] > 0),
            "Moving common layer poses missing from onion",
        )?;
        let mut hidden = motion.snapshot();
        hidden.scene.paper.folders.push(crate::paper::Folder {
            name: "Parent".into(),
            through: false,
            children: vec![crate::paper::Node::Raster(0)],
            paper: crate::paper::Paper {
                hidden: true,
                ..Default::default()
            },
        });
        hidden.scene.paper.roots = vec![crate::paper::Node::Folder(0)];
        motion.commit_state(hidden)?;
        let p = view::preview(
            &motion,
            None,
            false,
            Onion {
                enabled: true,
                before: 2,
                after: 2,
                ..Default::default()
            },
            64,
            B,
        )?;
        check(
            p.overlay.pixels.iter().all(|p| p[3] == 0),
            "Hidden parent leaked animated common-layer guides",
        )?;
        motion.undo();
        let mut masked = motion.snapshot();
        masked.scene.paper.folders.push(crate::paper::Folder {
            name: "Masked parent".into(),
            through: false,
            children: vec![crate::paper::Node::Raster(0)],
            paper: Default::default(),
        });
        masked.scene.paper.roots = vec![crate::paper::Node::Folder(0)];
        masked
            .scene
            .layer_properties
            .folders
            .entry(0)
            .or_default()
            .opacity = 50;
        masked
            .scene
            .layer_effects
            .folders
            .entry(0)
            .or_default()
            .mask = Some(crate::layer_effects::Mask {
            space: Default::default(),
            coverage: crate::selection::Mask {
                runs: vec![crate::selection::Run {
                    x: 2,
                    y: 3,
                    length: 1,
                    alpha: 255,
                }],
            },
            offset: [0., 0.],
            enabled: true,
            linked: true,
            inverted: false,
        });
        motion.commit_state(masked)?;
        let p = view::preview(
            &motion,
            None,
            false,
            Onion {
                enabled: true,
                before: 2,
                after: 0,
                opacity: 100,
                ..Default::default()
            },
            64,
            B,
        )?;
        check(
            (127..=129).contains(&p.overlay.pixels[3 * 32 + 2][3])
                && p.overlay.pixels[3 * 32 + 4][3] == 0,
            "Parent mask / opacity missing from common-layer onion",
        )?;
        check(
            view::set_preferences(
                &mut d,
                Onion {
                    before: 9,
                    ..Default::default()
                },
            )
            .is_err()
                && view::preview(&d, None, false, Onion { fade: 101, ..onion }, 64, B).is_err(),
            "Invalid onion settings accepted",
        )?;
        Ok(())
    })();
    let cleanup = std::fs::remove_dir_all(dir).map_err(|e| e.to_string());
    result.and(cleanup)
}
#[cfg(test)]
mod tests {
    #[test]
    fn onion_colors_fade_held_cells_native_and_output() {
        let root = std::env::temp_dir().join(format!("drawer-onion-{}", std::process::id()));
        std::fs::create_dir(&root).unwrap();
        let r = super::verify(&root);
        std::fs::remove_dir(&root).unwrap();
        r.unwrap();
    }
}
