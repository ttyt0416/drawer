//! Generated editable fixtures and faults around the manifest publication boundary.
use crate::{
    document::{DEFAULT_DOCUMENT_BUDGET as B, Document},
    page_work::{self, Model},
    story_text::Work,
};
use std::{fs, path::Path};
fn check(ok: bool, why: &str) -> Result<(), String> {
    if ok { Ok(()) } else { Err(why.into()) }
}
fn native() -> Result<Vec<Document>, String> {
    let text = crate::text_version_verify::fixture()?;
    let line = crate::line_tools_verify::fixture()?;
    let combined = crate::page_parts::merge(&[(1, &text), (2, &line)], false, B)?;
    let world = crate::lighting_verify::fixture()?;
    let (mut animation, track) = crate::animation_finish_verify::fixture()?;
    let state = crate::timeline_editor::prepare(
        &animation,
        crate::timeline_editor::Edit::CopyTrack {
            track,
            independent: false,
        },
        B,
    )?
    .state;
    animation.commit_state(state)?;
    animation.current_frame = 37;
    Ok(vec![combined, world, animation])
}
pub fn run(root: &Path) -> Result<(), String> {
    let dir = root.join("native-generation-fixture");
    fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let result = (|| {
        let mut docs = native()?;
        let mut paths = vec![];
        for (n, d) in docs.iter_mut().enumerate() {
            d.edit_pixel(0, 0, 0, [n as u8 + 50, 90, 110, 255]);
            let file = dir.join(format!("source-{n}.drawer"));
            crate::files::save(d, &file).map_err(|e| e.to_string())?;
            let reopened = crate::files::open(&file, B).map_err(|e| e.to_string())?;
            check(
                d.snapshot() == reopened.snapshot()
                    && d.current_frame == reopened.current_frame
                    && d.revision == reopened.revision
                    && reopened.history_counts() == (0, 0)
                    && !reopened.dirty,
                "Editable native roundtrip/frame/revision/session-only history",
            )?;
            let bytes = fs::read(&file).map_err(|e| e.to_string())?;
            for damaged in [0, 1, 2] {
                let mut bad = bytes.clone();
                match damaged {
                    0 => {
                        let last = bad.len() - 1;
                        bad[last] ^= 1;
                    }
                    1 => {
                        bad.truncate(bad.len() - 7);
                    }
                    _ => {
                        bad[8..12].copy_from_slice(&(crate::files::VERSION + 1).to_le_bytes());
                    }
                }
                check(
                    crate::files::read_native_snapshot(&file, &bad, B).is_err(),
                    "Corruption/truncation/unknown version was accepted",
                )?;
            }
            paths.push(file);
        }
        let work = Model::new(Work::new("Generated pages".into(), paths)?)?;
        let manifest = dir.join("pages.drawer-work");
        work.work.save(&manifest)?;
        let baseline = fs::read(&manifest).map_err(|e| e.to_string())?;
        for d in &mut docs {
            d.edit_pixel(0, 1, 0, [210, 40, 80, 255]);
        }
        let before = docs
            .iter()
            .map(|d| {
                (
                    d.snapshot(),
                    d.revision,
                    d.history_counts(),
                    d.dirty,
                    d.path.clone(),
                )
            })
            .collect::<Vec<_>>();
        let opened = work
            .work
            .page_ids
            .iter()
            .copied()
            .zip(docs.iter())
            .collect::<Vec<_>>();
        for fail in 0..9 {
            let failed = page_work::snapshot(&work, &manifest, &opened, &mut |n, _| {
                if n == fail {
                    Err("Injected pre-publication interruption".into())
                } else {
                    Ok(())
                }
            });
            check(
                failed.is_err()
                    && fs::read(&manifest).map_err(|e| e.to_string())? == baseline
                    && Work::load(&manifest)? == work.work,
                "Failure published a partial/mixed generation",
            )?;
            check(
                fs::read_dir(&dir).map_err(|e| e.to_string())?.all(|e| {
                    e.is_ok_and(|e| {
                        !e.file_name()
                            .to_string_lossy()
                            .starts_with(".drawer-project-")
                            && !e
                                .file_name()
                                .to_string_lossy()
                                .starts_with("Drawer-project-")
                    })
                }),
                "Failed generation leaked owned blocks",
            )?;
        }
        let saved = page_work::snapshot(&work, &manifest, &opened, &mut |n, total| {
            if n == total {
                Err("Observer after commit".into())
            } else {
                Ok(())
            }
        })?;
        check(
            Work::load(&manifest)? == saved
                && saved
                    .generation
                    .as_ref()
                    .is_some_and(|g| g.pages.len() == 3),
            "Committed generation not fully reopenable",
        )?;
        for ((id, d), path) in opened.iter().zip(&saved.pages) {
            let reopened = crate::files::open(path, B).map_err(|e| e.to_string())?;
            check(
                d.snapshot() == reopened.snapshot()
                    && d.current_frame == reopened.current_frame
                    && d.revision == reopened.revision,
                "Page/shared cel generation changed",
            )?;
            saved.generation.as_ref().unwrap().verify_page(*id, path)?;
        }
        check(
            docs.iter().zip(&before).all(|(d, b)| {
                (
                    d.snapshot(),
                    d.revision,
                    d.history_counts(),
                    d.dirty,
                    d.path.clone(),
                ) == *b
            }),
            "Snapshot changed original edit/path/history",
        )?;
        let frozen = crate::save_generation::capture(&opened)?;
        docs[0].edit_pixel(0, 2, 0, [11, 22, 33, 255]);
        let frozen_refs = frozen.iter().map(|(id, d)| (*id, d)).collect::<Vec<_>>();
        let later = page_work::snapshot(&work, &manifest, &frozen_refs, &mut |_, _| Ok(()))?;
        let reopened = crate::files::open(&later.pages[0], B).map_err(|e| e.to_string())?;
        check(
            reopened.snapshot() == frozen[0].1.snapshot()
                && reopened.revision < docs[0].revision
                && docs[0].dirty,
            "Later edit entered the frozen save or became clean",
        )?;
        check(
            saved.pages.iter().all(|p| p.exists()),
            "Referenced previous generation was removed",
        )?;
        let page = &later.pages[1];
        let mut corrupt = fs::read(page).map_err(|e| e.to_string())?;
        let last = corrupt.len() - 1;
        corrupt[last] ^= 1;
        fs::write(page, corrupt).map_err(|e| e.to_string())?;
        check(
            Work::load(&manifest).is_err() && Model::new(later)?.open_page(2, B).is_err(),
            "Manifest/page open ignored a block hash mismatch",
        )
    })();
    fs::remove_dir_all(&dir).map_err(|e| e.to_string())?;
    result
}
/// Verification fixtures imitate older wire versions without the v124 footer.
pub fn legacy_version(bytes: &mut Vec<u8>, version: u32) {
    assert!((1..124).contains(&version));
    if bytes.len() >= 64 && &bytes[bytes.len() - 64..bytes.len() - 56] == b"DWEND1\r\n" {
        bytes.truncate(bytes.len() - 64);
    }
    bytes[8..12].copy_from_slice(&version.to_le_bytes());
}

#[cfg(test)]
mod tests {
    #[test]
    fn native_roundtrip_shared_cels_manifest_faults_corruption_and_frozen_edits() {
        let root = std::env::temp_dir().join(format!("drawer-generation-{}", std::process::id()));
        std::fs::create_dir(&root).unwrap();
        let result = super::run(&root);
        std::fs::remove_dir(&root).unwrap();
        result.unwrap();
    }
}
