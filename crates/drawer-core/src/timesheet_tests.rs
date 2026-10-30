use super::*;
use crate::document::DEFAULT_DOCUMENT_BUDGET as B;
fn publish(a: &Path, b: &Path) -> Result<(), String> {
    if b.exists() {
        return Err("Existing output".into());
    }
    std::fs::rename(a, b).map_err(|e| e.to_string())
}
#[test]
fn actual_json_csv_png_shared_sources_names_background_undo_and_native() {
    let root = crate::test_support::TempDir::new();
    crate::timesheet_verify::verify(&root.0, &mut publish).unwrap();
}
#[test]
fn malformed_missing_changed_sources_sorted_rows_and_no_partial_outputs() {
    let root = crate::test_support::TempDir::new();
    let d = crate::timesheet_verify::fixture();
    let cancel = AtomicBool::new(false);
    let progress = AtomicU32::new(0);
    let s = capture(&d, Options::default(), B).unwrap();
    let dir = root.0.join("bundle");
    let sheet = export(&s, &dir, &cancel, &progress, &mut publish).unwrap();
    let csv = crate::timesheet_csv::encode(&sheet).unwrap();
    let mut rows =
        crate::shot_exchange::csv_rows(std::str::from_utf8(&csv).unwrap(), 22, 400001).unwrap();
    let header = rows.remove(0);
    rows.reverse();
    rows.insert(0, header);
    let b = rows
        .iter()
        .map(|r| {
            r.iter()
                .map(|v| crate::shot_exchange::quote(v))
                .collect::<Vec<_>>()
                .join(",")
        })
        .collect::<Vec<_>>()
        .join("\r\n");
    assert_eq!(Sheet::decode(b.as_bytes(), true).unwrap(), sheet);
    let mut bad = sheet.clone();
    bad.tracks[0].exposures[0].frame = u32::MAX;
    assert!(Sheet::decode(&serde_json::to_vec(&bad).unwrap(), false).is_err());
    bad = sheet.clone();
    bad.tracks[0].cells[1].id = bad.tracks[0].cells[0].id;
    assert!(!bad.valid());
    bad = sheet.clone();
    bad.images[0].path = "../escape.png".into();
    assert!(!bad.valid());
    bad = sheet.clone();
    bad.images[0].path = "C:/remote.png".into();
    assert!(!bad.valid());
    bad = sheet.clone();
    bad.version = 2;
    assert!(!bad.valid());
    let mut negative = rows.clone();
    let exposure = negative.iter_mut().find(|r| r[0] == "exposure").unwrap();
    exposure[5] = "-1".into();
    let negative = negative
        .iter()
        .map(|r| {
            r.iter()
                .map(|v| crate::shot_exchange::quote(v))
                .collect::<Vec<_>>()
                .join(",")
        })
        .collect::<Vec<_>>()
        .join("\r\n");
    assert!(Sheet::decode(negative.as_bytes(), true).is_err());
    let p = dir.join(&sheet.images[0].path);
    let bytes = std::fs::read(&p).unwrap();
    let prepared = prepare(&dir.join("timesheet.json"), B, &cancel).unwrap();
    std::fs::write(&p, b"changed").unwrap();
    assert!(prepared.fresh().is_err());
    assert!(prepare(&dir.join("timesheet.csv"), B, &cancel).is_err());
    std::fs::remove_file(&p).unwrap();
    assert!(prepare(&dir.join("timesheet.json"), B, &cancel).is_err());
    std::fs::write(&p, bytes).unwrap();
    cancel.store(true, Ordering::Relaxed);
    assert!(
        export(
            &s,
            &root.0.join("precancel"),
            &cancel,
            &progress,
            &mut publish
        )
        .is_err()
    );
    cancel.store(false, Ordering::Relaxed);
    assert!(
        export_with(
            &s,
            &root.0.join("cancel"),
            &cancel,
            &progress,
            &mut publish,
            |n| if n == 1 {
                cancel.store(true, Ordering::Relaxed)
            }
        )
        .is_err()
    );
    assert!(!root.0.join("cancel").exists());
    cancel.store(false, Ordering::Relaxed);
    let race = root.0.join("race");
    assert!(
        export_with(&s, &race, &cancel, &progress, &mut publish, |n| if n == 1 {
            std::fs::create_dir(&race).unwrap();
            std::fs::write(race.join("foreign"), b"keep").unwrap();
        })
        .is_err()
    );
    assert_eq!(std::fs::read(race.join("foreign")).unwrap(), b"keep");
    assert!(std::fs::read_dir(&root.0).unwrap().all(|e| {
        !e.unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".Drawer-animation")
    }));
}
#[test]
fn draft_overflow_long_names_frozen_capture_and_presets() {
    let root = crate::test_support::TempDir::new();
    let mut d = crate::animation_export_verify::fixture();
    let id = d.scene.cels.definitions[0].object;
    let mut child = d.scene.cels.get(id).unwrap().open().unwrap();
    let key = crate::layer_search::Key::Layer(props::raster(&child, 0));
    child
        .commit_state(
            crate::layer_roles::prepare(
                &child,
                key,
                crate::layer_roles::Flags {
                    draft: true,
                    reference: false,
                },
                None,
            )
            .unwrap(),
        )
        .unwrap();
    child.layers[0].placement = Some(crate::document::PlacedRaster {
        width: 2,
        height: 1,
        offset: [-1, 0],
    });
    d.commit_state(crate::cels::replace(&d, id, &child, B).unwrap())
        .unwrap();
    d.animation.as_mut().unwrap().tracks[0].name = "한".repeat(1000);
    d.scene.cels.definitions[0].name = "CON".into();
    d.scene.cels.definitions[1].name = "CON".into();
    let c = AtomicBool::new(false);
    let p = AtomicU32::new(0);
    let off = capture(
        &d,
        Options {
            naming: Naming::FolderCel,
            overflow: true,
            ..Default::default()
        },
        B,
    )
    .unwrap();
    let on = capture(
        &d,
        Options {
            drafts: true,
            naming: Naming::FolderCel,
            overflow: true,
            ..Default::default()
        },
        B,
    )
    .unwrap();
    let before = d.snapshot();
    d.edit_pixel(0, 0, 0, [20, 255, 0, 255]);
    let a = export(&off, &root.0.join("off"), &c, &p, &mut publish).unwrap();
    let b = export(&on, &root.0.join("on"), &c, &p, &mut publish).unwrap();
    assert!(a.images.iter().all(|i| i.path.len() <= 220));
    assert!(
        image::open(root.0.join("off").join(&a.images[0].path))
            .unwrap()
            .into_rgba8()
            .pixels()
            .all(|p| p[3] == 0)
    );
    let img = image::open(root.0.join("on").join(&b.images[0].path))
        .unwrap()
        .into_rgba8();
    assert_eq!(img.width(), 3);
    assert_eq!(b.images[0].origin, [-1, 0]);
    assert_eq!(img.get_pixel(0, 0).0, [255, 0, 0, 128]);
    assert!(d.undo());
    assert_eq!(d.snapshot(), before);
    let o = Options {
        naming: Naming::FolderSerial,
        drafts: true,
        common: true,
        overflow: true,
        unassigned: false,
    };
    let bytes = encode_options(o).unwrap();
    assert_eq!(decode_options(&bytes).unwrap(), o);
    save_options(&root.0.join("preset.json"), o).unwrap();
    assert_eq!(
        decode_options(&std::fs::read(root.0.join("preset.json")).unwrap()).unwrap(),
        o
    );
    assert!(
        decode_options(br#"{"format":"Drawer timesheet preset","version":2,"options":{}}"#)
            .is_err()
    );
}

#[test]
fn designated_paper_is_kept_without_flattening_transparent_cels() {
    let t = crate::test_support::TempDir::new();
    let mut d = crate::timesheet_verify::fixture();
    d.commit_state(crate::paper_background::prepare(&d, [80, 50, 30], true).unwrap())
        .unwrap();
    let s = capture(
        &d,
        Options {
            common: true,
            ..Default::default()
        },
        B,
    )
    .unwrap();
    let folder = t.0.join("paper");
    let sheet = export(
        &s,
        &folder,
        &AtomicBool::new(false),
        &AtomicU32::new(0),
        &mut publish,
    )
    .unwrap();
    let common = image::open(folder.join(&sheet.common.as_ref().unwrap().path))
        .unwrap()
        .into_rgba8();
    assert_eq!(common.get_pixel(1, 0).0, [80, 50, 30, 255]);
    let cel = image::open(folder.join(&sheet.images[0].path))
        .unwrap()
        .into_rgba8();
    assert_eq!(cel.get_pixel(0, 0).0, [255, 0, 0, 128]);
    assert_eq!(cel.get_pixel(1, 0).0, [0, 0, 0, 0]);
}
