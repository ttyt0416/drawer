use super::*;
use drawer_core::snapping::{Modifier, Settings};
pub(super) fn disabled(ctx: &egui::Context, settings: Settings) -> bool {
    ctx.input(|i| match settings.modifier {
        Modifier::Control => i.modifiers.ctrl,
        Modifier::Shift => i.modifiers.shift,
        Modifier::Alt => i.modifiers.alt,
    })
}
impl Drawer {
    pub(super) fn snapping_controls(&mut self, ui: &mut egui::Ui) {
        ui.collapsing("Snapping priority and temporary disable",|ui|{
            let old=self.workspaces.store.current.snapping;let s=&mut self.workspaces.store.current.snapping;
            ui.checkbox(&mut s.rulers,"Active drawing ruler (first priority)");ui.checkbox(&mut s.guides,"Nearest page guide (second priority)");ui.checkbox(&mut s.grid,"Page grid intersections (after other targets)");
            ui.checkbox(&mut s.objects,"Object bounds and center lines");ui.checkbox(&mut s.control_points,"Vector control points / object anchors");ui.checkbox(&mut s.canvas_edges,"Canvas edges");
            ui.add(egui::Slider::new(&mut s.screen_distance,0.1..=128.0).text("Capture distance in screen pixels"));
            egui::ComboBox::from_id_salt("snap-disable-modifier").selected_text(format!("Hold {:?} to disable snapping",s.modifier)).show_ui(ui,|ui|{for m in Modifier::ALL{ui.selectable_value(&mut s.modifier,m,format!("{m:?}"));}});
            ui.label("Line/perspective rulers constrain before guide/grid capture. Equidistant guides use their page order. Symmetry copies remain enabled during the temporary modifier. Brush and shape inputs share this priority; shape grid-step snapping is a fallback. Releasing the modifier restores snapping without changing saved options.");
            if old!=*s{self.finish_stroke();}
        });
    }
}
fn frame(
    ctx: &egui::Context,
    app: &mut Drawer,
    time: &mut f64,
    ctrl: bool,
    focused: bool,
    mut events: Vec<egui::Event>,
) {
    *time += 0.1;
    events.insert(
        0,
        egui::Event::ModifiersChanged(egui::Modifiers {
            ctrl,
            ..Default::default()
        }),
    );
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1000.0, 700.0),
            )),
            time: Some(*time),
            focused,
            events,
            ..Default::default()
        },
        |ui| app.show(ui),
    );
    output.textures_delta.clear();
}
pub(super) fn smoke(root: &std::path::Path) -> Result<(), String> {
    drawer_core::snapping_verify::verify(root)?;
    use drawer_core::pointer_input::{Kind, Phase, Source};
    for (pen, virtual_pressure) in [(false, false), (false, true), (true, false)] {
        let ctx = egui::Context::default();
        let mut app = Drawer::new(&ctx, None);
        app.simple_view = true;
        let mut time = 0.0;
        app.create_canvas(CanvasSpec {
            width: 64,
            height: 48,
            dpi: 300.0,
            background: Background::Transparent,
        })
        .map_err(|e| e.to_string())?;
        app.set_tool(BrushTip::PixelPen);
        app.panels.settings.brush_size = 1.0;
        app.workspaces.store.current.pressure.tools[6].size = false;
        app.workspaces.store.current.pressure.tools[6].opacity = false;
        app.workspaces.store.current.input.profiles[6].minimum_size = 1.0;
        app.workspaces.store.current.input.profiles[6].minimum_opacity = 1.0;
        app.workspaces.store.current.input.profiles[6]
            .pixel
            .pressure = false;
        app.workspaces.store.current.input.mouse_pressure[6] =
            drawer_core::mouse_pressure::Settings {
                enabled: virtual_pressure,
                start_length: 0.0,
                end_length: 0.0,
                ..Default::default()
            };
        app.choose_color([200, 40, 20, 255]);
        app.documents[0].scene.rulers.add(
            "Snap priority",
            drawer_core::rulers::Kind::Straight {
                a: [0.0, 8.5],
                b: [64.0, 8.5],
            },
        )?;
        app.documents[0].scene.guides = vec![[[0.0, 12.5], [64.0, 12.5]]];
        app.documents[0].scene.grid.spacing = [8.0; 2];
        app.workspaces.store.current.snapping = Settings {
            guides: true,
            grid: true,
            ..Default::default()
        };
        app.documents[0].view.zoom = 4.0;
        app.documents[0].view.angle = 90.0;
        frame(&ctx, &mut app, &mut time, false, true, vec![]);
        frame(&ctx, &mut app, &mut time, false, true, vec![]);
        let settings = app.workspaces.store.current.snapping;
        let before = app.documents[0].snapshot();
        let event = |app: &Drawer, p, phase| {
            let mut e = super::precision::pointer(&ctx, app, p, phase);
            if pen {
                e.source = Source {
                    kind: Kind::Pen,
                    device: 11,
                    contact: 7,
                };
                e.pressure = Some(0.7);
            }
            e
        };
        for (p, phase, ctrl) in [
            ([4.5, 12.5], Phase::Down, false),
            ([12.5, 12.5], Phase::Move, true),
            ([20.5, 12.5], Phase::Move, false),
        ] {
            app.drawing.virtual_events(vec![event(&app, p, phase)], pen);
            frame(&ctx, &mut app, &mut time, ctrl, true, vec![]);
            if app.documents[0].snapshot() != before {
                return Err("Snap pending stroke changed page".into());
            }
        }
        app.drawing
            .virtual_events(vec![event(&app, [20.5, 12.5], Phase::Up)], pen);
        frame(&ctx, &mut app, &mut time, false, true, vec![]);
        for (x, y, on) in [(4, 8, true), (12, 12, true), (20, 8, true), (20, 12, false)] {
            if (app.documents[0].layers[0].pixel(64, 48, x, y)[3] > 0) != on {
                return Err(format!(
                    "Production snapping pen={pen} virtual={virtual_pressure} at {x},{y}: {:?}",
                    app.status
                ));
            }
        }
        if app.documents[0].history_counts().0 != 1
            || app.workspaces.store.current.snapping != settings
            || app.documents[0].scene.rulers != before.scene.rulers
            || app.documents[0].scene.guides != before.scene.guides
        {
            return Err("Temporary snapping changed saved options/guides/history".into());
        }
        if !app.documents[0].undo() || app.documents[0].snapshot() != before {
            return Err("Snap stroke Undo".into());
        }
        app.drawing
            .virtual_events(vec![event(&app, [28.5, 12.5], Phase::Down)], pen);
        frame(&ctx, &mut app, &mut time, true, true, vec![]);
        frame(
            &ctx,
            &mut app,
            &mut time,
            true,
            false,
            vec![egui::Event::WindowFocused(false)],
        );
        if app.drawing.pending() || app.documents[0].snapshot() != before {
            return Err("Focus loss committed disabled-snap input".into());
        }
        frame(
            &ctx,
            &mut app,
            &mut time,
            false,
            true,
            vec![egui::Event::WindowFocused(true)],
        );
        app.drawing.virtual_events(
            vec![
                event(&app, [28.5, 12.5], Phase::Down),
                event(&app, [28.5, 12.5], Phase::Up),
            ],
            pen,
        );
        frame(&ctx, &mut app, &mut time, false, true, vec![]);
        if app.documents[0].layers[0].pixel(64, 48, 28, 8)[3] == 0
            || app.documents[0].layers[0].pixel(64, 48, 28, 12)[3] != 0
            || app.workspaces.store.current.snapping != settings
        {
            return Err("Snapping stayed disabled after lost modifier/focus".into());
        }
    }
    for pen in [false, true] {
        for click in [false, true] {
            for vector in [false, true] {
                let ctx = egui::Context::default();
                let mut app = Drawer::new(&ctx, None);
                app.simple_view = true;
                let mut time = 0.0;
                app.create_canvas(CanvasSpec {
                    width: 48,
                    height: 32,
                    dpi: 300.0,
                    background: Background::Transparent,
                })
                .map_err(|e| e.to_string())?;
                app.documents[0].scene.guides = vec![[[0.0, 10.5], [48.0, 10.5]]];
                app.documents[0].scene.grid = drawer_core::aids::Grid {
                    spacing: [8.0; 2],
                    origin: [0.5; 2],
                };
                app.documents[0].view.zoom = 4.0;
                app.documents[0].view.angle = 90.0;
                app.workspaces.store.current.snapping = Settings {
                    rulers: false,
                    guides: true,
                    grid: true,
                    screen_distance: 4.0,
                    ..Default::default()
                };
                app.workspaces.store.current.shapes = drawer_core::shapes::Settings {
                    kind: drawer_core::shapes::Kind::Polyline,
                    input: if click {
                        drawer_core::shapes::Input::ByClick
                    } else {
                        drawer_core::shapes::Input::ByDrag
                    },
                    snap: true,
                    snap_step: 4.0,
                    width: 1.0,
                    ..Default::default()
                };
                app.workspaces.store.current.shapes_vector = vector;
                frame(&ctx, &mut app, &mut time, false, true, vec![]);
                frame(&ctx, &mut app, &mut time, false, true, vec![]);
                app.execute_command(&ctx, "shape-tool");
                frame(&ctx, &mut app, &mut time, false, true, vec![]);
                let raw = [[5.25, 11.25], [20.25, 11.25]];
                let resolved = [[5.25, 10.5], [20.25, 10.5]];
                let mut expected = drawer_core::shapes::prepare_resolved(
                    &app.documents[0],
                    if vector {
                        drawer_core::shapes::Target::Vector(None)
                    } else {
                        drawer_core::shapes::Target::Raster(0)
                    },
                    &resolved,
                    app.workspaces.store.current.shapes,
                )?;
                if pen && click && vector {
                    let path = expected
                        .scene
                        .lines
                        .layers
                        .last_mut()
                        .unwrap()
                        .paths
                        .last_mut()
                        .unwrap();
                    path.pressure.device = vec![Some(0.8); 2];
                    path.pressure.device_enabled =
                        app.workspaces.store.current.pressure.tools[6].size;
                }
                let before = app.documents[0].snapshot();
                let event = |app: &Drawer, p, phase| {
                    let mut e = super::precision::pointer(&ctx, app, p, phase);
                    if pen {
                        e.source = Source {
                            kind: Kind::Pen,
                            device: 18,
                            contact: 2,
                        };
                        e.pressure = Some(0.8);
                    }
                    e
                };
                let input = if click {
                    vec![
                        event(&app, raw[0], Phase::Down),
                        event(&app, raw[0], Phase::Up),
                        event(&app, raw[1], Phase::Down),
                        event(&app, raw[1], Phase::Up),
                    ]
                } else {
                    vec![
                        event(&app, raw[0], Phase::Down),
                        event(&app, raw[1], Phase::Move),
                        event(&app, raw[1], Phase::Up),
                    ]
                };
                app.drawing.virtual_events(input, pen);
                frame(&ctx, &mut app, &mut time, false, true, vec![]);
                if click {
                    frame(
                        &ctx,
                        &mut app,
                        &mut time,
                        false,
                        true,
                        vec![egui::Event::Key {
                            key: egui::Key::Enter,
                            physical_key: None,
                            pressed: true,
                            repeat: false,
                            modifiers: Default::default(),
                        }],
                    );
                }
                if app.documents[0].snapshot() != expected {
                    return Err(format!(
                        "Shape snap priority pen={pen} click={click} vector={vector}: {:?}",
                        app.status
                    ));
                }
                if !app.documents[0].undo() || app.documents[0].snapshot() != before {
                    return Err("Shape snapping Undo".into());
                }
            }
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn production_modifiers_lost_release_brush_shape_priority() {
        let root = crate::tests::TestDirectory::new();
        super::smoke(&root.0).unwrap();
    }
}
