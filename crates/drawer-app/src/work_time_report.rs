use super::*;
use drawer_core::work_time::Record;
use std::{collections::BTreeSet, io::Write, path::Path};
const LIMIT: usize = 8 * 1024 * 1024;
fn text(s: &str) -> String {
    if s.trim_start()
        .chars()
        .next()
        .is_some_and(|c| matches!(c, '=' | '+' | '-' | '@'))
        || s.starts_with(['\t', '\r', '\n'])
    {
        format!("'{s}")
    } else {
        s.into()
    }
}
fn quote(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}
fn encode(records: &[Record]) -> Result<Vec<u8>, String> {
    if records.is_empty() || records.len() > 256 {
        return Err("Choose1..256 distinct open documents for the CSV report".into());
    }
    let mut seen = BTreeSet::new();
    let mut csv = String::from(
        "\u{feff}document_id,title,native_path,milliseconds,seconds,paused,sequence,undoable_resets\r\n",
    );
    for r in records {
        r.encode()?;
        if !seen.insert(&r.snapshot.id) {
            return Err("Duplicate work identity in report".into());
        }
        let s = &r.snapshot;
        let fields = [
            format!("drawer:{}", s.id),
            text(&r.title),
            text(r.origin.as_deref().unwrap_or("")),
            s.milliseconds.to_string(),
            format!("{}.{:03}", s.milliseconds / 1000, s.milliseconds % 1000),
            s.paused.to_string(),
            s.sequence.to_string(),
            s.reset_undo.len().to_string(),
        ];
        csv.push_str(
            &fields
                .iter()
                .map(|s| quote(s))
                .collect::<Vec<_>>()
                .join(","),
        );
        csv.push_str("\r\n");
        if csv.len() > LIMIT {
            return Err("Time report exceeds8MiB".into());
        }
    }
    Ok(csv.into_bytes())
}
fn same_destination(source: &Path, target: &Path) -> bool {
    if !drawer_core::font_library::local(source) {
        return false;
    }
    let resolved = source.canonicalize().ok().or_else(|| {
        Some(
            source
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or(Path::new("."))
                .canonicalize()
                .ok()?
                .join(source.file_name()?),
        )
    });
    resolved.is_some_and(|p| {
        if cfg!(windows) {
            p.to_string_lossy().to_lowercase() == target.to_string_lossy().to_lowercase()
        } else {
            p == target
        }
    })
}
struct Private {
    root: PathBuf,
    created: bool,
}
impl Drop for Private {
    fn drop(&mut self) {
        if self.created {
            let _ = std::fs::remove_file(self.root.join("report.csv"));
        }
        // An unexpected file in this exclusively created folder is never recursively removed.
        let _ = std::fs::remove_dir(&self.root);
    }
}
fn save(
    bytes: &[u8],
    path: &Path,
    publish: &mut impl FnMut(&Path, &Path) -> Result<(), String>,
) -> Result<(), String> {
    if bytes.len() > LIMIT || !drawer_core::font_library::local(path) {
        return Err("Choose a local report filename within the8MiB limit".into());
    }
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
        .canonicalize()
        .map_err(|e| e.to_string())?;
    if !drawer_core::font_library::local(&parent) {
        return Err("Choose a local report folder".into());
    }
    let target = parent.join(path.file_name().ok_or("Choose a report filename")?);
    if std::fs::symlink_metadata(&target).is_ok() {
        return Err("Choose a new CSV filename; existing files are preserved".into());
    }
    let root = parent.join(format!(
        ".Drawer-work-report-{}",
        drawer_core::work_time::identity()
    ));
    std::fs::create_dir(&root).map_err(|e| e.to_string())?;
    let mut stage = Private {
        root,
        created: false,
    };
    let source = stage.root.join("report.csv");
    let mut f = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&source)
        .map_err(|e| e.to_string())?;
    stage.created = true;
    f.write_all(bytes)
        .and_then(|_| f.sync_all())
        .map_err(|e| e.to_string())?;
    drop(f);
    if std::fs::read(&source).map_err(|e| e.to_string())? != bytes {
        return Err("CSV verification failed; no report published".into());
    }
    publish(&source, &target)
}
pub(super) struct Report {
    bytes: Vec<u8>,
    count: usize,
    path: String,
    notice: String,
    done: bool,
    pub write: Option<egui::Rect>,
}
impl Drawer {
    pub(super) fn prepare_work_time_report(&mut self, all: bool) -> Result<(), String> {
        let records = self.work_time_records(all)?;
        let bytes = encode(&records)?;
        self.work_time.report = Some(Report {
            bytes,
            count: records.len(),
            path: String::new(),
            notice: String::new(),
            done: false,
            write: None,
        });
        Ok(())
    }
    fn write_work_time_report(&mut self) -> Result<(), String> {
        let r = self
            .work_time
            .report
            .as_ref()
            .ok_or("Prepare a report first")?;
        let path = Path::new(r.path.trim());
        if !path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("csv"))
        {
            return Err("Choose a .csv report filename".into());
        }
        if !drawer_core::font_library::local(path) {
            return Err("Choose a local CSV path".into());
        }
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."))
            .canonicalize()
            .map_err(|e| e.to_string())?;
        let target = parent.join(path.file_name().ok_or("Choose a CSV filename")?);
        if self
            .protected_sources
            .iter()
            .any(|p| same_destination(p, &target))
            || self.documents.iter().any(|d| {
                d.path
                    .iter()
                    .chain(d.source_path.iter())
                    .any(|p| same_destination(p, &target))
            })
        {
            return Err("Choose another filename to preserve the drawing and source files".into());
        }
        if self
            .storage
            .as_ref()
            .and_then(|s| s.root().join("work-time").canonicalize().ok())
            .is_some_and(|p| parent.starts_with(p))
        {
            return Err("Write the report outside the local time journal folder".into());
        }
        save(&r.bytes, &target, &mut |a, b| {
            drawer_platform::local_files::publish_no_replace(a, b).map_err(|e| e.to_string())
        })?;
        let r = self.work_time.report.as_mut().unwrap();
        r.done = true;
        r.notice = format!(
            "Saved {} frozen document records to {}",
            r.count,
            target.display()
        );
        Ok(())
    }
    pub(super) fn work_time_report_window(&mut self, ctx: &egui::Context) {
        let Some(r) = &mut self.work_time.report else {
            return;
        };
        let mut open = true;
        let mut write = false;
        egui::Window::new("Frozen work-time CSV report").default_pos(egui::pos2(45.,35.)).default_width(650.).max_height(500.).vscroll(true).open(&mut open).show(ctx,|ui|{
            ui.label(format!("{} distinct documents · {} bytes · elapsed values captured at preparation",r.count,r.bytes.len()));
            ui.label("UTF-8 CSV with exact integer milliseconds and decimal seconds. Duplicate views appear once. Later time updates, resets, document edits and switches do not change this report. Drawing state and Undo are preserved.");
            ui.label("Columns: document_id, title, native_path, milliseconds, seconds, paused, sequence, undoable_resets");
            ui.label("IDs use a drawer: prefix; formula-like titles/paths receive a leading apostrophe for spreadsheet text. JSON keeps the original text.");
            ui.label("Choose a new local CSV filename. Existing files are preserved.");ui.text_edit_singleline(&mut r.path);
            ui.horizontal(|ui|{if ui.button("Choose CSV filename…").clicked() && let Some(p)=rfd::FileDialog::new().set_title("Write frozen work-time CSV").add_filter("CSV report",&["csv"]).set_file_name("work-time.csv").save_file(){r.path=p.to_string_lossy().into_owned();}
                let b=ui.add_enabled(!r.done&&!r.path.trim().is_empty(),egui::Button::new("Write frozen CSV"));r.write=Some(b.rect);if b.clicked(){write=true;}
            });ui.label(&r.notice);
        });
        if !open {
            self.work_time.report = None;
        } else if write && let Err(e) = self.write_work_time_report() {
            self.work_time.report.as_mut().unwrap().notice = e;
        }
    }
}

pub(super) fn smoke(root: &Path) -> Result<(), String> {
    let dir = root.join("work-time-report-ui");
    std::fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let result = (|| {
        let ctx = egui::Context::default();
        let mut app = Drawer::new(&ctx, None);
        app.simple_view = true;
        app.create_canvas(CanvasSpec {
            width: 8,
            height: 8,
            dpi: 300.,
            background: Background::Transparent,
        })
        .map_err(|e| e.to_string())?;
        let mut time = 0.;
        let frame = |app: &mut Drawer, t: &mut f64, events| {
            *t += 0.05;
            input_replay::frame(&ctx, app, *t, events);
        };
        let click = |app: &mut Drawer, t: &mut f64, r: egui::Rect| {
            for pressed in [true, false] {
                frame(
                    app,
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
            frame(app, t, vec![]);
        };
        app.execute_command(&ctx, "work-time");
        for _ in 0..3 {
            frame(&mut app, &mut time, vec![]);
        }
        let r = app
            .work_time
            .report_current
            .ok_or("Current CSV capture control missing")?;
        click(&mut app, &mut time, r);
        let frozen = app
            .work_time
            .report
            .as_ref()
            .ok_or("Pointer did not prepare current CSV")?
            .bytes
            .clone();
        if app.work_time.report.as_ref().unwrap().count != 1 {
            return Err("Current CSV included extra documents".into());
        }
        app.documents[0].title = "Later name".into();
        app.documents[0].edit_pixel(0, 1, 1, [255, 0, 0, 255]);
        let before = app.documents[0].snapshot();
        let counts = app.documents[0].history_counts();
        let dirty = app.documents[0].dirty;
        let output = dir.join("report.csv");
        app.work_time.report.as_mut().unwrap().path = output.to_string_lossy().into_owned();
        for _ in 0..2 {
            frame(&mut app, &mut time, vec![]);
        }
        let r = app
            .work_time
            .report
            .as_ref()
            .unwrap()
            .write
            .ok_or("CSV write control missing")?;
        click(&mut app, &mut time, r);
        if std::fs::read(&output).map_err(|e| e.to_string())? != frozen
            || app.documents[0].snapshot() != before
            || app.documents[0].history_counts() != counts
            || app.documents[0].dirty != dirty
        {
            return Err("CSV was not frozen or changed drawing".into());
        }
        app.work_time.report = None;
        app.create_canvas(CanvasSpec {
            width: 8,
            height: 8,
            dpi: 300.,
            background: Background::Transparent,
        })
        .map_err(|e| e.to_string())?;
        app.duplicate_view();
        app.prepare_work_time_report(true)?;
        if app.work_time.report.as_ref().unwrap().count != 2 {
            return Err("Multiple views duplicated CSV document rows".into());
        }
        let preserved = dir.join("preserve.csv");
        std::fs::write(&preserved, b"original").map_err(|e| e.to_string())?;
        app.work_time.report.as_mut().unwrap().path = preserved.to_string_lossy().into_owned();
        if app.write_work_time_report().is_ok() || std::fs::read(&preserved).unwrap() != b"original"
        {
            return Err("CSV overwrote existing file".into());
        }
        let source = dir.join("source.csv");
        app.documents[0].source_path = Some(source.clone());
        app.work_time.report.as_mut().unwrap().path = source.to_string_lossy().into_owned();
        if app.write_work_time_report().is_ok() || source.exists() {
            return Err("CSV source path protection failed".into());
        }
        app.work_time.report = None;
        app.prepare_work_time_report(true)?;
        if app.work_time.report.as_ref().unwrap().count != 2 {
            return Err("Report cancellation broke future captures".into());
        }
        app.work_time.report = None;
        if std::fs::read_dir(&dir).unwrap().any(|e| {
            e.unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".Drawer-work-report")
        }) {
            return Err("CSV private staging residue".into());
        }
        Ok(())
    })();
    let cleanup = std::fs::remove_dir_all(&dir).map_err(|e| e.to_string());
    result.and(cleanup)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_zero_time_unicode_quotes_newlines_spreadsheet_text_and_limits() {
        let mut r = Record::new(
            drawer_core::work_time::Snapshot::new("1".repeat(64)),
            "=한글,\"이름\"\n둘".into(),
            Some(" C:/로컬,원본.drawer".into()),
        );
        let b = encode(&[r.clone()]).unwrap();
        let s = String::from_utf8(b).unwrap();
        assert!(s.starts_with('\u{feff}'));
        assert!(s.contains("\"drawer:"));
        assert!(s.contains("\"'=한글,\"\"이름\"\"\n둘\""));
        assert!(s.contains("\"0\",\"0.000\",\"false\""));
        r.snapshot.add(1234).unwrap();
        let s = String::from_utf8(encode(&[r.clone()]).unwrap()).unwrap();
        assert!(s.contains("\"1234\",\"1.234\""));
        r.snapshot.reset().unwrap();
        let s = String::from_utf8(encode(&[r.clone()]).unwrap()).unwrap();
        assert!(s.contains("\"0\",\"0.000\""));
        assert!(s.ends_with("\"1\"\r\n"));
        assert!(encode(&[]).is_err());
        assert!(encode(&[r.clone(), r]).is_err());
        assert_eq!(text("@SUM(1)"), "'@SUM(1)");
        assert_eq!(text(" plain"), " plain");
    }
    #[test]
    fn failure_racing_destination_and_private_cleanup_preserve_existing_files() {
        let t = crate::tests::TestDirectory::new();
        let output = t.0.join("time.csv");
        assert!(
            save(b"data", &output, &mut |_, _| Err(
                "injected disk failure".into()
            ))
            .is_err()
        );
        assert!(!output.exists());
        assert!(
            save(b"data", &output, &mut |a, b| {
                std::fs::write(b, b"racing original").map_err(|e| e.to_string())?;
                drawer_platform::local_files::publish_no_replace(a, b).map_err(|e| e.to_string())
            })
            .is_err()
        );
        assert_eq!(std::fs::read(&output).unwrap(), b"racing original");
        assert_eq!(std::fs::read_dir(&t.0).unwrap().count(), 1);
    }
    #[test]
    fn pointer_frozen_current_open_documents_source_protection_and_cancel() {
        let t = crate::tests::TestDirectory::new();
        smoke(&t.0).unwrap();
    }
}
