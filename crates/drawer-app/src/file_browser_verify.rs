use super::*;
fn simulated_recycle(path: &Path, c: Arc<AtomicBool>) -> Result<(), String> {
    drawer_core::asset_catalog::canceled(&c)?;
    let root = path.parent().ok_or("Missing test parent")?;
    if root.file_name().is_none_or(|s| s != "file-browser-ui") {
        return Err("Recycle fixture escaped its exclusive test folder".into());
    }
    let bin = root.join("simulated-bin");
    std::fs::create_dir(&bin).map_err(|e| e.to_string())?;
    drawer_platform::local_files::publish_no_replace(
        path,
        &bin.join(path.file_name().ok_or("Missing test name")?),
    )
    .map_err(|e| e.to_string())
}
pub(super) fn run(root: &Path) -> Result<(), String> {
    core::verify::run(root)?;
    drawer_platform::file_browser::verify(root)?;
    let dir = root.join("file-browser-ui");
    std::fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let result = (|| {
        let source = dir.join("원본.drawer");
        let image = dir.join("preview.png");
        let mut d = drawer_core::png_io_verify::fixture()?;
        drawer_core::files::save(&mut d, &source).map_err(|e| e.to_string())?;
        drawer_core::png_export::export(&d, &image, 64 * 1024 * 1024).map_err(|e| e.to_string())?;
        let original = std::fs::read(&source).map_err(|e| e.to_string())?;
        let png = std::fs::read(&image).map_err(|e| e.to_string())?;
        let ctx = egui::Context::default();
        let mut a = Drawer::new(&ctx, None)
            .with_storage(UserStorage::at(dir.join("data")).map_err(|e| e.to_string())?);
        a.simple_view = true;
        a.file_browser.draft.folder = Some(dir.canonicalize().map_err(|e| e.to_string())?);
        a.begin_file_browser(Mode::Open)?;
        let mut t = 0.;
        let frame = |a: &mut Drawer, t: &mut f64, events| {
            *t += 0.05;
            input_replay::frame(&ctx, a, *t, events);
        };
        let wait = |a: &mut Drawer, t: &mut f64| -> Result<(), String> {
            for _ in 0..8000 {
                frame(a, t, vec![]);
                if a.file_browser.job.is_none() && a.file_browser.pending.is_none() {
                    return Ok(());
                }
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            Err(format!(
                "File browser job timeout: {}",
                a.file_browser.message
            ))
        };
        let click_rect = |a: &mut Drawer, t: &mut f64, r: egui::Rect| {
            for pressed in [true, false] {
                frame(
                    a,
                    t,
                    vec![
                        egui::Event::PointerMoved(r.center()),
                        egui::Event::PointerButton {
                            pos: r.center(),
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: Default::default(),
                        },
                    ],
                );
            }
        };
        let click = |a: &mut Drawer, t: &mut f64, key: &str| -> Result<(), String> {
            for _ in 0..3 {
                frame(a, t, vec![]);
            }
            let r = *a
                .file_browser
                .controls
                .get(key)
                .ok_or("Missing file browser control")?;
            click_rect(a, t, r);
            Ok(())
        };
        let row = |a: &mut Drawer, t: &mut f64, path: &Path| -> Result<(), String> {
            a.file_browser.focus = Some(path.into());
            for _ in 0..3 {
                frame(a, t, vec![]);
            }
            let r = *a
                .file_browser
                .rows
                .get(path)
                .filter(|r| r.is_positive() && r.height() > 8.)
                .ok_or("File row not visible after production focus/scroll")?;
            click_rect(a, t, r);
            Ok(())
        };
        wait(&mut a, &mut t)?;
        let selected = source.canonicalize().map_err(|e| e.to_string())?;
        row(&mut a, &mut t, &selected)?;
        frame(
            &mut a,
            &mut t,
            vec![egui::Event::Key {
                key: egui::Key::Enter,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Default::default(),
            }],
        );
        frame(
            &mut a,
            &mut t,
            vec![egui::Event::Key {
                key: egui::Key::Enter,
                physical_key: None,
                pressed: false,
                repeat: false,
                modifiers: Default::default(),
            }],
        );
        if a.documents.len() != 1 || a.file_browser.open {
            return Err(format!(
                "Keyboard picker open failed: {}",
                a.file_browser.message
            ));
        }
        let before = a.documents[0].snapshot();
        let history = a.documents[0].history_counts();
        let second = dir.join("second.drawer");
        let mut another = drawer_core::png_io_verify::fixture()?;
        drawer_core::files::save(&mut another, &second).map_err(|e| e.to_string())?;
        a.begin_file_browser(Mode::Open)?;
        a.file_browser.selections = [
            selected.clone(),
            second.canonicalize().map_err(|e| e.to_string())?,
        ]
        .into_iter()
        .collect();
        click(&mut a, &mut t, "open")?;
        for _ in 0..20 {
            frame(&mut a, &mut t, vec![]);
            if a.file_browser.opens.is_empty() {
                break;
            }
        }
        if a.documents.len() != 2 || !a.file_browser.opens.is_empty() {
            return Err("Multiple native-file Open lost a selected file".into());
        }
        a.begin_file_browser(Mode::Open)?;
        click(&mut a, &mut t, "refresh")?;
        wait(&mut a, &mut t)?;
        click(&mut a, &mut t, "favorite")?;
        if Settings::load(a.storage.as_ref().unwrap())?.favorites
            != vec![dir.canonicalize().map_err(|e| e.to_string())?]
        {
            return Err("Favorite not saved to actual local path".into());
        }
        row(&mut a, &mut t, &selected)?;
        a.file_browser.name = "renamed.drawer".into();
        click(&mut a, &mut t, "rename")?;
        if a.file_browser.ticket.is_none() || !source.exists() {
            return Err(format!(
                "Rename review changed target or failed: {}",
                a.file_browser.message
            ));
        }
        click(&mut a, &mut t, "confirm-operation")?;
        wait(&mut a, &mut t)?;
        let renamed = dir.join("renamed.drawer");
        if source.exists()
            || std::fs::read(&renamed).map_err(|e| e.to_string())? != original
            || a.documents[0].path.as_ref()
                != Some(&renamed.canonicalize().map_err(|e| e.to_string())?)
            || !a.recent.paths().iter().any(|p| core::same(p, &renamed))
        {
            return Err(format!(
                "Rename/open/recent paths not reconciled: {}",
                a.file_browser.message
            ));
        }
        let image_path = image.canonicalize().map_err(|e| e.to_string())?;
        row(&mut a, &mut t, &image_path)?;
        click(&mut a, &mut t, "thumbnails")?;
        wait(&mut a, &mut t)?;
        if !a
            .file_browser
            .thumbs
            .get(&image_path)
            .is_some_and(|t| t.image.is_some())
            || std::fs::read(&image).map_err(|e| e.to_string())? != png
        {
            return Err("Thumbnail view missing or changed image".into());
        }
        a.file_browser.draft.view = View::Details;
        frame(&mut a, &mut t, vec![]);
        let renamed_path = renamed.canonicalize().map_err(|e| e.to_string())?;
        row(&mut a, &mut t, &renamed_path)?;
        click(&mut a, &mut t, "recycle")?;
        click(&mut a, &mut t, "cancel-operation")?;
        if !renamed.exists() {
            return Err("Canceled recycle changed source".into());
        }
        a.file_browser.recycle = simulated_recycle;
        click(&mut a, &mut t, "recycle")?;
        click(&mut a, &mut t, "confirm-operation")?;
        wait(&mut a, &mut t)?;
        if renamed.exists()
            || a.documents[0].path.is_some()
            || !a.documents[0].dirty
            || a.documents[0].snapshot() != before
            || a.documents[0].history_counts() != history
            || std::fs::read(dir.join("simulated-bin/renamed.drawer")).map_err(|e| e.to_string())?
                != original
        {
            return Err(format!(
                "Confirmed recycle lost open drawing/changed history: {}",
                a.file_browser.message
            ));
        }
        a.file_browser.address = dir.join("unplugged-folder").display().to_string();
        click(&mut a, &mut t, "go")?;
        wait(&mut a, &mut t)?;
        if a.file_browser.listing.is_some() {
            return Err("Unavailable path showed stale folder targets".into());
        }
        click(&mut a, &mut t, "close")?;
        let saved = Settings::load(a.storage.as_ref().unwrap())?;
        let restarted = Drawer::new(&ctx, None)
            .with_storage(UserStorage::at(dir.join("data")).map_err(|e| e.to_string())?);
        if restarted.file_browser.draft != saved || restarted.file_browser.job.is_some() {
            return Err("Browser restart opened paths implicitly or lost preferences".into());
        }
        Ok(())
    })();
    drawer_core::asset_catalog::cleanup_verification(root, &dir)?;
    result
}
