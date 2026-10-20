use crate::{
    document::{DEFAULT_DOCUMENT_BUDGET, Document},
    layer_comps::{self as core, Command},
    layer_properties::{self as p, Target},
    layer_search::Key,
};
fn check(ok: bool, message: &str) -> Result<(), String> {
    if ok { Ok(()) } else { Err(message.into()) }
}
fn capture(d: &mut Document, name: &str) -> Result<u64, String> {
    let p = core::prepare(
        d,
        Command::Capture {
            name: name.into(),
            replace: None,
        },
    )?;
    let id = p.selected.unwrap();
    d.commit_state(p.state)?;
    Ok(id)
}
pub fn verify(root: &std::path::Path) -> Result<(), String> {
    let mut d = crate::raster_conversion_verify::mixed()?;
    d.animation = Some(crate::animation::Timeline {
        fps: Default::default(),
        frames: 2,
        audio: vec![],
        tracks: vec![crate::animation::Track {
            name: "Track".into(),
            end: 2,
            exposures: vec![crate::animation::Exposure { frame: 0, cell: 0 }],
            cells: vec![crate::animation::Cell {
                name: "Cell".into(),
                width: 1,
                height: 1,
                offset: [0, 0],
                pixels: std::sync::Arc::new(vec![[10, 20, 30, 128]]),
            }],
        }],
    });
    let state = crate::layer_folders::prepare(
        &d,
        crate::layer_folders::Command::Wrap {
            name: "Group".into(),
            targets: vec![Target::Raster(1)],
        },
    )?
    .state;
    d.commit_state(state)?;
    // Add a real gradient through the same creation path used by the tool.
    let gradient = crate::gradient::Layer {
        id: 1,
        name: "Gradient".into(),
        hidden: false,
        settings: Default::default(),
        creation: Default::default(),
        areas: vec![],
    };
    d.scene.gradients.layers.push(gradient);
    d.scene.gradients.next_id = 2;
    let targets = p::targets(&d);
    let mut state = d.snapshot();
    for (n, t) in targets.iter().copied().enumerate() {
        let mut props = p::properties(&d, t);
        props.opacity = 30 + (n % 60) as u8;
        props.hidden = n % 2 == 0;
        props.color = Some([10, 20, 30]);
        props.pixels_locked = !matches!(t, Target::Vector(_));
        if !matches!(t, Target::Cell { .. } | Target::Folder(_)) {
            props.blend = crate::layer_blend::Mode::Screen;
        }
        p::set_state(&d, &mut state, t, props)?;
    }
    d.commit_state(state)?;
    let before = d.snapshot();
    let counts = d.history_counts();
    let comp = capture(&mut d, "Visibility A")?;
    check(
        d.history_counts().0 == counts.0 + 1 && d.dirty,
        "Comp capture is a document transaction",
    )?;
    let saved = d.snapshot();
    check(
        d.undo() && d.snapshot() == before && d.redo() && d.snapshot() == saved,
        "Comp capture Undo/Redo",
    )?;
    let entries = d.scene.layer_comps.comps[0].entries.clone();
    check(
        entries.iter().any(|e| matches!(e.key, Key::Track(_)))
            && entries.iter().any(|e| matches!(e.key, Key::Cell { .. }))
            && entries.iter().any(|e| matches!(e.key, Key::Object(_)))
            && entries.iter().any(|e| matches!(e.key, Key::World(_)))
            && entries.iter().any(|e| matches!(e.key, Key::Gradient(_))),
        "Comp missed a renderable source",
    )?;
    let patch = p::Patch {
        opacity: Some(90),
        hidden: Some(false),
        blend: Some(crate::layer_blend::Mode::Multiply),
        ..Default::default()
    };
    let state = p::prepare_patch(&d, &targets, patch)?;
    d.commit_state(state)?;
    let original = d.snapshot();
    let counts = d.history_counts();
    let preview = core::prepare(&d, Command::Apply(comp))?;
    check(
        preview.missing.is_empty()
            && preview.unrecorded == 0
            && preview.changed > 0
            && d.snapshot() == original,
        "Comp preview changed source or missed states",
    )?;
    d.commit_state(preview.state)?;
    for entry in &entries {
        let t = crate::layer_catalog::target(&d, entry.key).unwrap();
        let actual = p::properties(&d, t);
        check(
            (actual.hidden, actual.opacity, actual.blend)
                == (entry.hidden, entry.opacity, entry.blend),
            "Comp did not restore own properties",
        )?;
        check(
            actual.color == Some([10, 20, 30])
                && actual.pixels_locked == !matches!(t, Target::Vector(_)),
            "Comp overwrote unrelated properties",
        )?;
    }
    check(
        d.scene.paper.folders[0].through
            && d.layers == original.layers
            && d.scene.objects == original.scene.objects
            && d.scene.lines == saved.scene.lines,
        "Comp changed content or failed to restore through",
    )?;
    let applied = d.snapshot();
    check(
        d.history_counts().0 == counts.0 + 1
            && d.undo()
            && d.snapshot() == original
            && d.redo()
            && d.snapshot() == applied,
        "Comp switch one Undo/Redo",
    )?;
    let path = root.join("layer-comps.drawer");
    crate::files::save(&mut d, &path).map_err(|e| e.to_string())?;
    let reopened = crate::files::open(&path, DEFAULT_DOCUMENT_BUDGET).map_err(|e| e.to_string())?;
    check(
        reopened.snapshot() == applied,
        "All comp fields native roundtrip",
    )?;
    let mut legacy = std::fs::read(&path).map_err(|e| e.to_string())?;
    legacy[8..12].copy_from_slice(&57u32.to_le_bytes());
    let legacy_path = root.join("layer-comps-invalid-version.drawer");
    std::fs::write(&legacy_path, legacy).map_err(|e| e.to_string())?;
    check(
        crate::files::open(&legacy_path, DEFAULT_DOCUMENT_BUDGET).is_err(),
        "Old version accepted comp data",
    )?;
    let state = crate::clipping::solo(&d, Some(0))?;
    d.commit_state(state)?;
    let solo = d.snapshot();
    check(
        core::prepare(&d, Command::Apply(comp)).is_err()
            && core::prepare(
                &d,
                Command::Capture {
                    name: "Solo".into(),
                    replace: None,
                },
            )
            .is_err()
            && d.snapshot() == solo,
        "Solo display leaked into comp",
    )?;
    let state = crate::clipping::solo(&d, None)?;
    d.commit_state(state)?;
    let mut state = d.snapshot();
    state.scene.paper.folders[0].through = false;
    state.scene.paper.folders[0].paper.seed = 77;
    d.commit_state(state)?;
    let conflict = d.snapshot();
    check(
        core::prepare(&d, Command::Apply(comp)).is_err() && d.snapshot() == conflict,
        "Incompatible through restore partially applied",
    )?;
    // Deleted stable IDs are reported; later sources keep their own settings.
    let mut d = crate::search_verify::page()?;
    d.scene.objects.push(crate::scene::PageObject::Line {
        start: [1., 1.],
        end: [3., 1.],
        width_px: 1.,
        color: [80, 120, 160, 255],
    });
    let comp = capture(&mut d, "Before deletion")?;
    let selected = crate::layer_search::Selection {
        keys: [Key::Object(1)].into_iter().collect(),
        primary: Some(Key::Object(1)),
        anchor: Some(Key::Object(1)),
    };
    let state = crate::search_edit::prepare(
        &d,
        &Default::default(),
        &selected,
        crate::search_edit::Edit::Delete,
    )?
    .state;
    d.commit_state(state)?;
    let mut scene = d.scene.clone();
    scene.objects.push(crate::scene::PageObject::Line {
        start: [2., 2.],
        end: [4., 2.],
        width_px: 1.,
        color: [40, 80, 120, 255],
    });
    d.edit_scene(scene)?;
    let new = d.scene.catalog.objects.id(0);
    check(new != 1, "Deleted object ID reused")?;
    let state = p::prepare_patch(
        &d,
        &[Target::Object(new)],
        p::Patch {
            opacity: Some(17),
            hidden: Some(true),
            ..Default::default()
        },
    )?;
    d.commit_state(state)?;
    let p = core::prepare(&d, Command::Apply(comp))?;
    check(
        p.missing == vec![Key::Object(1)] && p.unrecorded == 1,
        "Deleted/new comp layer report",
    )?;
    d.commit_state(p.state)?;
    check(
        p::properties(&d, Target::Object(new)).opacity == 17
            && p::properties(&d, Target::Object(new)).hidden,
        "Unrecorded layer overwritten",
    )?;
    let original = d.snapshot();
    for command in [
        Command::Apply(999),
        Command::Rename {
            id: comp,
            name: "\n".into(),
        },
        Command::Capture {
            name: " ".into(),
            replace: None,
        },
        Command::Capture {
            name: "bad".into(),
            replace: Some(999),
        },
        Command::Delete(999),
    ] {
        check(
            core::prepare(&d, command).is_err() && d.snapshot() == original,
            "Invalid comp command was not atomic",
        )?;
    }
    d.commit_state(
        core::prepare(
            &d,
            Command::Rename {
                id: comp,
                name: "Renamed".into(),
            },
        )?
        .state,
    )?;
    d.commit_state(
        core::prepare(
            &d,
            Command::Capture {
                name: "Updated".into(),
                replace: Some(comp),
            },
        )?
        .state,
    )?;
    check(
        core::prepare(&d, Command::Apply(comp))?.missing.is_empty(),
        "Comp update retained deleted entries",
    )?;
    d.commit_state(core::prepare(&d, Command::Delete(comp))?.state)?;
    let next = capture(&mut d, "Next")?;
    check(next > comp, "Deleted comp ID reused")?;
    let mut invalid = d.scene.layer_comps.clone();
    invalid.comps.push(invalid.comps[0].clone());
    check(!invalid.valid(), "Duplicate comp IDs accepted")?;
    let mut invalid = d.scene.layer_comps.clone();
    let e = invalid.comps[0].entries[0].clone();
    invalid.comps[0].entries.push(e);
    check(!invalid.valid(), "Duplicate comp layer accepted")?;
    let mut invalid = d.scene.layer_comps.clone();
    invalid.comps[0].entries[0].opacity = 101;
    check(!invalid.valid(), "Invalid comp opacity accepted")?;
    let mut bounded = crate::search_verify::page()?;
    for n in 0..64 {
        bounded.scene = core::prepare(
            &bounded,
            Command::Capture {
                name: format!("Comp {n}"),
                replace: None,
            },
        )?
        .state
        .scene;
    }
    check(
        core::prepare(
            &bounded,
            Command::Capture {
                name: "Overflow".into(),
                replace: None,
            },
        )
        .is_err(),
        "Comp count limit ignored",
    )?;
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn comp_all_sources_native_undo_deleted_new_validation() {
        let root = crate::test_support::TempDir::new();
        super::verify(&root.0).unwrap();
    }
}
