use super::*;
pub(super) fn run(root: &Path) -> Result<(), String> {
    drawer_core::layer_output::verify::run(root)?;
    let dir = root.join("layer-output-ui");
    std::fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let result = (|| {
        let ctx = egui::Context::default();
        let mut app = Drawer::new(&ctx, None);
        app.simple_view = true;
        app.add_document(drawer_core::layer_output::verify::fixture()?);
        let before = app.documents[0].snapshot();
        let identity = (
            app.documents[0].history_counts(),
            app.documents[0].dirty,
            app.documents[0].revision,
        );
        app.begin_layer_output(drawer_core::layer_output::Mode::Comps)?;
        app.output_batch.folder = dir.display().to_string();
        let mut t = 0.;
        let frame = |a: &mut Drawer, t: &mut f64, e| {
            *t += 0.05;
            input_replay::frame(&ctx, a, *t, e);
        };
        let click = |a: &mut Drawer, t: &mut f64, key: &str| -> Result<(), String> {
            for _ in 0..3 {
                frame(a, t, vec![]);
            }
            let pos = a
                .output_batch
                .controls
                .get(key)
                .ok_or("Missing layer-output control")?
                .center();
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
        click(&mut app, &mut t, "prepare")?;
        for _ in 0..500 {
            frame(&mut app, &mut t, vec![]);
            if app.output_batch.job.is_none() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        let plan = app
            .output_batch
            .plan
            .as_ref()
            .ok_or_else(|| format!("Comp UI plan failed: {}", app.output_batch.message))?;
        if plan.files.len() != 2 || plan.files[0].bytes == plan.files[1].bytes {
            return Err("Comp output plans do not reflect visibility".into());
        }
        let names = plan
            .files
            .iter()
            .map(|f| f.path.clone())
            .collect::<Vec<_>>();
        app.output_batch.reviewed = true;
        click(&mut app, &mut t, "write")?;
        for _ in 0..500 {
            frame(&mut app, &mut t, vec![]);
            if app.output_batch.job.is_none() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        if names.iter().any(|p| !p.is_file()) {
            return Err(format!(
                "Comp pointer publication failed: {}",
                app.output_batch.message
            ));
        }
        click(&mut app, &mut t, "close")?;
        app.begin_layer_output(drawer_core::layer_output::Mode::Layers)?;
        app.output_batch.folder = dir.display().to_string();
        app.output_batch.include_hidden = true;
        click(&mut app, &mut t, "prepare")?;
        for _ in 0..500 {
            frame(&mut app, &mut t, vec![]);
            if app.output_batch.job.is_none() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        let paths = app
            .output_batch
            .plan
            .as_ref()
            .ok_or("Hidden layer UI preparation failed")?
            .files
            .iter()
            .map(|f| f.path.clone())
            .collect::<Vec<_>>();
        click(&mut app, &mut t, "close")?;
        if paths.iter().any(|p| p.exists())
            || app.documents[0].snapshot() != before
            || identity
                != (
                    app.documents[0].history_counts(),
                    app.documents[0].dirty,
                    app.documents[0].revision,
                )
        {
            return Err("Comp/layer cancel changed visibility or wrote files".into());
        }
        Ok(())
    })();
    result.and(drawer_core::asset_catalog::cleanup_verification(root, &dir))
}
#[cfg(test)]
mod tests {
    #[test]
    fn comp_and_hidden_layer_actual_pointer_output_cancel_visibility() {
        let t = crate::tests::TestDirectory::new();
        super::run(&t.0).unwrap();
    }
}
