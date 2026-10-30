use crate::{
    animation::Cell,
    document::{Background, CanvasSpec, DEFAULT_DOCUMENT_BUDGET as B, Document},
    layer_properties::{Properties, Target},
    timesheet::{self as core, Naming, Options},
};
use std::{
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU32},
    },
};
fn check(b: bool, m: &str) -> Result<(), String> {
    if b { Ok(()) } else { Err(m.into()) }
}
pub fn fixture() -> Document {
    let mut d = crate::animation_export_verify::fixture();
    d.edit_pixel(0, 0, 0, [10, 30, 80, 255]);
    let mut s = d.snapshot();
    s.animation.as_mut().unwrap().tracks[0].name = "같은, \"폴더\"".into();
    let mut second = s.animation.as_ref().unwrap().tracks[0].clone();
    second.cells[0].offset = [3, 0];
    s.animation.as_mut().unwrap().tracks.push(second);
    s.scene.catalog.tracks.append(2).unwrap();
    let track = s.scene.catalog.tracks.id(1);
    let mut ids = crate::layer_catalog::Ids::default();
    ids.initialize(2).unwrap();
    s.scene.catalog.cells.insert(track, ids);
    for c in 0..2 {
        let source = s.scene.cels.links[c].source;
        s.scene.cels.links.push(crate::cels::Link {
            track,
            cell: s.scene.catalog.cell_id(track, c),
            source,
        });
    }
    s.scene.layer_properties.set(
        Target::Track(0),
        Properties {
            opacity: 80,
            ..Default::default()
        },
    );
    s.scene.layer_properties.set(
        Target::Cell { track: 0, cell: 0 },
        Properties {
            opacity: 75,
            ..Default::default()
        },
    );
    s.scene.layer_properties.set(
        Target::Track(1),
        Properties {
            hidden: true,
            ..Default::default()
        },
    );
    for p in &mut s.scene.cels.definitions {
        p.name = "A".into();
    }
    let mut unused = Document::create(
        CanvasSpec {
            width: 1,
            height: 1,
            dpi: 300.,
            background: Background::Transparent,
        },
        B,
    )
    .unwrap();
    unused.layers[0].pixels[0] = [0, 255, 20, 255];
    let source = crate::cels::insert(&mut s, &unused, "Unused", B).unwrap();
    let first = s.scene.catalog.tracks.id(0);
    s.scene
        .catalog
        .cells
        .get_mut(&first)
        .unwrap()
        .append(3)
        .unwrap();
    s.animation.as_mut().unwrap().tracks[0].cells.push(Cell {
        name: "Unused slot".into(),
        width: 1,
        height: 1,
        offset: [0, 0],
        pixels: Arc::new(vec![[0, 255, 20, 255]]),
    });
    s.scene.cels.links.push(crate::cels::Link {
        track: first,
        cell: s.scene.catalog.cell_id(first, 2),
        source,
    });
    unused.layers[0].pixels[0] = [100, 255, 0, 255];
    crate::cels::insert(&mut s, &unused, &"긴이름".repeat(20), B).unwrap();
    d.commit_state(s).unwrap();
    d.commit_state(crate::paper_background::prepare(&d, [80, 50, 30], true).unwrap())
        .unwrap();
    d
}
pub fn verify(
    root: &Path,
    publish: &mut impl FnMut(&Path, &Path) -> Result<(), String>,
) -> Result<(), String> {
    let dir = root.join("timesheet-verify");
    std::fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let result = (|| {
        let d = fixture();
        let before = d.snapshot();
        let history = d.history_counts();
        let revision = d.revision;
        let frame = d.current_frame;
        let dirty = d.dirty;
        let cancel = AtomicBool::new(false);
        let progress = AtomicU32::new(0);
        for (n, naming) in [
            Naming::Cel,
            Naming::Serial,
            Naming::FolderCel,
            Naming::FolderSerial,
        ]
        .into_iter()
        .enumerate()
        {
            let path = dir.join(format!("bundle-{n}"));
            let s = core::capture(
                &d,
                Options {
                    naming,
                    common: true,
                    ..Default::default()
                },
                B,
            )?;
            let sheet = core::export(&s, &path, &cancel, &progress, publish)?;
            check(
                sheet.images.len() == 4
                    && sheet.tracks.len() == 2
                    && sheet.tracks[0].cells.len() == 3
                    && sheet.common.is_some(),
                "Distinct / unexposed / unassigned source count",
            )?;
            check(
                sheet.tracks[0].cells[0].source == sheet.tracks[1].cells[0].source
                    && sheet.tracks[1].cells[0].offset == [3, 0],
                "Shared source / independent slot offset",
            )?;
            check(
                sheet
                    .images
                    .iter()
                    .map(|i| i.path.to_lowercase())
                    .collect::<std::collections::BTreeSet<_>>()
                    .len()
                    == 4
                    && sheet.images.iter().all(|i| i.path.len() <= 220),
                "Safe unique local filenames",
            )?;
            let mut json = core::prepare(&path.join("timesheet.json"), B, &cancel)?;
            let csv = core::prepare(&path.join("timesheet.csv"), B, &cancel)?;
            check(
                json.sheet == csv.sheet && json.document.snapshot() == csv.document.snapshot(),
                "JSON/CSV restored different references",
            )?;
            json.fresh()?;
            let a = json.document.animation.as_ref().unwrap();
            check(
                a.fps == d.animation.as_ref().unwrap().fps
                    && a.tracks[0].exposures == d.animation.as_ref().unwrap().tracks[0].exposures
                    && a.tracks[0].cells.len() == 3,
                "FPS / exposures / unexposed slots",
            )?;
            let tr = &json.document.scene.catalog.tracks;
            check(
                json.document
                    .scene
                    .cels
                    .source(tr.id(0), json.document.scene.catalog.cell_id(tr.id(0), 0))
                    == json
                        .document
                        .scene
                        .cels
                        .source(tr.id(1), json.document.scene.catalog.cell_id(tr.id(1), 0)),
                "Import duplicated a shared editable source",
            )?;
            check(
                crate::layer_properties::properties(&json.document, Target::Track(1)).hidden
                    && crate::layer_properties::properties(
                        &json.document,
                        Target::Cell { track: 0, cell: 0 },
                    )
                    .opacity
                        == 75,
                "Hidden / opacity metadata",
            )?;
            for f in 0..3 {
                for x in 0..2 {
                    let mut original = crate::layer_merge::preview(&d.snapshot(), f)?;
                    original.scene.include_drafts_in_output = false;
                    let imported = crate::layer_merge::preview(&json.document.snapshot(), f)?;
                    let p = [x as f64 + 0.5, 0.5];
                    check(
                        crate::render::Renderer::for_export(&original)?.pixel(p)
                            == crate::render::Renderer::for_export(&imported)?.pixel(p),
                        "Restored timeline composite differs",
                    )?;
                }
            }
            let state = json.document.snapshot();
            check(
                json.document.undo()
                    && json.document.animation.is_none()
                    && json.document.redo()
                    && json.document.snapshot() == state,
                "Import single Undo / Redo",
            )?;
            let bytes = crate::files::native_bytes(&json.document).map_err(|e| e.to_string())?;
            let reopened =
                crate::files::read_native_snapshot(&path.join("check.drawer"), &bytes, B)
                    .map_err(|e| e.to_string())?;
            check(
                reopened.snapshot() == json.document.snapshot(),
                "Shared cel native roundtrip",
            )?;
            check(
                core::export(&s, &path, &cancel, &progress, publish).is_err(),
                "Existing bundle was replaced",
            )?;
        }
        let s = core::capture(
            &d,
            Options {
                unassigned: false,
                ..Default::default()
            },
            B,
        )?;
        let path = dir.join("exposed-only");
        let sheet = core::export(&s, &path, &cancel, &progress, publish)?;
        check(
            sheet.images.len() == 2 && sheet.tracks[0].cells.len() == 2 && sheet.common.is_none(),
            "Unassigned filter / common background toggle",
        )?;
        check(
            d.snapshot() == before
                && d.history_counts() == history
                && d.revision == revision
                && d.current_frame == frame
                && d.dirty == dirty,
            "Export mutated live document / history / view",
        )?;
        check(
            core::prepare(&dir.join("foreign.xdts"), B, &cancel).is_err()
                && core::Sheet::decode(br#"{"format":"CSP","version":1}"#, false).is_err(),
            "Unverified proprietary format accepted",
        )?;
        Ok(())
    })();
    let clean = std::fs::remove_dir_all(&dir).map_err(|e| e.to_string());
    result.and(clean)
}
