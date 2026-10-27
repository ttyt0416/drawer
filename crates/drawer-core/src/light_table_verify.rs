use crate::{
    document::{DEFAULT_DOCUMENT_BUDGET as B, Document},
    layer_properties::{self as props},
    layer_search::Key,
    light_table::{self as core, Add, Scope},
    references::LocalImage,
};
use std::path::Path;
fn check(ok: bool, message: &str) -> Result<(), String> {
    if ok { Ok(()) } else { Err(message.into()) }
}
pub fn fixture() -> Document {
    let mut d = crate::onion_verify::fixture();
    d.commit_state(crate::cels::enable(&d, B).unwrap()).unwrap();
    d.layers[0].pixels[8] = [240, 200, 30, 255];
    d.mark_saved("light-table-fixture.drawer".into());
    d
}
fn add(d: &mut Document, name: &str, scope: Scope, input: Add) -> Result<u64, String> {
    let (data, id) = core::add(d, name.into(), scope, input)?;
    d.edit_light_table(data)?;
    Ok(id)
}
pub fn verify(root: &Path) -> Result<(), String> {
    let dir = root.join("light-table-verify");
    std::fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let result = (|| {
        let mut d = fixture();
        let track = d.scene.catalog.tracks.id(0);
        let cell_a = d.scene.catalog.cell_id(track, 0);
        let cell_b = d.scene.catalog.cell_id(track, 1);
        let cell_c = d.scene.catalog.cell_id(track, 2);
        let a = d.scene.cels.source(track, cell_a).unwrap();
        let b = d.scene.cels.source(track, cell_b).unwrap();
        let c = d.scene.cels.source(track, cell_c).unwrap();
        let raster = Key::Layer(props::raster(&d, 0));
        let state = d.snapshot();
        let history = d.history_counts();
        let revision = d.revision;
        let id = add(&mut d, "Common yellow", Scope::Common, Add::Layer(raster))?;
        check(
            d.dirty
                && d.snapshot() == state
                && d.history_counts() == history
                && d.revision == revision
                && d.light_table.counts() == (1, 0),
            "Reference edit did not mark dirty independently of drawing history",
        )?;
        check(
            d.undo_light_table()
                && !d.dirty
                && d.light_table.data.references.is_empty()
                && d.redo_light_table()
                && d.dirty,
            "Reference Undo/Redo did not restore placement/dirty",
        )?;
        add(
            &mut d,
            "A for B",
            Scope::Cell(b),
            Add::Layer(Key::Cell {
                track,
                cell: cell_a,
            }),
        )?;
        add(
            &mut d,
            "B for C",
            Scope::Cell(c),
            Add::Layer(Key::Cell {
                track,
                cell: cell_b,
            }),
        )?;
        let pb = core::preview(&d, &d.light_table.data, Some(b), 64)?;
        let pc = core::preview(&d, &d.light_table.data, Some(c), 64)?;
        check(
            pb.image.pixels[0][0] == 200
                && pb.image.pixels[2][3] == 0
                && pc.image.pixels[2][2] == 220
                && pc.image.pixels[0][3] == 0
                && pb.image.pixels[8][0] == 240
                && pc.image.pixels[8][0] == 240,
            "Per-cel/common reference lists mixed",
        )?;
        let mut data = d.light_table.data.clone();
        data.get_mut(id).unwrap().pose.opacity = 0.5;
        data.opacity = 100;
        d.edit_light_table(data)?;
        let full = core::preview(&d, &d.light_table.data, None, 64)?
            .image
            .pixels[8];
        let mut data = d.light_table.data.clone();
        data.opacity = 40;
        d.edit_light_table(data)?;
        check(
            d.light_table.data.get(id).unwrap().pose.opacity == 0.5
                && full[3]
                    > core::preview(&d, &d.light_table.data, None, 64)?
                        .image
                        .pixels[8][3],
            "Overall opacity overwrote individual opacity",
        )?;
        let mut zero = d.light_table.data.clone();
        zero.opacity = 0;
        let individual = zero.get(id).unwrap().pose.opacity;
        check(
            core::preview(&d, &zero, None, 64)?
                .image
                .pixels
                .iter()
                .all(|p| p[3] == 0)
                && zero.get(id).unwrap().pose.opacity == individual,
            "Zero overall alpha changed individual settings or left visible references",
        )?;
        let pose = crate::keyframes::Pose {
            offset: [2., 3.],
            scale: [1.2, 0.7],
            degrees: 45.,
            ..Default::default()
        };
        let pivot = core::pivot(pose, [5., 6.]);
        let m = pose.matrix();
        let next = pivot.matrix();
        check(
            m.0.iter().zip(next.0).all(|(a, b)| (*a - b).abs() < 1e-10),
            "Changing pivot moved the reference",
        )?;
        let rotated = core::drag(pivot, [7., 6.], [5., 8.], true);
        check(
            (rotated.degrees - pivot.degrees - 90.).abs() < 1e-10,
            "Drag rotation did not use the clicked pivot",
        )?;
        check(
            core::add(
                &d,
                "Self".into(),
                Scope::Cell(a),
                Add::Layer(Key::Cell {
                    track,
                    cell: cell_a,
                }),
            )
            .is_err()
                && core::add(
                    &d,
                    "Self copy".into(),
                    Scope::Common,
                    Add::Copy {
                        origin: d.id,
                        title: d.title.clone(),
                        image: core::capture(&d)?,
                    },
                )
                .is_err(),
            "Self-reference accepted",
        )?;
        core::pin(&mut d, b, Some(555))?;
        let pin_state = d.snapshot();
        let h = d.history_counts();
        let dirty = d.dirty;
        let frame = d.current_frame;
        d.current_frame = 0;
        check(
            d.light_table.pin.unwrap().source == b
                && d.snapshot() == pin_state
                && d.history_counts() == h
                && d.dirty == dirty
                && core::pin_valid(&d),
            "Frame change redirected editing pin or changed drawing state",
        )?;
        d.current_frame = frame;
        let file = dir.join("source.png");
        let mut source = crate::onion_verify::fixture();
        source.animation = None;
        source.layers[0].pixels.fill([10, 50, 90, 255]);
        crate::png_export::export(&source, &file, B).map_err(|e| e.to_string())?;
        let local = add(
            &mut d,
            "Local image",
            Scope::Common,
            Add::File(file.clone()),
        )?;
        let local_pose = d.light_table.data.get(local).unwrap().pose;
        source.layers[0].pixels.fill([30, 180, 60, 255]);
        crate::png_export::export(&source, &file, B).map_err(|e| e.to_string())?;
        let refreshed = core::refresh(&d, local)?;
        d.edit_light_table(refreshed)?;
        check(
            d.light_table.data.get(local).unwrap().pose == local_pose,
            "File refresh lost placement",
        )?;
        std::fs::remove_file(&file).unwrap();
        let data = d.light_table.data.clone();
        check(
            core::refresh(&d, local).is_err()
                && d.light_table.data == data
                && core::preview(&d, &data, Some(c), 64)?
                    .notices
                    .iter()
                    .any(|m| m.contains("source file missing")),
            "Missing file lost saved reference or lacked status",
        )?;
        let copy = add(
            &mut d,
            "Copied canvas",
            Scope::Common,
            Add::Copy {
                origin: 99999,
                title: "Other canvas".into(),
                image: LocalImage {
                    width: 1,
                    height: 1,
                    pixels: vec![[0, 100, 240, 255]],
                },
            },
        )?;
        add(
            &mut d,
            "Onion reference",
            Scope::Common,
            Add::Onion {
                track: Some(track),
                frame: 0,
            },
        )?;
        let mut edited = d.scene.cels.get(a).unwrap().open()?;
        edited.edit_pixel(0, 0, 0, [0, 210, 80, 255]);
        let state = crate::cels::replace(&d, a, &edited, B)?;
        d.commit_state(state)?;
        let mut isolated = d.light_table.data.clone();
        isolated.references.retain(|r| r.name == "A for B");
        let p = core::preview(&d, &isolated, Some(b), 64)?;
        check(
            p.image.pixels[0][1] == 210,
            "Editing cel content did not refresh its live reference",
        )?;
        let mut plain = Document::create(
            crate::document::CanvasSpec {
                width: 2,
                height: 2,
                dpi: 300.,
                background: crate::document::Background::Transparent,
            },
            B,
        )
        .unwrap();
        let plain_key = Key::Layer(props::raster(&plain, 0));
        add(
            &mut plain,
            "Live metadata only",
            Scope::Common,
            Add::Layer(plain_key),
        )?;
        let metadata_path = dir.join("metadata-only.drawer");
        crate::files::save(&mut plain, &metadata_path).map_err(|e| e.to_string())?;
        check(
            crate::files::open(&metadata_path, B)
                .map_err(|e| e.to_string())?
                .light_table
                .data
                == plain.light_table.data,
            "Metadata-only auxiliary document without precision footer failed",
        )?;
        add(
            &mut plain,
            "Independent cache",
            Scope::Common,
            Add::Copy {
                origin: 999999,
                title: "Other".into(),
                image: LocalImage {
                    width: 1,
                    height: 1,
                    pixels: vec![[40, 50, 60, 255]],
                },
            },
        )?;
        let plain_path = dir.join("plain.drawer");
        crate::files::save(&mut plain, &plain_path).map_err(|e| e.to_string())?;
        check(
            crate::files::open(&plain_path, B)
                .map_err(|e| e.to_string())?
                .light_table
                .data
                == plain.light_table.data,
            "Legacy raster precision footer confused with reference payload",
        )?;
        let path = dir.join("auxiliary.drawer");
        let data = d.light_table.data.clone();
        let state = d.snapshot();
        crate::files::save(&mut d, &path).map_err(|e| e.to_string())?;
        check(
            !d.dirty && !d.light_table.dirty(),
            "Saving did not mark reference history saved",
        )?;
        let mut reopened = crate::files::open(&path, B).map_err(|e| e.to_string())?;
        check(
            reopened.light_table.data == data
                && reopened.snapshot() == state
                && reopened.light_table.pin.is_none()
                && reopened.light_table.counts() == (0, 0)
                && !reopened.light_table.visible
                && !reopened.dirty
                && reopened.scene.light_table.is_none(),
            "Native100 reference/image reload or session isolation",
        )?;
        check(
            core::add(
                &reopened,
                "Self file".into(),
                Scope::Common,
                Add::File(path.clone()),
            )
            .is_err(),
            "Native self-file reference accepted",
        )?;
        let mut bytes = std::fs::read(&path).unwrap();
        let end = bytes.len() - 1;
        bytes[end] ^= 1;
        std::fs::write(dir.join("corrupt.drawer"), &bytes).unwrap();
        check(
            crate::files::open(&dir.join("corrupt.drawer"), B).is_err(),
            "Corrupt reference payload accepted",
        )?;
        let mut bytes = std::fs::read(&path).unwrap();
        bytes[8..12].copy_from_slice(&99u32.to_le_bytes());
        std::fs::write(dir.join("old.drawer"), bytes).unwrap();
        check(
            crate::files::open(&dir.join("old.drawer"), B).is_err(),
            "Old header accepted reference objects",
        )?;
        reopened.current_frame = 3;
        let output = dir.join("drawing-only.png");
        crate::png_export::export(&reopened, &output, B).map_err(|e| e.to_string())?;
        let png = image::open(&output)
            .map_err(|e| e.to_string())?
            .into_rgba8();
        check(
            png.get_pixel(0, 0).0[3] == 0 && png.get_pixel(2, 0).0 == [40, 60, 220, 255],
            "Reference overlay leaked into actual PNG output",
        )?;
        let source_before = crate::render::Renderer::for_export(&reopened)?.pixel([8.5, 0.5]);
        let mut data = reopened.light_table.data.clone();
        data.references.clear();
        reopened.edit_light_table(data)?;
        check(
            reopened.dirty
                && crate::render::Renderer::for_export(&reopened)?.pixel([8.5, 0.5])
                    == source_before
                && reopened.undo_light_table()
                && !reopened.dirty
                && reopened.light_table.data.get(copy).is_some(),
            "Reference deletion/output exclusion/Undo dirty",
        )?;
        let h = reopened.history_counts();
        let mut state = reopened.snapshot();
        state.layers[0].pixels[8] = [50, 100, 200, 255];
        reopened.commit_state(state)?;
        let data = reopened.light_table.data.clone();
        check(
            reopened.undo()
                && reopened.light_table.data == data
                && reopened.redo()
                && reopened.light_table.data == data
                && reopened.history_counts().0 == h.0 + 1,
            "Drawing Undo reverted references",
        )?;
        // Removing a source retains a missing-reference record and never redirects the pin.
        core::pin(&mut reopened, b, None)?;
        let request = crate::timeline_editor::prepare(
            &reopened,
            crate::timeline_editor::Edit::DeleteCel(b),
            B,
        )?;
        reopened.commit_state(request.state)?;
        check(
            !core::pin_valid(&reopened)
                && reopened.light_table.pin.unwrap().source == b
                && core::preview(&reopened, &reopened.light_table.data, Some(c), 64)?
                    .notices
                    .iter()
                    .any(|m| m.contains("source layer removed")),
            "Deleted source/pin redirected to another cel",
        )?;
        let mut locked = fixture();
        let mut state = locked.snapshot();
        state
            .scene
            .layer_properties
            .tracks
            .entry(0)
            .or_default()
            .locked = true;
        locked.commit_state(state)?;
        check(
            core::pin(&mut locked, b, None).is_err(),
            "Locked cel was pinned for drawing",
        )?;
        Ok(())
    })();
    let cleanup = std::fs::remove_dir_all(dir).map_err(|e| e.to_string());
    result.and(cleanup)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn auxiliary_undo_scopes_native_sources_pin_and_export() {
        let root = std::env::temp_dir().join(format!("drawer-light-table-{}", std::process::id()));
        std::fs::create_dir(&root).unwrap();
        let r = super::verify(&root);
        std::fs::remove_dir(&root).unwrap();
        r.unwrap();
    }
    #[test]
    fn rejects_untrusted_image_size_without_overflow() {
        let mut d = core::Data::default();
        d.images.push(core::Image {
            id: 1,
            width: u32::MAX,
            height: u32::MAX,
            hash: [0; 32],
            pixels: std::sync::Arc::new(vec![]),
        });
        d.image_next = 2;
        assert!(!d.valid(true));
    }
}
