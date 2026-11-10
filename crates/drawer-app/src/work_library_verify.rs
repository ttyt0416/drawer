use super::*;
fn fake_recycle(path: &Path, cancel: Arc<AtomicBool>) -> Result<(), String> {
    drawer_core::asset_catalog::canceled(&cancel)?;
    if path.file_name().is_none_or(|n| n != "work.drawer") {
        return Err("Unexpected verification target".into());
    }
    let bin = path.parent().ok_or("Fixture parent")?.join("fixture-bin");
    if !bin.is_dir() {
        return Err("Owned simulated bin missing".into());
    }
    std::fs::rename(path, bin.join("work.drawer")).map_err(|e| e.to_string())
}
pub(super) fn run(root: &Path) -> Result<(), String> {
    core::verify::run(root)?;
    let dir = root.join("work-library-ui");
    std::fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let result = (|| {
        let storage = UserStorage::at(dir.join("data")).map_err(|e| e.to_string())?;
        let native = dir.join("work.drawer");
        let png = dir.join("other.png");
        let mut d = Document::create(
            CanvasSpec {
                width: 2,
                height: 2,
                dpi: 300.,
                background: Background::Transparent,
            },
            DEFAULT_DOCUMENT_BUDGET,
        )
        .map_err(|e| e.to_string())?;
        d.layers[0].pixels.fill([10, 80, 150, 255]);
        drawer_core::files::save(&mut d, &native).map_err(|e| e.to_string())?;
        drawer_core::png_export::export(&d, &png, DEFAULT_DOCUMENT_BUDGET)
            .map_err(|e| e.to_string())?;
        let native_bytes = std::fs::read(&native).map_err(|e| e.to_string())?;
        let png_bytes = std::fs::read(&png).map_err(|e| e.to_string())?;
        std::fs::create_dir(dir.join("fixture-bin")).map_err(|e| e.to_string())?;
        let ctx = egui::Context::default();
        let mut a = Drawer::new(&ctx, None).with_storage(storage.clone());
        a.simple_view = true;
        a.open_path(&native).map_err(|e| e.to_string())?;
        let before = a.documents[0].snapshot();
        let history = a.documents[0].history_counts();
        a.execute_command(&ctx, "work-library");
        if !a.work_library.open {
            return Err(format!("Work library command unavailable: {:?}", a.status));
        }
        // Native picker callback registers exactly the chosen owned paths.
        a.register_work_paths(vec![native.clone(), png.clone()])?;
        let first = a.work_library.db.as_ref().unwrap().state.entries[0].id;
        let mut t = 0.;
        let frame = |a: &mut Drawer, t: &mut f64, events| {
            *t += 0.05;
            input_replay::frame(&ctx, a, *t, events);
        };
        let click = |a: &mut Drawer, t: &mut f64, key: &str| -> Result<(), String> {
            for _ in 0..3 {
                frame(a, t, vec![]);
            }
            let mut pos = egui::Pos2::ZERO;
            for attempt in 0..16 {
                pos = if key == "first-row" {
                    a.work_library.rows.get(&first)
                } else {
                    a.work_library.controls.get(key)
                }
                .ok_or_else(|| format!("Missing work-library control {key}"))?
                .center();
                let visible = a.work_library.controls["viewport"].shrink(8.);
                if visible.contains(pos) {
                    break;
                }
                if attempt == 15 {
                    return Err(format!("Work-library control clipped: {key} {pos:?}"));
                }
                frame(
                    a,
                    t,
                    vec![
                        egui::Event::PointerMoved(egui::pos2(
                            visible.left() + 3.,
                            visible.center().y,
                        )),
                        egui::Event::MouseWheel {
                            unit: egui::MouseWheelUnit::Point,
                            phase: egui::TouchPhase::Move,
                            delta: egui::vec2(
                                0.,
                                if pos.y > visible.bottom() {
                                    -180.
                                } else {
                                    180.
                                },
                            ),
                            modifiers: Default::default(),
                        },
                    ],
                );
                for _ in 0..15 {
                    frame(a, t, vec![]);
                }
            }
            for pressed in [true, false] {
                frame(
                    a,
                    t,
                    vec![
                        egui::Event::PointerMoved(pos),
                        egui::Event::PointerButton {
                            pos,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: Default::default(),
                        },
                    ],
                );
            }
            Ok(())
        };
        let wait = |a: &mut Drawer| -> Result<(), String> {
            for _ in 0..8000 {
                if !a.quiesce_work_library(&ctx) {
                    return Ok(());
                }
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            Err("Library worker timeout".into())
        };
        a.work_library.title = "한글 작품".into();
        a.work_library.labels = "comic, local".into();
        click(&mut a, &mut t, "favorite")?;
        click(&mut a, &mut t, "save-info")?;
        if !a.work_library.db.as_ref().unwrap().entry(first)?.favorite {
            return Err(format!("Favorite/save pointer: {}", a.work_library.notice));
        }
        click(&mut a, &mut t, "path-labels")?;
        click(&mut a, &mut t, "sort-path")?;
        click(&mut a, &mut t, "rebuild")?;
        wait(&mut a)?;
        if a.work_library
            .index
            .as_ref()
            .is_none_or(|i| i.rows.len() != 2 || i.rows.iter().any(|r| r.thumbnail.is_none()))
        {
            return Err(format!("Thumbnail worker: {}", a.work_library.notice));
        }
        click(&mut a, &mut t, "remove")?;
        if !native.is_file() || a.work_library.db.as_ref().unwrap().entry(first).is_ok() {
            return Err("List removal affected file or retained row".into());
        }
        click(&mut a, &mut t, "undo")?;
        if a.work_library.db.as_ref().unwrap().entry(first).is_err() {
            return Err("Library undo did not restore row".into());
        }
        click(&mut a, &mut t, "first-row")?;
        a.work_library.recycle = fake_recycle;
        click(&mut a, &mut t, "recycle")?;
        click(&mut a, &mut t, "discard-ticket")?;
        if !native.is_file() {
            return Err("Review cancellation recycled a file".into());
        }
        click(&mut a, &mut t, "recycle")?;
        click(&mut a, &mut t, "confirm-recycle")?;
        wait(&mut a)?;
        let bin = dir.join("fixture-bin/work.drawer");
        if native.exists()
            || !bin.is_file()
            || a.documents[0].path.is_some()
            || !a.documents[0].dirty
            || a.documents[0].snapshot() != before
            || a.documents[0].history_counts() != history
        {
            return Err(format!(
                "Physical recycle/open-document detachment: {}",
                a.work_library.notice
            ));
        }
        click(&mut a, &mut t, "undo")?;
        if native.exists() || a.work_library.db.as_ref().unwrap().entry(first).is_err() {
            return Err("Library undo tried to undo an OS file operation".into());
        }
        click(&mut a, &mut t, "first-row")?;
        let mut p = std::mem::take(&mut a.work_library);
        a.work_action(&mut p, "reconnect", Some(bin.clone()))?;
        a.work_library = p;
        let e = a.work_library.db.as_ref().unwrap().entry(first)?;
        if !e.favorite
            || e.title != "한글 작품"
            || e.path != drawer_core::file_browser::checked(&bin)?
        {
            return Err("Reconnection lost registered information".into());
        }
        click(&mut a, &mut t, "rebuild")?;
        click(&mut a, &mut t, "close")?;
        wait(&mut a)?;
        if a.work_library.open || a.work_library.busy() {
            return Err("Closed library failed to drain its worker".into());
        }
        a.begin_work_library()?;
        if a.work_library.db.as_ref().unwrap().entry(first)?.title != "한글 작품" {
            return Err("Library restart lost metadata".into());
        }
        std::fs::write(
            storage.root().join("settings/work-library.db"),
            b"damaged metadata",
        )
        .map_err(|e| e.to_string())?;
        a.begin_work_library()?;
        if a.work_library.db.is_some() {
            return Err("Broken database silently reset".into());
        }
        click(&mut a, &mut t, "recovery-header")?;
        click(&mut a, &mut t, "recovery-confirm")?;
        click(&mut a, &mut t, "recover")?;
        if a.work_library.db.is_none() {
            return Err(format!(
                "Confirmed library recovery: {}",
                a.work_library.notice
            ));
        }
        if a.documents[0].snapshot() != before
            || a.documents[0].history_counts() != history
            || std::fs::read(&bin).map_err(|e| e.to_string())? != native_bytes
            || std::fs::read(&png).map_err(|e| e.to_string())? != png_bytes
        {
            return Err("Library metadata history/recovery changed drawing/source bytes".into());
        }
        drop(a);
        Ok(())
    })();
    drawer_core::asset_catalog::cleanup_verification(root, &dir)?;
    result
}
#[cfg(test)]
mod tests {
    #[test]
    fn work_list_pointer_information_thumbnail_recovery_missing_recycle_and_closed_worker() {
        let t = crate::tests::TestDirectory::new();
        super::run(&t.0).unwrap();
    }
}
