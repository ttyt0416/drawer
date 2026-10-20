use super::*;
#[derive(Default)]
pub(super) struct Screen {
    #[cfg(windows)]
    job: Option<drawer_platform::ScreenSample>,
    table: Option<Option<u64>>,
    alpha: u8,
}
impl Screen {
    pub(super) fn cancel(&mut self) {
        #[cfg(windows)]
        {
            self.job = None;
        }
        self.table = None;
    }
}
impl Drawer {
    pub(super) fn begin_screen_sample(&mut self, ctx: &egui::Context) -> Result<(), String> {
        #[cfg(windows)]
        {
            if self.screen_sampler.job.is_some() {
                return Err("Screen sampler is already active".into());
            }
            self.drawing.cancel_sample();
            self.color_tables.cancel_drag();
            let native = self
                .native_mouse
                .as_ref()
                .ok_or("Native screen sampler is unavailable")?;
            let state = &self.workspaces.store.current.eyedropper;
            let repaint = ctx.clone();
            let job = native.screen_sample(
                state.settings.radius,
                state.hide_screen_window,
                std::sync::Arc::new(move || repaint.request_repaint()),
            )?;
            self.screen_sampler.alpha = self.references.session.workspace.color[3];
            self.screen_sampler.table = self
                .workspaces
                .store
                .current
                .color_tables
                .auto_sample
                .then_some(self.workspaces.store.current.color_tables.selected);
            self.screen_sampler.job = Some(job);
            self.status=Some("Display RGB: click to confirm; Escape/right-click cancels. Drawing alpha is preserved.".into());
            Ok(())
        }
        #[cfg(not(windows))]
        {
            let _ = ctx;
            Err("Screen sampler is available on Windows".into())
        }
    }
    pub(super) fn poll_screen_sample(&mut self) {
        #[cfg(windows)]
        if let Some(result) = self.screen_sampler.job.as_mut().and_then(|j| j.poll()) {
            self.screen_sampler.job = None;
            self.screen_sample_result(result);
        }
    }
    fn screen_sample_result(&mut self, result: Result<Option<[u8; 3]>, String>) {
        let table = self.screen_sampler.table.take();
        match result {
            Ok(Some(rgb)) => {
                let c = [rgb[0], rgb[1], rgb[2], self.screen_sampler.alpha];
                self.choose_color(c);
                self.status =
                    Some("Selected display RGB (drawing alpha kept); app window restored".into());
                if let Some(table) = table {
                    self.register_sample(table, c);
                }
            }
            Ok(None) => {
                self.status =
                    Some("Screen sampling cancelled; color preserved and window restored".into())
            }
            Err(e) => self.status = Some(format!("Screen sample failed; color preserved: {e}")),
        }
    }
    pub(super) fn sampling_reference_controls(&mut self, ui: &mut egui::Ui) {
        let Some(index) = self.active else {
            return;
        };
        let doc = &self.documents[index];
        let scope = drawer_core::eyedropper::scope(doc);
        let refs = &mut self.workspaces.store.current.eyedropper.references;
        ui.collapsing("Canvas composite reference / excluded layers",|ui|{
            if refs.scope.as_ref().is_some_and(|s|*s!=scope){
                ui.label("Stored references belong to another document; this canvas uses all visible layers.");
                if ui.button("Set references for this document").clicked(){*refs=drawer_core::eyedropper::References::default();refs.scope=Some(scope.clone());}
                return;
            }
            let before=refs.clone();
            ui.checkbox(&mut refs.enabled,"Use selected references (composite RGBA only)");
            if refs.enabled{
                ui.checkbox(&mut refs.all_rasters,"All visible raster layers");
                egui::ScrollArea::vertical().id_salt("sample-refs").max_height(150.0).show(ui,|ui|{
                    for (n,l) in doc.layers.iter().enumerate(){ui.push_id(n,|ui|{ui.horizontal_wrapped(|ui|{
                        ui.label(format!("{}: {}",n+1,l.name));
                        let mut include=refs.include.contains(&n);let mut exclude=refs.exclude.contains(&n);
                        if ui.add_enabled(!refs.all_rasters,egui::Checkbox::new(&mut include,"Reference")).changed(){refs.include.retain(|v|*v!=n);if include{refs.include.push(n);}}
                        if ui.checkbox(&mut exclude,"Exclude").changed(){refs.exclude.retain(|v|*v!=n);if exclude{refs.exclude.push(n);}}
                    });});}
                });
                ui.checkbox(&mut refs.objects,"Page objects and 3D preview");ui.checkbox(&mut refs.vectors,"Visible vector layers");ui.checkbox(&mut refs.animation,"Visible animation cells");
                if refs.include.iter().chain(&refs.exclude).any(|n|*n>=doc.layers.len())&&ui.button("Refresh changed layer list").clicked(){refs.include.retain(|n|*n<doc.layers.len());refs.exclude.retain(|n|*n<doc.layers.len());}
            }
            if *refs!=before{refs.scope=Some(scope);self.drawing.cancel_sample();}
            ui.label("Document color before display proofing; visible-layer effects apply. Exclusions override references. Empty explicit references sample no rasters.");
        });
    }
}
pub(super) fn smoke(root: &std::path::Path) -> Result<(), String> {
    drawer_core::screen_sample::verify()?;
    #[cfg(windows)]
    drawer_platform::verify_screen_boundary()?;
    let ctx = egui::Context::default();
    let user = root.join("screen-sampling-user");
    let mut app = Drawer::new(&ctx, None)
        .with_storage(UserStorage::at(user.clone()).map_err(|e| e.to_string())?);
    app.create_canvas(CanvasSpec {
        width: 3,
        height: 3,
        dpi: 300.0,
        background: Background::Transparent,
    })
    .map_err(|e| e.to_string())?;
    let doc = &mut app.documents[0];
    doc.layers[0].pixels = vec![[255, 0, 0, 128]; 9];
    doc.layers.push(drawer_core::document::RasterLayer {
        precise: None,
        locked: false,
        name: "Blue overlay".into(),
        pixels: vec![[0, 0, 255, 255]; 9],
        placement: None,
    });
    drawer_core::files::save(doc, &root.join("sampler-keeps-document.drawer"))
        .map_err(|e| e.to_string())?;
    let before = doc.snapshot();
    let refs = drawer_core::eyedropper::References {
        enabled: true,
        scope: Some(drawer_core::eyedropper::scope(doc)),
        exclude: vec![1],
        ..Default::default()
    };
    app.workspaces.store.current.eyedropper.references = refs;
    app.workspaces.store.current.eyedropper.settings.radius = 1;
    super::input_replay::frame(&ctx, &mut app, 0.0, vec![]);
    super::input_replay::frame(&ctx, &mut app, 0.1, vec![]);
    app.execute_command(&ctx, "Eyedropper");
    let events = vec![
        super::precision::pointer(
            &ctx,
            &app,
            [1.5, 1.5],
            drawer_core::pointer_input::Phase::Down,
        ),
        super::precision::pointer(
            &ctx,
            &app,
            [1.5, 1.5],
            drawer_core::pointer_input::Phase::Up,
        ),
    ];
    app.drawing.virtual_events(events, false);
    super::input_replay::frame(&ctx, &mut app, 0.2, vec![]);
    super::input_replay::frame(&ctx, &mut app, 0.3, vec![]);
    if app.references.session.workspace.color != [255, 0, 0, 128] {
        return Err("Canvas reference exclusion did not affect production sampler".into());
    }
    for mode in 0..3 {
        let color = app.references.session.workspace.color;
        app.screen_sampler.alpha = 128;
        app.screen_sample_result(match mode {
            0 => Ok(None),
            1 => Err("fixture failure".into()),
            _ => Ok(Some([20, 80, 160])),
        });
        if mode < 2 && app.references.session.workspace.color != color {
            return Err("Screen cancel/failure changed color".into());
        }
    }
    if app.references.session.workspace.color != [20, 80, 160, 128] {
        return Err("Screen result changed drawing alpha".into());
    }
    app.workspaces.store.current.eyedropper.hide_screen_window = false;
    for alpha in [0, 128, 255] {
        app.screen_sampler.alpha = alpha;
        app.screen_sampler.table = Some(app.workspaces.store.current.color_tables.selected);
        app.screen_sample_result(Ok(Some([21, 83, 177])));
        let c = [21, 83, 177, alpha];
        if app.references.session.workspace.color != c
            || !app.workspaces.store.current.color_tables.tables[0]
                .colors
                .iter()
                .any(|s| s.color == c)
        {
            return Err("Display alpha/recent/explicit table registration failed".into());
        }
    }
    let expected = app.workspaces.store.current.eyedropper.clone();
    app.persist_workspace()?;
    let restarted = Drawer::new(&egui::Context::default(), None)
        .with_storage(UserStorage::at(user).map_err(|e| e.to_string())?);
    if restarted.capture_workspace().eyedropper != expected {
        return Err("Sampling references/recent/settings restart mismatch".into());
    }
    if app.documents[0].snapshot() != before
        || app.documents[0].dirty
        || app.documents[0].history_counts() != (0, 0)
    {
        return Err("Sampling edited document/history".into());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn references_production_sampler_display_outcomes_and_restart() {
        let root = crate::tests::TestDirectory::new();
        super::smoke(&root.0).unwrap();
    }
}
