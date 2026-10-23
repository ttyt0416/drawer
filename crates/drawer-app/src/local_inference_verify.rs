use super::*;
fn frame(ctx: &egui::Context, app: &mut Drawer, time: &mut f64, events: Vec<egui::Event>) {
    *time += 0.1;
    crate::input_replay::frame(ctx, app, *time, events);
}
fn settle(ctx: &egui::Context, app: &mut Drawer, time: &mut f64) {
    for _ in 0..3 {
        frame(ctx, app, time, vec![]);
    }
}
fn click(ctx: &egui::Context, app: &mut Drawer, time: &mut f64, rect: egui::Rect) {
    for pressed in [true, false] {
        frame(
            ctx,
            app,
            time,
            vec![
                egui::Event::PointerMoved(rect.center()),
                egui::Event::PointerButton {
                    pos: rect.center(),
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: Default::default(),
                },
            ],
        );
    }
}
fn wait(ctx: &egui::Context, app: &mut Drawer, time: &mut f64) -> Result<(), String> {
    let end = std::time::Instant::now() + Duration::from_secs(30);
    while app.local_inference.job.is_some() {
        if std::time::Instant::now() > end {
            return Err("Local inference wait timed out".into());
        }
        std::thread::sleep(Duration::from_millis(20));
        frame(ctx, app, time, vec![]);
    }
    settle(ctx, app, time);
    Ok(())
}
fn check(value: bool, message: &str) -> Result<(), String> {
    if value { Ok(()) } else { Err(message.into()) }
}
pub(super) fn smoke(root: &Path) -> Result<(), String> {
    let ctx = egui::Context::default();
    let mut app = Box::new(Drawer::new(&ctx, None));
    app.simple_view = true;
    app.storage = Some(UserStorage::at(root.join("inference-user")).map_err(|e| e.to_string())?);
    app.create_canvas(CanvasSpec {
        width: 2,
        height: 2,
        dpi: 300.,
        background: Background::Transparent,
    })
    .map_err(|e| e.to_string())?;
    app.documents[0].layers[0].pixels = drawer_infer::fixtures::input().pixels;
    let inputpath = root.join("inference-input.png");
    drawer_core::png_export::export(&app.documents[0], &inputpath, DEFAULT_DOCUMENT_BUDGET)
        .map_err(|e| e.to_string())?;
    let hintpath = root.join("inference-hints.png");
    let hintdoc = drawer_infer::fixtures::hints()
        .document(DEFAULT_DOCUMENT_BUDGET)
        .map_err(|e| e.to_string())?;
    drawer_core::png_export::export(&hintdoc, &hintpath, DEFAULT_DOCUMENT_BUDGET)
        .map_err(|e| e.to_string())?;
    let inputbytes = std::fs::read(&inputpath).map_err(|e| e.to_string())?;
    let original = app.documents[0].snapshot();
    let mut time = 0.;
    settle(&ctx, &mut app, &mut time);
    for task in Task::ALL {
        app.execute_command(&ctx, "local-inference");
        let path = drawer_infer::fixtures::bundle(&root.join(format!("inference-{task:?}")), task)?;
        let e = app
            .local_inference
            .draft
            .as_mut()
            .ok_or("Local inference command unavailable")?;
        e.config.task = task;
        e.config.manifest = path.to_string_lossy().into_owned();
        e.config.hints = hintpath.to_string_lossy().into_owned();
        if task == Task::Hand {
            e.config.source = InputSource::File;
            e.config.path = inputpath.to_string_lossy().into_owned();
        }
        if task == Task::Denoise {
            e.config.source = InputSource::Selection;
            app.documents[0].scene.selection = Some(drawer_core::scene::Rect {
                min: [0., 0.],
                max: [1., 2.],
            });
        }
        settle(&ctx, &mut app, &mut time);
        let before = app.documents[0].snapshot();
        let counts = app.documents[0].history_counts();
        let r = app.local_inference.input.ok_or("Input button missing")?;
        click(&ctx, &mut app, &mut time, r);
        wait(&ctx, &mut app, &mut time)?;
        check(
            app.local_inference
                .draft
                .as_ref()
                .is_some_and(|e| e.input.is_some()),
            &format!("Input preview failed: {:?}", app.status),
        )?;
        if task == Task::Hand {
            let cfg = app.local_inference.draft.as_ref().unwrap().config.clone();
            app.local_inference.draft.as_mut().unwrap().config.page = 2;
            check(
                app.start_inference(&ctx, true).is_err(),
                "Changed input settings reused a frozen input",
            )?;
            app.local_inference.draft.as_mut().unwrap().config = cfg;
        }
        let frozen = app
            .local_inference
            .draft
            .as_ref()
            .unwrap()
            .input
            .as_ref()
            .unwrap()
            .image
            .clone();
        let r = app.local_inference.run.ok_or("Inference button missing")?;
        click(&ctx, &mut app, &mut time, r);
        wait(&ctx, &mut app, &mut time)?;
        check(
            app.local_inference
                .draft
                .as_ref()
                .is_some_and(|e| e.prepared.is_some() && e.result.is_some()),
            &format!("Local model preview failed ({task:?}): {:?}", app.status),
        )?;
        check(
            app.documents[0].snapshot() == before && app.documents[0].history_counts() == counts,
            "Inference preview mutated original/history",
        )?;
        if task == Task::Body {
            let cfg = app.local_inference.draft.as_ref().unwrap().config.clone();
            app.local_inference.draft.as_mut().unwrap().config.task = Task::Hand;
            check(
                app.adopt_inference().is_err(),
                "Changed adapter reused a prepared result",
            )?;
            app.local_inference.draft.as_mut().unwrap().config = cfg;
        }
        let pending = root.join("inference-pending.drawer");
        check(
            app.save_to(0, &pending).is_err() && !pending.exists(),
            "Unconfirmed inference was saved",
        )?;
        if task.joints() > 0 {
            check(
                app.adopt_inference().is_err(),
                "Uncertain pose adopted without consent",
            )?;
            app.local_inference.draft.as_mut().unwrap().accept_low = true;
        }
        settle(&ctx, &mut app, &mut time);
        let r = app.local_inference.apply.ok_or("Adopt button missing")?;
        click(&ctx, &mut app, &mut time, r);
        check(
            app.local_inference.draft.is_none(),
            &format!("Adoption failed: {:?}", app.status),
        )?;
        check(
            app.documents[0].history_counts().0 == counts.0 + 1
                && app.documents[0].layers[..before.layers.len()] == before.layers,
            "Adoption must be one Undo and preserve source pixels",
        )?;
        if task.joints() > 0 {
            check(
                app.documents[0]
                    .scene
                    .lines
                    .layers
                    .last()
                    .unwrap()
                    .paths
                    .iter()
                    .any(|p| p.color == [255, 160, 0, 255]),
                "Uncertain joints lost their warning color",
            )?;
        }
        let native = root.join(format!("inference-{task:?}.drawer"));
        app.save_to(0, &native)?;
        check(
            drawer_core::files::open(&native, DEFAULT_DOCUMENT_BUDGET)
                .map_err(|e| e.to_string())?
                .snapshot()
                == app.documents[0].snapshot(),
            "Inference result native mismatch",
        )?;
        check(
            app.documents[0].undo() && app.documents[0].snapshot() == before,
            "Inference Undo mismatch",
        )?;
        check(
            frozen.width == 2 && frozen.height == 2,
            "Frozen source changed size",
        )?;
        app.documents[0].scene.selection = None;
    }
    check(
        app.documents[0].snapshot() == original
            && std::fs::read(&inputpath).map_err(|e| e.to_string())? == inputbytes,
        "Source file/page changed",
    )?;
    // The same user-selected model can be compared against fixed owned samples.
    // Comparison previews deliberately have no document adoption state.
    for task in Task::ALL {
        app.begin_local_inference()?;
        let manifest =
            drawer_infer::fixtures::bundle(&root.join(format!("comparison-{task:?}")), task)?;
        let e = app.local_inference.draft.as_mut().unwrap();
        e.config.task = task;
        e.config.source = InputSource::Comparison;
        e.config.manifest = manifest.to_string_lossy().into_owned();
        app.start_inference(&ctx, false)?;
        wait(&ctx, &mut app, &mut time)?;
        app.start_inference(&ctx, true)?;
        wait(&ctx, &mut app, &mut time)?;
        let e = app.local_inference.draft.as_ref().unwrap();
        check(
            e.result
                .as_ref()
                .is_some_and(|r| r.report.contains("Fixed comparison set v1"))
                && e.prepared.is_none(),
            &format!("Comparison metrics missing: {:?}", app.status),
        )?;
        check(
            app.adopt_inference().is_err() && app.documents[0].snapshot() == original,
            "Comparison sample could alter the document",
        )?;
        app.local_inference.draft = None;
    }
    // Missing model is a guide, never a simulated inference result.
    app.begin_local_inference()?;
    app.start_inference(&ctx, false)?;
    wait(&ctx, &mut app, &mut time)?;
    app.start_inference(&ctx, true)?;
    wait(&ctx, &mut app, &mut time)?;
    check(
        app.status.as_ref().is_some_and(|s| s.contains("Install"))
            && app
                .local_inference
                .draft
                .as_ref()
                .unwrap()
                .prepared
                .is_none(),
        "Missing model did not show installation guidance",
    )?;
    let manifest = drawer_infer::fixtures::bundle(&root.join("inference-final"), Task::Body)?;
    app.local_inference.draft.as_mut().unwrap().config.manifest =
        manifest.to_string_lossy().into_owned();
    app.start_inference(&ctx, false)?;
    wait(&ctx, &mut app, &mut time)?;
    // A revision/frame change during inference cannot apply the frozen source to a different page state.
    app.start_inference(&ctx, true)?;
    app.documents[0].revision += 1;
    wait(&ctx, &mut app, &mut time)?;
    check(
        app.local_inference
            .draft
            .as_ref()
            .unwrap()
            .prepared
            .is_none()
            && app.adopt_inference().is_err(),
        "Stale model result was retained",
    )?;
    app.local_inference.draft = None;
    app.begin_local_inference()?;
    app.local_inference.draft.as_mut().unwrap().config.source = InputSource::File;
    app.local_inference.draft.as_mut().unwrap().config.path =
        root.join("absent.png").to_string_lossy().into_owned();
    app.start_inference(&ctx, false)?;
    wait(&ctx, &mut app, &mut time)?;
    check(
        app.local_inference.draft.as_ref().unwrap().input.is_none(),
        "Read failure reused an old input",
    )?;
    app.local_inference.draft = None;
    app.begin_local_inference()?;
    app.start_inference(&ctx, false)?;
    frame(
        &ctx,
        &mut app,
        &mut time,
        vec![egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Default::default(),
        }],
    );
    wait(&ctx, &mut app, &mut time)?;
    check(
        app.local_inference.draft.is_none() && app.documents[0].snapshot() == original,
        "Escape failed to discard the inference draft",
    )?;
    // Exercise the installed child-process gate, timeout, cancellation and scratch cleanup.
    let scratch = root.join("inference-scratch");
    let request = drawer_infer::Request {
        manifest: manifest.to_string_lossy().into_owned(),
        task: Task::Body,
    };
    let c = AtomicBool::new(false);
    check(
        drawer_infer::execute(
            &scratch,
            &executable()?,
            &request,
            &drawer_infer::fixtures::input(),
            None,
            &c,
            Duration::ZERO,
        )
        .is_err(),
        "Inference timeout failed",
    )?;
    c.store(true, Ordering::Relaxed);
    check(
        drawer_infer::execute(
            &scratch,
            &executable()?,
            &request,
            &drawer_infer::fixtures::input(),
            None,
            &c,
            Duration::from_secs(1),
        )
        .is_err(),
        "Cancelled inference ran",
    )?;
    check(
        std::fs::read_dir(&scratch)
            .map_err(|e| e.to_string())?
            .next()
            .is_none(),
        "Inference scratch files leaked",
    )?;
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn actual_local_model_inputs_onnx_preview_adoption_undo_stale_failure_cancel_and_cleanup() {
        let root = crate::tests::TestDirectory::new();
        super::smoke(&root.0).unwrap();
    }
}
