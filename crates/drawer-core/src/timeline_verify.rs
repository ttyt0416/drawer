use crate::{
    cels,
    document::{Background, CanvasSpec, DEFAULT_DOCUMENT_BUDGET as B, Document},
    layer_properties::{self as p, Target},
    scene::{PageObject, Rect},
    timeline_editor::{self as edit, Edit, Jump},
    timeline_view::{self, Onion},
};
use std::path::Path;
fn check(ok: bool, s: &str) -> Result<(), String> {
    if ok { Ok(()) } else { Err(s.into()) }
}
pub fn child() -> Document {
    let mut d = crate::page_parts_verify::fixture(0);
    d.animation = None;
    d.current_frame = 0;
    d.scene.catalog.tracks = Default::default();
    d.scene.catalog.cells.clear();
    d.scene.shots = Default::default();
    d.scene.objects.push(PageObject::Shadow {
        bounds: Rect {
            min: [3., 4.],
            max: [10., 15.],
        },
        offset: [1., 1.],
        radius_px: 2.,
        color: [0, 0, 0, 120],
    });
    d.commit_state(
        crate::layer_folders::prepare(
            &d,
            crate::layer_folders::Command::Wrap {
                name: "Editable cel folder".into(),
                targets: vec![Target::Raster(1)],
            },
        )
        .unwrap()
        .state,
    )
    .unwrap();
    d.commit_state(
        crate::layer_folders::prepare(
            &d,
            crate::layer_folders::Command::Mask {
                target: Target::Raster(1),
                mask: Some(crate::layer_effects::Mask {
                    coverage: crate::selection::Mask {
                        runs: (0..64)
                            .map(|y| crate::selection::Run {
                                x: 0,
                                y,
                                length: 32,
                                alpha: if y % 2 == 0 { 128 } else { 255 },
                            })
                            .collect(),
                    },
                    space: Default::default(),
                    offset: [0.; 2],
                    enabled: true,
                    linked: true,
                    inverted: false,
                }),
            },
        )
        .unwrap()
        .state,
    )
    .unwrap();
    d
}
pub fn parent() -> Document {
    Document::create(
        CanvasSpec {
            width: 32,
            height: 64,
            dpi: 300.,
            background: Background::Transparent,
        },
        B,
    )
    .unwrap()
}
fn apply(d: &mut Document, e: Edit) -> Result<edit::Prepared, String> {
    let p = edit::prepare(d, e, B)?;
    d.commit_state(p.state.clone())?;
    Ok(p)
}
pub fn verify(root: &Path) -> Result<(), String> {
    let dir = root.join("timeline-core-verify");
    std::fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let result = (|| {
        let mut d = parent();
        check(
            edit::create(&d, 0, Default::default()).is_err(),
            "Zero frame timeline accepted",
        )?;
        d.commit_state(edit::create(
            &d,
            10,
            crate::animation::Fps {
                numerator: 30000,
                denominator: 1001,
            },
        )?)?;
        let p = apply(&mut d, Edit::NewTrack("A".into()))?;
        let a = p.track.unwrap();
        let id = p.source.unwrap();
        let typed = child();
        d.commit_state(cels::replace(&d, id, &typed, B)?)?;
        check(
            d.scene.cels.get(id).unwrap().document().unwrap().snapshot() == typed.snapshot(),
            "Typed cell content lost on insert",
        )?;
        for frame in 1..10 {
            apply(
                &mut d,
                Edit::Expose {
                    track: a,
                    source: id,
                    frame,
                },
            )?;
        }
        let p = apply(
            &mut d,
            Edit::CopyTrack {
                track: a,
                independent: false,
            },
        )?;
        let b = p.track.unwrap();
        check(
            edit::uses(&d, id).len() == 20 && d.scene.cels.definitions.len() == 1,
            "Global cel is not shared across2 tracks/10 frames",
        )?;
        let before = d.snapshot();
        let mut changed = d.scene.cels.get(id).unwrap().open()?;
        changed.scene.objects.push(PageObject::Line {
            start: [0., 0.],
            end: [31., 63.],
            width_px: 3.,
            color: [10, 240, 220, 255],
        });
        changed.edit_pixel(0, 0, 0, [250, 0, 0, 255]);
        d.commit_state(cels::replace(&d, id, &changed, B)?)?;
        let a_index = d
            .scene
            .catalog
            .tracks
            .index(a, d.animation.as_ref().unwrap().tracks.len())
            .unwrap();
        let b_index = d
            .scene
            .catalog
            .tracks
            .index(b, d.animation.as_ref().unwrap().tracks.len())
            .unwrap();
        check(
            d.animation.as_ref().unwrap().tracks[a_index].cells[0].pixels
                == d.animation.as_ref().unwrap().tracks[b_index].cells[0].pixels,
            "Shared cel cache not updated for all tracks",
        )?;
        check(
            d.undo() && d.snapshot() == before && d.redo(),
            "Shared cel content one Undo/Redo failed",
        )?;
        let p = apply(
            &mut d,
            Edit::CopyTrack {
                track: a,
                independent: true,
            },
        )?;
        let independent = p.track.unwrap();
        let cell = d.scene.catalog.cell_id(independent, 0);
        let independent_id = d.scene.cels.source(independent, cell).unwrap();
        check(independent_id != id, "Independent copy reused global ID")?;
        let first = d.scene.cels.get(id).unwrap().document().unwrap().snapshot();
        let mut separate = d.scene.cels.get(independent_id).unwrap().open()?;
        separate.scene.objects.clear();
        separate.scene.catalog.objects = Default::default();
        d.commit_state(cels::replace(&d, independent_id, &separate, B)?)?;
        check(
            d.scene.cels.get(id).unwrap().document().unwrap().snapshot() == first,
            "Independent cel edit changed shared source",
        )?;
        let p = d.scene.cels.get(id).unwrap().document().unwrap();
        let renderer = crate::render::Renderer::new(&d);
        let child_renderer = crate::render::Renderer::new(p);
        let track_source = renderer.source_pixel(crate::layer_search::Key::Track(a), [5.5, 12.5]);
        check(
            track_source == child_renderer.pixel([5.5, 12.5]),
            "Typed cel renderer differs from independent editable source",
        )?;
        let mut hidden = p::properties(&d, Target::Track(a_index as u16));
        hidden.hidden = true;
        d.commit_state(p::prepare(&d, &[Target::Track(a_index as u16)], hidden)?)?;
        check(
            edit::navigate(&d, Some(a), Jump::NextCel).is_err(),
            "Hidden-track navigation accepted",
        )?;
        d.undo();
        check(
            edit::navigate(&d, Some(a), Jump::Last)? == 9
                && edit::navigate(&d, Some(a), Jump::First)? == 0
                && edit::navigate(&d, Some(a), Jump::NextLabel).is_err(),
            "First/last/no-label navigation failed",
        )?;
        let definitions = d.scene.cels.definitions.len();
        apply(&mut d, Edit::RemoveExposure { track: a, frame: 0 })?;
        check(
            d.scene.cels.definitions.len() == definitions && !edit::uses(&d, id).is_empty(),
            "Removing exposure deleted cel definition",
        )?;
        check(
            edit::navigate(&d, Some(a), Jump::Number(10)).is_err(),
            "Frame beyond end accepted",
        )?;
        d.current_frame = 1;
        check(
            edit::navigate(&d, Some(a), Jump::NextCel).is_err(),
            "Repeated same cel treated as a new cel",
        )?;
        apply(
            &mut d,
            Edit::Label {
                name: "Middle".into(),
                frame: 5,
            },
        )?;
        check(
            edit::navigate(&d, Some(a), Jump::NextLabel)? == 5
                && edit::navigate(&d, Some(a), Jump::NextKey).is_err(),
            "Label/no-key navigation failed",
        )?;
        let mut keyed = d.snapshot();
        keyed
            .scene
            .animation_output
            .keys
            .push(crate::animation_output::Frame {
                frame: 7,
                bounds: Rect {
                    min: [0., 0.],
                    max: [10., 10.],
                },
                creation: Default::default(),
            });
        d.commit_state(keyed)?;
        check(
            edit::navigate(&d, Some(a), Jump::NextKey)? == 7,
            "Existing key navigation failed",
        )?;
        d.undo();
        let mut outside = d.snapshot();
        outside
            .scene
            .animation_output
            .keys
            .push(crate::animation_output::Frame {
                frame: 9999,
                bounds: Rect {
                    min: [0., 0.],
                    max: [10., 10.],
                },
                creation: Default::default(),
            });
        d.commit_state(outside)?;
        check(
            edit::navigate(&d, Some(a), Jump::NextKey).is_err(),
            "Key outside this timeline was a navigation target",
        )?;
        d.undo();
        let p = apply(
            &mut d,
            Edit::NewCel {
                track: a,
                name: "Blank".into(),
            },
        )?;
        let blank = p.source.unwrap();
        d.current_frame = 0;
        check(
            edit::navigate(&d, Some(a), Jump::NextCel)? == 1,
            "Cel-boundary navigation failed",
        )?;
        let plain = d.snapshot();
        let history = d.history_counts();
        let onion = Onion {
            enabled: true,
            before: 1,
            after: 2,
            opacity: 40,
            ..Default::default()
        };
        let view = timeline_view::preview(&d, Some(a), true, onion, 64, B)?;
        check(
            view.overlay.pixels.iter().any(|p| p[3] > 0)
                && d.snapshot() == plain
                && d.history_counts() == history,
            "Onion view altered drawing or produced no guides",
        )?;
        timeline_view::preview(&d, Some(a), true, onion, 64, B)?;
        check(
            d.snapshot() == plain,
            "Only-edit-cell view changed track visibility",
        )?;
        let (a_frame, ended) = edit::playback(
            0,
            1.001,
            crate::animation::Fps {
                numerator: 30000,
                denominator: 1001,
            },
            100,
            false,
        )?;
        check(a_frame == 30 && !ended, "Rational FPS playback drift")?;
        check(
            edit::playback(0, 1., Default::default(), 1, true)?.0 == 0
                && edit::playback(0, 1., Default::default(), 1, false)?.1,
            "Single-frame playback endpoints",
        )?;
        let target = Target::Track(
            d.scene
                .catalog
                .tracks
                .index(b, d.animation.as_ref().unwrap().tracks.len())
                .unwrap() as u16,
        );
        let mut locked = p::properties(&d, target);
        locked.locked = true;
        d.commit_state(p::prepare(&d, &[target], locked)?)?;
        check(
            cels::replace(&d, id, &changed, B).is_err()
                && edit::prepare(&d, Edit::DeleteCel(id), B).is_err(),
            "Locked shared track did not reject whole edit",
        )?;
        d.undo();
        let path = dir.join("timeline.drawer");
        let saved = d.snapshot();
        crate::files::save(&mut d, &path).map_err(|e| e.to_string())?;
        let opened = crate::files::open(&path, B).map_err(|e| e.to_string())?;
        check(
            opened.snapshot() == saved && edit::uses(&opened, id).len() == edit::uses(&d, id).len(),
            "Native96 cel IDs/content/labels lost",
        )?;
        let mut old = std::fs::read(&path).unwrap();
        old[8..12].copy_from_slice(&95u32.to_le_bytes());
        std::fs::write(dir.join("old.drawer"), old).unwrap();
        check(
            crate::files::open(&dir.join("old.drawer"), B).is_err(),
            "Old native header accepted cel bank",
        )?;
        let count = edit::uses(&d, id).len();
        let before = d.snapshot();
        let p = apply(&mut d, Edit::DeleteCel(id))?;
        check(
            p.affected == count && d.scene.cels.get(id).is_none() && edit::uses(&d, id).is_empty(),
            "Cel deletion missed cross-track references",
        )?;
        check(
            d.undo() && d.snapshot() == before,
            "Cel deletion Undo failed to restore references",
        )?;
        let from_cell = d
            .scene
            .cels
            .links
            .iter()
            .find(|l| l.source == blank && l.track == a)
            .unwrap()
            .cell;
        apply(
            &mut d,
            Edit::MoveSlot {
                from: a,
                cell: from_cell,
                to: b,
            },
        )?;
        check(
            d.scene
                .cels
                .links
                .iter()
                .any(|l| l.track == b && l.source == blank)
                && !d
                    .scene
                    .cels
                    .links
                    .iter()
                    .any(|l| l.track == a && l.source == blank),
            "Cel move retained old slot reference",
        )?;
        let clip = child();
        d.current_frame = 2;
        let p = apply(
            &mut d,
            Edit::Paste {
                track: a,
                replace: None,
                document: Box::new(clip),
            },
        )?;
        let pasted = p.source.unwrap();
        check(
            edit::uses(&d, pasted).contains(&(a, 2)),
            "Pasted cel not assigned at current frame",
        )?;
        let paste_before = d.snapshot();
        let paste_undo = d.history_counts();
        let mut replacement = child();
        replacement.edit_pixel(0, 1, 0, [0, 255, 10, 255]);
        apply(
            &mut d,
            Edit::Paste {
                track: a,
                replace: Some(pasted),
                document: Box::new(replacement),
            },
        )?;
        check(
            d.history_counts().0 == paste_undo.0 + 1 && d.undo() && d.snapshot() == paste_before,
            "Selected cel paste did not restore content/references together",
        )?;
        apply(&mut d, Edit::DeleteTrack(a))?;
        check(
            d.scene.cels.get(pasted).is_some(),
            "Deleting track deleted an unused global cel definition",
        )?;
        let mut empty = parent();
        empty.commit_state(edit::create(&empty, 1, Default::default())?)?;
        let p = apply(&mut empty, Edit::NewTrack("Empty".into()))?;
        let t = p.track.unwrap();
        let cel = p.source.unwrap();
        apply(&mut empty, Edit::DeleteCel(cel))?;
        check(
            empty.animation.as_ref().unwrap().tracks[0].cells.is_empty()
                && empty.animation.as_ref().unwrap().tracks[0]
                    .exposures
                    .is_empty(),
            "Last cel cannot be explicitly deleted",
        )?;
        check(
            edit::navigate(&empty, Some(t), Jump::NextCel).is_err(),
            "Empty-track navigation fabricated a target",
        )?;
        let _ = independent;
        Ok(())
    })();
    let cleanup = std::fs::remove_dir_all(&dir).map_err(|e| e.to_string());
    result.and(cleanup)
}
#[cfg(test)]
mod tests {
    #[test]
    fn typed_shared_independent_cels_exposure_history_native_views_and_playback() {
        let root =
            std::env::temp_dir().join(format!("drawer-timeline-core-{}", std::process::id()));
        std::fs::create_dir(&root).unwrap();
        let result = super::verify(&root);
        std::fs::remove_dir(root).unwrap();
        result.unwrap();
    }
}
