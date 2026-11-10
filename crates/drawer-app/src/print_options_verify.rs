use super::*;
pub(super) fn run(root: &Path) -> Result<(), String> {
    drawer_core::print_options::verify::run(root)?;
    let dir = root.join("print-options-ui");
    std::fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let result = (|| {
        let ctx = egui::Context::default();
        let mut a = Drawer::new(&ctx, None)
            .with_storage(UserStorage::at(dir.join("data")).map_err(|e| e.to_string())?);
        a.simple_view = true;
        let mut d = drawer_core::layer_roles_verify::page()?;
        d.layers[0].pixels.fill([240, 20, 10, 255]);
        let state = drawer_core::layer_roles::prepare(
            &d,
            drawer_core::layer_search::Key::Layer(drawer_core::layer_properties::Target::Raster(1)),
            drawer_core::layer_roles::Flags {
                draft: true,
                ..Default::default()
            },
            None,
        )?;
        d.commit_state(state)?;
        let before = d.snapshot();
        let history = d.history_counts();
        a.add_document(d);
        a.begin_printing()?;
        let mut t = 0.;
        let frame = |a: &mut Drawer, t: &mut f64, events| {
            *t += 0.05;
            input_replay::frame(&ctx, a, *t, events);
        };
        let wait = |a: &mut Drawer, t: &mut f64| -> Result<(), String> {
            for _ in 0..8000 {
                frame(a, t, vec![]);
                if a.printing.job.is_none() {
                    return if a.printing.pages.is_some() {
                        Ok(())
                    } else {
                        Err(a.printing.message.clone())
                    };
                }
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            Err("Print content timeout".into())
        };
        let click = |a: &mut Drawer, t: &mut f64, key: &str| -> Result<(), String> {
            for _ in 0..3 {
                frame(a, t, vec![]);
            }
            let mut pos = egui::Pos2::ZERO;
            for attempt in 0..12 {
                pos = a
                    .printing
                    .controls
                    .get(key)
                    .ok_or_else(|| format!("Missing print option {key}"))?
                    .center();
                let visible = a.printing.controls["viewport"].shrink(8.);
                if visible.contains(pos) {
                    break;
                }
                if attempt == 11 {
                    return Err(format!("Print option {key} inaccessible at {pos:?}"));
                }
                frame(
                    a,
                    t,
                    vec![
                        egui::Event::PointerMoved(egui::pos2(
                            visible.left() + 2.,
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
        wait(&mut a, &mut t)?;
        if a.printing.pages.as_ref().unwrap()[0]
            .image
            .pixels
            .iter()
            .any(|p| p[3] != 0)
        {
            return Err("Draft was printed by default".into());
        }
        click(&mut a, &mut t, "advanced")?;
        click(&mut a, &mut t, "drafts")?;
        wait(&mut a, &mut t)?;
        if !a.printing.pages.as_ref().unwrap()[0]
            .image
            .pixels
            .iter()
            .any(|p| p[3] > 0)
        {
            return Err("Draft inclusion pointer/recomposition".into());
        }
        click(&mut a, &mut t, "red")?;
        click(&mut a, &mut t, "registration")?;
        a.printing.measured_mm = 79.375;
        a.printing.calibration_label = "Display".into();
        click(&mut a, &mut t, "calibrate")?;
        click(&mut a, &mut t, "print-size")?;
        let expected = a
            .printing
            .draft
            .output
            .calibration
            .as_ref()
            .ok_or_else(|| {
                format!(
                    "Calibration button: {}; {:?}",
                    a.printing.message, a.printing.controls
                )
            })?
            .zoom(a.documents[0].dpi)?;
        if (a.documents[0].view.zoom - expected).abs() > 1e-9
            || a.documents[0].snapshot() != before
            || a.documents[0].history_counts() != history
        {
            return Err("Physical-size view changed drawing or missed calibration".into());
        }
        click(&mut a, &mut t, "save")?;
        if Settings::load(a.storage.as_ref().unwrap())? != a.printing.draft {
            return Err("Extended print controls/preset save".into());
        }
        // File chooser callback supplies a local profile; review keeps its exact hash.
        let profile = dir.join("color.icc");
        let mut bytes = drawer_core::export_color::profile_bytes(
            &drawer_core::color_profile::RgbProfile::Srgb,
        )?;
        std::fs::write(&profile, &bytes).map_err(|e| e.to_string())?;
        a.printing.draft.output.profile = Some(profile.clone());
        Drawer::refresh_print_plan(&mut a.printing);
        for _ in 0..3 {
            frame(&mut a, &mut t, vec![]);
        }
        if a.printing.profile_stamp.is_none() {
            return Err("ICC preview fingerprint missing".into());
        }
        bytes[24..26].copy_from_slice(&2001u16.to_be_bytes());
        std::fs::write(&profile, &bytes).map_err(|e| e.to_string())?;
        let mut p = std::mem::take(&mut a.printing);
        p.reviewed = true;
        let output = dir.join("changed-icc.pdf");
        let failed = a.print_action(&mut p, Some(output.clone())).is_err();
        a.printing = p;
        if !failed || output.exists() || a.printing.job.is_some() || a.printing.reviewed {
            return Err("Changed ICC passed review or wrote a file".into());
        }
        a.printing.draft.output.profile = None;
        Drawer::refresh_print_plan(&mut a.printing);
        a.printing.open = false;
        drop(a);
        Ok(())
    })();
    drawer_core::asset_catalog::cleanup_verification(root, &dir)?;
    result
}
#[cfg(test)]
mod tests {
    #[test]
    fn print_content_channels_physical_view_preset_and_profile_review_pointer() {
        let t = crate::tests::TestDirectory::new();
        super::run(&t.0).unwrap();
    }
}
